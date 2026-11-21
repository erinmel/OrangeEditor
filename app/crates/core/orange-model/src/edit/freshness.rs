//! Freshness of detached plans: a plan records the document content state and registry
//! snapshot it was made against, and applying it is refused once either has changed.

use thiserror::Error;

use crate::catalog::RegistrySnapshot;
use crate::document::GraphDocument;
use crate::error::TxError;

/// Why a plan could not be applied.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlanApplyError {
    #[error("the document changed since the plan was made (state {expected}, now {actual})")]
    StaleDocument { expected: u64, actual: u64 },
    #[error("the registry snapshot was replaced since the plan was made")]
    StaleRegistry,
    #[error(transparent)]
    Edit(#[from] TxError),
}

/// What a plan was made against. Revision alone is insufficient across clones and reloads,
/// so the document side is its content-state identity. Any edit, layout included, makes
/// the plan stale: its commands were derived from exact content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Preconditions {
    document_state: u64,
    registry_generation: u64,
}

impl Preconditions {
    pub(crate) fn capture(document: &GraphDocument, registry: &RegistrySnapshot) -> Self {
        Self {
            document_state: document.state_id(),
            registry_generation: registry.generation(),
        }
    }

    /// A registry snapshot is immutable, so a matching generation also means every provider
    /// the plan selected is still available.
    pub(crate) fn check(
        &self,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
    ) -> Result<(), PlanApplyError> {
        if document.state_id() != self.document_state {
            return Err(PlanApplyError::StaleDocument {
                expected: self.document_state,
                actual: document.state_id(),
            });
        }
        if registry.generation() != self.registry_generation {
            return Err(PlanApplyError::StaleRegistry);
        }
        Ok(())
    }
}
