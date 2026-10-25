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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlIx(pub(crate) usize);
impl ControlIx {
    pub fn index(self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone)]
pub enum ControlOperation {
    Entry,
    While {
        condition: usize,
        body: Option<ControlIx>,
        completed: Option<ControlIx>,
        max_iterations: u64,
    },
    Count {
        count: usize,
        body: Option<ControlIx>,
        completed: Option<ControlIx>,
    },
    Break,
    Continue,
    Yield,
    Wait {
        duration: usize,
    },
    Sequence {
        steps: Box<[Option<ControlIx>]>,
    },
    Set {
        variable: usize,
        value: usize,
    },
    Branch {
        condition: usize,
        when_true: Option<ControlIx>,
        when_false: Option<ControlIx>,
    },
}

pub struct ControlVariable {
    id: VariableId,
    scope: VarScope,
    ty: PlanType,
    initial: Option<Value>,
}
impl ControlVariable {
    pub fn id(&self) -> VariableId {
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
}

#[derive(Debug, Clone)]
pub struct ControlNode {
    pub(crate) node: NodeId,
    pub(crate) operation: ControlOperation,
    pub(crate) then: Option<ControlIx>,
    pub(crate) failed: Option<ControlIx>,
    pub(crate) cancelled: Option<ControlIx>,
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
    pub fn then(&self) -> Option<ControlIx> {
        self.then
    }
    pub fn failed(&self) -> Option<ControlIx> {
        self.failed
    }
    pub fn cancelled(&self) -> Option<ControlIx> {
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
pub struct ControlPlan {
    pub(crate) data: Arc<DataPlan>,
    pub(crate) nodes: Box<[ControlNode]>,
    pub(crate) starts: Box<[ControlIx]>,
    pub(crate) stops: Box<[ControlIx]>,
    pub(crate) variables: Box<[ControlVariable]>,
    pub(crate) host_sources: Box<[HostSource]>,
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
    /// Parallel to data().host_inputs(), resolved once outside execution.
    pub fn host_sources(&self) -> &[HostSource] {
        &self.host_sources
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
    if analysis.document_state() != document.state_id() {
        return Err(CompileError::StaleDocument);
    }
    if analysis.registry_generation() != snapshot.generation() {
        return Err(CompileError::StaleRegistry);
    }
    if !analysis.is_ready_for_lowering(document, &snapshot) {
        return Err(CompileError::ValidationIncomplete);
    }
    let mut controls = BTreeSet::new();
    for node in document.root().nodes.values() {
        let sig = &analysis.report().resolved[&node.id];
        if matches!(sig.kind, NodeKind::Pure | NodeKind::Volatile) {
            continue;
        }
        if !matches!(
            node.definition.id.as_str(),
            "core/on-start"
                | "core/on-stop"
                | "core/branch"
                | "core/set-variable"
                | "core/sequence"
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
    let mut adjacency = BTreeMap::new();
    for edge in document.root().edges.values() {
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
                adjacency.insert((edge.from.node, edge.from.port.clone()), ids[&edge.to.node]);
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
                let condition = roots.len();
                roots.push(DemandRoot {
                    id: format!("{id}/condition"),
                    node: *id,
                    port: "condition".parse().unwrap(),
                    input: true,
                });
                ControlOperation::Branch {
                    condition,
                    when_true: target("true"),
                    when_false: target("false"),
                }
            }
            "core/set-variable" => {
                let value = roots.len();
                roots.push(DemandRoot {
                    id: format!("{id}/value"),
                    node: *id,
                    port: "value".parse().unwrap(),
                    input: true,
                });
                ControlOperation::Set {
                    variable: variable_nodes[id],
                    value,
                }
            }
            "core/sequence" => {
                let state = node
                    .variadic
                    .get(&"then".parse().expect("fixed port"))
                    .ok_or(CompileError::ValidationIncomplete)?;
                let steps = state
                    .order
                    .iter()
                    .map(|k| {
                        adjacency
                            .get(&(
                                *id,
                                orange_model::id::PortKey::new("then".parse().unwrap(), Some(*k)),
                            ))
                            .copied()
                    })
                    .collect::<Vec<_>>();
                ControlOperation::Sequence {
                    steps: steps.into(),
                }
            }
            "core/yield" => ControlOperation::Yield,
            "core/wait" => {
                let duration = roots.len();
                roots.push(DemandRoot {
                    id: format!("{id}/duration"),
                    node: *id,
                    port: "duration".parse().unwrap(),
                    input: true,
                });
                ControlOperation::Wait { duration }
            }
            "core/count" => {
                let count = roots.len();
                roots.push(DemandRoot {
                    id: format!("{id}/count"),
                    node: *id,
                    port: "count".parse().unwrap(),
                    input: true,
                });
                ControlOperation::Count {
                    count,
                    body: target("body"),
                    completed: target("completed"),
                }
            }
            "core/while" => {
                let condition = roots.len();
                roots.push(DemandRoot {
                    id: format!("{id}/condition"),
                    node: *id,
                    port: "condition".parse().unwrap(),
                    input: true,
                });
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
                document
                    .root()
                    .edges
                    .values()
                    .find(|e| e.to.node == r.node && e.to.port == r.port)
                    .map(|e| e.from.node)
            } else {
                Some(r.node)
            }
        })
        .collect();
    let admission = admit_roots(
        document,
        &snapshot,
        analysis,
        options,
        true,
        &root_nodes,
        &controls,
    )?;
    let data = lower_roots(
        document, snapshot, analysis, resolver, options, admission, &roots,
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
            match source.port.port().as_str() {
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
    Ok(ControlPlan {
        data: Arc::new(data),
        nodes: nodes.into(),
        starts: starts.into(),
        stops: stops.into(),
        variables: variables.into(),
        host_sources: host_sources.into(),
    })
}
