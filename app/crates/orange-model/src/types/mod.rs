//! Registered type resolution and snapshot-local interning (00c.1).
//!
//! Builders register immutable type metadata and pre-intern concrete expressions. The
//! published registry exposes only read-only validation and lookup; keys are
//! snapshot-local and never persist.

mod constraints;
mod errors;
mod identity;
mod implementations;
mod registry;
#[cfg(test)]
mod tests;

pub use constraints::{
    CapabilityError, CapabilityEvidence, CapabilityRequest, CapabilityWitness, Constraint,
    NumericOperation,
};
pub use errors::{TypeRegistrationError, TypeResolveError};
pub use identity::{TypeKey, TypeKind, TypeOwner};
pub use implementations::{ConstraintId, ConstraintImplementationRef, PackageConstraintId};
pub use registry::{TypeRegistry, TypeRegistryBuilder};
