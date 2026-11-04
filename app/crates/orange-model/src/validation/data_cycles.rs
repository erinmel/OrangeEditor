//! Port-level data-dependency cycle validation (11 §8.2).

use super::graph::dense_strongly_connected_components;
use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::RegistrySnapshot;
use crate::document::{
    GraphDocument, OutputSource, PortDescriptor, PortDirection, PortKind, PortRef,
    SignatureSnapshot,
};
use crate::id::{NodeId, PortId, PortKey};

use super::connections::ConnectionValidation;
use super::diagnostic::{
    Diagnostic, DiagnosticCode, HostOrigin, Severity, Subject, sort_diagnostics,
};
use super::ports::PortResolver;
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

/// A dependency vertex borrowing its port name. Fields order as `PortRef` does, node then
/// port then instance, so vertex order and witnesses match an owned port key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Vertex<'a> {
    // Keep real ports first so deterministic witnesses begin at a visible port.
    Port {
        node: NodeId,
        port: &'a PortId,
        instance: Option<u16>,
    },
    Computation(NodeId),
}

impl<'a> From<&'a PortRef> for Vertex<'a> {
    fn from(value: &'a PortRef) -> Self {
        Self::Port {
            node: value.node,
            port: value.port.port(),
            instance: value.port.instance(),
        }
    }
}

impl Vertex<'_> {
    fn port_ref(&self) -> Option<PortRef> {
        match self {
            Self::Port {
                node,
                port,
                instance,
            } => Some(PortRef {
                node: *node,
                port: PortKey::new((*port).clone(), *instance),
            }),
            Self::Computation(_) => None,
        }
    }
}

/// One body's dependency graph as collected vertices and edges; [`DependencyGraph::dense`]
/// orders and deduplicates them as an ordered map of ordered neighbour sets would.
#[derive(Default)]
struct DependencyGraph<'a> {
    vertices: Vec<Vertex<'a>>,
    edges: Vec<(Vertex<'a>, Vertex<'a>)>,
}

impl<'a> DependencyGraph<'a> {
    fn vertex(&mut self, vertex: Vertex<'a>) {
        self.vertices.push(vertex);
    }

    fn edge(&mut self, from: Vertex<'a>, to: Vertex<'a>) {
        self.vertices.push(from);
        self.vertices.push(to);
        self.edges.push((from, to));
    }

    /// Ascending vertices and each one's ascending, distinct neighbour indices.
    fn dense(mut self) -> (Vec<Vertex<'a>>, Vec<Vec<usize>>) {
        self.vertices.sort_unstable();
        self.vertices.dedup();
        let index = |vertex: &Vertex<'a>| self.vertices.binary_search(vertex).unwrap_or(0);
        let mut edges: Vec<(usize, usize)> = self
            .edges
            .iter()
            .map(|(from, to)| (index(from), index(to)))
            .collect();
        edges.sort_unstable();
        edges.dedup();
        let mut adjacency = vec![Vec::new(); self.vertices.len()];
        for (from, to) in edges {
            adjacency[from].push(to);
        }
        (self.vertices, adjacency)
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
    let ports = PortResolver::new(document, resolved);

    validate_data_cycles_in(document, &ports, connections, registry, Scope::All)
}

/// [`validate_data_cycles`] over the bodies in `scope`.
pub(crate) fn validate_data_cycles_in(
    document: &GraphDocument,
    resolved: &PortResolver<'_>,
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
        let mut graph = DependencyGraph::default();
        let mut incomplete = Vec::new();

        for node in body.nodes.values() {
            graph.node(resolved, node, &mut incomplete);
        }

        // A zone result also depends on its strict prefix (count/items/initial).
        // Begin iteration projections themselves remain independent of those
        // inputs: adding input-to-iteration edges would reject legal carry.
        for pair in super::zones::pair_zones(body, None).pairs {
            graph.zone(resolved, body, pair.begin, pair.end);
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
            graph.data_edge(resolved, edge, &mut incomplete);
        }
        for node in incomplete {
            mark_incomplete(node, origin, &mut result, &mut diagnosed_nodes);
        }

        for (diagnostic, path) in graph.cycles(origin) {
            result.diagnostics.push(diagnostic);
            result.cycle_paths.push(path);
        }
    }

    sort_diagnostics(&mut result.diagnostics);
    sort_cycle_paths(&mut result.cycle_paths);
    result
}

