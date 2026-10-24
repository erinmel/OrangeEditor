use orange_model::document::BodyRef;
use orange_model::id::FunctionId;
use orange_model::validation::{
    DiagnosticCode, DomainValidation, HostOrigin, validate_body_domains,
};
use orange_model::{GraphDocument, from_json, to_json};
use serde_json::{Value, json};

const FUNCTION: &str = "30000000-0000-4000-8000-000000000001";

fn origin() -> HostOrigin {
    HostOrigin::new("1.0.0", format!("sha256:{}", "a".repeat(64))).unwrap()
}

fn document(root_domain: Option<Value>, function_domain: Option<Value>) -> GraphDocument {
    let mut root = json!({
        "format": "orange.graph",
        "format_version": 1,
        "graph_id": "a0f3822f-9cf8-4ab6-a2bf-e21fd345c3c6",
        "revision": 0,
        "package_lock": [], "type_lock": [], "variables": [], "events": [],
        "signatures": {}, "nodes": [], "edges": [], "functions": [],
        "exports": [], "editor": {}
    });
    let object = root.as_object_mut().unwrap();
    if let Some(domain) = root_domain {
        object.insert("domain".into(), domain);
    }
    let mut function = json!({
        "id": FUNCTION, "name": "f", "kind": "action",
        "nodes": [], "edges": []
    });
    if let Some(domain) = function_domain {
        function["domain"] = domain;
    }
    object.insert("functions".into(), json!([function]));
    from_json(&root.to_string()).unwrap()
}

fn empty_validation() -> DomainValidation {
    DomainValidation::default()
}

#[test]
fn absent_and_explicit_control_domains_are_supported_in_each_body() {
    let missing = document(None, None);
    let missing_state = missing.state_id();
    assert_eq!(
        validate_body_domains(&missing, &origin()),
        empty_validation()
    );

    let explicit = document(Some(json!("control")), Some(json!("control")));
    assert_eq!(
        validate_body_domains(&explicit, &origin()),
        empty_validation()
    );
    assert_eq!(missing.state_id(), missing_state);
}

#[test]
fn unknown_domain_is_reported_once_per_body_even_when_empty() {
    let document = document(
        Some(json!("vendor.signal/signal")),
        Some(json!("vendor.audio/stream")),
    );
    let result = validate_body_domains(&document, &origin());
    let function: FunctionId = FUNCTION.parse().unwrap();
    assert_eq!(
        result.unsupported_bodies,
        [BodyRef::Root, BodyRef::Function(function)].into()
    );
    assert_eq!(result.diagnostics.len(), 2);
    assert!(result.diagnostics.iter().all(|diagnostic| {
        diagnostic.code == DiagnosticCode::DomainUnsupported
            && diagnostic.subject.node.is_none()
            && diagnostic.subject.port.is_none()
            && diagnostic.subject.edge.is_none()
    }));
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.subject.function == Some(function) })
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.subject.function.is_none() })
    );
    assert!(result.diagnostics.iter().all(|diagnostic| {
        diagnostic.message_args.is_empty()
            && !diagnostic.message.contains("vendor.signal")
            && !diagnostic.message.contains("vendor.audio")
    }));
}

#[test]
fn every_non_control_json_value_is_unsupported_without_echoing_it() {
    for value in [
        Value::Null,
        json!(17),
        json!(true),
        json!([]),
        json!({"private": "marker"}),
        json!(""),
        json!("control "),
    ] {
        let document = document(Some(value), None);
        let result = validate_body_domains(&document, &origin());
        assert_eq!(result.unsupported_bodies, [BodyRef::Root].into());
        assert_eq!(result.diagnostics.len(), 1);
        let diagnostic = &result.diagnostics[0];
        assert_eq!(diagnostic.code, DiagnosticCode::DomainUnsupported);
        assert!(diagnostic.message_args.is_empty());
        assert!(!diagnostic.message.contains("marker"));
    }
}

