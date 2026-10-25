//! Checked root control plans. The model owns language validation; this compiler
//! additionally admits only the executable vocabulary implemented by the runtime.
use crate::compile::{admit_roots, key, port_type};
use crate::data_compile::{DemandRoot, lower_roots};
use crate::*;
use orange_model::GraphDocument;
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{FaultPolicy, NodeKind, PortDirection, PortKind};
use orange_model::id::NodeId;
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
    Branch {
        condition: usize,
        when_true: Option<ControlIx>,
        when_false: Option<ControlIx>,
    },
}

#[derive(Debug, Clone)]
pub struct ControlNode {
    pub(crate) node: NodeId,
    pub(crate) operation: ControlOperation,
    pub(crate) then: Option<ControlIx>,
    pub(crate) failed: Option<ControlIx>,
    pub(crate) cancelled: Option<ControlIx>,
    pub(crate) fault_policy: FaultPolicy,
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
}

/// Private checked construction, no serde/default bypass. Debug omits payloads.
pub struct ControlPlan {
    pub(crate) data: Arc<DataPlan>,
    pub(crate) nodes: Box<[ControlNode]>,
    pub(crate) starts: Box<[ControlIx]>,
    pub(crate) stops: Box<[ControlIx]>,
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
            "core/on-start" | "core/on-stop" | "core/branch"
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
                    .any(|p| key(p) == edge.from.port && p.kind == PortKind::Exec)
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
    Ok(ControlPlan {
        data: Arc::new(data),
        nodes: nodes.into(),
        starts: starts.into(),
        stops: stops.into(),
    })
}
