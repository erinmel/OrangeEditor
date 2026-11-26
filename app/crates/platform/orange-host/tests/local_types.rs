//! A graph with its own enum, no package needed (12 §6.5): it checks, runs, and is
//! edited through the host like any declaration.
use orange_host::{Host, HostOptions, StopMode, TypeShape, Value, VariantDecl};
use orange_testkit::{core, edge, id, node, until, variable};
use serde_json::json;

const MOOD: &str = "6c0e4a52-8b1f-4a5e-9c3d-1e2f3a4b5c6d";
const FEELING: u32 = 100;

fn mood() -> String {
    format!("local/t{}@1", MOOD.replace('-', ""))
}

/// A run variable of the graph's own `Mood` (calm, happy) that a manual trigger sets to
/// `happy`.
fn graph() -> String {
    let mut set = core(2, "set-variable");
    set["config"] = json!({"variable": id(FEELING)});
    set["inputs"] = json!({"value": {"type": mood(), "value": {"variant": "happy"}}});
    json!({
        "format": "orange.graph", "format_version": 1,
        "graph_id": "20000000-0000-4000-8000-0000000000e1", "revision": 0,
        "package_lock": [], "type_lock": [], "events": [], "signatures": {},
        "functions": [], "exports": [], "editor": {},
        "types": [{"id": MOOD, "name": "Mood", "kind": "enum", "variants": [
            {"id": "calm", "name": "Calm"}, {"id": "happy", "name": "Happy"}]}],
        "variables": [{"id": id(FEELING), "name": "feeling", "type": mood(), "scope": "run",
            "init": {"type": mood(), "value": {"variant": "calm"}}}],
        "nodes": [core(1, "on-manual"), set],
        "edges": [edge(1, 1, "then", 2, "in")]
    })
    .to_string()
}

#[test]
fn a_graph_with_its_own_enum_checks_and_runs() {
    let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
    let text = graph();
    assert_eq!(host.check(&text).unwrap(), []);

    let flow = host.open(&text).unwrap();
    let feeling = || {
        host.inspect(flow, |run| run.variable(variable(FEELING)).cloned())
            .unwrap()
    };
    until("the start value", || {
        feeling()
            == Some(Value::Enum {
                tag: 0,
                payload: None,
            })
    });
    host.trigger(flow, node(1), None).unwrap();
    until("the trigger", || {
        feeling()
            == Some(Value::Enum {
                tag: 1,
                payload: None,
            })
    });
    host.stop(flow, StopMode::Graceful).unwrap();
}

#[test]
fn editing_the_type_through_a_draft_keeps_the_graph_valid_until_a_used_variant_goes() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&graph()).unwrap();
    assert_eq!(host.problems(&mut draft).unwrap(), []);
    let id = MOOD.parse().unwrap();
    let mut mood = draft.document().types()[&id].clone();

    // Renaming the type and a variant, and adding one, changes no saved value.
    mood.name = "Feeling".into();
    let TypeShape::Enum { variants } = &mut mood.shape else {
        unreachable!("Mood is an enum")
    };
    variants[1].name = "Glad".into();
    variants.push(VariantDecl {
        id: "tired".into(),
        name: "Tired".into(),
        payload: None,
    });
    assert!(host.declare_type(&mut draft, mood.clone()).unwrap());
    assert_eq!(host.problems(&mut draft).unwrap(), []);

    // Taking away `happy`, which the Set node writes, is a problem on that node.
    let mut without = mood;
    if let TypeShape::Enum { variants } = &mut without.shape {
        let _ = variants.remove(1);
    }
    assert!(host.declare_type(&mut draft, without).unwrap());
    let problems = host.problems(&mut draft).unwrap();
    assert!(
        problems
            .iter()
            .any(|problem| problem.subject.node == Some(node(2)))
    );

    // Undo brings `happy` back.
    assert!(draft.undo());
    assert_eq!(host.problems(&mut draft).unwrap(), []);

    // A new type gets an ID nothing else uses.
    let fresh = host.new_type_id(&draft).unwrap();
    assert_ne!(fresh, id);
}

