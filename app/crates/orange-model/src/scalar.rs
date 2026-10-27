//! Language-owned scalar arithmetic, shared by bundled kernels and state actions.
use crate::Value;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Add,
    Subtract,
    Multiply,
    Divide,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scalar {
    I32,
    I64,
    U32,
    U64,
    F32,
    F64,
    Bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    Checked,
    Wrapping,
    Saturating,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatPolicy {
    Finite,
    Ieee,
}

/// Scalar execution failures. Runtime orchestration adds node/activation provenance
/// and the full redacted MessageRef envelope; this layer never interpolates values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ScalarError {
    #[error("Numeric arithmetic overflowed.")]
    NumericOverflow,
    #[error("Division requires a nonzero divisor.")]
    DivideByZero,
    #[error("Finite arithmetic requires finite inputs and results.")]
    NonFinite,
    #[error("Kernel inputs violate the resolved contract.")]
    InputContract,
}

impl ScalarError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::NumericOverflow => "core/NumericOverflow",
            Self::DivideByZero => "core/DivideByZero",
            Self::NonFinite => "core/NonFinite",
            Self::InputContract => "core/InternalFault",
        }
    }

    pub const fn message_key(self) -> &'static str {
        match self {
            Self::NumericOverflow => "core/kernel-numeric-overflow",
            Self::DivideByZero => "core/kernel-divide-by-zero",
            Self::NonFinite => "core/kernel-non-finite",
            Self::InputContract => "core/kernel-input-contract",
        }
    }
}

/// Evaluates exact scalar inputs using resolved arithmetic policies.
pub fn evaluate(
    operation: Operation,
    scalar: Scalar,
    overflow: Overflow,
    float_policy: FloatPolicy,
    inputs: &[Value],
) -> Result<Value, ScalarError> {
    if operation == Operation::Not {
        return match inputs {
            [Value::Bool(value)] => Ok(Value::Bool(!value)),
            _ => Err(ScalarError::InputContract),
        };
    }
    let [left, right] = inputs else {
        return Err(ScalarError::InputContract);
    };

    macro_rules! integer {
        ($variant:ident) => {{
            let (Value::$variant(a), Value::$variant(b)) = (left, right) else {
                return Err(ScalarError::InputContract);
            };
            let (a, b) = (*a, *b);
            if operation == Operation::Divide && b == 0 {
                return Err(ScalarError::DivideByZero);
            }
            use Operation::{Add, Divide, Multiply, Subtract};
            use Overflow::{Checked, Saturating, Wrapping};
            let value = match (operation, overflow) {
                (Add, Checked) => a.checked_add(b),
                (Subtract, Checked) => a.checked_sub(b),
                (Multiply, Checked) => a.checked_mul(b),
                (Divide, Checked) => a.checked_div(b),
                (Add, Wrapping) => Some(a.wrapping_add(b)),
                (Subtract, Wrapping) => Some(a.wrapping_sub(b)),
                (Multiply, Wrapping) => Some(a.wrapping_mul(b)),
                (Divide, Wrapping) => Some(a.wrapping_div(b)),
                (Add, Saturating) => Some(a.saturating_add(b)),
                (Subtract, Saturating) => Some(a.saturating_sub(b)),
                (Multiply, Saturating) => Some(a.saturating_mul(b)),
                (Divide, Saturating) => Some(a.saturating_div(b)),
                _ => return Err(ScalarError::InputContract),
            }
            .ok_or(ScalarError::NumericOverflow)?;
            Ok(Value::$variant(value))
        }};
    }
    macro_rules! float {
        ($variant:ident) => {{
            let (Value::$variant(a), Value::$variant(b)) = (left, right) else {
                return Err(ScalarError::InputContract);
            };
            let (a, b) = (*a, *b);
            if float_policy == FloatPolicy::Finite {
                if !a.is_finite() || !b.is_finite() {
                    return Err(ScalarError::NonFinite);
                }
                if operation == Operation::Divide && b == 0.0 {
                    return Err(ScalarError::DivideByZero);
                }
            }
            let result = match operation {
                Operation::Add => a + b,
                Operation::Subtract => a - b,
                Operation::Multiply => a * b,
                Operation::Divide => a / b,
                Operation::Not => return Err(ScalarError::InputContract),
            };
            if float_policy == FloatPolicy::Finite && !result.is_finite() {
                return Err(ScalarError::NonFinite);
            }
            Ok(Value::$variant(result))
        }};
    }
    match scalar {
        Scalar::I32 => integer!(I32),
        Scalar::I64 => integer!(I64),
        Scalar::U32 => integer!(U32),
        Scalar::U64 => integer!(U64),
        Scalar::F32 => float!(F32),
        Scalar::F64 => float!(F64),
        Scalar::Bool => Err(ScalarError::InputContract),
    }
}
