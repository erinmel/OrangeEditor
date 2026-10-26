use orange_model::GraphDocument;
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::NodeKind;
use orange_model::validation::{DiagnosticCode, HostOrigin, ValidationReport, validate};
use serde_json::{Value, json};

const CALL: &str = "10000000-0000-4000-8000-000000000001";
const F: &str = "30000000-0000-4000-8000-000000000001";
const G: &str = "30000000-0000-4000-8000-000000000002";
const ENTRY: &str = "40000000-0000-4000-8000-000000000001";
const OUT: &str = "40000000-0000-4000-8000-000000000002";
const RET: &str = "40000000-0000-4000-8000-000000000003";
const INNER_CALL: &str = "40000000-0000-4000-8000-000000000004";

fn marker(id: &str, definition: &str) -> Value {
    json!({"id":id,"definition_id":definition,"definition_version":1,"config_version":1,"config":{}})
}

fn call(id: &str, function: &str) -> Value {
    json!({"id":id,"definition_id":"core/call","definition_version":1,"config_version":1,
        "config":{"function":function}})
}

fn edge(n: u8, from: &str, from_port: &str, to: &str, to_port: &str) -> Value {
    json!({"id":format!("20000000-0000-4000-8000-{n:012}"),
        "from":{"node":from,"port":from_port},"to":{"node":to,"port":to_port}})
}

fn port(id: &str, direction: &str, ty: &str) -> Value {
    json!({"port_id":id,"direction":direction,"kind":"data","type":ty})
}

fn function(id: &str, kind: &str, nodes: Vec<Value>, edges: Vec<Value>) -> Value {
    json!({"id":id,"name":"f","kind":kind,"nodes":nodes,"edges":edges,
        "inputs":[port("x","input","core/i64@1")],"outputs":[port("y","output","core/i64@1")]})
}

fn run(functions: Vec<Value>) -> ValidationReport {
    let document: GraphDocument = serde_json::from_value(json!({
        "format_version":1,"graph_id":"50000000-0000-4000-8000-000000000001","revision":0,
        "nodes":[call(CALL, F)],"functions":functions}))
    .unwrap();
    let origin = HostOrigin::new("1.0.0", format!("sha256:{}", "a".repeat(64))).unwrap();
    validate(&document, &RegistrySnapshot::core(origin))
}

fn codes(report: &ValidationReport) -> Vec<DiagnosticCode> {
    report.diagnostics.iter().map(|item| item.code).collect()
}

fn pure_identity(id: &str) -> Value {
    function(
        id,
        "pure",
        vec![
            marker(ENTRY, "core/function-entry"),
            marker(OUT, "core/function-outputs"),
        ],
        vec![edge(1, ENTRY, "x", OUT, "y")],
    )
}

#[test]
fn conformant_pure_function_resolves_its_call_from_the_summary() {
    let report = run(vec![pure_identity(F)]);
    let snapshot = &report.resolved[&CALL.parse().unwrap()];
    assert_eq!(snapshot.kind, NodeKind::Pure);
    let ids: Vec<_> = snapshot.ports.iter().map(|p| p.port_id.as_str()).collect();
    assert_eq!(ids, ["x", "y"]);
    assert!(!codes(&report).contains(&DiagnosticCode::DependencyUnavailable));
    assert!(
        !codes(&report).contains(&DiagnosticCode::FunctionMetadataUnavailable),
        "{:?}",
        report.diagnostics
    );
    // The unconnected call input is now checked like any resolved node's.
    assert!(codes(&report).contains(&DiagnosticCode::InputRequired));
}

#[test]
fn unwired_output_keeps_the_call_unresolved() {
    let mut f = pure_identity(F);
    f["edges"] = json!([]);
    let report = run(vec![f]);
    assert!(codes(&report).contains(&DiagnosticCode::InputRequired));
    assert!(!report.resolved.contains_key(&CALL.parse().unwrap()));
    assert!(codes(&report).contains(&DiagnosticCode::DependencyUnavailable));
}

#[test]
fn marker_kind_must_match_the_inferred_function_kind() {
    let mut f = pure_identity(F);
    f["nodes"]
        .as_array_mut()
        .unwrap()
        .push(marker(RET, "core/return"));
    let report = run(vec![f]);
    assert!(codes(&report).contains(&DiagnosticCode::FunctionBodyMarker));
    assert!(!report.resolved.contains_key(&CALL.parse().unwrap()));
}

