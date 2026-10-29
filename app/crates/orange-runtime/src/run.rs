//! A host-driven logical actor. No worker thread, OS clock or editor dependency.
use crate::data::{CleanupScope, HostPoll, PreparedData};
use crate::{DataActivation, DataError, DataOptions};
use orange_compiler::{
    ConstantFault, ControlIx, ControlNode, ControlOperation, ControlPlan, ExecInput, ExecTarget,
    FoldedValue, ForkJoin, HostSource, IterationProjection, KernelFault, SwitchKey, UpdateDemand,
    UpdateOperation, UpdateProjection,
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

mod cells;
mod function;
mod iteration;
mod loops;
mod retained;
mod select;
use cells::SegmentWrites;
use function::{FrameStorage, FunctionFrame, FunctionReturn, PendingCall, find_frame, frame_value};
mod ownership;
use iteration::{ForEachCursor, IterationFrame};
mod ids;
mod ready;
mod timers;
mod update;
use ids::{FrameId, OwnerIds, RegionId, StrandId};
use ownership::{IdSet, Region, RegionKind};
use ready::ReadyQueue;
use timers::{TimerKey, Timers};

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
    Suppressed,
    StateChanged,
    FlipA,
    FlipB,
    Case(u16),
    Default,
    MatchedOk,
    MatchedErr,
    Forked,
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
    /// Live strands, including suspended region continuations and stop reserve.
    pub max_strands: usize,
    /// Live runtime region instances, including stop reserve.
    pub max_regions: usize,
    /// Separately reserved On Stop frames, admitted and allocated at startup.
    pub max_stop_activations: usize,
    pub max_operations: usize,
    pub max_trace: usize,
    pub max_state_bytes: usize,
    pub max_region_depth: usize,
    pub max_timers: usize,
    /// Aggregate nominal storage that all start and On Stop frames reserve at
    /// startup: data cells, memo stamps, work/scratch capacity, entropy counters
    /// and run-owned frame cells, multiplied by the frame count. Values are
    /// shared clones charged by `max_state_bytes`, not by this limit.
    pub max_frame_storage_bytes: usize,
}
impl Default for RunLimits {
    fn default() -> Self {
        Self {
            max_activations: 256,
            max_strands: 4096,
            max_regions: 8192,
            max_stop_activations: 256,
            max_operations: 10_000_000,
            max_trace: 1024,
            max_state_bytes: 16 * 1024 * 1024,
            max_region_depth: 1024,
            max_timers: 1024,
            max_frame_storage_bytes: 64 * 1024 * 1024,
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
/// Retained owner counts, including reserved On Stop frames. No graph values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunOwnership {
    pub activations: usize,
    pub strands: usize,
    pub regions: usize,
    pub timers: usize,
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
    #[error("A provider callback panicked during a pump; stop the run.")]
    Interrupted,
}
impl RunError {
    pub const fn message_key(self) -> &'static str {
        match self {
            Self::Data(e) => e.message_key(),
            Self::InvalidState => "core/run-invalid-state",
            Self::ResourceLimit => "core/run-resource-limit",
            Self::IdentityExhausted => "core/run-identity-exhausted",
            Self::ClockWentBackwards => "core/run-clock-went-backwards",
            Self::Interrupted => "core/run-interrupted",
        }
    }
}
/// Dense storage follows ownership, while plan variable/source identities stay stable.
#[derive(Default)]
struct StateLayout {
    variables: Box<[usize]>,
    states: Box<[usize]>,
    state_sources: Box<[usize]>,
    outputs: Box<[Option<usize>]>,
    output_count: usize,
}
impl StateLayout {
    fn new(plan: &ControlPlan) -> Self {
        fn indices<Id: Copy>(cells: &[orange_compiler::ControlCell<Id>]) -> Box<[usize]> {
            let (mut local, mut run) = (0, 0);
            cells
                .iter()
                .map(|cell| {
                    let next = if cell.scope() == VarScope::Local {
                        &mut local
                    } else {
                        &mut run
                    };
                    let index = *next;
                    *next += 1;
                    index
                })
                .collect()
        }
        let variables = indices(plan.variables());
        let states = indices(plan.states());
        let mut sources = BTreeMap::new();
        let state_sources = plan
            .states()
            .iter()
            .enumerate()
            .map(|(index, cell)| *sources.entry(cell.id()).or_insert(index))
            .collect();
        let mut output_count = 0;
        let outputs = plan
            .host_sources()
            .iter()
            .map(|source| {
                if matches!(
                    source,
                    HostSource::Variable(_)
                        | HostSource::FrameLocal { .. }
                        | HostSource::Argument { .. }
                        | HostSource::CallLocal(_)
                        | HostSource::State(_)
                        | HostSource::Iteration(_, _)
                ) {
                    None
                } else {
                    let index = output_count;
                    output_count += 1;
                    Some(index)
                }
            })
            .collect();
        Self {
            variables,
            states,
            state_sources,
            outputs,
            output_count,
        }
    }
    /// Nominal bytes one start or On Stop frame reserves at startup: its data
    /// activation (sized by the data module that owns that layout) plus the run's
    /// own local cells, output cells and frame headers.
    fn frame_storage(&self, plan: &ControlPlan, local_cells: usize) -> Option<usize> {
        use std::mem::size_of;
        let header = size_of::<ActivationState>()
            + size_of::<Strand>()
            + size_of::<Region>()
            + size_of::<Option<Box<ActivationState>>>();
        [
            (local_cells, size_of::<Option<Value>>()),
            (self.output_count, size_of::<Option<FoldedValue>>()),
            (1, header),
        ]
        .into_iter()
        .try_fold(
            crate::data::activation_storage(plan.data())?,
            |sum, (count, bytes)| count.checked_mul(bytes)?.checked_add(sum),
        )
    }
    /// The state cell `index`: activation-local cells live in `shared`.
    fn state<'a>(
        &self,
        plan: &ControlPlan,
        run: &'a [Option<Value>],
        shared: &'a ActivationState,
        index: usize,
    ) -> Option<&'a Value> {
        let cell = self.states[index];
        if plan.states()[index].scope() == VarScope::Local {
            shared.states[cell].as_ref()
        } else {
            run[cell].as_ref()
        }
    }
}

