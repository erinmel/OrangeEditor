//! One running graph: a thread that owns the run and is the only code that touches it.
//!
//! Everything else reaches the run through its bounded mailbox or through a read-only
//! query the thread executes between turns, so a run needs no lock and two flows never
//! wait on each other.
use crate::effects::Pool;
use crate::services::ServiceEffect;
use crate::state::{self, Kept};
use crate::{FlowEvent, FlowId, HostEvent};
use orange_compiler::ControlPlan;
use orange_guest::{GuestCancellation, guest_effect};
use orange_kernel::{FaultText, KernelFault};
use orange_model::GraphDocument;
use orange_model::Value;
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::VarScope;
use orange_model::id::{ConnectionId, EventDeclId, VariableId};
use orange_runtime::{
    EffectLease, EffectRequest, HostInput, InboxError, Refused, Run, RunDriver, RunError,
    RunEventSource, RunInbox, RunOptions, RunState, SourceOptions, StopMode,
};
use orange_runtime::{PortValues, Workspace, WorkspaceSnapshot};
use orange_sdk::{Call, Session};
use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvError, RecvTimeoutError, Sender, SyncSender, TryRecvError};
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

/// Work applied between turns. Returns whether it may have scheduled graph work.
pub(crate) type Query = Box<dyn FnOnce(&mut Run) -> bool + Send>;

/// State a flow's thread shares with the host handle.
pub(crate) struct Shared {
    pub state: Mutex<RunState>,
    /// `None` once the thread has ended; queries are then refused instead of lost.
    pub queries: Mutex<Option<VecDeque<Query>>>,
    pub closing: AtomicBool,
    /// Whether variable values are reported as they change.
    pub observed: AtomicBool,
    /// An edited document waiting to replace the running one.
    pub swap: Mutex<Option<Swap>>,
}
impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("state", &self.state)
            .field("closing", &self.closing)
            .finish_non_exhaustive()
    }
}

/// The variables every flow of a host can share. Writes are applied one at a time
/// under the lock, in the order flows reach it; each flow then takes the new values
/// at its next turn.
#[derive(Default)]
pub(crate) struct Space {
    /// Where the persistent shared variables are stored, and which they are.
    pub keep: Mutex<Option<Keep>>,
    /// `None` until the application declares the shared variables.
    pub workspace: Mutex<Option<Workspace>>,
    /// Counts committed writes, so a flow can tell it is behind without the lock.
    pub version: AtomicU64,
    /// Threads to wake after a write.
    pub flows: Mutex<BTreeMap<FlowId, Thread>>,
}
impl std::fmt::Debug for Space {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Space")
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}
impl Space {
    pub(crate) fn snapshot(&self) -> Option<WorkspaceSnapshot> {
        self.workspace
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(Workspace::snapshot)
    }
    /// Stores the persistent shared variables as they are now.
    pub(crate) fn store(&self) -> Result<(), String> {
        let keep = self.keep.lock().unwrap_or_else(PoisonError::into_inner);
        let (Some(keep), Some(snapshot)) = (keep.as_ref(), self.snapshot()) else {
            return Ok(());
        };
        let now = state::snapshot(&keep.variables, keep.registry.types(), |id| {
            snapshot.get(id).ok().flatten().cloned()
        });
        state::save(&keep.directory, &keep.name, &now)
    }
    /// Wakes every flow, after a write the host itself made.
    pub(crate) fn wake_all(&self) {
        let flows = self.flows.lock().unwrap_or_else(PoisonError::into_inner);
        for thread in flows.values() {
            thread.unpark();
        }
    }
    fn wake_others(&self, me: FlowId) {
        let flows = self.flows.lock().unwrap_or_else(PoisonError::into_inner);
        for (flow, thread) in flows.iter() {
            if *flow != me {
                thread.unpark();
            }
        }
    }
}

/// Whether the graph reads or writes a shared variable.
pub(crate) fn uses_space(plan: &ControlPlan) -> bool {
    plan.variables()
        .iter()
        .any(|cell| cell.scope() == VarScope::Workspace)
}

/// A compiled replacement for a flow's graph, applied by the flow's own thread.
pub(crate) struct Swap {
    pub plan: Arc<ControlPlan>,
    pub document: GraphDocument,
    pub registry: Arc<RegistrySnapshot>,
    pub options: RunOptions,
    pub inbox_capacity: usize,
    pub sessions: BTreeMap<ConnectionId, Arc<dyn Session>>,
    pub keep: Option<Keep>,
    pub armables: Vec<crate::armed::Armable>,
    pub reply: Sender<Result<Swapped, RunError>>,
}

