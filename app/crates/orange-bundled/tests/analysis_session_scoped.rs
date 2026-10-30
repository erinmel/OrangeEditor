//! V12b: a literal or data-edge edit is re-checked only where it can reach and patched into
//! the retained report; every case that might change more falls back to the full analysis
//! (16 §3).
#[path = "support/data_graph.rs"]
#[allow(dead_code)]
mod support;

use std::collections::BTreeMap;

use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{Edge, NodeInstance, PortRef};
use orange_model::validation::{
    AnalysisLimits, AnalysisOptions, AnalysisSession, SessionWork, analyze_with_options,
};
use orange_model::{BodyRef, DocCommand, GraphDocument};
use serde_json::{Value as Json, json};
use support::*;

fn set(node: u32, port: &str, value: Option<Json>) -> DocCommand {
    DocCommand::SetLiteral {
        port: PortRef {
            node: id(node).parse().unwrap(),
            port: port.parse().unwrap(),
        },
        literal: value.map(|value| serde_json::from_value(value).unwrap()),
    }
}

fn u64_text(value: &str) -> Option<Json> {
    Some(json!({"type": "core/u64@1", "value": value}))
}

/// Applies `commands`, updates the session and checks it against the full analysis.
fn edit(
    session: &mut AnalysisSession,
    doc: &mut GraphDocument,
    registry: &RegistrySnapshot,
    options: AnalysisOptions<'_>,
    commands: Vec<DocCommand>,
) -> SessionWork {
    let committed = doc.apply(commands).unwrap();
    let work = session
        .update(doc, registry, &committed.change_set, options)
        .work;
    assert_eq!(
        session.analysis(),
        &analyze_with_options(doc, registry, BTreeMap::new(), options)
    );
    work
}

/// Ten chains of three adds; every `a` but the first in a chain is wired.
fn chains() -> (GraphDocument, std::sync::Arc<RegistrySnapshot>) {
    let (package, registry) = catalog();
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for chain in 0..10 {
        let first = 10 + chain * 3;
        for link in 0..3 {
            nodes.push(math(first + link, "add", 0, 1));
            if link > 0 {
                edges.push(edge(
                    first + link,
                    first + link - 1,
                    "sum",
                    first + link,
                    "a",
                ));
            }
        }
    }
    (graph(&package, nodes, edges, vec![]), registry)
}

fn session(doc: &GraphDocument, registry: &RegistrySnapshot) -> AnalysisSession {
    let session = AnalysisSession::new(doc, registry, BTreeMap::new(), AnalysisOptions::default());
    assert!(session.analysis().report().is_valid());
    session
}

#[test]
fn literals_that_keep_every_signature_patch_only_their_inputs() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(11, "b", u64_text("5"))],
    );
    assert_eq!(work, SessionWork::Scoped);
    let stats = *session.stats();
    assert_eq!(stats.full_analysis_runs, 0);
    // The chain's three adds, and nothing from the other nine chains.
    assert_eq!(stats.input_nodes, 3);
    assert!(session.changed_nodes().is_empty());
    assert!(session.changed_subjects().is_empty());

    // Removing a wired literal drops its shadowed warning; restoring it brings it back.
    let shadowed = session.diagnostics().len();
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(11, "a", None)],
    );
    assert_eq!(work, SessionWork::Scoped);
    assert_eq!(session.diagnostics().len(), shadowed - 1);
    assert_eq!(session.changed_subjects().len(), 1);
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(11, "a", u64_text("2"))],
    );
    assert_eq!(work, SessionWork::Scoped);
    assert_eq!(session.diagnostics().len(), shadowed);

    // Undo and redo are literal edits too.
    let committed = doc.apply([set(14, "b", u64_text("3"))]).unwrap();
    session.update(&doc, &registry, &committed.change_set, options);
    let undone = doc.revert(committed.inverse).unwrap();
    session.update(&doc, &registry, &undone.change_set, options);
    assert_eq!(session.stats().work, SessionWork::Scoped);
    assert_eq!(
        session.analysis(),
        &analyze_with_options(&doc, &registry, BTreeMap::new(), options)
    );
}

#[test]
fn a_literal_that_changes_inference_runs_the_full_analysis() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    // An i64 literal conflicts with the chain's u64 literals.
    let other = Some(json!({"type": "core/i64@1", "value": "1"}));
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(10, "b", other)],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(10, "b", u64_text("1"))],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(session.analysis().report().is_valid());
}

#[test]
fn gaining_or_losing_an_input_error_runs_the_full_analysis() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(11, "b", u64_text("x"))],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
    // Still invalid: the error status holds, so the edit stays scoped.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(11, "b", u64_text("y"))],
    );
    assert_eq!(work, SessionWork::Scoped);
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(11, "b", u64_text("1"))],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(session.analysis().report().is_valid());
}

#[test]
fn literals_with_structural_edits_run_the_full_analysis() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let connect = DocCommand::Connect {
        edge: serde_json::from_value::<Edge>(edge(99, 12, "sum", 13, "a")).unwrap(),
    };
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(11, "b", u64_text("1")), connect],
    );
    assert_eq!(work, SessionWork::Recompute);
    let node: NodeInstance = serde_json::from_value(math(90, "add", 1, 2)).unwrap();
    let add = DocCommand::AddNode {
        body: BodyRef::Root,
        node,
    };
    assert_eq!(
        edit(&mut session, &mut doc, &registry, options, vec![add]),
        SessionWork::Recompute
    );
}

