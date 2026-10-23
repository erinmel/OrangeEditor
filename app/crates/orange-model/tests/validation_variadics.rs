use std::collections::BTreeMap;

use orange_model::catalog::RegistrySnapshot;
use orange_model::document::SignatureSnapshot;
use orange_model::from_json;
use orange_model::id::NodeId;
use orange_model::validation::{Diagnostic, DiagnosticCode, HostOrigin, Subject, validate_inputs};
use serde_json::{Value, json};

const NODE: &str = "10000000-0000-4000-8000-000000000001";

fn origin() -> HostOrigin {
    HostOrigin::new("1.0.0", format!("sha256:{}", "a".repeat(64))).unwrap()
}

fn document(variadic: Value) -> orange_model::GraphDocument {
    from_json(
        &json!({
            "format":"orange.graph", "format_version":1,
            "graph_id":"10000000-0000-4000-8000-000000000099", "revision":0,
            "nodes":[{
                "id":NODE, "definition_id":"org.example/node", "definition_version":1,
                "config_version":1, "config":{}, "signature":"sha256:0000000000000000000000000000000000000000000000000000000000000000",
                "variadic":variadic
            }],
            "edges":[], "signatures":{}
        })
        .to_string(),
    )
    .unwrap()
}

fn signature(ports: Value) -> SignatureSnapshot {
    serde_json::from_value(json!({"kind":"action","ports":ports})).unwrap()
}

fn run(doc: &orange_model::GraphDocument, snapshot: SignatureSnapshot) -> Vec<Diagnostic> {
    let resolved: BTreeMap<NodeId, SignatureSnapshot> = [(NODE.parse().unwrap(), snapshot)].into();
    validate_inputs(doc, &resolved, &RegistrySnapshot::core(origin()))
}

fn port_subject() -> Subject {
    Subject::new(
        Some(NODE.parse().unwrap()),
        Some("items".parse().unwrap()),
        None,
    )
}

#[test]
fn variadic_count_uses_persisted_instance_order_and_inclusive_bounds() {
    let ports = json!([
        {"port_id":"items","direction":"input","kind":"data","type":"core/i64@1","variadic":{"min":2,"max":3}}
    ]);
    let below_document = document(json!({"items":{"order":[9],"next":10}}));
    let state_id = below_document.state_id();
    let revision = below_document.revision();
    let below = run(&below_document, signature(ports.clone()));
    let above = run(
        &document(json!({"items":{"order":[9,2,18,4],"next":19}})),
        signature(ports.clone()),
    );
    let lower_bound = run(
        &document(json!({"items":{"order":[9,2],"next":10}})),
        signature(ports.clone()),
    );
    let upper_bound = run(
        &document(json!({"items":{"order":[9,2,18],"next":19}})),
        signature(ports),
    );

    assert_eq!(
        below
            .iter()
            .filter(|d| d.code == DiagnosticCode::VariadicCount)
            .count(),
        1
    );
    assert_eq!(
        below
            .iter()
            .filter(|d| d.code == DiagnosticCode::InputRequired)
            .count(),
        1
    );
    assert_eq!(
        above
            .iter()
            .filter(|d| d.code == DiagnosticCode::VariadicCount)
            .count(),
        1
    );
    assert!(
        lower_bound
            .iter()
            .all(|d| d.code != DiagnosticCode::VariadicCount)
    );
    assert!(
        upper_bound
            .iter()
            .all(|d| d.code != DiagnosticCode::VariadicCount)
    );
    let diagnostic = above
        .iter()
        .find(|d| d.code == DiagnosticCode::VariadicCount)
        .unwrap();
    assert_eq!(diagnostic.subject, port_subject());
    assert!(diagnostic.message_args.is_empty());
    assert_eq!(below_document.state_id(), state_id);
    assert_eq!(below_document.revision(), revision);
}

#[test]
fn variadic_count_handles_absent_zero_width_and_unbounded_ranges() {
    let absent_min = run(
        &document(json!({})),
        signature(json!([
            {"port_id":"items","direction":"output","kind":"data","type":"core/i64@1","source":"computed","variadic":{"min":1,"max":null}}
        ])),
    );
    let zero_width = run(
        &document(json!({"items":{"order":[3],"next":4}})),
        signature(json!([
            {"port_id":"items","direction":"output","kind":"data","type":"core/i64@1","source":"computed","variadic":{"min":0,"max":0}}
        ])),
    );
    let unbounded = run(
        &document(json!({"items":{"order":[3,8,1],"next":9}})),
        signature(json!([
            {"port_id":"items","direction":"output","kind":"data","type":"core/i64@1","source":"computed","variadic":{"min":0,"max":null}}
        ])),
    );

    assert_eq!(absent_min[0].code, DiagnosticCode::VariadicCount);
    assert_eq!(absent_min[0].subject, port_subject());
    assert_eq!(zero_width[0].code, DiagnosticCode::VariadicCount);
    assert!(unbounded.is_empty());
    let absent_zero = run(
        &document(json!({})),
        signature(json!([
            {"port_id":"items","direction":"output","kind":"exec","variadic":{"min":0,"max":0}}
        ])),
    );
    assert!(absent_zero.is_empty());
    let exec_overflow = run(
        &document(json!({"items":{"order":[3],"next":4}})),
        signature(json!([
            {"port_id":"items","direction":"output","kind":"exec","variadic":{"min":0,"max":0}}
        ])),
    );
    assert_eq!(exec_overflow.len(), 1);
    assert_eq!(exec_overflow[0].code, DiagnosticCode::VariadicCount);
}

#[test]
fn variadic_count_does_not_depend_on_literals_or_connected_instances() {
    let mut fixture =
        serde_json::to_value(document(json!({"items":{"order":[9,2,18],"next":19}}))).unwrap();
    fixture["nodes"][0]["inputs"] = json!({"items#9":{"type":"core/i64@1","value":"7"}});
    fixture["edges"] = json!([{
        "id":"20000000-0000-4000-8000-000000000001",
        "from":{"node":NODE,"port":"source"},
        "to":{"node":NODE,"port":"items#2"}
    }]);
    let doc: orange_model::GraphDocument = serde_json::from_value(fixture).unwrap();
    let before = orange_model::to_json(&doc).unwrap();
    let diagnostics = run(
        &doc,
        signature(json!([
            {"port_id":"items","direction":"input","kind":"data","type":"core/i64@1","variadic":{"min":0,"max":2}},
            {"port_id":"source","direction":"output","kind":"data","type":"core/i64@1","source":"computed"}
        ])),
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d.code == DiagnosticCode::VariadicCount)
            .count(),
        1
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d.code == DiagnosticCode::InputRequired)
            .count(),
        1
    );
    assert_eq!(orange_model::to_json(&doc).unwrap(), before);
    // Input validation counts persisted instances; this fixture does not prove cycle legality.
}

#[test]
fn unresolved_nodes_skip_variadic_count_checks() {
    let doc = document(json!({}));
    let diagnostics = validate_inputs(&doc, &BTreeMap::new(), &RegistrySnapshot::core(origin()));
    assert!(diagnostics.is_empty());
}
