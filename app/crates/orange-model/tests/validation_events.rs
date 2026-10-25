mod common;

use common::{minimal_document, nid, node_id};
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{NodeKind, OutputSource, PortDirection, Suspends};
use orange_model::validation::{
    DiagnosticCode, FunctionEvidence, HostOrigin, LeafEffects, NodeValidation, analyze_functions,
    validate_nodes,
};
use orange_model::{GraphDocument, from_json};
use serde_json::{Value, json};

fn origin() -> HostOrigin {
    HostOrigin::new("0.1.0", format!("sha256:{}", "a".repeat(64))).unwrap()
}

fn event(n: u8, payload: &str) -> Value {
    json!({"id":node_id(n),"name":"e","payload":payload,"scope":"run"})
}

fn policy() -> Value {
    json!({"mode":"queue","capacity":8,"overflow":"drop-oldest","on_fault":"continue","order":0})
}

fn event_node(n: u8, definition: &str, event: Value) -> Value {
    json!({"id":node_id(n),"definition_id":definition,"definition_version":1,
        "config_version":1,"config":{"event":event}})
}

fn entry(n: u8, event: Value, activation: Option<Value>) -> Value {
    let mut node = event_node(n, "core/on-event", event);
    if let Some(activation) = activation {
        node["activation"] = activation;
    }
    node
}

fn doc(nodes: Value, events: Value, functions: Value) -> GraphDocument {
    from_json(&minimal_document(
        json!({"nodes":nodes,"events":events,"functions":functions}),
    ))
    .unwrap()
}

fn validate(doc: &GraphDocument) -> NodeValidation {
    validate_nodes(doc, &RegistrySnapshot::core(origin()))
}

fn codes(report: &NodeValidation) -> Vec<DiagnosticCode> {
    report.diagnostics.iter().map(|d| d.code).collect()
}

#[test]
fn event_nodes_project_ports_from_the_declaration() {
    let id = json!(node_id(10));
    let document = doc(
        json!([
            entry(1, id.clone(), Some(policy())),
            event_node(2, "core/emit", id.clone()),
            event_node(3, "core/await-event", id)
        ]),
        json!([event(10, "core/string@1")]),
        json!([]),
    );
    let report = validate(&document);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let on = &report.resolved[&nid(1)];
    assert_eq!(on.kind, NodeKind::Entry);
    assert_eq!(on.ports[0].source, Some(OutputSource::Payload));
    assert_eq!(on.ports[0].ty.as_ref().unwrap().as_str(), "core/string@1");
    let emit = &report.resolved[&nid(2)];
    assert_eq!(emit.suspends, Some(Suspends::Never));
    assert_eq!(emit.ports[0].direction, PortDirection::Input);
    let wait = &report.resolved[&nid(3)];
    assert_eq!(wait.kind, NodeKind::Async);
    assert!(wait.ports.iter().any(|p| p.port_id.as_str() == "timed_out"));
    assert!(wait.ports.iter().any(|p| p.port_id.as_str() == "payload"));
}

#[test]
fn unknown_event_is_reported_and_incomplete() {
    for reference in [json!(node_id(99)), json!("bad"), json!(7)] {
        let document = doc(
            json!([event_node(1, "core/emit", reference)]),
            json!([]),
            json!([]),
        );
        let report = validate(&document);
        assert_eq!(codes(&report), [DiagnosticCode::EventUnknown]);
        assert!(report.incomplete_nodes.contains(&nid(1)));
    }
}

#[test]
fn activation_policy_is_checked_for_entries() {
    let id = json!(node_id(10));
    let events = json!([event(10, "core/i64@1")]);
    let bad = |edit: fn(&mut Value)| {
        let mut p = policy();
        edit(&mut p);
        p
    };
    let cases = [
        None,
        Some(bad(|p| p["capacity"] = json!(0))),
        Some(bad(|p| p["max_parallel"] = json!(2))),
        Some(bad(|p| p["mode"] = json!("parallel"))),
        Some(bad(|p| {
            p["mode"] = json!("parallel");
            p["max_parallel"] = json!(0)
        })),
        Some(bad(|p| p["overflow"] = json!("backpressure"))),
    ];
    for activation in cases {
        let document = doc(
            json!([entry(1, id.clone(), activation.clone())]),
            events.clone(),
            json!([]),
        );
        assert_eq!(
            codes(&validate(&document)),
            [DiagnosticCode::ActivationPolicy],
            "{activation:?}"
        );
    }
    let mut parallel = policy();
    parallel["mode"] = json!("parallel");
    parallel["max_parallel"] = json!(4);
    let ok = doc(
        json!([entry(1, id.clone(), Some(parallel))]),
        events.clone(),
        json!([]),
    );
    assert!(validate(&ok).diagnostics.is_empty());

    let mut emit = event_node(1, "core/emit", id);
    emit["activation"] = policy();
    let stray = doc(json!([emit]), events, json!([]));
    assert_eq!(codes(&validate(&stray)), [DiagnosticCode::ActivationPolicy]);
}

#[test]
fn event_entries_in_functions_are_rejected_but_emit_and_await_are_allowed() {
    let id = json!(node_id(10));
    let function = |nodes: Value| {
        let kind = if nodes.to_string().contains("await-event") {
            "async"
        } else {
            "action"
        };
        json!([{"id":node_id(20),"name":"f","kind":kind,"nodes":nodes,"edges":[]}])
    };
    let events = json!([event(10, "core/i64@1")]);
    let analyze = |nodes: Value| {
        let document = doc(json!([]), events.clone(), function(nodes));
        let v = validate(&document);

        let resolved = v.resolved;
        let effects = resolved
            .keys()
            .map(|k| (*k, LeafEffects::Known(vec![])))
            .collect();
        analyze_functions(
            &document,
            &FunctionEvidence::new(&document, resolved, effects),
            &RegistrySnapshot::core(origin()),
        )
    };
    let report = analyze(json!([entry(1, id.clone(), Some(policy()))]));

    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::EntryInFunction)
    );
    let report = analyze(json!([event_node(2, "core/emit", id.clone())]));
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let report = analyze(json!([event_node(3, "core/await-event", id)]));
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let report = analyze(json!([event_node(4, "core/emit", json!(node_id(99)))]));
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::EventUnknown)
    );
}
