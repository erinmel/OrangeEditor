use std::collections::BTreeMap;
use std::sync::Arc;

use orange_model::catalog::{RegistryBuilder, RegistrySnapshot};
use orange_model::document::SignatureSnapshot;
use orange_model::from_json;
use orange_model::id::NodeId;
use orange_model::types::TypeRegistryBuilder;
use orange_model::validation::{Diagnostic, DiagnosticCode, HostOrigin, Severity, validate_inputs};
use serde_json::{Value, json};

const NODE: &str = "10000000-0000-4000-8000-000000000001";
const SIGNATURE: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000000";

fn origin() -> HostOrigin {
    HostOrigin::new("1.0.0", format!("sha256:{}", "a".repeat(64))).unwrap()
}

fn document(inputs: Value, variadic: Value, edges: Value) -> orange_model::GraphDocument {
    from_json(
        &json!({
            "format":"orange.graph", "format_version":1,
            "graph_id":"10000000-0000-4000-8000-000000000099", "revision":0,
            "nodes":[{
                "id":NODE, "definition_id":"org.example/node", "definition_version":1,
                "config_version":1, "config":{}, "signature":SIGNATURE,
                "inputs":inputs, "variadic":variadic
            }],
            "edges":edges, "signatures":{}
        })
        .to_string(),
    )
    .unwrap()
}

fn signature(ports: Value) -> SignatureSnapshot {
    serde_json::from_value(json!({"kind":"pure","ports":ports})).unwrap()
}

fn run(doc: &orange_model::GraphDocument, snapshot: Option<SignatureSnapshot>) -> Vec<Diagnostic> {
    run_with_registry(doc, snapshot, &RegistrySnapshot::core(origin()))
}

fn run_with_registry(
    doc: &orange_model::GraphDocument,
    snapshot: Option<SignatureSnapshot>,
    registry: &RegistrySnapshot,
) -> Vec<Diagnostic> {
    let resolved: BTreeMap<NodeId, SignatureSnapshot> = snapshot
        .map(|snapshot| [(NODE.parse().unwrap(), snapshot)].into())
        .unwrap_or_default();
    validate_inputs(doc, &resolved, registry)
}

fn code_list(diagnostics: &[Diagnostic]) -> Vec<DiagnosticCode> {
    diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect()
}

#[test]
fn required_default_optional_and_connected_input_precedence() {
    let doc = document(
        json!({
            "required":{"type":"core/i64@1","value":"999999999999999999999999"}
        }),
        json!({}),
        json!([{
            "id":"20000000-0000-4000-8000-000000000001",
            "from":{"node":"10000000-0000-4000-8000-000000000002","port":"value"},
            "to":{"node":NODE,"port":"required"}
        }]),
    );
    let diagnostics = run(
        &doc,
        Some(signature(json!([
            {"port_id":"required","direction":"input","kind":"data","type":"core/i64@1"},
            {"port_id":"defaulted","direction":"input","kind":"data","type":"core/i64@1","default":{"type":"core/i64@1","value":"7"}},
            {"port_id":"optional","direction":"input","kind":"data","type":"core/option@1<core/i64@1>","optional":true}
        ]))),
    );
    assert_eq!(code_list(&diagnostics), [DiagnosticCode::LiteralShadowed]);
    assert_eq!(diagnostics[0].severity, Severity::Warning);
    assert_eq!(doc.node(NODE.parse().unwrap()).unwrap().inputs.len(), 1);
}

