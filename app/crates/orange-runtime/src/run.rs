//! A host-driven logical actor. No worker thread, OS clock or editor dependency.
use crate::{DataActivation, DataError, DataOptions, DataPoll};
use orange_compiler::{
    ConstantFault, ControlIx, ControlOperation, ControlPlan, FoldedValue, HostSource, KernelFault,
};
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{FaultPolicy, VarScope};
use orange_model::id::{NodeId, VariableId};
use orange_model::{GraphDocument, Value};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use thiserror::Error;

static NEXT_RUN: AtomicU64 = AtomicU64::new(1);
fn run_identity() -> Result<u64, RunError> {
    let mut old = NEXT_RUN.load(Ordering::Relaxed);
    loop {
        let next = old.checked_add(1).ok_or(RunError::IdentityExhausted)?;
        match NEXT_RUN.compare_exchange_weak(old, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return Ok(old),
            Err(current) => old = current,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Created,
    Starting,
    Running,
    Draining,
    Cancelling,
    Faulted,
    Stopped,
    Disposed,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopMode {
    Graceful,
    Immediate,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlOutcome {
    Then,
    True,
    False,
    Failed,
    Step(usize),
    Suspended,
    Yielded,
    Body,
    Completed,
    Break,
    Continue,
    Cancelled,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunEvent {
    pub activation: u64,
    pub node: NodeId,
    pub outcome: ControlOutcome,
    pub fault: Option<ConstantFault>,
}
#[derive(Debug, Clone, Copy)]
pub struct RunLimits {
    pub max_activations: usize,
    /// Separately reserved On Stop frames, admitted and allocated at startup.
    pub max_stop_activations: usize,
    pub max_operations: usize,
    pub max_trace: usize,
    pub max_state_bytes: usize,
    pub max_region_depth: usize,
    pub max_timers: usize,
}
impl Default for RunLimits {
    fn default() -> Self {
        Self {
            max_activations: 256,
            max_stop_activations: 256,
            max_operations: 10_000_000,
            max_trace: 1024,
            max_state_bytes: 16 * 1024 * 1024,
            max_region_depth: 1024,
            max_timers: 1024,
        }
    }
}
#[derive(Debug, Clone)]
pub struct RunOptions {
    pub limits: RunLimits,
    pub data: DataOptions,
    pub drain_timeout_ns: u64,
    pub cleanup_timeout_ns: u64,
    pub max_cleanup_operations: usize,
}
impl Default for RunOptions {
    fn default() -> Self {
        Self {
            limits: RunLimits::default(),
            data: DataOptions::default(),
            drain_timeout_ns: 2_000_000_000,
            cleanup_timeout_ns: 500_000_000,
            max_cleanup_operations: 1024,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunProgress {
    pub operations: usize,
    pub ready: bool,
    pub next_wakeup: Option<u64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RunError {
    #[error("{0}")]
    Data(#[from] DataError),
    #[error("The run operation is invalid in its current state.")]
    InvalidState,
    #[error("The run exceeds a host resource limit.")]
    ResourceLimit,
    #[error("The process run identity space is exhausted.")]
    IdentityExhausted,
    #[error("Virtual monotonic time cannot move backwards.")]
    ClockWentBackwards,
}
impl RunError {
    pub const fn message_key(self) -> &'static str {
        match self {
            Self::Data(e) => e.message_key(),
            Self::InvalidState => "core/run-invalid-state",
            Self::ResourceLimit => "core/run-resource-limit",
            Self::IdentityExhausted => "core/run-identity-exhausted",
            Self::ClockWentBackwards => "core/run-clock-went-backwards",
        }
    }
}
struct Activation {
    sequence: u64,
    entry: ControlIx,
    pc: Option<ControlIx>,
    data: DataActivation,
    pull: Option<usize>,
    locals: Vec<Option<Value>>,
    local_bytes: usize,
    inputs: Vec<Option<FoldedValue>>,
    pending_input: Option<usize>,
    outputs: Vec<Option<FoldedValue>>,
    stack: Vec<Cursor>,
    waiting: Option<(u64, ControlIx)>,
    yielded: bool,
    reenter: Option<(ControlIx, u64)>,
    cleanup: bool,
    cleanup_remaining: usize,
}
struct Timer {
    run: u64,
    owner: ControlIx,
    activation: Activation,
}
enum Cursor {
    While {
        owner: ControlIx,
        iterations: u64,
    },
    Sequence {
        owner: ControlIx,
        next: usize,
    },
    Count {
        owner: ControlIx,
        next: i64,
        count: i64,
    },
}

/// Checked control execution. Inspection is explicit; Debug never formats values.
///
/// ```compile_fail
/// let run = orange_runtime::Run::default();
/// ```
pub struct Run {
    id: u64,
    plan: Option<Arc<ControlPlan>>,
    options: RunOptions,
    state: RunState,
    ready: VecDeque<Activation>,
    trace: VecDeque<RunEvent>,
    trace_dropped: u64,
    operations: usize,
    variables: Vec<Option<Value>>,
    state_bytes: usize,
    versions: Vec<u64>,
    now: u64,
    next_timer: u64,
    timers: BTreeMap<(u64, u64), Timer>,
    transitions: Vec<RunState>,
    deadline: Option<u64>,
    last_error: Option<RunError>,
    cancel_observed: bool,
    cleanup_truncated: u64,
    stop_ready: VecDeque<Activation>,
    segment_owner: Option<u64>,
    last_segment: Option<u64>,
    segment_indices: Vec<usize>,
    segment_writes: Vec<VariableId>,
    dirty: Vec<bool>,
}
impl std::fmt::Debug for Run {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Run")
            .field("id", &self.id)
            .field("state", &self.state)
            .field("ready", &self.ready.len())
            .field("operations", &self.operations)
            .finish_non_exhaustive()
    }
}
impl Run {
    pub fn start(
        plan: Arc<ControlPlan>,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        seed: u64,
        options: RunOptions,
    ) -> Result<Self, RunError> {
        if options
            .data
            .cancel
            .as_ref()
            .is_some_and(crate::CancelToken::is_cancelled)
        {
            return Err(DataError::Cancelled.into());
        }
        if !plan.data().is_current(document, plan.data().snapshot()) {
            return Err(DataError::StaleDocument.into());
        }
        if registry.generation() != plan.data().snapshot().generation() {
            return Err(DataError::StaleRegistry.into());
        }
        if plan.starts().len() > options.limits.max_activations
            || plan.stops().len() > options.limits.max_stop_activations
        {
            return Err(RunError::ResourceLimit);
        }
        let capacity = plan
            .starts()
            .len()
            .checked_add(plan.stops().len())
            .ok_or(RunError::ResourceLimit)?;
        let mut ready = VecDeque::with_capacity(capacity);
        let mut stop_ready = VecDeque::with_capacity(plan.stops().len());
        let mut state_bytes = 0usize;
        let mut local_bytes = 0usize;
        let mut variables = Vec::new();
        let mut locals = Vec::new();
        for var in plan.variables() {
            let value = var.initial().cloned();
            let bytes = value.as_ref().and_then(|v| var.charge(v)).unwrap_or(16);
            let (cell, total) = if var.scope() == VarScope::Local {
                (None, &mut local_bytes)
            } else {
                (value.clone(), &mut state_bytes)
            };
            *total = total
                .checked_add(bytes)
                .filter(|n| *n <= options.limits.max_state_bytes)
                .ok_or(RunError::ResourceLimit)?;
            variables.push(cell);
            locals.push(if var.scope() == VarScope::Local {
                value
            } else {
                None
            });
        }
        for (sequence, entry) in plan
            .starts()
            .iter()
            .chain(plan.stops())
            .copied()
            .enumerate()
        {
            let queue = if sequence < plan.starts().len() {
                &mut ready
            } else {
                &mut stop_ready
            };
            queue.push_back(Activation {
                sequence: sequence as u64,
                entry,
                pc: Some(entry),
                data: DataActivation::start(
                    plan.data().clone(),
                    document,
                    registry,
                    seed,
                    sequence as u64,
                    options.data.clone(),
                )?,
                pull: None,
                locals: locals.clone(),
                local_bytes,
                inputs: vec![],
                pending_input: None,
                outputs: vec![None; plan.host_sources().len()],
                stack: vec![],
                waiting: None,
                yielded: false,
                reenter: None,
                cleanup: false,
                cleanup_remaining: 0,
            });
        }
        let variable_count = variables.len();
        Ok(Self {
            id: run_identity()?,
            plan: Some(plan),
            options,
            state: RunState::Running,
            ready,
            trace: VecDeque::new(),
            trace_dropped: 0,
            operations: 0,
            versions: vec![0; variables.len()],
            variables,
            state_bytes,
            now: 0,
            next_timer: 0,
            timers: BTreeMap::new(),
            transitions: vec![RunState::Created, RunState::Starting, RunState::Running],
            deadline: None,
            last_error: None,
            cancel_observed: false,
            cleanup_truncated: 0,
            stop_ready,
            segment_owner: None,
            last_segment: None,
            segment_indices: vec![],
            segment_writes: vec![],
            dirty: vec![false; variable_count],
        })
    }
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn state(&self) -> RunState {
        self.state
    }
    pub fn transitions(&self) -> &[RunState] {
        &self.transitions
    }
    pub fn last_error(&self) -> Option<RunError> {
        self.last_error
    }
    pub fn cleanup_truncated(&self) -> u64 {
        self.cleanup_truncated
    }
    pub fn pending_continuations(&self) -> usize {
        self.timers.len()
    }
    /// IDs written by the last/current segment, once each. No equality comparison,
    /// change notification or Watch delivery is implied by this inspection seam.
    pub fn segment_writes(&self) -> Option<(u64, &[VariableId])> {
        self.last_segment
            .map(|owner| (owner, self.segment_writes.as_slice()))
    }
    fn transition(&mut self, state: RunState) {
        if self.state != state {
            self.state = state;
            self.transitions.push(state);
        }
    }
    pub fn now(&self) -> u64 {
        self.now
    }
    /// Advancing time enqueues no inline graph execution; pump resumes due waits.
    pub fn advance_to(&mut self, now: u64) -> Result<(), RunError> {
        if !matches!(
            self.state,
            RunState::Running | RunState::Draining | RunState::Cancelling
        ) {
            return Err(RunError::InvalidState);
        }
        if now < self.now {
            return Err(RunError::ClockWentBackwards);
        }
        self.now = now;
        Ok(())
    }
    pub fn trace(&self) -> &VecDeque<RunEvent> {
        &self.trace
    }
    pub fn trace_dropped(&self) -> u64 {
        self.trace_dropped
    }
    /// Explicit run-state inspection; activation-local cells are not shared.
    pub fn variable(&self, id: VariableId) -> Option<&Value> {
        let index = self
            .plan
            .as_ref()?
            .variables()
            .iter()
            .position(|v| v.id() == id && v.scope() == VarScope::Run)?;
        self.variables[index].as_ref()
    }
    fn record(&mut self, event: RunEvent) {
        if self.options.limits.max_trace == 0 {
            self.trace_dropped = self.trace_dropped.saturating_add(1);
            return;
        }
        if self.trace.len() == self.options.limits.max_trace {
            self.trace.pop_front();
            self.trace_dropped = self.trace_dropped.saturating_add(1);
        }
        self.trace.push_back(event);
    }
    /// Drive at most `budget` control/data/timer work items. Idle Running is live.
    /// Native callbacks are trusted; this is not a wall-time preemption guarantee.
    pub fn pump(&mut self, budget: usize) -> Result<RunProgress, RunError> {
        if !matches!(
            self.state,
            RunState::Running | RunState::Draining | RunState::Cancelling
        ) {
            return Err(RunError::InvalidState);
        }
        self.observe_cancellation()?;
        if self.deadline.is_some_and(|deadline| self.now >= deadline) {
            if self.state == RunState::Draining {
                self.request_stop(StopMode::Immediate)?;
            } else if self.state == RunState::Cancelling {
                self.abort_cleanup();
            }
        }
        let available = if self.state == RunState::Cancelling {
            usize::MAX
        } else {
            self.options
                .limits
                .max_operations
                .saturating_sub(self.operations)
        };
        let mut remaining = budget.min(available);
        let initial = remaining;
        let mut last = None;
        while remaining > 0
            && matches!(
                self.state,
                RunState::Running | RunState::Draining | RunState::Cancelling
            )
        {
            self.observe_cancellation()?;
            if self.state == RunState::Stopped {
                break;
            }
            if self
                .timers
                .first_key_value()
                .is_some_and(|((at, _), _)| *at <= self.now)
            {
                remaining -= 1;
                let (_, timer) = self.timers.pop_first().expect("due timer");
                if timer.run != self.id {
                    return self.fail(RunError::InvalidState);
                }
                self.record(RunEvent {
                    activation: timer.activation.sequence,
                    node: self.plan.as_ref().unwrap().nodes()[timer.owner.index()].node(),
                    outcome: ControlOutcome::Then,
                    fault: None,
                });
                self.ready.push_back(timer.activation);
                continue;
            }
            let Some(mut activation) = self.ready.pop_front() else {
                break;
            };
            last = Some(activation.sequence);
            let mut quantum = if activation.cleanup {
                remaining.min(activation.cleanup_remaining)
            } else {
                remaining
            };
            if quantum == 0 {
                self.cleanup_truncated = self.cleanup_truncated.saturating_add(1);
                remaining -= 1;
                continue;
            }
            let before = quantum;
            if self.segment_owner != Some(activation.sequence) {
                for index in self.segment_indices.drain(..) {
                    self.dirty[index] = false;
                }
                self.segment_writes.clear();
                self.segment_owner = Some(activation.sequence);
                self.last_segment = Some(activation.sequence);
            }
            let result = self.execute(&mut activation, &mut quantum);
            let used = before - quantum;
            remaining -= used;
            if activation.cleanup {
                activation.cleanup_remaining -= used;
            }
            if let Err(error) = result {
                self.operations = self.operations.saturating_add(initial - remaining);
                if activation.cleanup {
                    self.cleanup_truncated = self.cleanup_truncated.saturating_add(1);
                    self.finish_if_idle();
                    return Err(error);
                }
                return self.fail(error);
            }
            if self.state == RunState::Faulted {
                self.cancel_owned()?;
                break;
            }
            if let Some((at, owner)) = activation.waiting.take() {
                self.segment_owner = None;
                if self.timers.len() == self.options.limits.max_timers {
                    return self.fail(RunError::ResourceLimit);
                }
                let sequence = self.next_timer;
                let Some(next) = sequence.checked_add(1) else {
                    return self.fail(RunError::ResourceLimit);
                };
                self.next_timer = next;
                self.timers.insert(
                    (at, sequence),
                    Timer {
                        run: self.id,
                        owner,
                        activation,
                    },
                );
            } else if activation.pc.is_some() || !activation.stack.is_empty() {
                if activation.yielded {
                    self.segment_owner = None;
                    activation.yielded = false;
                    self.ready.push_back(activation);
                } else {
                    self.ready.push_front(activation);
                }
            } else {
                self.segment_owner = None;
            }
        }
        // A pending Set owns its read/compute/write segment across host slices.
        // Explicit suspension still releases ownership; read-only pulls may rotate.
        if remaining == 0
            && self.ready.front().is_some_and(|a| {
                Some(a.sequence) == last
                    && !(a.pull.is_some()
                        && a.pc.is_some_and(|pc| {
                            matches!(
                                self.plan.as_ref().unwrap().nodes()[pc.index()].operation(),
                                ControlOperation::Set { .. }
                            )
                        }))
            })
            && let Some(activation) = self.ready.pop_front()
        {
            self.ready.push_back(activation);
        }
        self.segment_owner = None;
        let count = initial - remaining;
        self.operations = self.operations.saturating_add(count);
        if budget > 0
            && self.state != RunState::Cancelling
            && self.state != RunState::Stopped
            && self.operations == self.options.limits.max_operations
            && (!self.ready.is_empty() || !self.timers.is_empty())
        {
            return self.fail(RunError::ResourceLimit);
        }
        self.finish_if_idle();
        let timer = self.timers.first_key_value().map(|((at, _), _)| *at);
        let next_wakeup = match (timer, self.deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        Ok(RunProgress {
            operations: count,
            ready: !self.ready.is_empty()
                || self
                    .timers
                    .first_key_value()
                    .is_some_and(|((at, _), _)| *at <= self.now),
            next_wakeup,
        })
    }
    fn observe_cancellation(&mut self) -> Result<(), RunError> {
        let cancelled = self
            .options
            .data
            .cancel
            .as_ref()
            .is_some_and(crate::CancelToken::is_cancelled);
        if cancelled && !self.cancel_observed {
            self.request_stop(StopMode::Immediate)?;
        }
        self.cancel_observed = cancelled;
        Ok(())
    }
    fn fail<T>(&mut self, error: RunError) -> Result<T, RunError> {
        self.last_error = Some(error);
        self.transition(RunState::Faulted);
        if self.cancel_owned().is_err() {
            self.abort_cleanup();
        }
        Err(error)
    }
    fn finish_if_idle(&mut self) {
        if matches!(self.state, RunState::Draining | RunState::Cancelling)
            && self.ready.is_empty()
            && self.timers.is_empty()
        {
            self.deadline = None;
            self.transition(RunState::Stopped);
        }
    }
    fn abort_cleanup(&mut self) {
        self.cleanup_truncated = self
            .cleanup_truncated
            .saturating_add(self.ready.len() as u64);
        self.stop_ready.clear();
        self.ready.clear();
        self.timers.clear();
        self.deadline = None;
        self.transition(RunState::Stopped);
    }
    fn execute(&mut self, activation: &mut Activation, budget: &mut usize) -> Result<(), RunError> {
        let plan = self.plan.as_ref().ok_or(RunError::InvalidState)?.clone();
        let Some(ix) = activation.pc else {
            *budget -= 1;
            let Some(cursor) = activation.stack.pop() else {
                return Ok(());
            };
            match cursor {
                Cursor::While { owner, iterations } => {
                    activation.pc = Some(owner);
                    activation.reenter = Some((owner, iterations));
                    activation.yielded = true;
                }
                Cursor::Count { owner, next, count } => {
                    let node = &plan.nodes()[owner.index()];
                    let ControlOperation::Count {
                        body, completed, ..
                    } = node.operation()
                    else {
                        return Err(RunError::InvalidState);
                    };
                    if next < count {
                        activation.stack.push(Cursor::Count {
                            owner,
                            next: next + 1,
                            count,
                        });
                        Self::publish_index(&plan, activation, owner, Some(next));
                        activation.pc = *body;
                        activation.yielded = true;
                        self.record(RunEvent {
                            activation: activation.sequence,
                            node: node.node(),
                            outcome: ControlOutcome::Body,
                            fault: None,
                        });
                    } else {
                        Self::publish_index(&plan, activation, owner, None);
                        activation.pc = *completed;
                        self.record(RunEvent {
                            activation: activation.sequence,
                            node: node.node(),
                            outcome: ControlOutcome::Completed,
                            fault: None,
                        });
                    }
                }
                Cursor::Sequence { owner, next } => {
                    let ControlOperation::Sequence { steps } =
                        plan.nodes()[owner.index()].operation()
                    else {
                        return Err(RunError::InvalidState);
                    };
                    if next < steps.len() {
                        activation.pc = steps[next];
                        activation.stack.push(Cursor::Sequence {
                            owner,
                            next: next + 1,
                        });
                        self.record(RunEvent {
                            activation: activation.sequence,
                            node: plan.nodes()[owner.index()].node(),
                            outcome: ControlOutcome::Step(next),
                            fault: None,
                        });
                    }
                }
            }
            return Ok(());
        };
        let node = &plan.nodes()[ix.index()];
        if activation.pull.is_none() {
            *budget -= 1;
        }
        if activation.cleanup
            && !matches!(
                node.operation(),
                ControlOperation::Entry
                    | ControlOperation::Branch { .. }
                    | ControlOperation::Sequence { .. }
                    | ControlOperation::Set { .. }
            )
        {
            let fault = ConstantFault {
                source: node.node(),
                fault: KernelFault {
                    code: "core/Cancelled",
                    message_key: "core/cleanup-unsafe-node",
                    message: "This operation is not permitted during cancellation cleanup.",
                },
            };
            Self::publish_error(&plan, activation, ix, &fault);
            self.record(RunEvent {
                activation: activation.sequence,
                node: node.node(),
                outcome: ControlOutcome::Failed,
                fault: Some(fault),
            });
            activation.pc = node.failed();
            activation.pull = None;
            if activation.pc.is_none() {
                activation.stack.clear();
            }
            return Ok(());
        }
        let (next, outcome, fault) = match node.operation() {
            ControlOperation::Entry => (node.then(), ControlOutcome::Then, None),
            ControlOperation::While {
                condition,
                body,
                completed,
                max_iterations,
            } => {
                let iterations = activation
                    .reenter
                    .filter(|(owner, _)| *owner == ix)
                    .map_or(0, |(_, n)| n);
                let Some(result) = self.consume(activation, *condition, budget)? else {
                    return Ok(());
                };
                match result {
                    FoldedValue::Fault(fault) => {
                        (node.failed(), ControlOutcome::Failed, Some(fault))
                    }
                    FoldedValue::Value(Value::Bool(false)) => {
                        (*completed, ControlOutcome::Completed, None)
                    }
                    FoldedValue::Value(Value::Bool(true)) if iterations >= *max_iterations => (
                        node.failed(),
                        ControlOutcome::Failed,
                        Some(ConstantFault {
                            source: node.node(),
                            fault: KernelFault {
                                code: "core/LoopBudgetExceeded",
                                message_key: "core/loop-budget-exceeded",
                                message: "The loop iteration budget is exhausted.",
                            },
                        }),
                    ),
                    FoldedValue::Value(Value::Bool(true)) => {
                        if activation.stack.len() == self.options.limits.max_region_depth {
                            return Err(RunError::ResourceLimit);
                        }
                        activation.stack.push(Cursor::While {
                            owner: ix,
                            iterations: iterations + 1,
                        });
                        (*body, ControlOutcome::Body, None)
                    }
                    _ => return Err(DataError::KernelContract { node: node.node() }.into()),
                }
            }
            ControlOperation::Count {
                count,
                body,
                completed,
            } => {
                let Some(result) = self.consume(activation, *count, budget)? else {
                    return Ok(());
                };
                match result {
                    FoldedValue::Fault(fault) => {
                        (node.failed(), ControlOutcome::Failed, Some(fault))
                    }
                    FoldedValue::Value(Value::I64(count)) if count < 0 => (
                        node.failed(),
                        ControlOutcome::Failed,
                        Some(ConstantFault {
                            source: node.node(),
                            fault: KernelFault {
                                code: "core/OutOfRange",
                                message_key: "core/count-out-of-range",
                                message: "Loop count must be nonnegative.",
                            },
                        }),
                    ),
                    FoldedValue::Value(Value::I64(0)) => {
                        (*completed, ControlOutcome::Completed, None)
                    }
                    FoldedValue::Value(Value::I64(count)) => {
                        if activation.stack.len() == self.options.limits.max_region_depth {
                            return Err(RunError::ResourceLimit);
                        }
                        activation.stack.push(Cursor::Count {
                            owner: ix,
                            next: 1,
                            count,
                        });
                        Self::publish_index(&plan, activation, ix, Some(0));
                        (*body, ControlOutcome::Body, None)
                    }
                    _ => return Err(DataError::KernelContract { node: node.node() }.into()),
                }
            }
            ControlOperation::Break | ControlOperation::Continue => {
                let owner = node.loop_owner().ok_or(RunError::InvalidState)?;
                let position = activation
                    .stack
                    .iter()
                    .rposition(|c| matches!(c,Cursor::Count {owner:active,..} | Cursor::While {owner:active,..} if *active==owner))
                    .ok_or(RunError::InvalidState)?;
                let completed = match plan.nodes()[owner.index()].operation() {
                    ControlOperation::Count { completed, .. }
                    | ControlOperation::While { completed, .. } => *completed,
                    _ => return Err(RunError::InvalidState),
                };
                if matches!(node.operation(), ControlOperation::Break) {
                    activation.stack.truncate(position);
                    Self::publish_index(&plan, activation, owner, None);
                    (completed, ControlOutcome::Break, None)
                } else {
                    activation.stack.truncate(position + 1);
                    activation.yielded = true;
                    (None, ControlOutcome::Continue, None)
                }
            }
            ControlOperation::Yield => {
                activation.yielded = true;
                (node.then(), ControlOutcome::Yielded, None)
            }
            ControlOperation::Wait { duration } => {
                let Some(result) = self.consume(activation, *duration, budget)? else {
                    return Ok(());
                };
                match result {
                    FoldedValue::Fault(fault) => {
                        (node.failed(), ControlOutcome::Failed, Some(fault))
                    }
                    FoldedValue::Value(Value::Duration(duration)) => {
                        if let Some(at) = u64::try_from(duration)
                            .ok()
                            .and_then(|d| self.now.checked_add(d))
                        {
                            activation.waiting = Some((at, ix));
                            (node.then(), ControlOutcome::Suspended, None)
                        } else {
                            (
                                node.failed(),
                                ControlOutcome::Failed,
                                Some(ConstantFault {
                                    source: node.node(),
                                    fault: KernelFault {
                                        code: "core/OutOfRange",
                                        message_key: "core/wait-out-of-range",
                                        message: "Wait duration is outside the clock range.",
                                    },
                                }),
                            )
                        }
                    }
                    _ => return Err(DataError::KernelContract { node: node.node() }.into()),
                }
            }
            ControlOperation::Sequence { steps } => {
                if activation.stack.len() == self.options.limits.max_region_depth {
                    return Err(RunError::ResourceLimit);
                }
                activation
                    .stack
                    .push(Cursor::Sequence { owner: ix, next: 1 });
                (
                    steps.first().copied().flatten(),
                    ControlOutcome::Step(0),
                    None,
                )
            }
            ControlOperation::Branch {
                condition,
                when_true,
                when_false,
            } => {
                let Some(result) = self.consume(activation, *condition, budget)? else {
                    return Ok(());
                };
                match result {
                    FoldedValue::Value(Value::Bool(true)) => {
                        (*when_true, ControlOutcome::True, None)
                    }
                    FoldedValue::Value(Value::Bool(false)) => {
                        (*when_false, ControlOutcome::False, None)
                    }
                    FoldedValue::Fault(fault) => {
                        (node.failed(), ControlOutcome::Failed, Some(fault))
                    }
                    _ => return Err(DataError::KernelContract { node: node.node() }.into()),
                }
            }
            ControlOperation::Set { variable, value } => {
                let Some(result) = self.consume(activation, *value, budget)? else {
                    return Ok(());
                };
                match result {
                    FoldedValue::Fault(fault) => {
                        (node.failed(), ControlOutcome::Failed, Some(fault))
                    }
                    FoldedValue::Value(value) => {
                        let var = &plan.variables()[*variable];
                        let (store, retained) = if var.scope() == VarScope::Local {
                            (&mut activation.locals, &mut activation.local_bytes)
                        } else {
                            (&mut self.variables, &mut self.state_bytes)
                        };
                        let old = store[*variable]
                            .as_ref()
                            .and_then(|v| var.charge(v))
                            .unwrap_or(16);
                        let new = var
                            .charge(&value)
                            .ok_or(DataError::KernelContract { node: node.node() })?;
                        let bytes = retained
                            .checked_sub(old)
                            .and_then(|b| b.checked_add(new))
                            .filter(|b| *b <= self.options.limits.max_state_bytes)
                            .ok_or(RunError::ResourceLimit)?;
                        let version = self.versions[*variable]
                            .checked_add(1)
                            .ok_or(RunError::ResourceLimit)?;
                        *retained = bytes;
                        store[*variable] = Some(value);
                        self.versions[*variable] = version;
                        if !self.dirty[*variable] {
                            self.dirty[*variable] = true;
                            self.segment_indices.push(*variable);
                            self.segment_writes.push(var.id());
                        }
                        (node.then(), ControlOutcome::Then, None)
                    }
                }
            }
        };
        activation.pull = None;
        activation.reenter = None;
        activation.inputs.clear();
        if let Some(fault) = &fault {
            Self::publish_error(&plan, activation, ix, fault);
        }
        self.record(RunEvent {
            activation: activation.sequence,
            node: node.node(),
            outcome,
            fault,
        });
        if outcome == ControlOutcome::Failed && next.is_none() {
            activation.stack.clear();
        }
        if outcome == ControlOutcome::Failed
            && next.is_none()
            && !activation.cleanup
            && plan.nodes()[activation.entry.index()].fault_policy() == FaultPolicy::StopRun
        {
            self.transition(RunState::Faulted);
        }
        activation.pc = next;
        Ok(())
    }
    fn consume(
        &mut self,
        activation: &mut Activation,
        root: usize,
        budget: &mut usize,
    ) -> Result<Option<FoldedValue>, RunError> {
        let plan = self.plan.as_ref().ok_or(RunError::InvalidState)?;
        if activation.pull.is_none() {
            activation.data.begin_epoch()?;
            activation.pull = Some(root);
            activation.pending_input = None;
            activation.inputs.clear();
            for &i in plan.inputs_for(root).ok_or(RunError::InvalidState)? {
                let source = &plan.host_sources()[i];
                let value = match source {
                    HostSource::Variable(index) => {
                        let var = &plan.variables()[*index];
                        let cell = if var.scope() == VarScope::Local {
                            &activation.locals[*index]
                        } else {
                            &self.variables[*index]
                        };
                        Some(match cell {
                            Some(value) => FoldedValue::Value(value.clone()),
                            None => FoldedValue::Fault(ConstantFault {
                                source: plan.data().nodes()[plan.data().host_inputs()[i].index()]
                                    .node(),
                                fault: KernelFault {
                                    code: "core/UninitializedVariable",
                                    message_key: "core/uninitialized-variable",
                                    message: "The variable has not been initialized.",
                                },
                            }),
                        })
                    }
                    HostSource::Error(_) | HostSource::Index(_) => activation.outputs[i].clone(),
                };
                activation.inputs.push(value);
            }
        }
        loop {
            if let Some(index) = activation.pending_input {
                if *budget == 0 {
                    return Ok(None);
                }
                *budget -= 1;
                let offset = plan
                    .inputs_for(root)
                    .ok_or(RunError::InvalidState)?
                    .binary_search(&index)
                    .map_err(|_| RunError::InvalidState)?;
                let value = activation.inputs[offset].clone().ok_or_else(|| {
                    RunError::Data(DataError::MissingInput {
                        node: plan.data().nodes()[plan.data().host_inputs()[index].index()].node(),
                    })
                })?;
                activation.data.set_input_outcome(index, value)?;
                activation.pending_input = None;
            }
            match activation.data.poll_host_export(root, budget)? {
                DataPoll::Pending => {}
                DataPoll::Ready(Some(value)) => {
                    return Ok(Some(match value.outcome {
                        Ok(value) => FoldedValue::Value(value.clone()),
                        Err(fault) => FoldedValue::Fault(fault.clone()),
                    }));
                }
                DataPoll::Ready(None) => return Err(RunError::InvalidState),
            }
            activation.pending_input = activation.data.take_requested_input();
            if activation.pending_input.is_none() {
                return Ok(None);
            }
        }
    }
    fn publish_index(
        plan: &ControlPlan,
        activation: &mut Activation,
        owner: ControlIx,
        index: Option<i64>,
    ) {
        for &i in plan.index_targets(owner) {
            activation.outputs[i] = index.map(|i| FoldedValue::Value(Value::I64(i)));
        }
    }
    fn publish_error(
        plan: &ControlPlan,
        activation: &mut Activation,
        owner: ControlIx,
        fault: &ConstantFault,
    ) {
        for &i in plan.error_targets(owner) {
            activation.outputs[i] = Some(FoldedValue::Value(Value::Err(Arc::new(
                orange_model::value::NodeError {
                    code: fault.fault.code.into(),
                    node: Some(fault.source),
                },
            ))));
        }
    }
    pub fn stop(&mut self) -> Result<(), RunError> {
        self.request_stop(StopMode::Immediate)
    }
    /// Graceful stop starts On Stop once and drains accepted work. Immediate stop
    /// cancels owned waits; a second stop aborts bounded cancellation cleanup.
    pub fn request_stop(&mut self, mode: StopMode) -> Result<(), RunError> {
        match (self.state, mode) {
            (
                RunState::Stopped | RunState::Disposed | RunState::Created | RunState::Starting,
                _,
            ) => Err(RunError::InvalidState),
            (RunState::Cancelling, _) => {
                self.abort_cleanup();
                Ok(())
            }
            (RunState::Draining, StopMode::Graceful) => Err(RunError::InvalidState),
            (RunState::Running, StopMode::Graceful) => {
                let deadline = self
                    .now
                    .checked_add(self.options.drain_timeout_ns)
                    .ok_or(RunError::ResourceLimit)?;
                self.transition(RunState::Draining);
                self.deadline = Some(deadline);
                self.ready.append(&mut self.stop_ready);
                self.finish_if_idle();
                Ok(())
            }
            _ => self.cancel_owned(),
        }
    }
    fn cancel_owned(&mut self) -> Result<(), RunError> {
        // Cancellation must remain possible at the end of the virtual clock range.
        let deadline = self.now.saturating_add(self.options.cleanup_timeout_ns);
        self.transition(RunState::Cancelling);
        self.deadline = Some(self.deadline.map_or(deadline, |old| old.min(deadline)));
        self.stop_ready.clear();
        self.ready.clear();
        let timers = std::mem::take(&mut self.timers);
        let plan = self.plan.as_ref().ok_or(RunError::InvalidState)?.clone();
        for (_, mut timer) in timers {
            let node = &plan.nodes()[timer.owner.index()];
            self.record(RunEvent {
                activation: timer.activation.sequence,
                node: node.node(),
                outcome: ControlOutcome::Cancelled,
                fault: None,
            });
            if let Some(target) = node.cancelled() {
                timer
                    .activation
                    .data
                    .enter_cleanup(self.options.max_cleanup_operations)?;
                timer.activation.pc = Some(target);
                timer.activation.pull = None;
                timer.activation.inputs.clear();
                timer.activation.stack.clear();
                timer.activation.reenter = None;
                timer.activation.cleanup = true;
                timer.activation.cleanup_remaining = self.options.max_cleanup_operations;
                self.ready.push_back(timer.activation);
            }
        }
        self.finish_if_idle();
        if self.deadline.is_some_and(|d| self.now >= d) {
            self.abort_cleanup();
        }
        Ok(())
    }
    pub fn dispose(&mut self) -> Result<(), RunError> {
        if self.state != RunState::Stopped {
            return Err(RunError::InvalidState);
        }
        self.plan = None;
        self.variables.clear();
        self.versions.clear();
        self.state_bytes = 0;
        self.stop_ready.clear();
        self.dirty.clear();
        self.segment_indices.clear();
        self.segment_writes.clear();
        self.segment_owner = None;
        self.last_segment = None;
        self.trace.clear();
        self.transition(RunState::Disposed);
        Ok(())
    }
}
