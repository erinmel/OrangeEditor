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

/// Supported scalar/JSON leaves and recursively typed options/results. Values
/// follow the model's boxed/unboxed representation, not JSON null.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanType {
    Bool,
    I32,
    I64,
    U32,
    U64,
    F32,
    F64,
    Json,
    Duration,
    Error,
    Option(Box<PlanType>),
    Result(Box<PlanType>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_types_preserve_unboxed_and_nested_value_contracts() {
        let option = PlanType::Option(Box::new(PlanType::I64));
        assert!(option.accepts(&Value::None));
        assert!(option.accepts(&Value::I64(7)));
        assert!(!option.accepts(&Value::Bool(false)));
        assert!(!option.accepts(&Value::Some(Arc::new(Value::I64(7)))));
        let nested = PlanType::Option(Box::new(option));
        assert!(nested.accepts(&Value::None));
        assert!(nested.accepts(&Value::Some(Arc::new(Value::None))));
        assert!(nested.accepts(&Value::Some(Arc::new(Value::I64(7)))));
        assert!(!nested.accepts(&Value::I64(7)));
        assert!(!nested.accepts(&Value::Some(Arc::new(Value::Bool(false)))));
        assert!(
            !nested.accepts(&Value::Err(Arc::new(orange_model::value::NodeError {
                code: "core/Test".into(),
                node: None
            })))
        );
    }

    #[test]
    fn option_type_depth_and_unsupported_leaves_are_bounded() {
        let wrap = |n| format!("{}core/json@1{}", "core/option@1<".repeat(n), ">".repeat(n));
        assert!(PlanType::parse(&wrap(orange_model::type_term::MAX_NESTING)).is_some());
        assert!(PlanType::parse(&wrap(orange_model::type_term::MAX_NESTING + 1)).is_none());
        for invalid in [
            "core/option@1<core/string@1>",
            "core/option@1<core/i64@1,core/i64@1>",
            "core/option@1<$T>",
            "core/option@2<core/i64@1>",
        ] {
            assert!(PlanType::parse(invalid).is_none());
        }
    }

    #[test]
    fn result_types_validate_error_codes_and_success_boxing() {
        let error = Value::Err(Arc::new(orange_model::value::NodeError {
            code: "core/DivideByZero".into(),
            node: None,
        }));
        let scalar = PlanType::Result(Box::new(PlanType::I64));
        assert!(scalar.accepts(&error));
        assert!(scalar.accepts(&Value::I64(1)));
        assert!(!scalar.accepts(&Value::Ok(Arc::new(Value::I64(1)))));
        let nested = PlanType::Result(Box::new(scalar));
        assert!(nested.accepts(&Value::Ok(Arc::new(error.clone()))));
        assert!(!nested.accepts(&Value::I64(1)));
        assert!(nested.accepts(&error)); // An actual outer Err, not Ok(Err).
        let optional = PlanType::Result(Box::new(PlanType::Option(Box::new(PlanType::Result(
            Box::new(PlanType::I64),
        )))));
        assert!(optional.accepts(&Value::Ok(Arc::new(Value::None))));
        assert!(!optional.accepts(&Value::None));
        let long = format!("core/{}", "x".repeat(257));
        for code in [
            "no_namespace",
            "core/secret\ntext",
            "core/a/b",
            long.as_str(),
        ] {
            assert!(
                !nested.accepts(&Value::Err(Arc::new(orange_model::value::NodeError {
                    code: code.into(),
                    node: None
                })))
            );
        }
    }
}
impl PlanType {
    pub(crate) fn parse(ty: &str) -> Option<Self> {
        Self::parse_depth(ty, 0)
    }
    fn parse_depth(ty: &str, depth: usize) -> Option<Self> {
        if depth > orange_model::type_term::MAX_NESTING {
            return None;
        }
        if let Some(inner) = ty
            .strip_prefix("core/option@1<")
            .and_then(|s| s.strip_suffix('>'))
        {
            return Self::parse_depth(inner, depth + 1).map(|inner| Self::Option(Box::new(inner)));
        }
        if let Some(inner) = ty
            .strip_prefix("core/result@1<")
            .and_then(|s| s.strip_suffix('>'))
        {
            return Self::parse_depth(inner, depth + 1).map(|inner| Self::Result(Box::new(inner)));
        }
        Some(match ty {
            "core/bool@1" => Self::Bool,
            "core/i32@1" => Self::I32,
            "core/i64@1" => Self::I64,
            "core/u32@1" => Self::U32,
            "core/u64@1" => Self::U64,
            "core/f32@1" => Self::F32,
            "core/f64@1" => Self::F64,
            "core/json@1" => Self::Json,
            "core/duration@1" => Self::Duration,
            "core/error@1" => Self::Error,
            _ => return None,
        })
    }
    pub(crate) fn accepts(&self, value: &Value) -> bool {
        if let Self::Result(inner) = self {
            return match value {
                Value::Err(error) => crate::compile::valid_identifier(&error.code),
                Value::Ok(value) if inner.unboxed_error() => inner.accepts(value),
                _ if inner.unboxed_error() => false,
                _ => inner.accepts(value),
            };
        }
        if let Self::Option(inner) = self {
            return match value {
                Value::None => true,
                Value::Some(value) if matches!(inner.as_ref(), Self::Option(_)) => {
                    inner.accepts(value)
                }
                _ if matches!(inner.as_ref(), Self::Option(_)) => false,
                _ => inner.accepts(value),
            };
        }
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
                | (Self::Duration, Value::Duration(_))
        ) || matches!((self, value), (Self::Error, Value::Err(error)) if crate::compile::valid_identifier(&error.code))
    }
    /// Whether a valid value of this type can carry the Err discriminant directly.
    pub(crate) fn unboxed_error(&self) -> bool {
        match self {
            Self::Result(_) | Self::Error => true,
            Self::Option(inner) if !matches!(inner.as_ref(), Self::Option(_)) => {
                inner.unboxed_error()
            }
            _ => false,
        }
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

/// Checked restricted pure-export plan, with a pinned registry and original source IDs.
/// It is not approval for control, volatile, function or dynamic epoch execution.
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
    pub(crate) types: Vec<PlanType>,
    pub(crate) sources: Vec<SlotSource>,
    pub(crate) constants: Vec<FoldedValue>,
    pub(crate) exports: Vec<Export>,
    pub(crate) diagnostics: Vec<Diagnostic>,
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