#[test]
fn active_literals_check_declared_type_then_payload() {
    let doc = document(
        json!({
            "wrong_type":{"type":"core/string@1","value":"text"},
            "wrong_payload":{"type":"core/i64@1","value":"not-an-integer"},
            "stream":{"type":"core/stream@1<core/i64@1>","value":[]},
            "result":{"type":"core/result@1<core/i64@1>","value":{"ok":"1"}},
            "error":{"type":"core/error@1","value":{}}
        }),
        json!({}),
        json!([]),
    );
    let diagnostics = run(
        &doc,
        Some(signature(json!([
            {"port_id":"wrong_type","direction":"input","kind":"data","type":"core/i64@1"},
            {"port_id":"wrong_payload","direction":"input","kind":"data","type":"core/i64@1"},
            {"port_id":"stream","direction":"input","kind":"data","type":"core/stream@1<core/i64@1>"},
            {"port_id":"result","direction":"input","kind":"data","type":"core/result@1<core/i64@1>"},
            {"port_id":"error","direction":"input","kind":"data","type":"core/error@1"}
        ]))),
    );
    assert_eq!(code_list(&diagnostics), [DiagnosticCode::LiteralType; 5]);
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.message_args.is_empty())
    );
}

#[test]
fn variadic_instances_follow_stored_order_and_unknown_keys_are_reported() {
    let doc = document(
        json!({
            "items#2":{"type":"core/i64@1","value":"2"},
            "items#3":{"type":"core/i64@1","value":"3"}
        }),
        json!({"items":{"order":[4,2],"next":5}}),
        json!([]),
    );
    let diagnostics = run(
        &doc,
        Some(signature(json!([
            {"port_id":"items","direction":"input","kind":"data","type":"core/i64@1","variadic":{"min":0,"max":null}}
        ]))),
    );
    assert_eq!(
        code_list(&diagnostics),
        [DiagnosticCode::PortUnknown, DiagnosticCode::InputRequired]
    );
    assert_eq!(
        diagnostics[0].subject.port.as_ref().unwrap().to_string(),
        "items#3"
    );
    assert_eq!(
        diagnostics[1].subject.port.as_ref().unwrap().to_string(),
        "items#4"
    );
}

#[test]
fn optional_flag_on_non_option_does_not_suppress_required_input() {
    let doc = document(json!({}), json!({}), json!([]));
    let diagnostics = run(
        &doc,
        Some(signature(json!([
            {"port_id":"required","direction":"input","kind":"data","type":"core/i64@1","optional":true}
        ]))),
    );
    assert_eq!(code_list(&diagnostics), [DiagnosticCode::InputRequired]);
}

#[test]
fn malformed_optional_metadata_cannot_silently_supply_none() {
    let doc = document(json!({}), json!({}), json!([]));
    let diagnostics = run(
        &doc,
        Some(signature(json!([
            {"port_id":"a_missing_type","direction":"input","kind":"data","optional":true},
            {"port_id":"b_bad_option","direction":"input","kind":"data","type":"core/option@1","optional":true}
        ]))),
    );
    assert_eq!(
        code_list(&diagnostics),
        [DiagnosticCode::LiteralType, DiagnosticCode::TypeMismatch]
    );
}

#[test]
fn unresolved_current_metadata_suppresses_dependent_input_diagnostics() {
    let doc = document(
        json!({"removed":{"type":"core/i64@1","value":"1"}}),
        json!({}),
        json!([]),
    );
    assert!(run(&doc, None).is_empty());
}

#[test]
fn handle_literals_have_their_specific_diagnostic() {
    let doc = document(
        json!({"handle":{"type":"core/handle@1<org.example/session@1>","value":{}}}),
        json!({}),
        json!([]),
    );
    let mut builder = TypeRegistryBuilder::core();
    builder
        .register_handle_kind("org.example", &"org.example/session@1".parse().unwrap())
        .unwrap();
    let registry = RegistryBuilder::core(origin())
        .types(Arc::new(builder.build().unwrap()))
        .build()
        .unwrap();
    let diagnostics = run_with_registry(
        &doc,
        Some(signature(json!([
            {"port_id":"handle","direction":"input","kind":"data","type":"core/handle@1<org.example/session@1>"}
        ]))),
        &registry,
    );
    assert_eq!(code_list(&diagnostics), [DiagnosticCode::HandleLiteral]);
}
