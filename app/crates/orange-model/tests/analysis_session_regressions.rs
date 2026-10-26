//! Session behavior across host edits, stale requests and cancellation. The full validator
//! is the oracle; these assertions do not depend on the session's cache representation.
mod common;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use common::generic::{document, registry};
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{NodeKind, VariableDecl};
use orange_model::validation::{
    AnalysisLimits, AnalysisOptions, AnalysisSession, DiagnosticCode, RebuildReason, SessionWork,
    analyze_with_options,
};
use orange_model::{DocCommand, GraphDocument, LayoutCommand, from_json, to_json};
use serde_json::json;

fn layout(document: &mut GraphDocument, value: i64) -> orange_model::edit::Committed {
    document
        .apply([DocCommand::Layout(LayoutCommand::Set {
            path: vec!["viewport".into()],
            value: Some(json!({"x": value})),
        })])
        .unwrap()
}

fn assert_full(
    session: &AnalysisSession,
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    options: AnalysisOptions<'_>,
) {
    let expected = analyze_with_options(document, registry, BTreeMap::new(), options);
    assert_eq!(session.analysis(), &expected);
    assert_eq!(session.materialize(), expected);
    assert_eq!(
        session.analysis().is_ready_for_lowering(document, registry),
        expected.is_ready_for_lowering(document, registry)
    );
}

#[test]
fn same_revision_foreign_document_never_inherits_another_report() {
    let registry = registry(None);
    let original = document(vec![common::branch_node(1)], vec![]);
    let mut foreign = common::empty_document();
    assert_eq!(original.revision(), foreign.revision());
    let mut session = AnalysisSession::new(
        &original,
        &registry,
        BTreeMap::new(),
        AnalysisOptions::default(),
    );
    let committed = layout(&mut foreign, 1);
    session.update(
        &foreign,
        &registry,
        &committed.change_set,
        AnalysisOptions::default(),
    );
    assert_eq!(
        session.stats().work,
        SessionWork::Rebuild(RebuildReason::StateMismatch)
    );
    assert_full(&session, &foreign, &registry, AnalysisOptions::default());
    assert!(
        session
            .analysis()
            .is_ready_for_lowering(&foreign, &registry)
    );
}

#[test]
fn divergent_clone_and_reloaded_document_rebuild_despite_matching_revisions() {
    let registry = registry(None);
    let mut edited = common::empty_document();
    let mut sibling = edited.clone();
    let mut session = AnalysisSession::new(
        &edited,
        &registry,
        BTreeMap::new(),
        AnalysisOptions::default(),
    );
    let semantic = edited.apply([common::add(1)]).unwrap();
    session.update(
        &edited,
        &registry,
        &semantic.change_set,
        AnalysisOptions::default(),
    );
    let _unseen_sibling_change = layout(&mut sibling, 1);
    let sibling_change = layout(&mut sibling, 2);
    session.update(
        &sibling,
        &registry,
        &sibling_change.change_set,
        AnalysisOptions::default(),
    );
    assert_eq!(
        session.stats().work,
        SessionWork::Rebuild(RebuildReason::StateMismatch)
    );
    assert_full(&session, &sibling, &registry, AnalysisOptions::default());

    let mut loaded = from_json(&to_json(&sibling).unwrap()).unwrap();
    assert!(!session.is_current(&loaded, &registry));
    let loaded_change = layout(&mut loaded, 3);
    session.update(
        &loaded,
        &registry,
        &loaded_change.change_set,
        AnalysisOptions::default(),
    );
    assert_eq!(
        session.stats().work,
        SessionWork::Rebuild(RebuildReason::StateMismatch)
    );
    assert_full(&session, &loaded, &registry, AnalysisOptions::default());
}

#[test]
fn skipped_layout_and_replayed_receipts_cannot_authorize_delta_reuse() {
    let registry = registry(None);
    let mut document = common::empty_document();
    let mut session = AnalysisSession::new(
        &document,
        &registry,
        BTreeMap::new(),
        AnalysisOptions::default(),
    );
    let _skipped = layout(&mut document, 1);
    let committed = layout(&mut document, 2);
    for _ in 0..2 {
        session.update(
            &document,
            &registry,
            &committed.change_set,
            AnalysisOptions::default(),
        );
        assert_eq!(
            session.stats().work,
            SessionWork::Rebuild(RebuildReason::StateMismatch)
        );
        assert_full(&session, &document, &registry, AnalysisOptions::default());
    }
}

