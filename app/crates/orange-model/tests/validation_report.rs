mod common;

use std::collections::BTreeMap;

use common::registry::Facts;
use orange_model::catalog::{NodeContract, ProviderKey, RegistrySnapshot};
use orange_model::document::{DefinitionRef, NodeKind};
use orange_model::id::{FunctionId, NodeId};
use orange_model::validation::{
    Diagnostic, DiagnosticCode, FunctionEvidence, HostOrigin, LeafEffects, VariableWrite,
    WriteOperation, analyze_functions, sort_diagnostics, validate, validate_connections,
    validate_inputs, validate_nodes, validate_with_effects,
};
use orange_model::{GraphDocument, ValidationReport, to_json};
use serde_json::{Value, json};

const NODE: &str = "10000000-0000-4000-8000-000000000001";
const OTHER: &str = "10000000-0000-4000-8000-000000000002";
const LEAF: &str = "20000000-0000-4000-8000-000000000001";
const SET: &str = "20000000-0000-4000-8000-000000000002";
const FUNC: &str = "30000000-0000-4000-8000-000000000001";
const VAR: &str = "40000000-0000-4000-8000-000000000001";
const VALUE_SIG: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const ACT_SIG: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn origin() -> HostOrigin {
    HostOrigin::new("1.0.0", format!("sha256:{}", "d".repeat(64))).unwrap()
}

fn contract(kind: NodeKind, ports: Value) -> NodeContract {
    NodeContract {
        kind,
        suspends: None,
        type_params: vec![],
        ports: serde_json::from_value(ports).unwrap(),
    }
}

