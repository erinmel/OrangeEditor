//! Comparable host-side authoring experiments. This is measurement tooling, not a UI API.
//! Both candidates retain AnalysisSession and preserve every accepted command. Capacity-one
//! channels require nonblocking admission/retry; explicit waiting is excluded from caller-work
//! time and included in newest-result latency. No newest-snapshot coalescing is assumed.
use crate::counter::{self, Counts, Snapshot};
use crate::graph::{Source, edge, id, literal, math};
use crate::harness::Config;
use orange_model::catalog::RegistrySnapshot;
use orange_model::validation::{
    AnalysisOptions, AnalysisSession, DiagnosticCode, LeafEffects, SessionWork,
    analyze_with_options,
};
use orange_model::{ChangeSet, DocCommand, GraphDocument, Inverse, LayoutCommand};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
enum Edit {
    Literal(u64),
    Undo,
    Redo,
    Layout,
    MissingReceipt,
    DivergentReceipt,
    Registry,
    Effects,
    Cancel,
    Recover,
    Check,
}
struct State {
    document: GraphDocument,
    registry: Arc<RegistrySnapshot>,
    effects: BTreeMap<orange_model::id::NodeId, LeafEffects>,
    undo: Vec<Inverse>,
    redo: Vec<Inverse>,
}
fn set(document: &mut GraphDocument, value: u64) -> orange_model::Committed {
    document
        .apply([DocCommand::SetLiteral {
            port: serde_json::from_value(serde_json::json!({"node":id(10),"port":"b"}))
                .expect("port"),
            literal: Some(serde_json::from_value(literal(value)).expect("literal")),
        }])
        .expect("committed edit")
}
impl State {
    fn apply(&mut self, edit: Edit) -> Option<ChangeSet> {
        let committed = match edit {
            Edit::Literal(value) => {
                let committed = set(&mut self.document, value);
                self.redo.clear();
                committed
            }
            Edit::Undo => {
                let old = self.undo.pop().expect("history");
                let committed = self.document.revert(old).expect("undo");
                self.redo.push(committed.inverse);
                return Some(committed.change_set);
            }
            Edit::Redo => {
                let old = self.redo.pop().expect("redo");
                self.document.revert(old).expect("redo committed")
            }
            Edit::Layout => self
                .document
                .apply([DocCommand::Layout(LayoutCommand::Set {
                    path: vec!["viewport".into()],
                    value: Some(serde_json::json!({"zoom":2})),
                })])
                .expect("layout"),
            Edit::MissingReceipt => {
                let first = set(&mut self.document, 80);
                self.undo.push(first.inverse);
                set(&mut self.document, 81)
            }
            Edit::DivergentReceipt => {
                let mut sibling = self.document.clone();
                let foreign = set(&mut sibling, 82);
                let own = set(&mut self.document, 83);
                self.undo.push(own.inverse);
                return Some(foreign.change_set);
            }
            Edit::Registry => {
                // Same registered definitions, distinct generation; no implicit provider rebind.
                self.registry = crate::graph::Source::new(vec![math(900_000, "add", 0, 1)], vec![])
                    .analyzed()
                    .1;
                return None;
            }
            Edit::Effects => {
                self.effects
                    .insert(id(10).parse().expect("id"), LeafEffects::Known(vec![]));
                return None;
            }
            Edit::Cancel | Edit::Recover | Edit::Check => return None,
        };
        self.undo.push(committed.inverse);
        Some(committed.change_set)
    }
}
fn fixture(nodes: u32) -> State {
    let instances = (0..nodes)
        .map(|n| {
            let mut node = math(10 + n, "add", 0, 1);
            if n > 0 {
                node["inputs"]
                    .as_object_mut()
                    .expect("literal inputs")
                    .remove("a");
            }
            node
        })
        .collect();
    let wires = (1..nodes)
        .map(|n| edge(n, 9 + n, "sum", 10 + n, "a"))
        .collect();
    let (document, registry) = Source::new(instances, wires).analyzed();
    State {
        document,
        registry,
        effects: BTreeMap::new(),
        undo: vec![],
        redo: vec![],
    }
}
struct Image {
    document: GraphDocument,
    registry: Arc<RegistrySnapshot>,
    effects: BTreeMap<orange_model::id::NodeId, LeafEffects>,
    receipt: Option<ChangeSet>,
}
enum Job {
    Snapshot { image: Box<Image>, edit: Edit },
    Owned(Edit),
}
struct Request {
    sequence: usize,
    job: Job,
    verify: bool,
    gate: Option<Gate>,
}
struct Gate {
    entered: SyncSender<()>,
    resume: Receiver<()>,
}
#[derive(Debug)]
struct Ack {
    sequence: usize,
    current: bool,
    work: SessionWork,
    worker_time: Duration,
}
fn analyze_job(
    state: &mut State,
    session: &mut AnalysisSession,
    request: Request,
    shutdown: &AtomicBool,
) -> Ack {
    let gated_shutdown = request.gate.is_some();
    if let Some(gate) = request.gate {
        gate.entered
            .send(())
            .expect("accepted analysis reached gate");
        gate.resume
            .recv()
            .expect("shutdown experiment released gate");
    }
    let start = Instant::now();
    let (edit, receipt) = match request.job {
        Job::Snapshot { image, edit } => {
            state.document = image.document;
            state.registry = image.registry;
            state.effects = image.effects;
            (edit, image.receipt)
        }
        Job::Owned(edit) => (edit, state.apply(edit)),
    };
    let cancel_this_job = AtomicBool::new(true);
    let cancelled = if matches!(edit, Edit::Cancel) {
        &cancel_this_job
    } else {
        shutdown
    };
    let options = AnalysisOptions {
        cancelled: Some(cancelled),
        ..Default::default()
    };
    let work = match receipt {
        Some(receipt) => {
            session
                .update(&state.document, &state.registry, &receipt, options)
                .work
        }
        None => {
            session
                .set_leaf_effects(
                    &state.document,
                    &state.registry,
                    state.effects.clone(),
                    options,
                )
                .work
        }
    };
    let worker_time = start.elapsed();
    let current =
        session.is_current(&state.document, &state.registry) && !cancelled.load(Ordering::Relaxed);
    if matches!(edit, Edit::Cancel) || gated_shutdown {
        assert!(
            session
                .analysis()
                .report()
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == DiagnosticCode::AnalysisCancelled)
        );
        assert!(!session.is_current(&state.document, &state.registry));
    }
    if !shutdown.load(Ordering::Relaxed) {
        assert_eq!(current, !matches!(edit, Edit::Cancel));
    }
    if matches!(edit, Edit::Layout) {
        assert_eq!(work, SessionWork::None);
    }
    if matches!(
        edit,
        Edit::MissingReceipt | Edit::DivergentReceipt | Edit::Registry | Edit::Effects
    ) {
        assert!(
            matches!(work, SessionWork::Rebuild(_)),
            "{edit:?}: {work:?}"
        );
    }
    // Oracle checking is never inside the reported update timer. Timing trials leave
    // this off; the separate scenario qualification checks every state.
    if request.verify {
        assert_eq!(
            session.analysis(),
            &analyze_with_options(
                &state.document,
                &state.registry,
                state.effects.clone(),
                options,
            )
        );
    }
    Ack {
        sequence: request.sequence,
        current,
        work,
        worker_time,
    }
}
struct Worker {
    sender: SyncSender<Request>,
    receiver: Receiver<Ack>,
    thread: thread::JoinHandle<()>,
    host: Option<State>,
    cancel: Arc<AtomicBool>,
}
struct Shutdown {
    time: Duration,
    cancelled_results: usize,
}
impl Worker {
    fn spawn(state: State, snapshots: bool) -> Self {
        let (sender, requests) = sync_channel::<Request>(1);
        let (replies, receiver) = sync_channel(1);
        let (initial, host) = if snapshots {
            let copy = State {
                document: state.document.clone(),
                registry: Arc::clone(&state.registry),
                effects: state.effects.clone(),
                undo: vec![],
                redo: vec![],
            };
            (copy, Some(state))
        } else {
            (state, None)
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let closing = Arc::clone(&cancel);
        let thread = thread::spawn(move || {
            let mut state = initial;
            let mut session = AnalysisSession::new(
                &state.document,
                &state.registry,
                state.effects.clone(),
                Default::default(),
            );
            assert!(
                session.analysis().report().diagnostics.is_empty(),
                "clean chain fixture"
            );
            replies
                .send(Ack {
                    sequence: 0,
                    current: true,
                    work: SessionWork::Recompute,
                    worker_time: Duration::ZERO,
                })
                .expect("initial acknowledgement");
            let mut expected = 1;
            while let Ok(request) = requests.recv() {
                assert_eq!(request.sequence, expected);
                expected += 1;
                let ack = analyze_job(&mut state, &mut session, request, &closing);
                if replies.send(ack).is_err() {
                    break;
                }
            }
        });
        let init = receiver.recv().expect("worker initialized");
        assert_eq!(init.sequence, 0);
        Self {
            sender,
            receiver,
            thread,
            host,
            cancel,
        }
    }
    fn prepare(&mut self, sequence: usize, edit: Edit, verify: bool) -> Request {
        let job = match &mut self.host {
            Some(state) => {
                let receipt = state.apply(edit);
                Job::Snapshot {
                    image: Box::new(Image {
                        document: state.document.clone(),
                        registry: Arc::clone(&state.registry),
                        effects: state.effects.clone(),
                        receipt,
                    }),
                    edit,
                }
            }
            None => Job::Owned(edit),
        };
        Request {
            sequence,
            job,
            verify,
            gate: None,
        }
    }
    fn shutdown(self) -> Shutdown {
        let begin = Instant::now();
        self.cancel.store(true, Ordering::Relaxed);
        drop(self.sender);
        // Drain a completed reply so the worker cannot be stranded on its bounded send.
        let mut cancelled_results = 0;
        while let Ok(ack) = self.receiver.recv() {
            cancelled_results += usize::from(!ack.current);
            // Closing publication is disabled regardless of an earlier completion.
        }
        self.thread.join().expect("worker joined");
        drop(self.host);
        Shutdown {
            time: begin.elapsed(),
            cancelled_results,
        }
    }
}
struct BatchTimes {
    caller: Duration,
    completion: Duration,
    newest: Duration,
    worker: Duration,
    full: usize,
    published: usize,
    suppressed: usize,
}
fn batch(worker: &mut Worker, edits: &[Edit], verify: bool, first: usize) -> BatchTimes {
    let start = Instant::now();
    let mut newest_start = start;
    let mut caller = Duration::ZERO;
    let mut worker_time = Duration::ZERO;
    let mut completed = 0;
    let mut full = 0;
    let mut published = 0;
    let mut suppressed = 0;
    for (offset, &edit) in edits.iter().enumerate() {
        let begin = Instant::now();
        if offset + 1 == edits.len() {
            newest_start = begin;
        }
        let mut pending = worker.prepare(first + offset, edit, verify);
        caller += begin.elapsed();
        loop {
            let begin = Instant::now();
            let outcome = worker.sender.try_send(pending);
            caller += begin.elapsed();
            match outcome {
                Ok(()) => break,
                Err(TrySendError::Full(request)) => {
                    pending = request;
                    // This explicit wait models host backpressure between attempts. It is
                    // in newest latency, not caller-work. A UI would poll on a later frame.
                    let ack = worker.receiver.recv().expect("progress");
                    assert_eq!(ack.sequence, first + completed);
                    assert_eq!(ack.current, !matches!(edits[completed], Edit::Cancel));
                    let publish = ack.current && ack.sequence == first + offset;
                    published += usize::from(publish);
                    suppressed += usize::from(!publish);
                    full += usize::from(matches!(
                        ack.work,
                        SessionWork::Rebuild(_) | SessionWork::Recompute
                    ));
                    worker_time += ack.worker_time;
                    completed += 1;
                }
                Err(TrySendError::Disconnected(_)) => panic!("worker stopped"),
            }
        }
    }
    while completed < edits.len() {
        let ack = worker.receiver.recv().expect("newest result");
        assert_eq!(ack.sequence, first + completed);
        assert_eq!(ack.current, !matches!(edits[completed], Edit::Cancel));
        let publish = ack.current && ack.sequence == first + edits.len() - 1;
        published += usize::from(publish);
        suppressed += usize::from(!publish);
        full += usize::from(matches!(
            ack.work,
            SessionWork::Rebuild(_) | SessionWork::Recompute
        ));
        worker_time += ack.worker_time;
        completed += 1;
    }
    assert_eq!(
        published,
        usize::from(!matches!(edits.last(), Some(Edit::Cancel)))
    );
    BatchTimes {
        caller,
        completion: start.elapsed(),
        newest: newest_start.elapsed(),
        worker: worker_time,
        full,
        published,
        suppressed,
    }
}
fn percentile(times: &mut [Duration], percent: usize) -> Duration {
    times.sort_unstable();
    times[(times.len() * percent)
        .div_ceil(100)
        .saturating_sub(1)
        .min(times.len() - 1)]
}
pub fn run(config: Config) {
    let candidates = if std::env::var_os("ORANGE_MEASURE_OWNER_FIRST").is_some() {
        [false, true]
    } else {
        [true, false]
    };
    println!(
        "authoring candidate_order={:?} (true=snapshot-session)",
        candidates
    );
    // Fair same-document semantics checks, outside timing windows.
    for snapshots in candidates {
        let mut worker = Worker::spawn(fixture(64), snapshots);
        let edits = [
            Edit::Literal(7),
            Edit::Undo,
            Edit::Redo,
            Edit::Layout,
            Edit::MissingReceipt,
            Edit::DivergentReceipt,
            Edit::Registry,
            Edit::Effects,
            Edit::Cancel,
            Edit::Recover,
            Edit::Literal(9),
            Edit::Check,
        ];
        batch(&mut worker, &edits, true, 1);
        batch(&mut worker, &[Edit::Cancel], true, 13);
        batch(&mut worker, &[Edit::Recover, Edit::Check], true, 14);
        worker.shutdown();
    }
    println!(
        "authoring scenarios=PASS ordered/undo/redo/layout/missing+foreign receipts/registry/effects/cancel/recovery/shutdown; publication=current newest sequence only"
    );
    for snapshots in candidates {
        let name = if snapshots {
            "snapshot-session"
        } else {
            "document-owner"
        };
        let mut times = Vec::with_capacity(config.runs);
        for _ in 0..config.runs {
            let mut worker = Worker::spawn(fixture(2_000), snapshots);
            let mut request = worker.prepare(1, Edit::MissingReceipt, false);
            let (entered_tx, entered_rx) = sync_channel(0);
            let (resume_tx, resume_rx) = sync_channel(0);
            request.gate = Some(Gate {
                entered: entered_tx,
                resume: resume_rx,
            });
            worker
                .sender
                .send(request)
                .expect("accepted before shutdown");
            entered_rx
                .recv()
                .expect("analysis accepted at prepass gate");
            let start = Instant::now();
            worker.cancel.store(true, Ordering::Relaxed);
            resume_tx.send(()).expect("release cancelled analysis");
            let outcome = worker.shutdown();
            assert_eq!(outcome.cancelled_results, 1);
            times.push(start.elapsed());
        }
        let p50 = percentile(&mut times, 50);
        let p95 = percentile(&mut times, 95);
        println!(
            "authoring accepted-job cancellation candidate={name} samples={} cancelled_results_per_sample=1 cancel-to-join_p50_us={:.3} p95_us={:.3}",
            config.runs,
            p50.as_secs_f64() * 1e6,
            p95.as_secs_f64() * 1e6
        );
    }
    for nodes in [2_000, 10_000] {
        for burst in [1, 32] {
            for snapshots in candidates {
                let name = if snapshots {
                    "snapshot-session"
                } else {
                    "document-owner"
                };
                let mut caller = Vec::with_capacity(config.runs);
                let mut newest = Vec::with_capacity(config.runs);
                let mut burst_times = Vec::with_capacity(config.runs);
                let mut analysis = Vec::with_capacity(config.runs);
                let mut shutdown = Vec::with_capacity(config.runs);
                let mut first_counts: Option<Counts> = None;
                let mut peak = 0;
                let mut retained = 0;
                let mut full = 0;
                let mut published = 0;
                let mut suppressed = 0;
                for sample in 0..config.warmup + config.runs {
                    let state = fixture(nodes);
                    let baseline = counter::live_bytes();
                    let mut worker = Worker::spawn(state, snapshots);
                    let initial = counter::live_bytes().saturating_sub(baseline);
                    let edits = (0..burst)
                        .map(|n| Edit::Literal((100 + n) as u64))
                        .collect::<Vec<_>>();
                    let window = Snapshot::start();
                    let timing = batch(&mut worker, &edits, false, 1);
                    let counts = window.finish();
                    let kept = counter::live_bytes().saturating_sub(baseline);
                    let end = worker.shutdown();
                    if sample >= config.warmup {
                        caller.push(timing.caller);
                        newest.push(timing.newest);
                        burst_times.push(timing.completion);
                        analysis.push(timing.worker);
                        shutdown.push(end.time);
                        first_counts.get_or_insert(counts);
                        peak = peak.max(counts.peak_live_bytes);
                        retained = retained.max(initial.max(kept));
                        full += timing.full;
                        published += timing.published;
                        suppressed += timing.suppressed;
                    }
                }
                let counts = first_counts.expect("samples");
                let caller_p50 = percentile(&mut caller, 50);
                let caller_p95 = percentile(&mut caller, 95);
                let newest_p50 = percentile(&mut newest, 50);
                let newest_p95 = percentile(&mut newest, 95);
                let burst_p50 = percentile(&mut burst_times, 50);
                let burst_p95 = percentile(&mut burst_times, 95);
                let worker_p50 = percentile(&mut analysis, 50);
                let shutdown_p95 = percentile(&mut shutdown, 95);
                println!(
                    "authoring candidate={name} nodes={nodes} burst={burst} samples={} caller_p50_us={:.3} caller_p95_us={:.3} newest_p50_us={:.3} newest_p95_us={:.3} burst_p50_us={:.3} burst_p95_us={:.3} worker_p50_us={:.3} shutdown_p95_us={:.3} extra_retained_max_bytes={retained} edit_peak_extra_max_bytes={peak} first_batch_cumulative_bytes={} first_batch_allocations={} full_runs={full} published={published} suppressed={suppressed}",
                    config.runs,
                    caller_p50.as_secs_f64() * 1e6,
                    caller_p95.as_secs_f64() * 1e6,
                    newest_p50.as_secs_f64() * 1e6,
                    newest_p95.as_secs_f64() * 1e6,
                    burst_p50.as_secs_f64() * 1e6,
                    burst_p95.as_secs_f64() * 1e6,
                    worker_p50.as_secs_f64() * 1e6,
                    shutdown_p95.as_secs_f64() * 1e6,
                    counts.allocated_bytes,
                    counts.allocations
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_newest_current_intent_is_published_across_history_and_rebuilds() {
        for snapshots in [true, false] {
            let mut worker = Worker::spawn(fixture(16), snapshots);
            let edits = [
                Edit::Literal(7),
                Edit::Undo,
                Edit::Redo,
                Edit::Layout,
                Edit::MissingReceipt,
                Edit::DivergentReceipt,
                Edit::Registry,
                Edit::Effects,
                Edit::Cancel,
                Edit::Recover,
                Edit::Check,
            ];
            let outcome = batch(&mut worker, &edits, true, 1);
            assert_eq!(outcome.published, 1);
            assert_eq!(outcome.suppressed, edits.len() - 1);
            let cancelled = batch(&mut worker, &[Edit::Cancel], true, edits.len() + 1);
            assert_eq!(cancelled.published, 0);
            assert_eq!(cancelled.suppressed, 1);
            worker.shutdown();
        }
    }

    #[test]
    fn closing_cancels_an_accepted_job_before_analysis_and_joins() {
        for snapshots in [true, false] {
            let mut worker = Worker::spawn(fixture(16), snapshots);
            let mut request = worker.prepare(1, Edit::MissingReceipt, false);
            let (entered_tx, entered_rx) = sync_channel(0);
            let (resume_tx, resume_rx) = sync_channel(0);
            request.gate = Some(Gate {
                entered: entered_tx,
                resume: resume_rx,
            });
            worker.sender.send(request).unwrap();
            entered_rx.recv().unwrap();
            worker.cancel.store(true, Ordering::Relaxed);
            resume_tx.send(()).unwrap();
            // The gated job also asserts the actual AnalysisCancelled diagnostic.
            assert_eq!(worker.shutdown().cancelled_results, 1);
        }
    }
}
