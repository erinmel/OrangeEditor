//! Building and changing a document through the host's edit operations alone, as an
//! editor does: one draft, edited in place, with its problems following the edits.
use orange_host::{Draft, Host, HostError, HostOptions, NEW_WHILE_BOUND, NodeId, Value};
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

/// `count` is set from a value node of `definition` placed beside the setter.
fn set_from(host: &Host, definition: &str) -> (Draft, NodeId, NodeId) {
    let (mut draft, trigger, set) = built(host);
    let value = host
        .add_node(&mut draft, definition, [160.0, 200.0])
        .unwrap();
    assert!(
        host.connect(&mut draft, (value, "result"), (set, "value"))
            .unwrap()
    );
    (draft, trigger, value)
}

/// Runs the draft, fires `trigger` with `payload` and returns `count` once it is set.
fn run_once(
    host: &mut Host,
    draft: &Draft,
    trigger: NodeId,
    payload: Option<Value>,
) -> Option<Value> {
    let flow = host.open(&draft.text().unwrap()).unwrap();
    host.trigger(flow, trigger, payload).unwrap();
    let count = |host: &Host| {
        host.inspect(flow, |run| run.variable(id(100).parse().unwrap()).cloned())
            .unwrap()
    };
    until("the variable", || count(host).is_some());
    count(host)
}

#[test]
fn value_nodes_are_in_the_palette_under_short_names_and_found_by_aliases() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let labels: Vec<_> = host
        .nodes()
        .filter(|info| {
            [
                "core/number",
                "core/whole-number",
                "core/text",
                "core/true-or-false",
                "core/value",
            ]
            .contains(&info.definition.definition.id.as_str())
        })
        .map(|info| info.label().into_owned())
        .collect();
    assert_eq!(labels, ["Float", "Text", "Bool", "Value", "Int"]);
    let int = host
        .nodes()
        .find(|info| info.definition.definition.id.as_str() == "core/whole-number")
        .unwrap();
    assert!(int.aliases().iter().any(|alias| alias == "integer"));
}

#[test]
fn a_typed_whole_number_is_wired_into_a_variable_and_runs() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    let (mut draft, trigger, value) = set_from(&host, "core/whole-number");
    // Untouched, the node holds zero.
    assert_eq!(same_problems(&host, &mut draft), 0);
    assert_eq!(
        run_once(&mut host, &draft, trigger, None),
        Some(Value::I64(0))
    );
    assert!(
        host.set_input(&mut draft, (value, "value"), Some("42"))
            .unwrap()
    );
    assert_eq!(same_problems(&host, &mut draft), 0);
    let shown = host.draft_view(&draft).unwrap();
    let node = shown.nodes.iter().find(|node| node.id == value).unwrap();
    let port = node.ports.iter().find(|port| port.id == "value").unwrap();
    assert_eq!(port.value.as_deref(), Some("42"));
    assert_eq!(
        run_once(&mut host, &draft, trigger, None),
        Some(Value::I64(42))
    );
    // A typed value that is not a whole number is refused and changes nothing.
    assert!(
        host.set_input(&mut draft, (value, "value"), Some("4.5"))
            .is_err()
    );
}

#[test]
fn a_generic_value_holds_any_type_once_it_is_chosen() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    let (mut draft, trigger, value) = set_from(&host, "core/value");
    // The wire alone settles the type; the value still has to be typed.
    assert!(same_problems(&host, &mut draft) > 0);
    assert!(
        host.choose_type(&mut draft, value, "T", Some("core/i64@1"))
            .unwrap()
    );
    assert!(
        host.set_input(&mut draft, (value, "value"), Some("-3"))
            .unwrap()
    );
    assert_eq!(same_problems(&host, &mut draft), 0);
    assert_eq!(
        run_once(&mut host, &draft, trigger, None),
        Some(Value::I64(-3))
    );
}

#[test]
fn a_value_node_of_the_wrong_type_cannot_be_wired() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let (mut draft, _trigger, set) = built(&host);
    for definition in ["core/number", "core/text", "core/true-or-false"] {
        let value = host
            .add_node(&mut draft, definition, [160.0, 200.0])
            .unwrap();
        assert!(
            host.connect(&mut draft, (value, "result"), (set, "value"))
                .is_err(),
            "{definition}"
        );
    }
}

