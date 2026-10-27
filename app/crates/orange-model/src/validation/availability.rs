//! Data-value availability and activation ownership (11 §7.2).
//!
//! The pass is read-only. It consumes the region index and the resolved signatures, expands
//! every body's structured control flow into a graph of vertices, and checks each data value
//! that an execution node consumes, directly or through pure and volatile nodes (the lazy
//! cone inherits the consuming exec node's requirement), against its producer's `source`.
//! All traversals are iterative.

use crate::catalog::intrinsic_id as ids;
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
use super::dominators::DomTree;
use super::ports::PortResolver;
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

type Outcomes = BTreeSet<PortId>;

/// Answers availability queries for one body without one whole-graph walk per query.
///
/// A queried producer is fast when its queried outcome sets are disjoint and all of its
/// outcome edges leave its own node. Each such set's edges then pass through one group
/// vertex, so "every path crosses the set" becomes "the group vertex dominates the
/// consumer", read from one dominator tree per activation root.
///
/// Dominators model every join as reachable from any branch. That over-approximates
/// reachability past a Fork `join: all`, so "dominated" stays exact and a "not dominated"
/// answer is rechecked with [`Cfg::reach_avoiding`] only when such a join exists. Slow
/// producers always use that walk, and each walk is cached per query key.
struct Answers<'a> {
    cfg: &'a Cfg,
    ids: BTreeMap<Vertex, u32>,
    succ: Vec<Vec<u32>>,
    groups: BTreeMap<NodeId, Vec<(&'a Outcomes, u32)>>,
    and_joins: bool,
    trees: BTreeMap<Vertex, DomTree>,
    walks: BTreeMap<(Vertex, NodeId, Outcomes), BTreeSet<Vertex>>,
}

impl<'a> Answers<'a> {
    fn new(cfg: &'a Cfg, queries: impl IntoIterator<Item = (NodeId, &'a Outcomes)>) -> Self {
        let mut sets: BTreeMap<NodeId, Vec<&'a Outcomes>> = BTreeMap::new();
        for (producer, outcomes) in queries {
            let entry = sets.entry(producer).or_default();
            if !entry.contains(&outcomes) {
                entry.push(outcomes);
            }
        }
        // Producers with an outcome edge that starts elsewhere (a loop's `completed`, a
        // sequence step) cannot route all of it through one vertex.
        let mut foreign = BTreeSet::new();
        for (from, edges) in &cfg.succ {
            for (node, _) in edges.iter().filter_map(|(_, label)| label.as_ref()) {
                if *from != Vertex::Node(*node) {
                    foreign.insert(*node);
                }
            }
        }
        let mut answers = Self {
            cfg,
            ids: BTreeMap::new(),
            succ: Vec::new(),
            groups: BTreeMap::new(),
            and_joins: cfg
                .forks
                .values()
                .any(|fork| fork.all && !fork.exits.is_empty()),
            trees: BTreeMap::new(),
            walks: BTreeMap::new(),
        };
        let mut port_group: BTreeMap<(NodeId, &PortId), u32> = BTreeMap::new();
        for (producer, sets) in sets {
            let disjoint = sets
                .iter()
                .enumerate()
                .all(|(index, set)| sets[..index].iter().all(|earlier| earlier.is_disjoint(set)));
            if foreign.contains(&producer) || !disjoint {
                continue;
            }
            for set in sets {
                let group = answers.fresh();
                for port in set {
                    port_group.insert((producer, port), group);
                }
                answers
                    .groups
                    .entry(producer)
                    .or_default()
                    .push((set, group));
            }
        }
        for (from, edges) in &cfg.succ {
            let from = answers.intern(from);
            let mut linked = BTreeSet::new();
            for (to, label) in edges {
                let to = answers.intern(to);
                match label
                    .as_ref()
                    .and_then(|(node, port)| port_group.get(&(*node, port)))
                {
                    Some(group) => {
                        if linked.insert(*group) {
                            answers.succ[from as usize].push(*group);
                        }
                        answers.succ[*group as usize].push(to);
                    }
                    None => answers.succ[from as usize].push(to),
                }
            }
        }
        for (fork, info) in &cfg.forks {
            let join = answers.intern(&Vertex::Join(*fork));
            for exit in &info.exits {
                let exit = answers.intern(exit);
                answers.succ[exit as usize].push(join);
            }
        }
        answers
    }

    fn fresh(&mut self) -> u32 {
        self.succ.push(Vec::new());
        (self.succ.len() - 1) as u32
    }

    fn intern(&mut self, vertex: &Vertex) -> u32 {
        if let Some(id) = self.ids.get(vertex) {
            return *id;
        }
        let id = self.fresh();
        self.ids.insert(vertex.clone(), id);
        id
    }

    /// True if `consumer` is reachable from `start` without crossing an outcome edge of
    /// `producer` named in `outcomes`: the value may be read before it exists.
    fn unavailable(
        &mut self,
        start: &Vertex,
        producer: NodeId,
        outcomes: &Outcomes,
        consumer: NodeId,
    ) -> bool {
        let group = self
            .groups
            .get(&producer)
            .and_then(|sets| sets.iter().find(|(set, _)| *set == outcomes))
            .map(|(_, group)| *group);
        if let (Some(group), Some(begin)) = (group, self.ids.get(start).copied()) {
            let Self { succ, trees, .. } = self;
            let tree = trees
                .entry(start.clone())
                .or_insert_with(|| DomTree::new(succ, begin));
            let Some(target) = self.ids.get(&Vertex::Node(consumer)).copied() else {
                return false;
            };
            if !tree.reachable(target) || tree.dominates(group, target) {
                return false;
            }
            if !self.and_joins {
                return true;
            }
        }
        self.walks
            .entry((start.clone(), producer, outcomes.clone()))
            .or_insert_with(|| self.cfg.reach_avoiding(start, producer, outcomes))
            .contains(&Vertex::Node(consumer))
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
    let ports = PortResolver::new(document, resolved);
    validate_availability_with_ports(document, &ports, connections, regions, registry)
}

