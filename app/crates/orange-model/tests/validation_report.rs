mod common;

use std::collections::BTreeMap;

use common::registry::Facts;
use orange_model::catalog::{NodeContract, ProviderKey, RegistrySnapshot};
use orange_model::document::{DefinitionRef, NodeKind};
use orange_model::id::{FunctionId, NodeId};
use orange_model::validation::{
    Diagnostic, DiagnosticCode, FunctionEvidence, HostOrigin, LeafEffects, ValidationCheck,
    VariableWrite, WriteOperation, analyze, analyze_functions, analyze_with_effects,
    sort_diagnostics, validate, validate_connections, validate_data_cycles, validate_inputs,
    validate_nodes, validate_with_effects,
};
use orange_model::{GraphDocument, ValidationReport, from_json, to_json};
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
    let connections = validate_connections(doc, &nodes.resolved, registry);
    let data_cycles = validate_data_cycles(doc, &nodes.resolved, &connections, registry);
    let evidence = FunctionEvidence::from_nodes(doc, &nodes, effects);
    let functions = analyze_functions(doc, &evidence, registry);
    let mut diagnostics: Vec<Diagnostic> = nodes.diagnostics.clone();
    diagnostics.extend(validate_inputs(doc, &nodes.resolved, registry));
    diagnostics.extend(connections.diagnostics);
    diagnostics.extend(functions.diagnostics);
    let explained_nodes: std::collections::BTreeSet<_> = diagnostics
        .iter()
        .filter(|item| item.severity == orange_model::validation::Severity::Error)
        .filter_map(|item| item.subject.node)
        .collect();
    diagnostics.extend(data_cycles.diagnostics.into_iter().filter(|item| {
        item.code != DiagnosticCode::DependencyUnavailable
            || item
                .subject
                .node
                .is_none_or(|node| !explained_nodes.contains(&node))
    }));
    sort_diagnostics(&mut diagnostics);
    diagnostics.dedup();
    let mut incomplete_nodes = nodes.incomplete_nodes;
    incomplete_nodes.extend(connections.incomplete_nodes);
    incomplete_nodes.extend(data_cycles.incomplete_nodes);
    ValidationReport {
        diagnostics,
        resolved: nodes.resolved,
        incomplete_nodes,
        functions: functions.summaries,
        incomplete_functions: functions.incomplete_functions,
        data_cycle_paths: data_cycles.cycle_paths,
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
    assert_explained(&report);
    report
}

/// Every incomplete node carries a diagnostic naming it.
fn assert_explained(report: &ValidationReport) {
    for node in &report.incomplete_nodes {
        assert!(
            report
                .diagnostics
                .iter()
                .any(|item| item.subject.node == Some(*node)),
            "{node:?} is incomplete without a diagnostic: {:?}",
            report.diagnostics
        );
    }
}

fn codes(report: &ValidationReport) -> Vec<DiagnosticCode> {
    report.diagnostics.iter().map(|item| item.code).collect()
}

#[test]
fn combined_validation_rejects_data_cycles_and_exposes_concrete_port_witnesses() {
    let cyclic = document(
        json!([node(NODE, "example/value", json!({}))]),
        json!([{
            "id":"60000000-0000-4000-8000-000000000001",
            "from":{"node":NODE,"port":"y"},"to":{"node":NODE,"port":"x"}
        }]),
        json!([]),
        json!([]),
    );
    let report = assert_equals_passes(&cyclic);
    assert!(!report.is_valid());
    assert_eq!(codes(&report), [DiagnosticCode::CycleData]);
    assert_eq!(report.data_cycle_paths.len(), 1);
    let path = &report.data_cycle_paths[0];
    assert_eq!(path.first(), path.last());
    assert_eq!(path.len(), 3);
    assert!(path.iter().all(|port| port.node == NODE.parse().unwrap()));
    assert!(
        report.incomplete_nodes.is_empty(),
        "a known invalid cycle is fully analyzed"
    );
    let registry = registry();
    assert!(!analyze(&cyclic, &registry).is_ready_for_compilation(&cyclic, &registry));
}

