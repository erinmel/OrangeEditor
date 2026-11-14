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

#[test]
fn variables_and_connections_are_declared_changed_and_removed() {
    use orange_host::{Connection, ConnectionScope, VarScope, Variable};
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let total = Variable {
        id: None,
        name: "total".into(),
        ty: "core/i64@1".into(),
        scope: VarScope::Run,
        persistent: false,
        initial: Some("3".into()),
    };
    let (text, made) = host.declare_variable(&empty(), &total).unwrap();
    let declared = host.declarations(&text).unwrap();
    assert_eq!(declared.variables.len(), 2);
    let found = &declared.variables[1];
    assert_eq!(
        (found.id.as_deref(), found.initial.as_deref()),
        (Some(made.as_str()), Some("3"))
    );
    // The same declaration of the same text gives the same ID.
    assert_eq!(host.declare_variable(&empty(), &total).unwrap().1, made);
    // Changing it keeps its ID; a value that is not of its type is refused.
    let renamed = Variable {
        id: Some(made.clone()),
        name: "sum".into(),
        initial: None,
        ..total.clone()
    };
    let text = host.declare_variable(&text, &renamed).unwrap().0;
    let found = &host.declarations(&text).unwrap().variables[1];
    assert_eq!((found.name.as_str(), found.initial.as_ref()), ("sum", None));
    let wrong = Variable {
        initial: Some("lots".into()),
        ..renamed
    };
    assert!(host.declare_variable(&text, &wrong).is_err());
    let text = host.remove_variable(&text, &made).unwrap();
    assert_eq!(host.declarations(&text).unwrap().variables.len(), 1);

    let mut api = Connection {
        id: None,
        name: "api".into(),
        adapter: "org.orange.net/http".into(),
        scope: ConnectionScope::Graph,
        settings: json!({"base_url": "https://example.test", "bearer": "plain"}),
    };
    // A secret is never written into the document, only a reference to the store.
    let refused = host.declare_connection(&text, &api).unwrap_err();
    assert!(refused.to_string().contains("/bearer"), "{refused}");
    assert!(!text.contains("plain"));
    api.settings["bearer"] = json!("credential://api/token");
    let (text, made) = host.declare_connection(&text, &api).unwrap();
    let found = &host.declarations(&text).unwrap().connections[0];
    assert_eq!(found.id.as_deref(), Some(made.as_str()));
    assert_eq!(found.settings["bearer"], "credential://api/token");
    let text = host.remove_connection(&text, &made).unwrap();
    assert!(host.declarations(&text).unwrap().connections.is_empty());
}

#[test]
fn a_view_says_what_a_node_settles_and_a_generic_type_is_chosen() {
    use orange_host::ReferenceKind;
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let (text, set) = host
        .add_node(&empty(), "core/set-variable", [0.0, 0.0])
        .unwrap();
    let (text, timer) = host.add_node(&text, "core/on-timer", [0.0, 200.0]).unwrap();
    let (text, collect) = host.add_node(&text, "core/collect", [0.0, 400.0]).unwrap();
    let view = host.view(&text).unwrap();
    let node = |id: NodeId| view.nodes.iter().find(|node| node.id == id).unwrap();
    assert_eq!(
        node(set).references,
        [("/variable".to_owned(), ReferenceKind::Variable)]
    );
    let schema = node(timer).schema.as_ref().unwrap();
    assert_eq!(schema["properties"]["interval"]["format"], "duration-ns");
    assert_eq!(
        node(collect).schema.as_ref().unwrap()["required"][0],
        "max_items"
    );
    let param = &node(collect).types[0];
    assert_eq!((param.name.as_str(), param.chosen.as_ref()), ("T", None));

    let text = host
        .set_config(&text, timer, &json!({"interval": "1000000000"}))
        .unwrap();
    assert_eq!(
        host.view(&text).unwrap().nodes[1].settings["interval"],
        "1000000000"
    );
    let chosen = host
        .choose_type(&text, collect, "T", Some("core/string@1"))
        .unwrap();
    let view = host.view(&chosen).unwrap();
    let param = &view
        .nodes
        .iter()
        .find(|node| node.id == collect)
        .unwrap()
        .types[0];
    assert_eq!(param.chosen.as_deref(), Some("core/string@1"));
    let cleared = host.choose_type(&chosen, collect, "T", None).unwrap();
    assert_eq!(host.view(&cleared).unwrap().nodes[2].types[0].chosen, None);
}

#[test]
fn copied_nodes_paste_with_their_wire_under_new_ids_and_keep_their_variable() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let (text, trigger, set) = built(&host);
    let clip = host.copy(&text, &[trigger, set]).unwrap();
    let (pasted, fresh) = host.paste(&text, &clip, [1000.0, 500.0]).unwrap();
    assert_eq!(fresh.len(), 2);
    assert!(!fresh.contains(&trigger) && !fresh.contains(&set));
    let view = host.view(&pasted).unwrap();
    assert_eq!((view.nodes.len(), view.wires.len()), (4, 2));
    let place = |id: NodeId| {
        view.nodes
            .iter()
            .find(|node| node.id == id)
            .unwrap()
            .position
    };
    // Places keep their distance; the top-left one is where it was pasted.
    assert_eq!(place(fresh[0]), Some([1000.0, 500.0]));
    assert_eq!(place(fresh[1]), Some([1320.0, 500.0]));
    // The pasted variable node names the same variable, and the document is sound.
    let declared = host.declarations(&pasted).unwrap();
    assert_eq!(declared.variables.len(), 1);
    assert_eq!(host.check(&pasted).unwrap(), []);
    assert!(host.paste(&text, "some text", [0.0, 0.0]).is_err());
}
