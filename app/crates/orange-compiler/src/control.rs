//! Checked root control plans. The model owns language validation; this compiler
//! additionally admits only the executable vocabulary implemented by the runtime.
use crate::compile::{admit_roots, charge, decode, key as port_key, port_type, value_bytes};
use crate::data_compile::{DemandRoot, lower_roots};
use crate::*;
use orange_model::GraphDocument;
use orange_model::Value;
use orange_model::catalog::ReferenceKind;
use orange_model::catalog::RegistrySnapshot;
use orange_model::catalog::TimerSchedule;
use orange_model::catalog::intrinsic_id as ids;
use orange_model::document::{
    EventScope, FaultPolicy, NodeKind, OverflowPolicy, PortDirection, PortKind, ReentrancyMode,
    VarScope,
};
use orange_model::edit::DeclId;
use orange_model::id::{EventDeclId, NodeId, VariableId};
use orange_model::validation::ValidationAnalysis;
use orange_model::value::shape::VALUE_CHARGE;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

mod dependencies;
mod dispatch;
mod update;
use dependencies::{UnionScratch, host_dependencies, union_inputs};
#[derive(Debug, Clone, Copy)]
pub enum IterationProjection {
    Index,
    Element,
    Key,
    Value,
}
/// Which value of a delivered timer tick a `core/on-timer`/`core/on-timeout` output reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickProjection {
    /// `scheduled_at: core/duration@1`, virtual nanoseconds since the run clock's zero.
    ScheduledAt,
    /// `missed: core/u64@1`, ticks not delivered since the previous delivered tick.
    Missed,
}
/// Which value of a delivered variable change a `core/on-variable-changed` output reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeProjection {
    /// `value`: the variable's value at the end of the writing segment.
    Value,
    /// `previous`: its value when that segment started.
    Previous,
}
pub use dispatch::{SwitchCase, SwitchKey};
pub use update::{UpdateDemand, UpdateOperation, UpdateProjection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlIx(pub(crate) usize);
impl ControlIx {
    pub fn index(self) -> usize {
        self.0
    }
}

/// Compiler-resolved destination command. These are language tags, not saved IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecInput {
    In,
    Enter,
    Open,
    Close,
    Toggle,
    Reset,
    Store,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecTarget {
    node: ControlIx,
    input: ExecInput,
}
impl ExecTarget {
    pub fn node(self) -> ControlIx {
        self.node
    }
    pub fn input(self) -> ExecInput {
        self.input
    }
    pub fn index(self) -> usize {
        self.node.index()
    }
}
impl From<ControlIx> for ExecTarget {
    fn from(node: ControlIx) -> Self {
        Self {
            node,
            input: ExecInput::In,
        }
    }
}
fn destination(
    node: ControlIx,
    definition: &str,
    port: &orange_model::id::PortKey,
) -> Option<ExecTarget> {
    if port.instance().is_some() {
        return None;
    }
    let input = match (definition, port.port().as_str()) {
        (ids::GATE, "in" | "enter") => ExecInput::Enter,
        (ids::GATE, "open") => ExecInput::Open,
        (ids::GATE, "close") => ExecInput::Close,
        (ids::GATE, "toggle") => ExecInput::Toggle,
        (ids::DO_ONCE | ids::REGISTER, "reset") => ExecInput::Reset,
        (ids::REGISTER, "in" | "store") => ExecInput::Store,
        (_, "in") => ExecInput::In,
        _ => return None,
    };
    Some(ExecTarget { node, input })
}

#[derive(Debug, Clone)]
pub enum ControlOperation {
    Call {
        scope: usize,
        demand: UpdateDemand,
    },
    Return {
        demand: UpdateDemand,
    },
    ForEach {
        items: usize,
        body: Option<ExecTarget>,
        completed: Option<ExecTarget>,
    },
    Update {
        variable: usize,
        operation: UpdateOperation,
        demand: UpdateDemand,
    },
    Gate {
        state: usize,
    },
    DoOnce {
        state: usize,
    },
    FlipFlop {
        state: usize,
        a: Option<ExecTarget>,
        b: Option<ExecTarget>,
    },
    Register {
        state: usize,
        next: usize,
    },
    Switch {
        key: usize,
        cases: Box<[SwitchCase]>,
        default: Option<ExecTarget>,
    },
    /// Enqueues a run-scope event after evaluating its payload demand root.
    Emit {
        event: usize,
        payload: usize,
    },
    /// Suspends this strand until its next matching run-scoped emission, or its
    /// optional timeout; it never creates a new activation.
    AwaitEvent {
        event: usize,
        timeout: Option<usize>,
        timed_out: Option<ExecTarget>,
    },
    MatchResult {
        result: usize,
        boxed: bool,
        ok: Option<ExecTarget>,
        err: Option<ExecTarget>,
    },
    Entry,
    Fork {
        branches: Box<[Option<ExecTarget>]>,
        completed: Option<ExecTarget>,
        join: ForkJoin,
    },
    While {
        condition: usize,
        body: Option<ExecTarget>,
        completed: Option<ExecTarget>,
        max_iterations: u64,
    },
    Count {
        count: usize,
        body: Option<ExecTarget>,
        completed: Option<ExecTarget>,
    },
    Break,
    Continue,
    Yield,
    Wait {
        duration: usize,
    },
    Sequence {
        steps: Box<[Option<ExecTarget>]>,
    },
    Set {
        variable: usize,
        value: usize,
    },
    Branch {
        condition: usize,
        when_true: Option<ExecTarget>,
        when_false: Option<ExecTarget>,
    },
}

/// Language-owned structured joins; packages cannot add scheduler policies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkJoin {
    All,
    Any,
    Race,
    Detach,
}

pub type ControlVariable = ControlCell<VariableId>;
pub type ControlState = ControlCell<NodeId>;
/// Checked typed cell metadata shared by declaration variables and node state.
/// Id keeps those two identity spaces distinct at compile time.
pub struct ControlCell<Id> {
    id: Id,
    scope: VarScope,
    ty: ValueShape,
    initial: Option<Value>,
}
/// Debug shows metadata and whether an initial value exists, never the value itself.
impl<Id: std::fmt::Debug> std::fmt::Debug for ControlCell<Id> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlCell")
            .field("id", &self.id)
            .field("scope", &self.scope)
            .field("ty", &self.ty)
            .field("has_initial", &self.initial.is_some())
            .finish()
    }
}
impl<Id: Copy> ControlCell<Id> {
    pub fn id(&self) -> Id {
        self.id
    }
    pub fn scope(&self) -> VarScope {
        self.scope
    }
    pub fn ty(&self) -> &ValueShape {
        &self.ty
    }
    pub fn initial(&self) -> Option<&Value> {
        self.initial.as_ref()
    }
    pub fn charge(&self, value: &Value) -> Option<usize> {
        self.ty.checked_charge(value, usize::MAX).ok()
    }
    pub fn checked_charge(
        &self,
        value: &Value,
        limit: usize,
    ) -> Result<usize, crate::ValueCheckError> {
        self.ty.checked_charge(value, limit)
    }
}
#[derive(Debug, Clone, Copy)]
pub enum HostSource {
    Variable(usize),
    /// Read-only volatile calls get their own initialized locals, never caller locals.
    /// Mutable action/async call frames are a separate execution contract.
    CallLocal(usize),
    FrameLocal {
        scope: usize,
        variable: usize,
    },
    Argument {
        scope: usize,
        index: usize,
    },
    CallOutput(ControlIx, usize),
    EventPayload(ControlIx),
    Tick(ControlIx, TickProjection),
    Change(ControlIx, ChangeProjection),
    /// `core/now`: the run's virtual time since its clock's zero, read once per epoch.
    Clock,
    Error(ControlIx),
    Index(ControlIx),
    Value(ControlIx),
    State(usize),
    Iteration(ControlIx, IterationProjection),
    Update(ControlIx, UpdateProjection),
}

