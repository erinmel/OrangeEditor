//! Checked root control plans. The model owns language validation; this compiler
//! additionally admits only the executable vocabulary implemented by the runtime.
use crate::compile::{admit_roots, charge, decode, key, port_type, value_bytes};
use crate::data_compile::{DemandRoot, lower_roots};
use crate::*;
use orange_model::GraphDocument;
use orange_model::Value;
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{FaultPolicy, NodeKind, PortDirection, PortKind, VarScope};
use orange_model::id::{NodeId, VariableId};
use orange_model::validation::ValidationAnalysis;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

mod dispatch;
pub use dispatch::{SwitchCase, SwitchKey};

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
        ("core/gate", "in" | "enter") => ExecInput::Enter,
        ("core/gate", "open") => ExecInput::Open,
        ("core/gate", "close") => ExecInput::Close,
        ("core/gate", "toggle") => ExecInput::Toggle,
        ("core/do-once" | "core/register", "reset") => ExecInput::Reset,
        ("core/register", "in" | "store") => ExecInput::Store,
        (_, "in") => ExecInput::In,
        _ => return None,
    };
    Some(ExecTarget { node, input })
}

#[derive(Debug, Clone)]
pub enum ControlOperation {
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
    ty: PlanType,
    initial: Option<Value>,
}
impl<Id: Copy> ControlCell<Id> {
    pub fn id(&self) -> Id {
        self.id
    }
    pub fn scope(&self) -> VarScope {
        self.scope
    }
    pub fn ty(&self) -> &PlanType {
        &self.ty
    }
    pub fn initial(&self) -> Option<&Value> {
        self.initial.as_ref()
    }
    pub fn charge(&self, value: &Value) -> Option<usize> {
        self.ty.accepts(value).then(|| value_bytes(value))
    }
}
#[derive(Debug, Clone, Copy)]
pub enum HostSource {
    Variable(usize),
    Error(ControlIx),
    Index(ControlIx),
    Value(ControlIx),
    State(usize),
}