pub(crate) fn validate_availability_with_ports(
    document: &GraphDocument,
    resolved: &PortResolver<'_>,
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
    resolved: &'a PortResolver<'a>,
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
    from: std::rc::Rc<PortDescriptor>,
}

enum Requirement {
    Diagnose(DiagnosticCode),
    Execution(BTreeSet<PortId>),
}

type AvailabilityKey = (RegionRoot, NodeId, BTreeSet<PortId>);
type AvailabilityUses = BTreeMap<AvailabilityKey, BTreeSet<(NodeId, Subject)>>;

#[allow(clippy::too_many_lines)]
fn check_body(context: &Context, body: &GraphBody, result: &mut AvailabilityValidation) {
    let Context {
        resolved, regions, ..
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
        let (Ok(from), Ok(to)) = (resolved.port(&edge.from), resolved.port(&edge.to)) else {
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
    let mut uses = AvailabilityUses::new();
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
            if let Some(requirement) = requirement(context, item, region, &top) {
                let subject = Subject::new(
                    Some(*consumer),
                    Some(direct.to.port.clone()),
                    Some(direct.id),
                );
                let subject = match context.function {
                    Some(function) => subject.with_function(function),
                    None => subject,
                };
                match requirement {
                    Requirement::Diagnose(code) => {
                        result
                            .diagnostics
                            .push(Diagnostic::new(code, subject, context.origin))
                    }
                    Requirement::Execution(outcomes) if roots.contains(&&top) => {
                        uses.entry((top.clone(), producer, outcomes))
                            .or_default()
                            .insert((*consumer, subject));
                    }
                    Requirement::Execution(_) => {}
                }
            }
        }
    }

    let mut answers = Answers::new(
        &cfg,
        uses.keys()
            .map(|(_, producer, outcomes)| (*producer, outcomes)),
    );
    for ((top, producer, outcomes), consumers) in &uses {
        let start = Vertex::Entry(top.node, top.port.clone());
        for (consumer, subject) in consumers {
            if answers.unavailable(&start, *producer, outcomes, *consumer) {
                result.diagnostics.push(Diagnostic::new(
                    DiagnosticCode::UnavailableValue,
                    subject.clone(),
                    context.origin,
                ));
            }
        }
    }
}