#[derive(Debug, Clone)]
pub struct ControlNode {
    scope: usize,
    pub(crate) node: NodeId,
    pub(crate) operation: ControlOperation,
    pub(crate) then: Option<ExecTarget>,
    pub(crate) failed: Option<ExecTarget>,
    pub(crate) cancelled: Option<ExecTarget>,
    pub(crate) fault_policy: FaultPolicy,
    pub(crate) loop_owner: Option<ControlIx>,
}

/// Checked run-scope event metadata used by the runtime dispatcher.
#[derive(Debug, Clone)]
pub struct ControlEvent {
    id: EventDeclId,
    payload_type: ValueShape,
    handlers: Box<[EventHandler]>,
}
impl ControlEvent {
    pub fn id(&self) -> EventDeclId {
        self.id
    }
    pub fn payload_type(&self) -> &ValueShape {
        &self.payload_type
    }
    pub fn handlers(&self) -> &[EventHandler] {
        &self.handlers
    }
}

/// One admitted top-level On Event entry, in persisted handler order.
#[derive(Debug, Clone, Copy)]
pub struct EventHandler {
    entry: ControlIx,
    capacity: usize,
    overflow: OverflowPolicy,
    mode: ReentrancyMode,
    max_parallel: usize,
}
impl EventHandler {
    pub fn entry(&self) -> ControlIx {
        self.entry
    }
    /// Effective pending capacity; Replace retains one latest envelope.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn overflow(&self) -> OverflowPolicy {
        self.overflow
    }
    /// Checked entry reentrancy policy.
    pub fn mode(&self) -> ReentrancyMode {
        self.mode
    }
    /// Maximum simultaneously owned handler roots; one except for Parallel.
    pub fn max_parallel(&self) -> usize {
        self.max_parallel
    }
}

/// One admitted root `core/on-manual` entry: a private single-handler channel that only
/// the host trigger API feeds. `payload_type` is `None` when the entry declares no payload.
#[derive(Debug, Clone)]
pub struct ControlManual {
    node: NodeId,
    payload_type: Option<ValueShape>,
    handler: EventHandler,
}
impl ControlManual {
    pub fn node(&self) -> NodeId {
        self.node
    }
    pub fn payload_type(&self) -> Option<&ValueShape> {
        self.payload_type.as_ref()
    }
    pub fn handler(&self) -> &EventHandler {
        &self.handler
    }
}
/// One admitted root timer entry: a private single-handler channel that only its run's
/// virtual-clock schedule feeds.
#[derive(Debug, Clone)]
pub struct ControlTimer {
    node: NodeId,
    schedule: TimerSchedule,
    handler: EventHandler,
}
impl ControlTimer {
    pub fn node(&self) -> NodeId {
        self.node
    }
    pub fn schedule(&self) -> TimerSchedule {
        self.schedule
    }
    pub fn handler(&self) -> &EventHandler {
        &self.handler
    }
}
/// One admitted root `core/on-variable-changed` entry: a private single-handler channel fed
/// at segment ends when its run variable's value changed under its type's host `eq`.
#[derive(Debug, Clone)]
pub struct ControlChange {
    node: NodeId,
    variable: usize,
    /// `list<T>` of `[value, previous]`, so one delivery charges both values.
    payload_type: ValueShape,
    handler: EventHandler,
}
impl ControlChange {
    pub fn node(&self) -> NodeId {
        self.node
    }
    /// Index into [`ControlPlan::variables`].
    pub fn variable(&self) -> usize {
        self.variable
    }
    pub fn payload_type(&self) -> &ValueShape {
        &self.payload_type
    }
    pub fn handler(&self) -> &EventHandler {
        &self.handler
    }
}
impl ControlNode {
    /// Static compilation scope; zero is the root, not a runtime invocation ID.
    pub fn scope(&self) -> usize {
        self.scope
    }
    pub fn node(&self) -> NodeId {
        self.node
    }
    pub fn operation(&self) -> &ControlOperation {
        &self.operation
    }
    pub fn then(&self) -> Option<ExecTarget> {
        self.then
    }
    pub fn failed(&self) -> Option<ExecTarget> {
        self.failed
    }
    pub fn cancelled(&self) -> Option<ExecTarget> {
        self.cancelled
    }
    pub fn fault_policy(&self) -> FaultPolicy {
        self.fault_policy
    }
    pub fn loop_owner(&self) -> Option<ControlIx> {
        self.loop_owner
    }
}

/// Immutable layout for one expanded action/async call site.
#[derive(Debug)]
pub struct FunctionLayout {
    outputs: usize,
    projections: Box<[usize]>,
    entry: Option<ExecTarget>,
    locals: Box<[usize]>,
}
impl FunctionLayout {
    /// Host-input indices whose execution projections belong to this invocation.
    pub fn projections(&self) -> &[usize] {
        &self.projections
    }
    pub fn outputs(&self) -> usize {
        self.outputs
    }
    pub fn entry(&self) -> Option<ExecTarget> {
        self.entry
    }
    pub fn locals(&self) -> &[usize] {
        &self.locals
    }
}

/// Private checked construction, no serde/default bypass. Debug omits payloads.
///
/// ```compile_fail
/// let plan = orange_compiler::ControlPlan::default();
/// ```
pub struct ControlPlan {
    functions: Box<[Option<FunctionLayout>]>,
    pub(crate) data: Arc<DataPlan>,
    pub(crate) nodes: Box<[ControlNode]>,
    pub(crate) starts: Box<[ControlIx]>,
    pub(crate) stops: Box<[ControlIx]>,
    events: Box<[ControlEvent]>,
    manuals: Box<[ControlManual]>,
    timers: Box<[ControlTimer]>,
    changes: Box<[ControlChange]>,
    pub(crate) variables: Box<[ControlVariable]>,
    states: Box<[ControlState]>,
    pub(crate) host_sources: Box<[HostSource]>,
    root_inputs: Box<[Arc<[usize]>]>,
    index_targets: Box<[Box<[usize]>]>,
    error_targets: Box<[Box<[usize]>]>,
    value_targets: Box<[Box<[usize]>]>,
}
impl std::fmt::Debug for ControlPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlPlan")
            .field("nodes", &self.nodes.len())
            .field("starts", &self.starts.len())
            .field("stops", &self.stops.len())
            .field("events", &self.events.len())
            .field("manuals", &self.manuals.len())
            .field("timers", &self.timers.len())
            .field("changes", &self.changes.len())
            .finish_non_exhaustive()
    }
}
impl ControlPlan {
    pub fn function(&self, scope: usize) -> Option<&FunctionLayout> {
        self.functions.get(scope.checked_sub(1)?)?.as_ref()
    }
    pub fn data(&self) -> &Arc<DataPlan> {
        &self.data
    }
    pub fn nodes(&self) -> &[ControlNode] {
        &self.nodes
    }
    pub fn starts(&self) -> &[ControlIx] {
        &self.starts
    }
    pub fn stops(&self) -> &[ControlIx] {
        &self.stops
    }
    pub fn events(&self) -> &[ControlEvent] {
        &self.events
    }
    /// Root manual entries in persisted handler order, then node ID.
    pub fn manuals(&self) -> &[ControlManual] {
        &self.manuals
    }
    /// Root timer entries in persisted handler order, then node ID.
    pub fn timers(&self) -> &[ControlTimer] {
        &self.timers
    }
    /// Root variable-changed entries in persisted handler order, then node ID.
    pub fn changes(&self) -> &[ControlChange] {
        &self.changes
    }
    pub fn variables(&self) -> &[ControlVariable] {
        &self.variables
    }
    pub fn states(&self) -> &[ControlState] {
        &self.states
    }
    /// Parallel to data().host_inputs(), resolved once outside execution.
    pub fn host_sources(&self) -> &[HostSource] {
        &self.host_sources
    }
    /// Potential inputs for one consumer, in host-input index order. Lazy arms
    /// remain potential here; the runtime publishes only actual data demands.
    pub fn inputs_for(&self, root: usize) -> Option<&[usize]> {
        self.root_inputs.get(root).map(AsRef::as_ref)
    }
    pub fn index_targets(&self, owner: ControlIx) -> &[usize] {
        &self.index_targets[owner.index()]
    }
    pub fn error_targets(&self, owner: ControlIx) -> &[usize] {
        &self.error_targets[owner.index()]
    }
    pub fn value_targets(&self, owner: ControlIx) -> &[usize] {
        &self.value_targets[owner.index()]
    }
    pub fn is_current(&self, document: &GraphDocument, registry: &RegistrySnapshot) -> bool {
        self.data.is_current(document, registry)
    }
}