#[test]
fn unchanged_effects_recheck_document_registry_and_limits() {
    let registry = registry(None);
    let mut document = common::empty_document();
    let mut session = AnalysisSession::new(
        &document,
        &registry,
        BTreeMap::new(),
        AnalysisOptions::default(),
    );
    let _unseen = document.apply([common::add(1)]).unwrap();
    session.set_leaf_effects(
        &document,
        &registry,
        BTreeMap::new(),
        AnalysisOptions::default(),
    );
    assert_full(&session, &document, &registry, AnalysisOptions::default());

    let replacement = common::generic::registry(Some(vec!["core/i64@1"]));
    session.set_leaf_effects(
        &document,
        &replacement,
        BTreeMap::new(),
        AnalysisOptions::default(),
    );
    assert_full(
        &session,
        &document,
        &replacement,
        AnalysisOptions::default(),
    );

    let restricted = AnalysisOptions {
        limits: AnalysisLimits {
            max_items: 0,
            ..AnalysisLimits::default()
        },
        cancelled: None,
    };
    session.set_leaf_effects(&document, &replacement, BTreeMap::new(), restricted);
    assert_full(&session, &document, &replacement, restricted);
    assert_eq!(session.diagnostics()[0].code, DiagnosticCode::AnalysisLimit);
    assert!(
        !session
            .analysis()
            .is_ready_for_lowering(&document, &replacement)
    );

    session.set_leaf_effects(
        &document,
        &replacement,
        BTreeMap::new(),
        AnalysisOptions::default(),
    );
    assert_full(
        &session,
        &document,
        &replacement,
        AnalysisOptions::default(),
    );
}

#[test]
fn cancellation_applies_to_layout_and_equal_effects_then_recovers() {
    let registry = registry(None);
    let mut document = common::empty_document();
    let mut session = AnalysisSession::new(
        &document,
        &registry,
        BTreeMap::new(),
        AnalysisOptions::default(),
    );
    let cancellation = AtomicBool::new(true);
    let options = AnalysisOptions {
        cancelled: Some(&cancellation),
        ..AnalysisOptions::default()
    };
    let committed = layout(&mut document, 1);
    session.update(&document, &registry, &committed.change_set, options);
    assert_full(&session, &document, &registry, options);
    assert!(!session.is_current(&document, &registry));
    assert!(session.analysis().covered_checks().next().is_none());

    cancellation.store(false, Ordering::Relaxed);
    session.set_leaf_effects(&document, &registry, BTreeMap::new(), options);
    assert_full(&session, &document, &registry, options);
    assert!(session.is_current(&document, &registry));
    assert!(
        session
            .analysis()
            .is_ready_for_lowering(&document, &registry)
    );

    cancellation.store(true, Ordering::Relaxed);
    session.set_leaf_effects(&document, &registry, BTreeMap::new(), options);
    assert_full(&session, &document, &registry, options);
    assert!(!session.is_current(&document, &registry));
    assert!(session.analysis().covered_checks().next().is_none());
}

#[test]
fn layout_undo_redo_preserve_valid_semantic_evidence() {
    let registry = registry(None);
    let mut document = common::empty_document();
    let mut session = AnalysisSession::new(
        &document,
        &registry,
        BTreeMap::new(),
        AnalysisOptions::default(),
    );
    let semantic_state = document.semantic_state_id();
    let committed = layout(&mut document, 10);
    session.update(
        &document,
        &registry,
        &committed.change_set,
        AnalysisOptions::default(),
    );
    let undone = document.revert(committed.inverse).unwrap();
    session.update(
        &document,
        &registry,
        &undone.change_set,
        AnalysisOptions::default(),
    );
    let redone = document.revert(undone.inverse).unwrap();
    session.update(
        &document,
        &registry,
        &redone.change_set,
        AnalysisOptions::default(),
    );
    assert_eq!(document.semantic_state_id(), semantic_state);
    assert_eq!(session.analysis().document_state(), semantic_state);
    assert_eq!(session.stats().work, SessionWork::None);
    assert_full(&session, &document, &registry, AnalysisOptions::default());
    assert!(
        session
            .analysis()
            .is_ready_for_lowering(&document, &registry)
    );
}

