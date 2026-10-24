//! Typed failures contain no input values, config text or build digest.

use orange_model::IdError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("The bundled build identity is invalid.")]
    InvalidBuildDigest,
    #[error("Bundled kernel metadata is invalid.")]
    Metadata(#[from] IdError),
}

impl CatalogError {
    pub const fn message_key(&self) -> &'static str {
        match self {
            Self::InvalidBuildDigest => "core/kernel-build-identity-invalid",
            Self::Metadata(_) => "core/kernel-metadata-invalid",
        }
    }
}

/// Resolution failures; the host associates these with the node and catalogue origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BindingError {
    #[error("The exact native kernel provider is unavailable.")]
    Unavailable,
    #[error("The native kernel does not support this type specialization.")]
    UnsupportedType,
    #[error("The native kernel configuration is invalid.")]
    InvalidConfig,
}

impl BindingError {
    pub const fn message_key(self) -> &'static str {
        match self {
            Self::Unavailable => "core/kernel-unavailable",
            Self::UnsupportedType => "core/kernel-unsupported-type",
            Self::InvalidConfig => "core/kernel-invalid-config",
        }
    }
}

/// Scalar execution failures. Runtime orchestration adds node/activation provenance
/// and the full redacted MessageRef envelope; this layer never interpolates values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum KernelError {
    #[error("Numeric arithmetic overflowed.")]
    NumericOverflow,
    #[error("Division requires a nonzero divisor.")]
    DivideByZero,
    #[error("Finite arithmetic requires finite inputs and results.")]
    NonFinite,
    #[error("Kernel inputs violate the resolved contract.")]
    InputContract,
}

impl KernelError {
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