/// A control node in its call scope; scope 0 is the root body.
type ControlKey = (usize, NodeId);
type PortSite = (usize, NodeId, orange_model::id::PortKey);

/// Borrowed inputs every control lowering phase reads.
struct Cx<'a> {
    document: &'a GraphDocument,
    snapshot: &'a RegistrySnapshot,
    analysis: &'a ValidationAnalysis,
    calls: &'a crate::calls::Calls<'a>,
    options: CompileOptions<'a>,
}

impl Cx<'_> {
    fn node(&self, (scope, id): ControlKey) -> &orange_model::document::NodeInstance {
        &self.calls.body(self.document, scope).nodes[&id]
    }
    fn ports(&self, (scope, id): ControlKey) -> &[orange_model::document::PortDescriptor] {
        &self.calls.signature(self.analysis, scope, id).ports
    }
    fn port(
        &self,
        key: ControlKey,
        name: &str,
    ) -> Result<&orange_model::document::PortDescriptor, CompileError> {
        self.ports(key)
            .iter()
            .find(|p| p.port_id.as_str() == name)
            .ok_or(CompileError::ValidationIncomplete)
    }
    /// Position of `port` among the node's data outputs.
    fn data_output(
        &self,
        key: ControlKey,
        port: &orange_model::id::PortKey,
    ) -> Result<usize, CompileError> {
        self.ports(key)
            .iter()
            .filter(|p| p.direction == PortDirection::Output && p.kind == PortKind::Data)
            .position(|p| port_key(p) == *port)
            .ok_or(CompileError::ValidationIncomplete)
    }
}

/// Compile a deliberately bounded executable control vocabulary. All root nodes,
/// including unsupported unreachable nodes, are admitted before resolver callbacks.
pub fn compile_control(
    document: &GraphDocument,
    snapshot: Arc<RegistrySnapshot>,
    analysis: &ValidationAnalysis,
    resolver: &impl EntropyKernelResolver,
    options: CompileOptions<'_>,
) -> Result<ControlPlan, CompileError> {
    options.checkpoint()?;
    if analysis.document_state() != document.semantic_state_id() {
        return Err(CompileError::StaleDocument);
    }
    if analysis.registry_generation() != snapshot.generation() {
        return Err(CompileError::StaleRegistry);
    }
    if !analysis.is_ready_for_lowering(document, &snapshot) {
        return Err(CompileError::ValidationIncomplete);
    }
    // Reject cheap count failures before decoding initializers or building tables.
    let mut graph_items = 0;
    for count in [
        document.root().nodes.len(),
        document.root().edges.len(),
        document.exports().len(),
    ] {
        charge(&mut graph_items, count, options.limits.max_graph_items)?;
    }
    let calls = Arc::new(crate::calls::Calls::prepare(
        document,
        &snapshot,
        analysis,
        options,
        crate::calls::CallProfile::Control,
    )?);
    let cx = Cx {
        document,
        snapshot: &snapshot,
        analysis,
        calls: &calls,
        options,
    };
    let controls = admit_controls(&cx)?;
    let control_ix: BTreeMap<_, _> = controls
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, ControlIx(i)))
        .collect();
    let (events, event_ids) = prepare_events(&cx, &controls, &control_ix, &mut graph_items)?;
    let manuals = prepare_manuals(&cx, &controls, &control_ix, &mut graph_items)?;
    let timers = prepare_timers(&cx, &controls, &control_ix, &mut graph_items)?;
    let mut bytes = 0;
    let (variables, variable_nodes) = collect_variables(&cx, &mut bytes)?;
    let changes = prepare_changes(
        &cx,
        &controls,
        &control_ix,
        &variables,
        &variable_nodes,
        &mut graph_items,
    )?;
    let mut functions = function_layouts(&cx, &control_ix, &variables, &variable_nodes)?;
    let (states, state_nodes) = collect_states(&cx, &controls, &mut bytes)?;
    let (edges, producers) = exec_edges(&cx, &controls, &control_ix)?;
    let mut lowering = NodeLowering {
        cx: &cx,
        edges: &edges,
        control_ix: &control_ix,
        event_ids: &event_ids,
        variables: &variables,
        variable_nodes: &variable_nodes,
        state_nodes: &state_nodes,
        roots: Vec::new(),
        starts: Vec::new(),
        stops: Vec::new(),
        graph_items: &mut graph_items,
        bytes: &mut bytes,
    };
    let mut nodes = Vec::with_capacity(controls.len());
    for key in &controls {
        options.checkpoint()?;
        nodes.push(lowering.lower(*key)?);
    }
    let NodeLowering {
        mut roots,
        mut starts,
        mut stops,
        ..
    } = lowering;
    let activation_order = |i: &ControlIx| {
        (
            document.root().nodes[&nodes[i.0].node]
                .activation
                .as_ref()
                .map_or(0, |a| a.order),
            nodes[i.0].node,
        )
    };
    starts.sort_by_key(activation_order);
    stops.sort_by_key(activation_order);
    for export in document.exports() {
        roots.push(DemandRoot {
            scope: 0,
            id: export.id.clone(),
            node: export.node,
            port: export.port.clone(),
            input: false,
        });
    }
    let root_nodes: Vec<_> = roots
        .iter()
        .filter(|r| r.scope == 0)
        .filter_map(|r| {
            if r.input {
                producers.get(&(r.scope, r.node, r.port.clone())).copied()
            } else {
                Some(r.node)
            }
        })
        .collect();
    let mut data_options = options;
    data_options.limits.max_constant_bytes = options
        .limits
        .max_constant_bytes
        .checked_sub(bytes)
        .ok_or(CompileError::ResourceLimit)?;
    let root_controls = controls
        .iter()
        .filter_map(|(scope, id)| (*scope == 0).then_some(*id))
        .collect();
    let mut admission = admit_roots(
        document,
        &snapshot,
        analysis,
        data_options,
        true,
        &root_nodes,
        &root_controls,
    )?;
    admission.host_nodes = controls.clone();
    let data = lower_roots(
        document,
        snapshot.clone(),
        analysis,
        resolver,
        data_options,
        admission,
        &roots,
        calls.clone(),
    )?;
    let tables = HostTables {
        controls: &controls,
        control_ix: &control_ix,
        nodes: &nodes,
        variables: &variables,
        variable_nodes: &variable_nodes,
        state_nodes: &state_nodes,
    };
    let host_sources = data
        .host_inputs()
        .iter()
        .map(|ix| host_source(&cx, &tables, &data, &data.nodes()[ix.index()]))
        .collect::<Result<Vec<_>, _>>()?;
    let mut owned_projections = vec![Vec::new(); functions.len()];
    for (input, source) in host_sources.iter().enumerate() {
        let scope = data.nodes()[data.host_inputs()[input].index()]
            .call_instance()
            .map(|i| i + 1);
        if let Some(scope) = scope
            && functions[scope - 1].is_some()
            && matches!(
                source,
                HostSource::Error(_)
                    | HostSource::Index(_)
                    | HostSource::Value(_)
                    | HostSource::EventPayload(_)
                    | HostSource::Update(_, _)
                    | HostSource::CallOutput(_, _)
            )
        {
            charge(&mut graph_items, 1, options.limits.max_graph_items)?;
            owned_projections[scope - 1].push(input);
        }
    }
    for (function, projections) in functions.iter_mut().zip(owned_projections) {
        if let Some(function) = function {
            function.projections = projections.into();
        }
    }
    let mut index_targets = vec![Vec::new(); nodes.len()];
    let mut error_targets = vec![Vec::new(); nodes.len()];
    let mut value_targets = vec![Vec::new(); nodes.len()];
    for (i, source) in host_sources.iter().enumerate() {
        match source {
            HostSource::Index(owner) => index_targets[owner.index()].push(i),
            HostSource::Error(owner) => error_targets[owner.index()].push(i),
            HostSource::Value(owner)
            | HostSource::EventPayload(owner)
            | HostSource::Tick(owner, _)
            | HostSource::Change(owner, _)
            | HostSource::Update(owner, _)
            | HostSource::CallOutput(owner, _) => value_targets[owner.index()].push(i),
            HostSource::Variable(_)
            | HostSource::CallLocal(_)
            | HostSource::FrameLocal { .. }
            | HostSource::Argument { .. }
            | HostSource::State(_)
            | HostSource::Clock
            | HostSource::Iteration(_, _) => {}
        }
    }
    let mut projection_items = 0;
    let mut scratch = UnionScratch::default();
    let dependencies = host_dependencies(&data, options, &mut projection_items, &mut scratch)?;
    let root_inputs = (0..data.exports().len())
        .map(|root| {
            data.export_node(root)
                .and_then(|n| dependencies.get(n.index()))
                .cloned()
                .ok_or(CompileError::ValidationIncomplete)
        })
        .collect::<Result<Vec<_>, _>>()?;
    for node in &mut nodes {
        if let ControlOperation::Update { demand, .. }
        | ControlOperation::Call { demand, .. }
        | ControlOperation::Return { demand } = &mut node.operation
        {
            demand.inputs = union_inputs(
                demand.roots.iter().map(|&root| &root_inputs[root]),
                options,
                &mut projection_items,
                &mut scratch,
            )?;
        }
    }
    let boxed = |targets: Vec<Vec<usize>>| targets.into_iter().map(Vec::into_boxed_slice).collect();
    Ok(ControlPlan {
        functions: functions.into(),
        data: Arc::new(data),
        nodes: nodes.into(),
        starts: starts.into(),
        stops: stops.into(),
        events: events.into(),
        manuals: manuals.into(),
        timers: timers.into(),
        changes: changes.into(),
        variables: variables.into(),
        states: states.into(),
        host_sources: host_sources.into(),
        root_inputs: root_inputs.into(),
        value_targets: boxed(value_targets),
        index_targets: boxed(index_targets),
        error_targets: boxed(error_targets),
    })
}

