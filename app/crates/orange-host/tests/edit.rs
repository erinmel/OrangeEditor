//! Building and changing a document through the host's edit operations alone, as an
//! editor does: one draft, edited in place, with its problems following the edits.
use orange_host::{Draft, Host, HostError, HostOptions, NodeId, Value};
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

fn output(host: &Host, draft: &Draft, node: NodeId) -> String {
    let view = host.draft_view(draft).unwrap();
    let node = view.nodes.iter().find(|found| found.id == node).unwrap();
    let port = node.ports.iter().find(|port| !port.input && !port.exec);
    port.unwrap().id.clone()
}

/// A trigger wired to a node that sets `count` to 7.
fn built(host: &Host) -> (Draft, NodeId, NodeId) {
    let mut draft = host.draft(&empty()).unwrap();
    let trigger = host
        .add_node(&mut draft, "core/on-manual", [0.0, 0.0])
        .unwrap();
    let set = host
        .add_node(&mut draft, "core/set-variable", [320.0, 0.0])
        .unwrap();
    assert!(
        host.set_config(&mut draft, set, &json!({"variable": id(100)}))
            .unwrap()
    );
    assert!(
        host.set_input(&mut draft, (set, "value"), Some("7"))
            .unwrap()
    );
    assert!(
        host.connect(&mut draft, (trigger, "then"), (set, "in"))
            .unwrap()
    );
    (draft, trigger, set)
}

/// The draft's problems agree with checking its text from scratch.
fn same_problems(host: &Host, draft: &mut Draft) -> usize {
    let followed = host.problems(draft).unwrap();
    assert_eq!(followed, host.check(&draft.text().unwrap()).unwrap());
    followed.len()
}

#[test]
fn view_connectivity_matches_edges_after_disconnect_and_reconnect() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let (mut draft, trigger, set) = built(&host);
    let mut views = vec![host.draft_view(&draft).unwrap()];
    assert!(host.disconnect(&mut draft, (set, "in"), true).unwrap());
    views.push(host.draft_view(&draft).unwrap());
    assert!(
        host.connect(&mut draft, (trigger, "then"), (set, "in"))
            .unwrap()
    );
    views.push(host.draft_view(&draft).unwrap());
    for view in &views {
        for node in &view.nodes {
            for port in &node.ports {
                assert_eq!(
                    port.connected,
                    view.wires.iter().any(|wire| {
                        let end = if port.input { &wire.to } else { &wire.from };
                        end.0 == node.id && end.1 == port.id
                    })
                );
            }
        }
        for wire in &view.wires {
            let source = view
                .nodes
                .iter()
                .find(|node| node.id == wire.from.0)
                .unwrap();
            let port = source
                .ports
                .iter()
                .find(|port| !port.input && port.id == wire.from.1)
                .unwrap();
            assert_eq!(wire.exec, port.exec);
        }
        let set = view.nodes.iter().find(|node| node.id == set).unwrap();
        let value = set.ports.iter().find(|port| port.id == "value").unwrap();
        assert_eq!(value.value.as_deref(), Some("7"));
    }
}

#[test]
fn a_graph_is_built_from_an_empty_document_and_runs() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    // Half built, the document says what it still needs instead of being refused.
    let mut half = host.draft(&empty()).unwrap();
    let _ = host
        .add_node(&mut half, "core/set-variable", [0.0, 0.0])
        .unwrap();
    assert!(same_problems(&host, &mut half) > 0);

    let (mut draft, trigger, set) = built(&host);
    assert_eq!(same_problems(&host, &mut draft), 0);
    let view = host.draft_view(&draft).unwrap();
    assert_eq!(view.nodes.len(), 2);
    assert_eq!(view.wires.len(), 1);
    let placed = view.nodes.iter().find(|node| node.id == set).unwrap();
    assert_eq!(placed.position, Some([320.0, 0.0]));
    let value = placed.ports.iter().find(|port| port.id == "value").unwrap();
    assert_eq!(value.value.as_deref(), Some("7"));
    // The same edits of the same document give the same document.
    let text = draft.text().unwrap();
    assert_eq!(built(&host).0.text().unwrap(), text);

    let flow = host.open(&text).unwrap();
    host.trigger(flow, trigger, None).unwrap();
    let count = |host: &Host| {
        host.inspect(flow, |run| run.variable(id(100).parse().unwrap()).cloned())
            .unwrap()
    };
    until("the variable", || count(&host).is_some());
    assert_eq!(count(&host), Some(Value::I64(7)));

    // A changed value reaches the running flow as an edit.
    assert!(
        host.set_input(&mut draft, (set, "value"), Some("8"))
            .unwrap()
    );
    assert!(host.update(flow, &draft.text().unwrap()).unwrap().changed);
    host.trigger(flow, trigger, None).unwrap();
    until("the edited value", || count(&host) == Some(Value::I64(8)));
}