#[test]
fn mistyped_marker_edges_are_connection_errors_and_block_the_call() {
    let mut f = pure_identity(F);
    f["outputs"] = json!([port("y", "output", "core/f64@1")]);
    let report = run(vec![f]);
    assert!(codes(&report).contains(&DiagnosticCode::TypeMismatch));
    assert!(!report.resolved.contains_key(&CALL.parse().unwrap()));
}

#[test]
fn recursive_and_generic_functions_never_back_a_call() {
    let mut recursive = pure_identity(F);
    recursive["nodes"]
        .as_array_mut()
        .unwrap()
        .push(call(INNER_CALL, F));
    let report = run(vec![recursive]);
    assert!(codes(&report).contains(&DiagnosticCode::Recursion));
    assert!(!report.resolved.contains_key(&CALL.parse().unwrap()));
}

fn generic_identity(output_ty: &str) -> Value {
    let mut generic = pure_identity(F);
    generic["type_params"] = json!([{"name":"T"}]);
    generic["inputs"] = json!([port("x", "input", "$T")]);
    generic["outputs"] = json!([port("y", "output", output_ty)]);
    generic
}

fn run_pinned(functions: Vec<Value>, pin: Option<&str>) -> ValidationReport {
    let mut node = call(CALL, F);
    if let Some(pin) = pin {
        node["type_pins"] = json!({"T": pin});
    }
    let document: GraphDocument = serde_json::from_value(json!({
        "format_version":1,"graph_id":"50000000-0000-4000-8000-000000000001","revision":0,
        "nodes":[node],"functions":functions}))
    .unwrap();
    let origin = HostOrigin::new("1.0.0", format!("sha256:{}", "a".repeat(64))).unwrap();
    validate(&document, &RegistrySnapshot::core(origin))
}

#[test]
fn generic_functions_treat_their_parameters_as_rigid_and_calls_instantiate_them() {
    let report = run_pinned(vec![generic_identity("$T")], Some("core/f64@1"));
    let snapshot = &report.resolved[&CALL.parse().unwrap()];
    assert_eq!(snapshot.kind, NodeKind::Pure);
    assert!(
        snapshot
            .ports
            .iter()
            .all(|p| p.ty.as_ref().unwrap().as_str() == "core/f64@1")
    );
    assert!(
        !codes(&report).contains(&DiagnosticCode::TypeMismatch),
        "{:?}",
        report.diagnostics
    );

    // A body that returns a different type than `$T` is a mismatch, not an instantiation.
    let report = run_pinned(vec![generic_identity("core/i64@1")], Some("core/i64@1"));
    assert!(codes(&report).contains(&DiagnosticCode::TypeMismatch));
    assert!(!report.resolved.contains_key(&CALL.parse().unwrap()));

    // Without evidence the call's own `$T` stays ambiguous.
    let report = run_pinned(vec![generic_identity("$T")], None);
    assert!(codes(&report).contains(&DiagnosticCode::TypeAmbiguous));
}

#[test]
fn nested_generic_calls_infer_from_declaration_owned_markers() {
    let mut outer = generic_identity("$T");
    outer["nodes"]
        .as_array_mut()
        .unwrap()
        .push(call(INNER_CALL, G));
    outer["edges"] = json!([
        edge(1, ENTRY, "x", INNER_CALL, "x"),
        edge(2, INNER_CALL, "y", OUT, "y")
    ]);
    let mut inner = generic_identity("$T");
    inner["id"] = json!(G);
    let input = "40000000-0000-4000-8000-000000000011";
    let output = "40000000-0000-4000-8000-000000000012";
    inner["nodes"] = json!([
        marker(input, "core/function-entry"),
        marker(output, "core/function-outputs")
    ]);
    inner["edges"] = json!([edge(3, input, "x", output, "y")]);
    let report = run_pinned(vec![outer, inner], Some("core/i64@1"));
    assert_eq!(
        report.resolved[&INNER_CALL.parse().unwrap()].ports[0]
            .ty
            .as_ref()
            .unwrap()
            .as_str(),
        "$T"
    );
    assert_eq!(
        report.resolved[&CALL.parse().unwrap()].ports[0]
            .ty
            .as_ref()
            .unwrap()
            .as_str(),
        "core/i64@1"
    );
    assert!(
        !codes(&report).iter().any(|c| matches!(
            c,
            DiagnosticCode::TypeMismatch
                | DiagnosticCode::TypeAmbiguous
                | DiagnosticCode::FunctionMetadataUnavailable
        )),
        "{:?}",
        report.diagnostics
    );
}