#[test]
fn a_wired_value_node_passes_on_what_reaches_it() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    let (mut draft, trigger, value) = set_from(&host, "core/whole-number");
    assert!(
        host.set_config(&mut draft, trigger, &json!({"payload": "core/i64@1"}))
            .unwrap()
    );
    assert!(
        host.connect(&mut draft, (trigger, "payload"), (value, "value"))
            .unwrap()
    );
    assert_eq!(same_problems(&host, &mut draft), 0);
    assert_eq!(
        run_once(&mut host, &draft, trigger, Some(Value::I64(9))),
        Some(Value::I64(9))
    );
}

#[test]
fn loops_and_forks_are_shown_once_and_start_valid_with_ports_added_and_taken_off() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&empty()).unwrap();
    let trigger = host
        .add_node(&mut draft, "core/on-manual", [0.0, 0.0])
        .unwrap();
    let fork = host
        .add_node(&mut draft, "core/fork", [300.0, 0.0])
        .unwrap();
    let sequence = host
        .add_node(&mut draft, "core/sequence", [300.0, 200.0])
        .unwrap();
    let looped = host
        .add_node(&mut draft, "core/while", [300.0, 400.0])
        .unwrap();
    let set = host
        .add_node(&mut draft, "core/set-variable", [600.0, 0.0])
        .unwrap();
    assert!(
        host.set_config(&mut draft, set, &json!({"variable": id(100)}))
            .unwrap()
    );
    let ports = |draft: &Draft, node: NodeId| -> Vec<String> {
        let view = host.draft_view(draft).unwrap();
        let node = view.nodes.iter().find(|found| found.id == node).unwrap();
        node.ports
            .iter()
            .map(|port| format!("{}{}", if port.input { "<" } else { ">" }, port.id))
            .collect()
    };
    // A port the kind both implies and declares is shown once.
    assert_eq!(
        ports(&draft, looped),
        ["<in", "<condition", ">body", ">completed"]
    );
    // Variadic ports start with as many instances as they need.
    assert_eq!(ports(&draft, fork), ["<in", ">completed", ">branch#0"]);
    assert_eq!(ports(&draft, sequence), ["<in", ">then#0", ">then#1"]);
    let view = host.draft_view(&draft).unwrap();
    let node = view.nodes.iter().find(|found| found.id == looped).unwrap();
    assert_eq!(node.settings["max_iterations"], NEW_WHILE_BOUND);
    assert_eq!(
        node.schema.as_ref().unwrap()["required"][0],
        "max_iterations"
    );
    let set_view = view.nodes.iter().find(|found| found.id == set).unwrap();
    assert_eq!(set_view.detail.as_deref(), Some("count"));
    assert!(
        host.set_input(&mut draft, (set, "value"), Some("1"))
            .unwrap()
    );

    // Another branch is added, wired, and taken off with its wire.
    let added = host.add_port(&mut draft, fork, "branch").unwrap();
    assert_eq!(added, "branch#1");
    assert_eq!(
        ports(&draft, fork),
        ["<in", ">completed", ">branch#0", ">branch#1"]
    );
    assert!(
        host.connect(&mut draft, (trigger, "then"), (fork, "in"))
            .unwrap()
    );
    assert!(
        host.connect(&mut draft, (fork, "branch#1"), (set, "in"))
            .unwrap()
    );
    assert!(host.problems(&mut draft).unwrap().iter().all(|problem| {
        problem.subject.node != Some(fork) && problem.subject.node != Some(set)
    }));
    assert!(host.remove_port(&mut draft, (fork, "branch#1")).unwrap());
    assert_eq!(ports(&draft, fork), ["<in", ">completed", ">branch#0"]);
    assert!(host.draft_view(&draft).unwrap().wires.len() == 1);
    // The fewest a port may have stay, and a plain port is not variadic.
    assert!(host.remove_port(&mut draft, (fork, "branch#0")).is_err());
    assert!(host.add_port(&mut draft, set, "value").is_err());
    // Undo takes back one port at a time.
    assert!(draft.undo());
    assert_eq!(ports(&draft, fork).len(), 4);
    let _ = same_problems(&host, &mut draft);
}

