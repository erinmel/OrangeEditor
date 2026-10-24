//! Which bodies a body-local validation pass visits.

use crate::document::{BodyRef, GraphBody, GraphDocument};
use crate::id::FunctionId;

/// Bodies a pass visits. Every body-local pass gives a body the same result in either scope,
/// so function acceptance can check one function body at a time, callee first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    /// Every body of the document, root first.
    All,
    /// Only the body of one graph-local function.
    Function(FunctionId),
}

impl Scope {
    pub(crate) fn bodies(self, document: &GraphDocument) -> Vec<(BodyRef, &GraphBody)> {
        match self {
            Self::All => document.bodies_with_refs().collect(),
            Self::Function(id) => document
                .functions()
                .get(&id)
                .map(|function| (BodyRef::Function(id), &function.body))
                .into_iter()
                .collect(),
        }
    }
}