#[test]
fn a_literal_past_the_admission_quota_is_rejected_as_in_full() {
    let (mut doc, registry) = chains();
    let limited = |bytes: usize| AnalysisOptions {
        limits: AnalysisLimits {
            max_metadata_bytes: bytes,
            ..AnalysisLimits::default()
        },
        cancelled: None,
    };
    let fits = |bytes: usize, doc: &GraphDocument| {
        analyze_with_options(doc, &registry, BTreeMap::new(), limited(bytes))
            .report()
            .is_valid()
    };
    // The smallest metadata quota the document fits.
    let (mut low, mut high) = (0, 1 << 20);
    assert!(fits(high, &doc));
    while low + 1 < high {
        let middle = (low + high) / 2;
        if fits(middle, &doc) {
            high = middle;
        } else {
            low = middle;
        }
    }
    let options = limited(high + 2);
    let mut session = AnalysisSession::new(&doc, &registry, BTreeMap::new(), options);
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(11, "b", u64_text("12"))],
    );
    assert_eq!(work, SessionWork::Scoped);
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(11, "b", u64_text("123456789"))],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
}

fn connect(n: u32, from: u32, to: u32, input: &str) -> DocCommand {
    DocCommand::Connect {
        edge: serde_json::from_value::<Edge>(edge(n, from, "sum", to, input)).unwrap(),
    }
}

fn disconnect(n: u32) -> DocCommand {
    DocCommand::Disconnect {
        edge: serde_json::from_value::<Edge>(edge(n, 0, "sum", 0, "a"))
            .unwrap()
            .id,
    }
}

#[test]
fn data_edges_between_pure_nodes_patch_only_their_targets() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let shadowed = session.diagnostics().len();
    // The wired `a` literal is no longer shadowed once its edge goes.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![disconnect(11)],
    );
    assert_eq!(work, SessionWork::Scoped);
    assert_eq!(session.stats().full_analysis_runs, 0);
    assert_eq!(session.diagnostics().len(), shadowed - 1);
    assert!(session.changed_nodes().is_empty());
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![connect(11, 10, 11, "a")],
    );
    assert_eq!(work, SessionWork::Scoped);
    assert_eq!(session.diagnostics().len(), shadowed);

    // Joining two chains merges their components without changing a signature.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![connect(90, 12, 13, "a")],
    );
    assert_eq!(work, SessionWork::Scoped);
    // A second consumer of one output is fine for a plain value.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![connect(91, 12, 16, "a")],
    );
    assert_eq!(work, SessionWork::Scoped);

    // Undo and redo of an edge are edge edits too.
    let committed = doc.apply([connect(92, 18, 19, "a")]).unwrap();
    session.update(&doc, &registry, &committed.change_set, options);
    assert_eq!(session.stats().work, SessionWork::Scoped);
    let undone = doc.revert(committed.inverse).unwrap();
    session.update(&doc, &registry, &undone.change_set, options);
    assert_eq!(session.stats().work, SessionWork::Scoped);
    assert_eq!(
        session.analysis(),
        &analyze_with_options(&doc, &registry, BTreeMap::new(), options)
    );
}

#[test]
fn edges_that_may_change_more_run_the_full_analysis() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    // A second wire into a wired input.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![connect(90, 13, 11, "a")],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
    // Nothing is checked locally while the report has an error.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![connect(91, 15, 16, "b")],
    );
    assert_eq!(work, SessionWork::Recompute);
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![disconnect(90)],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(session.analysis().report().is_valid());

    // An edge closing a cycle.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![connect(92, 12, 10, "a")],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
}

#[test]
fn edges_feeding_an_export_run_the_full_analysis() {
    let (package, registry) = catalog();
    let nodes = (0..3).map(|link| math(10 + link, "add", 0, 1)).collect();
    let edges = vec![edge(11, 10, "sum", 11, "a")];
    let mut doc = graph(&package, nodes, edges, vec![export(12, "sum")]);
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![connect(12, 11, 12, "a")],
    );
    assert_eq!(work, SessionWork::Recompute);
    // Removing an edge upstream of the export runs in full too.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![disconnect(11)],
    );
    assert_eq!(work, SessionWork::Recompute);
}

#[test]
fn an_edge_from_an_event_payload_runs_the_full_analysis() {
    let (package, registry) = catalog();
    let event = "40000000-0000-4000-8000-000000000001";
    let on_event = json!({"id": id(50), "definition_id": "core/on-event",
        "definition_version": 1, "config_version": 1, "config": {"event": event},
        "activation": {"mode": "queue", "capacity": 4, "overflow": "drop-oldest",
            "on_fault": "continue", "order": 0}});
    let base = graph(
        &package,
        vec![on_event, math(60, "add", 0, 1)],
        vec![],
        vec![],
    );
    let mut raw: Json = serde_json::from_str(&orange_model::to_json(&base).unwrap()).unwrap();
    raw["events"] = json!([{"id": event, "name": "tick", "payload": "core/u64@1", "scope": "run"}]);
    let mut doc = orange_model::from_json(&raw.to_string()).unwrap();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let payload = DocCommand::Connect {
        edge: serde_json::from_value::<Edge>(edge(61, 50, "payload", 60, "a")).unwrap(),
    };
    // The payload carries an availability requirement to whatever consumes the add.
    let work = edit(&mut session, &mut doc, &registry, options, vec![payload]);
    assert_eq!(work, SessionWork::Recompute);
}
