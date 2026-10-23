use orange_model::GraphDocument;
use orange_model::catalog::RegistrySnapshot;
use orange_model::validation::{DiagnosticCode, HostOrigin, validate_nodes};
use serde_json::{Value, json};

const NODE: &str = "10000000-0000-4000-8000-000000000001";
const FUNCTION: &str = "30000000-0000-4000-8000-000000000001";

fn registry() -> std::sync::Arc<RegistrySnapshot> {
    RegistrySnapshot::core(HostOrigin::new("1.0.0", format!("sha256:{}", "a".repeat(64))).unwrap())
}

fn document(config: Value) -> Value {
    json!({"format_version":1,"graph_id":"50000000-0000-4000-8000-000000000001","revision":0,
        "nodes":[{"id":NODE,"definition_id":"core/call","definition_version":1,"config_version":1,"config":config}],
        "functions":[{"id":FUNCTION,"name":"f","kind":"pure","nodes":[],"edges":[],"inputs":[],"outputs":[]}]
    })
}

fn assert_unresolved(wire: Value, expected: DiagnosticCode) {
    let document: GraphDocument = serde_json::from_value(wire).unwrap();
    let before = (document.clone(), document.state_id());
    let result = validate_nodes(&document, &registry());
    assert_eq!(
        result
            .diagnostics
            .iter()
            .map(|item| item.code)
            .collect::<Vec<_>>(),
        [expected]
    );
    assert!(result.resolved.is_empty());
    assert_eq!(result.incomplete_nodes, [NODE.parse().unwrap()].into());
    assert_eq!(
        result.diagnostics[0].subject.node,
        Some(NODE.parse().unwrap())
    );
    assert!(result.diagnostics[0].message_args.is_empty());
    assert_eq!((document.clone(), document.state_id()), before);
}

#[test]
fn call_target_failures_are_distinct_from_unsupported_intrinsics() {
    for config in [
        json!({}),
        json!({"function":"private-invalid-target"}),
        json!({"function":7}),
    ] {
        assert_unresolved(document(config), DiagnosticCode::CallTargetMissing);
    }
    assert_unresolved(
        document(json!({"function":"30000000-0000-4000-8000-000000000099"})),
        DiagnosticCode::CallTargetUnknown,
    );
}

#[test]
fn valid_type_only_interfaces_do_not_manufacture_execution_metadata() {
    for kind in ["pure", "action", "async"] {
        let mut wire = document(json!({"function":FUNCTION}));
        wire["functions"][0]["kind"] = json!(kind);
        assert_unresolved(wire, DiagnosticCode::FunctionMetadataUnavailable);
    }
    let mut malformed = document(json!({"function":FUNCTION}));
    malformed["functions"][0]["inputs"] =
        json!([{"port_id":"value","direction":"output","kind":"data","type":"core/i64@1"}]);
    assert_unresolved(malformed, DiagnosticCode::FunctionMetadataUnavailable);
}

#[test]
fn forged_call_identity_and_foreign_domains_cannot_supply_typing_evidence() {
    for field in ["definition_version", "config_version"] {
        let mut wire = document(json!({"function":FUNCTION}));
        wire["nodes"][0][field] = json!(2);
        assert_unresolved(wire, DiagnosticCode::ProviderUnavailable);
    }
    let mut wire = document(json!({"function":FUNCTION}));
    wire["nodes"][0]["provider"] = json!({"package":"acme","version":"1.0.0","id":"forged"});
    assert_unresolved(wire, DiagnosticCode::ProviderUnavailable);
    let mut wire = document(json!({"function":FUNCTION}));
    wire["functions"][0]["domain"] = json!("vendor/signal");
    assert_unresolved(wire, DiagnosticCode::DomainUnsupported);
}