#[derive(Debug, Clone)]
pub struct ControlNode {
    pub(crate) node: NodeId,
    pub(crate) operation: ControlOperation,
    pub(crate) then: Option<ExecTarget>,
    pub(crate) failed: Option<ExecTarget>,
    pub(crate) cancelled: Option<ExecTarget>,
    pub(crate) fault_policy: FaultPolicy,
    pub(crate) loop_owner: Option<ControlIx>,
}
impl ControlNode {
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

/// Private checked construction, no serde/default bypass. Debug omits payloads.
///
/// ```compile_fail
/// let plan = orange_compiler::ControlPlan::default();
/// ```
pub struct ControlPlan {
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
    let mut controls = BTreeSet::new();
    for node in document.root().nodes.values() {
        let sig = &analysis.report().resolved[&node.id];
        if matches!(sig.kind, NodeKind::Pure | NodeKind::Volatile) {
            continue;
        }
        if !matches!(
            node.definition.id.as_str(),
            "core/gate"
                | "core/do-once"
                | "core/flip-flop"
                | "core/register"
                | "core/switch"
                | "core/match-result"
                | "core/on-start"
                | "core/on-stop"
                | "core/branch"
                | "core/set-variable"
                | "core/sequence"
                | "core/fork"
                | "core/yield"
                | "core/wait"
                | "core/count"
                | "core/break"
                | "core/continue"
                | "core/while"
        ) || node.definition.version != 1
            || node.config_version != 1
            || node.provider.is_some()
        {
            return Err(CompileError::UnsupportedNode { node: node.id });
        }
        controls.insert(node.id);
        for port in sig.ports.iter().filter(|p| p.kind == PortKind::Data) {
            port_type(node.id, port)?;
        }
    }
    let ids: BTreeMap<_, _> = controls
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, ControlIx(i)))
        .collect();
    let mut variables = Vec::new();
    let mut variable_ids = BTreeMap::new();
    let mut variable_nodes = BTreeMap::new();
    let mut bytes = 0;
    for node in document.root().nodes.values() {
        options.checkpoint()?;
        if crate::compile::is_state_read(&snapshot, node)
            && node.definition.id.as_str() != "core/get-variable"
        {
            return Err(CompileError::UnsupportedNode { node: node.id });
        }
        if !matches!(
            node.definition.id.as_str(),
            "core/get-variable" | "core/set-variable"
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
            let ty = PlanType::parse(declaration.ty.as_str())
                .ok_or(CompileError::UnsupportedNode { node: node.id })?;
            let initial = if let Some(literal) = &declaration.init {
                Some(
                    decode(
                        literal,
                        &ty,
                        &snapshot,
                        options.limits.max_constant_bytes.saturating_sub(bytes),
                    )?
                    .ok_or(CompileError::UnsupportedNode { node: node.id })?,
                )
            } else {
                None
            };
            charge(
                &mut bytes,
                initial.as_ref().map_or(16, value_bytes),
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
        variable_nodes.insert(node.id, variable_ids[&id]);
    }
    let mut states = Vec::new();
    let mut state_nodes = BTreeMap::new();
    for id in &controls {
        options.checkpoint()?;
        let node = &document.root().nodes[id];
        let (scope, ty, initial) = match node.definition.id.as_str() {
            "core/gate" => {
                let open = match node.config.get("start_open") {
                    None => false,
                    Some(value) => value.as_bool().ok_or(CompileError::ValidationIncomplete)?,
                };
                (VarScope::Run, PlanType::Bool, Some(Value::Bool(open)))
            }
            "core/do-once" | "core/flip-flop" => {
                (VarScope::Run, PlanType::Bool, Some(Value::Bool(false)))
            }
            "core/register" => {
                let port = analysis.report().resolved[id]
                    .ports
                    .iter()
                    .find(|p| p.port_id.as_str() == "current")
                    .ok_or(CompileError::ValidationIncomplete)?;
                let ty = port_type(*id, port)?;
                let scope = match node.config.get("scope").and_then(|s| s.as_str()) {
                    None | Some("run") => VarScope::Run,
                    Some("activation") => VarScope::Local,
                    _ => return Err(CompileError::ValidationIncomplete),
                };
                let initial = if let Some(raw) = node.config.get("initial") {
                    let mut admitted = bytes;
                    crate::calls::charge_json(raw, &mut admitted, options)?;
                    let literal = serde_json::from_value(raw.clone()).map_err(|_| {
                        CompileError::InvalidLiteral {
                            node: *id,
                            port: key(port),
                        }
                    })?;
                    Some(
                        decode(
                            &literal,
                            &ty,
                            &snapshot,
                            options.limits.max_constant_bytes.saturating_sub(bytes),
                        )?
                        .ok_or_else(|| CompileError::InvalidLiteral {
                            node: *id,
                            port: key(port),
                        })?,
                    )
                } else {
                    None
                };
                (scope, ty, initial)
            }
            _ => continue,
        };
        charge(
            &mut bytes,
            initial.as_ref().map_or(16, value_bytes),
            options.limits.max_constant_bytes,
        )?;
        state_nodes.insert(*id, states.len());
        states.push(ControlState {
            id: *id,
            scope,
            ty,
            initial,
        });
    }
    let mut adjacency = BTreeMap::new();
    let mut producers = BTreeMap::new();
    for edge in document.root().edges.values() {
        producers.insert((edge.to.node, edge.to.port.clone()), edge.from.node);
        if controls.contains(&edge.to.node) && controls.contains(&edge.from.node) {
            let sig = &analysis.report().resolved[&edge.from.node];
            if (sig
                .implicit_ports()
                .contains(&edge.from.port.port().as_str())
                && edge.from.port.port().as_str() != "error")
                || sig
                    .ports
                    .iter()
                    .any(|p| p.port_id == *edge.from.port.port() && p.kind == PortKind::Exec)
            {
                let to = destination(
                    ids[&edge.to.node],
                    document.root().nodes[&edge.to.node].definition.id.as_str(),
                    &edge.to.port,
                )
                .ok_or_else(|| CompileError::UnsupportedPort {
                    node: edge.to.node,
                    port: edge.to.port.clone(),
                })?;
                adjacency.insert((edge.from.node, edge.from.port.clone()), to);
            }
        }
    }
    let mut roots = Vec::new();
    let mut nodes = Vec::new();
    let mut starts = Vec::new();
    let mut stops = Vec::new();
    for id in &controls {
        options.checkpoint()?;
        let node = &document.root().nodes[id];
        let target = |name: &str| {
            adjacency
                .get(&(*id, name.parse().expect("fixed port")))
                .copied()
        };
        let operation = match node.definition.id.as_str() {
            "core/gate" => ControlOperation::Gate {
                state: state_nodes[id],
            },
            "core/do-once" => ControlOperation::DoOnce {
                state: state_nodes[id],
            },
            "core/flip-flop" => ControlOperation::FlipFlop {
                state: state_nodes[id],
                a: target("a"),
                b: target("b"),
            },
            "core/register" => ControlOperation::Register {
                state: state_nodes[id],
                next: input_root(&mut roots, *id, "next"),
            },
            "core/switch" => {
                let port = analysis.report().resolved[id]
                    .ports
                    .iter()
                    .find(|p| p.port_id.as_str() == "key")
                    .ok_or(CompileError::ValidationIncomplete)?;
                let ty = port_type(*id, port)?;
                if !matches!(
                    ty,
                    PlanType::Bool
                        | PlanType::I32
                        | PlanType::I64
                        | PlanType::U32
                        | PlanType::U64
                        | PlanType::String
                ) {
                    return Err(CompileError::UnsupportedPort {
                        node: *id,
                        port: key(port),
                    });
                }
                let state = node
                    .variadic
                    .get(&"case".parse().expect("fixed port"))
                    .ok_or(CompileError::ValidationIncomplete)?;
                charge(
                    &mut graph_items,
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
                        &mut bytes,
                        value.as_str().map_or(64, |s| s.len().saturating_add(96)),
                        options.limits.max_constant_bytes,
                    )?;
                    let key = dispatch::case_key(&ty, value).ok_or_else(|| {
                        CompileError::InvalidLiteral {
                            node: *id,
                            port: key(port),
                        }
                    })?;
                    let case_port = orange_model::id::PortKey::new(
                        "case".parse().expect("fixed port"),
                        Some(*instance),
                    );
                    cases.push(SwitchCase {
                        key,
                        instance: *instance,
                        target: adjacency.get(&(*id, case_port)).copied(),
                    });
                }
                cases.sort_unstable_by(|a, b| a.key.cmp(&b.key));
                if cases.windows(2).any(|p| p[0].key == p[1].key) {
                    return Err(CompileError::ValidationIncomplete);
                }
                ControlOperation::Switch {
                    key: input_root(&mut roots, *id, "key"),
                    cases: cases.into(),
                    default: target("default"),
                }
            }
            "core/match-result" => {
                let port = analysis.report().resolved[id]
                    .ports
                    .iter()
                    .find(|p| p.port_id.as_str() == "result")
                    .ok_or(CompileError::ValidationIncomplete)?;
                let PlanType::Result(inner) = port_type(*id, port)? else {
                    return Err(CompileError::ValidationIncomplete);
                };
                ControlOperation::MatchResult {
                    result: input_root(&mut roots, *id, "result"),
                    boxed: inner.unboxed_error(),
                    ok: target("ok"),
                    err: target("err"),
                }
            }

            "core/on-start" | "core/on-stop" => {
                let entries = if node.definition.id.as_str() == "core/on-start" {
                    &mut starts
                } else {
                    &mut stops
                };
                entries.push(ids[id]);
                ControlOperation::Entry
            }
            "core/branch" => {
                let condition = input_root(&mut roots, *id, "condition");
                ControlOperation::Branch {
                    condition,
                    when_true: target("true"),
                    when_false: target("false"),
                }
            }
            "core/set-variable" => {
                let value = input_root(&mut roots, *id, "value");
                ControlOperation::Set {
                    variable: variable_nodes[id],
                    value,
                }
            }
            "core/sequence" | "core/fork" => {
                let fork = node.definition.id.as_str() == "core/fork";
                let port = if fork { "branch" } else { "then" };
                let state = node
                    .variadic
                    .get(&port.parse().expect("fixed port"))
                    .ok_or(CompileError::ValidationIncomplete)?;
                let steps = state
                    .order
                    .iter()
                    .map(|k| {
                        adjacency
                            .get(&(
                                *id,
                                orange_model::id::PortKey::new(port.parse().unwrap(), Some(*k)),
                            ))
                            .copied()
                    })
                    .collect::<Vec<_>>();
                if fork {
                    let join = match node.config.get("join").and_then(|j| j.as_str()) {
                        Some("all") => ForkJoin::All,
                        Some("any") => ForkJoin::Any,
                        Some("race") => ForkJoin::Race,
                        Some("detach") => ForkJoin::Detach,
                        _ => return Err(CompileError::UnsupportedNode { node: *id }),
                    };
                    ControlOperation::Fork {
                        branches: steps.into(),
                        completed: target("completed"),
                        join,
                    }
                } else {
                    ControlOperation::Sequence {
                        steps: steps.into(),
                    }
                }
            }
            "core/yield" => ControlOperation::Yield,
            "core/wait" => {
                let duration = input_root(&mut roots, *id, "duration");
                ControlOperation::Wait { duration }
            }
            "core/count" => {
                let count = input_root(&mut roots, *id, "count");
                ControlOperation::Count {
                    count,
                    body: target("body"),
                    completed: target("completed"),
                }
            }
            "core/while" => {
                let condition = input_root(&mut roots, *id, "condition");
                let max_iterations = node
                    .config
                    .get("max_iterations")
                    .and_then(|v| v.as_u64())
                    .filter(|n| *n > 0)
                    .ok_or(CompileError::ValidationIncomplete)?;
                ControlOperation::While {
                    condition,
                    body: target("body"),
                    completed: target("completed"),
                    max_iterations,
                }
            }
            "core/break" => ControlOperation::Break,
            "core/continue" => ControlOperation::Continue,
            _ => unreachable!("admitted control"),
        };
        for port in analysis.report().resolved[id]
            .ports
            .iter()
            .filter(|p| p.kind == PortKind::Data && p.direction == PortDirection::Input)
        {
            if port.is_optional() || port.is_lazy() || port.variadic.is_some() {
                return Err(CompileError::UnsupportedPort {
                    node: *id,
                    port: key(port),
                });
            }
        }
        nodes.push(ControlNode {
            node: *id,
            operation,
            then: target("then"),
            failed: target("failed"),
            cancelled: target("cancelled"),
            fault_policy: node
                .activation
                .as_ref()
                .map_or(FaultPolicy::Continue, |a| a.on_fault),
            loop_owner: analysis
                .report()
                .regions
                .loop_of(*id)
                .and_then(|r| ids.get(&r.node))
                .copied(),
        });
    }
    starts.sort_by_key(|i| {
        (
            document.root().nodes[&nodes[i.0].node]
                .activation
                .as_ref()
                .map_or(0, |a| a.order),
            nodes[i.0].node,
        )
    });
    stops.sort_by_key(|i| {
        (
            document.root().nodes[&nodes[i.0].node]
                .activation
                .as_ref()
                .map_or(0, |a| a.order),
            nodes[i.0].node,
        )
    });
    for export in document.exports() {
        roots.push(DemandRoot {
            id: export.id.clone(),
            node: export.node,
            port: export.port.clone(),
            input: false,
        });
    }
    let root_nodes: Vec<_> = roots
        .iter()
        .filter_map(|r| {
            if r.input {
                producers.get(&(r.node, r.port.clone())).copied()
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
    let admission = admit_roots(
        document,
        &snapshot,
        analysis,
        data_options,
        true,
        &root_nodes,
        &controls,
    )?;
    let data = lower_roots(
        document,
        snapshot,
        analysis,
        resolver,
        data_options,
        admission,
        &roots,
    )?;
    let host_sources = data
        .host_inputs()
        .iter()
        .map(|ix| {
            let node = &data.nodes()[ix.index()];
            if document.root().nodes[&node.node()].definition.id.as_str() == "core/get-variable"
                && let Some(variable) = variable_nodes.get(&node.node())
            {
                return Ok(HostSource::Variable(*variable));
            }
            let source = &data.slot_sources()[node.outputs()[0].index()];
            if matches!(source.port.port().as_str(), "is_open" | "is_a" | "current") {
                return state_nodes
                    .get(&source.node)
                    .copied()
                    .map(HostSource::State)
                    .ok_or(CompileError::ValidationIncomplete);
            }
            match source.port.port().as_str() {
                "value"
                    if matches!(
                        nodes[ids[&source.node].index()].operation(),
                        ControlOperation::MatchResult { .. }
                    ) =>
                {
                    Ok(HostSource::Value(ids[&source.node]))
                }
                "index" => ids
                    .get(&source.node)
                    .copied()
                    .map(HostSource::Index)
                    .ok_or(CompileError::ValidationIncomplete),
                "error" => ids
                    .get(&source.node)
                    .copied()
                    .map(HostSource::Error)
                    .ok_or(CompileError::ValidationIncomplete),
                _ => Err(CompileError::UnsupportedNode { node: source.node }),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut index_targets = vec![Vec::new(); nodes.len()];
    let mut error_targets = vec![Vec::new(); nodes.len()];
    let mut value_targets = vec![Vec::new(); nodes.len()];
    let host_indices: BTreeMap<_, _> = data
        .host_inputs()
        .iter()
        .enumerate()
        .map(|(i, n)| (n.index(), i))
        .collect();
    for (i, source) in host_sources.iter().enumerate() {
        match source {
            HostSource::Index(owner) => index_targets[owner.index()].push(i),
            HostSource::Error(owner) => error_targets[owner.index()].push(i),
            HostSource::Value(owner) => value_targets[owner.index()].push(i),
            HostSource::Variable(_) | HostSource::State(_) => {}
        }
    }
    let empty: Arc<[usize]> = Arc::from([]);
    let mut dependencies: Vec<Arc<[usize]>> = Vec::with_capacity(data.nodes().len());
    let mut projection_items = 0;
    for node in data.nodes() {
        options.checkpoint()?;
        let inputs = if let Some(index) = host_indices.get(&node.index().index()) {
            charge(&mut projection_items, 1, options.limits.max_graph_items)?;
            Arc::from([*index])
        } else {
            let producers: Vec<_> = node
                .inputs()
                .iter()
                .filter_map(|i| i.producer)
                .map(|i| {
                    dependencies
                        .get(i.index())
                        .cloned()
                        .ok_or(CompileError::ValidationIncomplete)
                })
                .collect::<Result<_, _>>()?;
            if let Some(first) = producers
                .first()
                .filter(|first| producers.iter().all(|p| p.as_ref() == first.as_ref()))
            {
                first.clone()
            } else if producers.is_empty() {
                empty.clone()
            } else {
                let mut union = BTreeSet::new();
                for producer in producers {
                    for index in producer.iter().copied() {
                        options.checkpoint()?;
                        if union.insert(index) {
                            charge(&mut projection_items, 1, options.limits.max_graph_items)?;
                        }
                    }
                }
                Arc::from(union.into_iter().collect::<Vec<_>>())
            }
        };
        dependencies.push(inputs);
    }
    let root_inputs = (0..data.exports().len())
        .map(|root| {
            data.export_node(root)
                .and_then(|n| dependencies.get(n.index()))
                .cloned()
                .ok_or(CompileError::ValidationIncomplete)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ControlPlan {
        data: Arc::new(data),
        nodes: nodes.into(),
        starts: starts.into(),
        stops: stops.into(),
        variables: variables.into(),
        states: states.into(),
        host_sources: host_sources.into(),
        root_inputs: root_inputs.into(),
        value_targets: value_targets
            .into_iter()
            .map(Vec::into_boxed_slice)
            .collect(),
        index_targets: index_targets
            .into_iter()
            .map(Vec::into_boxed_slice)
            .collect(),
        error_targets: error_targets
            .into_iter()
            .map(Vec::into_boxed_slice)
            .collect(),
    })
}

fn input_root(roots: &mut Vec<DemandRoot>, node: NodeId, port: &str) -> usize {
    let index = roots.len();
    roots.push(DemandRoot {
        id: format!("{node}/{port}"),
        node,
        port: port.parse().expect("fixed input port"),
        input: true,
    });
    index
}
