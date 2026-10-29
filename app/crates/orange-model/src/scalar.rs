//! Language-owned scalar arithmetic, shared by bundled kernels and state actions.
use crate::Value;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarError {
    NumericOverflow,
    DivideByZero,
    NonFinite,
    InputContract,
}

impl std::fmt::Display for ScalarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for ScalarError {}

impl ScalarError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::NumericOverflow => "core/NumericOverflow",
            Self::DivideByZero => "core/DivideByZero",
            Self::NonFinite => "core/NonFinite",
            Self::InputContract => "core/InternalFault",
        }
    }

    /// The safe static fallback text, also its `Display`. Adapters that describe
    /// `InputContract` in their own context replace only that case.
    pub const fn message(self) -> &'static str {
        match self {
            Self::NumericOverflow => "Numeric arithmetic overflowed.",
            Self::DivideByZero => "Division requires a nonzero divisor.",
            Self::NonFinite => "Finite arithmetic requires finite inputs and results.",
            Self::InputContract => "Kernel inputs violate the resolved contract.",
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

/// Evaluates exact scalar inputs using resolved arithmetic policies. Not requires the
/// `Bool` scalar and arithmetic a numeric one; any other combination is `InputContract`.
pub fn evaluate(
    operation: Operation,
    scalar: Scalar,
    overflow: Overflow,
    float_policy: FloatPolicy,
    inputs: &[Value],
) -> Result<Value, ScalarError> {
    if operation == Operation::Not {
        return match inputs {
            [Value::Bool(value)] if scalar == Scalar::Bool => Ok(Value::Bool(!value)),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_requires_the_bool_scalar_and_arithmetic_a_numeric_one() {
        let not = |scalar| {
            evaluate(
                Operation::Not,
                scalar,
                Overflow::Checked,
                FloatPolicy::Finite,
                &[Value::Bool(true)],
            )
        };
        assert_eq!(not(Scalar::Bool), Ok(Value::Bool(false)));
        for scalar in [
            Scalar::I32,
            Scalar::I64,
            Scalar::U32,
            Scalar::U64,
            Scalar::F32,
            Scalar::F64,
        ] {
            assert_eq!(not(scalar), Err(ScalarError::InputContract), "{scalar:?}");
        }
        let add = evaluate(
            Operation::Add,
            Scalar::Bool,
            Overflow::Checked,
            FloatPolicy::Finite,
            &[Value::Bool(true), Value::Bool(false)],
        );
        assert_eq!(add, Err(ScalarError::InputContract));
    }
}