#[test]
fn variable_feedback_has_a_state_boundary_in_combined_validation() {
    let get = "10000000-0000-4000-8000-000000000003";
    let set = "10000000-0000-4000-8000-000000000004";
    let feedback = document(
        json!([
            {"id":get,"definition_id":"core/get-variable","definition_version":1,"config_version":1,"config":{"variable":VAR}},
            node(NODE,"example/value",json!({})),
            {"id":set,"definition_id":"core/set-variable","definition_version":1,"config_version":1,"config":{"variable":VAR}}
        ]),
        json!([
            {"id":"60000000-0000-4000-8000-000000000001","from":{"node":get,"port":"value"},"to":{"node":NODE,"port":"x"}},
            {"id":"60000000-0000-4000-8000-000000000002","from":{"node":NODE,"port":"y"},"to":{"node":set,"port":"value"}}
        ]),
        run_variable(),
        json!([]),
    );
    let report = assert_equals_passes(&feedback);
    assert!(report.is_valid(), "{:?}", report.diagnostics);
    assert!(report.data_cycle_paths.is_empty());
    // This checks data dependencies; initialization, availability and execution remain separate.
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
    let partial = analyze(&doc, &registry);
    assert!(partial.report().incomplete_functions.contains(&function));
    assert!(!partial.is_ready_for_compilation(&doc, &registry));

    let write = VariableWrite {
        variable: VAR.parse().unwrap(),
        operation: WriteOperation::Add,
    };
    let effects = BTreeMap::from([(LEAF.parse().unwrap(), LeafEffects::Known(vec![write]))]);
    let with = validate_with_effects(&doc, &registry, effects.clone());
    assert_eq!(with, merged(&doc, &registry, effects.clone()));
    assert!(with.is_valid(), "{:?}", with.diagnostics);
    let with_analysis = analyze_with_effects(&doc, &registry, effects.clone());
    assert_eq!(with_analysis.report(), &with);
    assert!(!with_analysis.is_ready_for_compilation(&doc, &registry));
    assert_eq!(
        with.functions[&function].run_writes[&VAR.parse().unwrap()],
        [WriteOperation::Add].into()
    );
}

#[test]
fn analysis_reports_actual_coverage_and_legacy_wrappers_keep_the_same_report() {
    let doc = document(json!([]), json!([]), json!([]), json!([]));
    let registry = registry();
    let before = (to_json(&doc).unwrap(), doc.revision(), doc.state_id());
    let analysis = analyze(&doc, &registry);
    assert_eq!(
        analysis.covered_checks().collect::<Vec<_>>(),
        [
            ValidationCheck::NodeResolution,
            ValidationCheck::InputValues,
            ValidationCheck::Connections,
            ValidationCheck::FunctionSummaries,
            ValidationCheck::BodyDomains,
            ValidationCheck::DataCycles,
        ]
    );
    assert_eq!(
        analysis.missing_checks().collect::<Vec<_>>(),
        [
            ValidationCheck::DefinitionAndConfig,
            ValidationCheck::ContextualContracts,
            ValidationCheck::FunctionRestrictions,
            ValidationCheck::Regions,
            ValidationCheck::ControlCycles,
            ValidationCheck::ValueAvailability,
            ValidationCheck::StateRules,
            ValidationCheck::StreamRules,
            ValidationCheck::EntryPoliciesAndExports,
        ]
    );
    assert!(analysis.is_current(&doc, &registry));
    assert!(!analysis.is_ready_for_compilation(&doc, &registry));
    assert_eq!(analysis.report(), &validate(&doc, &registry));
    assert_eq!(analysis.clone().into_report(), validate(&doc, &registry));
    assert_eq!(
        (to_json(&doc).unwrap(), doc.revision(), doc.state_id()),
        before
    );
}

#[test]
fn analysis_freshness_tracks_content_identity_and_registry_generation() {
    let doc = document(json!([]), json!([]), json!([]), json!([]));
    let another_registry = registry();
    let registry = registry();
    let analysis = analyze(&doc, &registry);
    assert!(analysis.is_current(&doc.clone(), &registry));

    let reloaded = from_json(&to_json(&doc).unwrap()).unwrap();
    assert!(!analysis.is_current(&reloaded, &registry));
    assert!(!analysis.is_current(&doc, &another_registry));

    let mut edited = doc.clone();
    let inverse = edited.apply([common::add(99)]).unwrap().inverse;
    assert!(!analysis.is_current(&edited, &registry));
    assert!(!analysis.is_ready_for_compilation(&edited, &registry));
    let _redo = edited.revert(inverse).unwrap();
    assert!(analysis.is_current(&edited, &registry));

    let mut left = doc.clone();
    let mut right = doc.clone();
    let _left_inverse = left.apply([common::add(97)]).unwrap();
    let _right_inverse = right.apply([common::add(98)]).unwrap();
    assert_eq!(left.revision(), right.revision());
    assert_ne!(left.state_id(), right.state_id());
    let left_analysis = analyze(&left, &registry);
    assert!(!left_analysis.is_current(&right, &registry));
}

