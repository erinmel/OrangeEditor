//! Organising a draft: groups of nodes kept in the document's layout, and selected nodes
//! made into a graph function used through one call node, each as edits that undo.
use orange_host::{CanvasComment, Draft, Host, HostOptions, NodeId, Value};
use orange_testkit::{id, until};
use serde_json::{Value as Json, json};

/// An empty document with two whole-number variables, `first` and `second`.
fn empty() -> String {
    json!({"format": "orange.graph", "format_version": 1,
        "graph_id": "20000000-0000-4000-8000-0000000000d1", "revision": 1,
        "package_lock": [], "type_lock": [], "events": [], "connections": [],
        "signatures": {}, "functions": [], "exports": [], "editor": {},
        "variables": [
            {"id": id(100), "name": "first", "type": "core/i64@1",
                "scope": "run", "persistent": false, "conflict": "reject"},
            {"id": id(101), "name": "second", "type": "core/bool@1",
                "scope": "run", "persistent": false, "conflict": "reject"}],
        "nodes": [], "edges": []})
    .to_string()
}

struct Built {
    draft: Draft,
    trigger: NodeId,
    set_first: NodeId,
    a: NodeId,
    b: NodeId,
    greater: NodeId,
    set_second: NodeId,
}

/// A trigger that sets `first` to 7 and then `second` to whether 5 > 3.
fn built(host: &Host) -> Built {
    let mut draft = host.draft(&empty()).unwrap();
    let mut add = |kind: &str, at: [f32; 2]| host.add_node(&mut draft, kind, at).unwrap();
    let trigger = add("core/on-manual", [0.0, 0.0]);
    let set_first = add("core/set-variable", [300.0, 0.0]);
    let set_second = add("core/set-variable", [700.0, 0.0]);
    let a = add("core/whole-number", [0.0, 200.0]);
    let b = add("core/whole-number", [0.0, 300.0]);
    let greater = add("org.orange.math/greater", [300.0, 200.0]);
    for (node, variable) in [(set_first, 100), (set_second, 101)] {
        assert!(
            host.set_config(&mut draft, node, &json!({"variable": id(variable)}))
                .unwrap()
        );
    }
    for (at, typed) in [
        ((set_first, "value"), "7"),
        ((a, "value"), "5"),
        ((b, "value"), "3"),
    ] {
        assert!(host.set_input(&mut draft, at, Some(typed)).unwrap());
    }
    for (from, to) in [
        ((a, "result"), (greater, "a")),
        ((b, "result"), (greater, "b")),
        ((greater, "result"), (set_second, "value")),
        ((trigger, "then"), (set_first, "in")),
        ((set_first, "then"), (set_second, "in")),
    ] {
        assert!(host.connect(&mut draft, from, to).unwrap());
    }
    assert!(host.problems(&mut draft).unwrap().is_empty());
    Built {
        draft,
        trigger,
        set_first,
        a,
        b,
        greater,
        set_second,
    }
}

/// Runs the draft once from `trigger` and returns both variables once `second` is set.
fn run(host: &mut Host, draft: &Draft, trigger: NodeId) -> (Option<Value>, Option<Value>) {
    let flow = host.open(&draft.text().unwrap()).unwrap();
    host.trigger(flow, trigger, None).unwrap();
    let read = |host: &Host, n: u32| {
        host.inspect(flow, move |run| {
            run.variable(id(n).parse().unwrap()).cloned()
        })
        .unwrap()
    };
    until("the second variable", || read(host, 101).is_some());
    (read(host, 100), read(host, 101))
}

/// The document without its revision, which every edit and undo counts up.
fn content(draft: &Draft) -> Json {
    let mut document = saved(draft);
    let _ = document.as_object_mut().unwrap().remove("revision");
    document
}

fn saved(draft: &Draft) -> Json {
    serde_json::from_str(&draft.text().unwrap()).unwrap()
}