/// Admit every executable control node before any table is built or callback runs.
fn admit_controls(cx: &Cx<'_>) -> Result<BTreeSet<ControlKey>, CompileError> {
    let calls = cx.calls;
    let mut controls = BTreeSet::new();
    for scope in 0..calls.scopes.len() {
        if scope != 0 && !calls.scopes[scope].control {
            continue;
        }
        for node in calls.body(cx.document, scope).nodes.values() {
            let sig = calls.signature(cx.analysis, scope, node.id);
            if matches!(sig.kind, NodeKind::Pure | NodeKind::Volatile)
                && node.definition.id.as_str() != ids::RETURN
            {
                continue;
            }
            if !matches!(
                node.definition.id.as_str(),
                ids::CALL
                    | ids::FUNCTION_ENTRY
                    | ids::RETURN
                    | ids::GATE
                    | ids::DO_ONCE
                    | ids::FLIP_FLOP
                    | ids::REGISTER
                    | ids::SWITCH
                    | ids::MATCH_RESULT
                    | ids::ON_START
                    | ids::ON_STOP
                    | ids::ON_MANUAL
                    | ids::ON_TIMER
                    | ids::ON_TIMEOUT
                    | ids::ON_VARIABLE_CHANGED
                    | ids::ON_EVENT
                    | ids::EMIT
                    | ids::AWAIT_EVENT
                    | ids::BRANCH
                    | ids::SET_VARIABLE
                    | ids::UPDATE_VARIABLE
                    | ids::SEQUENCE
                    | ids::FORK
                    | ids::YIELD
                    | ids::WAIT
                    | ids::COUNT
                    | ids::FOR_EACH
                    | ids::BREAK
                    | ids::CONTINUE
                    | ids::WHILE
            ) || node.definition.version != 1
                || node.config_version != 1
                || node.provider.is_some()
            {
                return Err(CompileError::UnsupportedNode { node: node.id });
            }
            controls.insert((scope, node.id));
            for port in sig.ports.iter().filter(|p| p.kind == PortKind::Data) {
                port_type(cx.snapshot, node.id, port)?;
            }
        }
    }
    Ok(controls)
}

fn prepare_events(
    cx: &Cx<'_>,
    controls: &BTreeSet<ControlKey>,
    control_ix: &BTreeMap<ControlKey, ControlIx>,
    graph_items: &mut usize,
) -> Result<(Vec<ControlEvent>, BTreeMap<EventDeclId, usize>), CompileError> {
    let mut referenced: BTreeMap<EventDeclId, Vec<(u32, NodeId, EventHandler)>> = BTreeMap::new();
    let mut payload_types = BTreeMap::new();
    for &(scope, node_id) in controls {
        cx.options.checkpoint()?;
        let node = cx.node((scope, node_id));
        let is_handler = node.definition.id.as_str() == ids::ON_EVENT;
        let is_emitter = node.definition.id.as_str() == ids::EMIT;
        let is_awaiter = node.definition.id.as_str() == ids::AWAIT_EVENT;
        if !is_handler && !is_emitter && !is_awaiter {
            continue;
        }
        let event_id = match cx.snapshot.reference(node, ReferenceKind::Event) {
            Ok(DeclId::Event(id)) => id,
            _ => return Err(CompileError::UnsupportedNode { node: node_id }),
        };
        let declaration = cx
            .document
            .events()
            .get(&event_id)
            .ok_or(CompileError::UnsupportedNode { node: node_id })?;
        if declaration.scope != EventScope::Run {
            return Err(CompileError::UnsupportedNode { node: node_id });
        }
        let payload_type = ValueShape::of_in(&declaration.payload, cx.snapshot.types().as_ref())
            .ok_or(CompileError::UnsupportedNode { node: node_id })?;
        payload_types.entry(event_id).or_insert(payload_type);
        // Charge before retaining executable event metadata. The type is a bounded
        // recursive shape (ValueShape::of_in enforces the language nesting limit).
        charge(graph_items, 1, cx.options.limits.max_graph_items)?;
        if is_handler {
            let (order, handler) = entry_handler(cx, control_ix, (scope, node_id), graph_items)?;
            referenced
                .entry(event_id)
                .or_default()
                .push((order, node_id, handler));
        } else {
            referenced.entry(event_id).or_default();
        }
    }

    let mut indices = BTreeMap::new();
    let mut events = Vec::with_capacity(referenced.len());
    for (id, mut handlers) in referenced {
        cx.options.checkpoint()?;
        handlers.sort_by_key(|(order, node, _)| (*order, *node));
        let payload_type = payload_types
            .remove(&id)
            .ok_or(CompileError::ValidationIncomplete)?;
        indices.insert(id, events.len());
        events.push(ControlEvent {
            id,
            payload_type,
            handlers: handlers
                .into_iter()
                .map(|(_, _, handler)| handler)
                .collect(),
        });
    }
    Ok((events, indices))
}

