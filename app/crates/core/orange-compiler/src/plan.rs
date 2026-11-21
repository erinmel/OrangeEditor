use crate::{KernelBinding, KernelFault, ValueShape};
use orange_model::catalog::RegistrySnapshot;
use orange_model::id::{EdgeId, GraphId, NodeId, PortKey};
use orange_model::validation::Diagnostic;
use orange_model::{GraphDocument, Value};
use std::sync::Arc;

/// Dense plan-local indices. Constructors stay inside the checked compiler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeIx(pub(crate) u32);
impl NodeIx {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotIx(pub(crate) u32);
impl SlotIx {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotSource {
    pub node: NodeId,
    pub port: PortKey,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSource {
    pub port: PortKey,
    pub slot: SlotIx,
    pub edge: Option<EdgeId>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstantFault {
    pub source: NodeId,
    pub fault: KernelFault,
}
#[derive(Debug, Clone, PartialEq)]
pub enum FoldedValue {
    Value(Value),
    Fault(ConstantFault),
}

#[derive(Debug, Clone)]
pub struct PureStep {
    pub(crate) index: NodeIx,
    pub(crate) node: NodeId,
    pub(crate) inputs: Box<[InputSource]>,
    pub(crate) outputs: Box<[SlotIx]>,
    pub(crate) binding: KernelBinding,
    pub(crate) call: Option<usize>,
}
impl PureStep {
    /// Index into PurePlan::call_sites; None denotes the root body.
    pub fn call_instance(&self) -> Option<usize> {
        self.call
    }
    pub fn index(&self) -> NodeIx {
        self.index
    }
    pub fn node(&self) -> NodeId {
        self.node
    }
    /// Actual demanded inputs in declaration order; inactive lazy ports are absent.
    pub fn inputs(&self) -> &[InputSource] {
        &self.inputs
    }
    pub fn outputs(&self) -> &[SlotIx] {
        &self.outputs
    }
    /// Binding specialized to this immutable constant step. Compiler-owned lazy
    /// projections are not reusable dynamic branch instructions.
    pub fn binding(&self) -> &KernelBinding {
        &self.binding
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Export {
    pub id: String,
    pub slot: SlotIx,
}

/// One specialized potential call instance. PurePlan uses Pure calls; DataPlan
/// also admits the supported seeded-volatile subset. Parent indexes the same table;
/// None denotes a root call. Unselected lazy arms may have no executed steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PureCallSite {
    pub call: NodeId,
    pub function: orange_model::id::FunctionId,
    pub parent: Option<usize>,
}

/// Checked restricted pure-export plan, with a pinned registry and original source IDs.
/// Includes bounded graph-local pure calls; it does not authorize control, volatile,
/// action/async calls or dynamic epoch execution.
/// Debug reports metadata/counts without values, registry contents or callbacks.
///
/// ```compile_fail
/// let plan = orange_compiler::PurePlan::default();
/// ```
#[derive(Clone)]
pub struct PurePlan {
    pub(crate) snapshot: Arc<RegistrySnapshot>,
    pub(crate) graph: GraphId,
    pub(crate) revision: u64,
    pub(crate) state: u64,
    pub(crate) steps: Vec<PureStep>,
    pub(crate) types: Vec<ValueShape>,
    pub(crate) sources: Vec<SlotSource>,
    pub(crate) constants: Vec<FoldedValue>,
    pub(crate) exports: Vec<Export>,
    pub(crate) diagnostics: Vec<Diagnostic>,
    pub(crate) calls: Vec<PureCallSite>,
    pub(crate) slot_calls: Vec<Option<usize>>,
}
impl std::fmt::Debug for PurePlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PurePlan")
            .field("graph", &self.graph)
            .field("revision", &self.revision)
            .field("registry_generation", &self.snapshot.generation())
            .field("steps", &self.steps.len())
            .field("slots", &self.constants.len())
            .field("exports", &self.exports.len())
            .field("diagnostics", &self.diagnostics.len())
            .finish_non_exhaustive()
    }
}
impl PurePlan {
    pub fn call_sites(&self) -> &[PureCallSite] {
        &self.calls
    }
    /// Original producer's call instance; aliases retain source ownership.
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
    pub fn is_current(&self, doc: &GraphDocument, snapshot: &RegistrySnapshot) -> bool {
        self.state == doc.semantic_state_id() && self.snapshot.generation() == snapshot.generation()
    }
    pub fn steps(&self) -> &[PureStep] {
        &self.steps
    }
    pub fn slot_types(&self) -> &[ValueShape] {
        &self.types
    }
    pub fn slot_sources(&self) -> &[SlotSource] {
        &self.sources
    }
    pub fn constants(&self) -> &[FoldedValue] {
        &self.constants
    }
    pub fn exports(&self) -> &[Export] {
        &self.exports
    }
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
}