#[derive(Debug)]
struct Thing;
impl orange_host::Session for Thing {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
#[derive(Debug)]
struct Things;
impl orange_host::ConnectionAdapter for Things {
    fn open(
        &self,
        _: &serde_json::Map<String, Json>,
    ) -> Result<std::sync::Arc<dyn orange_host::Session>, String> {
        Ok(std::sync::Arc::new(Thing))
    }
    fn choices(&self) -> Vec<orange_host::SettingChoices> {
        vec![orange_host::SettingChoices {
            setting: "which".into(),
            unset: "any".into(),
            values: vec!["red".into(), "blue".into()],
            labels: Vec::new(),
            problem: None,
        }]
    }
}
#[derive(Debug)]
struct Nothing;
impl orange_host::ServiceNode for Nothing {
    fn run(&self, _: &orange_host::Call<'_>) -> Result<Vec<Value>, orange_host::KernelFault> {
        Ok(Vec::new())
    }
}

#[test]
fn a_node_that_uses_a_connection_is_bound_to_one_of_its_kind_when_added() {
    use orange_sdk::Definition;
    let uses = Definition::node("acme.kit", "uses", "Uses a thing")
        .uses("acme.kit/thing")
        .build();
    let any = Definition::node("acme.kit", "any", "Uses any connection")
        .connection()
        .build();
    let package = orange_host::ServicePackage::new("acme.kit", "1.0.0")
        .adapter("acme.kit/thing", Things)
        .node(&uses, Nothing)
        .and_then(|package| package.node(&any, Nothing))
        .unwrap();
    let host = Host::with_services(HostOptions::default(), vec![package])
        .unwrap()
        .0;
    assert_eq!(
        host.adapter_choices("acme.kit/thing")[0].values,
        ["red", "blue"]
    );
    assert!(host.adapter_choices("acme.kit/none").is_empty());
    let mut draft = host.draft(&empty()).unwrap();

    // A node that does not say which kind it uses needs a connection declared first.
    let refused = host.add_node(&mut draft, "acme.kit/any", [0.0, 0.0]);
    assert!(
        matches!(&refused, Err(HostError::Document(text)) if text.contains("declare one")),
        "{refused:?}"
    );
    // One that says is given a new connection of its kind, as one edit with the node.
    let first = host
        .add_node(&mut draft, "acme.kit/uses", [0.0, 0.0])
        .unwrap();
    let declared = host.declarations(&draft).connections;
    assert_eq!(declared.len(), 1);
    assert_eq!(
        (declared[0].name.as_str(), declared[0].adapter.as_str()),
        ("thing", "acme.kit/thing")
    );
    let named = |draft: &Draft, node: NodeId| {
        let view = host.draft_view(draft).unwrap();
        view.nodes
            .iter()
            .find(|found| found.id == node)
            .unwrap()
            .settings["connection"]
            .clone()
    };
    assert_eq!(named(&draft, first), json!(declared[0].id));
    assert!(
        host.problems(&mut draft)
            .unwrap()
            .iter()
            .all(|problem| problem.subject.node != Some(first))
    );
    // The next one, and one of any kind, use that connection.
    let second = host
        .add_node(&mut draft, "acme.kit/uses", [0.0, 100.0])
        .unwrap();
    let third = host
        .add_node(&mut draft, "acme.kit/any", [0.0, 200.0])
        .unwrap();
    assert_eq!(host.declarations(&draft).connections.len(), 1);
    assert_eq!(named(&draft, second), named(&draft, third));
    assert!(draft.undo() && draft.undo() && draft.undo());
    assert!(host.declarations(&draft).connections.is_empty());
}

#[test]
fn every_suggested_kind_takes_the_wire_on_the_input_it_names() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let (draft, trigger, _set) = built(&host);
    let concat_draft = {
        let mut draft = host.draft(&empty()).unwrap();
        let concat = host
            .add_node(&mut draft, "org.orange.text/concat", [0.0, 0.0])
            .unwrap();
        (draft, concat)
    };
    let joined = output(&host, &concat_draft.0, concat_draft.1);
    for (draft, from) in [
        (&draft, (trigger, "then")),
        (&concat_draft.0, (concat_draft.1, joined.as_str())),
    ] {
        let takers = host.takers(draft, from);
        assert!(!takers.is_empty());
        for taker in takers {
            let mut trial = host.draft(&draft.text().unwrap()).unwrap();
            let Ok(node) = host.add_node(&mut trial, &taker.definition, [400.0, 0.0]) else {
                continue;
            };
            // The input is there; the language may still refuse the wire with a reason.
            match host.connect(&mut trial, from, (node, &taker.input)) {
                Ok(_) | Err(HostError::Invalid(_)) => {}
                Err(other) => panic!("{} has no input {}: {other}", taker.definition, taker.input),
            }
        }
    }
    let text = host.takers(&concat_draft.0, (concat_draft.1, &joined));
    assert!(
        text.iter()
            .any(|taker| taker.definition == "org.orange.text/concat")
    );
    assert!(text.iter().all(|taker| taker.definition != "core/sequence"));
    let steps = host.takers(&draft, (trigger, "then"));
    assert!(
        steps
            .iter()
            .any(|taker| taker.definition == "core/set-variable")
    );
}

