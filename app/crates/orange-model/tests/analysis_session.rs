//! V12a: an `AnalysisSession` always equals the full analysis of its document, and says
//! what each update did (16 §3).

mod common;

use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;

use common::edits::{command, op};
use common::generic::{add, data_edge, document, registry, typed_source};
use common::{empty_document, read_fixture};
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::Edge;
use orange_model::validation::{
    AnalysisLimits, AnalysisOptions, AnalysisSession, LeafEffects, RebuildReason, SessionWork,
    analyze_with_options,
};
use orange_model::{DocCommand, GraphDocument, LayoutCommand, from_json};
use proptest::prelude::*;
use serde_json::json;

fn options() -> AnalysisOptions<'static> {
    AnalysisOptions::default()
}

fn full(document: &GraphDocument, registry: &RegistrySnapshot) -> orange_model::ValidationAnalysis {
    analyze_with_options(document, registry, BTreeMap::new(), options())
}

fn connect(n: u8, from: (u8, &str), to: (u8, &str)) -> DocCommand {
    DocCommand::Connect {
        edge: serde_json::from_value::<Edge>(data_edge(n, from, to)).unwrap(),
    }
}

fn chain() -> GraphDocument {
    document(
        vec![typed_source(1, "i64"), add(2, json!({})), add(3, json!({}))],
        vec![data_edge(1, (1, "value"), (2, "a"))],
    )
}

#[test]
fn creation_and_semantic_edits_recompute_to_the_full_result() {
    let registry = registry(None);
    let mut doc = chain();
    let mut session = AnalysisSession::new(&doc, &registry, BTreeMap::new(), options());
    assert_eq!(
        session.stats().work,
        SessionWork::Rebuild(RebuildReason::Created)
    );
    assert_eq!(
        (session.stats().input_nodes, session.stats().input_edges),
        (3, 1)
    );
    assert_eq!(session.analysis(), &full(&doc, &registry));
    assert!(session.is_current(&doc, &registry));

    let committed = doc.apply([connect(2, (2, "sum"), (3, "a"))]).unwrap();
    let stats = *session.update(&doc, &registry, &committed.change_set, options());
    assert_eq!(stats.work, SessionWork::Recompute);
    assert_eq!((stats.input_nodes, stats.input_edges), (3, 2));
    assert_eq!(session.materialize(), full(&doc, &registry));
    assert_eq!(
        session.resolved(common::nid(3)),
        full(&doc, &registry).report().resolved.get(&common::nid(3))
    );

    // Undo is an ordinary committed change.
    let undone = doc.revert(committed.inverse).unwrap();
    session.update(&doc, &registry, &undone.change_set, options());
    assert_eq!(session.stats().work, SessionWork::Recompute);
    assert_eq!(session.analysis(), &full(&doc, &registry));
    assert_eq!(
        session.diagnostics(),
        &full(&doc, &registry).report().diagnostics[..]
    );
}

#[test]
fn layout_edits_rebind_without_work_and_stay_ready_for_lowering() {
    let registry = registry(None);
    let mut doc = chain();
    let mut session = AnalysisSession::new(&doc, &registry, BTreeMap::new(), options());
    let before = session.analysis().document_state();
    let committed = doc
        .apply([DocCommand::Layout(LayoutCommand::Set {
            path: vec!["nodes".into(), common::node_id(2)],
            value: Some(json!({"x": 40})),
        })])
        .unwrap();
    assert!(committed.change_set.layout_only());
    let stats = *session.update(&doc, &registry, &committed.change_set, options());
    assert_eq!(stats.work, SessionWork::None);
    assert_eq!((stats.input_nodes, stats.input_edges), (0, 0));
    assert_eq!(session.analysis().document_state(), before);
    assert_eq!(session.analysis(), &full(&doc, &registry));
    assert_eq!(
        session.analysis().is_ready_for_lowering(&doc, &registry),
        full(&doc, &registry).is_ready_for_lowering(&doc, &registry)
    );
}