/// Checked bounded handler policy of a root entry: its persisted order and handler.
fn entry_handler(
    cx: &Cx<'_>,
    control_ix: &BTreeMap<ControlKey, ControlIx>,
    (scope, node_id): ControlKey,
    graph_items: &mut usize,
) -> Result<(u32, EventHandler), CompileError> {
    if scope != 0 {
        return Err(CompileError::UnsupportedNode { node: node_id });
    }
    let policy = cx
        .node((scope, node_id))
        .activation
        .as_ref()
        .ok_or(CompileError::ValidationIncomplete)?;
    let max_parallel = match (policy.mode, policy.max_parallel) {
        (ReentrancyMode::Queue | ReentrancyMode::Drop | ReentrancyMode::Replace, None) => 1,
        (ReentrancyMode::Parallel, Some(max)) if max > 0 => {
            usize::try_from(max).map_err(|_| CompileError::ResourceLimit)?
        }
        _ => return Err(CompileError::UnsupportedNode { node: node_id }),
    };
    if policy.capacity == 0 || policy.overflow == OverflowPolicy::Backpressure {
        return Err(CompileError::UnsupportedNode { node: node_id });
    }
    charge(graph_items, 1, cx.options.limits.max_graph_items)?;
    let handler = EventHandler {
        entry: *control_ix
            .get(&(scope, node_id))
            .ok_or(CompileError::ValidationIncomplete)?,
        capacity: if policy.mode == ReentrancyMode::Replace {
            1
        } else {
            usize::try_from(policy.capacity).map_err(|_| CompileError::ResourceLimit)?
        },
        overflow: policy.overflow,
        mode: policy.mode,
        max_parallel,
    };
    Ok((policy.order, handler))
}

/// Root `core/on-manual` entries with their checked payload shape and handler policy.
fn prepare_manuals(
    cx: &Cx<'_>,
    controls: &BTreeSet<ControlKey>,
    control_ix: &BTreeMap<ControlKey, ControlIx>,
    graph_items: &mut usize,
) -> Result<Vec<ControlManual>, CompileError> {
    let mut manuals = Vec::new();
    for &key in controls {
        cx.options.checkpoint()?;
        if cx.node(key).definition.id.as_str() != ids::ON_MANUAL {
            continue;
        }
        let payload_type = match cx.port(key, "payload") {
            Ok(port) => Some(port_type(cx.snapshot, key.1, port)?),
            Err(_) => None,
        };
        charge(graph_items, 1, cx.options.limits.max_graph_items)?;
        let (order, handler) = entry_handler(cx, control_ix, key, graph_items)?;
        manuals.push((order, key.1, payload_type, handler));
    }
    manuals.sort_by_key(|(order, node, _, _)| (*order, *node));
    Ok(manuals
        .into_iter()
        .map(|(_, node, payload_type, handler)| ControlManual {
            node,
            payload_type,
            handler,
        })
        .collect())
}

/// Root `core/on-timer`/`core/on-timeout` entries with their checked schedule and handler.
fn prepare_timers(
    cx: &Cx<'_>,
    controls: &BTreeSet<ControlKey>,
    control_ix: &BTreeMap<ControlKey, ControlIx>,
    graph_items: &mut usize,
) -> Result<Vec<ControlTimer>, CompileError> {
    let mut timers = Vec::new();
    for &key in controls {
        cx.options.checkpoint()?;
        let node = cx.node(key);
        if !matches!(node.definition.id.as_str(), ids::ON_TIMER | ids::ON_TIMEOUT) {
            continue;
        }
        let schedule = TimerSchedule::from_node(node).ok_or(CompileError::ValidationIncomplete)?;
        charge(graph_items, 1, cx.options.limits.max_graph_items)?;
        let (order, handler) = entry_handler(cx, control_ix, key, graph_items)?;
        timers.push((order, key.1, schedule, handler));
    }
    timers.sort_by_key(|(order, node, _, _)| (*order, *node));
    Ok(timers
        .into_iter()
        .map(|(_, node, schedule, handler)| ControlTimer {
            node,
            schedule,
            handler,
        })
        .collect())
}

/// Root `core/on-variable-changed` entries. Equality must run on the host, because a
/// segment end compares values without package code.
fn prepare_changes(
    cx: &Cx<'_>,
    controls: &BTreeSet<ControlKey>,
    control_ix: &BTreeMap<ControlKey, ControlIx>,
    variables: &[ControlVariable],
    variable_nodes: &BTreeMap<NodeId, usize>,
    graph_items: &mut usize,
) -> Result<Vec<ControlChange>, CompileError> {
    let mut changes = Vec::new();
    for &key in controls {
        cx.options.checkpoint()?;
        if cx.node(key).definition.id.as_str() != ids::ON_VARIABLE_CHANGED {
            continue;
        }
        let variable = *variable_nodes
            .get(&key.1)
            .ok_or(CompileError::ValidationIncomplete)?;
        let ty = variables[variable].ty();
        if variables[variable].scope() != VarScope::Run || !ty.host_eq() {
            return Err(CompileError::UnsupportedNode { node: key.1 });
        }
        charge(graph_items, 1, cx.options.limits.max_graph_items)?;
        let (order, handler) = entry_handler(cx, control_ix, key, graph_items)?;
        changes.push((order, key.1, variable, handler));
    }
    changes.sort_by_key(|(order, node, _, _)| (*order, *node));
    Ok(changes
        .into_iter()
        .map(|(_, node, variable, handler)| ControlChange {
            node,
            variable,
            payload_type: ValueShape::List(Box::new(variables[variable].ty().clone())),
            handler,
        })
        .collect())
}

/// Variable layouts for every Get/Set/Update, including unused or lazy call bodies,
/// and the layout each such node uses.
fn collect_variables(
    cx: &Cx<'_>,
    bytes: &mut usize,
) -> Result<(Vec<ControlVariable>, BTreeMap<NodeId, usize>), CompileError> {
    let (document, calls, options) = (cx.document, cx.calls, cx.options);
    let mut variables = Vec::new();
    let mut variable_ids = BTreeMap::new();
    let mut variable_nodes = BTreeMap::new();
    let called_nodes = calls
        .scopes
        .iter()
        .enumerate()
        .skip(1)
        .flat_map(|(scope, _)| calls.body(document, scope).nodes.values());
    // Visit every potential call before binding, including unused/lazy body nodes.
    for node in document.root().nodes.values().chain(called_nodes) {
        options.checkpoint()?;
        if crate::compile::is_state_read(cx.snapshot, node)
            && node.definition.id.as_str() != ids::GET_VARIABLE
        {
            return Err(CompileError::UnsupportedNode { node: node.id });
        }
        if !matches!(
            node.definition.id.as_str(),
            ids::GET_VARIABLE | ids::SET_VARIABLE | ids::UPDATE_VARIABLE | ids::ON_VARIABLE_CHANGED
        ) {
            continue;
        }
        let id: VariableId = node
            .config
            .get("variable")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .ok_or(CompileError::ValidationIncomplete)?;
        let declaration = &document.variables()[&id];
        if declaration.scope == VarScope::Workspace {
            return Err(CompileError::UnsupportedNode { node: node.id });
        }
        if let std::collections::btree_map::Entry::Vacant(entry) = variable_ids.entry(id) {
            let ty = ValueShape::of_in(&declaration.ty, cx.snapshot.types().as_ref())
                .ok_or(CompileError::UnsupportedNode { node: node.id })?;
            let initial = match &declaration.init {
                Some(literal) => Some(
                    decode(
                        literal,
                        &ty,
                        cx.snapshot,
                        options.limits.max_constant_bytes.saturating_sub(*bytes),
                    )?
                    .ok_or(CompileError::UnsupportedNode { node: node.id })?,
                ),
                None => None,
            };
            charge(
                bytes,
                initial
                    .as_ref()
                    .map_or(Ok(VALUE_CHARGE), |v| value_bytes(&ty, v))?,
                options.limits.max_constant_bytes,
            )?;
            entry.insert(variables.len());
            variables.push(ControlVariable {
                id,
                scope: declaration.scope,
                ty,
                initial,
            });
        }
        if node.definition.id.as_str() == ids::UPDATE_VARIABLE {
            update::resolve(node.id, &node.config, variables[variable_ids[&id]].ty())?;
        }
        variable_nodes.insert(node.id, variable_ids[&id]);
    }
    Ok((variables, variable_nodes))
}

