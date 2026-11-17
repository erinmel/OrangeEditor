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
