//! NFR-15 visit counters: a scoped edit reports the work of its cone rather than of the
//! graph, and an edit that falls back to the full analysis says why (16 §3).
#[path = "support/data_graph.rs"]
#[allow(dead_code)]
mod support;

use std::collections::BTreeMap;

use orange_model::document::PortRef;
use orange_model::validation::{
    AnalysisOptions, AnalysisSession, PassVisits, RebuildReason, ScopeFallback, SessionWork,
    analyze_with_options,
};
use orange_model::{DocCommand, GraphDocument, LayoutCommand};
use serde_json::{Value as Json, json};
use support::*;

const CHAINS: u32 = 500;
const LENGTH: u32 = 10;

/// `CHAINS` independent chains of `LENGTH` adds, each `a` but the first wired to the add
/// before it.
fn chains() -> (
    GraphDocument,
    std::sync::Arc<orange_model::catalog::RegistrySnapshot>,
) {
    let (package, registry) = catalog();
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for chain in 0..CHAINS {
        let first = 10 + chain * LENGTH;
        nodes.push(math(first, "add", 0, 1));
        for link in 1..LENGTH {
            let n = first + link;
            nodes.push(math(n, "add", 0, 1));
            edges.push(edge(n, n - 1, "sum", n, "a"));
        }
    }
    (graph(&package, nodes, edges, vec![]), registry)
}

fn set_b(node: u32, literal: Json) -> DocCommand {
    DocCommand::SetLiteral {
        port: PortRef {
            node: id(node).parse().unwrap(),
            port: "b".parse().unwrap(),
        },
        literal: Some(serde_json::from_value(literal).unwrap()),
    }
}

#[test]
fn a_scoped_literal_edit_visits_its_cone_and_a_fallback_says_why() {
    let (mut doc, registry) = chains();
    let options = AnalysisOptions::default();
    let full = |doc: &GraphDocument| analyze_with_options(doc, &registry, BTreeMap::new(), options);
    let mut session = AnalysisSession::new(&doc, &registry, BTreeMap::new(), options);
    assert!(session.analysis().report().is_valid());
    let total = (CHAINS * LENGTH) as usize;

    // Creation charges every pass the whole document.
    let created = *session.stats();
    assert_eq!(created.work, SessionWork::Rebuild(RebuildReason::Created));
    assert_eq!(created.fallback, None);
    assert_eq!(created.visits.resolution.nodes, total);
    assert_eq!(created.visits.resolution.bodies, 1);

    // A literal in the middle of one chain re-checks only around that add.
    let middle = 10 + (CHAINS / 2) * LENGTH + LENGTH / 2;
    let committed = doc.apply([set_b(middle, literal(7))]).unwrap();
    let stats = *session.update(&doc, &registry, &committed.change_set, options);
    assert!(stats.is_scoped(), "{stats:?}");
    assert_eq!(stats.fallback, None);
    assert_eq!(session.analysis(), &full(&doc));
    let visits = stats.visits.total();
    assert!(visits.nodes > 0 && visits.ports > 0, "{visits:?}");
    assert_eq!(visits.bodies, 1);
    assert!(
        visits.nodes * 100 < total && visits.edges * 100 < total,
        "{visits:?} on {total} nodes"
    );
    assert_eq!(stats.visits.inputs.nodes, 1);
    assert_eq!(stats.visits.resolution.bodies, 1);
    // A literal edit reads no edge check, order, flow, zone or function pass.
    for pass in [
        stats.visits.connections,
        stats.visits.order,
        stats.visits.availability,
        stats.visits.control_flow,
        stats.visits.state,
        stats.visits.zones,
        stats.visits.functions,
    ] {
        assert!(pass.is_zero(), "{:?}", stats.visits);
    }

    // An i64 literal conflicts with the chain's u64 values: the inference result changes,
    // which only the full analysis publishes.
    let committed = doc
        .apply([set_b(middle, json!({"type": "core/i64@1", "value": "1"}))])
        .unwrap();
    let stats = *session.update(&doc, &registry, &committed.change_set, options);
    assert_eq!(stats.work, SessionWork::Recompute);
    assert_eq!(stats.fallback, Some(ScopeFallback::Resolution));
    assert_eq!(stats.visits.resolution.nodes, total);
    assert_eq!(session.analysis(), &full(&doc));

    // A layout-only edit visits nothing.
    let moved = doc
        .apply([DocCommand::Layout(LayoutCommand::Set {
            path: vec!["nodes".into(), id(middle)],
            value: Some(json!({"x": 40})),
        })])
        .unwrap();
    let stats = *session.update(&doc, &registry, &moved.change_set, options);
    assert_eq!(stats.work, SessionWork::None);
    assert_eq!(stats.visits, PassVisits::ZERO);
    assert_eq!(stats.fallback, None);
}

#[test]
fn an_edit_the_journal_does_not_scope_reports_it() {
    let (mut doc, registry) = chains();
    let options = AnalysisOptions::default();
    let mut session = AnalysisSession::new(&doc, &registry, BTreeMap::new(), options);
    // A literal edit together with a new variable declaration is neither a node-field
    // edit nor a structure edit.
    let variable = serde_json::from_value(json!({
        "id": "40000000-0000-4000-8000-000000000001", "name": "total", "type": "core/u64@1",
        "scope": "run"
    }))
    .unwrap();
    let committed = doc
        .apply([set_b(10, literal(3)), DocCommand::UpsertVariable(variable)])
        .unwrap();
    let stats = *session.update(&doc, &registry, &committed.change_set, options);
    assert_eq!(stats.work, SessionWork::Recompute);
    assert_eq!(stats.fallback, Some(ScopeFallback::UnscopedChange));
    assert_eq!(
        session.analysis(),
        &analyze_with_options(&doc, &registry, BTreeMap::new(), options)
    );
}