#[test]
fn each_edit_is_one_step_back_and_forward() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&empty()).unwrap();
    let mut texts = vec![draft.text().unwrap()];
    let trigger = host
        .add_node(&mut draft, "core/on-manual", [0.0, 0.0])
        .unwrap();
    texts.push(draft.text().unwrap());
    let set = host
        .add_node(&mut draft, "core/set-variable", [320.0, 0.0])
        .unwrap();
    texts.push(draft.text().unwrap());
    assert!(
        host.connect(&mut draft, (trigger, "then"), (set, "in"))
            .unwrap()
    );
    texts.push(draft.text().unwrap());
    assert!(draft.place(&[(set, [400.0, 40.0])]).unwrap());
    // Placing a node where it already is changes nothing.
    assert!(!draft.place(&[(set, [400.0, 40.0])]).unwrap());
    texts.push(draft.text().unwrap());

    // Undo walks back through every edit, problems following each step; redo returns.
    let same_content = |a: &str, b: &str| {
        let strip = |text: &str| {
            let mut saved: Json = serde_json::from_str(text).unwrap();
            saved.as_object_mut().unwrap().remove("revision");
            saved
        };
        strip(a) == strip(b)
    };
    for expected in texts.iter().rev().skip(1) {
        assert!(draft.undo());
        assert!(same_content(&draft.text().unwrap(), expected));
        let _ = same_problems(&host, &mut draft);
    }
    assert!(!draft.undo());
    for expected in texts.iter().skip(1) {
        assert!(draft.redo());
        assert!(same_content(&draft.text().unwrap(), expected));
        let _ = same_problems(&host, &mut draft);
    }
    assert_eq!(draft.can_undo(), (true, false));
}