#[test]
fn a_group_is_kept_in_the_layout_changed_and_taken_away_in_steps_that_undo() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let Built {
        mut draft,
        a,
        b,
        greater,
        ..
    } = built(&host);
    assert!(draft.groups().is_empty());

    let numbers = draft.group(&[a, b], "  Numbers ").unwrap();
    let groups = draft.groups();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].id, numbers);
    assert_eq!(groups[0].name, "Numbers");
    assert_eq!(groups[0].nodes, [a, b]);
    assert!(!groups[0].collapsed);
    // Saved with the document, in its layout, where the compiler never looks.
    let layout = &saved(&draft)["editor"]["groups"][&numbers];
    assert_eq!(layout["name"], "Numbers");
    assert!(layout["color"].as_str().unwrap().starts_with('#'));
    assert!(host.problems(&mut draft).unwrap().is_empty());
    // And read back when the document is opened again.
    assert_eq!(host.draft(&draft.text().unwrap()).unwrap().groups(), groups);

    // Renamed, recoloured and collapsed in one edit.
    let mut changed = groups[0].clone();
    changed.name = "Inputs".into();
    changed.color = [1, 2, 3];
    changed.collapsed = true;
    assert!(draft.set_group(&changed).unwrap());
    assert_eq!(draft.groups(), [changed.clone()]);
    assert!(!draft.set_group(&changed).unwrap());

    // A node grouped again leaves the group it was in.
    let compare = draft.group(&[b, greater], "Compare").unwrap();
    let groups = draft.groups();
    assert_eq!(groups.len(), 2);
    let find = |id: &str| groups.iter().find(|group| group.id == id).unwrap().clone();
    assert_eq!(find(&numbers).nodes, [a]);
    assert_eq!(find(&compare).nodes, [b, greater]);
    assert_ne!(find(&numbers).color, find(&compare).color);

    assert!(draft.ungroup(&compare).unwrap());
    assert!(!draft.ungroup(&compare).unwrap());
    assert_eq!(draft.groups().len(), 1);

    // Each change was one step back.
    assert!(draft.undo());
    assert_eq!(draft.groups().len(), 2);
    assert!(draft.undo());
    assert_eq!(draft.groups(), [changed]);
    assert!(draft.undo());
    assert!(draft.undo());
    assert!(draft.groups().is_empty());
    assert!(draft.redo());
    assert_eq!(draft.groups()[0].name, "Numbers");

    // A deleted node leaves its group; a group with no nodes left is not listed.
    assert!(host.remove_nodes(&mut draft, &[a, b]).unwrap());
    assert!(draft.groups().is_empty());
    assert!(draft.undo());
    assert_eq!(draft.groups()[0].nodes, [a, b]);
    assert!(draft.group(&[], "Nothing").is_err());
}

#[test]
fn old_comments_get_default_size_and_resized_comments_round_trip() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&empty()).unwrap();
    let id = draft.add_comment([12.0, 48.0], "A note").unwrap();
    let mut document = saved(&draft);
    document["editor"]["comments"][&id]
        .as_object_mut()
        .unwrap()
        .remove("size");

    let mut reopened = host.draft(&document.to_string()).unwrap();
    let mut comment = reopened.comments().next().unwrap();
    assert_eq!(comment.size, CanvasComment::DEFAULT_SIZE);
    comment.size = [320.0, 180.0];
    assert!(reopened.update_comment(&comment).unwrap());
    assert_eq!(
        saved(&reopened)["editor"]["comments"][&id]["size"],
        json!([320.0, 180.0])
    );
    assert_eq!(reopened.comments().next().unwrap().size, comment.size);

    comment.size = [f32::INFINITY, 180.0];
    assert!(reopened.update_comment(&comment).is_err());
}

