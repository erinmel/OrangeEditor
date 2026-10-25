//! A host-driven logical actor. No worker thread, OS clock or editor dependency.
use crate::{DataActivation, DataError, DataOptions};
use orange_compiler::{ConstantFault, ControlIx, ControlOperation, ControlPlan};
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::FaultPolicy;
use orange_model::id::NodeId;
use orange_model::{GraphDocument, Value};
use std::collections::VecDeque;
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
}
impl Default for RunLimits {
    fn default() -> Self {
        Self {
            max_activations: 256,
            max_operations: 10_000_000,
            max_trace: 1024,
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
}
impl RunError {
    pub const fn message_key(self) -> &'static str {
        match self {
            Self::Data(e) => e.message_key(),
            Self::InvalidState => "core/run-invalid-state",
            Self::ResourceLimit => "core/run-resource-limit",
            Self::IdentityExhausted => "core/run-identity-exhausted",
        }
    }
}
struct Activation {
    sequence: u64,
    entry: ControlIx,
    pc: Option<ControlIx>,
    data: DataActivation,
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
        })
    }
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn state(&self) -> RunState {
        self.state
    }
    pub fn trace(&self) -> &VecDeque<RunEvent> {
        &self.trace
    }
    pub fn trace_dropped(&self) -> u64 {
        self.trace_dropped
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
        let mut count = 0;
        while count < budget {
            let Some(mut activation) = self.ready.pop_front() else {
                break;
            };
            self.operations = self
                .operations
                .checked_add(1)
                .filter(|n| *n <= self.options.limits.max_operations)
                .ok_or(RunError::ResourceLimit)?;
            let result = self.execute(&mut activation);
            if let Err(error) = result {
                self.ready.clear();
                self.state = RunState::Faulted;
                return Err(error);
            }
            count += 1;
            if activation.pc.is_some() {
                self.ready.push_back(activation);
            }
        }
        Ok(RunProgress {
            operations: count,
            ready: !self.ready.is_empty(),
            next_wakeup: None,
        })
    }
    fn execute(&mut self, activation: &mut Activation) -> Result<(), RunError> {
        let plan = self.plan.as_ref().ok_or(RunError::InvalidState)?.clone();
        let ix = activation.pc.ok_or(RunError::InvalidState)?;
        let node = &plan.nodes()[ix.index()];
        let (next, outcome, fault) = match node.operation() {
            ControlOperation::Entry => (node.then(), ControlOutcome::Then, None),
            ControlOperation::Branch {
                condition,
                when_true,
                when_false,
            } => {
                activation.data.begin_epoch()?;
                let result = activation
                    .data
                    .export(*condition)?
                    .ok_or(DataError::KernelContract { node: node.node() })?;
                match result.outcome {
                    Ok(Value::Bool(true)) => (*when_true, ControlOutcome::True, None),
                    Ok(Value::Bool(false)) => (*when_false, ControlOutcome::False, None),
                    Err(fault) => (node.failed(), ControlOutcome::Failed, Some(fault.clone())),
                    _ => return Err(DataError::KernelContract { node: node.node() }.into()),
                }
            }
        };
        self.record(RunEvent {
            activation: activation.sequence,
            node: node.node(),
            outcome,
            fault,
        });
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
    pub fn stop(&mut self) -> Result<(), RunError> {
        if self.state != RunState::Running && self.state != RunState::Faulted {
            return Err(RunError::InvalidState);
        }
        self.ready.clear();
        self.state = RunState::Stopped;
        Ok(())
    }
    pub fn dispose(&mut self) -> Result<(), RunError> {
        if self.state != RunState::Stopped {
            return Err(RunError::InvalidState);
        }
        self.plan = None;
        self.trace.clear();
        self.state = RunState::Disposed;
        Ok(())
    }
}
