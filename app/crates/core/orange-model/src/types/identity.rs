use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::type_term::NamedTerm;

/// The canonical expression's identity within one [`super::TypeRegistry`].
///
/// The numeric slot is private, and equality includes the owning registry identity. Keys
/// from separate registries never compare equal, even if both registries assigned slot 0.
#[derive(Clone)]
pub struct TypeKey {
    pub(super) owner: Arc<RegistryIdentity>,
    pub(super) slot: u32,
}

impl fmt::Debug for TypeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TypeKey")
            .field("slot", &self.slot)
            .finish_non_exhaustive()
    }
}

impl PartialEq for TypeKey {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.owner, &other.owner) && self.slot == other.slot
    }
}

impl Eq for TypeKey {}

impl Hash for TypeKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.owner) as usize).hash(state);
        self.slot.hash(state);
    }
}

#[derive(Debug)]
pub(super) struct RegistryIdentity;

/// Identity of a registered type's owner, for coherence checks in later slices.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeOwner {
    /// A language-owned type in the reserved `core` namespace.
    Core,
    /// The package namespace that owns this nominal type.
    Package(String),
}

/// Kind of registered type, independent of its runtime value representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeKind {
    Core,
    Record,
    Enum,
    /// Opaque handle-kind marker; it has no literal codec.
    HandleKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct NamedTypeId {
    pub(super) namespace: String,
    pub(super) name: String,
    pub(super) major: u32,
}

impl NamedTypeId {
    pub(super) fn from_term(term: &NamedTerm) -> Self {
        Self {
            namespace: term.namespace.clone(),
            name: term.name.clone(),
            major: term.major,
        }
    }
}

impl fmt::Display for NamedTypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}@{}", self.namespace, self.name, self.major)
    }
}