#[test]
fn domain_values_round_trip_unchanged_on_root_and_function_bodies() {
    let source = json!({
        "format": "orange.graph", "format_version": 1,
        "graph_id": "a0f3822f-9cf8-4ab6-a2bf-e21fd345c3c6", "revision": 0,
        "package_lock": [], "type_lock": [], "variables": [], "events": [],
        "signatures": {}, "nodes": [], "edges": [],
        "domain": {"future": [1, "opaque"]},
        "functions": [{
            "id": FUNCTION, "name": "f", "kind": "action",
            "type_params": [], "inputs": [], "outputs": [],
            "nodes": [], "edges": [], "domain": null
        }],
        "exports": [], "editor": {}
    });
    let document = from_json(&source.to_string()).unwrap();
    let before = document.clone();
    let result = validate_body_domains(&document, &origin());
    assert_eq!(result.unsupported_bodies.len(), 2);
    assert_eq!(document, before);
    assert_eq!(
        serde_json::from_str::<Value>(&to_json(&document).unwrap()).unwrap(),
        source
    );
}

#[test]
fn combined_analysis_does_not_interpret_foreign_bodies_as_control() {
    use orange_model::catalog::RegistrySnapshot;
    use orange_model::validation::{ValidationCheck, analyze, analyze_with_effects, validate};
    use std::collections::BTreeMap;
    let document = document(None, Some(json!("private.domain/marker")));
    let mut wire = serde_json::to_value(&document).unwrap();
    wire["nodes"] = json!([{
        "id":"10000000-0000-4000-8000-000000000001", "definition_id":"core/yield",
        "definition_version":1, "config_version":1, "config":{}
    }]);
    wire["functions"][0]["nodes"] = json!([{
        "id":"10000000-0000-4000-8000-000000000002", "definition_id":"core/yield",
        "definition_version":1, "config_version":1, "config":{}
    }]);
    let document: GraphDocument = serde_json::from_value(wire).unwrap();
    let before = (
        to_json(&document).unwrap(),
        document.state_id(),
        document.revision(),
    );
    let registry = RegistrySnapshot::core(origin());
    let analysis = analyze(&document, &registry);
    let report = analysis.report();
    assert_eq!(
        analysis.covered_checks().collect::<Vec<_>>(),
        [ValidationCheck::BodyDomains]
    );
    assert_eq!(analysis.missing_checks().count(), 14);
    assert!(analysis.is_current(&document, &registry));
    assert!(!analysis.is_ready_for_compilation(&document, &registry));
    assert!(report.resolved.is_empty());
    assert!(report.functions.is_empty());
    assert!(report.data_cycle_paths.is_empty());
    assert_eq!(report.incomplete_nodes.len(), 2);
    assert_eq!(
        report.incomplete_functions,
        [FUNCTION.parse().unwrap()].into()
    );
    assert!(report.incomplete_nodes.iter().all(|node| {
        report.diagnostics.iter().any(|item| {
            item.subject.node == Some(*node) && item.code == DiagnosticCode::DomainAnalysisBlocked
        })
    }));
    assert_eq!(
        report
            .diagnostics
            .iter()
            .filter(|item| item.code == DiagnosticCode::DomainUnsupported)
            .count(),
        1
    );
    assert!(
        !serde_json::to_string(&report.diagnostics)
            .unwrap()
            .contains("private.domain")
    );
    assert_eq!(report, &validate(&document, &registry));
    assert_eq!(
        report,
        analyze_with_effects(&document, &registry, BTreeMap::new()).report()
    );
    assert_eq!(
        (
            to_json(&document).unwrap(),
            document.state_id(),
            document.revision()
        ),
        before
    );
}

#[test]
fn domain_edits_and_undo_change_actual_coverage_and_freshness() {
    use orange_model::DocCommand;
    use orange_model::catalog::RegistrySnapshot;
    use orange_model::validation::{ValidationCheck, analyze};
    let mut document = document(None, None);
    let registry = RegistrySnapshot::core(origin());
    let initial = analyze(&document, &registry);
    assert_eq!(initial.covered_checks().count(), 7);
    assert_eq!(initial.missing_checks().count(), 8);
    let function_id: FunctionId = FUNCTION.parse().unwrap();
    let mut function = document.functions()[&function_id].clone();
    function
        .extensions
        .insert("domain".into(), json!("vendor/signal"));
    let committed = document
        .apply([DocCommand::UpsertFunction(function)])
        .unwrap();
    assert!(!initial.is_current(&document, &registry));
    let blocked = analyze(&document, &registry);
    assert_eq!(
        blocked.covered_checks().collect::<Vec<_>>(),
        [ValidationCheck::BodyDomains]
    );
    let _redo = document.revert(committed.inverse).unwrap();
    assert!(initial.is_current(&document, &registry));
    assert!(!blocked.is_current(&document, &registry));
    assert_eq!(analyze(&document, &registry).covered_checks().count(), 7);
}
