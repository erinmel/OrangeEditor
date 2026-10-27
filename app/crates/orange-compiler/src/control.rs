//! Checked root control plans. The model owns language validation; this compiler
//! additionally admits only the executable vocabulary implemented by the runtime.
use crate::compile::{admit_roots, charge, decode, key as port_key, port_type, value_bytes};
use crate::data_compile::{DemandRoot, lower_roots};
use crate::*;
use orange_model::GraphDocument;
use orange_model::Value;
use orange_model::catalog::RegistrySnapshot;
use orange_model::catalog::intrinsic_id as ids;
use orange_model::document::{FaultPolicy, NodeKind, PortDirection, PortKind, VarScope};
use orange_model::id::{NodeId, VariableId};
use orange_model::validation::ValidationAnalysis;
use orange_model::value::shape::VALUE_CHARGE;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

mod dispatch;
mod update;
#[derive(Debug, Clone, Copy)]
pub enum IterationProjection {
    Index,
    Element,
    Key,
    Value,
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
    let mut bytes = 0;
    let (variables, variable_nodes) = collect_variables(&cx, &mut bytes)?;
    let mut functions = function_layouts(&cx, &control_ix, &variables, &variable_nodes)?;
    let (states, state_nodes) = collect_states(&cx, &controls, &mut bytes)?;
    let (edges, producers) = exec_edges(&cx, &controls, &control_ix)?;
    let mut lowering = NodeLowering {
        cx: &cx,
        edges: &edges,
        control_ix: &control_ix,
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
            | HostSource::Update(owner, _)
            | HostSource::CallOutput(owner, _) => value_targets[owner.index()].push(i),
            HostSource::Variable(_)
            | HostSource::CallLocal(_)
            | HostSource::FrameLocal { .. }
            | HostSource::Argument { .. }
            | HostSource::State(_)
            | HostSource::Iteration(_, _) => {}
        }
    }
    let mut projection_items = 0;
    let dependencies = host_dependencies(&data, options, &mut projection_items)?;
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
                port_type(node.id, port)?;
            }
        }
    }
    Ok(controls)
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
            ids::GET_VARIABLE | ids::SET_VARIABLE | ids::UPDATE_VARIABLE
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
            let ty = ValueShape::of(&declaration.ty)
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
                let ty = port_type(key.1, port)?;
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
    fn target(&self, (scope, id): ControlKey, name: &str) -> Option<ExecTarget> {
        self.edges
            .get(&(scope, id, name.parse().expect("fixed port")))
            .copied()
    }
    fn input(&mut self, (scope, id): ControlKey, port: &str) -> usize {
        input_root(&mut self.roots, scope, id, port)
    }
    /// Targets of a variadic exec output, in instance order.
    fn variadic_targets(
        &mut self,
        key: ControlKey,
        port: &str,
    ) -> Result<Vec<Option<ExecTarget>>, CompileError> {
        let cx = self.cx;
        let name: orange_model::id::PortId = port.parse().expect("fixed port");
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
            if port.is_optional() || port.is_lazy() || port.variadic.is_some() {
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
                    roots: ports.iter().map(|p| self.input(key, p.as_str())).collect(),
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
                    operands.push(self.input(key, "expected"));
                }
                operands.push(self.input(key, "value"));
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
                next: self.input(key, "next"),
            },
            ids::SWITCH => self.switch(key)?,
            ids::MATCH_RESULT => {
                let port = cx.port(key, "result")?;
                let ValueShape::Result(inner) = port_type(id, port)? else {
                    return Err(CompileError::ValidationIncomplete);
                };
                ControlOperation::MatchResult {
                    result: self.input(key, "result"),
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
            ids::BRANCH => ControlOperation::Branch {
                condition: self.input(key, "condition"),
                when_true: self.target(key, "true"),
                when_false: self.target(key, "false"),
            },
            ids::SET_VARIABLE => ControlOperation::Set {
                variable: self.variable_nodes[&id],
                value: self.input(key, "value"),
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
                duration: self.input(key, "duration"),
            },
            ids::FOR_EACH => ControlOperation::ForEach {
                items: self.input(key, "items"),
                body: self.target(key, "body"),
                completed: self.target(key, "completed"),
            },
            ids::COUNT => ControlOperation::Count {
                count: self.input(key, "count"),
                body: self.target(key, "body"),
                completed: self.target(key, "completed"),
            },
            ids::WHILE => {
                let condition = self.input(key, "condition");
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
        let ty = port_type(id, port)?;
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
        ) {
            return Err(CompileError::UnsupportedPort {
                node: id,
                port: port_key(port),
            });
        }
        let case: orange_model::id::PortId = "case".parse().expect("fixed port");
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
        Ok(ControlOperation::Switch {
            key: self.input(key, "key"),
            cases: cases.into(),
            default: self.target(key, "default"),
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

/// For each data node, the sorted host inputs its value transitively depends on.
fn host_dependencies(
    data: &DataPlan,
    options: CompileOptions<'_>,
    items: &mut usize,
) -> Result<Vec<Arc<[usize]>>, CompileError> {
    let host_indices: BTreeMap<_, _> = data
        .host_inputs()
        .iter()
        .enumerate()
        .map(|(i, n)| (n.index(), i))
        .collect();
    let mut dependencies: Vec<Arc<[usize]>> = Vec::with_capacity(data.nodes().len());
    for node in data.nodes() {
        options.checkpoint()?;
        let inputs = if let Some(index) = host_indices.get(&node.index().index()) {
            charge(items, 1, options.limits.max_graph_items)?;
            Arc::from([*index])
        } else {
            let producers = node
                .inputs()
                .iter()
                .filter_map(|i| i.producer)
                .map(|i| {
                    dependencies
                        .get(i.index())
                        .ok_or(CompileError::ValidationIncomplete)
                })
                .collect::<Result<Vec<_>, _>>()?;
            union_inputs(producers, options, items)?
        };
        dependencies.push(inputs);
    }
    Ok(dependencies)
}

/// Union of sorted host-input sets, sharing the input set when all are identical.
fn union_inputs<'s>(
    sets: impl IntoIterator<Item = &'s Arc<[usize]>>,
    options: CompileOptions<'_>,
    items: &mut usize,
) -> Result<Arc<[usize]>, CompileError> {
    let sets: Vec<_> = sets.into_iter().collect();
    let Some(first) = sets.first() else {
        return Ok(Arc::from([]));
    };
    if sets.iter().all(|s| s.as_ref() == first.as_ref()) {
        return Ok(Arc::clone(first));
    }
    let mut union = BTreeSet::new();
    for set in sets {
        for &index in set.iter() {
            options.checkpoint()?;
            if union.insert(index) {
                charge(items, 1, options.limits.max_graph_items)?;
            }
        }
    }
    Ok(union.into_iter().collect::<Vec<_>>().into())
}

fn input_root(roots: &mut Vec<DemandRoot>, scope: usize, node: NodeId, port: &str) -> usize {
    let index = roots.len();
    roots.push(DemandRoot {
        scope,
        id: format!("{node}/{port}"),
        node,
        port: port.parse().expect("fixed input port"),
        input: true,
    });
    index
}
