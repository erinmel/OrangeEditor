//! Host metadata for node definitions: contracts, type arguments and provider identity.

pub mod definition;
pub mod provider;

pub use definition::{NodeContract, TypeArgs};
pub use provider::ProviderKey;
