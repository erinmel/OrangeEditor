//! Regressions for deterministic rejection freshness and early resource rejection.
mod common;

use orange_model::catalog::RegistryBuilder;
use orange_model::validation::{
    AnalysisLimits, AnalysisOptions, AnalysisSession, DiagnosticCode, HostOrigin, LeafEffects,
    SessionWork, VariableWrite, WriteOperation, analyze_with_options, config_satisfies_schema,
    validate_config,
};
use orange_model::{DocCommand, GraphDocument, LayoutCommand, from_json};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

fn layout(document: &mut GraphDocument, value: i32) -> orange_model::edit::Committed {
    document
        .apply([DocCommand::Layout(LayoutCommand::Set {
            path: vec!["view".into()],
            value: Some(json!(value)),
        })])
        .unwrap()
}

fn assert_no_validation(
    session: &AnalysisSession,
    doc: &GraphDocument,
    registry: &orange_model::catalog::RegistrySnapshot,
    options: AnalysisOptions<'_>,
) {
    assert!(session.is_current(doc, registry));
    assert!(!session.analysis().is_ready_for_lowering(doc, registry));
    assert_eq!(session.stats().work, SessionWork::None);
    assert_eq!(session.stats().full_analysis_runs, 0);
    assert_eq!(session.stats().local.nodes_visited, 0);
    assert!(!session.stats().index_rebuilt);
    assert_eq!(
        session.analysis(),
        &analyze_with_options(doc, registry, BTreeMap::new(), options)
    );
}

#[test]
fn deterministic_rejections_stay_fresh_through_layout_undo_redo_and_recheck_changed_inputs() {
    for foreign in [false, true] {
        let registry = common::generic::registry(None);
        let mut doc = from_json(&common::minimal_document(json!({
            "nodes":[common::branch_node(1)],
            "domain":if foreign {"example/foreign"} else {"control"}
        })))
        .unwrap();
        let options = AnalysisOptions {
            limits: if foreign {
                AnalysisLimits::default()
            } else {
                AnalysisLimits {
                    max_items: 0,
                    ..AnalysisLimits::default()
                }
            },
            cancelled: None,
        };
        let mut session = AnalysisSession::new(&doc, &registry, BTreeMap::new(), options);
        assert!(session.is_current(&doc, &registry));
        let original = session.materialize();
        let edit = layout(&mut doc, 1);
        session.update(&doc, &registry, &edit.change_set, options);
        assert_no_validation(&session, &doc, &registry, options);
        let undo = doc.revert(edit.inverse).unwrap();
        session.update(&doc, &registry, &undo.change_set, options);
        assert_no_validation(&session, &doc, &registry, options);
        let redo = doc.revert(undo.inverse).unwrap();
        session.update(&doc, &registry, &redo.change_set, options);
        assert_no_validation(&session, &doc, &registry, options);
        assert_eq!(session.analysis(), &original);
        session.set_leaf_effects(&doc, &registry, BTreeMap::new(), options);
        assert_no_validation(&session, &doc, &registry, options);

        // New limits and registry inputs still force full orchestration.
        session.set_leaf_effects(&doc, &registry, BTreeMap::new(), AnalysisOptions::default());
        assert_eq!(
            session.analysis(),
            &analyze_with_options(&doc, &registry, BTreeMap::new(), AnalysisOptions::default())
        );
        if !foreign {
            assert_eq!(session.stats().full_analysis_runs, 1);
        }
        let replacement = RegistryBuilder::core(
            HostOrigin::new("1.0.0", format!("sha256:{}", "0".repeat(64))).unwrap(),
        )
        .build()
        .unwrap();
        session.set_leaf_effects(
            &doc,
            &replacement,
            BTreeMap::new(),
            AnalysisOptions::default(),
        );
        assert_eq!(session.stats().full_analysis_runs, 1);
        let semantic = doc
            .apply([DocCommand::SetConfig {
                node: common::nid(1),
                config_version: 1,
                config: Default::default(),
            }])
            .unwrap();
        session.update(
            &doc,
            &replacement,
            &semantic.change_set,
            AnalysisOptions::default(),
        );
        assert_eq!(session.stats().full_analysis_runs, 1);
        assert_eq!(
            session.analysis(),
            &analyze_with_options(
                &doc,
                &replacement,
                BTreeMap::new(),
                AnalysisOptions::default()
            )
        );
    }
}