/// Frame layout of each control function: its entry and the local variables it owns.
fn function_layouts(
    cx: &Cx<'_>,
    control_ix: &BTreeMap<ControlKey, ControlIx>,
    variables: &[ControlVariable],
    variable_nodes: &BTreeMap<NodeId, usize>,
) -> Result<Vec<Option<FunctionLayout>>, CompileError> {
    let calls = cx.calls;
    (1..calls.scopes.len())
        .map(|scope| {
            cx.options.checkpoint()?;
            if !calls.scopes[scope].control {
                return Ok(None);
            }
            let mut locals = BTreeSet::new();
            for node in calls.body(cx.document, scope).nodes.values() {
                cx.options.checkpoint()?;
                if let Some(&variable) = variable_nodes.get(&node.id)
                    && variables[variable].scope() == VarScope::Local
                {
                    locals.insert(variable);
                }
            }
            Ok(Some(FunctionLayout {
                projections: Box::new([]),
                outputs: cx.document.functions()[&calls.sites[scope - 1].function]
                    .outputs
                    .len(),
                entry: calls
                    .entry(scope)
                    .and_then(|id| control_ix.get(&(scope, id)))
                    .copied()
                    .map(Into::into),
                locals: locals.into_iter().collect::<Vec<_>>().into(),
            }))
        })
        .collect()
}

/// Run- or activation-owned cells of Gate, DoOnce, FlipFlop and Register.
fn collect_states(
    cx: &Cx<'_>,
    controls: &BTreeSet<ControlKey>,
    bytes: &mut usize,
) -> Result<(Vec<ControlState>, BTreeMap<ControlKey, usize>), CompileError> {
    let options = cx.options;
    let mut states = Vec::new();
    let mut state_nodes = BTreeMap::new();
    for key in controls {
        options.checkpoint()?;
        let node = cx.node(*key);
        let (scope, ty, initial) = match node.definition.id.as_str() {
            ids::GATE => {
                let open = match node.config.get("start_open") {
                    None => false,
                    Some(value) => value.as_bool().ok_or(CompileError::ValidationIncomplete)?,
                };
                (VarScope::Run, ValueShape::Bool, Some(Value::Bool(open)))
            }
            ids::DO_ONCE | ids::FLIP_FLOP => {
                (VarScope::Run, ValueShape::Bool, Some(Value::Bool(false)))
            }
            ids::REGISTER => {
                let port = cx.port(*key, "current")?;
                let ty = port_type(cx.snapshot, key.1, port)?;
                let scope = match node.config.get("scope").and_then(|s| s.as_str()) {
                    None | Some("run") => VarScope::Run,
                    Some("activation") => VarScope::Local,
                    _ => return Err(CompileError::ValidationIncomplete),
                };
                let invalid = || CompileError::InvalidLiteral {
                    node: key.1,
                    port: port_key(port),
                };
                let initial = match node.config.get("initial") {
                    Some(raw) => {
                        let mut admitted = *bytes;
                        crate::calls::charge_json(raw, &mut admitted, options)?;
                        let literal = serde_json::from_value(raw.clone()).map_err(|_| invalid())?;
                        Some(
                            decode(
                                &literal,
                                &ty,
                                cx.snapshot,
                                options.limits.max_constant_bytes.saturating_sub(*bytes),
                            )?
                            .ok_or_else(invalid)?,
                        )
                    }
                    None => None,
                };
                (scope, ty, initial)
            }
            _ => continue,
        };
        charge(
            bytes,
            initial
                .as_ref()
                .map_or(Ok(VALUE_CHARGE), |v| value_bytes(&ty, v))?,
            options.limits.max_constant_bytes,
        )?;
        state_nodes.insert(*key, states.len());
        states.push(ControlState {
            id: key.1,
            scope,
            ty,
            initial,
        });
    }
    Ok((states, state_nodes))
}

/// Exec edges between admitted controls as typed destinations, plus every data
/// input's producer.
#[allow(clippy::type_complexity)]
fn exec_edges(
    cx: &Cx<'_>,
    controls: &BTreeSet<ControlKey>,
    control_ix: &BTreeMap<ControlKey, ControlIx>,
) -> Result<(BTreeMap<PortSite, ExecTarget>, BTreeMap<PortSite, NodeId>), CompileError> {
    let mut edges = BTreeMap::new();
    let mut producers = BTreeMap::new();
    for scope in 0..cx.calls.scopes.len() {
        let body = cx.calls.body(cx.document, scope);
        for edge in body.edges.values() {
            producers.insert((scope, edge.to.node, edge.to.port.clone()), edge.from.node);
            if !controls.contains(&(scope, edge.to.node))
                || !controls.contains(&(scope, edge.from.node))
            {
                continue;
            }
            let sig = cx.calls.signature(cx.analysis, scope, edge.from.node);
            let port = edge.from.port.port();
            let exec = (sig.implicit_ports().contains(&port.as_str()) && port.as_str() != "error")
                || sig
                    .ports
                    .iter()
                    .any(|p| p.port_id == *port && p.kind == PortKind::Exec);
            if exec {
                let to = destination(
                    control_ix[&(scope, edge.to.node)],
                    body.nodes[&edge.to.node].definition.id.as_str(),
                    &edge.to.port,
                )
                .ok_or_else(|| CompileError::UnsupportedPort {
                    node: edge.to.node,
                    port: edge.to.port.clone(),
                })?;
                edges.insert((scope, edge.from.node, edge.from.port.clone()), to);
            }
        }
    }
    Ok((edges, producers))
}

/// Per-node lowering state; demand roots and entry lists accumulate across nodes.
struct NodeLowering<'a, 'c> {
    cx: &'a Cx<'c>,
    edges: &'a BTreeMap<PortSite, ExecTarget>,
    control_ix: &'a BTreeMap<ControlKey, ControlIx>,
    event_ids: &'a BTreeMap<EventDeclId, usize>,
    variables: &'a [ControlVariable],
    variable_nodes: &'a BTreeMap<NodeId, usize>,
    state_nodes: &'a BTreeMap<ControlKey, usize>,
    roots: Vec<DemandRoot>,
    starts: Vec<ControlIx>,
    stops: Vec<ControlIx>,
    graph_items: &'a mut usize,
    bytes: &'a mut usize,
}

impl NodeLowering<'_, '_> {
    fn target(&self, (scope, id): ControlKey, name: &'static str) -> Option<ExecTarget> {
        self.edges
            .get(&(scope, id, orange_model::id::fixed_id(name)))
            .copied()
    }
    fn input(&mut self, (scope, id): ControlKey, port: orange_model::id::PortId) -> usize {
        input_root(&mut self.roots, scope, id, port)
    }
    /// Targets of a variadic exec output, in instance order.
    fn variadic_targets(
        &mut self,
        key: ControlKey,
        port: &'static str,
    ) -> Result<Vec<Option<ExecTarget>>, CompileError> {
        let cx = self.cx;
        let name: orange_model::id::PortId = orange_model::id::fixed_id(port);
        let state = cx
            .node(key)
            .variadic
            .get(&name)
            .ok_or(CompileError::ValidationIncomplete)?;
        charge(
            self.graph_items,
            state.order.len(),
            cx.options.limits.max_graph_items,
        )?;
        Ok(state
            .order
            .iter()
            .map(|k| {
                let port = orange_model::id::PortKey::new(name.clone(), Some(*k));
                self.edges.get(&(key.0, key.1, port)).copied()
            })
            .collect())
    }