#[test]
fn wires_that_cannot_be_are_refused_and_wires_and_nodes_are_taken_away() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let (mut draft, trigger, set) = built(&host);
    let before = draft.text().unwrap();
    // A step does not go into a value, and a port has to exist on the right side.
    for (from, to) in [
        ((trigger, "then"), (set, "value")),
        ((trigger, "nope"), (set, "in")),
        ((set, "in"), (trigger, "then")),
    ] {
        assert!(matches!(
            host.connect(&mut draft, from, to),
            Err(HostError::Document(_))
        ));
    }
    // Text does not go into a whole number: refused with the reason, and the draft and
    // its history are as they were.
    let concat = host
        .add_node(&mut draft, "org.orange.text/concat", [0.0, 200.0])
        .unwrap();
    let with_concat = draft.text().unwrap();
    let joined = output(&host, &draft, concat);
    match host.connect(&mut draft, (concat, &joined), (set, "value")) {
        Err(HostError::Invalid(reasons)) => assert!(!reasons.is_empty()),
        other => panic!("the wire was not refused: {other:?}"),
    }
    // Only the revision counts the attempt.
    let content = |text: &str| {
        let mut saved: Json = serde_json::from_str(text).unwrap();
        saved.as_object_mut().unwrap().remove("revision");
        saved
    };
    assert_eq!(content(&draft.text().unwrap()), content(&with_concat));
    assert_eq!(draft.can_undo(), (true, false));
    assert!(draft.undo());
    assert_eq!(
        serde_json::from_str::<Json>(&draft.text().unwrap()).unwrap()["nodes"],
        serde_json::from_str::<Json>(&before).unwrap()["nodes"]
    );
    // A value that is not of the input's type is refused too.
    assert!(
        host.set_input(&mut draft, (set, "value"), Some("seven"))
            .is_err()
    );
    assert!(host.set_input(&mut draft, (set, "in"), Some("7")).is_err());

    // A second trigger wired to the same way in: both wires stay. A second wire out
    // of one step replaces the first.
    let second = host
        .add_node(&mut draft, "core/on-manual", [0.0, 300.0])
        .unwrap();
    assert!(
        host.connect(&mut draft, (second, "then"), (set, "in"))
            .unwrap()
    );
    assert_eq!(host.draft_view(&draft).unwrap().wires.len(), 2);
    let other = host
        .add_node(&mut draft, "core/set-variable", [320.0, 300.0])
        .unwrap();
    assert!(
        host.connect(&mut draft, (second, "then"), (other, "in"))
            .unwrap()
    );
    let wires = host.draft_view(&draft).unwrap().wires;
    assert_eq!(wires.len(), 2);
    assert!(
        wires
            .iter()
            .any(|wire| wire.from.0 == second && wire.to.0 == other)
    );

    // Taking the value off, unwiring and removing.
    assert!(host.set_input(&mut draft, (set, "value"), None).unwrap());
    let bare = draft.text().unwrap();
    assert!(!bare.contains("\"inputs\""));
    assert!(!host.set_input(&mut draft, (set, "value"), None).unwrap());
    assert!(host.disconnect(&mut draft, (set, "in"), true).unwrap());
    assert!(!host.disconnect(&mut draft, (set, "in"), true).unwrap());
    assert!(host.remove_nodes(&mut draft, &[set]).unwrap());
    let removed = draft.text().unwrap();
    assert!(!removed.contains(&set.to_string()));
    assert!(
        host.add_node(&mut draft, "org.example/missing", [0.0, 0.0])
            .is_err()
    );
    let _ = same_problems(&host, &mut draft);
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
    let mut draft = host.draft(&empty()).unwrap();
    let made = host.declare_variable(&mut draft, &total).unwrap();
    let declared = host.declarations(&draft);
    assert_eq!(declared.variables.len(), 2);
    let found = &declared.variables[1];
    assert_eq!(
        (found.id.as_deref(), found.initial.as_deref()),
        (Some(made.as_str()), Some("3"))
    );
    // The same declaration of the same document gives the same ID.
    let mut again = host.draft(&empty()).unwrap();
    assert_eq!(host.declare_variable(&mut again, &total).unwrap(), made);
    // Changing it keeps its ID; a value that is not of its type is refused.
    let renamed = Variable {
        id: Some(made.clone()),
        name: "sum".into(),
        initial: None,
        ..total.clone()
    };
    let _ = host.declare_variable(&mut draft, &renamed).unwrap();
    let found = &host.declarations(&draft).variables[1];
    assert_eq!((found.name.as_str(), found.initial.as_ref()), ("sum", None));
    let wrong = Variable {
        initial: Some("lots".into()),
        ..renamed
    };
    assert!(host.declare_variable(&mut draft, &wrong).is_err());
    assert!(host.remove_variable(&mut draft, &made).unwrap());
    assert_eq!(host.declarations(&draft).variables.len(), 1);

    let mut api = Connection {
        id: None,
        name: "api".into(),
        adapter: "org.orange.net/http".into(),
        scope: ConnectionScope::Graph,
        settings: json!({"base_url": "https://example.test", "bearer": "plain"}),
    };
    // A secret is never written into the document, only a reference to the store.
    let refused = host.declare_connection(&mut draft, &api).unwrap_err();
    assert!(refused.to_string().contains("/bearer"), "{refused}");
    assert!(!draft.text().unwrap().contains("plain"));
    api.settings["bearer"] = json!("credential://api/token");
    let made = host.declare_connection(&mut draft, &api).unwrap();
    let found = &host.declarations(&draft).connections[0];
    assert_eq!(found.id.as_deref(), Some(made.as_str()));
    assert_eq!(found.settings["bearer"], "credential://api/token");
    assert!(host.remove_connection(&mut draft, &made).unwrap());
    assert!(host.declarations(&draft).connections.is_empty());
}

