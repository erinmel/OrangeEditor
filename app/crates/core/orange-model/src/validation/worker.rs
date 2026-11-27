//! Combined analysis on an owned background thread, so an editor never runs a whole-graph
//! pass on its UI thread (16 §3).
//!
//! The host submits immutable snapshots of each semantic document state it wants analyzed,
//! and every submission gets its own [`AnalysisRequestId`]. The worker analyzes only the
//! newest request, cancels the one it supersedes at the next pass boundary and discards
//! cancelled results. [`AnalysisWorker::current`] returns a finished analysis only for the
//! newest request and only while it still belongs to the host's document and registry, so
//! a rapid run of edits, undos, effect or registry updates never shows a result for
//! content or evidence that is gone. Layout edits keep the semantic state, so they need no
//! submission and keep the current result.
//!
//! The mailbox between host and thread holds at most one pending request and one finished
//! analysis; a newer submission or result replaces the older one, so a burst of edits
//! retains at most the snapshot being analyzed plus the newest pending one. Submission
//! never blocks on analysis. The host owns the thread's lifetime: [`request_shutdown`]
//! returns promptly, and [`join`] waits for the thread away from the host's UI thread.
//!
//! [`request_shutdown`]: AnalysisWorker::request_shutdown
//! [`join`]: AnalysisWorker::join

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};

use super::{
    AnalysisLimits, AnalysisOptions, LeafEffects, ValidationAnalysis, analyze_with_options,
};
use crate::catalog::RegistrySnapshot;
use crate::document::GraphDocument;
use crate::id::NodeId;

/// Identity of one submission to an [`AnalysisWorker`]. Every submission gets a new,
/// larger one, also when only the effect evidence changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AnalysisRequestId(u64);

/// A background analysis worker can no longer analyze submissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AnalysisWorkerError {
    #[error("the analysis worker has shut down")]
    Closed,
    #[error("the analysis worker stopped unexpectedly")]
    Panicked,
}

impl AnalysisWorkerError {
    /// Stable message key for host presentation (13 §2).
    pub const fn message_key(&self) -> &'static str {
        match self {
            Self::Closed => "core/analysis-worker-closed",
            Self::Panicked => "core/analysis-worker-panicked",
        }
    }
}

/// One requested analysis. `cancel` is set when a newer submission supersedes it.
struct Job {
    request: AnalysisRequestId,
    document: Arc<GraphDocument>,
    registry: Arc<RegistrySnapshot>,
    effects: BTreeMap<NodeId, LeafEffects>,
    cancel: Arc<AtomicBool>,
}

struct Completed {
    request: AnalysisRequestId,
    analysis: ValidationAnalysis,
}

/// Capacity-one exchange between host and thread. It is locked only to move a value in or
/// out, never while analyzing.
#[derive(Default)]
struct Mailbox {
    pending: Option<Job>,
    completed: Option<Completed>,
    /// The host asked the thread to stop.
    closed: bool,
    /// The thread has stopped, normally or by unwinding.
    ended: bool,
}

