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

/// `manual_graph(1)` after an edit: the entry now stores `value`, and the document has a
/// second run variable.
fn edited_graph(value: i64) -> String {
    let mut saved: Json = serde_json::from_str(&manual_graph(1)).unwrap();
    for node in saved["nodes"].as_array_mut().unwrap() {
        if node["definition_id"] == "core/set-variable" {
            node["inputs"]["value"]["value"] = json!(value.to_string());
        }
    }
    // The new variable is written after the old one, so the plan holds it.
    let mut also = core(3, "set-variable");
    also["config"] = json!({"variable": id(TOTAL + 1)});
    also["inputs"] = json!({"value": {"type": I64, "value": "5"}});
    saved["nodes"].as_array_mut().unwrap().push(also);
    saved["edges"]
        .as_array_mut()
        .unwrap()
        .push(edge(2, 2, "then", 3, "in"));
    saved["variables"].as_array_mut().unwrap().push(
        json!({"id": id(TOTAL + 1), "name": "extra", "type": I64, "scope": "run",
            "init": {"type": I64, "value": "3"}}),
    );
    to_json(&from_json(&saved.to_string()).unwrap()).unwrap()
}

#[test]
fn an_edit_replaces_a_running_graph_and_keeps_its_variables() {
    let (mut host, events) = Host::new(HostOptions::default()).unwrap();
    let flow = host.open(&manual_graph(1)).unwrap();
    host.trigger(flow, node(1), None).unwrap();
    until("the first value", || {
        read_total(&host, flow) == Some(Value::I64(7))
    });

    // Moving nodes around is not an edit of what runs.
    let mut moved: Json = serde_json::from_str(&manual_graph(1)).unwrap();
    moved["editor"] = json!({"nodes": {"a": {"x": 10, "y": 20}}});
    moved["revision"] = json!(5);
    assert!(!host.update(flow, &moved.to_string()).unwrap().changed);

    let updated = host.update(flow, &edited_graph(9)).unwrap();
    assert!(updated.changed);
    assert_eq!((updated.kept_variables, updated.reset_variables), (1, 1));
    // The value written by the old graph is still there, and the new variable has its
    // initial value.
    assert_eq!(read_total(&host, flow), Some(Value::I64(7)));
    let extra: VariableId = id(TOTAL + 1).parse().unwrap();
    assert_eq!(
        host.inspect(flow, move |run| run.variable(extra).cloned())
            .unwrap(),
        Some(Value::I64(3))
    );
    // The same entry now runs the edited graph.
    host.trigger(flow, node(1), None).unwrap();
    until("the edited value", || {
        read_total(&host, flow) == Some(Value::I64(9))
    });

    host.stop(flow, StopMode::Graceful).unwrap();
    let seen = finished(&events, &[flow]);
    assert!(seen.iter().any(|event| matches!(
        event.kind,
        FlowEvent::Updated {
            kept_variables: 1,
            reset_variables: 1
        }
    )));
    assert!(matches!(
        host.update(flow, &edited_graph(11)),
        Err(HostError::FlowEnded)
    ));
}

#[test]
fn a_refused_edit_leaves_the_flow_running_and_a_paused_flow_stays_paused() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    let flow = host.open(&manual_graph(1)).unwrap();

    let mut broken: Json = serde_json::from_str(&manual_graph(1)).unwrap();
    broken["variables"] = json!([]);
    assert!(matches!(
        host.update(flow, &broken.to_string()),
        Err(HostError::Invalid(_))
    ));
    assert!(matches!(
        host.update(flow, &manual_graph(2)),
        Err(HostError::Document(_))
    ));
    host.trigger(flow, node(1), None).unwrap();
    until("the untouched graph", || {
        read_total(&host, flow) == Some(Value::I64(7))
    });

    host.pause(flow).unwrap();
    until("the pause", || host.state(flow) == Some(RunState::Paused));
    assert!(host.update(flow, &edited_graph(9)).unwrap().changed);
    host.trigger(flow, node(1), None).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(host.state(flow), Some(RunState::Paused));
    assert_eq!(read_total(&host, flow), Some(Value::I64(7)));
    host.resume(flow).unwrap();
    until("the edited value", || {
        read_total(&host, flow) == Some(Value::I64(9))
    });
}

#[test]
fn the_host_lists_every_node_a_document_can_use() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    let find = |host: &Host, id: &str| {
        host.nodes()
            .find(|node| node.definition.definition.id.as_str() == id)
            .map(|node| (node.provider.cloned(), node.presentation.cloned()))
    };
    // A core node is implemented by the language, so it names no provider.
    assert_eq!(find(&host, "core/on-manual"), Some((None, None)));
    assert_eq!(find(&host, ACCUMULATE), None);

    let key = host
        .install(&probe_archive(), Grants { log: true })
        .unwrap();
    let (provider, _) = find(&host, ACCUMULATE).expect("an installed node is listed");
    assert_eq!(provider.unwrap().package, key.id.as_str());
}