#[test]
fn unusable_changes_rebuild_and_say_why() {
    let registry = registry(None);
    let mut doc = chain();
    let mut session = AnalysisSession::new(&doc, &registry, BTreeMap::new(), options());

    // A skipped change: the session never saw the first commit.
    let _skipped = doc.apply([connect(2, (2, "sum"), (3, "a"))]).unwrap();
    let second = doc
        .apply([DocCommand::Disconnect {
            edge: common::eid(1),
        }])
        .unwrap();
    session.update(&doc, &registry, &second.change_set, options());
    assert_eq!(
        session.stats().work,
        SessionWork::Rebuild(RebuildReason::StateMismatch)
    );
    assert_eq!(session.analysis(), &full(&doc, &registry));

    // Another registry generation, then other limits.
    let replacement = common::generic::registry(Some(vec!["core/i64@1"]));
    let third = doc.apply([connect(1, (1, "value"), (2, "a"))]).unwrap();
    session.update(&doc, &replacement, &third.change_set, options());
    assert_eq!(
        session.stats().work,
        SessionWork::Rebuild(RebuildReason::RegistryChanged)
    );
    assert_eq!(session.analysis(), &full(&doc, &replacement));
    let limits = AnalysisOptions {
        limits: AnalysisLimits {
            max_items: 1,
            ..AnalysisLimits::default()
        },
        cancelled: None,
    };
    let fourth = doc
        .apply([DocCommand::Disconnect {
            edge: common::eid(2),
        }])
        .unwrap();
    session.update(&doc, &replacement, &fourth.change_set, limits);
    assert_eq!(
        session.stats().work,
        SessionWork::Rebuild(RebuildReason::LimitsChanged)
    );
    // An over-quota document gets exactly the full pass's rejection.
    assert_eq!(
        session.analysis(),
        &analyze_with_options(&doc, &replacement, BTreeMap::new(), limits)
    );
}

#[test]
fn leaf_effect_changes_rebuild_and_equal_effects_do_nothing() {
    let registry = registry(None);
    let doc = chain();
    let mut session = AnalysisSession::new(&doc, &registry, BTreeMap::new(), options());
    session.set_leaf_effects(&doc, &registry, BTreeMap::new(), options());
    assert_eq!(session.stats().work, SessionWork::None);
    let effects = BTreeMap::from([(common::nid(2), LeafEffects::Known(vec![]))]);
    session.set_leaf_effects(&doc, &registry, effects.clone(), options());
    assert_eq!(
        session.stats().work,
        SessionWork::Rebuild(RebuildReason::EffectsChanged)
    );
    assert_eq!(
        session.analysis(),
        &analyze_with_options(&doc, &registry, effects, options())
    );
}

#[test]
fn a_cancelled_analysis_is_never_reused() {
    let registry = registry(None);
    let mut doc = chain();
    let cancel = AtomicBool::new(true);
    let cancelled = AnalysisOptions {
        limits: AnalysisLimits::default(),
        cancelled: Some(&cancel),
    };
    let mut session = AnalysisSession::new(&doc, &registry, BTreeMap::new(), cancelled);
    assert!(!session.is_current(&doc, &registry));
    assert_eq!(
        session.analysis(),
        &analyze_with_options(&doc, &registry, BTreeMap::new(), cancelled)
    );
    // Even a layout edit cannot rebind a cancelled result: it reruns in full.
    let committed = doc
        .apply([DocCommand::Layout(LayoutCommand::Set {
            path: vec!["nodes".into(), common::node_id(2)],
            value: Some(json!({"x": 1})),
        })])
        .unwrap();
    session.update(&doc, &registry, &committed.change_set, options());
    assert_eq!(
        session.stats().work,
        SessionWork::Rebuild(RebuildReason::Stale)
    );
    assert!(session.is_current(&doc, &registry));
    assert_eq!(session.analysis(), &full(&doc, &registry));
}