#[derive(Default)]
struct PullState {
    active: Option<PendingPull>,
    inputs: Vec<Option<FoldedValue>>,
    completed: [Option<Value>; 2],
    completed_len: usize,
    call_values: Vec<Value>,
}
struct PendingPull {
    root: usize,
    input: Option<usize>,
}
impl PullState {
    fn begin(&mut self, root: usize) {
        self.reset();
        self.active = Some(PendingPull { root, input: None });
    }
    fn reset(&mut self) {
        self.active = None;
        self.inputs.clear();
        self.completed = [None, None];
        self.completed_len = 0;
        self.call_values.clear();
    }
    /// Lend idle scratch capacity to a structured child (body or call) that runs
    /// while this strand waits as its continuation. An active pull stays put.
    fn lend(&mut self) -> Self {
        if self.active.is_some() {
            return Self::default();
        }
        let mut lent = std::mem::take(self);
        lent.reset();
        lent
    }
    /// Take back scratch capacity from a finished child. Buffers hold no state
    /// while idle: every new pull begins with `reset`.
    fn recycle(&mut self, mut child: Self) {
        child.reset();
        self.keep_values(child.call_values);
        if self.active.is_none() && child.inputs.capacity() > self.inputs.capacity() {
            self.inputs = child.inputs;
        }
    }
    fn keep_values(&mut self, mut values: Vec<Value>) {
        if self.active.is_none() && values.capacity() > self.call_values.capacity() {
            values.clear();
            self.call_values = values;
        }
    }
}

