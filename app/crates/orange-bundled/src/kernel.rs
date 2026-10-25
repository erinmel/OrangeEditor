//! Pre-resolved native dispatch. Successful scalar evaluation allocates no buffers.

use std::sync::Arc;

use orange_model::Value;
use orange_model::catalog::{ProviderKey, RegistrySnapshot};

use crate::KernelError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Add,
    Subtract,
    Multiply,
    Divide,
    Not,
}

impl Operation {
    pub const ALL: [Self; 5] = [
        Self::Add,
        Self::Subtract,
        Self::Multiply,
        Self::Divide,
        Self::Not,
    ];

    pub const fn definition_id(self) -> &'static str {
        match self {
            Self::Add => "org.orange.math/add",
            Self::Subtract => "org.orange.math/subtract",
            Self::Multiply => "org.orange.math/multiply",
            Self::Divide => "org.orange.math/divide",
            Self::Not => "org.orange.math/not",
        }
    }

    pub const fn output_port(self) -> &'static str {
        match self {
            Self::Add => "sum",
            Self::Subtract => "difference",
            Self::Multiply => "product",
            Self::Divide => "quotient",
            Self::Not => "result",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scalar {
    I32,
    I64,
    U32,
    U64,
    F32,
    F64,
    Bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Overflow {
    Checked,
    Wrapping,
    Saturating,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FloatPolicy {
    Finite,
    Ieee,
}

/// An immutable exact binding. Retaining it pins its registry metadata even after the
/// host publishes another snapshot. This does not assert that a graph was validated.
#[derive(Debug, Clone)]
pub struct ResolvedKernel {
    pub(crate) snapshot: Arc<RegistrySnapshot>,
    pub(crate) provider: ProviderKey,
    pub(crate) operation: Operation,
    pub(crate) scalar: Scalar,
    pub(crate) overflow: Overflow,
    pub(crate) float_policy: FloatPolicy,
}

impl ResolvedKernel {
    pub fn snapshot(&self) -> &Arc<RegistrySnapshot> {
        &self.snapshot
    }
    pub fn provider(&self) -> &ProviderKey {
        &self.provider
    }
    pub fn operation(&self) -> Operation {
        self.operation
    }

    /// Evaluates in declared input order (`a`, `b`, or `value` for Not). No coercion,
    /// registry lookup, parsing or host calls occur here. Wrong arity/types return a
    /// contract error; the eventual compiler may not treat this as validation evidence.
    pub fn evaluate(&self, inputs: &[Value]) -> Result<Value, KernelError> {
        if self.operation == Operation::Not {
            return match inputs {
                [Value::Bool(value)] => Ok(Value::Bool(!value)),
                _ => Err(KernelError::InputContract),
            };
        }
        let [left, right] = inputs else {
            return Err(KernelError::InputContract);
        };

        macro_rules! integer {
            ($variant:ident) => {{
                let (Value::$variant(a), Value::$variant(b)) = (left, right) else {
                    return Err(KernelError::InputContract);
                };
                let (a, b) = (*a, *b);
                if self.operation == Operation::Divide && b == 0 {
                    return Err(KernelError::DivideByZero);
                }
                use Operation::{Add, Divide, Multiply, Subtract};
                use Overflow::{Checked, Saturating, Wrapping};
                let value = match (self.operation, self.overflow) {
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
                    _ => return Err(KernelError::InputContract),
                }
                .ok_or(KernelError::NumericOverflow)?;
                Ok(Value::$variant(value))
            }};
        }
        macro_rules! float {
            ($variant:ident) => {{
                let (Value::$variant(a), Value::$variant(b)) = (left, right) else {
                    return Err(KernelError::InputContract);
                };
                let (a, b) = (*a, *b);
                if self.float_policy == FloatPolicy::Finite {
                    if !a.is_finite() || !b.is_finite() {
                        return Err(KernelError::NonFinite);
                    }
                    if self.operation == Operation::Divide && b == 0.0 {
                        return Err(KernelError::DivideByZero);
                    }
                }
                let result = match self.operation {
                    Operation::Add => a + b,
                    Operation::Subtract => a - b,
                    Operation::Multiply => a * b,
                    Operation::Divide => a / b,
                    Operation::Not => return Err(KernelError::InputContract),
                };
                if self.float_policy == FloatPolicy::Finite && !result.is_finite() {
                    return Err(KernelError::NonFinite);
                }
                Ok(Value::$variant(result))
            }};
        }
        match self.scalar {
            Scalar::I32 => integer!(I32),
            Scalar::I64 => integer!(I64),
            Scalar::U32 => integer!(U32),
            Scalar::U64 => integer!(U64),
            Scalar::F32 => float!(F32),
            Scalar::F64 => float!(F64),
            Scalar::Bool => Err(KernelError::InputContract),
        }
    }
}