/// Orders witnesses by their first port, which no two cyclic components share, so the
/// order does not depend on how the components were found (16 §5.3).
pub(super) fn sort_cycle_paths(paths: &mut [Vec<PortRef>]) {
    paths.sort_by(|a, b| {
        let first = |path: &[PortRef]| path.first().map(|port| (port.node, port.port.clone()));
        first(a).cmp(&first(b))
    });
}

/// The data cycles among `nodes` of `body` alone, each with its diagnostic and witness,
/// built and chosen exactly as [`validate_data_cycles`] builds and chooses them for the
/// whole body: from the nodes' data ports and computations, the prefix dependencies of
/// the `zones` (begin, end) pairs, and the body edges joining two of the nodes. Every
/// strongly connected component the whole body's graph has inside these nodes is found
/// with the same vertices, edges and vertex order, so its witness is the same, provided
/// each cycle of the body lies wholly among them. `None` if a node, its ports or one of
/// the edges would be incomplete there, which only the whole pass reports.
pub(super) fn cycles_among<'a>(
    resolved: &'a PortResolver<'_>,
    body: &'a crate::document::GraphBody,
    nodes: &BTreeSet<NodeId>,
    zones: impl IntoIterator<Item = (NodeId, NodeId)>,
    edges: impl IntoIterator<Item = &'a crate::document::Edge>,
    origin: &HostOrigin,
) -> Option<Vec<(Diagnostic, Vec<PortRef>)>> {
    let mut graph = DependencyGraph::default();
    let mut incomplete = Vec::new();
    for id in nodes {
        graph.node(resolved, body.nodes.get(id)?, &mut incomplete);
    }
    for (begin, end) in zones {
        graph.zone(resolved, body, begin, end);
    }
    for edge in edges {
        graph.data_edge(resolved, edge, &mut incomplete);
    }
    incomplete
        .is_empty()
        .then(|| graph.cycles(origin).collect())
}

