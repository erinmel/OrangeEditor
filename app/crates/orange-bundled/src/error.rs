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

pub use orange_model::scalar::ScalarError as KernelError;

/// Explicit scalar conversion failures; the caller adds runtime provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ConversionError {
    #[error("The value is outside the target conversion range.")]
    OutOfRange,
    #[error("The value is not finite and has no integer conversion.")]
    NonFinite,
    #[error("The text is not in the invariant format of the target type.")]
    Parse,
    #[error("The bytes are not valid UTF-8.")]
    InvalidUtf8,
    #[error("The value exceeds the conversion's size or depth limit.")]
    LimitExceeded,
    #[error("Converter input violates the resolved contract.")]
    InputContract,
}

impl ConversionError {
    /// Safe static fallback text; it never contains the converted value.
    pub const fn message(self) -> &'static str {
        match self {
            Self::OutOfRange => "The value is outside the target conversion range.",
            Self::NonFinite => "The value is not finite and has no integer conversion.",
            Self::Parse => "The text is not in the invariant format of the target type.",
            Self::InvalidUtf8 => "The bytes are not valid UTF-8.",
            Self::LimitExceeded => "The value exceeds the conversion's size or depth limit.",
            Self::InputContract => "Converter input violates the resolved contract.",
        }
    }

    pub const fn code(self) -> &'static str {
        match self {
            Self::OutOfRange => "core/OutOfRange",
            Self::NonFinite => "core/NonFinite",
            Self::Parse => "core/Parse",
            Self::InvalidUtf8 => "core/InvalidUtf8",
            Self::LimitExceeded => "core/LimitExceeded",
            Self::InputContract => "core/InternalFault",
        }
    }

    pub const fn message_key(self) -> &'static str {
        match self {
            Self::OutOfRange => "core/conversion-out-of-range",
            Self::NonFinite => "core/conversion-non-finite",
            Self::Parse => "core/conversion-parse",
            Self::InvalidUtf8 => "core/conversion-invalid-utf8",
            Self::LimitExceeded => "core/conversion-limit-exceeded",
            Self::InputContract => "core/conversion-input-contract",
        }
    }
}