#[test]
fn an_observed_flow_reports_its_variable_values_as_they_change() {
    let (mut host, events) = Host::new(HostOptions::default()).unwrap();
    let flow = host.open(&manual_graph(1)).unwrap();
    let quiet = host.open(&manual_graph(2)).unwrap();
    host.observe(flow, true).unwrap();
    for flow in [flow, quiet] {
        host.trigger(flow, node(1), None).unwrap();
        until("the value", || {
            read_total(&host, flow) == Some(Value::I64(7))
        });
        host.stop(flow, StopMode::Graceful).unwrap();
    }
    let seen = finished(&events, &[flow, quiet]);
    let reports: Vec<_> = seen
        .iter()
        .filter_map(|event| match &event.kind {
            FlowEvent::Variables(values) => Some((event.flow, values.clone())),
            _ => None,
        })
        .collect();
    // Only the observed flow reports, each report names the one variable, and the last
    // one holds the value the graph wrote.
    assert!(!reports.is_empty());
    assert!(
        reports
            .iter()
            .all(|(from, values)| *from == flow && values.len() == 1 && values[0].0 == total())
    );
    assert_eq!(reports.last().unwrap().1[0].1, Some(Value::I64(7)));
    // A value is reported once, not on every turn.
    let sevens = reports
        .iter()
        .filter(|(_, values)| values[0].1 == Some(Value::I64(7)))
        .count();
    assert_eq!(sevens, 1);
}

