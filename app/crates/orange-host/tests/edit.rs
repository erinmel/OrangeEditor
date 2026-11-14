//! Building and changing a document through the host's edit operations alone, as an
//! editor does.
use orange_host::{Host, HostError, HostOptions, NodeId, Value};
use orange_testkit::{id, until};
use serde_json::{Value as Json, json};

/// An empty document that declares one whole-number variable, `count`.
fn empty() -> String {
    json!({"format": "orange.graph", "format_version": 1,
        "graph_id": "20000000-0000-4000-8000-0000000000c3", "revision": 1,
        "package_lock": [], "type_lock": [], "events": [], "connections": [],
        "signatures": {}, "functions": [], "exports": [], "editor": {},
        "variables": [{"id": id(100), "name": "count", "type": "core/i64@1",
            "scope": "run", "persistent": false, "conflict": "reject"}],
        "nodes": [], "edges": []})
    .to_string()
}

fn output(host: &Host, text: &str, node: NodeId) -> String {
    let view = host.view(text).unwrap();
    let node = view.nodes.iter().find(|found| found.id == node).unwrap();
    let port = node.ports.iter().find(|port| !port.input && !port.exec);
    port.unwrap().id.clone()
}

/// A trigger wired to a node that sets `count` to 7.
fn built(host: &Host) -> (String, NodeId, NodeId) {
    let (text, trigger) = host
        .add_node(&empty(), "core/on-manual", [0.0, 0.0])
        .unwrap();
    let (text, set) = host
        .add_node(&text, "core/set-variable", [320.0, 0.0])
        .unwrap();
    let text = host
        .set_config(&text, set, &json!({"variable": id(100)}))
        .unwrap();
    let text = host.set_input(&text, (set, "value"), Some("7")).unwrap();
    let text = host.connect(&text, (trigger, "then"), (set, "in")).unwrap();
    (text, trigger, set)
}

#[test]
fn a_graph_is_built_from_an_empty_document_and_runs() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    // Half built, the document says what it still needs instead of being refused.
    let (text, _) = host
        .add_node(&empty(), "core/set-variable", [0.0, 0.0])
        .unwrap();
    assert!(!host.check(&text).unwrap().is_empty());

    let (text, trigger, set) = built(&host);
    assert_eq!(host.check(&text).unwrap(), []);
    let view = host.view(&text).unwrap();
    assert_eq!(view.nodes.len(), 2);
    assert_eq!(view.wires.len(), 1);
    let placed = view.nodes.iter().find(|node| node.id == set).unwrap();
    assert_eq!(placed.position, Some([320.0, 0.0]));
    let value = placed.ports.iter().find(|port| port.id == "value").unwrap();
    assert_eq!(value.value.as_deref(), Some("7"));
    // The same edits of the same text give the same document.
    assert_eq!(built(&host).0, text);

    let flow = host.open(&text).unwrap();
    host.trigger(flow, trigger, None).unwrap();
    let count = |host: &Host| {
        host.inspect(flow, |run| run.variable(id(100).parse().unwrap()).cloned())
            .unwrap()
    };
    until("the variable", || count(&host).is_some());
    assert_eq!(count(&host), Some(Value::I64(7)));

    // A changed value reaches the running flow as an edit.
    let edited = host.set_input(&text, (set, "value"), Some("8")).unwrap();
    assert!(host.update(flow, &edited).unwrap().changed);
    host.trigger(flow, trigger, None).unwrap();
    until("the edited value", || count(&host) == Some(Value::I64(8)));
}

#[test]
fn wires_that_cannot_be_are_refused_and_wires_and_nodes_are_taken_away() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let (text, trigger, set) = built(&host);
    // A step does not go into a value, and a port has to exist on the right side.
    assert!(matches!(
        host.connect(&text, (trigger, "then"), (set, "value")),
        Err(HostError::Document(_))
    ));
    assert!(matches!(
        host.connect(&text, (trigger, "nope"), (set, "in")),
        Err(HostError::Document(_))
    ));
    assert!(matches!(
        host.connect(&text, (set, "in"), (trigger, "then")),
        Err(HostError::Document(_))
    ));
    // Text does not go into a whole number: refused with the reason, nothing changed.
    let (with_text, concat) = host
        .add_node(&text, "org.orange.text/concat", [0.0, 200.0])
        .unwrap();
    let joined = output(&host, &with_text, concat);
    match host.connect(&with_text, (concat, &joined), (set, "value")) {
        Err(HostError::Invalid(reasons)) => assert!(!reasons.is_empty()),
        other => panic!("the wire was not refused: {other:?}"),
    }
    // A value that is not of the input's type is refused too.
    assert!(
        host.set_input(&text, (set, "value"), Some("seven"))
            .is_err()
    );
    assert!(host.set_input(&text, (set, "in"), Some("7")).is_err());

    // A second trigger wired to the same way in: both wires stay. A second wire out
    // of one step replaces the first.
    let (two, second) = host
        .add_node(&text, "core/on-manual", [0.0, 300.0])
        .unwrap();
    let two = host.connect(&two, (second, "then"), (set, "in")).unwrap();
    assert_eq!(host.view(&two).unwrap().wires.len(), 2);
    let (three, other) = host
        .add_node(&two, "core/set-variable", [320.0, 300.0])
        .unwrap();
    let three = host
        .connect(&three, (second, "then"), (other, "in"))
        .unwrap();
    let wires = host.view(&three).unwrap().wires;
    assert_eq!(wires.len(), 2);
    assert!(
        wires
            .iter()
            .any(|wire| wire.from.0 == second && wire.to.0 == other)
    );

    // Taking the value off, unwiring and removing.
    let bare = host.set_input(&text, (set, "value"), None).unwrap();
    let saved: Json = serde_json::from_str(&bare).unwrap();
    assert!(saved["nodes"].to_string().contains("core/set-variable"));
    assert!(!bare.contains("\"inputs\""));
    assert_eq!(host.set_input(&bare, (set, "value"), None).unwrap(), bare);
    let unwired = host.disconnect(&text, (set, "in"), true).unwrap();
    assert!(host.view(&unwired).unwrap().wires.is_empty());
    assert_eq!(
        host.disconnect(&unwired, (set, "in"), true).unwrap(),
        unwired
    );
    let removed = host.remove_nodes(&text, &[set]).unwrap();
    let view = host.view(&removed).unwrap();
    assert_eq!((view.nodes.len(), view.wires.len()), (1, 0));
    assert!(!removed.contains(&set.to_string()));
    assert!(
        host.add_node(&text, "org.example/missing", [0.0, 0.0])
            .is_err()
    );
}