/// Applies random edits (with rollbacks, layout edits, undo and redo) and checks the
/// session against the full pass after every committed change.
fn equals_full_through_edits(
    mut document: GraphDocument,
    registry: &RegistrySnapshot,
    ops: Vec<common::edits::Op>,
) -> Result<(), TestCaseError> {
    let mut session = AnalysisSession::new(&document, registry, BTreeMap::new(), options());
    let mut inverses = Vec::new();
    for op in ops {
        let Some(command) = command(&document, op) else {
            continue;
        };
        if let Ok(committed) = document.apply([command]) {
            session.update(&document, registry, &committed.change_set, options());
            prop_assert_ne!(
                session.stats().work,
                SessionWork::Rebuild(RebuildReason::StateMismatch)
            );
            prop_assert_eq!(session.analysis(), &full(&document, registry));
            inverses.push(committed.inverse);
        }
    }
    let mut redo = Vec::new();
    while let Some(inverse) = inverses.pop() {
        let committed = document.revert(inverse).unwrap();
        session.update(&document, registry, &committed.change_set, options());
        prop_assert_eq!(session.analysis(), &full(&document, registry));
        redo.push(committed.inverse);
    }
    while let Some(inverse) = redo.pop() {
        let committed = document.revert(inverse).unwrap();
        session.update(&document, registry, &committed.change_set, options());
        prop_assert_eq!(session.analysis(), &full(&document, registry));
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn session_equals_full_analysis_from_an_empty_document(
        ops in proptest::collection::vec(op(), 1..16),
    ) {
        equals_full_through_edits(empty_document(), &registry(None), ops)?;
    }

    #[test]
    fn session_equals_full_analysis_from_the_saved_example(
        ops in proptest::collection::vec(op(), 1..16),
    ) {
        let document = from_json(&read_fixture("threshold-v1.json")).unwrap();
        let registry = RegistrySnapshot::core(common::generic::origin());
        equals_full_through_edits(document, &registry, ops)?;
    }
}

/// V12b: an edit rechecks only the edges it can affect.
#[test]
fn edits_recheck_only_the_edges_of_touched_nodes() {
    use common::generic::typed_sink;
    let registry = registry(None);
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for i in 1..=10u8 {
        nodes.push(typed_source(i, "i64"));
        nodes.push(typed_sink(i + 10, "i64", json!({})));
        edges.push(data_edge(i, (i, "value"), (i + 10, "value")));
    }
    let mut doc = document(nodes, edges);
    let mut session = AnalysisSession::new(&doc, &registry, BTreeMap::new(), options());
    let work = |session: &AnalysisSession| {
        let local = session.stats().local;
        (local.edges_computed, local.edge_hits)
    };
    assert_eq!(work(&session), (10, 0));

    // A new unconnected node touches no edge.
    let added = doc
        .apply([DocCommand::AddNode {
            body: orange_model::BodyRef::Root,
            node: serde_json::from_value(typed_source(30, "i64")).unwrap(),
        }])
        .unwrap();
    session.update(&doc, &registry, &added.change_set, options());
    assert_eq!(work(&session), (0, 10));
    assert_eq!(session.analysis(), &full(&doc, &registry));

    // Removing an edge drops only its own entry.
    let removed = doc
        .apply([DocCommand::Disconnect {
            edge: common::eid(3),
        }])
        .unwrap();
    session.update(&doc, &registry, &removed.change_set, options());
    assert_eq!(work(&session), (0, 9));
    assert_eq!(session.analysis(), &full(&doc, &registry));

    // Editing a node rechecks the edges that end at it.
    let edited = doc
        .apply([DocCommand::SetLiteral {
            port: orange_model::document::PortRef {
                node: common::nid(14),
                port: "value".parse().unwrap(),
            },
            literal: Some(
                serde_json::from_value(json!({"type":"core/i64@1","value":"1"})).unwrap(),
            ),
        }])
        .unwrap();
    session.update(&doc, &registry, &edited.change_set, options());
    assert_eq!(work(&session), (1, 8));
    assert_eq!(session.analysis(), &full(&doc, &registry));
}

/// V12b: generic components are solved again only when something they read changed,
/// including an edge that joins a component without touching its nodes.
#[test]
fn edits_resolve_only_the_generic_components_they_reach() {
    let registry = registry(None);
    let mut doc = document(
        vec![
            typed_source(1, "i64"),
            add(2, json!({})),
            typed_source(3, "i64"),
            add(4, json!({})),
        ],
        vec![data_edge(1, (1, "value"), (2, "a"))],
    );
    let mut session = AnalysisSession::new(&doc, &registry, BTreeMap::new(), options());
    let work = |session: &AnalysisSession| {
        let local = session.stats().local;
        (local.components_computed, local.component_hits)
    };
    assert_eq!(session.analysis(), &full(&doc, &registry));
    let (computed, _) = work(&session);
    assert_eq!(computed, 2);

    // Wiring the free add node changes only its own component.
    let wired = doc.apply([connect(2, (3, "value"), (4, "a"))]).unwrap();
    session.update(&doc, &registry, &wired.change_set, options());
    assert_eq!(session.analysis(), &full(&doc, &registry));
    let (computed, hits) = work(&session);
    assert_eq!(computed, 1);
    assert!(hits >= 1);

    // A layout-free unrelated edit reuses both.
    let added = doc
        .apply([DocCommand::AddNode {
            body: orange_model::BodyRef::Root,
            node: serde_json::from_value(typed_source(9, "i64")).unwrap(),
        }])
        .unwrap();
    session.update(&doc, &registry, &added.change_set, options());
    assert_eq!(session.analysis(), &full(&doc, &registry));
    assert_eq!(work(&session).0, 0);
}
