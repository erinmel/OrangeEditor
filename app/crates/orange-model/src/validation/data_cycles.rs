//! Port-level data-dependency cycle validation (11 §8.2).

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::RegistrySnapshot;
use crate::document::{
    GraphDocument, OutputSource, PortDescriptor, PortDirection, PortKind, PortRef,
    SignatureSnapshot,
};
use crate::id::{NodeId, PortKey};

use super::connections::ConnectionValidation;
use super::diagnostic::{
    Diagnostic, DiagnosticCode, HostOrigin, Severity, Subject, sort_diagnostics,
};
use super::ports::lookup_port;
use super::scope::Scope;

/// Result of checking the data-dependency graph using current resolved signatures.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataCycleValidation {
    pub diagnostics: Vec<Diagnostic>,
    /// One deterministic closed witness path for each cyclic strongly connected component.
    pub cycle_paths: Vec<Vec<PortRef>>,
    /// Nodes with unavailable metadata needed to construct their dependency edges.
    pub incomplete_nodes: BTreeSet<NodeId>,
}

impl DataCycleValidation {
    /// True only when dependency analysis completed and found no cycles.
    pub fn is_valid(&self) -> bool {
        self.incomplete_nodes.is_empty()
            && self
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity != Severity::Error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Vertex {
    node: NodeId,
    port: PortKey,
}

impl From<&PortRef> for Vertex {
    fn from(value: &PortRef) -> Self {
        Self {
            node: value.node,
            port: value.port.clone(),
        }
    }
}

impl From<Vertex> for PortRef {
    fn from(value: Vertex) -> Self {
        Self {
            node: value.node,
            port: value.port,
        }
    }
}

/// Builds D independently for each body. Stored signature snapshots never supply edges:
/// unresolved nodes and invalid connections remain the responsibility of their earlier
/// passes, and are carried forward as incomplete here.
pub fn validate_data_cycles(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    connections: &ConnectionValidation,
    registry: &RegistrySnapshot,
) -> DataCycleValidation {
    validate_data_cycles_in(document, resolved, connections, registry, Scope::All)
}

/// [`validate_data_cycles`] over the bodies in `scope`.
pub(crate) fn validate_data_cycles_in(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    connections: &ConnectionValidation,
    registry: &RegistrySnapshot,
    scope: Scope,
) -> DataCycleValidation {
    let origin = registry.origin();
    let mut result = DataCycleValidation {
        incomplete_nodes: connections.incomplete_nodes.clone(),
        ..DataCycleValidation::default()
    };
    let mut diagnosed_nodes: BTreeSet<_> = connections
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .filter_map(|diagnostic| diagnostic.subject.node)
        .collect();
    let error_edges: BTreeSet<_> = connections
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .filter_map(|diagnostic| diagnostic.subject.edge)
        .collect();
    for node in &connections.incomplete_nodes {
        if diagnosed_nodes.insert(*node) {
            result.diagnostics.push(Diagnostic::new(
                DiagnosticCode::DependencyUnavailable,
                Subject::new(Some(*node), None, None),
                origin,
            ));
        }
    }

    for (_, body) in scope.bodies(document) {
        let body_nodes: BTreeSet<_> = body.nodes.keys().copied().collect();
        let mut graph = BTreeMap::<Vertex, BTreeSet<Vertex>>::new();

        for node in body.nodes.values() {
            let Some(signature) = resolved.get(&node.id) else {
                mark_incomplete(node.id, origin, &mut result, &mut diagnosed_nodes);
                continue;
            };
            for descriptor in &signature.ports {
                if descriptor.kind != PortKind::Data {
                    continue;
                }
                if descriptor.ty.is_none()
                    || (descriptor.direction == PortDirection::Output
                        && descriptor.source.is_none())
                {
                    mark_incomplete(node.id, origin, &mut result, &mut diagnosed_nodes);
                }
                for vertex in concrete_vertices(node.id, node, descriptor) {
                    graph.entry(vertex).or_default();
                }
            }

            let inputs: Vec<_> = signature
                .ports
                .iter()
                .filter(|port| {
                    port.kind == PortKind::Data
                        && port.direction == PortDirection::Input
                        && port.ty.is_some()
                })
                .flat_map(|port| concrete_vertices(node.id, node, port))
                .collect();
            let outputs: Vec<_> = signature
                .ports
                .iter()
                .filter(|port| {
                    port.kind == PortKind::Data
                        && port.direction == PortDirection::Output
                        && port.source == Some(OutputSource::Computed)
                        && port.ty.is_some()
                })
                .flat_map(|port| concrete_vertices(node.id, node, port))
                .collect();
            for input in &inputs {
                for output in &outputs {
                    add_edge(&mut graph, input.clone(), output.clone());
                }
            }
        }

        let mut edges: Vec<_> = body.edges.values().collect();
        edges.sort_by_key(|edge| edge.id);
        for edge in edges {
            if error_edges.contains(&edge.id)
                || !body_nodes.contains(&edge.from.node)
                || !body_nodes.contains(&edge.to.node)
            {
                continue;
            }
            if !resolved.contains_key(&edge.from.node) || !resolved.contains_key(&edge.to.node) {
                mark_incomplete(edge.from.node, origin, &mut result, &mut diagnosed_nodes);
                mark_incomplete(edge.to.node, origin, &mut result, &mut diagnosed_nodes);
                continue;
            }
            let from = lookup_port(document, resolved, &edge.from);
            let to = lookup_port(document, resolved, &edge.to);
            match (from, to) {
                (Ok(from), Ok(to))
                    if from.direction == PortDirection::Output
                        && to.direction == PortDirection::Input
                        && from.kind == PortKind::Data
                        && to.kind == PortKind::Data
                        && from.ty.is_some()
                        && to.ty.is_some() =>
                {
                    add_edge(&mut graph, Vertex::from(&edge.from), Vertex::from(&edge.to));
                }
                (Ok(from), Ok(to)) if from.kind != PortKind::Data || to.kind != PortKind::Data => {}
                (Err(_), _) | (_, Err(_)) => {
                    mark_incomplete(edge.from.node, origin, &mut result, &mut diagnosed_nodes);
                    mark_incomplete(edge.to.node, origin, &mut result, &mut diagnosed_nodes);
                }
                _ => {}
            }
        }

        for component in strongly_connected_components(&graph) {
            let Some(path) = cycle_witness(&graph, &component) else {
                continue;
            };
            let subject = path.first().map_or_else(
                || Subject::new(None, None, None),
                |port| Subject::new(Some(port.node), Some(port.port.clone()), None),
            );
            result
                .diagnostics
                .push(Diagnostic::new(DiagnosticCode::CycleData, subject, origin));
            result.cycle_paths.push(path);
        }
    }

    sort_diagnostics(&mut result.diagnostics);
    result
}

fn mark_incomplete(
    node: NodeId,
    origin: &HostOrigin,
    result: &mut DataCycleValidation,
    diagnosed_nodes: &mut BTreeSet<NodeId>,
) {
    result.incomplete_nodes.insert(node);
    if diagnosed_nodes.insert(node) {
        result.diagnostics.push(Diagnostic::new(
            DiagnosticCode::DependencyUnavailable,
            Subject::new(Some(node), None, None),
            origin,
        ));
    }
}

fn concrete_vertices(
    node: NodeId,
    instance: &crate::document::NodeInstance,
    descriptor: &PortDescriptor,
) -> Vec<Vertex> {
    if descriptor.variadic.is_some() {
        instance
            .variadic
            .get(&descriptor.port_id)
            .into_iter()
            .flat_map(|state| state.order.iter().copied())
            .map(|key| Vertex {
                node,
                port: PortKey::new(descriptor.port_id.clone(), Some(key)),
            })
            .collect()
    } else {
        vec![Vertex {
            node,
            port: PortKey::new(descriptor.port_id.clone(), None),
        }]
    }
}

fn add_edge(graph: &mut BTreeMap<Vertex, BTreeSet<Vertex>>, from: Vertex, to: Vertex) {
    graph.entry(from.clone()).or_default().insert(to.clone());
    graph.entry(to).or_default();
}

fn strongly_connected_components(
    graph: &BTreeMap<Vertex, BTreeSet<Vertex>>,
) -> Vec<BTreeSet<Vertex>> {
    let mut visited = BTreeSet::new();
    let mut finish = Vec::with_capacity(graph.len());
    for start in graph.keys() {
        if visited.contains(start) {
            continue;
        }
        visited.insert(start.clone());
        let neighbors: Vec<_> = graph
            .get(start)
            .into_iter()
            .flat_map(|items| items.iter().cloned())
            .collect();
        let mut stack = vec![(start.clone(), neighbors, 0_usize)];
        while let Some((vertex, neighbors, cursor)) = stack.last_mut() {
            if *cursor >= neighbors.len() {
                finish.push(vertex.clone());
                stack.pop();
                continue;
            }
            let next = neighbors[*cursor].clone();
            *cursor += 1;
            if visited.insert(next.clone()) {
                let next_neighbors: Vec<_> = graph
                    .get(&next)
                    .into_iter()
                    .flat_map(|items| items.iter().cloned())
                    .collect();
                stack.push((next, next_neighbors, 0));
            }
        }
    }

    let mut reverse = BTreeMap::<Vertex, BTreeSet<Vertex>>::new();
    for (from, neighbors) in graph {
        reverse.entry(from.clone()).or_default();
        for to in neighbors {
            reverse.entry(to.clone()).or_default().insert(from.clone());
        }
    }

    visited.clear();
    let mut components = Vec::new();
    for start in finish.into_iter().rev() {
        if !visited.insert(start.clone()) {
            continue;
        }
        let mut component = BTreeSet::new();
        let mut stack = vec![start];
        while let Some(vertex) = stack.pop() {
            component.insert(vertex.clone());
            if let Some(neighbors) = reverse.get(&vertex) {
                for neighbor in neighbors.iter().rev() {
                    if visited.insert(neighbor.clone()) {
                        stack.push(neighbor.clone());
                    }
                }
            }
        }
        components.push(component);
    }
    components
}

fn cycle_witness(
    graph: &BTreeMap<Vertex, BTreeSet<Vertex>>,
    component: &BTreeSet<Vertex>,
) -> Option<Vec<PortRef>> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Color {
        Gray,
        Black,
    }

    let mut colors = BTreeMap::<Vertex, Color>::new();
    for start in component {
        if colors.contains_key(start) {
            continue;
        }
        colors.insert(start.clone(), Color::Gray);
        let neighbors: Vec<_> = graph
            .get(start)?
            .iter()
            .filter(|vertex| component.contains(*vertex))
            .cloned()
            .collect();
        let mut stack = vec![(start.clone(), neighbors, 0_usize)];
        while let Some((vertex, neighbors, cursor)) = stack.last_mut() {
            if *cursor >= neighbors.len() {
                let vertex = vertex.clone();
                colors.insert(vertex, Color::Black);
                stack.pop();
                continue;
            }
            let next = neighbors[*cursor].clone();
            *cursor += 1;
            match colors.get(&next) {
                Some(Color::Gray) => {
                    let begin = stack.iter().position(|(item, _, _)| item == &next)?;
                    let mut path: Vec<_> = stack[begin..]
                        .iter()
                        .map(|(item, _, _)| PortRef::from(item.clone()))
                        .collect();
                    path.push(PortRef::from(next));
                    return Some(path);
                }
                Some(Color::Black) => {}
                None => {
                    colors.insert(next.clone(), Color::Gray);
                    let next_neighbors: Vec<_> = graph
                        .get(&next)?
                        .iter()
                        .filter(|vertex| component.contains(*vertex))
                        .cloned()
                        .collect();
                    stack.push((next, next_neighbors, 0));
                }
            }
        }
    }
    None
}