#[test]
fn whole_numbers_compared_by_greater_run_into_a_variable() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    let text = json!({"format": "orange.graph", "format_version": 1,
        "graph_id": "20000000-0000-4000-8000-0000000000c9", "revision": 1,
        "package_lock": [], "type_lock": [], "events": [], "connections": [],
        "signatures": {}, "functions": [], "exports": [], "editor": {},
        "variables": [{"id": id(100), "name": "bigger", "type": "core/bool@1",
            "scope": "run", "persistent": false, "conflict": "reject"}],
        "nodes": [], "edges": []})
    .to_string();
    let mut draft = host.draft(&text).unwrap();
    let trigger = host
        .add_node(&mut draft, "core/on-manual", [0.0, 0.0])
        .unwrap();
    let set = host
        .add_node(&mut draft, "core/set-variable", [600.0, 0.0])
        .unwrap();
    assert!(
        host.set_config(&mut draft, set, &json!({"variable": id(100)}))
            .unwrap()
    );
    let a = host
        .add_node(&mut draft, "core/whole-number", [0.0, 200.0])
        .unwrap();
    let b = host
        .add_node(&mut draft, "core/whole-number", [0.0, 300.0])
        .unwrap();
    let greater = host
        .add_node(&mut draft, "org.orange.math/greater", [300.0, 200.0])
        .unwrap();
    assert!(host.set_input(&mut draft, (a, "value"), Some("5")).unwrap());
    assert!(host.set_input(&mut draft, (b, "value"), Some("3")).unwrap());
    // Greater is generic: wiring whole numbers into it settles its type, and its saved
    // signature must follow in the same edit, or the graph does not run.
    for (from, to) in [
        ((a, "result"), (greater, "a")),
        ((b, "result"), (greater, "b")),
        ((greater, "result"), (set, "value")),
        ((trigger, "then"), (set, "in")),
    ] {
        assert!(host.connect(&mut draft, from, to).unwrap());
    }
    assert_eq!(same_problems(&host, &mut draft), 0);
    let flow = host.open(&draft.text().unwrap()).unwrap();
    host.trigger(flow, trigger, None).unwrap();
    let value = |host: &Host| {
        host.inspect(flow, |run| run.variable(id(100).parse().unwrap()).cloned())
            .unwrap()
    };
    until("the variable", || value(&host).is_some());
    assert_eq!(value(&host), Some(Value::Bool(true)));
}

#[test]
fn two_nodes_of_one_kind_are_called_the_same_on_the_canvas() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&empty()).unwrap();
    let a = host
        .add_node(&mut draft, "core/whole-number", [0.0, 0.0])
        .unwrap();
    let b = host
        .add_node(&mut draft, "core/whole-number", [0.0, 100.0])
        .unwrap();
    let view = host.draft_view(&draft).unwrap();
    let name = |id: NodeId| view.nodes.iter().find(|n| n.id == id).unwrap().name.clone();
    assert_eq!(name(a), "Int");
    assert_eq!(name(b), "Int");
    // Text that has nothing else to tell them apart by keeps the ID ending.
    let names = orange_host::Names::of_document(&host, draft.document());
    assert_ne!(names.node(&a), names.node(&b));
    assert_eq!(names.without_ids().node(&a), "Int");
}

#[test]
fn whole_numbers_past_two_to_the_53_are_kept_exactly() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&empty()).unwrap();
    let value = host
        .add_node(&mut draft, "core/whole-number", [0.0, 0.0])
        .unwrap();
    for typed in [
        "9007199254740993",
        "9223372036854775807",
        "-9223372036854775808",
    ] {
        assert!(
            host.set_input(&mut draft, (value, "value"), Some(typed))
                .unwrap()
        );
        let view = host.draft_view(&draft).unwrap();
        let node = view.nodes.iter().find(|n| n.id == value).unwrap();
        let shown = node.ports.iter().find(|p| p.id == "value").unwrap();
        assert_eq!(shown.value.as_deref(), Some(typed));
    }
    assert!(
        host.set_input(&mut draft, (value, "value"), Some("9223372036854775808"))
            .is_err()
    );
}

#[test]
fn a_value_is_inspected_whole_with_bytes_as_a_bounded_hex_preview() {
    use orange_host::{MAX_INSPECT_BYTES, inspect};
    let long = "x".repeat(500);
    assert_eq!(
        inspect(&Value::Str(std::sync::Arc::new(long.clone()))),
        long
    );
    assert_eq!(
        inspect(&Value::I64(9_007_199_254_740_993)),
        "9007199254740993"
    );
    let bytes = Value::Bytes(std::sync::Arc::new(vec![0xab; MAX_INSPECT_BYTES + 10]));
    let shown = inspect(&bytes);
    assert!(shown.starts_with(&format!("{} bytes\nab ab", MAX_INSPECT_BYTES + 10)));
    assert!(shown.ends_with("… 10 more bytes"));
}

