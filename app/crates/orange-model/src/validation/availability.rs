//! Data-value availability and activation ownership (11 §7.2).
//!
//! The pass is read-only. It consumes the region index and the resolved signatures, expands
//! every body's structured control flow into a graph of vertices, and checks each data value
//! that an execution node consumes, directly or through pure and volatile nodes (the lazy
//! cone inherits the consuming exec node's requirement), against its producer's `source`.
//! All traversals are iterative.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::RegistrySnapshot;
use crate::document::{
    BodyRef, Edge, GraphBody, GraphDocument, NodeKind, OutputSource, PortDescriptor, PortDirection,
    PortKind, SignatureSnapshot,
};
use crate::id::{EdgeId, FunctionId, NodeId, PortId, PortKey};

use super::connections::ConnectionValidation;
use super::diagnostic::{
    Diagnostic, DiagnosticCode, HostOrigin, Severity, Subject, sort_diagnostics,
};
use super::ports::lookup_port;
use super::regions::{RegionIndex, RegionKind, RegionRoot};

/// Result of the availability pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AvailabilityValidation {
    pub diagnostics: Vec<Diagnostic>,
}

impl AvailabilityValidation {
    /// True when no availability error was found.
    pub fn is_valid(&self) -> bool {
        self.diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != Severity::Error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Vertex {
    Node(NodeId),
    /// Start of the region rooted at this output port.
    Entry(NodeId, PortKey),
    /// Completion of that region.
    Exit(NodeId, PortKey),
    /// A fork's `completed` firing.
    Join(NodeId),
    /// A loop deciding between another iteration and `completed`.
    Next(NodeId),
    /// A loop's `completed` firing.
    Done(NodeId),
}

/// The outcome edge a control-flow edge belongs to, as (node, base port).
type Label = Option<(NodeId, PortId)>;

#[derive(Default)]
struct Fork {
    /// `all` waits for every branch exit; the others fire on the first.
    all: bool,
    exits: BTreeSet<Vertex>,
}

#[derive(Default)]
struct Cfg {
    succ: BTreeMap<Vertex, Vec<(Vertex, Label)>>,
    fork_exit: BTreeMap<Vertex, NodeId>,
    forks: BTreeMap<NodeId, Fork>,
}

impl Cfg {
    fn add(&mut self, from: Vertex, to: Vertex, label: Label) {
        self.succ.entry(from).or_default().push((to, label));
    }

    /// Edges from `from` to the targets wired to `(node, port)`, or to `fallback` when the
    /// output is unconnected (the strand ends there).
    fn goto(
        &mut self,
        from: &Vertex,
        targets: &BTreeMap<(NodeId, PortKey), Vec<NodeId>>,
        node: NodeId,
        port: &PortKey,
        fallback: Option<Vertex>,
    ) {
        let label = Some((node, port.port().clone()));
        match targets.get(&(node, port.clone())) {
            Some(list) if !list.is_empty() => {
                for target in list {
                    self.add(from.clone(), Vertex::Node(*target), label.clone());
                }
            }
            _ => {
                if let Some(fallback) = fallback {
                    self.add(from.clone(), fallback, label);
                }
            }
        }
    }

    /// Vertices reachable from `start` without crossing an outcome edge of `producer` named
    /// in `outcomes`. A vertex outside the set is dominated by those edges.
    fn reach_avoiding(
        &self,
        start: &Vertex,
        producer: NodeId,
        outcomes: &BTreeSet<PortId>,
    ) -> BTreeSet<Vertex> {
        let mut seen = BTreeSet::new();
        let mut hits: BTreeMap<NodeId, BTreeSet<Vertex>> = BTreeMap::new();
        let mut stack = vec![start.clone()];
        while let Some(vertex) = stack.pop() {
            if !seen.insert(vertex.clone()) {
                continue;
            }
            if let Some(fork) = self.fork_exit.get(&vertex) {
                let reached = hits.entry(*fork).or_default();
                reached.insert(vertex.clone());
                let info = &self.forks[fork];
                if !info.all || reached.len() == info.exits.len() {
                    stack.push(Vertex::Join(*fork));
                }
            }
            for (next, label) in self.succ.get(&vertex).into_iter().flatten() {
                let blocked = label
                    .as_ref()
                    .is_some_and(|(node, port)| *node == producer && outcomes.contains(port));
                if !blocked {
                    stack.push(next.clone());
                }
            }
        }
        seen
    }
}

/// Checks value availability and activation ownership for every body.
pub fn validate_availability(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    connections: &ConnectionValidation,
    regions: &RegionIndex,
    registry: &RegistrySnapshot,
) -> AvailabilityValidation {
    let error_edges: BTreeSet<_> = connections
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .filter_map(|diagnostic| diagnostic.subject.edge)
        .collect();
    let mut result = AvailabilityValidation::default();
    for (body_ref, body) in document.bodies_with_refs() {
        let function = match body_ref {
            BodyRef::Root => None,
            BodyRef::Function(id) => Some(id),
        };
        let context = Context {
            document,
            resolved,
            regions,
            error_edges: &error_edges,
            origin: registry.origin(),
            function,
        };
        check_body(&context, body, &mut result);
    }
    sort_diagnostics(&mut result.diagnostics);
    result.diagnostics.dedup();
    result
}

struct Context<'a> {
    document: &'a GraphDocument,
    resolved: &'a BTreeMap<NodeId, SignatureSnapshot>,
    regions: &'a RegionIndex,
    error_edges: &'a BTreeSet<EdgeId>,
    origin: &'a HostOrigin,
    function: Option<FunctionId>,
}

fn top_of(regions: &RegionIndex, root: &RegionRoot) -> RegionRoot {
    let mut current = root.clone();
    while let Some(parent) = regions
        .regions
        .get(&current)
        .and_then(|region| region.parent.clone())
    {
        current = parent;
    }
    current
}

/// Exec outputs of a node: declared ports, variadic instances and kind-implicit ports.
fn exec_outputs(
    document: &GraphDocument,
    node: NodeId,
    signature: &SignatureSnapshot,
) -> BTreeSet<PortKey> {
    let mut ports = BTreeSet::new();
    let Some(instance) = document.node(node) else {
        return ports;
    };
    for port in &signature.ports {
        if port.direction != PortDirection::Output || port.kind != PortKind::Exec {
            continue;
        }
        if port.variadic.is_some() {
            for k in instance
                .variadic
                .get(&port.port_id)
                .into_iter()
                .flat_map(|state| &state.order)
            {
                ports.insert(PortKey::new(port.port_id.clone(), Some(*k)));
            }
        } else {
            ports.insert(PortKey::new(port.port_id.clone(), None));
        }
    }
    for name in signature.implicit_ports() {
        if !matches!(name, "in" | "error") {
            ports.insert(PortKey::new(
                name.parse().expect("implicit port id is valid"),
                None,
            ));
        }
    }
    ports
}

struct DataEdge<'a> {
    edge: &'a Edge,
    from: PortDescriptor,
}

