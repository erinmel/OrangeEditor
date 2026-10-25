mod common;

use common::registry::Facts;
use common::{minimal_document, node_id};
use orange_model::catalog::{NodeContract, ProviderKey, RegistrySnapshot};
use orange_model::document::{DefinitionRef, NodeKind};
use orange_model::validation::{DiagnosticCode, HostOrigin, ValidationCheck, analyze};
use orange_model::{GraphDocument, from_json};
use serde_json::{Value, json};

const VAR: &str = "40000000-0000-4000-8000-000000000001";
const EVENT: &str = "40000000-0000-4000-8000-000000000002";
const DIGEST: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

fn origin() -> HostOrigin {
    HostOrigin::new("0.1.0", format!("sha256:{}", "a".repeat(64))).unwrap()
}

fn get_variable(n: u8) -> Value {
    json!({"id":node_id(n),"definition_id":"core/get-variable","definition_version":1,
        "config_version":1,"config":{"variable":VAR}})
}

fn on_event(n: u8) -> Value {
    json!({"id":node_id(n),"definition_id":"core/on-event","definition_version":1,
        "config_version":1,"config":{"event":EVENT},
        "activation":{"mode":"queue","capacity":8,"overflow":"drop-oldest","on_fault":"continue","order":0}})
}

fn export(id: &str, node: u8, port: &str) -> Value {
    json!({"id":id,"node":node_id(node),"port":port})
}

fn document(nodes: Value, exports: Value) -> GraphDocument {
    from_json(&minimal_document(json!({
        "nodes":nodes,
        "variables":[{"id":VAR,"name":"total","type":"core/i64@1","scope":"run"}],
        "events":[{"id":EVENT,"name":"e","payload":"core/i64@1","scope":"run"}],
        "exports":exports
    })))
    .unwrap()
}

fn export_codes(document: &GraphDocument, registry: &RegistrySnapshot) -> Vec<DiagnosticCode> {
    analyze(document, registry)
        .report()
        .diagnostics
        .iter()
        .map(|d| d.code)
        .collect()
}

fn core_codes(document: &GraphDocument) -> Vec<DiagnosticCode> {
    export_codes(document, &RegistrySnapshot::core(origin()))
}

#[test]
fn a_state_output_is_exportable() {
    let document = document(
        json!([get_variable(1)]),
        json!([export("total", 1, "value")]),
    );
    assert!(core_codes(&document).is_empty());
}

#[test]
fn exports_must_name_an_existing_root_data_output() {
    let nodes = json!([get_variable(1), on_event(2)]);
    // Missing node, undeclared port, and a non-data (exec) output.
    for target in [
        export("a", 9, "value"),
        export("a", 1, "nope"),
        export("a", 2, "then"),
    ] {
        let document = document(nodes.clone(), json!([target]));
        let codes = core_codes(&document);
        assert!(codes.contains(&DiagnosticCode::ExportTarget), "{codes:?}");
    }
}

#[test]
fn an_export_of_a_function_body_node_is_not_a_root_target() {
    let function =
        json!([{"id":node_id(20),"name":"f","kind":"pure","nodes":[get_variable(5)],"edges":[]}]);
    let document = from_json(&minimal_document(json!({
        "variables":[{"id":VAR,"name":"total","type":"core/i64@1","scope":"run"}],
        "functions":function,
        "exports":[export("a", 5, "value")]
    })))
    .unwrap();
    assert!(core_codes(&document).contains(&DiagnosticCode::ExportTarget));
}

#[test]
fn duplicate_export_ids_report_every_later_use() {
    let document = document(
        json!([get_variable(1), get_variable(2)]),
        json!([
            export("a", 1, "value"),
            export("a", 2, "value"),
            export("b", 1, "value")
        ]),
    );
    let codes = core_codes(&document);
    assert_eq!(
        codes
            .iter()
            .filter(|c| **c == DiagnosticCode::ExportDuplicate)
            .count(),
        1
    );
}

