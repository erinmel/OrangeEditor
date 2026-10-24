mod common;

use common::{minimal_document, node_id};
use orange_model::catalog::RegistrySnapshot;
use orange_model::from_json;
use orange_model::validation::{
    DiagnosticCode, FunctionEvidence, HostOrigin, LeafEffects, Severity, analyze_functions,
    validate_nodes,
};
use serde_json::{Value, json};

fn origin() -> HostOrigin {
    HostOrigin::new("0.1.0", format!("sha256:{}", "a".repeat(64))).unwrap()
}

fn fork(join: Value) -> Value {
    json!({"id":node_id(1),"definition_id":"core/fork","definition_version":1,
        "config_version":1,"config":{"join":join},
        "variadic":{"branch":{"order":[0,1],"next":2}}})
}

fn function_diagnostics(join: Value) -> Vec<(DiagnosticCode, Severity)> {
    let functions = json!([{"id":node_id(20),"name":"f","kind":"action",
        "nodes":[fork(join)],"edges":[]}]);
    let document = from_json(&minimal_document(json!({"functions":functions}))).unwrap();
    let registry = RegistrySnapshot::core(origin());
    let resolved = validate_nodes(&document, &registry).resolved;
    let effects = resolved
        .keys()
        .map(|k| (*k, LeafEffects::Known(vec![])))
        .collect();
    analyze_functions(
        &document,
        &FunctionEvidence::new(&document, resolved, effects),
        &registry,
    )
    .diagnostics
    .iter()
    .map(|d| (d.code, d.severity))
    .collect()
}

#[test]
fn documented_host_entries_are_rejected_in_functions_even_when_unregistered() {
    for definition in ["core/on-watch", "core/on-start", "core/on-variable-changed"] {
        let node = json!({"id":node_id(1),"definition_id":definition,"definition_version":1,
            "config_version":1,"config":{}});
        let functions = json!([{"id":node_id(20),"name":"f","kind":"action",
            "nodes":[node],"edges":[]}]);
        let document = from_json(&minimal_document(json!({"functions":functions}))).unwrap();
        let registry = RegistrySnapshot::core(origin());
        let resolved = validate_nodes(&document, &registry).resolved;
        let report = analyze_functions(
            &document,
            &FunctionEvidence::new(&document, resolved, Default::default()),
            &registry,
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.code == DiagnosticCode::EntryInFunction),
            "{definition}"
        );
    }
}

#[test]
fn detaching_fork_in_a_function_warns_without_blocking() {
    assert_eq!(
        function_diagnostics(json!("detach")),
        [(DiagnosticCode::DetachInFunction, Severity::Warning)]
    );
    for join in [json!("all"), json!("any"), json!("race")] {
        assert!(function_diagnostics(join).is_empty());
    }
}
