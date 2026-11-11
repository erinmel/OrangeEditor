//! One running graph: a thread that owns the run and is the only code that touches it.
//!
//! Everything else reaches the run through its bounded mailbox or through a read-only
//! query the thread executes between turns, so a run needs no lock and two flows never
//! wait on each other.
use crate::effects::Pool;
use crate::services::{Call, ServiceEffect, Session};
use crate::{FlowEvent, FlowId, HostEvent};
use orange_guest::{GuestCancellation, guest_effect};
use orange_kernel::{FaultText, KernelFault};
use orange_model::id::ConnectionId;
use orange_runtime::{
    EffectLease, EffectRequest, HostInput, InboxError, Refused, Run, RunDriver, RunInbox, RunState,
    StopMode,
};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, Thread};
use std::time::{Duration, Instant};

/// Longest sleep of an idle flow. Every sender wakes the thread, so this bounds only
/// the cost of a wake-up lost to a bug, not responsiveness.
const IDLE: Duration = Duration::from_secs(1);
/// Poll interval while guest work is running, to forward lease cancellation to it.
const WATCH: Duration = Duration::from_millis(5);
/// Wall time a closing flow gets to finish its stop before its thread leaves anyway.
const CLOSE_GRACE: Duration = Duration::from_secs(2);

pub(crate) type Query = Box<dyn FnOnce(&Run) + Send>;

/// State a flow's thread shares with the host handle.
pub(crate) struct Shared {
    pub state: Mutex<RunState>,
    /// `None` once the thread has ended; queries are then refused instead of lost.
    pub queries: Mutex<Option<VecDeque<Query>>>,
    pub closing: AtomicBool,
}
impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("state", &self.state)
            .field("closing", &self.closing)
            .finish_non_exhaustive()
    }
}