#[test]
fn foreign_body_domain_is_never_compile_ready() {
    let doc = from_json(&common::minimal_document(
        json!({"domain":"vendor.signal/signal"}),
    ))
    .unwrap();
    let registry = registry();
    let analysis = analyze(&doc, &registry);
    assert_eq!(
        analysis.covered_checks().collect::<Vec<_>>(),
        [ValidationCheck::BodyDomains]
    );
    assert_eq!(analysis.missing_checks().count(), 14);
    assert!(!analysis.report().is_valid());
    assert!(!analysis.is_ready_for_compilation(&doc, &registry));
}

const GET: &str = "20000000-0000-4000-8000-000000000003";
const TEXT_VAR: &str = "40000000-0000-4000-8000-000000000002";

fn variable_node(id: &str, definition: &str, variable: &str) -> Value {
    json!({"id":id,"definition_id":definition,"definition_version":1,
        "config_version":1,"config":{"variable":variable}})
}

fn wire(from: &str, from_port: &str, to: &str, to_port: &str) -> Value {
    json!([{"id":"60000000-0000-4000-8000-000000000001",
        "from":{"node":from,"port":from_port},"to":{"node":to,"port":to_port}}])
}

#[test]
fn a_get_variable_edge_is_checked_against_its_declaration() {
    let doc = document(
        json!([
            variable_node(GET, "core/get-variable", VAR),
            node(NODE, "example/value", json!({}))
        ]),
        wire(GET, "value", NODE, "x"),
        run_variable(),
        json!([]),
    );
    let report = assert_equals_passes(&doc);
    assert!(report.is_valid(), "{:?}", report.diagnostics);
}

#[test]
fn a_mistyped_variable_edge_is_a_type_mismatch() {
    let doc = document(
        json!([
            variable_node(GET, "core/get-variable", TEXT_VAR),
            variable_node(SET, "core/set-variable", VAR)
        ]),
        wire(GET, "value", SET, "value"),
        json!([
            {"id":VAR,"name":"v","type":"core/i64@1","scope":"run"},
            {"id":TEXT_VAR,"name":"s","type":"core/string@1","scope":"run"}
        ]),
        json!([]),
    );
    let report = assert_equals_passes(&doc);
    assert_eq!(codes(&report), [DiagnosticCode::TypeMismatch]);
    assert!(report.incomplete_nodes.is_empty());
}

const OPEN_SIG: &str = "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

#[test]
fn a_node_blocked_by_an_unresolved_peer_says_so() {
    let open = json!({"id":OTHER,"definition_id":"example/open","definition_version":1,
        "signature":OPEN_SIG,"provider":{"package":"example.pkg","version":"1.0.0","id":"example/open"},
        "config_version":1,"config":{}});
    let mut value = serde_json::to_value(document(
        json!([node(NODE, "example/value", json!({})), open]),
        wire(OTHER, "out", NODE, "x"),
        json!([]),
        json!([]),
    ))
    .unwrap();
    value["signatures"][OPEN_SIG] = json!({"kind":"pure","ports":[
        {"port_id":"out","direction":"output","kind":"data","type":"$T","source":"computed"}]});
    let doc: GraphDocument = serde_json::from_value(value).unwrap();
    let report = assert_equals_passes(&doc);
    let blocked = report
        .diagnostics
        .iter()
        .find(|item| item.code == DiagnosticCode::DependencyUnavailable)
        .expect("the resolved endpoint reports its unresolved peer");
    assert_eq!(blocked.subject.node, Some(NODE.parse().unwrap()));
    assert!(codes(&report).contains(&DiagnosticCode::ProviderUnavailable));
    assert!(!report.is_valid());
}

#[test]
fn an_intrinsic_this_core_does_not_support_says_so() {
    for definition in ["core/call", "core/on-event", "core/switch"] {
        let doc = document(
            json!([{"id":NODE,"definition_id":definition,"definition_version":1,
                "config_version":1,"config":{}}]),
            json!([]),
            json!([]),
            json!([]),
        );
        let report = assert_equals_passes(&doc);
        assert_eq!(
            codes(&report),
            [DiagnosticCode::IntrinsicUnsupported],
            "{definition}"
        );
    }
}