    fn lower(&mut self, key: ControlKey) -> Result<ControlNode, CompileError> {
        let cx = self.cx;
        let (scope, id) = key;
        let node = cx.node(key);
        let operation = self.operation(key)?;
        for port in cx
            .ports(key)
            .iter()
            .filter(|p| p.kind == PortKind::Data && p.direction == PortDirection::Input)
        {
            let optional_await_timeout = node.definition.id.as_str() == ids::AWAIT_EVENT
                && port.port_id.as_str() == "timeout";
            if (port.is_optional() && !optional_await_timeout)
                || port.is_lazy()
                || port.variadic.is_some()
            {
                return Err(CompileError::UnsupportedPort {
                    node: id,
                    port: port_key(port),
                });
            }
        }
        Ok(ControlNode {
            scope,
            node: id,
            operation,
            then: self.target(key, "then"),
            failed: self.target(key, "failed"),
            cancelled: self.target(key, "cancelled"),
            fault_policy: node
                .activation
                .as_ref()
                .map_or(FaultPolicy::Continue, |a| a.on_fault),
            loop_owner: cx
                .analysis
                .report()
                .regions
                .loop_of(id)
                .and_then(|r| self.control_ix.get(&(scope, r.node)))
                .copied(),
        })
    }

    fn operation(&mut self, key: ControlKey) -> Result<ControlOperation, CompileError> {
        let cx = self.cx;
        let (scope, id) = key;
        let node = cx.node(key);
        let definition = node.definition.id.as_str();
        Ok(match definition {
            ids::CALL | ids::RETURN => {
                let ports: Vec<_> = cx
                    .ports(key)
                    .iter()
                    .filter(|p| p.direction == PortDirection::Input && p.kind == PortKind::Data)
                    .map(|p| p.port_id.clone())
                    .collect();
                let demand = UpdateDemand {
                    roots: ports.into_iter().map(|p| self.input(key, p)).collect(),
                    inputs: Arc::from([]),
                };
                if definition == ids::CALL {
                    ControlOperation::Call {
                        scope: cx.calls.scopes[scope].calls[&id],
                        demand,
                    }
                } else {
                    ControlOperation::Return { demand }
                }
            }
            ids::FUNCTION_ENTRY => ControlOperation::Entry,
            ids::UPDATE_VARIABLE => {
                let variable = self.variable_nodes[&id];
                let operation = update::resolve(id, &node.config, self.variables[variable].ty())?;
                let mut operands = Vec::new();
                if matches!(operation, UpdateOperation::CompareAndSet) {
                    operands.push(self.input(key, orange_model::port_id!("expected")));
                }
                operands.push(self.input(key, orange_model::port_id!("value")));
                ControlOperation::Update {
                    variable,
                    operation,
                    demand: UpdateDemand {
                        roots: operands.into(),
                        inputs: Arc::from([]),
                    },
                }
            }
            ids::GATE => ControlOperation::Gate {
                state: self.state_nodes[&key],
            },
            ids::DO_ONCE => ControlOperation::DoOnce {
                state: self.state_nodes[&key],
            },
            ids::FLIP_FLOP => ControlOperation::FlipFlop {
                state: self.state_nodes[&key],
                a: self.target(key, "a"),
                b: self.target(key, "b"),
            },
            ids::REGISTER => ControlOperation::Register {
                state: self.state_nodes[&key],
                next: self.input(key, orange_model::port_id!("next")),
            },
            ids::SWITCH => self.switch(key)?,
            ids::EMIT => {
                let event_id = match cx.snapshot.reference(node, ReferenceKind::Event) {
                    Ok(DeclId::Event(id)) => id,
                    _ => return Err(CompileError::UnsupportedNode { node: id }),
                };
                ControlOperation::Emit {
                    event: *self
                        .event_ids
                        .get(&event_id)
                        .ok_or(CompileError::ValidationIncomplete)?,
                    payload: self.input(key, orange_model::port_id!("payload")),
                }
            }
            ids::AWAIT_EVENT => {
                let event_id = match cx.snapshot.reference(node, ReferenceKind::Event) {
                    Ok(DeclId::Event(id)) => id,
                    _ => return Err(CompileError::UnsupportedNode { node: id }),
                };
                let timeout_port =
                    orange_model::id::PortKey::new(orange_model::port_id!("timeout"), None);
                let timeout_provided = node.inputs.contains_key(&timeout_port)
                    || cx
                        .calls
                        .body(cx.document, scope)
                        .edges
                        .values()
                        .any(|edge| edge.to.node == id && edge.to.port == timeout_port);
                ControlOperation::AwaitEvent {
                    event: *self
                        .event_ids
                        .get(&event_id)
                        .ok_or(CompileError::ValidationIncomplete)?,
                    timeout: timeout_provided
                        .then(|| self.input(key, orange_model::port_id!("timeout"))),
                    timed_out: self.target(key, "timed_out"),
                }
            }
            ids::MATCH_RESULT => {
                let port = cx.port(key, "result")?;
                let ValueShape::Result(inner) = port_type(cx.snapshot, id, port)? else {
                    return Err(CompileError::ValidationIncomplete);
                };
                ControlOperation::MatchResult {
                    result: self.input(key, orange_model::port_id!("result")),
                    boxed: inner.unboxed_error(),
                    ok: self.target(key, "ok"),
                    err: self.target(key, "err"),
                }
            }
            ids::ON_START | ids::ON_STOP => {
                let entries = if definition == ids::ON_START {
                    &mut self.starts
                } else {
                    &mut self.stops
                };
                entries.push(self.control_ix[&key]);
                ControlOperation::Entry
            }
            ids::ON_EVENT
            | ids::ON_MANUAL
            | ids::ON_TIMER
            | ids::ON_TIMEOUT
            | ids::ON_VARIABLE_CHANGED => ControlOperation::Entry,
            ids::BRANCH => ControlOperation::Branch {
                condition: self.input(key, orange_model::port_id!("condition")),
                when_true: self.target(key, "true"),
                when_false: self.target(key, "false"),
            },
            ids::SET_VARIABLE => ControlOperation::Set {
                variable: self.variable_nodes[&id],
                value: self.input(key, orange_model::port_id!("value")),
            },
            ids::SEQUENCE => ControlOperation::Sequence {
                steps: self.variadic_targets(key, "then")?.into(),
            },
            ids::FORK => {
                let branches = self.variadic_targets(key, "branch")?;
                let join = match node.config.get("join").and_then(|j| j.as_str()) {
                    Some("all") => ForkJoin::All,
                    Some("any") => ForkJoin::Any,
                    Some("race") => ForkJoin::Race,
                    Some("detach") => ForkJoin::Detach,
                    _ => return Err(CompileError::UnsupportedNode { node: id }),
                };
                ControlOperation::Fork {
                    branches: branches.into(),
                    completed: self.target(key, "completed"),
                    join,
                }
            }
            ids::YIELD => ControlOperation::Yield,
            ids::WAIT => ControlOperation::Wait {
                duration: self.input(key, orange_model::port_id!("duration")),
            },
            ids::FOR_EACH => ControlOperation::ForEach {
                items: self.input(key, orange_model::port_id!("items")),
                body: self.target(key, "body"),
                completed: self.target(key, "completed"),
            },
            ids::COUNT => ControlOperation::Count {
                count: self.input(key, orange_model::port_id!("count")),
                body: self.target(key, "body"),
                completed: self.target(key, "completed"),
            },
            ids::WHILE => {
                let condition = self.input(key, orange_model::port_id!("condition"));
                let max_iterations = node
                    .config
                    .get("max_iterations")
                    .and_then(|v| v.as_u64())
                    .filter(|n| *n > 0)
                    .ok_or(CompileError::ValidationIncomplete)?;
                ControlOperation::While {
                    condition,
                    body: self.target(key, "body"),
                    completed: self.target(key, "completed"),
                    max_iterations,
                }
            }
            ids::BREAK => ControlOperation::Break,
            ids::CONTINUE => ControlOperation::Continue,
            _ => unreachable!("admitted control"),
        })
    }