/// Bounded, never-blocking delivery of observations. A slow observer loses events and
/// can see how many; it never slows a run.
#[derive(Debug, Clone)]
pub(crate) struct Sink {
    pub events: SyncSender<HostEvent>,
    pub dropped: Arc<AtomicU64>,
}
impl Sink {
    fn send(&self, flow: FlowId, kind: FlowEvent) {
        if self.events.try_send(HostEvent { flow, kind }).is_err() {
            let _ = self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub(crate) struct Context {
    pub id: FlowId,
    pub shared: Arc<Shared>,
    pub sink: Sink,
    pub pool: Pool,
    pub inbox: RunInbox,
    pub sessions: BTreeMap<ConnectionId, Arc<dyn Session>>,
    pub clock: Instant,
    pub turn_inputs: usize,
    pub turn_budget: usize,
}

fn terminal(state: RunState) -> bool {
    matches!(
        state,
        RunState::Faulted | RunState::Stopped | RunState::Disposed
    )
}

const UNAVAILABLE: KernelFault = KernelFault {
    code: FaultText::Static("host/EffectUnavailable"),
    message_key: FaultText::Static("host/effect-unavailable"),
    message: "No host service runs this effect.",
};

const NO_CONNECTION: KernelFault = KernelFault {
    code: FaultText::Static("host/ConnectionUnavailable"),
    message_key: FaultText::Static("host/connection-unavailable"),
    message: "The node names a connection this flow has not opened.",
};

/// Hands a finished effect back to its run. The mailbox is bounded, so a full one is
/// retried: an effect result is never dropped while its flow is alive.
fn deliver(inbox: &RunInbox, mut input: HostInput, flow: &Thread) {
    loop {
        match inbox.send(input) {
            Ok(()) => break,
            Err(InboxError::Closed(_)) => return,
            Err(InboxError::Full(back)) => {
                input = back;
                flow.unpark();
                thread::sleep(Duration::from_millis(1));
            }
        }
    }
    flow.unpark();
}

pub(crate) fn drive(mut driver: RunDriver, cx: Context) {
    let me = thread::current();
    let mut recorded = 0u64;
    let mut last_state = None;
    let mut guest_work: Vec<(EffectLease, GuestCancellation)> = Vec::new();
    let mut errors = 0u8;
    let mut closing_since: Option<Instant> = None;
    loop {
        if closing_since.is_none() && cx.shared.closing.load(Ordering::Relaxed) {
            closing_since = Some(Instant::now());
            let _ = driver.run_mut().request_stop(StopMode::Immediate);
        }
        let now = u64::try_from(cx.clock.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let mut refused = |refusal: Refused| {
            cx.sink.send(cx.id, FlowEvent::Refused(refusal.reason));
        };
        let turn = driver.turn(now, cx.turn_inputs, cx.turn_budget, &mut refused);

        let mut issued = false;
        while let Some(request) = driver.run_mut().take_effect() {
            issued = true;
            let EffectRequest {
                id,
                context,
                inputs,
                lease,
                ..
            } = request;
            if let Some(effect) = guest_effect(&context) {
                guest_work.push((lease, effect.cancellation()));
                let (inbox, flow) = (cx.inbox.clone(), me.clone());
                cx.pool.submit(move || {
                    let outcome = effect.run(&inputs);
                    deliver(&inbox, HostInput::EffectDone { id, outcome }, &flow);
                });
            } else if let Ok(service) = context.downcast::<ServiceEffect>() {
                // A node that names a connection this flow did not open cannot run.
                let session = match service.connection {
                    None => Ok(None),
                    Some(id) => cx.sessions.get(&id).cloned().map(Some).ok_or(NO_CONNECTION),
                };
                match session {
                    Err(fault) => {
                        let _ = driver.run_mut().complete_effect(id, Err(fault));
                    }
                    Ok(session) => {
                        let (inbox, flow) = (cx.inbox.clone(), me.clone());
                        cx.pool.submit(move || {
                            let outcome = service.node.run(&Call {
                                inputs: &inputs,
                                session: session.as_deref(),
                                lease: &lease,
                            });
                            deliver(&inbox, HostInput::EffectDone { id, outcome }, &flow);
                        });
                    }
                }
            } else {
                let _ = driver.run_mut().complete_effect(id, Err(UNAVAILABLE));
            }
        }
        guest_work.retain(|(lease, cancellation)| {
            if lease.is_cancelled() {
                cancellation.cancel();
            }
            !lease.is_finished()
        });

        // New trace entries since the last turn. The run keeps a bounded window, so a
        // burst larger than the window is reported as lost rather than replayed.
        let run = driver.run();
        let total = run.trace_dropped() + run.trace().len() as u64;
        let fresh = total.saturating_sub(recorded);
        let kept = usize::try_from(fresh).map_or(run.trace().len(), |n| n.min(run.trace().len()));
        if fresh > kept as u64 {
            cx.sink
                .send(cx.id, FlowEvent::TraceLost(fresh - kept as u64));
        }
        for event in run.trace().iter().skip(run.trace().len() - kept) {
            cx.sink.send(cx.id, FlowEvent::Node(event.clone()));
        }
        recorded = total;

        let queries = cx
            .shared
            .queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
            .map(std::mem::take);
        for query in queries.into_iter().flatten() {
            query(run);
        }

        let state = run.state();
        if last_state != Some(state) {
            last_state = Some(state);
            *cx.shared
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = state;
            cx.sink.send(cx.id, FlowEvent::State(state));
        }
        if terminal(state) || closing_since.is_some_and(|since| since.elapsed() > CLOSE_GRACE) {
            break;
        }
        match turn {
            Err(error) => {
                cx.sink.send(cx.id, FlowEvent::Fault(error));
                errors += 1;
                if errors == 3 {
                    break;
                }
                let _ = driver.run_mut().request_stop(StopMode::Immediate);
            }
            Ok(turn) => {
                errors = 0;
                if turn.progress.ready || turn.inbox_pending || issued {
                    continue;
                }
                let mut wait = turn.progress.next_wakeup.map_or(IDLE, |at| {
                    Duration::from_nanos(at.saturating_sub(driver.run().now())).min(IDLE)
                });
                if !guest_work.is_empty() || closing_since.is_some() {
                    wait = wait.min(WATCH);
                }
                thread::park_timeout(wait);
            }
        }
    }
    for (_, cancellation) in &guest_work {
        cancellation.cancel();
    }
    // Refuse queries from here on, and release any that were waiting.
    drop(
        cx.shared
            .queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take(),
    );
    let mut run = driver.into_run();
    let _ = run.dispose();
    *cx.shared
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = run.state();
    cx.sink.send(cx.id, FlowEvent::Finished);
}
