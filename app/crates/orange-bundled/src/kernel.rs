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
        let operation = match self.operation {
            Operation::Add => orange_model::scalar::Operation::Add,
            Operation::Subtract => orange_model::scalar::Operation::Subtract,
            Operation::Multiply => orange_model::scalar::Operation::Multiply,
            Operation::Divide => orange_model::scalar::Operation::Divide,
            Operation::Not => orange_model::scalar::Operation::Not,
        };
        orange_model::scalar::evaluate(
            operation,
            self.scalar,
            self.overflow,
            self.float_policy,
            inputs,
        )
    }
}
