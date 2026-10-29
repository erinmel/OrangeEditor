//! The background analysis worker publishes only results for the newest request and its
//! current content, retains a bounded number of snapshots and stops when the host says
//! (16 §3).

mod common;

use std::sync::Arc;

use common::{add, empty_document};
use orange_model::validation::{AnalysisLimits, AnalysisWorker, AnalysisWorkerError};
use orange_model::{DocCommand, GraphDocument, LayoutCommand, analyze};
use serde_json::json;

fn registry() -> Arc<orange_model::catalog::RegistrySnapshot> {
    common::generic::registry(None)
}

fn worker() -> AnalysisWorker {
    AnalysisWorker::spawn(AnalysisLimits::default()).unwrap()
}

fn snapshot(document: &GraphDocument) -> Arc<GraphDocument> {
    Arc::new(document.clone())
}

#[test]
fn rapid_edits_publish_only_the_newest_content() {
    let registry = registry();
    let mut worker = worker();
    let mut document = empty_document();
    let mut states = Vec::new();
    for n in 1..=20 {
        let _ = document.apply([add(n)]).unwrap();
        states.push(document.clone());
        worker
            .submit(snapshot(&document), &registry, Default::default())
            .unwrap();
    }
    let newest = worker.wait().unwrap().clone();
    assert!(newest.is_current(&document, &registry));
    assert_eq!(newest.report(), analyze(&document, &registry).report());
    // Earlier contents never see the newest result as theirs.
    for earlier in &states[..states.len() - 1] {
        assert!(worker.current(earlier, &registry).is_none());
    }
}

#[test]
fn a_burst_of_submissions_retains_at_most_the_active_and_newest_snapshots() {
    let registry = registry();
    let mut worker = worker();
    let mut document = empty_document();
    let mut snapshots = Vec::new();
    for n in 1..=200 {
        let _ = document.apply([add(n)]).unwrap();
        let submitted = snapshot(&document);
        snapshots.push(Arc::clone(&submitted));
        worker
            .submit(submitted, &registry, Default::default())
            .unwrap();
    }
    // Besides this test, only the pending newest one and at most one in analysis remain.
    let (newest, earlier) = snapshots.split_last().unwrap();
    let retained = earlier
        .iter()
        .filter(|earlier| Arc::strong_count(earlier) > 1)
        .count();
    assert!(
        retained <= 1,
        "{retained} superseded snapshots are retained"
    );
    assert!(worker.wait().unwrap().is_current(newest, &registry));
    assert!(
        earlier
            .iter()
            .all(|earlier| Arc::strong_count(earlier) == 1)
    );
}

#[test]
fn stale_results_are_withheld_until_the_content_is_resubmitted() {
    let registry = registry();
    let mut worker = worker();
    let mut document = empty_document();
    let _ = document.apply([add(1)]).unwrap();
    worker
        .submit(snapshot(&document), &registry, Default::default())
        .unwrap();
    assert!(worker.wait().is_some());
    assert!(worker.current(&document, &registry).is_some());

    // A layout edit keeps the semantic state, so the result stays current with no new job.
    let _ = document
        .apply([DocCommand::Layout(LayoutCommand::Set {
            path: vec!["zoom".into()],
            value: Some(json!(2)),
        })])
        .unwrap();
    assert!(worker.current(&document, &registry).is_some());

    // A semantic edit makes it stale; undo returns to content that was never submitted
    // after the edit, so it stays withheld until it is submitted again.
    let edited = document.apply([add(2)]).unwrap();
    assert!(worker.current(&document, &registry).is_none());
    worker
        .submit(snapshot(&document), &registry, Default::default())
        .unwrap();
    assert!(worker.wait().unwrap().is_current(&document, &registry));
    let _ = document.revert(edited.inverse).unwrap();
    assert!(worker.current(&document, &registry).is_none());
    worker
        .submit(snapshot(&document), &registry, Default::default())
        .unwrap();
    assert!(worker.wait().unwrap().is_current(&document, &registry));

    // A replaced registry snapshot is stale for the same content.
    let replaced = common::generic::registry(Some(vec![]));
    assert!(worker.current(&document, &replaced).is_none());
    worker
        .submit(snapshot(&document), &replaced, Default::default())
        .unwrap();
    assert!(worker.wait().unwrap().is_current(&document, &replaced));
}

#[test]
fn a_worker_without_submissions_has_no_result_and_drops_cleanly() {
    let registry = registry();
    let mut worker = worker();
    let document: GraphDocument = empty_document();
    assert!(worker.current(&document, &registry).is_none());
    assert!(worker.wait().is_none());
    // Dropping requests shutdown of a submission still running without waiting for it.
    worker
        .submit(snapshot(&document), &registry, Default::default())
        .unwrap();
    drop(worker);
}

#[test]
fn shutdown_refuses_new_work_and_join_stops_the_thread() {
    let registry = registry();
    let mut worker = worker();
    let document = empty_document();
    worker
        .submit(snapshot(&document), &registry, Default::default())
        .unwrap();
    worker.request_shutdown();
    assert_eq!(
        worker.submit(snapshot(&document), &registry, Default::default()),
        Err(AnalysisWorkerError::Closed)
    );
    // The cancelled submission is never published, so waiting returns instead of blocking.
    let _ = worker.wait();
    assert_eq!(worker.join(), Ok(()));
    assert_eq!(
        AnalysisWorkerError::Closed.message_key(),
        "core/analysis-worker-closed"
    );
}