#[test]
fn an_activation_scoped_export_crosses_activations() {
    let document = document(
        json!([on_event(1)]),
        json!([export("payload", 1, "payload")]),
    );
    assert!(core_codes(&document).contains(&DiagnosticCode::CrossActivation));
}

fn registry() -> std::sync::Arc<RegistrySnapshot> {
    let mut facts = Facts::new(origin());
    for (id, port_type, source) in [
        ("example/stream", "core/stream@1<core/i64@1>", "computed"),
        (
            "example/streams",
            "core/list@1<core/stream@1<core/i64@1>>",
            "computed",
        ),
        ("example/number", "core/i64@1", "computed"),
        ("example/produced", "core/i64@1", "execution"),
    ] {
        let kind = if source == "execution" {
            NodeKind::Action
        } else {
            NodeKind::Pure
        };
        let definition = DefinitionRef {
            id: id.parse().unwrap(),
            version: 1,
        };
        facts.definition(
            definition.clone(),
            NodeContract {
                kind,
                suspends: None,
                type_params: vec![],
                ports: serde_json::from_value(json!([
                    {"port_id":"out","direction":"output","kind":"data","type":port_type,"source":source}
                ]))
                .unwrap(),
            },
        );
        facts.provider(
            definition,
            1,
            ProviderKey {
                package: "example.pkg".into(),
                version: "1.0.0".into(),
                content_digest: DIGEST.into(),
                id: id.into(),
            },
        );
    }
    facts.snapshot()
}

fn package_node(n: u8, definition: &str, signature: &str) -> Value {
    json!({"id":node_id(n),"definition_id":definition,"definition_version":1,"signature":signature,
        "provider":{"package":"example.pkg","version":"1.0.0","id":definition},
        "config_version":1,"config":{}})
}

fn package_document(nodes: Value, signatures: Value, exports: Value) -> GraphDocument {
    from_json(&minimal_document(json!({
        "package_lock":[{"id":"example.pkg","version":"1.0.0","content_digest":DIGEST}],
        "nodes":nodes,"signatures":signatures,"exports":exports
    })))
    .unwrap()
}

fn signature(kind: &str, port_type: &str, source: &str) -> Value {
    json!({"kind":kind,"ports":[
        {"port_id":"out","direction":"output","kind":"data","type":port_type,"source":source}]})
}

#[test]
fn exported_types_must_be_serializable() {
    let registry = registry();
    let document = package_document(
        json!([
            package_node(1, "example/stream", "sig-stream"),
            package_node(2, "example/streams", "sig-streams"),
            package_node(3, "example/number", "sig-number")
        ]),
        json!({
            "sig-stream": signature("pure", "core/stream@1<core/i64@1>", "computed"),
            "sig-streams": signature("pure", "core/list@1<core/stream@1<core/i64@1>>", "computed"),
            "sig-number": signature("pure", "core/i64@1", "computed")
        }),
        json!([
            export("s", 1, "out"),
            export("l", 2, "out"),
            export("n", 3, "out")
        ]),
    );
    let report = analyze(&document, &registry).into_report();
    let bad: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.code == DiagnosticCode::ExportType)
        .filter_map(|d| d.subject.node)
        .collect();
    assert_eq!(bad.len(), 2, "{:?}", report.diagnostics);
    assert!(!report.is_valid());
}

#[test]
fn an_execution_sourced_export_crosses_activations() {
    let registry = registry();
    let document = package_document(
        json!([package_node(1, "example/produced", "sig-produced")]),
        json!({"sig-produced": signature("action", "core/i64@1", "execution")}),
        json!([export("p", 1, "out")]),
    );
    let codes = export_codes(&document, &registry);
    assert!(
        codes.contains(&DiagnosticCode::CrossActivation),
        "{codes:?}"
    );
}

#[test]
fn entry_policies_and_exports_is_covered() {
    let document = document(json!([]), json!([]));
    let analysis = analyze(&document, &RegistrySnapshot::core(origin()));
    assert!(
        analysis
            .covered_checks()
            .any(|check| check == ValidationCheck::EntryPoliciesAndExports)
    );
}
