use crate::{KernelBinding, KernelFault};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanType {
    Bool,
    I32,
    I64,
    U32,
    U64,
    F32,
    F64,
    Json,
}
impl PlanType {
    pub(crate) fn parse(ty: &str) -> Option<Self> {
        Some(match ty {
            "core/bool@1" => Self::Bool,
            "core/i32@1" => Self::I32,
            "core/i64@1" => Self::I64,
            "core/u32@1" => Self::U32,
            "core/u64@1" => Self::U64,
            "core/f32@1" => Self::F32,
            "core/f64@1" => Self::F64,
            "core/json@1" => Self::Json,
            _ => return None,
        })
    }
    pub(crate) fn accepts(self, value: &Value) -> bool {
        matches!(
            (self, value),
            (Self::Bool, Value::Bool(_))
                | (Self::I32, Value::I32(_))
                | (Self::I64, Value::I64(_))
                | (Self::U32, Value::U32(_))
                | (Self::U64, Value::U64(_))
                | (Self::F32, Value::F32(_))
                | (Self::F64, Value::F64(_))
                | (Self::Json, Value::Json(_))
        )
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
}
impl PureStep {
    pub fn index(&self) -> NodeIx {
        self.index
    }
    pub fn node(&self) -> NodeId {
        self.node
    }
    pub fn inputs(&self) -> &[InputSource] {
        &self.inputs
    }
    pub fn outputs(&self) -> &[SlotIx] {
        &self.outputs
    }
    pub fn binding(&self) -> &KernelBinding {
        &self.binding
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Export {
    pub id: String,
    pub slot: SlotIx,
}

/// Checked restricted pure-export plan, with a pinned registry and original source IDs.
/// It is not approval for control, lazy, volatile or function execution.
///
/// ```compile_fail
/// let plan = orange_compiler::PurePlan::default();
/// ```
#[derive(Debug, Clone)]
pub struct PurePlan {
    pub(crate) snapshot: Arc<RegistrySnapshot>,
    pub(crate) graph: GraphId,
    pub(crate) revision: u64,
    pub(crate) state: u64,
    pub(crate) steps: Vec<PureStep>,
    pub(crate) types: Vec<PlanType>,
    pub(crate) sources: Vec<SlotSource>,
    pub(crate) constants: Vec<FoldedValue>,
    pub(crate) exports: Vec<Export>,
    pub(crate) diagnostics: Vec<Diagnostic>,
}
impl PurePlan {
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
        self.state == doc.state_id() && self.snapshot.generation() == snapshot.generation()
    }
    pub fn steps(&self) -> &[PureStep] {
        &self.steps
    }
    pub fn slot_types(&self) -> &[PlanType] {
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