#[test]
fn an_observed_flow_reports_the_values_on_its_wires() {
    let (mut host, events) = Host::new(HostOptions::default()).unwrap();
    // The entry's payload is wired into the variable write.
    let saved: Json = serde_json::from_str(&to_json(&empty(1)).unwrap()).unwrap();
    let mut entry = core(1, "on-manual");
    entry["config"] = json!({"payload": I64});
    let mut set = core(2, "set-variable");
    set["config"] = json!({"variable": id(TOTAL)});
    let text = with_total(
        saved,
        vec![entry, set],
        json!([
            edge(1, 1, "then", 2, "in"),
            edge(2, 1, "payload", 2, "value")
        ]),
    );
    let flow = host.open(&text).unwrap();
    host.observe(flow, true).unwrap();
    for value in [5, 6] {
        host.trigger(flow, node(1), Some(Value::I64(value)))
            .unwrap();
        until("the value", || {
            read_total(&host, flow) == Some(Value::I64(value))
        });
    }
    host.stop(flow, StopMode::Graceful).unwrap();
    let seen = finished(&events, &[flow]);
    let on_wire: Vec<_> = seen
        .iter()
        .filter_map(|event| match &event.kind {
            FlowEvent::Ports(ports) => Some(ports.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    // Each report names the entry's payload port, and the values arrive in order.
    assert!(
        on_wire
            .iter()
            .all(|(from, port, _)| *from == node(1) && port.to_string() == "payload")
    );
    let values: Vec<_> = on_wire.into_iter().map(|(_, _, value)| value).collect();
    assert_eq!(values, [Value::I64(5), Value::I64(6)]);
}

#[test]
fn installed_packages_are_there_again_after_a_restart() {
    let directory = std::env::temp_dir().join(format!(
        "orange-host-store-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    let options = || HostOptions {
        package_dir: Some(directory.clone()),
        ..HostOptions::default()
    };
    let listed = |host: &Host| {
        host.nodes()
            .any(|node| node.definition.definition.id.as_str() == REMEMBER)
    };
    {
        let (mut host, _events) = Host::new(options()).unwrap();
        assert!(!listed(&host));
        let _ = host
            .install(&graph_package(I64, true).unwrap(), Grants::default())
            .unwrap();
        // A refused package is not stored.
        assert!(host.install(b"not an archive", Grants::default()).is_err());
    }
    {
        let (host, _events) = Host::new(options()).unwrap();
        assert!(host.not_restored().is_empty());
        assert!(listed(&host));
    }
    // A stored archive that no longer verifies is reported and the host still starts.
    let archive = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|e| e == "orangeplug"))
        .unwrap();
    std::fs::write(&archive, b"damaged").unwrap();
    let (host, _events) = Host::new(options()).unwrap();
    assert_eq!(host.not_restored().len(), 1);
    assert!(!listed(&host));
    drop(host);
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn the_base_nodes_can_be_left_out() {
    let packages = |base_nodes| {
        let options = HostOptions {
            base_nodes,
            ..HostOptions::default()
        };
        let (host, _events) = Host::new(options).unwrap();
        host.nodes()
            .filter(|node| !node.definition.definition.id.is_core())
            .count()
    };
    assert!(packages(true) > 0);
    assert_eq!(packages(false), 0);
    // The core language alone still runs a graph.
    let options = HostOptions {
        base_nodes: false,
        ..HostOptions::default()
    };
    let (mut host, _events) = Host::new(options).unwrap();
    let flow = host.open(&manual_graph(1)).unwrap();
    host.trigger(flow, node(1), None).unwrap();
    until("the value", || {
        read_total(&host, flow) == Some(Value::I64(7))
    });
}

const ON_POKE: &str = "org.example.triggers/on-poke";

/// A package with no code whose one node is a trigger: its fragment starts at a manual
/// entry and emits what it received.
fn trigger_package(with_event: bool) -> Result<Vec<u8>, HostError> {
    let manifest = json!({
        "format": "orange.plugin", "format_version": 1,
        "id": "org.example.triggers", "version": "1.0.0", "publisher": "org.example",
        "host_api": {"major": 1, "min_minor": 0, "max_minor": 0},
        "dependencies": [],
        "contributions": {
            "nodes": [ON_POKE],
            "providers": [{"slot": format!("node:{ON_POKE}@1"), "id": orange_host::GRAPH_PROVIDER}]
        },
        "permissions": [],
        "license": "MIT"
    });
    let definition = json!({"definition_id": ON_POKE, "definition_version": 1,
        "config_version": 1, "kind": "entry", "determinism": "external",
        "ports": [{"port_id": "payload", "direction": "output", "kind": "data", "type": I64,
            "source": "payload"}]});
    let mut inner = core(301, "on-manual");
    inner["config"] = json!({"payload": I64});
    let mut emit = core(302, "emit");
    emit["config"] = json!({"event": if with_event { id(310) } else { id(311) }});
    let graph = json!({
        "format": orange_host::GRAPH_NODE_FORMAT, "format_version": 1,
        "trigger": {
            "event": id(310),
            "nodes": [inner, emit],
            "edges": [
                edge(301, 301, "then", 302, "in"),
                edge(302, 301, "payload", 302, "payload"),
            ]
        }
    });
    orange_host::pack(
        manifest.to_string().as_bytes(),
        &[
            (
                "nodes/on-poke.json".into(),
                definition.to_string().into_bytes(),
            ),
            ("graphs/on-poke.json".into(), graph.to_string().into_bytes()),
        ],
    )
}

#[test]
fn a_trigger_made_as_a_graph_starts_the_documents_own_nodes() {
    // A fragment that emits an event it does not own is refused.
    let (mut strict, _events) = Host::new(HostOptions::default()).unwrap();
    assert!(
        strict
            .install(&trigger_package(false).unwrap(), Grants::default())
            .is_err()
    );
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    let _ = host
        .install(&trigger_package(true).unwrap(), Grants::default())
        .unwrap();

    // Two uses of the trigger, each writing its own variable.
    let mut document = empty(1);
    for n in [1, 3] {
        let poke = NewNode::new(
            node(n),
            DefinitionRef {
                id: ON_POKE.parse().unwrap(),
                version: 1,
            },
        );
        let edit = new_node_edit(&document, host.registry(), poke).unwrap();
        let _ = document.apply(edit.commands).unwrap();
    }
    let mut saved: Json = serde_json::from_str(&to_json(&document).unwrap()).unwrap();
    for entry in saved["nodes"].as_array_mut().unwrap() {
        entry["activation"] = core(0, "on-manual")["activation"].clone();
    }
    let mut first = core(2, "set-variable");
    first["config"] = json!({"variable": id(TOTAL)});
    let mut second = core(4, "set-variable");
    second["config"] = json!({"variable": id(TOTAL + 1)});
    saved["nodes"]
        .as_array_mut()
        .unwrap()
        .extend([first, second]);
    saved["edges"] = json!([
        edge(1, 1, "then", 2, "in"),
        edge(2, 1, "payload", 2, "value"),
        edge(3, 3, "then", 4, "in"),
        edge(4, 3, "payload", 4, "value"),
    ]);
    saved["variables"] = json!([
        {"id": id(TOTAL), "name": "total", "type": I64, "scope": "run"},
        {"id": id(TOTAL + 1), "name": "other", "type": I64, "scope": "run"}
    ]);
    let text = to_json(&from_json(&saved.to_string()).unwrap()).unwrap();
    assert!(!text.contains("core/emit"));
    let diagnostics = host.check(&text).unwrap();
    assert!(diagnostics.is_empty(), "{diagnostics:?}");

    let flow = host.open(&text).unwrap();
    // Each use has its own inner entry; firing them reaches only that use's nodes.
    let inner = host.info(flow).unwrap().manual_entries.clone();
    assert_eq!(inner.len(), 2);
    host.trigger(flow, inner[0].0, Some(Value::I64(5))).unwrap();
    host.trigger(flow, inner[1].0, Some(Value::I64(6))).unwrap();
    let other: VariableId = id(TOTAL + 1).parse().unwrap();
    until("both uses", || {
        read_total(&host, flow).is_some()
            && host
                .inspect(flow, move |run| run.variable(other).cloned())
                .unwrap()
                .is_some()
    });
    let mut seen = vec![
        read_total(&host, flow).unwrap(),
        host.inspect(flow, move |run| run.variable(other).cloned())
            .unwrap()
            .unwrap(),
    ];
    seen.sort_by_key(|value| format!("{value:?}"));
    assert_eq!(seen, [Value::I64(5), Value::I64(6)]);
}
