//! Immutable root data plans. Execution and epoch ownership live in orange-runtime.
use crate::compile::{valid_fault, value_bytes};
use crate::*;
use orange_model::catalog::RegistrySnapshot;
use orange_model::id::{GraphId, NodeId};
use orange_model::validation::Diagnostic;
use orange_model::{GraphDocument, Value};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataStability {
    Const,
    Epoch,
}

/// Numeric operand positions refer to DataNode.inputs, never names at execution.
#[derive(Debug, Clone)]
pub enum DataOperation {
    Constant,
    Pure(KernelBinding),
    Entropy(EntropyBinding),
    /// Host-supplied value (DataPlan::host_inputs): a state read today, control-plane
    /// outputs later. The data plane never evaluates it, only reads its slot.
    Input,
    /// Strict argument prefix followed by all declared results. First faults in
    /// either collection propagate to every output. No provider callback runs.
    Call {
        arguments: usize,
    },
    Select {
        when_true: usize,
        when_false: usize,
    },
    And,
    Or,
    Coalesce,
    Try {
        boxed: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataInput {
    pub source: InputSource,
    pub producer: Option<NodeIx>,
}

#[derive(Debug, Clone)]
pub struct DataNode {
    pub(crate) call: Option<usize>,
    pub(crate) index: NodeIx,
    pub(crate) node: NodeId,
    pub(crate) stability: DataStability,
    pub(crate) inputs: Box<[DataInput]>,
    pub(crate) outputs: Box<[SlotIx]>,
    pub(crate) operation: DataOperation,
}
impl DataNode {
    /// Index into DataPlan::call_sites; None denotes the root body.
    pub fn call_instance(&self) -> Option<usize> {
        self.call
    }

    pub fn index(&self) -> NodeIx {
        self.index
    }
    pub fn node(&self) -> NodeId {
        self.node
    }
    pub fn stability(&self) -> DataStability {
        self.stability
    }
    pub fn inputs(&self) -> &[DataInput] {
        &self.inputs
    }
    pub fn outputs(&self) -> &[SlotIx] {
        &self.outputs
    }
    pub fn operation(&self) -> &DataOperation {
        &self.operation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataResources {
    pub slots: usize,
    pub nodes: usize,
    pub max_inputs: usize,
    pub max_outputs: usize,
    pub work_items: usize,
    pub entropy_nodes: usize,
}

/// Checked pure/seeded-entropy root exports. No default, public constructor or
/// deserialization can bypass lowering. Includes bounded graph-local Pure/seeded-volatile calls
/// with epoch-dependent arguments; control ownership remains in ControlPlan/Run.
/// Debug reports metadata/counts without values, registry contents or callbacks.
///
/// ```compile_fail
/// let plan = orange_compiler::DataPlan::default();
/// ```
pub struct DataPlan {
    pub(crate) snapshot: Arc<RegistrySnapshot>,
    pub(crate) graph: GraphId,
    pub(crate) revision: u64,
    pub(crate) state: u64,
    pub(crate) calls: Vec<PureCallSite>,
    pub(crate) slot_calls: Vec<Option<usize>>,
    pub(crate) nodes: Vec<DataNode>,
    pub(crate) types: Vec<PlanType>,
    pub(crate) sources: Vec<SlotSource>,
    pub(crate) initial: Vec<Option<FoldedValue>>,
    pub(crate) exports: Vec<Export>,
    pub(crate) export_nodes: Vec<NodeIx>,
    pub(crate) host_inputs: Vec<NodeIx>,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) resources: DataResources,
}
impl std::fmt::Debug for DataPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataPlan")
            .field("graph", &self.graph)
            .field("revision", &self.revision)
            .field("registry_generation", &self.snapshot.generation())
            .field("resources", &self.resources)
            .field("exports", &self.exports.len())
            .field("diagnostics", &self.diagnostics.len())
            .finish_non_exhaustive()
    }
}
impl DataPlan {
    pub fn call_sites(&self) -> &[PureCallSite] {
        &self.calls
    }
    /// Compilation source scope, not a runtime entropy frame identity.
    pub fn slot_call_instance(&self, slot: SlotIx) -> Option<usize> {
        self.slot_calls.get(slot.index()).copied().flatten()
    }

    pub fn snapshot(&self) -> &Arc<RegistrySnapshot> {
        &self.snapshot
    }
    pub fn graph_id(&self) -> GraphId {
        self.graph
    }
    pub fn source_revision(&self) -> u64 {
        self.revision
    }
    pub fn is_current(&self, document: &GraphDocument, registry: &RegistrySnapshot) -> bool {
        self.state == document.semantic_state_id()
            && self.snapshot.generation() == registry.generation()
    }
    pub fn nodes(&self) -> &[DataNode] {
        &self.nodes
    }
    pub fn slot_types(&self) -> &[PlanType] {
        &self.types
    }
    pub fn slot_sources(&self) -> &[SlotSource] {
        &self.sources
    }
    /// None is a dynamic slot awaiting execution. A language None is instead
    /// Some(FoldedValue::Value(Value::None)).
    pub fn initial_values(&self) -> &[Option<FoldedValue>] {
        &self.initial
    }
    pub fn exports(&self) -> &[Export] {
        &self.exports
    }
    pub fn export_node(&self, index: usize) -> Option<NodeIx> {
        self.export_nodes.get(index).copied()
    }
    /// Input nodes in plan order. Only inputs some export may demand are present.
    pub fn host_inputs(&self) -> &[NodeIx] {
        &self.host_inputs
    }
    pub fn initial_charge(&self) -> Option<usize> {
        let mut bytes = 0usize;
        for (index, value) in self.initial.iter().enumerate() {
            let amount = match value {
                Some(FoldedValue::Value(value)) => {
                    self.value_charge(SlotIx(u32::try_from(index).ok()?), value)?
                }
                Some(FoldedValue::Fault(fault)) => self.fault_charge(fault)?,
                None => 0,
            };
            bytes = bytes.checked_add(amount)?;
        }
        Some(bytes)
    }
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
    pub fn resources(&self) -> DataResources {
        self.resources
    }
    /// Check this plan's bounded resolved type before computing a retained charge.
    /// None means wrong slot/type; no arbitrary type tree is trusted.
    pub fn value_charge(&self, slot: SlotIx, value: &Value) -> Option<usize> {
        self.types
            .get(slot.index())
            .filter(|ty| ty.accepts(value))
            .map(|_| value_bytes(value))
    }
    pub fn fault_charge(&self, fault: &ConstantFault) -> Option<usize> {
        valid_fault(fault.fault).then_some(64)
    }
}