#[test]
fn a_node_dropped_on_a_wire_goes_into_it_as_one_edit() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let (mut draft, trigger, set) = built(&host);
    let wait = host
        .add_node(&mut draft, "core/wait", [160.0, 0.0])
        .unwrap();
    let before = draft.text().unwrap();
    assert!(
        host.insert_into_wire(&mut draft, wait, (trigger, "then"), (set, "in"))
            .unwrap()
    );
    let wires = host.draft_view(&draft).unwrap().wires;
    let has = |from: NodeId, to: NodeId| wires.iter().any(|w| w.from.0 == from && w.to.0 == to);
    assert!(has(trigger, wait) && has(wait, set) && !has(trigger, set));
    // One step back puts the wire back as it was.
    assert!(draft.undo());
    let content = |text: &str| {
        let mut saved: Json = serde_json::from_str(text).unwrap();
        saved.as_object_mut().unwrap().remove("revision");
        saved
    };
    assert_eq!(content(&draft.text().unwrap()), content(&before));
    // A node with no step input does not go into a step wire; nothing changes.
    let int = host
        .add_node(&mut draft, "core/whole-number", [160.0, 200.0])
        .unwrap();
    let held = draft.text().unwrap();
    assert!(
        host.insert_into_wire(&mut draft, int, (trigger, "then"), (set, "in"))
            .is_err()
    );
    assert_eq!(draft.text().unwrap(), held);
    // One wire can be taken off by its two ends.
    assert!(
        host.disconnect_wire(&mut draft, (trigger, "then"), (set, "in"))
            .unwrap()
    );
    assert!(host.draft_view(&draft).unwrap().wires.is_empty());
    assert!(
        !host
            .disconnect_wire(&mut draft, (trigger, "then"), (set, "in"))
            .unwrap()
    );
}

#[test]
fn dropping_a_new_kind_splices_atomically_and_previews_without_editing() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let (mut draft, trigger, set) = built(&host);
    let before = draft.text().unwrap();
    let (from, to) = ((trigger, "then"), (set, "in"));
    let card = host.placement_preview(&draft, "core/wait").unwrap();
    assert!(card.ports.iter().any(|port| port.input && port.exec));
    assert!(card.ports.iter().any(|port| !port.input && port.exec));
    assert_eq!(draft.text().unwrap(), before);
    assert!(
        host.add_node_into_wire(&mut draft, "core/whole-number", [160.0, 0.0], from, to)
            .is_err()
    );
    assert_eq!(draft.text().unwrap(), before);
    let wait = host
        .add_node_into_wire(&mut draft, "core/wait", [160.0, 0.0], from, to)
        .unwrap();
    let wires = host.draft_view(&draft).unwrap().wires;
    assert!(
        wires
            .iter()
            .any(|wire| wire.from.0 == trigger && wire.to.0 == wait)
    );
    assert!(
        wires
            .iter()
            .any(|wire| wire.from.0 == wait && wire.to.0 == set)
    );
    assert!(
        wires
            .iter()
            .all(|wire| wire.from.0 != trigger || wire.to.0 != set)
    );
    assert!(draft.undo());
    let without_revision = |text: &str| {
        let mut value: Json = serde_json::from_str(text).unwrap();
        value.as_object_mut().unwrap().remove("revision");
        value
    };
    assert_eq!(
        without_revision(&draft.text().unwrap()),
        without_revision(&before)
    );
}

#[test]
fn every_name_in_an_operators_formula_is_one_of_its_inputs() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&empty()).unwrap();
    let mut checked = 0;
    let kinds: Vec<String> = host
        .nodes()
        .map(|info| info.definition.definition.id.as_str().to_owned())
        .filter(|id| orange_host::formula(id).is_some())
        .collect();
    for kind in kinds {
        let node = host.add_node(&mut draft, &kind, [0.0, 0.0]).unwrap();
        let view = host.draft_view(&draft).unwrap();
        let card = view.nodes.iter().find(|n| n.id == node).unwrap();
        let formula = orange_host::formula(&kind).unwrap();
        for word in formula
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .filter(|word| !word.is_empty() && !["min", "max"].contains(word))
        {
            assert!(
                card.ports.iter().any(|p| p.input && p.id == word),
                "{kind}: `{word}` is not an input"
            );
        }
        checked += 1;
    }
    assert!(checked >= 10, "{checked}");
}