/// The one ownership transfer a step requests from the actor. A node
/// completion or cursor resume (including a callee completing the caller's
/// Call) sets it, and `pump` takes it after that strand's step, so the former
/// call/return/fault/fork/unwind/body/wait flags cannot coexist.
#[derive(Default)]
enum Transfer {
    #[default]
    None,
    /// An unhandled fault outside a function retires the activation tree.
    Fault,
    /// Returned values, or an unhandled fault, leave the current function.
    Return(FunctionReturn),
    Call(PendingCall),
    Fork(ControlIx),
    Unwind {
        region: RegionId,
        is_break: bool,
    },
    /// Enter a structured body region; `Some` names its loop owner.
    EnterBody(Option<ControlIx>),
    Wait {
        at: u64,
        owner: ControlIx,
    },
}
struct Strand {
    function: Option<FrameId>,
    transfer: Transfer,
    cleanup_deadline: Option<u64>,
    iteration_context: Option<Arc<IterationFrame>>,
    id: StrandId,
    region: RegionId,
    sequence: u64,
    entry: ControlIx,
    pc: Option<ExecTarget>,
    pull: PullState,
    stack: Vec<Cursor>,
    /// Release the actor segment and rotate behind other ready strands at the
    /// next transfer-free step. Independent of `transfer`: a loop resume sets
    /// both, and the body entry leaves the flag on the waiting continuation.
    yielded: bool,
    reenter: Option<(ControlIx, u64)>,
    cleanup: bool,
    cleanup_remaining: usize,
}
/// One activation tree's evaluator, local cells and execution outputs. `pump`
/// checks it out of `Run::activations` for one strand step and restores it
/// before any error or transfer is handled, so ownership code never sees a gap.
/// `retained_bytes`, `local_bytes` and `owned_bytes` share `max_state_bytes`.
struct ActivationState {
    retained_bytes: usize,
    owned_bytes: Arc<std::sync::atomic::AtomicUsize>,
    cleanup_scope: Option<CleanupScope>,
    cleanup_remaining: usize,
    pull_owner: Option<StrandId>,
    data: DataActivation,
    locals: Vec<Option<Value>>,
    states: Vec<Option<Value>>,
    local_bytes: usize,
    outputs: Vec<Option<FoldedValue>>,
}
struct Timer {
    run: u64,
    owner: ControlIx,
    activation: Strand,
}
enum Cursor {
    ForEach(Box<ForEachCursor>),
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

/// A finished node step: its next target, trace outcome and published fault.
type Step = (Option<ExecTarget>, ControlOutcome, Option<ConstantFault>);
/// A demanded value's graph fault takes the node's Failed edge with its source.
fn failed(node: &ControlNode, fault: ConstantFault) -> Step {
    (node.failed(), ControlOutcome::Failed, Some(fault))
}
/// A fault the node raises itself.
fn failure(node: &ControlNode, fault: KernelFault) -> Step {
    failed(
        node,
        ConstantFault {
            source: node.node(),
            fault,
        },
    )
}
/// Nodes cleanup may run: they neither start work nor publish ordinary results.
fn cleanup_safe(operation: &ControlOperation) -> bool {
    matches!(
        operation,
        ControlOperation::Entry
            | ControlOperation::Branch { .. }
            | ControlOperation::Switch { .. }
            | ControlOperation::MatchResult { .. }
            | ControlOperation::Sequence { .. }
            | ControlOperation::Set { .. }
            | ControlOperation::Gate { .. }
            | ControlOperation::DoOnce { .. }
            | ControlOperation::FlipFlop { .. }
            | ControlOperation::Register { .. }
            | ControlOperation::Update { .. }
    )
}
/// The one data root an operation consumes before it acts. Call, Return and
/// Update consume ordered multi-root demand themselves.
fn demand_root(operation: &ControlOperation, input: ExecInput) -> Option<usize> {
    match operation {
        ControlOperation::Branch {
            condition: root, ..
        }
        | ControlOperation::Switch { key: root, .. }
        | ControlOperation::MatchResult { result: root, .. }
        | ControlOperation::ForEach { items: root, .. }
        | ControlOperation::While {
            condition: root, ..
        }
        | ControlOperation::Count { count: root, .. }
        | ControlOperation::Wait { duration: root }
        | ControlOperation::Set { value: root, .. } => Some(*root),
        ControlOperation::Register { next, .. } if input != ExecInput::Reset => Some(*next),
        _ => None,
    }
}
/// Fork, Entry, Yield and Wait request a strand transfer or simply continue.
fn schedule(
    now: u64,
    node: &ControlNode,
    ix: ControlIx,
    activation: &mut Strand,
    value: Option<Value>,
) -> Result<Step, RunError> {
    Ok(match (node.operation(), value) {
        (ControlOperation::Fork { .. }, None) => {
            activation.transfer = Transfer::Fork(ix);
            (None, ControlOutcome::Forked, None)
        }
        (ControlOperation::Entry, None) => (node.then(), ControlOutcome::Then, None),
        (ControlOperation::Yield, None) => {
            activation.yielded = true;
            (node.then(), ControlOutcome::Yielded, None)
        }
        (ControlOperation::Wait { .. }, Some(Value::Duration(duration))) => {
            match u64::try_from(duration)
                .ok()
                .and_then(|d| now.checked_add(d))
            {
                Some(at) => {
                    activation.transfer = Transfer::Wait { at, owner: ix };
                    (node.then(), ControlOutcome::Suspended, None)
                }
                None => failure(
                    node,
                    KernelFault {
                        code: "core/OutOfRange",
                        message_key: "core/wait-out-of-range",
                        message: "Wait duration is outside the clock range.",
                    },
                ),
            }
        }
        (ControlOperation::Wait { .. }, Some(_)) => {
            return Err(DataError::KernelContract { node: node.node() }.into());
        }
        _ => return Err(RunError::InvalidState),
    })
}

/// Checked control execution. Inspection is explicit; Debug never formats values.
///
/// ```compile_fail
/// let run = orange_runtime::Run::default();
/// ```
pub struct Run {
    frames: BTreeMap<FrameId, FunctionFrame>,
    spare_frames: Vec<FrameStorage>,
    id: u64,
    plan: Option<Arc<ControlPlan>>,
    options: RunOptions,
    state: RunState,
    ready: ReadyQueue<Strand>,
    activations: Vec<Option<Box<ActivationState>>>,
    regions: BTreeMap<RegionId, Region>,
    owner_ids: OwnerIds,
    live_strands: usize,
    trace: VecDeque<RunEvent>,
    trace_dropped: u64,
    operations: usize,
    variables: Vec<Option<Value>>,
    states: Vec<Option<Value>>,
    layout: StateLayout,
    state_bytes: usize,
    versions: Vec<u64>,
    now: u64,
    timers: Timers,
    transitions: Vec<RunState>,
    deadline: Option<u64>,
    last_error: Option<RunError>,
    cancel_observed: bool,
    cleanup_truncated: u64,
    stop_ready: VecDeque<Strand>,
    segment_owner: Option<StrandId>,
    last_segment: Option<u64>,
    segment: SegmentWrites,
    /// Reused cancellation traversal buffers; empty between calls.
    cancel_regions: Vec<RegionId>,
    cancel_timers: Vec<TimerKey>,
    /// Set while `pump` runs. Still set on the next call only if a trusted callback
    /// panicked through a pump, which can leave work taken out of the run's stores.
    pump_open: bool,
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
        if capacity > options.limits.max_strands || capacity > options.limits.max_regions {
            return Err(RunError::ResourceLimit);
        }
        let mut regions = BTreeMap::new();
        let mut ready = ReadyQueue::with_capacity(capacity);
        let mut activations = Vec::with_capacity(capacity);
        let mut stop_ready = VecDeque::with_capacity(plan.stops().len());
        let layout = StateLayout::new(&plan);
        let mut state_bytes = 0usize;
        let mut local_bytes = 0usize;
        struct InitialCells {
            run: Vec<Option<Value>>,
            local: Vec<Option<Value>>,
        }
        fn initialize<Id: Copy>(
            cells: &[orange_compiler::ControlCell<Id>],
            run_bytes: &mut usize,
            local_bytes: &mut usize,
            limit: usize,
        ) -> Result<InitialCells, RunError> {
            let (mut run, mut local) = (Vec::new(), Vec::new());
            for cell in cells {
                let bytes = cell.initial().and_then(|v| cell.charge(v)).unwrap_or(16);
                let (store, total) = if cell.scope() == VarScope::Local {
                    (&mut local, &mut *local_bytes)
                } else {
                    (&mut run, &mut *run_bytes)
                };
                *total = total
                    .checked_add(bytes)
                    .filter(|n| *n <= limit)
                    .ok_or(RunError::ResourceLimit)?;
                store.push(cell.initial().cloned());
            }
            Ok(InitialCells { run, local })
        }
        let InitialCells {
            run: variables,
            local: locals,
        } = initialize(
            plan.variables(),
            &mut state_bytes,
            &mut local_bytes,
            options.limits.max_state_bytes,
        )?;
        let InitialCells {
            run: states,
            local: local_states,
        } = initialize(
            plan.states(),
            &mut state_bytes,
            &mut local_bytes,
            options.limits.max_state_bytes,
        )?;
        // Every start and On Stop frame reserves its storage now, so admit the
        // aggregate before constructing any of them.
        layout
            .frame_storage(&plan, locals.len() + local_states.len())
            .and_then(|bytes| bytes.checked_mul(capacity))
            .filter(|total| *total <= options.limits.max_frame_storage_bytes)
            .ok_or(RunError::ResourceLimit)?;
        let prepared = if capacity > 0 {
            Some(PreparedData::new(plan.data().clone(), options.data.limits)?)
        } else {
            None
        };
        for (sequence, entry) in plan
            .starts()
            .iter()
            .chain(plan.stops())
            .copied()
            .enumerate()
        {
            activations.push(Some(Box::new(ActivationState {
                retained_bytes: 0,
                owned_bytes: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                cleanup_scope: None,
                cleanup_remaining: options.max_cleanup_operations,
                pull_owner: None,
                data: DataActivation::from_prepared(
                    prepared
                        .as_ref()
                        .expect("entry capacity was admitted")
                        .clone(),
                    document.revision(),
                    seed,
                    sequence as u64,
                    options.data.clone(),
                )?,
                locals: locals.clone(),
                states: local_states.clone(),
                local_bytes,
                outputs: vec![None; layout.output_count],
            })));
            regions.insert(
                RegionId::root(sequence as u64),
                Region {
                    frames: None,
                    retired_cursor: None,
                    parent: None,
                    activation: sequence as u64,
                    depth: 0,
                    live: IdSet::one(StrandId::root(sequence as u64)),
                    children: IdSet::default(),
                    continuation: None,
                    kind: RegionKind::Root,
                },
            );
            let strand = Strand {
                function: None,
                transfer: Transfer::None,
                cleanup_deadline: None,
                iteration_context: None,
                id: StrandId::root(sequence as u64),
                region: RegionId::root(sequence as u64),
                sequence: sequence as u64,
                entry,
                pc: Some(entry.into()),
                pull: PullState::default(),
                stack: vec![],
                yielded: false,
                reenter: None,
                cleanup: false,
                cleanup_remaining: 0,
            };
            if sequence < plan.starts().len() {
                ready.push_back(strand.id, strand);
            } else {
                stop_ready.push_back(strand);
            }
        }
        let variable_count = plan.variables().len();
        let state_count = plan.states().len();
        Ok(Self {
            frames: BTreeMap::new(),
            spare_frames: Vec::new(),
            id: run_identity()?,
            plan: Some(plan),
            options,
            state: RunState::Running,
            ready,
            activations,
            regions,
            owner_ids: OwnerIds::after_roots(capacity as u64),
            live_strands: capacity,
            trace: VecDeque::new(),
            trace_dropped: 0,
            operations: 0,
            versions: vec![0; variable_count],
            layout,
            variables,
            states,
            state_bytes,
            now: 0,
            timers: Timers::default(),
            transitions: vec![RunState::Created, RunState::Starting, RunState::Running],
            deadline: None,
            last_error: None,
            cancel_observed: false,
            cleanup_truncated: 0,
            stop_ready,
            segment_owner: None,
            last_segment: None,
            segment: SegmentWrites::new(variable_count, state_count),
            cancel_regions: vec![],
            cancel_timers: vec![],
            pump_open: false,
        })
    }
    pub fn id(&self) -> u64 {
        self.id
    }
    /// `Faulted` after a pump that unwound, until the host stops the run.
    pub fn state(&self) -> RunState {
        if self.pump_open {
            RunState::Faulted
        } else {
            self.state
        }
    }
    pub fn transitions(&self) -> &[RunState] {
        &self.transitions
    }
    pub fn last_error(&self) -> Option<RunError> {
        if self.pump_open {
            Some(RunError::Interrupted)
        } else {
            self.last_error
        }
    }
    pub fn cleanup_truncated(&self) -> u64 {
        self.cleanup_truncated
    }
    pub fn pending_continuations(&self) -> usize {
        self.timers.len()
    }
    pub fn ownership(&self) -> RunOwnership {
        RunOwnership {
            activations: self.activations.iter().flatten().count(),
            strands: self.live_strands,
            regions: self.regions.len(),
            timers: self.timers.len(),
        }
    }
    /// IDs written by the last/current segment, once each. No equality comparison,
    /// change notification or Watch delivery is implied by this inspection seam.
    pub fn segment_writes(&self) -> Option<(u64, &[VariableId])> {
        self.last_segment
            .map(|owner| (owner, self.segment.variables()))
    }
    fn transition(&mut self, state: RunState) {
        if self.state != state {
            self.state = state;
            self.transitions.push(state);
        }
    }
    /// State cells written by the last actor segment; these are write intents,
    /// not Eq-based change notifications. Activation identifies local cell owners.
    pub fn segment_state_writes(&self) -> Option<(u64, &[NodeId])> {
        self.last_segment
            .map(|owner| (owner, self.segment.states()))
    }
    pub fn now(&self) -> u64 {
        self.now
    }
    /// Advancing time enqueues no inline graph execution; pump resumes due waits.
    pub fn advance_to(&mut self, now: u64) -> Result<(), RunError> {
        if self.settle_interruption() {
            return Err(RunError::Interrupted);
        }
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
    /// Unambiguous run-owned source state; repeated call sites require `control_state`.
    pub fn node_state(&self, id: NodeId) -> Option<&Value> {
        let plan = self.plan.as_ref()?;
        let mut cells = plan
            .states()
            .iter()
            .enumerate()
            .filter(|(_, cell)| cell.id() == id);
        let (index, _) = cells.next()?;
        if cells.next().is_some() {
            return None;
        }
        if plan.states()[index].scope() == VarScope::Local {
            return None;
        }
        self.states[self.layout.states[index]].as_ref()
    }
    /// Run-owned state at an exact control index in this run's pinned plan.
    pub fn control_state(&self, owner: ControlIx) -> Option<&Value> {
        let plan = self.plan.as_ref()?;
        let index = match plan.nodes().get(owner.index())?.operation() {
            ControlOperation::Gate { state }
            | ControlOperation::DoOnce { state }
            | ControlOperation::FlipFlop { state, .. }
            | ControlOperation::Register { state, .. } => *state,
            _ => return None,
        };
        if plan.states()[index].scope() == VarScope::Local {
            return None;
        }
        self.states[self.layout.states[index]].as_ref()
    }
    /// Explicit run-state inspection; activation-local cells are not shared.
    pub fn variable(&self, id: VariableId) -> Option<&Value> {
        let index = self
            .plan
            .as_ref()?
            .variables()
            .iter()
            .position(|v| v.id() == id && v.scope() == VarScope::Run)?;
        self.variables[self.layout.variables[index]].as_ref()
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
    /// Runs up to `budget` work items. If a trusted provider callback panics, the panic
    /// propagates and the run is left faulted: every later call reports
    /// `RunError::Interrupted` until the host stops it, which cancels its remaining work.
    pub fn pump(&mut self, budget: usize) -> Result<RunProgress, RunError> {
        if self.settle_interruption() {
            return Err(RunError::Interrupted);
        }
        self.pump_open = true;
        let progress = self.pump_open_run(budget);
        self.pump_open = false;
        progress
    }

    /// Records a pump that unwound: the run faults with `Interrupted` and refuses ordinary
    /// work, so lost activations cannot pass for completed ones. True when one did.
    fn settle_interruption(&mut self) -> bool {
        if !self.pump_open {
            return false;
        }
        self.pump_open = false;
        self.last_error = Some(RunError::Interrupted);
        self.transition(RunState::Faulted);
        true
    }

    fn pump_open_run(&mut self, budget: usize) -> Result<RunProgress, RunError> {
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
        let plan = self.plan.as_ref().ok_or(RunError::InvalidState)?.clone();
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
            if let Some(timer) = self.timers.pop_due(self.now) {
                remaining -= 1;
                if timer.run != self.id {
                    return self.fail(RunError::InvalidState);
                }
                self.record(RunEvent {
                    activation: timer.activation.sequence,
                    node: self.plan.as_ref().unwrap().nodes()[timer.owner.index()].node(),
                    outcome: ControlOutcome::Then,
                    fault: None,
                });
                self.ready.push_back(timer.activation.id, timer.activation);
                continue;
            }
            // One pending consumption owns the shared activation evaluator.
            // Other activations can interleave; sibling strands cannot open a
            // replacement epoch or reset activation entropy/fuel under it.
            let pull_owner = self.ready.front().and_then(|front| {
                self.activations
                    .get(front.sequence as usize)
                    .and_then(Option::as_ref)
                    .and_then(|a| a.pull_owner)
                    .filter(|owner| *owner != front.id)
            });
            let next = if let Some(owner) = pull_owner {
                Some(self.ready.remove(owner).ok_or(RunError::InvalidState)?)
            } else {
                self.ready.pop_front()
            };
            let Some(mut activation) = next else {
                break;
            };
            last = Some(activation.id);
            let mut quantum = if activation.cleanup {
                remaining.min(activation.cleanup_remaining).min(
                    self.activations[activation.sequence as usize]
                        .as_ref()
                        .unwrap()
                        .cleanup_remaining,
                )
            } else {
                remaining
            };
            if quantum == 0
                || (activation.cleanup
                    && activation
                        .cleanup_deadline
                        .is_some_and(|deadline| self.now >= deadline))
            {
                self.cleanup_truncated = self.cleanup_truncated.saturating_add(1);
                remaining -= 1;
                let shared = self
                    .activations
                    .get_mut(activation.sequence as usize)
                    .and_then(Option::as_mut)
                    .ok_or(RunError::InvalidState)?;
                if let Some(scope) = shared.cleanup_scope.take() {
                    shared.data.finish_cleanup(scope);
                }
                shared.pull_owner = None;
                self.retire_strand(activation)?;
                continue;
            }
            let before = quantum;
            if self.segment_owner != Some(activation.id) {
                self.segment.open();
                self.segment_owner = Some(activation.id);
                self.last_segment = Some(activation.sequence);
            }
            let mut shared = self
                .activations
                .get_mut(activation.sequence as usize)
                .and_then(Option::take)
                .ok_or(RunError::InvalidState)?;
            // Entry failure must follow the same restoration/retirement path as execution
            // failure: the activation has already been checked out of its owner store.
            let result = (|| {
                if activation.cleanup && shared.cleanup_scope.is_none() {
                    shared.cleanup_scope =
                        Some(shared.data.scoped_cleanup(shared.cleanup_remaining)?);
                }
                self.execute(&plan, &mut activation, &mut shared, &mut quantum)
            })();
            let used = before - quantum;
            remaining -= used;
            if activation.cleanup {
                activation.cleanup_remaining -= used;
                shared.cleanup_remaining -= used;
            }
            if activation.cleanup && (activation.pull.active.is_none() || result.is_err()) {
                if let Some(scope) = shared.cleanup_scope.take() {
                    shared.data.finish_cleanup(scope);
                }
                shared.pull_owner = None;
            }
            self.activations[activation.sequence as usize] = Some(shared);
            if let Err(error) = result {
                if activation.cleanup {
                    self.cleanup_truncated = self.cleanup_truncated.saturating_add(1);
                    self.retire_strand(activation)?;
                    self.finish_if_idle();
                    continue;
                }
                self.operations = self.operations.saturating_add(initial - remaining);
                return self.fail(error);
            }
            if self.state == RunState::Faulted {
                self.cancel_owned()?;
                break;
            }
            match std::mem::take(&mut activation.transfer) {
                Transfer::Return(result) => {
                    if let Err(error) = self.leave_function(activation, result) {
                        return self.fail(error);
                    }
                }
                Transfer::Call(call) => {
                    if let Err(error) = self.enter_function(activation, call) {
                        return self.fail(error);
                    }
                }
                // A function fault always carries its source to the caller.
                Transfer::Fault if activation.function.is_some() => {
                    return Err(RunError::InvalidState);
                }
                Transfer::Fault => self.retire_failed(activation)?,
                Transfer::Fork(owner) => self.spawn_fork(activation, owner)?,
                Transfer::Unwind { region, is_break } => {
                    self.unwind_loop(activation, region, is_break)?;
                }
                Transfer::EnterBody(loop_owner) => self.enter_body(activation, loop_owner)?,
                Transfer::Wait { at, owner } => {
                    self.segment_owner = None;
                    let timer = Timer {
                        run: self.id,
                        owner,
                        activation,
                    };
                    let max = self.options.limits.max_timers;
                    if self.timers.schedule(at, timer, max).is_err() {
                        return self.fail(RunError::ResourceLimit);
                    }
                }
                Transfer::None if activation.pc.is_some() || !activation.stack.is_empty() => {
                    if activation.yielded {
                        self.segment_owner = None;
                        activation.yielded = false;
                        self.ready.push_back(activation.id, activation);
                    } else {
                        self.ready.push_front(activation.id, activation);
                    }
                }
                Transfer::None => self.retire_strand(activation)?,
            }
        }
        if self.state == RunState::Faulted {
            self.cancel_owned()?;
        }
        // A pending mutation owns its read/compute/write segment across host slices.
        // Explicit suspension still releases ownership; read-only pulls may rotate.
        if remaining == 0
            && self.ready.front().is_some_and(|a| {
                Some(a.id) == last
                    && !(a.function.is_some()
                        || (a.pull.active.is_some()
                            && a.pc.is_some_and(|pc| {
                                matches!(
                                    self.plan.as_ref().unwrap().nodes()[pc.index()].operation(),
                                    ControlOperation::Set { .. }
                                        | ControlOperation::Call { .. }
                                        | ControlOperation::Return { .. }
                                        | ControlOperation::Register { .. }
                                        | ControlOperation::Update { .. }
                                )
                            })))
            })
            && let Some(activation) = self.ready.pop_front()
        {
            self.ready.push_back(activation.id, activation);
        }
        // A host budget boundary does not itself suspend the logical strand.
        // Explicit suspension or selecting a different strand opens a new segment.
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
        let timer = self.timers.next_at();
        let next_wakeup = match (timer, self.deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        Ok(RunProgress {
            operations: count,
            ready: !self.ready.is_empty() || self.timers.is_due(self.now),
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
        self.regions.clear();
        self.frames.clear();
        self.activations.clear();
        self.live_strands = 0;
        self.deadline = None;
        self.transition(RunState::Stopped);
    }
    /// One node step. Single-root demand is consumed here, once: pending keeps
    /// the pull for the next quantum, and a graph fault takes the node's Failed
    /// edge. Family handlers then act on the value with only the state they own.
    fn execute(
        &mut self,
        plan: &ControlPlan,
        activation: &mut Strand,
        shared: &mut ActivationState,
        budget: &mut usize,
    ) -> Result<(), RunError> {
        let Some(target) = activation.pc else {
            *budget -= 1;
            return self.resume_cursor(plan, activation, shared);
        };
        let ix = target.node();
        let node = &plan.nodes()[ix.index()];
        if activation.pull.active.is_none() {
            *budget -= 1;
        }
        let operation = node.operation();
        if activation.cleanup && !cleanup_safe(operation) {
            self.refuse_in_cleanup(plan, activation, shared, ix);
            return Ok(());
        }
        let value = match demand_root(operation, target.input()) {
            None => None,
            Some(root) => match self.consume(plan, activation, shared, root, budget)? {
                None => return Ok(()),
                Some(FoldedValue::Value(value)) => Some(value),
                Some(FoldedValue::Fault(fault)) => {
                    if let ControlOperation::MatchResult { .. } = operation {
                        // A failed match clears its value outputs.
                        Self::publish_match_value(
                            plan,
                            &self.layout,
                            shared,
                            ix,
                            None,
                            self.options.limits.max_state_bytes,
                        )?;
                    }
                    self.complete(plan, activation, shared, ix, failed(node, fault));
                    return Ok(());
                }
            },
        };
        let step = match operation {
            ControlOperation::Call { .. } | ControlOperation::Return { .. } => {
                return self.execute_function(plan, activation, shared, ix, budget);
            }
            ControlOperation::Update { .. } => {
                match self.execute_update(plan, activation, shared, ix, budget)? {
                    Some(step) => step,
                    None => return Ok(()),
                }
            }
            ControlOperation::Gate { .. }
            | ControlOperation::DoOnce { .. }
            | ControlOperation::FlipFlop { .. }
            | ControlOperation::Register { .. }
            | ControlOperation::Set { .. } => {
                let function = activation.function;
                self.cells().step(plan, target, shared, function, value)?
            }
            ControlOperation::Branch { .. }
            | ControlOperation::Switch { .. }
            | ControlOperation::MatchResult { .. } => Self::select(
                plan,
                (&self.layout, self.options.limits.max_state_bytes),
                ix,
                shared,
                value,
            )?,
            ControlOperation::ForEach { .. }
            | ControlOperation::While { .. }
            | ControlOperation::Count { .. }
            | ControlOperation::Sequence { .. }
            | ControlOperation::Break
            | ControlOperation::Continue => self.loop_step(plan, ix, activation, shared, value)?,
            ControlOperation::Fork { .. }
            | ControlOperation::Entry
            | ControlOperation::Yield
            | ControlOperation::Wait { .. } => schedule(self.now, node, ix, activation, value)?,
        };
        self.complete(plan, activation, shared, ix, step);
        Ok(())
    }
    /// Cleanup never publishes ordinary results or revives a held caller: Return
    /// is cancelled, and any other unsafe node fails at itself.
    fn refuse_in_cleanup(
        &mut self,
        plan: &ControlPlan,
        activation: &mut Strand,
        shared: &mut ActivationState,
        ix: ControlIx,
    ) {
        let node = &plan.nodes()[ix.index()];
        if matches!(node.operation(), ControlOperation::Return { .. }) {
            activation.pc = None;
            activation.stack.clear();
            activation.pull.reset();
            shared.pull_owner = None;
            self.record(RunEvent {
                activation: activation.sequence,
                node: node.node(),
                outcome: ControlOutcome::Cancelled,
                fault: None,
            });
            return;
        }
        let fault = ConstantFault {
            source: node.node(),
            fault: KernelFault {
                code: "core/Cancelled",
                message_key: "core/cleanup-unsafe-node",
                message: "This operation is not permitted during cancellation cleanup.",
            },
        };
        Self::publish_error(plan, &self.layout, shared, ix, &fault);
        self.record(RunEvent {
            activation: activation.sequence,
            node: node.node(),
            outcome: ControlOutcome::Failed,
            fault: Some(fault),
        });
        activation.pc = node.failed();
        activation.pull.reset();
        if activation.pc.is_none() {
            activation.stack.clear();
        }
    }
    fn complete(
        &mut self,
        plan: &ControlPlan,
        activation: &mut Strand,
        shared: &mut ActivationState,
        ix: ControlIx,
        (next, outcome, fault): (Option<ExecTarget>, ControlOutcome, Option<ConstantFault>),
    ) {
        let node = &plan.nodes()[ix.index()];
        shared.pull_owner = None;
        activation.pull.reset();
        activation.reenter = None;
        if let Some(fault) = &fault {
            Self::publish_error(plan, &self.layout, shared, ix, fault);
        }
        if outcome == ControlOutcome::Failed && next.is_none() {
            activation.stack.clear();
            // Cleanup faults only end their own strand.
            if !activation.cleanup {
                activation.transfer = match (activation.function, &fault) {
                    (Some(_), Some(fault)) => {
                        Transfer::Return(FunctionReturn::Fault(fault.clone()))
                    }
                    _ => Transfer::Fault,
                };
            }
        }
        self.record(RunEvent {
            activation: activation.sequence,
            node: node.node(),
            outcome,
            fault,
        });
        if outcome == ControlOutcome::Failed
            && next.is_none()
            && !activation.cleanup
            && activation.function.is_none()
            && plan.nodes()[activation.entry.index()].fault_policy() == FaultPolicy::StopRun
        {
            self.transition(RunState::Faulted);
        }
        activation.pc = next;
    }
    fn consume(
        &self,
        plan: &ControlPlan,
        activation: &mut Strand,
        shared: &mut ActivationState,
        root: usize,
        budget: &mut usize,
    ) -> Result<Option<FoldedValue>, RunError> {
        let inputs = plan.inputs_for(root).ok_or(RunError::InvalidState)?;
        self.consume_inputs(plan, activation, shared, root, inputs, budget)
    }
    fn consume_inputs(
        &self,
        plan: &ControlPlan,
        activation: &mut Strand,
        shared: &mut ActivationState,
        root: usize,
        inputs: &[usize],
        budget: &mut usize,
    ) -> Result<Option<FoldedValue>, RunError> {
        // Snapshot: a pull's first poll opens one data epoch and copies every host
        // input it may read; later polls resume that epoch, so cell writes by other
        // strands meanwhile cannot change what this demand observes.
        if activation.pull.active.is_none() {
            shared.data.begin_epoch()?;
            shared.pull_owner = Some(activation.id);
            activation.pull.begin(root);
            for &i in inputs {
                let source = &plan.host_sources()[i];
                let value = match source {
                    HostSource::Iteration(owner, projection) => {
                        let frame =
                            IterationFrame::find(activation.iteration_context.as_deref(), *owner)
                                .ok_or(DataError::MissingInput {
                                node: plan.nodes()[owner.index()].node(),
                            })?;
                        Some(FoldedValue::Value(
                            frame.value(*projection).ok_or(RunError::InvalidState)?,
                        ))
                    }
                    HostSource::FrameLocal { scope, variable } => {
                        let frame = find_frame(&self.frames, activation.function, *scope)?;
                        let value = frame_value(&self.frames, plan, frame, *variable)?.cloned();
                        Some(
                            value
                                .map(FoldedValue::Value)
                                .unwrap_or_else(|| FunctionFrame::uninitialized(plan, i)),
                        )
                    }
                    HostSource::Argument { scope, index } => {
                        let frame = find_frame(&self.frames, activation.function, *scope)?;
                        Some(FoldedValue::Value(
                            self.frames[&frame]
                                .arguments
                                .get(*index)
                                .ok_or(RunError::InvalidState)?
                                .clone(),
                        ))
                    }
                    HostSource::State(index) => Some(
                        match self.layout.state(plan, &self.states, shared, *index) {
                            Some(value) => FoldedValue::Value(value.clone()),
                            None => FoldedValue::Fault(ConstantFault {
                                source: plan.states()[*index].id(),
                                fault: KernelFault {
                                    code: "core/UninitializedRegister",
                                    message_key: "core/uninitialized-register",
                                    message: "The register has not been initialized.",
                                },
                            }),
                        },
                    ),
                    HostSource::Variable(index) | HostSource::CallLocal(index) => {
                        let var = &plan.variables()[*index];
                        let cell = if matches!(source, HostSource::CallLocal(_)) {
                            // Volatile bodies cannot mutate locals; immutable initial values
                            // implement fresh call-local reads without mutable frame allocation.
                            var.initial()
                        } else if var.scope() == VarScope::Local {
                            shared.locals[self.layout.variables[*index]].as_ref()
                        } else {
                            self.variables[self.layout.variables[*index]].as_ref()
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
                    HostSource::Error(_)
                    | HostSource::Index(_)
                    | HostSource::Value(_)
                    | HostSource::Update(_, _)
                    | HostSource::CallOutput(_, _) => {
                        self.layout.outputs[i].and_then(|slot| shared.outputs[slot].clone())
                    }
                };
                activation.pull.inputs.push(value);
            }
        }
        if activation
            .pull
            .active
            .as_ref()
            .is_none_or(|pull| pull.root != root)
        {
            return Err(RunError::InvalidState);
        }
        loop {
            if let Some(index) = activation.pull.active.as_ref().and_then(|pull| pull.input) {
                if *budget == 0 {
                    return Ok(None);
                }
                *budget -= 1;
                let offset = inputs
                    .binary_search(&index)
                    .map_err(|_| RunError::InvalidState)?;
                let value = activation.pull.inputs[offset].clone().ok_or_else(|| {
                    RunError::Data(DataError::MissingInput {
                        node: plan.data().nodes()[plan.data().host_inputs()[index].index()].node(),
                    })
                })?;
                shared.data.set_input_outcome(index, value)?;
                activation.pull.active.as_mut().expect("active pull").input = None;
            }
            match shared.data.poll_host_export(root, budget)? {
                HostPoll::Pending => return Ok(None),
                HostPoll::Input(index) => {
                    activation.pull.active.as_mut().expect("active pull").input = Some(index)
                }
                HostPoll::Ready(Some(value)) => {
                    return Ok(Some(match value.outcome {
                        Ok(value) => FoldedValue::Value(value.clone()),
                        Err(fault) => FoldedValue::Fault(fault.clone()),
                    }));
                }
                HostPoll::Ready(None) => return Err(RunError::InvalidState),
            }
        }
    }
    fn publish_match_value(
        plan: &ControlPlan,
        layout: &StateLayout,
        shared: &mut ActivationState,
        owner: ControlIx,
        value: Option<Value>,
        limit: usize,
    ) -> Result<(), RunError> {
        let mut old = 0usize;
        let mut new = 0usize;
        for &i in plan.value_targets(owner) {
            let slot = plan.data().nodes()[plan.data().host_inputs()[i].index()].outputs()[0];
            let cell = layout.outputs[i].expect("checked execution output");
            let charge = |value: &Value| {
                plan.data()
                    .value_charge(slot, value)
                    .ok_or(DataError::KernelContract {
                        node: plan.nodes()[owner.index()].node(),
                    })
            };
            if let Some(FoldedValue::Value(value)) = &shared.outputs[cell] {
                old = old
                    .checked_add(charge(value)?)
                    .ok_or(RunError::ResourceLimit)?;
            }
            if let Some(value) = &value {
                new = new
                    .checked_add(charge(value)?)
                    .ok_or(RunError::ResourceLimit)?;
            }
        }
        let retained = shared
            .retained_bytes
            .checked_sub(old)
            .and_then(|n| n.checked_add(new))
            .filter(|n| {
                n.checked_add(shared.local_bytes)
                    .and_then(|n| n.checked_add(shared.owned_bytes.load(Ordering::Relaxed)))
                    .is_some_and(|total| total <= limit)
            })
            .ok_or(RunError::ResourceLimit)?;
        shared.retained_bytes = retained;
        for &i in plan.value_targets(owner) {
            shared.outputs[layout.outputs[i].expect("checked execution output")] =
                value.clone().map(FoldedValue::Value);
        }
        Ok(())
    }
    fn publish_index(
        plan: &ControlPlan,
        layout: &StateLayout,
        shared: &mut ActivationState,
        owner: ControlIx,
        index: Option<i64>,
    ) {
        for &i in plan.index_targets(owner) {
            shared.outputs[layout.outputs[i].expect("checked execution output")] =
                index.map(|i| FoldedValue::Value(Value::I64(i)));
        }
    }
    fn publish_error(
        plan: &ControlPlan,
        layout: &StateLayout,
        shared: &mut ActivationState,
        owner: ControlIx,
        fault: &ConstantFault,
    ) {
        for &i in plan.error_targets(owner) {
            shared.outputs[layout.outputs[i].expect("checked execution output")] = Some(
                FoldedValue::Value(Value::Err(Arc::new(orange_model::value::NodeError {
                    code: fault.fault.code.into(),
                    node: Some(fault.source),
                }))),
            );
        }
    }
    pub fn stop(&mut self) -> Result<(), RunError> {
        self.request_stop(StopMode::Immediate)
    }
    /// Graceful stop starts On Stop once and drains accepted work. Immediate stop
    /// cancels owned waits; a second stop aborts bounded cancellation cleanup.
    pub fn request_stop(&mut self, mode: StopMode) -> Result<(), RunError> {
        self.settle_interruption();
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
                while let Some(strand) = self.stop_ready.pop_front() {
                    self.ready.push_back(strand.id, strand);
                }
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
        let roots: Vec<_> = self
            .regions
            .iter()
            .filter(|(_, r)| r.parent.is_none())
            .map(|(id, _)| *id)
            .collect();
        for root in roots {
            self.cancel_region(root)?;
            self.complete_regions(root)?;
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
        // Disposal releases the run's storage, pooled frame buffers and reusable traversal
        // buffers included. Replacing each container frees its capacity, which clearing
        // would keep. Only the terminal summary stays: identity, options, state history,
        // last error, counters and the virtual clock.
        self.plan = None;
        self.frames = BTreeMap::new();
        self.spare_frames = Vec::new();
        self.ready = ReadyQueue::with_capacity(0);
        self.activations = Vec::new();
        self.regions = BTreeMap::new();
        self.variables = Vec::new();
        self.states = Vec::new();
        self.layout = StateLayout::default();
        self.versions = Vec::new();
        self.state_bytes = 0;
        self.timers.release();
        self.stop_ready = VecDeque::new();
        self.segment = SegmentWrites::default();
        self.segment_owner = None;
        self.last_segment = None;
        self.trace = VecDeque::new();
        self.cancel_regions = Vec::new();
        self.cancel_timers = Vec::new();
        self.transition(RunState::Disposed);
        Ok(())
    }
}
