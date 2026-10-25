//! Control-cycle validation (11 §8.1, §8.2 "C").

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::RegistrySnapshot;
use crate::document::{
    BodyRef, GraphDocument, PortDirection, PortKind, SignatureSnapshot, Suspends,
};
use crate::id::NodeId;

use super::connections::ConnectionValidation;
use super::diagnostic::{Diagnostic, DiagnosticCode, Severity, Subject, sort_diagnostics};
use super::ports::{PortLookupError, PortResolver};
use super::scope::Scope;

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
    let ports = PortResolver::new(document, resolved);

    validate_control_cycles_in(document, &ports, connections, registry, Scope::All)
}

/// [`validate_control_cycles`] over the bodies in `scope`.
pub(crate) fn validate_control_cycles_in(
    document: &GraphDocument,
    resolved: &PortResolver<'_>,
    connections: &ConnectionValidation,
    registry: &RegistrySnapshot,
    scope: Scope,
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
    for (body_ref, body) in scope.bodies(document) {
        let function = match body_ref {
            BodyRef::Root => None,
            BodyRef::Function(id) => Some(id),
        };
        let mut full = Graph::new();
        let mut edges: Vec<_> = body.edges.values().collect();
        edges.sort_by_key(|edge| edge.id);
        for edge in edges {
            if error_edges.contains(&edge.id)
                || !body.nodes.contains_key(&edge.from.node)
                || !body.nodes.contains_key(&edge.to.node)
            {
                continue;
            }
            match (resolved.port(&edge.from), resolved.port(&edge.to)) {
                (Ok(from), Ok(to))
                    if from.direction == PortDirection::Output
                        && to.direction == PortDirection::Input
                        && to.kind == PortKind::Exec
                        && matches!(from.kind, PortKind::Exec | PortKind::Event) =>
                {
                    full.entry(edge.from.node).or_default().insert(edge.to.node);
                }
                (Err(PortLookupError::MissingMetadata), _)
                | (_, Err(PortLookupError::MissingMetadata)) => {
                    result.incomplete_nodes.insert(edge.from.node);
                    result.incomplete_nodes.insert(edge.to.node);
                }
                _ => {}
            }
        }
        let strict = filtered(&full, &certified);
        let mut errors = BTreeSet::new();
        for component in cyclic_components(&strict) {
            let path = cycle_witness(&strict, &component);
            errors.extend(component);
            let subject = cycle_subject(function, &path);
            result.diagnostics.push(Diagnostic::new(
                DiagnosticCode::CycleSyncControl,
                subject,
                origin,
            ));
            result.cycle_paths.push(path);
        }
        // Cycles certified only by yields are legal but busy (11 §8.1).
        let lenient = filtered(&full, &|node| certified(node) && !is_yield(document, node));
        for component in cyclic_components(&lenient) {
            if component.iter().any(|node| errors.contains(node)) {
                continue;
            }
            let path = cycle_witness(&lenient, &component);
            let subject = cycle_subject(function, &path);
            result
                .diagnostics
                .push(Diagnostic::new(DiagnosticCode::BusyLoop, subject, origin));
        }
    }
    sort_diagnostics(&mut result.diagnostics);
    result
}

fn is_yield(document: &GraphDocument, node: NodeId) -> bool {
    document
        .node(node)
        .is_some_and(|instance| instance.definition.id.as_str() == "core/yield")
}

fn cycle_subject(function: Option<crate::id::FunctionId>, path: &[NodeId]) -> Subject {
    let subject = Subject::new(path.first().copied(), None, None);
    match function {
        Some(function) => subject.with_function(function),
        None => subject,
    }
}

/// `graph` without the nodes `removed` selects.
fn filtered(graph: &Graph, removed: &dyn Fn(NodeId) -> bool) -> Graph {
    let mut result = Graph::new();
    for (from, targets) in graph {
        if removed(*from) {
            continue;
        }
        for to in targets.iter().filter(|to| !removed(**to)) {
            result.entry(*from).or_default().insert(*to);
        }
    }
    result
}

fn successors(graph: &Graph, node: NodeId) -> Vec<NodeId> {
    graph.get(&node).into_iter().flatten().copied().collect()
}

/// Ordered cyclic components; suspension filtering remains control-specific.
fn cyclic_components(graph: &Graph) -> Vec<BTreeSet<NodeId>> {
    let mut components: Vec<_> = super::graph::strongly_connected_components(graph)
        .into_iter()
        .filter(|component| {
            component.len() > 1
                || component
                    .first()
                    .is_some_and(|node| graph.get(node).is_some_and(|edges| edges.contains(node)))
        })
        .collect();
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
