//! Host metadata for node definitions: contracts, provider identity and the immutable
//! registry snapshot that owns them (12 §4, §8).

pub mod definition;
pub(crate) mod intrinsics;
pub mod provider;
pub mod snapshot;

pub use definition::{
    NodeContract, NodeDefinition, Ports, ReferenceError, ReferenceField, ReferenceKind, TypeArgs,
    VariableAccess,
};
pub(crate) use intrinsics::{core_reference, intrinsic_definitions};
pub use provider::{ProviderDescriptor, ProviderKey};
pub use snapshot::{RegistryBuilder, RegistryError, RegistrySnapshot};
