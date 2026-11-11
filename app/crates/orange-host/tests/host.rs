//! The host runs several graphs at once on real time, controls each independently, and
//! runs an installed package's asynchronous node on a worker thread.
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use orange_host::{
    ControlOutcome, FlowEvent, FlowId, Grants, Host, HostError, HostEvent, HostOptions, NodeId,
    RunState, StopMode, Value, VariableId,
};
use orange_model::binding::{NewNode, new_node_edit};
use orange_model::document::DefinitionRef;
use orange_model::{GraphDocument, from_json, to_json};
use orange_packages::ArchiveFiles;
use serde_json::{Value as Json, json};

const I64: &str = "core/i64@1";
const ACCUMULATE: &str = "org.example.probe/accumulate";
const TOTAL: u32 = 100;
const WAIT: Duration = Duration::from_secs(20);

fn id(n: u32) -> String {
    format!("10000000-0000-4000-8000-{n:012x}")
}
fn node(n: u32) -> NodeId {
    id(n).parse().unwrap()
}
fn total() -> VariableId {
    id(TOTAL).parse().unwrap()
}
fn core(n: u32, name: &str) -> Json {
    let mut node = json!({"id": id(n), "definition_id": format!("core/{name}"),
        "definition_version": 1, "config_version": 1, "config": {}});
    if name.starts_with("on-") {
        node["activation"] = json!({"mode": "queue", "capacity": 4, "overflow": "fault",
            "on_fault": "continue", "order": 0});
    }
    node
}
fn edge(n: u32, from: u32, out: &str, to: u32, input: &str) -> Json {
    json!({"id": format!("30000000-0000-4000-8000-{n:012x}"),
        "from": {"node": id(from), "port": out}, "to": {"node": id(to), "port": input}})
}
fn with_total(mut saved: Json, nodes: Vec<Json>, edges: Json) -> String {
    saved["nodes"].as_array_mut().unwrap().extend(nodes);
    saved["edges"] = edges;
    saved["variables"] = json!([{"id": id(TOTAL), "name": "total", "type": I64, "scope": "run"}]);
    // Round-trip through the model so the text is exactly what a host would save.
    to_json(&from_json(&saved.to_string()).unwrap()).unwrap()
}
fn empty(graph: u32) -> GraphDocument {
    GraphDocument::new(
        format!("20000000-0000-4000-8000-{graph:012x}")
            .parse()
            .unwrap(),
    )
}

/// A manual entry that stores 7 in `total` each time the host triggers it.
fn manual_graph(graph: u32) -> String {
    let saved: Json = serde_json::from_str(&to_json(&empty(graph)).unwrap()).unwrap();
    let mut set = core(2, "set-variable");
    set["config"] = json!({"variable": id(TOTAL)});
    set["inputs"] = json!({"value": {"type": I64, "value": "7"}});
    with_total(
        saved,
        vec![core(1, "on-manual"), set],
        json!([edge(1, 1, "then", 2, "in")]),
    )
}

fn until(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn read_total(host: &Host, flow: FlowId) -> Option<Value> {
    host.inspect(flow, |run| run.variable(total()).cloned())
        .unwrap()
}
fn finished(events: &Receiver<HostEvent>, flows: &[FlowId]) -> Vec<HostEvent> {
    let (mut seen, mut left) = (Vec::new(), flows.to_vec());
    while !left.is_empty() {
        let event = events.recv_timeout(WAIT).expect("flows finish");
        if matches!(event.kind, FlowEvent::Finished) {
            left.retain(|flow| *flow != event.flow);
        }
        seen.push(event);
    }
    seen
}

#[test]
fn two_flows_run_at_once_and_one_pauses_without_holding_the_other() {
    let (mut host, events) = Host::new(HostOptions::default()).unwrap();
    let held = host.open(&manual_graph(1)).unwrap();
    let free = host.open(&manual_graph(2)).unwrap();
    assert_eq!(host.info(held).unwrap().manual_entries, [(node(1), false)]);
    until("both running", || {
        host.state(held) == Some(RunState::Running) && host.state(free) == Some(RunState::Running)
    });

    host.pause(held).unwrap();
    until("the pause", || host.state(held) == Some(RunState::Paused));
    host.trigger(held, node(1), None).unwrap();
    host.trigger(free, node(1), None).unwrap();

    until("the free flow", || {
        read_total(&host, free) == Some(Value::I64(7))
    });
    // The paused flow took its trigger and kept it; it has not run.
    assert_eq!(read_total(&host, held), None);
    assert_eq!(host.state(held), Some(RunState::Paused));

    host.resume(held).unwrap();
    until("the held trigger", || {
        read_total(&host, held) == Some(Value::I64(7))
    });

    host.stop(held, StopMode::Graceful).unwrap();
    host.stop(free, StopMode::Graceful).unwrap();
    let seen = finished(&events, &[held, free]);
    for flow in [held, free] {
        assert!(seen.iter().any(|event| event.flow == flow
            && matches!(&event.kind, FlowEvent::Node(step) if step.node == node(2))));
        assert!(matches!(
            host.inspect(flow, |_| ()),
            Err(HostError::FlowEnded)
        ));
    }
    assert_eq!(host.events_dropped(), 0);
    host.close(held).unwrap();
    assert!(matches!(host.pause(held), Err(HostError::UnknownFlow)));
}

#[test]
fn an_invalid_document_is_refused_with_its_diagnostics() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    let mut saved: Json = serde_json::from_str(&manual_graph(1)).unwrap();
    saved["variables"] = json!([]);
    let text = saved.to_string();
    assert!(!host.check(&text).unwrap().is_empty());
    let Err(HostError::Invalid(diagnostics)) = host.open(&text) else {
        panic!("a reference to an undeclared variable must not run");
    };
    assert!(!diagnostics.is_empty());
    assert_eq!(host.flows().count(), 0);
    assert!(host.check(&manual_graph(1)).unwrap().is_empty());
}

