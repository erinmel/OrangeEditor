//! Dynamic topological order of one body's data dependencies between nodes (V12d, 16 §9).
//!
//! A scoped structure edit must prove that its added data edges close no cycle. Walking
//! everything downstream of each target visits the rest of a long chain; a kept order lets
//! an edge that already runs forward in it pass at once, and searches only the nodes whose
//! positions lie between its endpoints otherwise (Pearce and Kelly, 2006). The forward and
//! backward searches advance in turn and the first to finish is the side that moves, so an
//! edge costs the smaller of the two affected sets rather than both; positions are sparse
//! labels, so that side moves into the gap beside the other endpoint without renumbering the
//! nodes between them (two-way search, Haeupler et al., 2012).

use std::collections::{BTreeMap, BTreeSet};

use crate::document::{BodyRef, GraphDocument, PortKind};
use crate::id::{EdgeId, NodeId};
use crate::index::DocIndex;

use super::ports::PortResolver;
use super::visits::VisitCounts;

/// The distance between neighbouring positions in a fresh order.
const GAP: u64 = 1 << 32;

/// A topological order of one body's nodes under its data edges, where a data edge is any
/// body edge leaving a data port, and each checked zone's begin to its end: the data-cycle
/// pass makes a zone result depend on its begin's prefix inputs (V12c). Node-level order is
/// coarser than that pass's port-level graph, so an edge it accepts closes no cycle there
/// either.
///
/// A body with data cycles keeps its order over the acyclic rest (V12d): the data edges
/// that could not be placed are held apart, and the order runs every other data edge and
/// zone pair forward. So every cycle of the body, at node or port level, runs through a
/// held edge.
#[derive(Clone, Debug, Default)]
pub(super) struct TopoOrder {
    position: BTreeMap<NodeId, u64>,
    /// The nodes by position, which leaves gaps between them.
    nodes: BTreeMap<u64, NodeId>,
    /// Each checked zone's end by its begin, and its begin by its end. Scoped edits never
    /// add, remove or reconfigure a marker, so the pairs hold while the order is kept.
    ends: BTreeMap<NodeId, NodeId>,
    begins: BTreeMap<NodeId, NodeId>,
    /// The data edges held out of the order, by ID, with their source and target nodes.
    held: BTreeMap<EdgeId, (NodeId, NodeId)>,
}

/// How many nodes [`TopoOrder::cyclic_region`] visits before only the whole data-cycle
/// pass decides the body's cycles.
const CYCLE_REGION_BUDGET: usize = 4096;

/// Pushes onto `stack` the far ends of the data edges leaving `node`, or entering it
/// unless `forward`, and its zone partner from `pair`, counting every incident edge it
/// scans, each source port it looks up and the partner as an edge.
#[expect(
    clippy::too_many_arguments,
    reason = "the session's visit counters are charged with the scan"
)]
fn push_data_edges(
    document: &GraphDocument,
    ports: &PortResolver<'_>,
    index: &DocIndex,
    node: NodeId,
    forward: bool,
    pair: &BTreeMap<NodeId, NodeId>,
    stack: &mut Vec<NodeId>,
    visits: &mut VisitCounts,
) {
    for id in index.incident(node) {
        visits.edges += 1;
        let Some(edge) = document.edge(id) else {
            continue;
        };
        let (near, far) = if forward {
            (edge.from.node, edge.to.node)
        } else {
            (edge.to.node, edge.from.node)
        };
        if near != node {
            continue;
        }
        visits.ports += 1;
        if matches!(ports.port(&edge.from), Ok(port) if port.kind == PortKind::Data) {
            stack.push(far);
        }
    }
    if let Some(partner) = pair.get(&node) {
        visits.edges += 1;
        stack.push(*partner);
    }
}

/// One side of an insertion's two-way search: a depth-first walk over the nodes whose
/// positions lie between the new edge's endpoints.
struct Walk {
    goal: NodeId,
    forward: bool,
    seen: BTreeSet<NodeId>,
    stack: Vec<NodeId>,
}

enum Step {
    Going,
    Done,
    Cycle,
}