impl<'a> DependencyGraph<'a> {
    /// A node's data port vertices and its input-to-computed-output dependencies; nodes
    /// whose metadata cannot supply them are pushed to `incomplete`.
    fn node(
        &mut self,
        resolved: &'a PortResolver<'_>,
        node: &'a crate::document::NodeInstance,
        incomplete: &mut Vec<NodeId>,
    ) {
        let Some(signature) = resolved.get(&node.id) else {
            incomplete.push(node.id);
            return;
        };
        for descriptor in &signature.ports {
            if descriptor.kind != PortKind::Data {
                continue;
            }
            if descriptor.ty.is_none()
                || (descriptor.direction == PortDirection::Output && descriptor.source.is_none())
            {
                incomplete.push(node.id);
            }
            for vertex in concrete_vertices(node.id, node, descriptor) {
                self.vertex(vertex);
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
        // A private hub represents the complete input-to-computed-output relation
        // using O(inputs + outputs) edges rather than O(inputs * outputs).
        if inputs.len().saturating_mul(outputs.len()) <= inputs.len().saturating_add(outputs.len())
        {
            // Narrow computations are already linear; avoid adding a hub there.
            for &input in &inputs {
                for &output in &outputs {
                    self.edge(input, output);
                }
            }
        } else {
            let hub = Vertex::Computation(node.id);
            for input in inputs {
                self.edge(input, hub);
            }
            for output in outputs {
                self.edge(hub, output);
            }
        }
    }

    /// A zone result's dependency on each begin input.
    fn zone(
        &mut self,
        resolved: &'a PortResolver<'_>,
        body: &'a crate::document::GraphBody,
        begin: NodeId,
        end: NodeId,
    ) {
        let (Some(begin_signature), Some(end_signature)) =
            (resolved.get(&begin), resolved.get(&end))
        else {
            return;
        };
        let (Some(begin_node), Some(end_node)) = (body.nodes.get(&begin), body.nodes.get(&end))
        else {
            return;
        };
        let inputs = begin_signature.ports.iter().filter(|port| {
            port.kind == PortKind::Data
                && port.direction == PortDirection::Input
                && port.ty.is_some()
        });
        let outputs: Vec<_> = end_signature
            .ports
            .iter()
            .filter(|port| {
                port.kind == PortKind::Data
                    && port.direction == PortDirection::Output
                    && port.source == Some(OutputSource::Computed)
                    && port.ty.is_some()
            })
            .collect();
        for input in inputs {
            for from in concrete_vertices(begin, begin_node, input) {
                for output in &outputs {
                    for to in concrete_vertices(end, end_node, output) {
                        self.edge(from, to);
                    }
                }
            }
        }
    }

    /// A body edge between two of the body's nodes, if it joins typed data ports; its
    /// endpoints are pushed to `incomplete` if their ports do not resolve.
    fn data_edge(
        &mut self,
        resolved: &'a PortResolver<'_>,
        edge: &'a crate::document::Edge,
        incomplete: &mut Vec<NodeId>,
    ) {
        if !resolved.contains_key(&edge.from.node) || !resolved.contains_key(&edge.to.node) {
            incomplete.extend([edge.from.node, edge.to.node]);
            return;
        }
        match (resolved.port(&edge.from), resolved.port(&edge.to)) {
            (Ok(from), Ok(to))
                if from.direction == PortDirection::Output
                    && to.direction == PortDirection::Input
                    && from.kind == PortKind::Data
                    && to.kind == PortKind::Data
                    && from.ty.is_some()
                    && to.ty.is_some() =>
            {
                self.edge(Vertex::from(&edge.from), Vertex::from(&edge.to));
            }
            (Ok(from), Ok(to)) if from.kind != PortKind::Data || to.kind != PortKind::Data => {}
            (Err(_), _) | (_, Err(_)) => incomplete.extend([edge.from.node, edge.to.node]),
            _ => {}
        }
    }

    /// Each cyclic strongly connected component's diagnostic and witness.
    fn cycles(self, origin: &HostOrigin) -> impl Iterator<Item = (Diagnostic, Vec<PortRef>)> {
        let (vertices, adjacency) = self.dense();
        let mut member = vec![usize::MAX; vertices.len()];
        let mut found = Vec::new();
        for (index, component) in dense_strongly_connected_components(&adjacency)
            .into_iter()
            .enumerate()
        {
            // A single vertex is cyclic only through an edge to itself.
            if let [vertex] = component[..]
                && !adjacency[vertex].contains(&vertex)
            {
                continue;
            }
            for &vertex in &component {
                member[vertex] = index;
            }
            let Some(path) = cycle_witness(&vertices, &adjacency, &component, &member, index)
            else {
                continue;
            };
            let subject = path.first().map_or_else(
                || Subject::new(None, None, None),
                |port| Subject::new(Some(port.node), Some(port.port.clone()), None),
            );
            found.push((
                Diagnostic::new(DiagnosticCode::CycleData, subject, origin),
                path,
            ));
        }
        found.into_iter()
    }
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

fn concrete_vertices<'a>(
    node: NodeId,
    instance: &crate::document::NodeInstance,
    descriptor: &'a PortDescriptor,
) -> Vec<Vertex<'a>> {
    if descriptor.variadic.is_some() {
        instance
            .variadic
            .get(&descriptor.port_id)
            .into_iter()
            .flat_map(|state| state.order.iter().copied())
            .map(|key| Vertex::Port {
                node,
                port: &descriptor.port_id,
                instance: Some(key),
            })
            .collect()
    } else {
        vec![Vertex::Port {
            node,
            port: &descriptor.port_id,
            instance: None,
        }]
    }
}

/// The first cycle found by a depth-first search of `component` (ascending dense vertices
/// whose `member` entry is `index`), from its lowest vertex and through ascending neighbours.
fn cycle_witness(
    vertices: &[Vertex<'_>],
    adjacency: &[Vec<usize>],
    component: &[usize],
    member: &[usize],
    index: usize,
) -> Option<Vec<PortRef>> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Color {
        White,
        Gray,
        Black,
    }

    let mut colors = vec![Color::White; vertices.len()];
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for &start in component {
        if colors[start] != Color::White {
            continue;
        }
        colors[start] = Color::Gray;
        stack.push((start, 0));
        while let Some((vertex, cursor)) = stack.last_mut() {
            let vertex = *vertex;
            let rest = &adjacency[vertex][*cursor..];
            let Some(offset) = rest.iter().position(|&item| member[item] == index) else {
                colors[vertex] = Color::Black;
                stack.pop();
                continue;
            };
            let next = rest[offset];
            *cursor += offset + 1;
            match colors[next] {
                Color::Gray => {
                    let begin = stack.iter().position(|&(item, _)| item == next)?;
                    let mut path: Vec<_> = stack[begin..]
                        .iter()
                        .filter_map(|&(item, _)| vertices[item].port_ref())
                        .collect();
                    // A back edge can target the internal hub. Close the projected
                    // cycle at its first real port, never expose a synthetic vertex.
                    path.push(path.first()?.clone());
                    return Some(path);
                }
                Color::Black => {}
                Color::White => {
                    colors[next] = Color::Gray;
                    stack.push((next, 0));
                }
            }
        }
    }
    None
}