/// The probe package as a publisher would ship it: manifest, component and one
/// definition file per node.
fn probe_archive() -> Vec<u8> {
    let component = match std::env::var_os("ORANGE_GUEST_PROBE") {
        Some(path) => std::fs::read(path).expect("ORANGE_GUEST_PROBE names a readable component"),
        None => include_bytes!("../../orange-guest/tests/fixtures/probe.wasm").to_vec(),
    };
    let definition = json!({"definition_id": ACCUMULATE, "definition_version": 1,
    "config_version": 1, "kind": "async", "suspends": "always", "determinism": "external",
    "ports": [
        {"port_id": "amount", "direction": "input", "kind": "data", "type": I64},
        {"port_id": "result", "direction": "output", "kind": "data", "type": I64,
            "source": "execution", "available_on": ["then"]}
    ]})
    .to_string();
    let mut manifest = json!({
        "format": "orange.plugin", "format_version": 1,
        "id": "org.example.probe", "version": "1.0.0", "publisher": "org.example",
        "host_api": {"major": 1, "min_minor": 0, "max_minor": 0},
        "execution": {"kind": "wasm-component", "entry": "component.wasm",
            "world": "orange:expansion/plugin@1.0.0"},
        "dependencies": [],
        "contributions": {
            "nodes": [ACCUMULATE],
            "providers": [{"slot": format!("node:{ACCUMULATE}@1"), "id": "guest"}]
        },
        "permissions": [],
        "integrity": {"algorithm": "sha256", "content_digest": "0".repeat(64)},
        "license": "MIT"
    });
    let files = ArchiveFiles::new(vec![
        ("component.wasm".parse().unwrap(), &component[..]),
        (
            "nodes/accumulate.json".parse().unwrap(),
            definition.as_bytes(),
        ),
    ])
    .unwrap();
    let digest = files
        .content_digest(&serde_json::to_vec(&manifest).unwrap())
        .unwrap();
    manifest["integrity"]["content_digest"] = json!(digest.strip_prefix("sha256:").unwrap());
    files.encode(&serde_json::to_vec(&manifest).unwrap())
}

#[test]
fn an_installed_package_node_runs_on_a_worker_and_returns_to_its_flow() {
    let (mut host, events) = Host::new(HostOptions::default()).unwrap();
    let definition = DefinitionRef {
        id: ACCUMULATE.parse().unwrap(),
        version: 1,
    };
    assert!(host.registry().definition(&definition).is_none());
    // The probe imports `log`, so the user's grant must include it.
    assert!(matches!(
        host.install(&probe_archive()[..10], Grants { log: true }),
        Err(HostError::Archive(_))
    ));
    let key = host
        .install(&probe_archive(), Grants { log: true })
        .unwrap();
    assert_eq!(key.id.as_str(), "org.example.probe");

    // On Start calls the package node three times, adding five each time, and stores
    // each result. The node is authored against the host's registry, as an editor would.
    let mut document = empty(3);
    let mut call = NewNode::new(node(3), definition);
    call.inputs = serde_json::from_value(json!({"amount": {"type": I64, "value": "5"}})).unwrap();
    let edit = new_node_edit(&document, host.registry(), call).unwrap();
    let _ = document.apply(edit.commands).unwrap();
    let saved: Json = serde_json::from_str(&to_json(&document).unwrap()).unwrap();
    let mut count = core(2, "count");
    count["inputs"] = json!({"count": {"type": I64, "value": "3"}});
    let mut set = core(4, "set-variable");
    set["config"] = json!({"variable": id(TOTAL)});
    let text = with_total(
        saved,
        vec![core(1, "on-start"), count, set],
        json!([
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 3, "in"),
            edge(3, 3, "then", 4, "in"),
            edge(4, 3, "result", 4, "value"),
        ]),
    );

    let flow = host.open(&text).unwrap();
    until("three guest calls", || {
        matches!(
            host.inspect(flow, |run| run.variable(total()).cloned()),
            Ok(Some(Value::I64(15))) | Err(HostError::FlowEnded)
        )
    });
    host.close(flow).unwrap();
    let seen = finished(&events, &[flow]);
    let calls = seen
        .iter()
        .filter(|event| {
            matches!(&event.kind, FlowEvent::Node(step)
                if step.node == node(3) && step.outcome == ControlOutcome::Then)
        })
        .count();
    assert_eq!(calls, 3, "{seen:?}");
}

