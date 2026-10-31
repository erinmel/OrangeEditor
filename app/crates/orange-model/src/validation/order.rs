//! Dynamic topological order of one body's data dependencies between nodes (V12d, 16 §9).
//!
//! A scoped structure edit must prove that its added data edges close no cycle. Walking
//! everything downstream of each target visits the rest of a long chain; a kept order lets
//! an edge that already runs forward in it pass at once, and searches only the nodes whose
//! positions lie between its endpoints otherwise (Pearce and Kelly, 2006).

use std::collections::{BTreeMap, BTreeSet};

use crate::document::{BodyRef, GraphDocument, PortKind};
use crate::id::NodeId;
use crate::index::DocIndex;

use super::ports::PortResolver;

/// A topological order of one body's nodes under its data edges, where a data edge is any
/// body edge leaving a data port. Node-level order is coarser than the data-cycle pass's
/// port-level graph, so an edge it accepts closes no cycle there either.
#[derive(Clone, Debug, Default)]
pub(super) struct TopoOrder {
    position: BTreeMap<NodeId, usize>,
    /// The next free position; removed nodes leave holes.
    next: usize,
}

/// The far ends of the data edges leaving `node`, or entering it unless `forward`.
fn data_edges<'a>(
    document: &'a GraphDocument,
    ports: &'a PortResolver<'_>,
    index: &'a DocIndex,
    node: NodeId,
    forward: bool,
) -> impl Iterator<Item = NodeId> + 'a {
    index.incident(node).filter_map(move |id| {
        let edge = document.edge(id)?;
        let (near, far) = if forward {
            (edge.from.node, edge.to.node)
        } else {
            (edge.to.node, edge.from.node)
        };
        (near == node && matches!(ports.port(&edge.from), Ok(port) if port.kind == PortKind::Data))
            .then_some(far)
    })
}

impl TopoOrder {
    /// The order of `body`'s nodes, or `None` if its data edges join nodes in a cycle, which
    /// a port-level acyclic graph may still do through a node whose outputs do not read
    /// every input.
    pub(super) fn build(
        document: &GraphDocument,
        ports: &PortResolver<'_>,
        body: BodyRef,
    ) -> Option<Self> {
        let body = document.body(body)?;
        // Dense indices over the sorted node IDs, and each data edge resolved once. A body
        // keeps its nodes in insertion order, so the IDs are sorted here.
        let mut ids: Vec<NodeId> = body.nodes.keys().copied().collect();
        ids.sort_unstable();
        let dense = |node: NodeId| ids.binary_search(&node).ok();
        let mut next: Vec<Vec<usize>> = vec![Vec::new(); ids.len()];
        let mut indegree = vec![0_usize; ids.len()];
        for edge in body.edges.values() {
            let (Some(from), Some(to)) = (dense(edge.from.node), dense(edge.to.node)) else {
                continue;
            };
            if matches!(ports.port(&edge.from), Ok(port) if port.kind == PortKind::Data) {
                next[from].push(to);
                indegree[to] += 1;
            }
        }
        let mut ready: Vec<usize> = (0..ids.len()).filter(|at| indegree[*at] == 0).collect();
        let mut placed = Vec::with_capacity(ids.len());
        while let Some(node) = ready.pop() {
            placed.push((ids[node], placed.len()));
            for &to in &next[node] {
                indegree[to] -= 1;
                if indegree[to] == 0 {
                    ready.push(to);
                }
            }
        }
        if placed.len() != ids.len() {
            return None;
        }
        placed.sort_unstable();
        Some(Self {
            position: placed.into_iter().collect(),
            next: ids.len(),
        })
    }

    pub(super) fn position(&self, node: NodeId) -> Option<usize> {
        self.position.get(&node).copied()
    }

    /// Places a new node, which has no edges yet, last.
    pub(super) fn push(&mut self, node: NodeId) {
        self.position.insert(node, self.next);
        self.next += 1;
    }

    pub(super) fn remove(&mut self, node: NodeId) {
        self.position.remove(&node);
    }

    /// Accounts for a data edge `from -> to` already in the document, or returns `false` if
    /// it closes a cycle.
    ///
    /// Both searches stay between the endpoints' positions. Other edges added by the same
    /// edit and not yet accounted for may be followed there: a path through one is a path
    /// once the edit is whole, so a cycle found through it is real, and nodes it reaches
    /// inside the interval move as any reachable node does.
    pub(super) fn insert(
        &mut self,
        document: &GraphDocument,
        ports: &PortResolver<'_>,
        index: &DocIndex,
        from: NodeId,
        to: NodeId,
    ) -> bool {
        let (Some(upper), Some(lower)) = (self.position(from), self.position(to)) else {
            return false;
        };
        if lower > upper {
            return true;
        }
        if from == to {
            return false;
        }
        // Everything `to` reaches and everything reaching `from`, within the interval; only
        // these can be out of order.
        let within = |at: usize| (lower..=upper).contains(&at);
        let mut forward = BTreeSet::new();
        let mut stack = vec![to];
        while let Some(node) = stack.pop() {
            if node == from {
                return false;
            }
            if self.position(node).is_some_and(within) && forward.insert(node) {
                stack.extend(data_edges(document, ports, index, node, true));
            }
        }
        let mut backward = BTreeSet::new();
        let mut stack = vec![from];
        while let Some(node) = stack.pop() {
            if self.position(node).is_some_and(within) && backward.insert(node) {
                stack.extend(data_edges(document, ports, index, node, false));
            }
        }
        // Reuse the affected positions: the backward set first, then the forward set, each in
        // its old relative order.
        let by_position = |set: BTreeSet<NodeId>| {
            let mut nodes: Vec<(usize, NodeId)> = set
                .into_iter()
                .map(|node| (self.position[&node], node))
                .collect();
            nodes.sort_unstable();
            nodes
        };
        let backward = by_position(backward);
        let forward = by_position(forward);
        let mut slots: Vec<usize> = backward.iter().chain(&forward).map(|(at, _)| *at).collect();
        slots.sort_unstable();
        for (at, (_, node)) in slots.into_iter().zip(backward.into_iter().chain(forward)) {
            self.position.insert(node, at);
        }
        true
    }
}