impl TopoOrder {
    /// The order of `body`'s nodes. If its data edges join nodes in a cycle, which a
    /// port-level acyclic graph may still do through a node whose outputs do not read every
    /// input, it is `None` unless `hold`; then, whenever no node is free to place next, the
    /// lowest unplaced node is placed and its data edges from unplaced nodes are held. It
    /// is `None` too if a zone pair would have to be held.
    pub(super) fn build(
        document: &GraphDocument,
        ports: &PortResolver<'_>,
        body: BodyRef,
        zones: &super::zones::ZoneIndex,
        hold: bool,
        visits: &mut VisitCounts,
    ) -> Option<Self> {
        let body = document.body(body)?;
        // The body's node list, then each edge with its source port, once more when the
        // body is cyclic, and each node again as it is placed.
        visits.bodies += 1;
        visits.nodes += 2 * body.nodes.len();
        visits.edges += body.edges.len();
        visits.ports += body.edges.len();
        let ends: BTreeMap<NodeId, NodeId> =
            zones.iter().map(|pair| (pair.begin, pair.end)).collect();
        let begins: BTreeMap<NodeId, NodeId> =
            zones.iter().map(|pair| (pair.end, pair.begin)).collect();
        // Dense indices over the sorted node IDs, and each data edge resolved once. A body
        // keeps its nodes in insertion order, so the IDs are sorted here.
        let mut ids: Vec<NodeId> = body.nodes.keys().copied().collect();
        ids.sort_unstable();
        let dense = |node: NodeId| ids.binary_search(&node).ok();
        let mut next: Vec<Vec<usize>> = vec![Vec::new(); ids.len()];
        let mut indegree = vec![0_usize; ids.len()];
        // Each node's incoming data edges, with `None` for a zone pair; only a cyclic
        // body needs them.
        let mut incoming: Vec<Vec<(usize, Option<EdgeId>)>> = Vec::new();
        for edge in body.edges.values() {
            let (Some(from), Some(to)) = (dense(edge.from.node), dense(edge.to.node)) else {
                continue;
            };
            if matches!(ports.port(&edge.from), Ok(port) if port.kind == PortKind::Data) {
                next[from].push(to);
                indegree[to] += 1;
            }
        }
        for (begin, end) in &ends {
            if let (Some(from), Some(to)) = (dense(*begin), dense(*end)) {
                next[from].push(to);
                indegree[to] += 1;
            }
        }
        let mut ready: Vec<usize> = (0..ids.len()).filter(|at| indegree[*at] == 0).collect();
        let mut placed = vec![false; ids.len()];
        let mut nodes = BTreeMap::new();
        let mut held = BTreeMap::new();
        let mut lowest = 0;
        loop {
            while let Some(node) = ready.pop() {
                placed[node] = true;
                nodes.insert(GAP * (nodes.len() as u64 + 1), ids[node]);
                for &to in &next[node] {
                    // A node placed with edges held still counts them.
                    if placed[to] {
                        continue;
                    }
                    indegree[to] -= 1;
                    if indegree[to] == 0 {
                        ready.push(to);
                    }
                }
            }
            if nodes.len() == ids.len() {
                break;
            }
            if !hold {
                return None;
            }
            if incoming.is_empty() {
                visits.edges += body.edges.len();
                visits.ports += body.edges.len();
                incoming = vec![Vec::new(); ids.len()];
                for edge in body.edges.values() {
                    let (Some(from), Some(to)) = (dense(edge.from.node), dense(edge.to.node))
                    else {
                        continue;
                    };
                    if matches!(ports.port(&edge.from), Ok(port) if port.kind == PortKind::Data) {
                        incoming[to].push((from, Some(edge.id)));
                    }
                }
                for (begin, end) in &ends {
                    if let (Some(from), Some(to)) = (dense(*begin), dense(*end)) {
                        incoming[to].push((from, None));
                    }
                }
            }
            while placed[lowest] {
                lowest += 1;
            }
            for &(from, edge) in &incoming[lowest] {
                if !placed[from] {
                    held.insert(edge?, (ids[from], ids[lowest]));
                }
            }
            ready.push(lowest);
        }
        Some(Self {
            position: nodes.iter().map(|(at, node)| (*node, *at)).collect(),
            nodes,
            ends,
            begins,
            held,
        })
    }

    pub(super) fn position(&self, node: NodeId) -> Option<u64> {
        self.position.get(&node).copied()
    }

    /// Whether some data edge is held out of the order, so the body may have a cycle.
    pub(super) fn is_cyclic(&self) -> bool {
        !self.held.is_empty()
    }

    /// Holds a data edge `from -> to` that [`Self::insert`] could not place.
    pub(super) fn hold(&mut self, edge: EdgeId, from: NodeId, to: NodeId) {
        self.held.insert(edge, (from, to));
    }