const REMEMBER: &str = "org.example.tools/remember";

/// A package with no code: its one node is a graph that stores its input in a local
/// variable of its own and returns what it reads back.
fn graph_package(output_type: &str, with_variable: bool) -> Result<Vec<u8>, HostError> {
    let port = |name: &str, direction: &str, ty: &str| json!({"port_id": name, "direction": direction, "kind": "data", "type": ty});
    let manifest = json!({
        "format": "orange.plugin", "format_version": 1,
        "id": "org.example.tools", "version": "1.0.0", "publisher": "org.example",
        "host_api": {"major": 1, "min_minor": 0, "max_minor": 0},
        "dependencies": [],
        "contributions": {
            "nodes": [REMEMBER],
            "providers": [{"slot": format!("node:{REMEMBER}@1"), "id": orange_host::GRAPH_PROVIDER}]
        },
        "permissions": [],
        "license": "MIT"
    });
    let definition = json!({"definition_id": REMEMBER, "definition_version": 1,
    "config_version": 1, "kind": "action", "suspends": "never", "determinism": "external",
    "ports": [
        port("value", "input", I64),
        {"port_id": "result", "direction": "output", "kind": "data", "type": output_type,
            "source": "execution", "available_on": ["then"]}
    ]});
    let mut keep = core(203, "set-variable");
    keep["config"] = json!({"variable": id(210)});
    let mut read = core(204, "get-variable");
    read["config"] = json!({"variable": id(210)});
    let variables = if with_variable {
        json!([{"id": id(210), "name": "kept", "type": I64, "scope": "local",
            "init": {"type": I64, "value": "0"}}])
    } else {
        json!([])
    };
    let graph = json!({
        "format": orange_host::GRAPH_NODE_FORMAT, "format_version": 1,
        "function": {"id": id(200), "name": "remember", "kind": "action",
            "inputs": [port("value", "input", I64)],
            "outputs": [port("result", "output", I64)],
            "nodes": [core(201, "function-entry"), keep, read, core(205, "return")],
            "edges": [
                edge(201, 201, "then", 203, "in"),
                edge(202, 201, "value", 203, "value"),
                edge(203, 203, "then", 205, "in"),
                edge(204, 204, "value", 205, "result"),
            ]},
        "variables": variables
    });
    orange_host::pack(
        manifest.to_string().as_bytes(),
        &[
            (
                "nodes/remember.json".into(),
                definition.to_string().into_bytes(),
            ),
            (
                "graphs/remember.json".into(),
                graph.to_string().into_bytes(),
            ),
        ],
    )
}

#[test]
fn a_package_node_made_as_a_graph_runs_like_any_other_node() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    let key = host
        .install(&graph_package(I64, true).unwrap(), Grants::default())
        .unwrap();
    assert_eq!(key.id.as_str(), "org.example.tools");

    // The document holds a locked package node; it knows nothing of the graph behind it.
    let mut document = empty(4);
    let mut call = NewNode::new(
        node(3),
        DefinitionRef {
            id: REMEMBER.parse().unwrap(),
            version: 1,
        },
    );
    call.inputs = serde_json::from_value(json!({"value": {"type": I64, "value": "5"}})).unwrap();
    let edit = new_node_edit(&document, host.registry(), call).unwrap();
    assert_eq!(edit.provider.id, orange_host::GRAPH_PROVIDER);
    let _ = document.apply(edit.commands).unwrap();
    let saved: Json = serde_json::from_str(&to_json(&document).unwrap()).unwrap();
    let mut set = core(4, "set-variable");
    set["config"] = json!({"variable": id(TOTAL)});
    let text = with_total(
        saved,
        vec![core(1, "on-manual"), set],
        json!([
            edge(1, 1, "then", 3, "in"),
            edge(2, 3, "then", 4, "in"),
            edge(3, 3, "result", 4, "value"),
        ]),
    );
    assert!(text.contains(REMEMBER) && !text.contains("core/call"));
    assert!(host.check(&text).unwrap().is_empty());

    let flow = host.open(&text).unwrap();
    host.trigger(flow, node(1), None).unwrap();
    until("the graph node's result", || {
        read_total(&host, flow) == Some(Value::I64(5))
    });
}

#[test]
fn a_graph_package_that_breaks_its_contract_or_is_invalid_is_not_installed() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    // The node promises a string result; its graph returns an integer.
    assert!(matches!(
        host.install(&graph_package("core/string@1", true).unwrap(), Grants::default()),
        Err(HostError::Graph(name, _)) if name == REMEMBER
    ));
    // The body uses a variable the package does not declare.
    let Err(HostError::GraphInvalid(package, diagnostics)) =
        host.install(&graph_package(I64, false).unwrap(), Grants::default())
    else {
        panic!("an invalid graph must not be published");
    };
    assert_eq!(package, "org.example.tools");
    assert!(!diagnostics.is_empty());
    let definition = DefinitionRef {
        id: REMEMBER.parse().unwrap(),
        version: 1,
    };
    assert!(host.registry().new_binding_default(&definition).is_none());
}