#[test]
fn malformed_markers_do_not_supply_generic_inference_evidence() {
    let mut f = generic_identity("$T");
    f["nodes"].as_array_mut().unwrap().push(call(INNER_CALL, G));
    f["edges"] = json!([
        edge(1, ENTRY, "x", INNER_CALL, "x"),
        edge(2, INNER_CALL, "y", OUT, "y")
    ]);
    f["nodes"][0]["definition_version"] = json!(99);
    let mut target = generic_identity("$T");
    target["id"] = json!(G);
    let input = "40000000-0000-4000-8000-000000000011";
    let output = "40000000-0000-4000-8000-000000000012";
    target["nodes"] = json!([
        marker(input, "core/function-entry"),
        marker(output, "core/function-outputs")
    ]);
    target["edges"] = json!([edge(3, input, "x", output, "y")]);
    let report = run_pinned(vec![f, target], Some("core/i64@1"));
    assert!(!report.resolved.contains_key(&CALL.parse().unwrap()));
    assert!(!report.resolved.contains_key(&ENTRY.parse().unwrap()));
}

const SET: &str = "40000000-0000-4000-8000-000000000005";
const VAR: &str = "60000000-0000-4000-8000-000000000001";

fn action(connect_set_then: bool) -> Value {
    let mut edges = vec![
        edge(1, ENTRY, "then", SET, "in"),
        edge(2, ENTRY, "x", SET, "value"),
        edge(3, ENTRY, "x", RET, "y"),
    ];
    if connect_set_then {
        edges.push(edge(4, SET, "then", RET, "in"));
    }
    function(
        F,
        "action",
        vec![
            marker(ENTRY, "core/function-entry"),
            marker(RET, "core/return"),
            json!({"id":SET,"definition_id":"core/set-variable","definition_version":1,
                "config_version":1,"config":{"variable":VAR}}),
        ],
        edges,
    )
}

fn run_with_variable(functions: Vec<Value>) -> ValidationReport {
    let document: GraphDocument = serde_json::from_value(json!({
        "format_version":1,"graph_id":"50000000-0000-4000-8000-000000000001","revision":0,
        "nodes":[call(CALL, F)],"functions":functions,
        "variables":[{"id":VAR,"name":"v","type":"core/i64@1","scope":"run"}]}))
    .unwrap();
    let origin = HostOrigin::new("1.0.0", format!("sha256:{}", "a".repeat(64))).unwrap();
    validate(&document, &RegistrySnapshot::core(origin))
}

#[test]
fn action_calls_resolve_only_when_every_path_reaches_a_return() {
    let report = run_with_variable(vec![action(true)]);
    assert_eq!(
        report.resolved[&CALL.parse().unwrap()].kind,
        NodeKind::Action,
        "{:?}",
        report.diagnostics
    );
    assert!(!codes(&report).contains(&DiagnosticCode::MissingReturn));

    let report = run_with_variable(vec![action(false)]);
    let missing: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|item| item.code == DiagnosticCode::MissingReturn)
        .collect();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].subject.node, Some(SET.parse().unwrap()));
    assert!(!report.resolved.contains_key(&CALL.parse().unwrap()));
}

#[test]
fn nested_callees_are_accepted_before_their_callers() {
    // G is accepted first, so F's nested call resolves and reports its own unwired input
    // instead of an unresolved callee; that error then keeps F itself from being accepted.
    let mut outer = pure_identity(F);
    outer["nodes"]
        .as_array_mut()
        .unwrap()
        .push(call(INNER_CALL, G));
    let mut callee = pure_identity(G);
    callee["nodes"] = json!([
        marker(
            "40000000-0000-4000-8000-000000000011",
            "core/function-entry"
        ),
        marker(
            "40000000-0000-4000-8000-000000000012",
            "core/function-outputs"
        ),
    ]);
    callee["edges"] = json!([edge(
        9,
        "40000000-0000-4000-8000-000000000011",
        "x",
        "40000000-0000-4000-8000-000000000012",
        "y"
    )]);
    let report = run(vec![outer, callee]);
    let inner: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|item| item.subject.node == Some(INNER_CALL.parse().unwrap()))
        .map(|item| item.code)
        .collect();
    assert!(inner.contains(&DiagnosticCode::InputRequired), "{inner:?}");
    assert!(!inner.contains(&DiagnosticCode::DependencyUnavailable));
    assert!(report.resolved.contains_key(&INNER_CALL.parse().unwrap()));
    assert!(!report.resolved.contains_key(&CALL.parse().unwrap()));
}
