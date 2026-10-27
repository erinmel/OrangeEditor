//! Pre-resolved native dispatch. Successful scalar evaluation allocates no buffers.

use std::sync::Arc;

use orange_model::Value;
use orange_model::catalog::{ProviderKey, RegistrySnapshot};
use orange_model::scalar::Operation as ScalarOperation;

use crate::KernelError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Add,
    Subtract,
    Multiply,
    Divide,
    Not,
}

/// One operation's identity, ports and typed scalar rule. Every catalog, binding and
/// dispatch use reads this single declaration; execution still matches typed enums.
struct Spec {
    definition_id: &'static str,
    /// Numeric operations are generic over `T` with overflow/float policies; the
    /// remaining operation is boolean and has no type parameters or config.
    numeric: bool,
    inputs: &'static [&'static str],
    output: &'static str,
    scalar: ScalarOperation,
}

const fn numeric(
    definition_id: &'static str,
    output: &'static str,
    scalar: ScalarOperation,
) -> Spec {
    Spec {
        definition_id,
        numeric: true,
        inputs: &["a", "b"],
        output,
        scalar,
    }
}

impl Operation {
    pub const ALL: [Self; 5] = [
        Self::Add,
        Self::Subtract,
        Self::Multiply,
        Self::Divide,
        Self::Not,
    ];

    const fn spec(self) -> Spec {
        match self {
            Self::Add => numeric("org.orange.math/add", "sum", ScalarOperation::Add),
            Self::Subtract => numeric(
                "org.orange.math/subtract",
                "difference",
                ScalarOperation::Subtract,
            ),
            Self::Multiply => numeric(
                "org.orange.math/multiply",
                "product",
                ScalarOperation::Multiply,
            ),
            Self::Divide => numeric(
                "org.orange.math/divide",
                "quotient",
                ScalarOperation::Divide,
            ),
            Self::Not => Spec {
                definition_id: "org.orange.math/not",
                numeric: false,
                inputs: &["value"],
                output: "result",
                scalar: ScalarOperation::Not,
            },
        }
    }

    /// The operation whose definition ID is exactly `id`.
    pub fn from_definition_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.definition_id() == id)
    }

    pub const fn definition_id(self) -> &'static str {
        self.spec().definition_id
    }

    /// Declared data inputs in executable order: `a`, `b`, or `value` for Not.
    pub const fn input_ports(self) -> &'static [&'static str] {
        self.spec().inputs
    }

    pub const fn output_port(self) -> &'static str {
        self.spec().output
    }

    /// Whether the operation is generic over numeric `T` and accepts numeric policies.
    pub const fn is_numeric(self) -> bool {
        self.spec().numeric
    }
}

pub(crate) use orange_model::scalar::{FloatPolicy, Overflow, Scalar};

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
        orange_model::scalar::evaluate(
            self.operation.spec().scalar,
            self.scalar,
            self.overflow,
            self.float_policy,
            inputs,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_operation_identity_is_unique_and_round_trips() {
        for operation in Operation::ALL {
            assert_eq!(
                Operation::from_definition_id(operation.definition_id()),
                Some(operation)
            );
            assert_eq!(operation.is_numeric(), operation != Operation::Not);
            assert_eq!(
                operation.input_ports().len(),
                if operation.is_numeric() { 2 } else { 1 }
            );
        }
        assert_eq!(
            Operation::from_definition_id("org.orange.math/random-u64"),
            None
        );
    }
}