fn requirement(
    context: &Context,
    item: &DataEdge,
    region: &RegionRoot,
    top: &RegionRoot,
) -> Option<Requirement> {
    let regions = context.regions;
    let producer = item.edge.from.node;
    match item.from.source? {
        OutputSource::Computed | OutputSource::State => None,
        OutputSource::Payload => {
            let same = match regions.owners.get(&producer) {
                Some(owner) => top_of(regions, owner) == *top,
                None => top.node == producer,
            };
            (!same).then_some(Requirement::Diagnose(DiagnosticCode::CrossActivation))
        }
        OutputSource::Iteration => {
            let owner = regions.owners.get(&producer)?;
            if top_of(regions, owner) != *top {
                return Some(Requirement::Diagnose(DiagnosticCode::CrossActivation));
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
            Some(Requirement::Diagnose(DiagnosticCode::UnavailableValue))
        }
        OutputSource::Execution => {
            let owner = regions.owners.get(&producer)?;
            if top_of(regions, owner) != *top {
                return Some(Requirement::Diagnose(DiagnosticCode::CrossActivation));
            }
            let outcomes: BTreeSet<PortId> = item.from.available_on.as_ref().map_or_else(
                || BTreeSet::from(["then".parse().expect("port id is valid")]),
                |list| list.iter().cloned().collect(),
            );
            Some(Requirement::Execution(outcomes))
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
        ids::BREAK | ids::CONTINUE => {
            if let Some(enclosing) = context
                .regions
                .regions
                .get(region)
                .and_then(|found| found.enclosing_loop.as_ref())
            {
                let loop_node = enclosing.node;
                let to = if definition == ids::BREAK {
                    Vertex::Done(loop_node)
                } else {
                    Vertex::Next(loop_node)
                };
                cfg.add(here, to, None);
            }
            return;
        }
        ids::SEQUENCE => {
            let then = "then".parse().expect("port id is valid");
            let steps: Vec<_> = node
                .variadic
                .get(&then)
                .into_iter()
                .flat_map(|state| &state.order)
                .map(|key| PortKey::new(then.clone(), Some(*key)))
                .filter(|port| ports.contains(port))
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
        ids::FORK => {
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn node(n: u32) -> NodeId {
        format!("10000000-0000-4000-8000-{n:012x}").parse().unwrap()
    }
    fn port(name: &str) -> PortId {
        name.parse().unwrap()
    }

    /// Outcome-set families a producer may be queried with; the last overlaps.
    fn families() -> [Vec<Outcomes>; 5] {
        let set = |names: &[&str]| names.iter().map(|name| port(name)).collect::<Outcomes>();
        [
            vec![set(&["then"])],
            vec![set(&["failed"])],
            vec![set(&["then"]), set(&["failed"])],
            vec![set(&["then", "failed"])],
            vec![set(&["then"]), set(&["then", "failed"])],
        ]
    }

    /// Random control-flow graph: `nodes` plain vertices, two forks with two exits each,
    /// and labeled edges that mostly leave their producer's own node.
    fn cfg(nodes: u32, all: [bool; 2], edges: &[(u32, u32, u8, u32)]) -> (Cfg, Vertex) {
        let start = Vertex::Entry(node(999), PortKey::new(port("then"), None));
        let mut pool = vec![start.clone()];
        pool.extend((0..nodes).map(|n| Vertex::Node(node(n))));
        let mut cfg = Cfg::default();
        for (index, all) in all.into_iter().enumerate() {
            let fork = node(500 + index as u32);
            let exits: BTreeSet<_> = (0..2)
                .map(|k| Vertex::Exit(fork, PortKey::new(port("branch"), Some(k))))
                .collect();
            for exit in &exits {
                cfg.fork_exit.insert(exit.clone(), fork);
                pool.push(exit.clone());
            }
            pool.push(Vertex::Join(fork));
            cfg.forks.insert(fork, Fork { all, exits });
        }
        for (from, to, kind, other) in edges {
            let from = &pool[*from as usize % pool.len()];
            let to = pool[*to as usize % pool.len()].clone();
            let own = match from {
                Vertex::Node(id) => *id,
                _ => node(other % nodes),
            };
            let label = match kind % 4 {
                0 => None,
                1 => Some((own, port("then"))),
                2 => Some((own, port("failed"))),
                _ => Some((node(other % nodes), port("then"))),
            };
            cfg.add(from.clone(), to, label);
        }
        (cfg, start)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]
        #[test]
        fn dominator_answers_match_the_reference_walk(
            nodes in 1u32..8,
            all in any::<[bool; 2]>(),
            edges in prop::collection::vec((0u32..20, 0u32..20, 0u8..4, 0u32..8), 0..32),
            choice in prop::collection::vec(0usize..5, 8),
        ) {
            let (cfg, start) = cfg(nodes, all, &edges);
            let families = families();
            let queries: Vec<(NodeId, &Outcomes)> = (0..nodes)
                .flat_map(|n| families[choice[n as usize]].iter().map(move |set| (node(n), set)))
                .collect();
            let mut answers = Answers::new(&cfg, queries.iter().map(|(p, set)| (*p, *set)));
            for (producer, outcomes) in &queries {
                let reference = cfg.reach_avoiding(&start, *producer, outcomes);
                for consumer in (0..nodes).map(node) {
                    prop_assert_eq!(
                        answers.unavailable(&start, *producer, outcomes, consumer),
                        reference.contains(&Vertex::Node(consumer)),
                        "producer {} outcomes {:?} consumer {}", producer, outcomes, consumer
                    );
                }
            }
        }
    }

    #[test]
    fn fast_path_covers_disjoint_own_outcomes_and_skips_the_rest() {
        let (cfg, _) = cfg(
            3,
            [false, false],
            &[(1, 2, 1, 0), (1, 3, 2, 0), (3, 1, 3, 1)],
        );
        let then = families()[0][0].clone();
        let both = families()[3][0].clone();
        let answers = Answers::new(&cfg, [(node(0), &then), (node(1), &then)]);
        // Node 0 labels only its own edges; node 2 carries an edge labeled for node 1.
        assert!(answers.groups.contains_key(&node(0)));
        assert!(!answers.groups.contains_key(&node(1)));
        let overlapping = Answers::new(&cfg, [(node(0), &then), (node(0), &both)]);
        assert!(overlapping.groups.is_empty());
    }
}