#[test]
fn an_enum_input_offers_its_choices_by_name_and_shows_the_one_written() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&graph()).unwrap();
    let choices = host.enum_choices(&draft, &mood()).unwrap();
    let names: Vec<_> = choices.iter().map(|choice| choice.name.as_str()).collect();
    assert_eq!(names, ["Calm", "Happy"]);
    assert!(host.enum_choices(&draft, "core/i64@1").is_none());
    assert_eq!(shown_value(&host, &draft).as_deref(), Some("Happy"));
    host.set_input(&mut draft, (node(2), "value"), Some(&choices[0].literal))
        .unwrap();
    assert_eq!(shown_value(&host, &draft).as_deref(), Some("Calm"));
    assert_eq!(host.problems(&mut draft).unwrap(), []);
}

#[test]
fn a_switch_adds_named_enum_cases_without_replacing_saved_wires() {
    let (host, _) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&graph()).unwrap();
    let switch = host
        .add_node(&mut draft, "core/switch", [0.0, 0.0])
        .unwrap();
    host.choose_type(&mut draft, switch, "K", Some(&mood()))
        .unwrap();
    let choices = host.enum_choices(&draft, &mood()).unwrap();
    host.set_input(&mut draft, (switch, "key"), Some(&choices[0].literal))
        .unwrap();
    assert!(host.add_enum_cases(&mut draft, switch).unwrap());
    assert!(!host.add_enum_cases(&mut draft, switch).unwrap());
    let view = host.draft_view(&draft).unwrap();
    let card = view.nodes.iter().find(|node| node.id == switch).unwrap();
    let calm = card
        .ports
        .iter()
        .find(|port| port.label.as_deref() == Some("Calm"))
        .unwrap()
        .id
        .clone();
    assert!(
        card.ports
            .iter()
            .any(|port| port.label.as_deref() == Some("Happy"))
    );
    host.connect(&mut draft, (switch, &calm), (node(2), "in"))
        .unwrap();
    let mood_id = MOOD.parse::<orange_host::TypeDeclId>().unwrap();
    let mut declaration = draft.document().types()[&mood_id].clone();
    let TypeShape::Enum { variants } = &mut declaration.shape else {
        unreachable!()
    };
    variants[0].name = "Relaxed".into();
    variants.push(VariantDecl {
        id: "tired".into(),
        name: "Tired".into(),
        payload: None,
    });
    host.declare_type(&mut draft, declaration).unwrap();
    assert!(host.add_enum_cases(&mut draft, switch).unwrap());
    let view = host.draft_view(&draft).unwrap();
    let card = view.nodes.iter().find(|node| node.id == switch).unwrap();
    assert!(
        card.ports.iter().any(|port| port.id == calm
            && port.connected
            && port.label.as_deref() == Some("Relaxed"))
    );
    assert!(
        card.ports
            .iter()
            .any(|port| port.label.as_deref() == Some("Tired"))
    );
    assert!(draft.undo());
    assert_eq!(
        draft.document().node(switch).unwrap().config["cases"]
            .as_object()
            .unwrap()
            .len(),
        2
    );
}

/// What node 2's `value` input shows on its card.
fn shown_value(host: &Host, draft: &orange_host::Draft) -> Option<String> {
    let graph = host.draft_view(draft).unwrap();
    let set = graph
        .nodes
        .iter()
        .find(|shown| shown.id == node(2))
        .unwrap();
    set.ports
        .iter()
        .find(|port| port.id == "value")
        .unwrap()
        .value
        .clone()
}

/// A variable's starting value reads by its choice's name, and declaring the variable
/// again with what Declarations shows (a rename keeps it) or with another name works.
#[test]
fn an_enum_variable_starts_at_a_choice_given_by_name() {
    let (host, _events) = Host::new(HostOptions::default()).unwrap();
    let mut draft = host.draft(&graph()).unwrap();
    let feeling = host.declarations(&draft).variables.remove(0);
    assert_eq!(feeling.initial.as_deref(), Some("Calm"));
    let renamed = orange_host::Variable {
        name: "mood".into(),
        ..feeling.clone()
    };
    host.declare_variable(&mut draft, &renamed).unwrap();
    let happy = orange_host::Variable {
        initial: Some("Happy".into()),
        ..renamed
    };
    host.declare_variable(&mut draft, &happy).unwrap();
    let declared = host.declarations(&draft).variables.remove(0);
    assert_eq!(
        (declared.name.as_str(), declared.initial.as_deref()),
        ("mood", Some("Happy"))
    );
}