#[test]
fn a_view_says_what_a_node_settles_and_a_generic_type_is_chosen() {
    use orange_host::ReferenceKind;
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&empty()).unwrap();
    let set = host
        .add_node(&mut draft, "core/set-variable", [0.0, 0.0])
        .unwrap();
    let timer = host
        .add_node(&mut draft, "core/on-timer", [0.0, 200.0])
        .unwrap();
    let collect = host
        .add_node(&mut draft, "core/collect", [0.0, 400.0])
        .unwrap();
    let view = host.draft_view(&draft).unwrap();
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

    assert!(
        host.set_config(&mut draft, timer, &json!({"interval": "1000000000"}))
            .unwrap()
    );
    let view = host.draft_view(&draft).unwrap();
    let node = |id: NodeId| view.nodes.iter().find(|node| node.id == id).unwrap();
    assert_eq!(node(timer).settings["interval"], "1000000000");
    assert!(
        host.choose_type(&mut draft, collect, "T", Some("core/string@1"))
            .unwrap()
    );
    let view = host.draft_view(&draft).unwrap();
    let param = &view
        .nodes
        .iter()
        .find(|node| node.id == collect)
        .unwrap()
        .types[0];
    assert_eq!(param.chosen.as_deref(), Some("core/string@1"));
    assert!(host.choose_type(&mut draft, collect, "T", None).unwrap());
    let view = host.draft_view(&draft).unwrap();
    let param = &view
        .nodes
        .iter()
        .find(|node| node.id == collect)
        .unwrap()
        .types[0];
    assert_eq!(param.chosen, None);
    let _ = same_problems(&host, &mut draft);
}

#[test]
fn copied_nodes_paste_with_their_wire_under_new_ids_and_keep_their_variable() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let (mut draft, trigger, set) = built(&host);
    let clip = host.copy(&draft, &[trigger, set]).unwrap();
    let fresh = host.paste(&mut draft, &clip, [1000.0, 500.0]).unwrap();
    assert_eq!(fresh.len(), 2);
    assert!(!fresh.contains(&trigger) && !fresh.contains(&set));
    let view = host.draft_view(&draft).unwrap();
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
    assert_eq!(host.declarations(&draft).variables.len(), 1);
    assert_eq!(same_problems(&host, &mut draft), 0);
    // A paste is one edit.
    assert!(draft.undo());
    assert_eq!(host.draft_view(&draft).unwrap().nodes.len(), 2);
    assert!(host.paste(&mut draft, "some text", [0.0, 0.0]).is_err());
}

#[test]
fn a_refused_wire_keeps_what_could_be_done_again() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let (mut draft, _trigger, set) = built(&host);
    let concat = host
        .add_node(&mut draft, "org.orange.text/concat", [0.0, 200.0])
        .unwrap();
    let joined = output(&host, &draft, concat);
    assert!(
        host.set_input(&mut draft, (set, "value"), Some("9"))
            .unwrap()
    );
    assert!(draft.undo());
    assert_eq!(draft.can_undo(), (true, true));
    match host.connect(&mut draft, (concat, &joined), (set, "value")) {
        Err(HostError::Invalid(_)) => {}
        other => panic!("the wire was not refused: {other:?}"),
    }
    // The refused wire left the undone value to be done again.
    assert_eq!(draft.can_undo(), (true, true));
    assert!(draft.redo());
    let view = host.draft_view(&draft).unwrap();
    let node = view.nodes.iter().find(|node| node.id == set).unwrap();
    let value = node.ports.iter().find(|port| port.id == "value").unwrap();
    assert_eq!(value.value.as_deref(), Some("9"));
}

#[test]
fn edits_between_checks_are_followed_without_starting_over() {
    use orange_host::SessionWork;
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let (mut draft, _trigger, set) = built(&host);
    let _ = host.problems(&mut draft).unwrap();
    // Several edits before the next check, as the editor makes a move then an edit.
    assert!(draft.place(&[(set, [500.0, 80.0])]).unwrap());
    assert!(
        host.set_input(&mut draft, (set, "value"), Some("9"))
            .unwrap()
    );
    assert!(draft.undo());
    assert!(draft.redo());
    let work = draft.analysis_work().unwrap();
    assert!(!matches!(work.work, SessionWork::Rebuild(_)), "{work:?}");
    assert_eq!(same_problems(&host, &mut draft), 0);
}