    /// Forgets a removed edge, if it was held.
    pub(super) fn release(&mut self, edge: EdgeId) {
        self.held.remove(&edge);
    }

    /// Places every held edge that no longer closes a cycle, after an edit removed edges
    /// or nodes, and forgets held edges the document no longer has.
    pub(super) fn retry(
        &mut self,
        document: &GraphDocument,
        ports: &PortResolver<'_>,
        index: &DocIndex,
        visits: &mut VisitCounts,
    ) {
        for (edge, (from, to)) in std::mem::take(&mut self.held) {
            if document.edge(edge).is_some()
                && !self.insert(document, ports, index, from, to, visits)
            {
                self.held.insert(edge, (from, to));
            }
        }
    }

    /// Every node that can lie on a data cycle of the body, or `None` past the budget.
    ///
    /// Each cycle runs through a held edge, and every other edge runs forward in the
    /// order, so a cycle is a chain of forward paths, each from a held edge's target to a
    /// held edge's source. Its nodes therefore lie at or after the lowest held target and
    /// at or before the highest held source, and each is reached forward from a held
    /// target and backward from a held source without leaving that interval. Data edges
    /// are followed as the order counts them, which is coarser than the port-level graph.
    pub(super) fn cyclic_region(
        &self,
        document: &GraphDocument,
        ports: &PortResolver<'_>,
        index: &DocIndex,
        visits: &mut VisitCounts,
    ) -> Option<BTreeSet<NodeId>> {
        let mut low = u64::MAX;
        let mut high = 0;
        for (from, to) in self.held.values() {
            low = low.min(self.position(*to)?);
            high = high.max(self.position(*from)?);
        }
        let mut visited = 0;
        let mut reach = |forward: bool| -> Option<BTreeSet<NodeId>> {
            let mut seen = BTreeSet::new();
            let mut stack: Vec<NodeId> = self
                .held
                .values()
                .map(|(from, to)| if forward { *to } else { *from })
                .collect();
            while let Some(node) = stack.pop() {
                if !self
                    .position(node)
                    .is_some_and(|at| (low..=high).contains(&at))
                    || !seen.insert(node)
                {
                    continue;
                }
                visited += 1;
                visits.nodes += 1;
                if visited > CYCLE_REGION_BUDGET {
                    return None;
                }
                let pair = if forward { &self.ends } else { &self.begins };
                push_data_edges(
                    document, ports, index, node, forward, pair, &mut stack, visits,
                );
            }
            Some(seen)
        };
        let after = reach(true)?;
        let before = reach(false)?;
        Some(after.intersection(&before).copied().collect())
    }

    /// How many checked zone pairs the order holds.
    pub(super) fn zone_pairs(&self) -> usize {
        self.ends.len()
    }