#[test]
fn large_retained_effects_reject_before_preparation_and_cancelled_requests_recover() {
    let registry = common::generic::registry(None);
    let mut doc = common::empty_document();
    let cancel = AtomicBool::new(true);
    let effects = BTreeMap::from([(
        common::nid(1),
        LeafEffects::Known(vec![
            VariableWrite {
                variable: "00000000-0000-4000-8000-000000000001".parse().unwrap(),
                operation: WriteOperation::Set,
            };
            100_000
        ]),
    )]);
    let options = AnalysisOptions {
        limits: AnalysisLimits {
            max_items: 0,
            ..AnalysisLimits::default()
        },
        cancelled: Some(&cancel),
    };
    let mut session = AnalysisSession::new(&doc, &registry, effects, options);
    assert_eq!(
        session.diagnostics()[0].code,
        DiagnosticCode::AnalysisCancelled
    );
    assert!(!session.is_current(&doc, &registry));
    assert!(session.index().is_none());
    cancel.store(false, Ordering::Relaxed);
    let edit = layout(&mut doc, 1);
    session.update(&doc, &registry, &edit.change_set, options);
    assert_eq!(session.diagnostics()[0].code, DiagnosticCode::AnalysisLimit);
    assert!(session.is_current(&doc, &registry));
    assert!(session.index().is_none());
    assert_eq!(session.stats().local.config_computed, 0);
    let edit = layout(&mut doc, 2);
    session.update(&doc, &registry, &edit.change_set, options);
    assert_eq!(session.stats().full_analysis_runs, 0);
    cancel.store(true, Ordering::Relaxed);
    let edit = layout(&mut doc, 3);
    session.update(&doc, &registry, &edit.change_set, options);
    assert_eq!(
        session.diagnostics()[0].code,
        DiagnosticCode::AnalysisCancelled
    );
    assert!(!session.is_current(&doc, &registry));
}

#[test]
fn borrowed_config_objects_preserve_root_and_nested_rules_across_registry_replacement() {
    let object = json!({"enabled":true,"nested":{"count":7},"ignored":"x".repeat(100_000)});
    for (schema, expected) in [
        (
            json!({"type":"object","required":["enabled"],"properties":{"nested":{"properties":{"count":{"minimum":7,"maximum":7}}}}}),
            true,
        ),
        (json!({"type":"array"}), false),
        (json!({"enum":[object.clone()]}), true),
        (json!({"enum":[{"enabled":true}]}), false),
        (json!({"required":["absent"]}), false),
        (
            json!({"properties":{"enabled":{"type":"boolean"}},"additionalProperties":false}),
            false,
        ),
    ] {
        assert_eq!(config_satisfies_schema(&object, &schema), expected);
    }
    let make_registry = |schema| {
        let original = common::generic::registry(None);
        let mut definition = original
            .definition(&common::generic::add_definition())
            .unwrap()
            .clone();
        definition.config_schema = Some(schema);
        RegistryBuilder::core(common::generic::origin())
            .definition(definition)
            .build()
            .unwrap()
    };
    let registry = make_registry(json!({"type":"object"}));
    let mut doc = common::generic::document(
        vec![common::generic::add(1, json!({"config":object}))],
        vec![],
    );
    assert!(validate_config(&doc, &registry).is_empty());
    let mut session =
        AnalysisSession::new(&doc, &registry, BTreeMap::new(), AnalysisOptions::default());
    let replacement = make_registry(json!({"$schema":"unsupported-dialect"}));
    let edit = layout(&mut doc, 1);
    session.update(
        &doc,
        &replacement,
        &edit.change_set,
        AnalysisOptions::default(),
    );
    assert!(
        session
            .diagnostics()
            .iter()
            .any(|d| d.code == DiagnosticCode::ConfigSchemaUnsupported)
    );
    assert_eq!(
        session.analysis(),
        &analyze_with_options(
            &doc,
            &replacement,
            BTreeMap::new(),
            AnalysisOptions::default()
        )
    );
}
