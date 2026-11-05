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
}

impl<'a> NodeSubset<'a> {
    /// The subset's nodes in ID order, as a body lists them, counting each subset node
    /// looked up.
    pub fn nodes_in(
        &self,
        body: &'a crate::document::GraphBody,
        tally: Tally<'_>,
    ) -> Vec<&'a crate::document::NodeInstance> {
        self.nodes
            .iter()
            .filter_map(|id| {
                tally.add(|visits| visits.nodes += 1);
                body.nodes.get(id)
            })
            .collect()
    }
}

/// Where a body pass counts what it iterates (16 §3), or nowhere. A scoped session update
/// passes one; the full analysis passes [`Tally::NONE`].
#[derive(Clone, Copy, Default)]
pub(crate) struct Tally<'a>(Option<&'a std::cell::Cell<super::visits::VisitCounts>>);

impl<'a> Tally<'a> {
    pub const NONE: Self = Self(None);

    pub fn new(cell: &'a std::cell::Cell<super::visits::VisitCounts>) -> Self {
        Self(Some(cell))
    }

    /// Adds to the counts, if any are kept.
    pub fn add(self, add: impl FnOnce(&mut super::visits::VisitCounts)) {
        if let Some(cell) = self.0 {
            let mut visits = cell.get();
            add(&mut visits);
            cell.set(visits);
        }
    }
}

/// `body`'s nodes, or the subset's, in ID order, counting each one listed.
pub(crate) fn nodes_of<'a>(
    body: &'a crate::document::GraphBody,
    only: Option<&NodeSubset<'a>>,
    tally: Tally<'_>,
) -> Vec<&'a crate::document::NodeInstance> {
    match only {
        Some(subset) => subset.nodes_in(body, tally),
        None => body
            .nodes
            .values()
            .inspect(|_| tally.add(|visits| visits.nodes += 1))
            .collect(),
    }
}

/// `body`'s edges, or the subset's, sorted by ID, counting each one listed.
pub(crate) fn edges_of<'a>(
    body: &'a crate::document::GraphBody,
    only: Option<&NodeSubset<'a>>,
    tally: Tally<'_>,
) -> Vec<&'a crate::document::Edge> {
    let count = |_: &&crate::document::Edge| tally.add(|visits| visits.edges += 1);
    match only {
        Some(subset) => subset.edges.iter().copied().inspect(count).collect(),
        None => {
            let mut edges: Vec<_> = body.edges.values().inspect(count).collect();
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
