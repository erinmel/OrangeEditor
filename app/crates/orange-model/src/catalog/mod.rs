//! Host metadata for node definitions: contracts, provider identity and the immutable
//! registry snapshot that owns them (12 §4, §8).

pub mod definition;
pub mod provider;
pub mod snapshot;

pub use definition::{NodeContract, NodeDefinition, TypeArgs};
pub use provider::{ProviderDescriptor, ProviderKey};
pub use snapshot::{RegistryBuilder, RegistryError, RegistrySnapshot};