/// What the host needs to reach the replacement run.
pub(crate) struct Swapped {
    pub inbox: RunInbox,
    pub sources: BTreeMap<EventDeclId, RunEventSource>,
    pub kept: usize,
    pub reset: usize,
}

/// Registers a host source for every event the run opens to the host.
pub(crate) fn event_sources(
    driver: &mut RunDriver,
    plan: &ControlPlan,
) -> BTreeMap<EventDeclId, RunEventSource> {
    let mut sources = BTreeMap::new();
    for event in plan.events() {
        // Events the run does not open to the host are simply not offered.
        if let Ok(source) = driver.register_source(event.id(), SourceOptions::default()) {
            let _ = sources.insert(event.id(), source);
        }
    }
    sources
}

/// Replaces the flow's run with one compiled from an edited document.
///
/// The new run is started first, with the value of every run variable whose identity and
/// type are unchanged; only then is the old run stopped. If the new run cannot start,
/// the old one is untouched and keeps running.
fn replace(driver: &mut RunDriver, cx: &mut Context, swap: Swap) -> Result<Swapped, RunError> {
    let old = driver.run();
    let mut values = BTreeMap::new();
    let (mut kept, mut reset) = (0, 0);
    for cell in swap.plan.variables() {
        if cell.scope() != VarScope::Run {
            continue;
        }
        let unchanged = cx.plan.variables().iter().any(|before| {
            before.id() == cell.id() && before.scope() == VarScope::Run && before.ty() == cell.ty()
        });
        if unchanged {
            kept += 1;
            // An unset variable takes the new document's initial value, if it has one.
            if let Some(value) = old.variable(cell.id()) {
                let _ = values.insert(cell.id(), Some(value.clone()));
            }
        } else {
            reset += 1;
        }
    }
    let paused = old.state() == RunState::Paused;
    let snapshot = if uses_space(&swap.plan) {
        Some(cx.space.snapshot().ok_or(RunError::InvalidState)?)
    } else {
        None
    };
    let mut run = match &snapshot {
        Some(snapshot) => Run::start_in_workspace_with_variables(
            Arc::clone(&swap.plan),
            &swap.document,
            &swap.registry,
            0,
            swap.options,
            snapshot,
            &values,
        )?,
        None => Run::start_with_variables(
            Arc::clone(&swap.plan),
            &swap.document,
            &swap.registry,
            0,
            swap.options,
            &values,
        )?,
    };
    if paused {
        run.pause()?;
    }
    let now = u64::try_from(cx.clock.elapsed().as_nanos()).unwrap_or(u64::MAX);
    let (mut next, inbox) = RunDriver::new(run, swap.inbox_capacity, now);
    let sources = event_sources(&mut next, &swap.plan);

    // Retire the old run: cancel what it was doing and let its cleanup finish. Its
    // On Stop entries do not run, because the graph is being edited, not stopped.
    let mut old = std::mem::replace(driver, next).into_run();
    let _ = old.request_stop(StopMode::Immediate);
    for _ in 0..64 {
        if terminal(old.state()) {
            break;
        }
        match old.pump(cx.turn_budget) {
            Ok(progress) if progress.ready => {}
            Ok(_) => {
                // Nothing runnable: let its cleanup deadline pass.
                let later = old.now().saturating_add(1_000_000_000);
                if old.advance_to(later).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = old.dispose();

    cx.inbox = inbox.clone();
    cx.armed.replace(swap.armables);
    cx.sessions = swap.sessions;
    cx.plan = swap.plan;
    cx.keep = swap.keep;
    Ok(Swapped {
        inbox,
        sources,
        kept,
        reset,
    })
}

/// Bounded, never-blocking delivery of observations. A slow observer loses telemetry
/// (steps, values) and can see how many; it never slows a run. Lifecycle events (state,
/// faults, refusals, a replaced graph, the end of a flow) are never lost to a full
/// queue: they wait in a small overflow list the observer reads once the queue is empty.
#[derive(Debug, Clone)]
pub(crate) struct Sink {
    pub events: SyncSender<HostEvent>,
    pub dropped: Arc<AtomicU64>,
    pub overflow: Arc<Mutex<VecDeque<HostEvent>>>,
}

/// Most lifecycle events waiting beside a full queue; past it even they are counted
/// as dropped. Each flow sends only a few.
pub(crate) const MAX_OVERFLOW: usize = 1024;

/// Whether an observer must see this event even when it reads slowly.
fn lifecycle(kind: &FlowEvent) -> bool {
    matches!(
        kind,
        FlowEvent::State(_)
            | FlowEvent::Fault(_)
            | FlowEvent::Refused(_)
            | FlowEvent::Updated { .. }
            | FlowEvent::SourceRefused { .. }
            | FlowEvent::SourceFault { .. }
            | FlowEvent::Finished
    )
}

impl Sink {
    pub(crate) fn send(&self, flow: FlowId, kind: FlowEvent) {
        let event = HostEvent { flow, kind };
        let Err(full) = self.events.try_send(event) else {
            return;
        };
        let event = match full {
            std::sync::mpsc::TrySendError::Full(event) => event,
            // No one observes any more.
            std::sync::mpsc::TrySendError::Disconnected(_) => return,
        };
        if lifecycle(&event.kind) {
            let mut overflow = self.overflow.lock().unwrap_or_else(PoisonError::into_inner);
            if overflow.len() < MAX_OVERFLOW {
                overflow.push_back(event);
                return;
            }
        }
        let _ = self.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

/// What a host observes of its flows, in order. Telemetry comes through a bounded queue;
/// lifecycle events that did not fit wait beside it and are read once it is empty, so
/// the end of a flow is always seen.
#[derive(Debug)]
pub struct Events {
    pub(crate) queue: Receiver<HostEvent>,
    pub(crate) overflow: Arc<Mutex<VecDeque<HostEvent>>>,
}

impl From<Receiver<HostEvent>> for Events {
    /// Events read from a plain channel, with nothing waiting beside it: for an
    /// application or test that feeds an observer itself.
    fn from(queue: Receiver<HostEvent>) -> Self {
        Self {
            queue,
            overflow: Arc::default(),
        }
    }
}

impl Events {
    fn waiting(&self) -> Option<HostEvent> {
        self.overflow
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
    }

    /// The next event if there is one now.
    pub fn try_recv(&self) -> Result<HostEvent, TryRecvError> {
        match self.queue.try_recv() {
            Ok(event) => Ok(event),
            Err(error) => self.waiting().ok_or(error),
        }
    }

    /// The next event, waiting at most `timeout` for one.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<HostEvent, RecvTimeoutError> {
        match self.try_recv() {
            Ok(event) => return Ok(event),
            Err(TryRecvError::Disconnected) => return Err(RecvTimeoutError::Disconnected),
            Err(TryRecvError::Empty) => {}
        }
        match self.queue.recv_timeout(timeout) {
            Ok(event) => Ok(event),
            Err(error) => self.waiting().ok_or(error),
        }
    }

    /// Every event there is now, without waiting.
    pub fn try_iter(&self) -> impl Iterator<Item = HostEvent> + '_ {
        std::iter::from_fn(|| self.try_recv().ok())
    }

    /// The next event, waiting for one.
    pub fn recv(&self) -> Result<HostEvent, RecvError> {
        match self.try_recv() {
            Ok(event) => Ok(event),
            Err(TryRecvError::Disconnected) => Err(RecvError),
            Err(TryRecvError::Empty) => self.queue.recv(),
        }
    }
}

/// The run variables a flow stores as they change, and where.
#[derive(Debug, Clone)]
pub(crate) struct Keep {
    pub directory: PathBuf,
    /// The file name: the graph's ID.
    pub name: String,
    pub variables: Vec<Kept>,
    pub registry: Arc<RegistrySnapshot>,
}

pub(crate) struct Context {
    pub keep: Option<Keep>,
    pub id: FlowId,
    pub shared: Arc<Shared>,
    pub sink: Sink,
    pub pool: Pool,
    pub inbox: RunInbox,
    pub plan: Arc<ControlPlan>,
    pub sessions: BTreeMap<ConnectionId, Arc<dyn Session>>,
    pub space: Arc<Space>,
    pub clock: Instant,
    pub turn_inputs: usize,
    pub turn_budget: usize,
    /// Live nodes and entries with data inputs, armed with what the run evaluates.
    pub armed: crate::armed::Armed,
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

/// The host already has as much work waiting as it holds.
pub(crate) const BUSY: KernelFault = KernelFault {
    code: FaultText::Static("host/Busy"),
    message_key: FaultText::Static("host/busy"),
    message: "The host has too much work waiting; try again.",
};

const NO_CONNECTION: KernelFault = KernelFault {
    code: FaultText::Static("host/ConnectionUnavailable"),
    message_key: FaultText::Static("host/connection-unavailable"),
    message: "The node names a connection this flow has not opened.",
};

/// Hands a finished effect back to its run. The mailbox is bounded, so a full one is
/// waited on: an effect result is never dropped while its flow is alive. Called from
/// worker threads only; the flow is woken first so it makes room.
fn deliver(inbox: &RunInbox, input: HostInput, flow: &Thread) {
    match inbox.send(input) {
        Ok(()) => {}
        Err(InboxError::Closed(_)) => return,
        Err(InboxError::Full(input)) => {
            flow.unpark();
            if inbox.send_waiting(input).is_err() {
                return;
            }
        }
    }
    flow.unpark();
}

fn start_sources(cx: &mut Context, thread: &thread::Thread) {
    for (node, reason) in cx.armed.start(&cx.inbox, thread, (&cx.sink, cx.id)) {
        cx.sink
            .send(cx.id, FlowEvent::SourceRefused { node, reason });
    }
}

pub(crate) fn drive(mut driver: RunDriver, mut cx: Context) {
    let me = thread::current();
    let mut recorded = 0u64;
    let mut last_state = None;
    let mut guest_work: Vec<(EffectLease, GuestCancellation)> = Vec::new();
    let mut errors = 0u8;
    let mut closing_since: Option<Instant> = None;
    // The count of shared writes this flow's run has been given.
    let mut space_seen = cx.space.version.load(Ordering::Acquire);
    // The value of each variable as last reported to the observer.
    let mut reported: BTreeMap<VariableId, Option<Value>> = BTreeMap::new();
    // The persistent variables as last stored. `None` until the first comparison, so
    // the values a flow starts with are stored once.
    let mut stored: Option<Vec<Option<Value>>> = None;
    // Whether this flow wrote a shared variable since the shared ones were stored.
    let mut shared_dirty = false;
    // The value of each output port as last reported to the observer.
    let mut ports: PortValues = PortValues::new();
    start_sources(&mut cx, &me);
    loop {
        if closing_since.is_none() && cx.shared.closing.load(Ordering::Relaxed) {
            closing_since = Some(Instant::now());
            let _ = driver.run_mut().request_stop(StopMode::Immediate);
        }
        let pending = cx
            .shared
            .swap
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(swap) = pending {
            let reply = swap.reply.clone();
            let outcome = replace(&mut driver, &mut cx, swap);
            if let Ok(swapped) = &outcome {
                // Work issued by the old run has nowhere to return to.
                for (_, cancellation) in guest_work.drain(..) {
                    cancellation.cancel();
                }
                recorded = 0;
                last_state = None;
                reported.clear();
                ports.clear();
                start_sources(&mut cx, &me);
                cx.sink.send(
                    cx.id,
                    FlowEvent::Updated {
                        kept_variables: swapped.kept,
                        reset_variables: swapped.reset,
                    },
                );
            }
            let _ = reply.send(outcome);
        }
        // Take shared values other flows have written since this one last looked.
        let version = cx.space.version.load(Ordering::Acquire);
        if version != space_seen && uses_space(&cx.plan) {
            if let Some(snapshot) = cx.space.snapshot() {
                let _ = driver.run_mut().publish_workspace(&snapshot);
            }
            space_seen = version;
        }
        // Read before the turn, so input sent after observation began is observed.
        let observed = cx.shared.observed.load(Ordering::Relaxed);
        driver.run_mut().keep_port_values(observed);
        // Live values published since the last turn, at most one each per interval.
        let live_deadline = cx.armed.flush(driver.run_mut());
        let turn_started = Instant::now();
        let now =
            u64::try_from(turn_started.duration_since(cx.clock).as_nanos()).unwrap_or(u64::MAX);
        let mut refused = |refusal: Refused| {
            cx.sink.send(cx.id, FlowEvent::Refused(refusal.reason));
        };
        let turn = driver.turn(now, cx.turn_inputs, cx.turn_budget, &mut refused);

        let mut issued = false;
        // Shared writes are applied here, on the flow's thread, one at a time across
        // the host.
        while let Ok(Some(request)) = driver.run_mut().take_shared() {
            issued = true;
            let mut workspace = cx
                .space
                .workspace
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let Some(shared) = workspace.as_mut() else {
                let _ = driver.run_mut().fail_shared(request, UNAVAILABLE);
                continue;
            };
            match shared.apply_shared(request) {
                Ok(receipt) => {
                    let committed = receipt.commit().is_ok();
                    let _ = driver.run_mut().complete_shared(receipt);
                    if committed {
                        shared_dirty = true;
                        let _ = cx.space.version.fetch_add(1, Ordering::AcqRel);
                        drop(workspace);
                        cx.space.wake_others(cx.id);
                    }
                }
                Err(refused) => {
                    let _ = driver.run_mut().fail_shared(refused.value, UNAVAILABLE);
                }
            }
        }
        // Sources whose data inputs changed are started again with the new values.
        while let Some(arguments) = driver.run_mut().take_arguments() {
            if let Err(reason) = cx.armed.arm(
                arguments.node,
                &arguments.values,
                &cx.inbox,
                &me,
                (&cx.sink, cx.id),
            ) {
                cx.sink.send(
                    cx.id,
                    FlowEvent::SourceRefused {
                        node: arguments.node,
                        reason,
                    },
                );
            }
        }
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
                let cancellation = effect.cancellation();
                let (inbox, flow) = (cx.inbox.clone(), me.clone());
                let submitted = cx.pool.submit(move || {
                    let outcome = effect.run(&inputs);
                    deliver(&inbox, HostInput::EffectDone { id, outcome }, &flow);
                });
                // Work the host cannot hold now fails at once instead of queueing.
                if submitted.is_ok() {
                    guest_work.push((lease, cancellation));
                } else {
                    let _ = driver.run_mut().complete_effect(id, Err(BUSY));
                }
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
                        let (shared, pool) = (Arc::clone(&cx.shared), cx.pool.clone());
                        let submitted = cx.pool.submit(move || {
                            let call = Call {
                                inputs: &inputs,
                                session: session.as_deref(),
                                lease: &lease,
                            };
                            let Some(item) = service.stream.clone() else {
                                let outcome = service.node.run(&call);
                                deliver(&inbox, HostInput::EffectDone { id, outcome }, &flow);
                                return;
                            };
                            match service.node.items(&call) {
                                Some(Ok(items)) => {
                                    let producer = crate::streams::Pulled::new(
                                        items,
                                        pool,
                                        Arc::clone(&shared),
                                        flow.clone(),
                                    );
                                    crate::streams::ask(&shared, &flow, move |run| {
                                        if let Err(refused) = run.complete_effect_stream(
                                            id,
                                            &item,
                                            Box::new(producer),
                                        ) {
                                            refused
                                                .producer
                                                .close(orange_runtime::CloseReason::Closed);
                                            let _ = run.complete_effect(id, Err(UNAVAILABLE));
                                        }
                                    });
                                }
                                Some(Err(fault)) => {
                                    let outcome = Err(fault);
                                    deliver(&inbox, HostInput::EffectDone { id, outcome }, &flow);
                                }
                                None => {
                                    let outcome = Err(UNAVAILABLE);
                                    deliver(&inbox, HostInput::EffectDone { id, outcome }, &flow);
                                }
                            }
                        });
                        if submitted.is_err() {
                            let _ = driver.run_mut().complete_effect(id, Err(BUSY));
                        }
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

        if observed {
            let mut changed = Vec::new();
            let mut cleared = Vec::new();
            for ((node, port), value) in driver.run_mut().port_values().into_iter().flatten() {
                if cx.plan.live_port_index(*node, port).is_some() {
                    continue;
                }
                let key = (*node, port.clone());
                if ports.get(&key) != Some(value) {
                    let _ = ports.insert(key, value.clone());
                    changed.push((*node, port.clone(), value.clone()));
                }
            }
            // A live value is on its output whether or not a node has read it yet.
            for live in cx.plan.lives() {
                let key = (live.node(), live.port().clone());
                let Some(value) = driver.run().live_port_value(live.node(), live.port()) else {
                    if ports.remove(&key).is_some() {
                        cleared.push(key);
                    }
                    continue;
                };
                if ports.get(&key) != Some(value) {
                    let _ = ports.insert(key, value.clone());
                    changed.push((live.node(), live.port().clone(), value.clone()));
                }
            }
            if !changed.is_empty() {
                cx.sink.send(cx.id, FlowEvent::Ports(changed));
            }
            if !cleared.is_empty() {
                cx.sink.send(cx.id, FlowEvent::PortsCleared(cleared));
            }
        } else if !ports.is_empty() {
            ports.clear();
        }

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

        if observed {
            let mut changed = Vec::new();
            for cell in cx.plan.variables() {
                let value = run.variable(cell.id()).cloned();
                if reported.get(&cell.id()) != Some(&value) {
                    let _ = reported.insert(cell.id(), value.clone());
                    changed.push((cell.id(), value));
                }
            }
            if !changed.is_empty() {
                cx.sink.send(cx.id, FlowEvent::Variables(changed));
            }
        } else if !reported.is_empty() {
            reported.clear();
        }

        let queries = cx
            .shared
            .queries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
            .map(std::mem::take);
        let mut scheduled = false;
        for query in queries.into_iter().flatten() {
            scheduled |= query(driver.run_mut());
        }
        let run = driver.run();
        if let Some(keep) = &cx.keep {
            // Values are compared as they are; encoding and writing happen only when
            // one differs from what was last stored.
            let now: Vec<Option<Value>> = keep
                .variables
                .iter()
                .map(|kept| run.variable(kept.id).cloned())
                .collect();
            if stored.as_ref() != Some(&now) {
                let mut values = now.iter();
                let encoded = state::snapshot(&keep.variables, keep.registry.types(), |_| {
                    values.next().cloned().flatten()
                });
                // A failed write is tried again at the next turn.
                if state::save(&keep.directory, &keep.name, &encoded).is_ok() {
                    stored = Some(now);
                }
            }
        }
        if shared_dirty {
            shared_dirty = cx.space.store().is_err();
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
                // Stream completions and host writes can make work ready after the pump.
                if turn.progress.ready || turn.inbox_pending || issued || scheduled {
                    continue;
                }
                let wait = turn_wait(
                    turn_started,
                    Instant::now(),
                    turn.progress
                        .next_wakeup
                        .map(|at| Duration::from_nanos(at.saturating_sub(driver.run().now()))),
                    live_deadline,
                    !guest_work.is_empty() || closing_since.is_some(),
                );
                if !wait.is_zero() {
                    thread::park_timeout(wait);
                }
            }
        }
    }
    cx.armed.stop();
    for (_, cancellation) in &guest_work {
        cancellation.cancel();
    }
    // A replacement that arrives now is refused by dropping its reply channel.
    drop(
        cx.shared
            .swap
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take(),
    );
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

// Sample the clock after the turn so work already done counts toward its deadlines.
fn turn_wait(
    turn_started: Instant,
    now: Instant,
    run_delay_at_start: Option<Duration>,
    live_deadline: Option<Instant>,
    watching: bool,
) -> Duration {
    let mut wait = if watching { WATCH } else { IDLE };
    if let Some(delay) = run_delay_at_start {
        wait = wait.min(delay.saturating_sub(now.duration_since(turn_started)));
    }
    if let Some(at) = live_deadline {
        wait = wait.min(at.saturating_duration_since(now));
    }
    wait
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_spent_in_the_turn_counts_toward_run_and_live_deadlines() {
        let start = Instant::now();
        let ms = Duration::from_millis;
        let live = Some(start + ms(30));
        let run = Some(ms(25));

        assert_eq!(turn_wait(start, start + ms(17), run, live, false), ms(8));
        assert_eq!(turn_wait(start, start + ms(17), None, live, false), ms(13));
        assert_eq!(
            turn_wait(start, start + ms(26), run, live, false),
            Duration::ZERO
        );
        assert_eq!(
            turn_wait(start, start + ms(31), None, live, false),
            Duration::ZERO
        );
        assert_eq!(turn_wait(start, start, run, live, true), WATCH);
        assert_eq!(turn_wait(start, start, None, None, false), IDLE);
    }

    /// A full queue loses telemetry but never the end of a flow: it waits beside the
    /// queue and is read once the queue is empty.
    #[test]
    fn a_full_queue_drops_telemetry_but_keeps_the_end_of_a_flow() {
        let (events, queue) = std::sync::mpsc::sync_channel(1);
        let overflow = Arc::new(Mutex::new(VecDeque::new()));
        let sink = Sink {
            events,
            dropped: Arc::new(AtomicU64::new(0)),
            overflow: Arc::clone(&overflow),
        };
        let observer = Events { queue, overflow };
        let flow = FlowId(7);
        sink.send(flow, FlowEvent::TraceLost(1));
        sink.send(flow, FlowEvent::TraceLost(2));
        sink.send(flow, FlowEvent::Finished);
        assert_eq!(sink.dropped.load(Ordering::Relaxed), 1);
        let kinds: Vec<_> = observer.try_iter().map(|event| event.kind).collect();
        assert!(matches!(
            kinds.as_slice(),
            [FlowEvent::TraceLost(1), FlowEvent::Finished]
        ));
    }
}
