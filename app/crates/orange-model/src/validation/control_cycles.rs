//! Control-cycle validation (11 §8.1, §8.2 "C").

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::RegistrySnapshot;
use crate::document::{
    BodyRef, GraphDocument, PortDirection, PortKind, SignatureSnapshot, Suspends,
};
use crate::id::NodeId;

use super::connections::ConnectionValidation;
use super::diagnostic::{Diagnostic, DiagnosticCode, Severity, Subject, sort_diagnostics};
use super::ports::{PortLookupError, lookup_port};

/// Result of checking explicit exec-edge cycles for guaranteed suspension.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ControlCycleValidation {
    pub diagnostics: Vec<Diagnostic>,
    /// One deterministic closed node path per offending component, starting at its lowest node.
    pub cycle_paths: Vec<Vec<NodeId>>,
    /// Nodes whose metadata was needed to build exec edges but is unavailable.
    pub incomplete_nodes: BTreeSet<NodeId>,
}

impl ControlCycleValidation {
    /// True only when analysis completed and found no unsuspended control cycle.
    pub fn is_valid(&self) -> bool {
        self.incomplete_nodes.is_empty()
            && self
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity != Severity::Error)
    }
}

type Graph = BTreeMap<NodeId, BTreeSet<NodeId>>;

/// Builds C per body from valid exec and event-to-exec edges between fresh resolved nodes.
///
/// Implicit structured-loop back edges are not edges of the document, so they never form a
/// cycle here. Every explicit cycle needs a node whose resolved `suspends` is `always`; a
/// `maybe`, unknown or unresolved node never certifies. The check removes certifying nodes and
/// reports each remaining cyclic component, which is exactly "some cycle avoids them".
pub fn validate_control_cycles(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    connections: &ConnectionValidation,
    registry: &RegistrySnapshot,
) -> ControlCycleValidation {
    let origin = registry.origin();
    let mut result = ControlCycleValidation {
        incomplete_nodes: connections.incomplete_nodes.clone(),
        ..ControlCycleValidation::default()
    };
    let error_edges: BTreeSet<_> = connections
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .filter_map(|diagnostic| diagnostic.subject.edge)
        .collect();
    let certified = |node: NodeId| {
        resolved
            .get(&node)
            .is_some_and(|signature| signature.suspension() == Some(Suspends::Always))
    };
    for (body_ref, body) in document.bodies_with_refs() {
        let function = match body_ref {
            BodyRef::Root => None,
            BodyRef::Function(id) => Some(id),
        };
        let mut graph = Graph::new();
        let mut edges: Vec<_> = body.edges.values().collect();
        edges.sort_by_key(|edge| edge.id);
        for edge in edges {
            if error_edges.contains(&edge.id)
                || !body.nodes.contains_key(&edge.from.node)
                || !body.nodes.contains_key(&edge.to.node)
            {
                continue;
            }
            match (
                lookup_port(document, resolved, &edge.from),
                lookup_port(document, resolved, &edge.to),
            ) {
                (Ok(from), Ok(to))
                    if from.direction == PortDirection::Output
                        && to.direction == PortDirection::Input
                        && to.kind == PortKind::Exec
                        && matches!(from.kind, PortKind::Exec | PortKind::Event) =>
                {
                    if !certified(edge.from.node) && !certified(edge.to.node) {
                        graph
                            .entry(edge.from.node)
                            .or_default()
                            .insert(edge.to.node);
                    }
                }
                (Err(PortLookupError::MissingMetadata), _)
                | (_, Err(PortLookupError::MissingMetadata)) => {
                    result.incomplete_nodes.insert(edge.from.node);
                    result.incomplete_nodes.insert(edge.to.node);
                }
                _ => {}
            }
        }
        for component in cyclic_components(&graph) {
            let path = cycle_witness(&graph, &component);
            let subject = Subject::new(path.first().copied(), None, None);
            let subject = match function {
                Some(function) => subject.with_function(function),
                None => subject,
            };
            result.diagnostics.push(Diagnostic::new(
                DiagnosticCode::CycleSyncControl,
                subject,
                origin,
            ));
            result.cycle_paths.push(path);
        }
    }
    sort_diagnostics(&mut result.diagnostics);
    result
}

struct Frame {
    node: NodeId,
    next: Vec<NodeId>,
    cursor: usize,
}

fn successors(graph: &Graph, node: NodeId) -> Vec<NodeId> {
    graph.get(&node).into_iter().flatten().copied().collect()
}

#[allow(clippy::too_many_lines)]
/// Iterative Tarjan; keeps only components with a cycle (several nodes or a self-loop).
fn cyclic_components(graph: &Graph) -> Vec<BTreeSet<NodeId>> {
    let mut index = BTreeMap::<NodeId, usize>::new();
    let mut low = BTreeMap::<NodeId, usize>::new();
    let mut on_stack = BTreeSet::new();
    let mut scc_stack = Vec::new();
    let mut components = Vec::new();
    let mut counter = 0;
    for &start in graph.keys() {
        if index.contains_key(&start) {
            continue;
        }
        index.insert(start, counter);
        low.insert(start, counter);
        counter += 1;
        on_stack.insert(start);
        scc_stack.push(start);
        let mut frames = vec![Frame {
            node: start,
            next: successors(graph, start),
            cursor: 0,
        }];
        while let Some(frame) = frames.last_mut() {
            if let Some(&next) = frame.next.get(frame.cursor) {
                frame.cursor += 1;
                let node = frame.node;
                if let Some(&seen) = index.get(&next) {
                    if on_stack.contains(&next) {
                        let value = low[&node].min(seen);
                        low.insert(node, value);
                    }
                } else {
                    index.insert(next, counter);
                    low.insert(next, counter);
                    counter += 1;
                    on_stack.insert(next);
                    scc_stack.push(next);
                    frames.push(Frame {
                        node: next,
                        next: successors(graph, next),
                        cursor: 0,
                    });
                }
                continue;
            }
            let node = frame.node;
            frames.pop();
            if let Some(parent) = frames.last() {
                let value = low[&parent.node].min(low[&node]);
                low.insert(parent.node, value);
            }
            if low[&node] == index[&node] {
                let mut component = BTreeSet::new();
                while let Some(member) = scc_stack.pop() {
                    on_stack.remove(&member);
                    component.insert(member);
                    if member == node {
                        break;
                    }
                }
                let cyclic = component.len() > 1
                    || graph.get(&node).is_some_and(|items| items.contains(&node));
                if cyclic {
                    components.push(component);
                }
            }
        }
    }
    components.sort_by_key(|component| component.first().copied());
    components
}

/// Closed path through `component`, starting at its lowest node, found by depth-first search
/// over ordered successors so the result is deterministic.
fn cycle_witness(graph: &Graph, component: &BTreeSet<NodeId>) -> Vec<NodeId> {
    let start = *component.first().expect("components are non-empty");
    let inside = |node: NodeId| -> Vec<NodeId> {
        successors(graph, node)
            .into_iter()
            .filter(|next| component.contains(next))
            .collect()
    };
    let mut visited = BTreeSet::from([start]);
    let mut stack = vec![(start, inside(start), 0_usize)];
    while let Some((_, next, cursor)) = stack.last_mut() {
        let Some(&target) = next.get(*cursor) else {
            stack.pop();
            continue;
        };
        *cursor += 1;
        if target == start {
            let mut path: Vec<_> = stack.iter().map(|(node, _, _)| *node).collect();
            path.push(start);
            return path;
        }
        if visited.insert(target) {
            stack.push((target, inside(target), 0));
        }
    }
    vec![start, start]
}
