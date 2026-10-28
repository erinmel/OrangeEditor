//! The background analysis worker publishes only results for current content (16 §3).

mod common;

use common::{add, empty_document};
use orange_model::validation::{AnalysisLimits, AnalysisWorker};
use orange_model::{DocCommand, GraphDocument, LayoutCommand, analyze};
use serde_json::json;

fn registry() -> std::sync::Arc<orange_model::catalog::RegistrySnapshot> {
    common::generic::registry(None)
}

fn worker() -> AnalysisWorker {
    AnalysisWorker::spawn(AnalysisLimits::default()).unwrap()
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
        worker.submit(&document, &registry, Default::default());
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
fn stale_results_are_withheld_until_the_content_is_resubmitted() {
    let registry = registry();
    let mut worker = worker();
    let mut document = empty_document();
    let _ = document.apply([add(1)]).unwrap();
    worker.submit(&document, &registry, Default::default());
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
    worker.submit(&document, &registry, Default::default());
    assert!(worker.wait().unwrap().is_current(&document, &registry));
    let _ = document.revert(edited.inverse).unwrap();
    assert!(worker.current(&document, &registry).is_none());
    worker.submit(&document, &registry, Default::default());
    assert!(worker.wait().unwrap().is_current(&document, &registry));

    // A replaced registry snapshot is stale for the same content.
    let replaced = common::generic::registry(Some(vec![]));
    assert!(worker.current(&document, &replaced).is_none());
    worker.submit(&document, &replaced, Default::default());
    assert!(worker.wait().unwrap().is_current(&document, &replaced));
}

#[test]
fn a_worker_without_submissions_has_no_result_and_drops_cleanly() {
    let registry = registry();
    let mut worker = worker();
    let document: GraphDocument = empty_document();
    assert!(worker.current(&document, &registry).is_none());
    assert!(worker.wait().is_none());
    // A submission still running when the worker is dropped is cancelled and joined.
    worker.submit(&document, &registry, Default::default());
    drop(worker);
}
