use crate::error::IdError;
use crate::id::TypeExpr;
use crate::types::TypeResolveError;

use super::{ConverterId, ConverterKey};

/// A rejected converter definition or provider publication.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConverterRegistrationError {
    #[error(transparent)]
    InvalidId(#[from] IdError),
    #[error(transparent)]
    TypeResolve(#[from] TypeResolveError),
    #[error("converter error code `{code}` is not a stable namespaced code")]
    InvalidErrorCode { code: String },
    #[error("converter contract version must be non-zero")]
    InvalidContractVersion,
    #[error("lossless-total converter `{entry}` cannot declare failures")]
    LosslessHasErrors { entry: ConverterKey },
    #[error("checked-partial converter `{entry}` must declare at least one failure code")]
    PartialNeedsErrors { entry: ConverterKey },
    #[error("converter `{entry}` repeats an error code")]
    DuplicateErrorCode { entry: ConverterKey },
    #[error("policy name `{name}` is blank or contains whitespace")]
    InvalidPolicyName { name: String },
    #[error("converter policy `{name}` is declared more than once")]
    DuplicatePolicy { name: String },
    #[error("policy `{name}` must declare unique, non-blank string choices")]
    InvalidPolicyChoices { name: String },
    #[error("policy `{name}` default must be one of its declared choices")]
    InvalidPolicyDefault { name: String },
    #[error("converter `{entry}` received unknown policy `{name}`")]
    UnknownPolicy { entry: ConverterKey, name: String },
    #[error("required converter policy `{name}` was not supplied for `{entry}`")]
    MissingPolicy { entry: ConverterKey, name: String },
    #[error("value `{value}` is not a choice for policy `{name}` on `{entry}`")]
    UnknownPolicyChoice {
        entry: ConverterKey,
        name: String,
        value: String,
    },
    #[error("converter ID `{entry}` is not owned by registration authority `{owner}`")]
    EntryNamespaceMismatch { entry: ConverterId, owner: String },
    #[error("converter `{from}` → `{to}` is foreign-to-foreign for authority `{owner}`")]
    ForeignToForeign {
        from: TypeExpr,
        to: TypeExpr,
        owner: String,
    },
    #[error("converter entry `{entry}` is already registered with different contract content")]
    ConflictingEntry { entry: ConverterKey },
    #[error("converter entry `{entry}` is not registered")]
    EntryNotFound { entry: ConverterKey },
    #[error("provider authority `{authority}` does not match provider package `{provider}`")]
    ProviderAuthorityMismatch { authority: String, provider: String },
    #[error("converter provider identity is incomplete or non-canonical")]
    InvalidProviderIdentity,
    #[error("converter provider digest `{digest}` is not a canonical sha256 digest")]
    InvalidProviderDigest { digest: String },
    #[error("provider contract version {found} does not match converter entry `{entry}`")]
    ProviderContractMismatch { entry: ConverterKey, found: u32 },
    #[error("provider is not registered for converter entry `{entry}`")]
    ProviderNotRegistered { entry: ConverterKey },
    #[error(
        "provider `{package}/{id}` version `{version}` has conflicting content digests for `{entry}`"
    )]
    ConflictingProviderDigest {
        entry: ConverterKey,
        package: String,
        version: String,
        id: String,
        contract_version: u32,
    },
}