#[test]
fn declaration_edit_invalidates_a_consumer_inside_a_function_and_undo_restores_it() {
    let registry = registry(None);
    let variable: VariableDecl = serde_json::from_value(json!({
        "id": "30000000-0000-4000-8000-000000000001", "name": "state",
        "type": "core/i64@1", "scope": "run"
    }))
    .unwrap();
    let mut getter = common::node(1);
    getter.definition.id = "core/get-variable".parse().unwrap();
    getter.config.insert("variable".into(), json!(variable.id));
    let mut function = common::function(common::body([getter], []));
    function.kind = NodeKind::Volatile;
    let mut document = common::empty_document();
    let _ = document
        .apply([
            DocCommand::UpsertVariable(variable.clone()),
            DocCommand::UpsertFunction(function),
        ])
        .unwrap();
    let mut session = AnalysisSession::new(
        &document,
        &registry,
        BTreeMap::new(),
        AnalysisOptions::default(),
    );
    let mut changed_variable = variable.clone();
    changed_variable.ty = "core/bool@1".parse().unwrap();
    let changed = document
        .apply([DocCommand::UpsertVariable(changed_variable)])
        .unwrap();
    session.update(
        &document,
        &registry,
        &changed.change_set,
        AnalysisOptions::default(),
    );
    assert_full(&session, &document, &registry, AnalysisOptions::default());
    assert!(
        session
            .resolved(common::nid(1))
            .unwrap()
            .ports
            .iter()
            .any(|port| {
                port.port_id.as_str() == "value"
                    && port
                        .ty
                        .as_ref()
                        .is_some_and(|ty| ty.as_str() == "core/bool@1")
            })
    );
    let undone = document.revert(changed.inverse).unwrap();
    session.update(
        &document,
        &registry,
        &undone.change_set,
        AnalysisOptions::default(),
    );
    assert_full(&session, &document, &registry, AnalysisOptions::default());
    assert!(
        session
            .resolved(common::nid(1))
            .unwrap()
            .ports
            .iter()
            .any(|port| {
                port.port_id.as_str() == "value"
                    && port
                        .ty
                        .as_ref()
                        .is_some_and(|ty| ty.as_str() == "core/i64@1")
            })
    );
    let removed = document
        .apply([DocCommand::RemoveVariable(variable.id)])
        .unwrap();
    session.update(
        &document,
        &registry,
        &removed.change_set,
        AnalysisOptions::default(),
    );
    assert_full(&session, &document, &registry, AnalysisOptions::default());
    assert!(session.resolved(common::nid(1)).is_none());
}

#[test]
fn adding_missing_lock_and_signature_refreshes_an_untouched_node() {
    use common::registry::Facts;
    use orange_model::catalog::{NodeContract, ProviderKey};
    use orange_model::document::{DefinitionRef, PackageLockEntry, SignatureSnapshot};

    let definition = DefinitionRef {
        id: "org.test/value".parse().unwrap(),
        version: 1,
    };
    let signature: SignatureSnapshot = serde_json::from_value(json!({
        "kind": "pure", "ports": [{"port_id": "value", "direction": "output",
            "kind": "data", "type": "core/i64@1", "source": "computed"}]
    }))
    .unwrap();
    let digest = format!("sha256:{}", "a".repeat(64));
    let mut facts = Facts::new(common::generic::origin());
    facts.definition(
        definition.clone(),
        NodeContract {
            kind: signature.kind,
            suspends: signature.suspends,
            type_params: vec![],
            ports: signature.ports.clone(),
        },
    );
    facts.provider(
        definition,
        1,
        ProviderKey {
            package: "org.test.package".into(),
            version: "1.0.0".into(),
            content_digest: digest.clone(),
            id: "native".into(),
        },
    );
    let registry = facts.snapshot();
    let mut document = document(
        vec![
            json!({"id": common::node_id(1), "definition_id": "org.test/value",
            "definition_version": 1, "config_version": 1, "config": {},
            "signature": "session-signature",
            "provider": {"package": "org.test.package", "version": "1.0.0", "id": "native"}}),
        ],
        vec![],
    );
    let mut session = AnalysisSession::new(
        &document,
        &registry,
        BTreeMap::new(),
        AnalysisOptions::default(),
    );
    assert!(!session.analysis().report().is_valid());
    let lock = document
        .apply([DocCommand::PutPackageLock(PackageLockEntry {
            id: "org.test.package".into(),
            version: "1.0.0".into(),
            content_digest: digest,
            extensions: Default::default(),
        })])
        .unwrap();
    session.update(
        &document,
        &registry,
        &lock.change_set,
        AnalysisOptions::default(),
    );
    assert_full(&session, &document, &registry, AnalysisOptions::default());
    assert!(
        session
            .diagnostics()
            .iter()
            .all(|diagnostic| { diagnostic.code != DiagnosticCode::PackageLockMissing })
    );

    let added_signature = document
        .apply([DocCommand::PutSignature {
            key: "session-signature".parse().unwrap(),
            snapshot: signature,
        }])
        .unwrap();
    session.update(
        &document,
        &registry,
        &added_signature.change_set,
        AnalysisOptions::default(),
    );
    assert_full(&session, &document, &registry, AnalysisOptions::default());
    assert!(session.analysis().report().is_valid());
    let undone = document.revert(added_signature.inverse).unwrap();
    session.update(
        &document,
        &registry,
        &undone.change_set,
        AnalysisOptions::default(),
    );
    assert_full(&session, &document, &registry, AnalysisOptions::default());
    assert!(!session.analysis().report().is_valid());
}

