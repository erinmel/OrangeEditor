//! Closed Core update operations. Capability proofs do not authorize package callbacks.
use super::*;
use orange_model::scalar::{FloatPolicy, Operation, Overflow, Scalar};

#[derive(Debug, Clone, Copy)]
pub enum UpdateOperation {
    Numeric {
        operation: Operation,
        scalar: Scalar,
        overflow: Overflow,
        float_policy: FloatPolicy,
    },
    Min,
    Max,
    And,
    Or,
    Xor,
    Append,
    CompareAndSet,
}
#[derive(Debug, Clone, Copy)]
pub enum UpdateProjection {
    Old,
    New,
    Exchanged,
}

/// Ordered operand roots and their compiler-resolved frozen host-input union.
#[derive(Debug, Clone)]
pub struct UpdateDemand {
    pub(super) roots: Box<[usize]>,
    pub(super) inputs: Arc<[usize]>,
}
impl UpdateDemand {
    pub fn roots(&self) -> &[usize] {
        &self.roots
    }
    pub fn inputs(&self) -> &[usize] {
        &self.inputs
    }
}

fn scalar(ty: &PlanType) -> Option<Scalar> {
    Some(match ty {
        PlanType::I32 => Scalar::I32,
        PlanType::I64 => Scalar::I64,
        PlanType::U32 => Scalar::U32,
        PlanType::U64 => Scalar::U64,
        PlanType::F32 => Scalar::F32,
        PlanType::F64 => Scalar::F64,
        _ => return None,
    })
}
fn equal(ty: &PlanType) -> bool {
    match ty {
        PlanType::Bool
        | PlanType::I32
        | PlanType::I64
        | PlanType::U32
        | PlanType::U64
        | PlanType::String
        | PlanType::Duration => true,
        PlanType::Option(inner) => equal(inner),
        _ => false,
    }
}
pub(super) fn resolve(
    node: NodeId,
    config: &serde_json::Map<String, serde_json::Value>,
    ty: &PlanType,
) -> Result<UpdateOperation, CompileError> {
    let unsupported = || CompileError::UnsupportedNode { node };
    let token = config
        .get("operation")
        .and_then(|v| v.as_str())
        .ok_or_else(unsupported)?;
    Ok(match token {
        "add" | "sub" => {
            let scalar = scalar(ty).ok_or_else(unsupported)?;
            let overflow = match config.get("int_overflow") {
                None => Overflow::Checked,
                Some(v) => match v.as_str() {
                    Some("checked") => Overflow::Checked,
                    Some("wrapping") => Overflow::Wrapping,
                    Some("saturating") => Overflow::Saturating,
                    _ => return Err(unsupported()),
                },
            };
            let float_policy = match config.get("float_policy") {
                None => FloatPolicy::Finite,
                Some(v) => match v.as_str() {
                    Some("finite") => FloatPolicy::Finite,
                    Some("ieee") => FloatPolicy::Ieee,
                    _ => return Err(unsupported()),
                },
            };
            UpdateOperation::Numeric {
                operation: if token == "add" {
                    Operation::Add
                } else {
                    Operation::Subtract
                },
                scalar,
                overflow,
                float_policy,
            }
        }
        "min" | "max"
            if matches!(
                ty,
                PlanType::Bool
                    | PlanType::I32
                    | PlanType::I64
                    | PlanType::U32
                    | PlanType::U64
                    | PlanType::String
                    | PlanType::Duration
            ) =>
        {
            if token == "min" {
                UpdateOperation::Min
            } else {
                UpdateOperation::Max
            }
        }
        "and" | "or" | "xor"
            if matches!(
                ty,
                PlanType::Bool | PlanType::I32 | PlanType::I64 | PlanType::U32 | PlanType::U64
            ) =>
        {
            match token {
                "and" => UpdateOperation::And,
                "or" => UpdateOperation::Or,
                _ => UpdateOperation::Xor,
            }
        }
        "append" if matches!(ty, PlanType::String) => UpdateOperation::Append,
        "compare-and-set" if equal(ty) => UpdateOperation::CompareAndSet,
        _ => return Err(unsupported()),
    })
}