    fn switch(&mut self, key: ControlKey) -> Result<ControlOperation, CompileError> {
        let cx = self.cx;
        let options = cx.options;
        let id = key.1;
        let node = cx.node(key);
        let port = cx.port(key, "key")?;
        let ty = port_type(cx.snapshot, id, port)?;
        let invalid = || CompileError::InvalidLiteral {
            node: id,
            port: port_key(port),
        };
        if !matches!(
            ty,
            ValueShape::Bool
                | ValueShape::I32
                | ValueShape::I64
                | ValueShape::U32
                | ValueShape::U64
                | ValueShape::String
                | ValueShape::Enum(_)
        ) {
            return Err(CompileError::UnsupportedPort {
                node: id,
                port: port_key(port),
            });
        }
        let case: orange_model::id::PortId = orange_model::port_id!("case");
        let state = node
            .variadic
            .get(&case)
            .ok_or(CompileError::ValidationIncomplete)?;
        charge(
            self.graph_items,
            state.order.len(),
            options.limits.max_graph_items,
        )?;
        let raw = node
            .config
            .get("cases")
            .and_then(|v| v.as_object())
            .ok_or(CompileError::ValidationIncomplete)?;
        let mut cases = Vec::with_capacity(state.order.len());
        for instance in &state.order {
            options.checkpoint()?;
            let value = raw
                .get(&instance.to_string())
                .ok_or(CompileError::ValidationIncomplete)?;
            charge(
                self.bytes,
                value.as_str().map_or(64, |s| s.len().saturating_add(96)),
                options.limits.max_constant_bytes,
            )?;
            let case_port = orange_model::id::PortKey::new(case.clone(), Some(*instance));
            cases.push(SwitchCase {
                key: dispatch::case_key(&ty, value).ok_or_else(invalid)?,
                instance: *instance,
                target: self.edges.get(&(key.0, id, case_port)).copied(),
            });
        }
        cases.sort_unstable_by(|a, b| a.key.cmp(&b.key));
        if cases.windows(2).any(|p| p[0].key == p[1].key) {
            return Err(CompileError::ValidationIncomplete);
        }
        let default = self.target(key, "default");
        // Checked values carry only locked-schema tags, so an exhaustive enum switch
        // always matches; a partial one must route the rest to a connected default.
        if let ValueShape::Enum(shape) = &ty
            && cases.len() < shape.len()
            && default.is_none()
        {
            return Err(CompileError::ValidationIncomplete);
        }
        Ok(ControlOperation::Switch {
            key: self.input(key, orange_model::port_id!("key")),
            cases: cases.into(),
            default,
        })
    }
}

/// Read-only tables a host input source is resolved against.
struct HostTables<'a> {
    controls: &'a BTreeSet<ControlKey>,
    control_ix: &'a BTreeMap<ControlKey, ControlIx>,
    nodes: &'a [ControlNode],
    variables: &'a [ControlVariable],
    variable_nodes: &'a BTreeMap<NodeId, usize>,
    state_nodes: &'a BTreeMap<ControlKey, usize>,
}

/// Where the runtime supplies one host input of the data plan.
fn host_source(
    cx: &Cx<'_>,
    tables: &HostTables<'_>,
    data: &DataPlan,
    node: &DataNode,
) -> Result<HostSource, CompileError> {
    let calls = cx.calls;
    let scope = node.call_instance().map_or(0, |i| i + 1);
    if !tables.controls.contains(&(scope, node.node()))
        && let Some(&variable) = tables.variable_nodes.get(&node.node())
    {
        let local = tables.variables[variable].scope() == VarScope::Local;
        return Ok(match (node.call_instance(), local) {
            (Some(_), true) if calls.scopes[scope].control => {
                HostSource::FrameLocal { scope, variable }
            }
            (Some(_), true) => HostSource::CallLocal(variable),
            _ => HostSource::Variable(variable),
        });
    }
    let source = &data.slot_sources()[node.outputs()[0].index()];
    if cx.node((scope, source.node)).definition.id.as_str() == ids::NOW {
        return Ok(HostSource::Clock);
    }
    let key = (scope, source.node);
    let owner = tables
        .control_ix
        .get(&key)
        .copied()
        .ok_or(CompileError::ValidationIncomplete)?;
    let port = source.port.port().as_str();
    if calls.entry(scope) == Some(source.node) && scope != 0 && calls.scopes[scope].control {
        let index = cx.data_output(key, &source.port)?;
        return Ok(HostSource::Argument { scope, index });
    }
    let operation = tables.nodes[owner.index()].operation();
    if port == "payload"
        && matches!(
            cx.node((scope, source.node)).definition.id.as_str(),
            ids::ON_EVENT | ids::ON_MANUAL | ids::AWAIT_EVENT
        )
    {
        return Ok(HostSource::EventPayload(owner));
    }
    if matches!(
        cx.node((scope, source.node)).definition.id.as_str(),
        ids::ON_TIMER | ids::ON_TIMEOUT
    ) {
        return match port {
            "scheduled_at" => Ok(HostSource::Tick(owner, TickProjection::ScheduledAt)),
            "missed" => Ok(HostSource::Tick(owner, TickProjection::Missed)),
            _ => Err(CompileError::ValidationIncomplete),
        };
    }
    if cx.node((scope, source.node)).definition.id.as_str() == ids::ON_VARIABLE_CHANGED {
        return match port {
            "value" => Ok(HostSource::Change(owner, ChangeProjection::Value)),
            "previous" => Ok(HostSource::Change(owner, ChangeProjection::Previous)),
            _ => Err(CompileError::ValidationIncomplete),
        };
    }
    if matches!(operation, ControlOperation::Call { .. }) && port != "error" {
        return Ok(HostSource::CallOutput(
            owner,
            cx.data_output(key, &source.port)?,
        ));
    }
    if matches!(port, "is_open" | "is_a" | "current") {
        return tables
            .state_nodes
            .get(&key)
            .copied()
            .map(HostSource::State)
            .ok_or(CompileError::ValidationIncomplete);
    }
    let unsupported = CompileError::UnsupportedNode { node: source.node };
    match (operation, port) {
        (ControlOperation::ForEach { .. }, "index") => {
            Ok(HostSource::Iteration(owner, IterationProjection::Index))
        }
        (ControlOperation::ForEach { .. }, "element") => {
            Ok(HostSource::Iteration(owner, IterationProjection::Element))
        }
        (ControlOperation::ForEach { .. }, "key") => {
            Ok(HostSource::Iteration(owner, IterationProjection::Key))
        }
        (ControlOperation::ForEach { .. }, "value") => {
            Ok(HostSource::Iteration(owner, IterationProjection::Value))
        }
        (ControlOperation::Update { .. }, "old") => {
            Ok(HostSource::Update(owner, UpdateProjection::Old))
        }
        (ControlOperation::Update { .. }, "new") => {
            Ok(HostSource::Update(owner, UpdateProjection::New))
        }
        (ControlOperation::Update { .. }, "exchanged") => {
            Ok(HostSource::Update(owner, UpdateProjection::Exchanged))
        }
        (_, "error") => Ok(HostSource::Error(owner)),
        (ControlOperation::ForEach { .. } | ControlOperation::Update { .. }, _) => Err(unsupported),
        (ControlOperation::MatchResult { .. }, "value") => Ok(HostSource::Value(owner)),
        (_, "index") => Ok(HostSource::Index(owner)),
        _ => Err(unsupported),
    }
}

fn input_root(
    roots: &mut Vec<DemandRoot>,
    scope: usize,
    node: NodeId,
    port: orange_model::id::PortId,
) -> usize {
    let index = roots.len();
    roots.push(DemandRoot {
        scope,
        id: format!("{node}/{}", port.as_str()),
        node,
        port: orange_model::id::PortKey::new(port, None),
        input: true,
    });
    index
}
