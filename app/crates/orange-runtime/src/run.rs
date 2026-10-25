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
    Running,
    Faulted,
    Stopped,
    Disposed,
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
            max_operations: 10_000_000,
            max_trace: 1024,
            max_state_bytes: 16 * 1024 * 1024,
            max_region_depth: 1024,
            max_timers: 1024,
        }
    }
}
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    pub limits: RunLimits,
    pub data: DataOptions,
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
    supplied: usize,
    outputs: Vec<Option<FoldedValue>>,
    stack: Vec<Cursor>,
    waiting: Option<(u64, ControlIx)>,
    yielded: bool,
    reenter: Option<(ControlIx, u64)>,
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
        if !plan.data().is_current(document, plan.data().snapshot()) {
            return Err(DataError::StaleDocument.into());
        }
        if registry.generation() != plan.data().snapshot().generation() {
            return Err(DataError::StaleRegistry.into());
        }
        if plan.starts().len() > options.limits.max_activations {
            return Err(RunError::ResourceLimit);
        }
        let mut ready = VecDeque::new();
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
        for (sequence, entry) in plan.starts().iter().copied().enumerate() {
            ready.push_back(Activation {
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
                supplied: 0,
                outputs: vec![None; plan.host_sources().len()],
                stack: vec![],
                waiting: None,
                yielded: false,
                reenter: None,
            });
        }
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
        })
    }
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn state(&self) -> RunState {
        self.state
    }
    pub fn now(&self) -> u64 {
        self.now
    }
    /// Advancing time enqueues no inline graph execution; pump resumes due waits.
    pub fn advance_to(&mut self, now: u64) -> Result<(), RunError> {
        if self.state != RunState::Running {
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
    /// Drive at most `budget` control instructions. Idle Running is a live run.
    pub fn pump(&mut self, budget: usize) -> Result<RunProgress, RunError> {
        if self.state != RunState::Running {
            return Err(RunError::InvalidState);
        }
        let available = self
            .options
            .limits
            .max_operations
            .saturating_sub(self.operations);
        let mut remaining = budget.min(available);
        let initial = remaining;
        while remaining > 0 && self.state == RunState::Running {
            if self
                .timers
                .first_key_value()
                .is_some_and(|((at, _), _)| *at <= self.now)
            {
                remaining -= 1;
                let (_, timer) = self.timers.pop_first().expect("due timer");
                if timer.run != self.id {
                    return Err(RunError::InvalidState);
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
            let result = self.execute(&mut activation, &mut remaining);
            if let Err(error) = result {
                self.ready.clear();
                self.timers.clear();
                self.state = RunState::Faulted;
                return Err(error);
            }
            if let Some((at, owner)) = activation.waiting.take() {
                if self.timers.len() == self.options.limits.max_timers {
                    self.ready.clear();
                    self.timers.clear();
                    self.state = RunState::Faulted;
                    return Err(RunError::ResourceLimit);
                }
                let sequence = self.next_timer;
                self.next_timer = sequence.checked_add(1).ok_or(RunError::ResourceLimit)?;
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
                    activation.yielded = false;
                    self.ready.push_back(activation);
                } else {
                    self.ready.push_front(activation);
                }
            }
        }
        let count = initial - remaining;
        self.operations += count;
        if budget > 0
            && self.operations == self.options.limits.max_operations
            && (!self.ready.is_empty() || !self.timers.is_empty())
        {
            self.ready.clear();
            self.timers.clear();
            self.state = RunState::Faulted;
            return Err(RunError::ResourceLimit);
        }
        Ok(RunProgress {
            operations: count,
            ready: !self.ready.is_empty()
                || self
                    .timers
                    .first_key_value()
                    .is_some_and(|((at, _), _)| *at <= self.now),
            next_wakeup: self.timers.first_key_value().map(|((at, _), _)| *at),
        })
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
                        (node.then(), ControlOutcome::Then, None)
                    }
                }
            }
        };
        activation.pull = None;
        activation.reenter = None;
        activation.inputs.clear();
        if let Some(fault) = &fault {
            for (i, source) in plan.host_sources().iter().enumerate() {
                if matches!(source, HostSource::Error(owner) if *owner == ix) {
                    activation.outputs[i] = Some(FoldedValue::Value(Value::Err(Arc::new(
                        orange_model::value::NodeError {
                            code: fault.fault.code.into(),
                            node: Some(fault.source),
                        },
                    ))));
                }
            }
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
            && plan.nodes()[activation.entry.index()].fault_policy() == FaultPolicy::StopRun
        {
            self.ready.clear();
            self.state = RunState::Faulted;
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
            activation.supplied = 0;
            activation.inputs.clear();
            for (i, source) in plan.host_sources().iter().enumerate() {
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
        while activation.supplied < activation.inputs.len() {
            if *budget == 0 {
                return Ok(None);
            }
            *budget -= 1;
            if let Some(value) = &activation.inputs[activation.supplied] {
                activation
                    .data
                    .set_input_outcome(activation.supplied, value.clone())?;
            }
            activation.supplied += 1;
        }
        match activation.data.poll_export(root, budget)? {
            DataPoll::Pending => Ok(None),
            DataPoll::Ready(Some(value)) => Ok(Some(match value.outcome {
                Ok(value) => FoldedValue::Value(value.clone()),
                Err(fault) => FoldedValue::Fault(fault.clone()),
            })),
            DataPoll::Ready(None) => Err(RunError::InvalidState),
        }
    }
    fn publish_index(
        plan: &ControlPlan,
        activation: &mut Activation,
        owner: ControlIx,
        index: Option<i64>,
    ) {
        for (i, source) in plan.host_sources().iter().enumerate() {
            if matches!(source,HostSource::Index(active) if *active==owner) {
                activation.outputs[i] = index.map(|i| FoldedValue::Value(Value::I64(i)));
            }
        }
    }
    pub fn stop(&mut self) -> Result<(), RunError> {
        if self.state != RunState::Running && self.state != RunState::Faulted {
            return Err(RunError::InvalidState);
        }
        self.ready.clear();
        self.timers.clear();
        self.state = RunState::Stopped;
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
        self.trace.clear();
        self.state = RunState::Disposed;
        Ok(())
    }
}