    /// The zone pairs (begin, end) with both markers among `nodes`.
    pub(super) fn zones_among<'a>(
        &'a self,
        nodes: &'a BTreeSet<NodeId>,
    ) -> impl Iterator<Item = (NodeId, NodeId)> + 'a {
        self.ends
            .iter()
            .filter(|(begin, end)| nodes.contains(begin) && nodes.contains(end))
            .map(|(begin, end)| (*begin, *end))
    }

    /// Places a new node, which has no edges yet, last.
    pub(super) fn push(&mut self, node: NodeId) {
        let at = self
            .nodes
            .last_key_value()
            .map_or(GAP, |(last, _)| last.saturating_add(GAP));
        if self.nodes.contains_key(&at) {
            self.relabel(&[node], None);
        } else {
            self.position.insert(node, at);
            self.nodes.insert(at, node);
        }
    }

    pub(super) fn remove(&mut self, node: NodeId) {
        if let Some(at) = self.position.remove(&node) {
            self.nodes.remove(&at);
        }
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
        visits: &mut VisitCounts,
    ) -> bool {
        visits.edges += 1;
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
        // these can be out of order. Either set, once complete, can move past the other
        // endpoint on its own: what `to` reaches to just after `from`, or what reaches
        // `from` to just before `to`. Every accounted edge leaving the first set ends in it
        // or past `from`, and every one entering the second starts in it or before `to`.
        let mut walks = [Walk::new(to, from, true), Walk::new(from, to, false)];
        let moved = 'search: loop {
            for walk in &mut walks {
                match walk.step(self, document, ports, index, lower..=upper, visits) {
                    Step::Going => {}
                    Step::Done => break 'search walk,
                    Step::Cycle => return false,
                }
            }
        };
        let mut nodes: Vec<(u64, NodeId)> = moved
            .seen
            .iter()
            .map(|node| (self.position[node], *node))
            .collect();
        nodes.sort_unstable();
        let nodes: Vec<NodeId> = nodes.into_iter().map(|(_, node)| node).collect();
        for node in &nodes {
            self.remove(*node);
        }
        if moved.forward {
            let after = self.nodes.range(upper + 1..).next().map(|(at, _)| *at);
            self.place(&nodes, upper, after, Some(from), true);
        } else {
            let before = self.nodes.range(..lower).next_back().map(|(at, _)| *at);
            self.place(&nodes, before.unwrap_or(0), Some(lower), Some(to), false);
        }
        true
    }

    /// Places `nodes`, in this order, strictly between the positions `low` and `high` (or
    /// anywhere past `low` without one), next to `anchor`, renumbering every node if the gap
    /// is too narrow.
    fn place(
        &mut self,
        nodes: &[NodeId],
        low: u64,
        high: Option<u64>,
        anchor: Option<NodeId>,
        after: bool,
    ) {
        let count = nodes.len() as u64;
        let high = high.unwrap_or_else(|| low.saturating_add(GAP.saturating_mul(count + 1)));
        let step = (high - low) / (count + 1);
        if step == 0 {
            self.relabel(nodes, anchor.map(|anchor| (anchor, after)));
            return;
        }
        for (at, node) in (1..).map(|k| low + step * k).zip(nodes) {
            self.position.insert(*node, at);
            self.nodes.insert(at, *node);
        }
    }

    /// Renumbers every node `GAP` apart, with `nodes` placed next to the anchor, after it
    /// when the flag says so, or last without one.
    fn relabel(&mut self, nodes: &[NodeId], anchor: Option<(NodeId, bool)>) {
        let mut order: Vec<NodeId> = std::mem::take(&mut self.nodes).into_values().collect();
        let at = anchor
            .and_then(|(anchor, after)| {
                let at = order.iter().position(|node| *node == anchor)?;
                Some(at + usize::from(after))
            })
            .unwrap_or(order.len());
        order.splice(at..at, nodes.iter().copied());
        self.position.clear();
        for (k, node) in order.into_iter().enumerate() {
            let at = GAP * (k as u64 + 1);
            self.position.insert(node, at);
            self.nodes.insert(at, node);
        }
    }
}

impl Walk {
    fn new(start: NodeId, goal: NodeId, forward: bool) -> Self {
        Self {
            goal,
            forward,
            seen: BTreeSet::new(),
            stack: vec![start],
        }
    }

    /// Visits the next node on the stack, if it lies within `interval`.
    fn step(
        &mut self,
        order: &TopoOrder,
        document: &GraphDocument,
        ports: &PortResolver<'_>,
        index: &DocIndex,
        interval: std::ops::RangeInclusive<u64>,
        visits: &mut VisitCounts,
    ) -> Step {
        let Some(node) = self.stack.pop() else {
            return Step::Done;
        };
        if node == self.goal {
            return Step::Cycle;
        }
        if order
            .position(node)
            .is_some_and(|at| interval.contains(&at))
            && self.seen.insert(node)
        {
            visits.nodes += 1;
            let pair = if self.forward {
                &order.ends
            } else {
                &order.begins
            };
            push_data_edges(
                document,
                ports,
                index,
                node,
                self.forward,
                pair,
                &mut self.stack,
                visits,
            );
        }
        Step::Going
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(n: u32) -> NodeId {
        format!("10000000-0000-4000-8000-{n:012x}").parse().unwrap()
    }

    #[test]
    fn placing_into_a_full_gap_renumbers_around_the_anchor() {
        let mut order = TopoOrder::default();
        for n in 1..=3 {
            order.push(node(n));
        }
        let one = order.position(node(1)).unwrap();
        let sequence = |order: &TopoOrder| order.nodes.values().copied().collect::<Vec<_>>();
        // A gap with one free position takes one node; a gap with none renumbers.
        order.remove(node(3));
        order.place(&[node(3)], one, Some(one + 2), Some(node(1)), true);
        assert_eq!(sequence(&order), [node(1), node(3), node(2)]);
        order.remove(node(2));
        order.place(&[node(2)], one, Some(one + 1), Some(node(1)), true);
        assert_eq!(sequence(&order), [node(1), node(2), node(3)]);
        assert_eq!(order.position(node(3)), Some(3 * GAP));
        for (at, node) in &order.nodes {
            assert_eq!(order.position[node], *at);
        }
    }
}
