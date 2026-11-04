//! Which bodies a body-local validation pass visits.

use crate::document::{BodyRef, GraphBody, GraphDocument};
use crate::id::FunctionId;

/// Root-body nodes a pass runs for alone, with the edges it may read, sorted by ID. The
/// caller gathers them through the document index, so the pass never walks the body.
pub(crate) struct NodeSubset<'a> {
    pub nodes: &'a std::collections::BTreeSet<crate::id::NodeId>,
    /// The nodes among `nodes` that availability checks as consumers; the region and
    /// control-cycle passes ignore it.
    pub consumers: &'a std::collections::BTreeSet<crate::id::NodeId>,
    pub edges: Vec<&'a crate::document::Edge>,
    /// What the passes run over the subset iterated (16 §3): they count into it as they go.
    pub visits: std::cell::Cell<super::visits::VisitCounts>,
}

impl<'a> NodeSubset<'a> {
    /// The subset's nodes in ID order, as a body lists them. Each subset node is looked up.
    pub fn nodes_in(
        &self,
        body: &'a crate::document::GraphBody,
    ) -> Vec<&'a crate::document::NodeInstance> {
        self.count(|visits| visits.nodes += self.nodes.len());
        self.nodes
            .iter()
            .filter_map(|id| body.nodes.get(id))
            .collect()
    }

    /// Adds to the subset's visit counts.
    pub fn count(&self, add: impl FnOnce(&mut super::visits::VisitCounts)) {
        let mut visits = self.visits.get();
        add(&mut visits);
        self.visits.set(visits);
    }
}

/// Adds to `only`'s visit counts, when a pass runs over a subset.
pub(crate) fn count(
    only: Option<&NodeSubset<'_>>,
    add: impl FnOnce(&mut super::visits::VisitCounts),
) {
    if let Some(only) = only {
        only.count(add);
    }
}

/// `body`'s nodes, or the subset's, in ID order.
pub(crate) fn nodes_of<'a>(
    body: &'a crate::document::GraphBody,
    only: Option<&NodeSubset<'a>>,
) -> Vec<&'a crate::document::NodeInstance> {
    match only {
        Some(subset) => subset.nodes_in(body),
        None => body.nodes.values().collect(),
    }
}

/// `body`'s edges, or the subset's, sorted by ID.
pub(crate) fn edges_of<'a>(
    body: &'a crate::document::GraphBody,
    only: Option<&NodeSubset<'a>>,
) -> Vec<&'a crate::document::Edge> {
    match only {
        Some(subset) => {
            subset.count(|visits| visits.edges += subset.edges.len());
            subset.edges.clone()
        }
        None => {
            let mut edges: Vec<_> = body.edges.values().collect();
            edges.sort_by_key(|edge| edge.id);
            edges
        }
    }
}

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