#[allow(clippy::too_many_lines)]
fn check_body(context: &Context, body: &GraphBody, result: &mut AvailabilityValidation) {
    let Context {
        document,
        resolved,
        regions,
        ..
    } = *context;
    let mut targets: BTreeMap<(NodeId, PortKey), Vec<NodeId>> = BTreeMap::new();
    let mut incoming: BTreeMap<NodeId, Vec<DataEdge>> = BTreeMap::new();
    let mut edges: Vec<_> = body.edges.values().collect();
    edges.sort_by_key(|edge| edge.id);
    for edge in edges {
        if context.error_edges.contains(&edge.id)
            || !body.nodes.contains_key(&edge.from.node)
            || !body.nodes.contains_key(&edge.to.node)
        {
            continue;
        }
        let (Ok(from), Ok(to)) = (
            lookup_port(document, resolved, &edge.from),
            lookup_port(document, resolved, &edge.to),
        ) else {
            continue;
        };
        if from.direction != PortDirection::Output || to.direction != PortDirection::Input {
            continue;
        }
        match (from.kind, to.kind) {
            (PortKind::Exec | PortKind::Event, PortKind::Exec) => targets
                .entry((edge.from.node, edge.from.port.clone()))
                .or_default()
                .push(edge.to.node),
            (PortKind::Data, PortKind::Data) => incoming
                .entry(edge.to.node)
                .or_default()
                .push(DataEdge { edge, from }),
            _ => {}
        }
    }

    // Top-level activation roots in this body.
    let roots: Vec<&RegionRoot> = regions
        .regions
        .iter()
        .filter(|(root, region)| {
            region.kind == RegionKind::Entry && body.nodes.contains_key(&root.node)
        })
        .map(|(root, _)| root)
        .collect();

    let mut cfg = Cfg::default();
    for root in &roots {
        let entry = Vertex::Entry(root.node, root.port.clone());
        let exit = Vertex::Exit(root.node, root.port.clone());
        cfg.goto(&entry, &targets, root.node, &root.port, Some(exit));
    }
    for node in body.nodes.values() {
        build_node(context, &mut cfg, &targets, node.id);
    }

    // Consumers: owned exec nodes, reading their data inputs and the cones behind them.
    let mut cache: BTreeMap<(RegionRoot, NodeId, BTreeSet<PortId>), BTreeSet<Vertex>> =
        BTreeMap::new();
    for consumer in body.nodes.keys() {
        let Some(region) = regions.owners.get(consumer) else {
            continue;
        };
        let top = top_of(regions, region);
        let mut stack: Vec<(&DataEdge, &Edge)> = Vec::new();
        for item in incoming.get(consumer).into_iter().flatten() {
            stack.push((item, item.edge));
        }
        let mut visited = BTreeSet::new();
        while let Some((item, direct)) = stack.pop() {
            if !visited.insert(item.edge.id) {
                continue;
            }
            let producer = item.edge.from.node;
            if matches!(
                resolved.get(&producer).map(|signature| signature.kind),
                Some(NodeKind::Pure | NodeKind::Volatile)
            ) && !regions.owners.contains_key(&producer)
            {
                for upstream in incoming.get(&producer).into_iter().flatten() {
                    stack.push((upstream, direct));
                }
            }
            let verdict = verdict(
                context, &cfg, &roots, &mut cache, item, *consumer, region, &top,
            );
            if let Some(code) = verdict {
                let subject = Subject::new(
                    Some(*consumer),
                    Some(direct.to.port.clone()),
                    Some(direct.id),
                );
                let subject = match context.function {
                    Some(function) => subject.with_function(function),
                    None => subject,
                };
                result
                    .diagnostics
                    .push(Diagnostic::new(code, subject, context.origin));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn verdict(
    context: &Context,
    cfg: &Cfg,
    roots: &[&RegionRoot],
    cache: &mut BTreeMap<(RegionRoot, NodeId, BTreeSet<PortId>), BTreeSet<Vertex>>,
    item: &DataEdge,
    consumer: NodeId,
    region: &RegionRoot,
    top: &RegionRoot,
) -> Option<DiagnosticCode> {
    let regions = context.regions;
    let producer = item.edge.from.node;
    match item.from.source? {
        OutputSource::Computed | OutputSource::State => None,
        OutputSource::Payload => {
            let same = match regions.owners.get(&producer) {
                Some(owner) => top_of(regions, owner) == *top,
                None => top.node == producer,
            };
            (!same).then_some(DiagnosticCode::CrossActivation)
        }
        OutputSource::Iteration => {
            let owner = regions.owners.get(&producer)?;
            if top_of(regions, owner) != *top {
                return Some(DiagnosticCode::CrossActivation);
            }
            let mut current = Some(region.clone());
            while let Some(root) = current {
                if root.node == producer && root.port.port().as_str() == "body" {
                    return None;
                }
                current = regions
                    .regions
                    .get(&root)
                    .and_then(|found| found.parent.clone());
            }
            Some(DiagnosticCode::UnavailableValue)
        }
        OutputSource::Execution => {
            let owner = regions.owners.get(&producer)?;
            if top_of(regions, owner) != *top {
                return Some(DiagnosticCode::CrossActivation);
            }
            if !roots.contains(&top) {
                return None;
            }
            let outcomes: BTreeSet<PortId> = item.from.available_on.as_ref().map_or_else(
                || BTreeSet::from(["then".parse().expect("port id is valid")]),
                |list| list.iter().cloned().collect(),
            );
            let reached = cache
                .entry((top.clone(), producer, outcomes.clone()))
                .or_insert_with(|| {
                    cfg.reach_avoiding(
                        &Vertex::Entry(top.node, top.port.clone()),
                        producer,
                        &outcomes,
                    )
                });
            reached
                .contains(&Vertex::Node(consumer))
                .then_some(DiagnosticCode::UnavailableValue)
        }
    }
}

/// Adds the control-flow edges leaving one node, expanding structured constructs.
fn build_node(
    context: &Context,
    cfg: &mut Cfg,
    targets: &BTreeMap<(NodeId, PortKey), Vec<NodeId>>,
    id: NodeId,
) {
    let (Some(region), Some(signature), Some(node)) = (
        context.regions.owners.get(&id),
        context.resolved.get(&id),
        context.document.node(id),
    ) else {
        return;
    };
    let definition = node.definition.id.as_str();
    let here = Vertex::Node(id);
    let region_exit = Vertex::Exit(region.node, region.port.clone());
    let ports = exec_outputs(context.document, id, signature);
    let is_loop = signature.kind == NodeKind::Iterator;

    match definition {
        "core/break" | "core/continue" => {
            if let Some(enclosing) = context
                .regions
                .regions
                .get(region)
                .and_then(|found| found.enclosing_loop.as_ref())
            {
                let loop_node = enclosing.node;
                let to = if definition == "core/break" {
                    Vertex::Done(loop_node)
                } else {
                    Vertex::Next(loop_node)
                };
                cfg.add(here, to, None);
            }
            return;
        }
        "core/sequence" => {
            let steps: Vec<_> = ports
                .iter()
                .filter(|port| port.port().as_str() == "then")
                .cloned()
                .collect();
            for (position, port) in steps.iter().enumerate() {
                let entry = Vertex::Entry(id, port.clone());
                let exit = Vertex::Exit(id, port.clone());
                cfg.goto(&entry, targets, id, port, Some(exit.clone()));
                let after = steps.get(position + 1).map_or_else(
                    || region_exit.clone(),
                    |next| Vertex::Entry(id, next.clone()),
                );
                cfg.add(exit, after, None);
            }
            match steps.first() {
                Some(first) => cfg.add(here, Vertex::Entry(id, first.clone()), None),
                None => cfg.add(here, region_exit, None),
            }
            return;
        }
        "core/fork" => {
            let join = node.config.get("join").and_then(serde_json::Value::as_str);
            let mut fork = Fork {
                // Only an explicit `all` makes branch values available at `completed`.
                all: join == Some("all"),
                exits: BTreeSet::new(),
            };
            for port in ports.iter().filter(|port| port.port().as_str() == "branch") {
                let entry = Vertex::Entry(id, port.clone());
                let exit = Vertex::Exit(id, port.clone());
                cfg.goto(&entry, targets, id, port, Some(exit.clone()));
                cfg.add(here.clone(), entry, None);
                cfg.fork_exit.insert(exit.clone(), id);
                fork.exits.insert(exit);
            }
            if join == Some("detach") || fork.exits.is_empty() {
                cfg.add(here.clone(), Vertex::Join(id), None);
            }
            cfg.forks.insert(id, fork);
            let completed = PortKey::new("completed".parse().expect("port id is valid"), None);
            cfg.goto(
                &Vertex::Join(id),
                targets,
                id,
                &completed,
                Some(region_exit),
            );
            return;
        }
        _ => {}
    }

    for port in &ports {
        let name = port.port().as_str();
        if is_loop && name == "body" {
            let entry = Vertex::Entry(id, port.clone());
            let exit = Vertex::Exit(id, port.clone());
            cfg.goto(&entry, targets, id, port, Some(exit.clone()));
            cfg.add(here.clone(), entry.clone(), None);
            cfg.add(exit, Vertex::Next(id), None);
            cfg.add(Vertex::Next(id), entry, None);
            cfg.add(Vertex::Next(id), Vertex::Done(id), None);
            // Zero iterations reach `completed` directly.
            cfg.add(here.clone(), Vertex::Done(id), None);
        } else if is_loop && name == "completed" {
            cfg.goto(
                &Vertex::Done(id),
                targets,
                id,
                port,
                Some(region_exit.clone()),
            );
        } else {
            // An unhandled `failed` or `cancelled` faults or stops the strand: no continuation.
            let fallback = (!matches!(name, "failed" | "cancelled")).then(|| region_exit.clone());
            cfg.goto(&here, targets, id, port, fallback);
        }
    }
}
