//! Combined analysis on an owned background thread, so an editor never runs a whole-graph
//! pass on its UI thread (16 §3).
//!
//! The host submits each semantic document state it wants analyzed. The worker analyzes
//! only the newest submission, cancels the one it supersedes at the next pass boundary and
//! discards cancelled results. [`AnalysisWorker::current`] returns a finished analysis only
//! while it still belongs to the host's document and registry, so a rapid run of edits,
//! undos or registry updates never shows a result for content that is gone. Layout edits
//! keep the semantic state, so they need no submission and keep the current result.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use super::{
    AnalysisLimits, AnalysisOptions, LeafEffects, ValidationAnalysis, analyze_with_options,
};
use crate::catalog::RegistrySnapshot;
use crate::document::GraphDocument;
use crate::id::NodeId;

/// One requested analysis. `cancel` is set when a newer submission supersedes it.
struct Job {
    document: Arc<GraphDocument>,
    registry: Arc<RegistrySnapshot>,
    effects: BTreeMap<NodeId, LeafEffects>,
    cancel: Arc<AtomicBool>,
}

/// A background analysis thread owned by one host session. Dropping it cancels the work
/// in progress and joins the thread.
pub struct AnalysisWorker {
    jobs: Option<Sender<Job>>,
    results: Receiver<ValidationAnalysis>,
    thread: Option<JoinHandle<()>>,
    /// Cancellation flag of the newest submission.
    cancel: Arc<AtomicBool>,
    /// Semantic state and registry generation of the newest submission.
    submitted: Option<(u64, u64)>,
    latest: Option<ValidationAnalysis>,
}

impl AnalysisWorker {
    /// Starts the worker thread. Every analysis uses `limits`.
    pub fn spawn(limits: AnalysisLimits) -> std::io::Result<Self> {
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (outbox, results) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("orange-analysis".into())
            .spawn(move || {
                while let Ok(mut job) = inbox.recv() {
                    // Only the newest queued submission matters.
                    while let Ok(newer) = inbox.try_recv() {
                        job = newer;
                    }
                    let options = AnalysisOptions {
                        limits,
                        cancelled: Some(&job.cancel),
                    };
                    let analysis =
                        analyze_with_options(&job.document, &job.registry, job.effects, options);
                    if job.cancel.load(Ordering::Relaxed) {
                        continue;
                    }
                    if outbox.send(analysis).is_err() {
                        return;
                    }
                }
            })?;
        Ok(Self {
            jobs: Some(jobs),
            results,
            thread: Some(thread),
            cancel: Arc::new(AtomicBool::new(false)),
            submitted: None,
            latest: None,
        })
    }

    /// Queues `document`'s current semantic content for analysis and cancels the previous
    /// submission. The document is copied, so the host keeps editing its own. Skip
    /// layout-only changes: they leave the semantic state, and the result, unchanged.
    pub fn submit(
        &mut self,
        document: &GraphDocument,
        registry: &Arc<RegistrySnapshot>,
        effects: BTreeMap<NodeId, LeafEffects>,
    ) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.submitted = Some((document.semantic_state_id(), registry.generation()));
        let job = Job {
            document: Arc::new(document.clone()),
            registry: Arc::clone(registry),
            effects,
            cancel: Arc::clone(&self.cancel),
        };
        if let Some(jobs) = &self.jobs {
            // A closed channel means the thread ended; `current` then reports no result.
            let _ = jobs.send(job);
        }
    }

    /// The newest finished analysis if it belongs to `document` and `registry`; otherwise
    /// `None`, also while the analysis of the current content is still running.
    pub fn current(
        &mut self,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
    ) -> Option<&ValidationAnalysis> {
        while let Ok(analysis) = self.results.try_recv() {
            self.latest = Some(analysis);
        }
        self.latest
            .as_ref()
            .filter(|analysis| analysis.is_current(document, registry))
    }

    /// Blocks until the newest submission is analyzed and returns it; `None` without a
    /// submission or if the worker thread has ended.
    pub fn wait(&mut self) -> Option<&ValidationAnalysis> {
        let submitted = self.submitted?;
        let identity = |analysis: &ValidationAnalysis| {
            (analysis.document_state(), analysis.registry_generation())
        };
        if self.latest.as_ref().map(identity) != Some(submitted) {
            loop {
                let analysis = self.results.recv().ok()?;
                let done = identity(&analysis) == submitted;
                self.latest = Some(analysis);
                if done {
                    break;
                }
            }
        }
        self.latest.as_ref()
    }
}

impl Drop for AnalysisWorker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.jobs = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl std::fmt::Debug for AnalysisWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalysisWorker")
            .field("submitted", &self.submitted)
            .field("has_result", &self.latest.is_some())
            .finish_non_exhaustive()
    }
}
