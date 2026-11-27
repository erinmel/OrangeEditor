//! Opaque resource-reference values. Metadata is never authority: an embedding
//! runtime must authenticate its own private evidence before using a resource.
use super::HandleId;
use crate::id::TypeExpr;
use std::any::Any;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Opaque runtime alias. Its public metadata is descriptive; the issuing runtime
/// must authenticate private evidence and live ownership before using it.
pub struct HandleValue {
    kind: TypeExpr,
    id: HandleId,
    evidence: Arc<dyn Any + Send + Sync>,
}
impl HandleValue {
    /// Construct a host reference. This does not issue or acquire a resource and
    /// does not make a reference valid in any runtime, even with matching IDs.
    pub fn new(kind: TypeExpr, id: HandleId, evidence: Arc<dyn Any + Send + Sync>) -> Self {
        Self { kind, id, evidence }
    }
    pub fn kind(&self) -> &TypeExpr {
        &self.kind
    }
    pub fn id(&self) -> HandleId {
        self.id
    }
    /// A runtime can recognize its private issuing evidence without exposing it
    /// for cloning or manufacturing another capability.
    pub fn evidence<T: Any>(&self) -> Option<&T> {
        self.evidence.downcast_ref()
    }
}
impl std::fmt::Debug for HandleValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HandleValue")
            .field("kind", &self.kind)
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
impl PartialEq for HandleValue {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.id == other.id
            && Arc::ptr_eq(&self.evidence, &other.evidence)
    }
}
impl Eq for HandleValue {}
impl Hash for HandleValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.kind.hash(state);
        self.id.hash(state);
        std::ptr::hash(Arc::as_ptr(&self.evidence), state);
    }
}