#[test]
fn pure_nodes_become_a_function_with_their_inputs_and_outputs_and_still_run() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    let Built {
        mut draft,
        trigger,
        a,
        b,
        greater,
        set_second,
        ..
    } = built(&host);
    let before = content(&draft);
    assert_eq!(
        run(&mut host, &draft, trigger),
        (Some(Value::I64(7)), Some(Value::Bool(true)))
    );

    // `b` and Greater: `a` comes in from outside, the result goes out to Set.
    let preview = host.extraction(&draft, &[b, greater]).unwrap();
    assert_eq!(preview.kind, "pure");
    assert_eq!(preview.inputs, [("a".to_owned(), "core/i64@1".to_owned())]);
    assert_eq!(
        preview.outputs,
        [("result".to_owned(), "core/bool@1".to_owned())]
    );
    assert!(
        host.extract(&mut draft, &[b, greater], " ", [0.0, 0.0])
            .is_err()
    );
    let call = host
        .extract(
            &mut draft,
            &[b, greater],
            "Bigger than three",
            [300.0, 250.0],
        )
        .unwrap();
    assert!(host.problems(&mut draft).unwrap().is_empty());

    let document = saved(&draft);
    let functions = document["functions"].as_array().unwrap();
    assert_eq!(functions.len(), 1);
    assert_eq!(functions[0]["name"], "Bigger than three");
    assert_eq!(functions[0]["kind"], "pure");
    assert_eq!(functions[0]["nodes"].as_array().unwrap().len(), 4);

    let view = host.draft_view(&draft).unwrap();
    assert_eq!(view.nodes.len(), 5);
    let shown = view.nodes.iter().find(|node| node.id == call).unwrap();
    assert_eq!(shown.name, "Bigger than three");
    assert_eq!(shown.position, Some([300.0, 250.0]));
    let ports: Vec<_> = shown
        .ports
        .iter()
        .map(|port| (port.id.as_str(), port.input, port.ty.as_deref()))
        .collect();
    assert_eq!(
        ports,
        [
            ("a", true, Some("core/i64@1")),
            ("result", false, Some("core/bool@1"))
        ]
    );
    let wired = |from: (NodeId, &str), to: (NodeId, &str)| {
        view.wires
            .iter()
            .any(|wire| wire.from == (from.0, from.1.into()) && wire.to == (to.0, to.1.into()))
    };
    assert!(wired((a, "result"), (call, "a")));
    assert!(wired((call, "result"), (set_second, "value")));
    assert_eq!(
        run(&mut host, &draft, trigger),
        (Some(Value::I64(7)), Some(Value::Bool(true)))
    );

    // One edit: one undo gives back the document exactly, one redo makes it again.
    let after = content(&draft);
    assert!(draft.undo());
    assert_eq!(content(&draft), before);
    assert!(draft.redo());
    assert_eq!(content(&draft), after);
}

#[test]
fn steps_pass_through_an_action_function_and_out_of_its_return() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    let Built {
        mut draft,
        trigger,
        set_first,
        set_second,
        ..
    } = built(&host);
    let preview = host.extraction(&draft, &[set_first]).unwrap();
    assert_eq!(preview.kind, "action");
    assert!(preview.inputs.is_empty() && preview.outputs.is_empty());
    let call = host
        .extract(&mut draft, &[set_first], "Set first", [300.0, 0.0])
        .unwrap();
    assert!(host.problems(&mut draft).unwrap().is_empty());
    let document = saved(&draft);
    let kinds: Vec<&str> = document["functions"][0]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["definition_id"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"core/function-entry") && kinds.contains(&"core/return"));

    let view = host.draft_view(&draft).unwrap();
    let wired = |from: (NodeId, &str), to: (NodeId, &str)| {
        view.wires
            .iter()
            .any(|wire| wire.from == (from.0, from.1.into()) && wire.to == (to.0, to.1.into()))
    };
    assert!(wired((trigger, "then"), (call, "in")));
    assert!(wired((call, "then"), (set_second, "in")));
    assert_eq!(
        run(&mut host, &draft, trigger),
        (Some(Value::I64(7)), Some(Value::Bool(true)))
    );
}

#[test]
fn a_selection_that_cannot_be_a_function_is_refused_and_left_as_it_was() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let Built {
        mut draft,
        trigger,
        set_first,
        ..
    } = built(&host);
    let before = draft.text().unwrap();
    let (undo, redo) = draft.can_undo();

    let error = host
        .extract(&mut draft, &[trigger, set_first], "Start", [0.0, 0.0])
        .unwrap_err()
        .to_string();
    assert!(error.contains("trigger"), "{error}");
    assert!(host.extraction(&draft, &[]).is_err());
    assert!(
        host.extraction(&draft, &[id(999).parse().unwrap()])
            .is_err()
    );
    assert_eq!(draft.text().unwrap(), before);
    assert_eq!(draft.can_undo(), (undo, redo));
}
