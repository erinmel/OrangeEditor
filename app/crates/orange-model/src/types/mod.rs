//! Registered type resolution and snapshot-local interning (00c.1).
//!
//! Builders register immutable type metadata and pre-intern concrete expressions. The
//! published registry exposes only read-only validation and lookup; keys are
//! snapshot-local and never persist.

mod errors;
mod identity;
mod registry;
#[cfg(test)]
mod tests;

pub use errors::{TypeRegistrationError, TypeResolveError};
pub use identity::{TypeKey, TypeKind, TypeOwner};
pub use registry::{TypeRegistry, TypeRegistryBuilder};