#[derive(Default)]
struct Shared {
    mailbox: Mutex<Mailbox>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Mailbox> {
        // Analysis never runs under the lock, so a poisoned mailbox is still consistent.
        self.mailbox.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Marks the mailbox ended when the thread exits, also by unwinding, and wakes the host.
struct EndGuard(Arc<Shared>);

impl Drop for EndGuard {
    fn drop(&mut self) {
        self.0.lock().ended = true;
        self.0.changed.notify_all();
    }
}

/// A background analysis thread owned by one host session. Dropping it requests shutdown
/// without waiting for the analysis in progress; call [`join`](Self::join) to wait.
pub struct AnalysisWorker {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    /// Cancellation flag of the newest submission.
    cancel: Arc<AtomicBool>,
    next: u64,
    /// The newest submission.
    submitted: Option<AnalysisRequestId>,
    /// The newest submission's analysis once it is collected from the mailbox.
    latest: Option<Completed>,
}

impl AnalysisWorker {
    /// Starts the worker thread. Every analysis uses `limits`.
    pub fn spawn(limits: AnalysisLimits) -> std::io::Result<Self> {
        let shared = Arc::new(Shared::default());
        let thread = thread::Builder::new()
            .name("orange-analysis".into())
            .spawn({
                let shared = Arc::clone(&shared);
                move || analyze_submissions(&shared, limits)
            })?;
        Ok(Self {
            shared,
            thread: Some(thread),
            cancel: Arc::new(AtomicBool::new(false)),
            next: 0,
            submitted: None,
            latest: None,
        })
    }

    /// Queues `document` with `registry` and `effects` for analysis, replacing a pending
    /// submission and cancelling the one in progress at its next pass boundary. The host
    /// builds the immutable snapshot, where and when it chooses, and keeps editing its own
    /// document. Skip layout-only changes: they leave the semantic state, and the result,
    /// unchanged. Fails once the worker has shut down or stopped.
    pub fn submit(
        &mut self,
        document: Arc<GraphDocument>,
        registry: &Arc<RegistrySnapshot>,
        effects: BTreeMap<NodeId, LeafEffects>,
    ) -> Result<AnalysisRequestId, AnalysisWorkerError> {
        let mut mailbox = self.shared.lock();
        if mailbox.closed {
            return Err(AnalysisWorkerError::Closed);
        }
        if mailbox.ended {
            return Err(AnalysisWorkerError::Panicked);
        }
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.next += 1;
        let request = AnalysisRequestId(self.next);
        let replaced = mailbox.pending.replace(Job {
            request,
            document,
            registry: Arc::clone(registry),
            effects,
            cancel: Arc::clone(&self.cancel),
        });
        let obsolete = mailbox.completed.take();
        drop(mailbox);
        self.shared.changed.notify_all();
        self.submitted = Some(request);
        self.latest = None;
        // Release superseded snapshots and results outside the lock.
        drop((replaced, obsolete));
        Ok(request)
    }

    /// The newest submission's analysis if it is finished and belongs to `document` and
    /// `registry`; otherwise `None`, also while that analysis is still running.
    pub fn current(
        &mut self,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
    ) -> Option<&ValidationAnalysis> {
        self.collect();
        self.latest
            .as_ref()
            .map(|completed| &completed.analysis)
            .filter(|analysis| analysis.is_current(document, registry))
    }

    /// Blocks until the newest submission is analyzed and returns it; `None` without a
    /// submission or once the worker has stopped without analyzing it.
    pub fn wait(&mut self) -> Option<&ValidationAnalysis> {
        let submitted = self.submitted?;
        if self.latest.is_none() {
            let mut mailbox = self.shared.lock();
            loop {
                if let Some(completed) = mailbox.completed.take_if(|c| c.request == submitted) {
                    self.latest = Some(completed);
                    break;
                }
                if mailbox.ended || mailbox.closed {
                    return None;
                }
                mailbox = self
                    .shared
                    .changed
                    .wait(mailbox)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        }
        self.latest.as_ref().map(|completed| &completed.analysis)
    }

    /// Cancels the analysis in progress and tells the thread to stop at its next pass
    /// boundary; the thread releases the pending submission. Returns without waiting.
    pub fn request_shutdown(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.shared.lock().closed = true;
        self.shared.changed.notify_all();
    }

    /// Requests shutdown and waits for the thread to stop. Hosts call it away from their
    /// UI thread; it reports a thread that stopped by panicking.
    pub fn join(mut self) -> Result<(), AnalysisWorkerError> {
        self.request_shutdown();
        match self.thread.take().map(JoinHandle::join) {
            Some(Err(_)) => Err(AnalysisWorkerError::Panicked),
            _ => Ok(()),
        }
    }

    fn collect(&mut self) {
        let Some(submitted) = self.submitted else {
            return;
        };
        if self.latest.is_none() {
            self.latest = self
                .shared
                .lock()
                .completed
                .take_if(|completed| completed.request == submitted);
        }
    }
}

/// The worker thread: analyze the newest pending job until the host closes the mailbox.
fn analyze_submissions(shared: &Arc<Shared>, limits: AnalysisLimits) {
    let _end = EndGuard(Arc::clone(shared));
    loop {
        let job = {
            let mut mailbox = shared.lock();
            loop {
                if mailbox.closed {
                    let pending = mailbox.pending.take();
                    drop(mailbox);
                    drop(pending);
                    return;
                }
                if let Some(job) = mailbox.pending.take() {
                    break job;
                }
                mailbox = shared
                    .changed
                    .wait(mailbox)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        };
        let options = AnalysisOptions {
            limits,
            cancelled: Some(&job.cancel),
        };
        let analysis = analyze_with_options(&job.document, &job.registry, job.effects, options);
        let mut mailbox = shared.lock();
        // `submit` cancels under this lock, so a superseded result is never published.
        if job.cancel.load(Ordering::Relaxed) {
            continue;
        }
        let replaced = mailbox.completed.replace(Completed {
            request: job.request,
            analysis,
        });
        drop(mailbox);
        shared.changed.notify_all();
        drop(replaced);
    }
}

impl Drop for AnalysisWorker {
    fn drop(&mut self) {
        // The thread holds only shared state, so it stops at its next pass boundary on its
        // own; joining here could stall a UI thread for the rest of that pass.
        self.request_shutdown();
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