fn registry() -> std::sync::Arc<RegistrySnapshot> {
    let mut facts = Facts::new(origin());
    facts.definition(
        DefinitionRef {
            id: "example/value".parse().unwrap(),
            version: 1,
        },
        contract(
            NodeKind::Pure,
            json!([
                {"port_id":"x","direction":"input","kind":"data","type":"core/i64@1"},
                {"port_id":"y","direction":"output","kind":"data","type":"core/i64@1","source":"computed"}
            ]),
        ),
    );
    facts.definition(
        DefinitionRef {
            id: "example/act".parse().unwrap(),
            version: 1,
        },
        contract(NodeKind::Action, json!([])),
    );
    for id in ["example/value", "example/act"] {
        let definition = DefinitionRef {
            id: id.parse().unwrap(),
            version: 1,
        };
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

fn node(id: &str, definition: &str, inputs: Value) -> Value {
    let sig = if definition == "example/act" {
        ACT_SIG
    } else {
        VALUE_SIG
    };
    json!({"id":id,"definition_id":definition,"definition_version":1,"signature":sig,
        "provider":{"package":"example.pkg","version":"1.0.0","id":definition},
        "config_version":1,"config":{},"inputs":inputs})
}

fn document(nodes: Value, edges: Value, variables: Value, functions: Value) -> GraphDocument {
    serde_json::from_value(json!({
        "format_version":1,"graph_id":"50000000-0000-4000-8000-000000000001","revision":3,
        "package_lock":[{"id":"example.pkg","version":"1.0.0","content_digest":DIGEST}],
        "nodes":nodes,"edges":edges,"variables":variables,"functions":functions,
        "signatures":{
            VALUE_SIG: {"kind":"pure","ports":[
                {"port_id":"x","direction":"input","kind":"data","type":"core/i64@1"},
                {"port_id":"y","direction":"output","kind":"data","type":"core/i64@1","source":"computed"}]},
            ACT_SIG: {"kind":"action","ports":[]}
        }
    }))
    .unwrap()
}

fn function_with(nodes: Value, kind: &str) -> Value {
    json!([{"id":FUNC,"name":"f","kind":kind,"nodes":nodes,"edges":[]}])
}

fn run_variable() -> Value {
    json!([{"id":VAR,"name":"v","type":"core/i64@1","scope":"run"}])
}

fn merged(
    doc: &GraphDocument,
    registry: &RegistrySnapshot,
    effects: BTreeMap<NodeId, LeafEffects>,
) -> ValidationReport {
    let nodes = validate_nodes(doc, registry);
    let connections = validate_connections(doc, registry);
    let evidence = FunctionEvidence::from_nodes(doc, &nodes, effects);
    let functions = analyze_functions(doc, &evidence, registry);
    let mut diagnostics: Vec<Diagnostic> = nodes.diagnostics.clone();
    diagnostics.extend(validate_inputs(doc, &nodes.resolved, registry));
    diagnostics.extend(connections.diagnostics);
    diagnostics.extend(functions.diagnostics);
    sort_diagnostics(&mut diagnostics);
    diagnostics.dedup();
    let mut incomplete_nodes = nodes.incomplete_nodes;
    incomplete_nodes.extend(connections.incomplete_nodes);
    ValidationReport {
        diagnostics,
        resolved: nodes.resolved,
        incomplete_nodes,
        functions: functions.summaries,
        incomplete_functions: functions.incomplete_functions,
    }
}

/// The entry point equals its passes and leaves the document untouched.
fn assert_equals_passes(doc: &GraphDocument) -> ValidationReport {
    let registry = registry();
    let (json, revision, state) = (to_json(doc).unwrap(), doc.revision(), doc.state_id());
    let report = validate(doc, &registry);
    assert_eq!(report, merged(doc, &registry, BTreeMap::new()));
    assert_eq!(to_json(doc).unwrap(), json);
    assert_eq!((doc.revision(), doc.state_id()), (revision, state));
    let mut sorted = report.diagnostics.clone();
    sort_diagnostics(&mut sorted);
    sorted.dedup();
    assert_eq!(report.diagnostics, sorted);
    report
}

fn codes(report: &ValidationReport) -> Vec<DiagnosticCode> {
    report.diagnostics.iter().map(|item| item.code).collect()
}

#[test]
fn a_valid_document_is_valid() {
    let doc = document(
        json!([node(
            NODE,
            "example/value",
            json!({"x":{"type":"core/i64@1","value":"5"}})
        )]),
        json!([]),
        json!([]),
        json!([]),
    );
    let report = assert_equals_passes(&doc);
    assert!(report.is_valid(), "{:?}", report.diagnostics);
    assert!(report.resolved.contains_key(&NODE.parse().unwrap()));
}

#[test]
fn an_orphan_edge_is_reported() {
    let doc = document(
        json!([node(NODE, "example/value", json!({}))]),
        json!([{"id":"60000000-0000-4000-8000-000000000001",
            "from":{"node":NODE,"port":"y"},"to":{"node":OTHER,"port":"x"}}]),
        json!([]),
        json!([]),
    );
    let report = assert_equals_passes(&doc);
    assert!(!report.is_valid());
    assert!(codes(&report).contains(&DiagnosticCode::PortUnknown));
}

#[test]
fn a_missing_definition_is_incomplete() {
    let doc = document(
        json!([node(NODE, "example/unknown", json!({}))]),
        json!([]),
        json!([]),
        json!([]),
    );
    let report = assert_equals_passes(&doc);
    assert!(!report.is_valid());
    assert!(report.incomplete_nodes.contains(&NODE.parse().unwrap()));
}

#[test]
fn a_bad_literal_is_reported() {
    let doc = document(
        json!([node(
            NODE,
            "example/value",
            json!({"x":{"type":"core/i64@1","value":"not-an-integer"}})
        )]),
        json!([]),
        json!([]),
        json!([]),
    );
    let report = assert_equals_passes(&doc);
    assert!(!report.is_valid());
    assert!(!report.diagnostics.is_empty());
}

#[test]
fn a_function_with_run_writes_is_summarized() {
    let set = json!({"id":SET,"definition_id":"core/set-variable","definition_version":1,
        "config_version":1,"config":{"variable":VAR}});
    let doc = document(
        json!([]),
        json!([]),
        run_variable(),
        function_with(json!([set]), "action"),
    );
    let report = assert_equals_passes(&doc);
    let summary = &report.functions[&FUNC.parse::<FunctionId>().unwrap()];
    assert_eq!(
        summary.run_writes[&VAR.parse().unwrap()],
        [WriteOperation::Set].into()
    );
}

#[test]
fn a_package_action_leaf_needs_host_supplied_effects() {
    let doc = document(
        json!([]),
        json!([]),
        run_variable(),
        function_with(json!([node(LEAF, "example/act", json!({}))]), "action"),
    );
    let registry = registry();
    let function: FunctionId = FUNC.parse().unwrap();

    let without = validate(&doc, &registry);
    assert!(without.incomplete_functions.contains(&function));
    assert!(codes(&without).contains(&DiagnosticCode::FunctionMetadataUnavailable));
    assert!(!without.is_valid());

    let write = VariableWrite {
        variable: VAR.parse().unwrap(),
        operation: WriteOperation::Add,
    };
    let effects = BTreeMap::from([(LEAF.parse().unwrap(), LeafEffects::Known(vec![write]))]);
    let with = validate_with_effects(&doc, &registry, effects.clone());
    assert_eq!(with, merged(&doc, &registry, effects));
    assert!(with.is_valid(), "{:?}", with.diagnostics);
    assert_eq!(
        with.functions[&function].run_writes[&VAR.parse().unwrap()],
        [WriteOperation::Add].into()
    );
}
