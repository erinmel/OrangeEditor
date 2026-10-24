mod common;

use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use orange_model::catalog::RegistrySnapshot;
use orange_model::validation::{DiagnosticCode, HostOrigin, validate_declaration_types};
use orange_model::{
    AnalysisLimits, AnalysisOptions, analyze, analyze_with_options, from_json, to_json,
};
use serde_json::json;

fn registry() -> std::sync::Arc<RegistrySnapshot> {
    RegistrySnapshot::core(HostOrigin::new("1.0.0", format!("sha256:{}", "a".repeat(64))).unwrap())
}

#[test]
fn unused_declarations_require_concrete_available_well_formed_types() {
    let registry = registry();
    for ty in [
        "not-a-type",
        "org.missing/unknown@1",
        "$T",
        "core/list@1<>",
        "core/list@1",
        "core/map@1<core/f64@1,core/i64@1>",
        "core/handle@1<core/i64@1>",
    ] {
        let doc = from_json(&common::minimal_document(json!({
            "variables":[{"id":common::node_id(1),"name":"private-name","type":ty,"scope":"run"}],
            "events":[{"id":common::node_id(2),"name":"private-name","payload":ty,"scope":"run"}]
        })))
        .unwrap();
        let before = (to_json(&doc).unwrap(), doc.state_id(), doc.revision());
        let diagnostics = validate_declaration_types(&doc, &registry);
        assert_eq!(diagnostics.len(), 2, "{ty}: {diagnostics:?}");
        assert!(
            diagnostics
                .iter()
                .all(|d| d.code == DiagnosticCode::DeclarationType)
        );
        assert!(
            diagnostics
                .iter()
                .any(|d| d.subject.variable == Some(common::node_id(1).parse().unwrap()))
        );
        assert!(
            diagnostics
                .iter()
                .any(|d| d.subject.event == Some(common::node_id(2).parse().unwrap()))
        );
        for diagnostic in &diagnostics {
            assert!(diagnostic.message_args.is_empty());
            assert!(!diagnostic.message.contains(ty));
            assert!(
                !serde_json::to_string(diagnostic)
                    .unwrap()
                    .contains("private-name")
            );
            assert_eq!(
                serde_json::from_str::<orange_model::validation::Diagnostic>(
                    &serde_json::to_string(diagnostic).unwrap()
                )
                .unwrap(),
                *diagnostic
            );
        }
        let analysis = analyze(&doc, &registry);
        assert!(!analysis.report().is_valid(), "{ty}");
        assert!(!analysis.is_ready_for_lowering(&doc, &registry));
        assert!(!analysis.is_ready_for_compilation(&doc, &registry));
        assert_eq!(
            (to_json(&doc).unwrap(), doc.state_id(), doc.revision()),
            before
        );
    }
}

#[test]
fn valid_unused_container_types_allow_lowering_but_not_plan_publication() {
    let registry = registry();
    let doc = from_json(&common::minimal_document(json!({
        "variables":[{"id":common::node_id(1),"name":"list","type":"core/list@1<core/i64@1>","scope":"run"}],
        "events":[{"id":common::node_id(2),"name":"map","payload":"core/map@1<core/string@1,core/i64@1>","scope":"run"}]
    }))).unwrap();
    let analysis = analyze(&doc, &registry);
    assert!(
        analysis.report().is_valid(),
        "{:?}",
        analysis.report().diagnostics
    );
    assert!(analysis.is_ready_for_lowering(&doc, &registry));
    assert!(!analysis.is_ready_for_lowering(&doc, &self::registry()));
    assert!(!analysis.is_ready_for_compilation(&doc, &registry));
}

#[test]
fn declaration_type_nesting_uses_the_existing_complexity_error() {
    let ty = format!("{}core/i64@1{}", "core/list@1<".repeat(65), ">".repeat(65));
    let doc = from_json(&common::minimal_document(json!({
        "variables":[{"id":common::node_id(1),"name":"deep","type":ty,"scope":"run"}]
    })))
    .unwrap();
    let diagnostics = validate_declaration_types(&doc, &registry());
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, DiagnosticCode::TypeComplexity);
}