#[test]
fn unsupported_domain_and_admission_rejections_do_no_retained_preparation() {
    let registry = registry(None);
    let cancellation = AtomicBool::new(true);
    let document = from_json(&common::minimal_document(json!({
        "domain": "example/foreign",
        "nodes": [common::branch_node(1)]
    })))
    .unwrap();
    let options = [
        AnalysisOptions::default(),
        AnalysisOptions {
            cancelled: Some(&cancellation),
            ..AnalysisOptions::default()
        },
        AnalysisOptions {
            limits: AnalysisLimits {
                max_items: 0,
                ..AnalysisLimits::default()
            },
            cancelled: None,
        },
    ];
    for options in options {
        let session = AnalysisSession::new(&document, &registry, BTreeMap::new(), options);
        assert_full(&session, &document, &registry, options);
        assert!(session.index().is_none());
        assert!(!session.stats().index_rebuilt);
        assert_eq!(session.stats().local.nodes_visited, 0);
        assert_eq!(session.stats().local.declarations_visited, 0);
        assert_eq!(session.stats().local.config_computed, 0);
        assert_eq!(session.stats().local.declarations_computed, 0);
        assert!(
            !session
                .analysis()
                .is_ready_for_lowering(&document, &registry)
        );
    }
}

#[test]
fn session_debug_omits_saved_configuration_and_literal_payloads() {
    let registry = registry(None);
    let sentinel = "SESSION_PAYLOAD_MUST_NOT_BE_FORMATTED";
    let document = document(
        vec![json!({
            "id": common::node_id(1), "definition_id": "core/branch",
            "definition_version": 1, "config_version": 1,
            "config": {"private": sentinel},
            "inputs": {"condition": {"type": "core/string@1", "value": sentinel}}
        })],
        vec![],
    );
    assert!(to_json(&document).unwrap().contains(sentinel));
    let session = AnalysisSession::new(
        &document,
        &registry,
        BTreeMap::new(),
        AnalysisOptions::default(),
    );
    assert!(!session.diagnostics().is_empty());
    for rendered in [format!("{session:?}"), format!("{session:#?}")] {
        assert!(rendered.contains("AnalysisSession"));
        assert!(!rendered.contains(sentinel));
        assert!(!rendered.contains("private"));
    }
    assert_full(&session, &document, &registry, AnalysisOptions::default());
}

#[test]
fn layout_defers_a_foreign_trial_index_rebuild_until_semantic_work() {
    let registry = registry(None);
    let mut original = document((1..=64).map(common::branch_node).collect(), vec![]);
    let mut foreign = common::empty_document();
    let mut session = AnalysisSession::new(
        &original,
        &registry,
        BTreeMap::new(),
        AnalysisOptions::default(),
    );
    let before = session.materialize();
    let foreign_saved = to_json(&foreign).unwrap();
    let _plan = session
        .plan_edit(&mut foreign, [common::add(200)], &registry)
        .unwrap();
    assert_eq!(to_json(&foreign).unwrap(), foreign_saved);
    assert_eq!(session.analysis(), &before);
    assert!(!session.index().unwrap().is_current(&original));

    let presentation = layout(&mut original, 1);
    session.update(
        &original,
        &registry,
        &presentation.change_set,
        AnalysisOptions::default(),
    );
    assert_eq!(session.analysis(), &before);
    assert_eq!(session.stats().work, SessionWork::None);
    assert_eq!(session.stats().full_analysis_runs, 0);
    assert_eq!(session.stats().local.nodes_visited, 0);
    assert!(!session.stats().index_rebuilt);
    assert!(session.index().is_none());

    let semantic = original.apply([common::add(65)]).unwrap();
    session.update(
        &original,
        &registry,
        &semantic.change_set,
        AnalysisOptions::default(),
    );
    assert!(session.stats().index_rebuilt);
    assert!(session.index().unwrap().is_current(&original));
    assert_full(&session, &original, &registry, AnalysisOptions::default());
}