#[test]
fn admission_quotas_are_aggregate_and_allow_the_exact_boundary() {
    let registry = registry();
    let ty = "core/i64@1";
    let doc = from_json(&common::minimal_document(json!({
        "variables":[{"id":common::node_id(1),"name":"v","type":ty,"scope":"run"}],
        "events":[{"id":common::node_id(2),"name":"e","payload":ty,"scope":"run"}]
    })))
    .unwrap();
    let limits = AnalysisLimits {
        max_items: 2,
        max_pair_estimate: 4,
        max_json_values: 0,
        max_metadata_bytes: 2 * ty.len(),
    };
    let cancelled = AtomicBool::new(false);
    let options = AnalysisOptions {
        limits,
        cancelled: Some(&cancelled),
    };
    let allowed = analyze_with_options(&doc, &registry, BTreeMap::new(), options);
    assert!(allowed.is_ready_for_lowering(&doc, &registry));
    assert_eq!(allowed.report(), analyze(&doc, &registry).report());
    for limits in [
        AnalysisLimits {
            max_items: 1,
            ..limits
        },
        AnalysisLimits {
            max_pair_estimate: 3,
            ..limits
        },
        AnalysisLimits {
            max_metadata_bytes: 2 * ty.len() - 1,
            ..limits
        },
    ] {
        let denied = analyze_with_options(
            &doc,
            &registry,
            BTreeMap::new(),
            AnalysisOptions { limits, ..options },
        );
        assert_eq!(
            denied.report().diagnostics[0].code,
            DiagnosticCode::AnalysisLimit
        );
    }
    cancelled.store(true, Ordering::Relaxed);
    let stopped = analyze_with_options(&doc, &registry, BTreeMap::new(), options);
    assert_eq!(
        stopped.report().diagnostics[0].code,
        DiagnosticCode::AnalysisCancelled
    );
    cancelled.store(false, Ordering::Relaxed);
    assert!(
        analyze_with_options(&doc, &registry, BTreeMap::new(), options)
            .is_ready_for_lowering(&doc, &registry)
    );
}

#[test]
fn admission_checks_literal_payloads_without_requiring_a_connected_node() {
    let doc = from_json(&common::minimal_document(json!({
        "variables":[{"id":common::node_id(1),"name":"large","type":"core/string@1","scope":"run",
            "init":{"type":"core/string@1","value":"private".repeat(1000)}}]
    })))
    .unwrap();
    let analysis = analyze_with_options(
        &doc,
        &registry(),
        BTreeMap::new(),
        AnalysisOptions {
            limits: AnalysisLimits {
                max_metadata_bytes: 100,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    assert_eq!(
        analysis.report().diagnostics[0].code,
        DiagnosticCode::AnalysisLimit
    );
    assert!(
        !serde_json::to_string(&analysis.report().diagnostics)
            .unwrap()
            .contains("private")
    );
}

#[test]
fn admission_and_cancellation_fail_closed_without_changing_the_document() {
    let registry = registry();
    let mut node = common::branch_node(1);
    node["config"] = json!({"private":"value"});
    let doc = from_json(&common::minimal_document(json!({
        "nodes":[node]
    })))
    .unwrap();
    let before = (to_json(&doc).unwrap(), doc.state_id(), doc.revision());
    let cancelled = AtomicBool::new(true);
    for (options, code) in [
        (
            AnalysisOptions {
                cancelled: Some(&cancelled),
                ..Default::default()
            },
            DiagnosticCode::AnalysisCancelled,
        ),
        (
            AnalysisOptions {
                limits: AnalysisLimits {
                    max_items: 0,
                    ..Default::default()
                },
                ..Default::default()
            },
            DiagnosticCode::AnalysisLimit,
        ),
        (
            AnalysisOptions {
                limits: AnalysisLimits {
                    max_pair_estimate: 0,
                    ..Default::default()
                },
                ..Default::default()
            },
            DiagnosticCode::AnalysisLimit,
        ),
        (
            AnalysisOptions {
                limits: AnalysisLimits {
                    max_json_values: 0,
                    ..Default::default()
                },
                ..Default::default()
            },
            DiagnosticCode::AnalysisLimit,
        ),
        (
            AnalysisOptions {
                limits: AnalysisLimits {
                    max_metadata_bytes: 0,
                    ..Default::default()
                },
                ..Default::default()
            },
            DiagnosticCode::AnalysisLimit,
        ),
    ] {
        let analysis = analyze_with_options(&doc, &registry, BTreeMap::new(), options);
        assert_eq!(analysis.report().diagnostics.len(), 1, "{options:?}");
        assert_eq!(analysis.report().diagnostics[0].code, code, "{options:?}");
        assert!(analysis.report().resolved.is_empty());
        assert!(analysis.report().functions.is_empty());
        assert_eq!(analysis.covered_checks().count(), 0);
        assert!(!analysis.report().is_valid());
        assert!(!analysis.is_ready_for_lowering(&doc, &registry));
        assert!(!analysis.is_ready_for_compilation(&doc, &registry));
    }
    assert_eq!(
        (to_json(&doc).unwrap(), doc.state_id(), doc.revision()),
        before
    );
}
