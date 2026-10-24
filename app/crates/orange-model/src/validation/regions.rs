//! Control-region ownership, loop nesting and structured-boundary validation (11 §7.1, §8.3).

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::RegistrySnapshot;
use crate::document::{
    BodyRef, GraphBody, GraphDocument, NodeKind, PortDirection, PortKind, SignatureSnapshot,
};
use crate::id::{EdgeId, FunctionId, NodeId, PortKey};

use super::connections::ConnectionValidation;
use super::diagnostic::{
    Diagnostic, DiagnosticCode, HostOrigin, Severity, Subject, sort_diagnostics,
};
use super::ports::lookup_port;
use super::scope::Scope;

/// The exec output port whose targets start a region.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RegionRoot {
    pub node: NodeId,
    pub port: PortKey,
}

/// The construct that owns a region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionKind {
    /// An entry's exec output or an event output.
    Entry,
    /// A `core/sequence` `then#k`.
    Sequence,
    /// A `core/fork` `branch#k`.
    Fork,
    /// An iterator `body`.
    LoopBody,
}

/// One region of a body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub kind: RegionKind,
    /// Region of the node that owns this region's root, when that region is known.
    pub parent: Option<RegionRoot>,
    /// Innermost loop-body region containing this region, itself included.
    pub enclosing_loop: Option<RegionRoot>,
    /// Number of loop bodies containing this region, itself included.
    pub loop_depth: usize,
    /// Nodes owned by exactly this region.
    pub nodes: BTreeSet<NodeId>,
}

/// Region ownership across all bodies. Nodes reached by several regions are omitted from
/// `owners` and diagnosed; they never get a guessed owner.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegionIndex {
    pub regions: BTreeMap<RegionRoot, Region>,
    pub owners: BTreeMap<NodeId, RegionRoot>,
    /// Exec nodes that no region reaches (`W-UNREACHABLE`).
    pub unreachable: BTreeSet<NodeId>,
}

impl RegionIndex {
    /// Innermost loop-body region of an owned node.
    pub fn loop_of(&self, node: NodeId) -> Option<&RegionRoot> {
        self.regions
            .get(self.owners.get(&node)?)?
            .enclosing_loop
            .as_ref()
    }
}

/// Result of the region pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegionValidation {
    pub diagnostics: Vec<Diagnostic>,
    pub index: RegionIndex,
    /// Nodes whose metadata was needed to build exec edges or regions but is unavailable.
    pub incomplete_nodes: BTreeSet<NodeId>,
}

impl RegionValidation {
    /// True only when region analysis completed and found no errors.
    pub fn is_valid(&self) -> bool {
        self.incomplete_nodes.is_empty()
            && self
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity != Severity::Error)
    }
}

struct ExecEdge {
    id: EdgeId,
    from_port: PortKey,
    from_event: bool,
    to: NodeId,
}

/// Source node, source port and, for a region's root edge, that region.
type Incoming = (NodeId, PortKey, Option<usize>);

struct Walk {
    kind: RegionKind,
    root: RegionRoot,
    parent: Option<usize>,
    enclosing_loop: Option<usize>,
    loop_depth: usize,
}

/// Builds the region index for every body from exec edges and fresh resolved signatures.
/// Traversal is iterative; each region visits each node at most once.
pub fn validate_regions(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    connections: &ConnectionValidation,
    registry: &RegistrySnapshot,
) -> RegionValidation {
    validate_regions_in(document, resolved, connections, registry, Scope::All)
}

/// [`validate_regions`] over the bodies in `scope`.
pub(crate) fn validate_regions_in(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    connections: &ConnectionValidation,
    registry: &RegistrySnapshot,
    scope: Scope,
) -> RegionValidation {
    let origin = registry.origin();
    let mut result = RegionValidation {
        incomplete_nodes: connections.incomplete_nodes.clone(),
        ..RegionValidation::default()
    };
    let error_edges: BTreeSet<_> = connections
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .filter_map(|diagnostic| diagnostic.subject.edge)
        .collect();
    for (body_ref, body) in scope.bodies(document) {
        let function = match body_ref {
            BodyRef::Root => None,
            BodyRef::Function(id) => Some(id),
        };
        check_body(
            document,
            resolved,
            body,
            function,
            &error_edges,
            origin,
            &mut result,
        );
    }
    sort_diagnostics(&mut result.diagnostics);
    result
}

fn subject(
    function: Option<FunctionId>,
    node: NodeId,
    port: Option<PortKey>,
    edge: Option<EdgeId>,
) -> Subject {
    let subject = Subject::new(Some(node), port, edge);
    match function {
        Some(function) => subject.with_function(function),
        None => subject,
    }
}

fn child_kind(definition: &str, kind: NodeKind, port: &PortKey) -> Option<RegionKind> {
    match (definition, port.port().as_str()) {
        ("core/sequence", "then") => Some(RegionKind::Sequence),
        ("core/fork", "branch") => Some(RegionKind::Fork),
        (_, "body") if kind == NodeKind::Iterator => Some(RegionKind::LoopBody),
        _ => None,
    }
}

fn is_exec_node(signature: &SignatureSnapshot) -> bool {
    signature.kind != NodeKind::Entry
        && (signature.implicit_ports().contains(&"in")
            || signature
                .ports
                .iter()
                .any(|port| port.kind == PortKind::Exec && port.direction == PortDirection::Input))
}

#[allow(clippy::too_many_lines)]
fn check_body(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    body: &GraphBody,
    function: Option<FunctionId>,
    error_edges: &BTreeSet<EdgeId>,
    origin: &HostOrigin,
    result: &mut RegionValidation,
) {
    let mut local_incomplete = BTreeSet::new();
    for node in body.nodes.values() {
        if !resolved.contains_key(&node.id) {
            local_incomplete.insert(node.id);
        }
    }

    // Exec adjacency from valid same-body edges.
    let mut out: BTreeMap<NodeId, Vec<ExecEdge>> = BTreeMap::new();
    let mut raw: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    let mut edges: Vec<_> = body.edges.values().collect();
    edges.sort_by_key(|edge| edge.id);
    for edge in edges {
        if error_edges.contains(&edge.id)
            || !body.nodes.contains_key(&edge.from.node)
            || !body.nodes.contains_key(&edge.to.node)
        {
            continue;
        }
        raw.entry(edge.from.node).or_default().push(edge.to.node);
        let (from, to) = (
            lookup_port(document, resolved, &edge.from),
            lookup_port(document, resolved, &edge.to),
        );
        match (from, to) {
            (Ok(from), Ok(to))
                if from.direction == PortDirection::Output
                    && to.direction == PortDirection::Input
                    && to.kind == PortKind::Exec
                    && matches!(from.kind, PortKind::Exec | PortKind::Event) =>
            {
                out.entry(edge.from.node).or_default().push(ExecEdge {
                    id: edge.id,
                    from_port: edge.from.port.clone(),
                    from_event: from.kind == PortKind::Event,
                    to: edge.to.node,
                });
            }
            (Err(super::ports::PortLookupError::MissingMetadata), _)
            | (_, Err(super::ports::PortLookupError::MissingMetadata)) => {
                local_incomplete.insert(edge.from.node);
                local_incomplete.insert(edge.to.node);
            }
            _ => {}
        }
    }

    let kind_of = |node: NodeId| resolved.get(&node).map(|signature| signature.kind);
    let definition_of = |node: NodeId| {
        document
            .node(node)
            .map_or("", |instance| instance.definition.id.as_str())
    };

    // Top-level roots: entry exec outputs and event outputs.
    let mut walks: Vec<Walk> = Vec::new();
    let mut by_root: BTreeMap<RegionRoot, usize> = BTreeMap::new();
    for (node, edges) in &out {
        for edge in edges {
            if kind_of(*node) == Some(NodeKind::Entry) || edge.from_event {
                let root = RegionRoot {
                    node: *node,
                    port: edge.from_port.clone(),
                };
                by_root.entry(root.clone()).or_insert_with(|| {
                    walks.push(Walk {
                        kind: RegionKind::Entry,
                        root,
                        parent: None,
                        enclosing_loop: None,
                        loop_depth: 0,
                    });
                    walks.len() - 1
                });
            }
        }
    }

    // membership[node] = regions that reach it; incoming[node][edge] = (source node,
    // source port, source region when it is the root).
    let mut membership: BTreeMap<NodeId, BTreeSet<usize>> = BTreeMap::new();
    let mut incoming: BTreeMap<NodeId, BTreeMap<EdgeId, Incoming>> = BTreeMap::new();
    let mut index = 0;
    while index < walks.len() {
        let root = walks[index].root.clone();
        let mut visited = BTreeSet::new();
        let mut stack = Vec::new();
        for edge in out.get(&root.node).into_iter().flatten() {
            if edge.from_port == root.port {
                incoming
                    .entry(edge.to)
                    .or_default()
                    .insert(edge.id, (root.node, root.port.clone(), Some(index)));
                stack.push(edge.to);
            }
        }
        while let Some(node) = stack.pop() {
            if !visited.insert(node) {
                continue;
            }
            membership.entry(node).or_default().insert(index);
            let kind = kind_of(node).unwrap_or(NodeKind::Action);
            for edge in out.get(&node).into_iter().flatten() {
                if let Some(child) = child_kind(definition_of(node), kind, &edge.from_port) {
                    let child_root = RegionRoot {
                        node,
                        port: edge.from_port.clone(),
                    };
                    if let std::collections::btree_map::Entry::Vacant(slot) =
                        by_root.entry(child_root.clone())
                    {
                        let in_loop = child == RegionKind::LoopBody;
                        let parent = &walks[index];
                        let next = Walk {
                            kind: child,
                            root: child_root.clone(),
                            parent: Some(index),
                            enclosing_loop: if in_loop {
                                Some(walks.len())
                            } else {
                                parent.enclosing_loop
                            },
                            loop_depth: parent.loop_depth + usize::from(in_loop),
                        };
                        slot.insert(walks.len());
                        walks.push(next);
                    }
                    continue;
                }
                incoming
                    .entry(edge.to)
                    .or_default()
                    .insert(edge.id, (node, edge.from_port.clone(), None));
                stack.push(edge.to);
            }
        }
        index += 1;
    }

    let is_ancestor = |ancestor: usize, mut region: usize| {
        loop {
            if region == ancestor {
                return true;
            }
            match walks[region].parent {
                Some(parent) => region = parent,
                None => return false,
            }
        }
    };
    let regions_of = |node: NodeId| membership.get(&node).cloned().unwrap_or_default();

    // Conflicts: report only where an edge first brings a node into more regions than its
    // source belongs to, so downstream nodes do not repeat the same fault.
    for (node, regions) in &membership {
        if regions.len() < 2 {
            continue;
        }
        let edges = incoming.get(node).cloned().unwrap_or_default();
        let frontier = edges.values().any(|(source, _, root)| {
            let set = root.map_or_else(BTreeSet::new, |region| BTreeSet::from([region]));
            let set = if root.is_some() {
                set
            } else {
                regions_of(*source)
            };
            !regions.is_subset(&set)
        });
        if !frontier {
            continue;
        }
        let nested = regions.iter().all(|a| {
            regions
                .iter()
                .all(|b| is_ancestor(*a, *b) || is_ancestor(*b, *a))
        });
        if !nested {
            result.diagnostics.push(Diagnostic::new(
                DiagnosticCode::RegionAmbiguous,
                subject(function, *node, None, None),
                origin,
            ));
            continue;
        }
        let home = edges
            .values()
            .find_map(|(_, _, root)| *root)
            .filter(|region| regions.contains(region))
            .or_else(|| regions.iter().copied().min_by_key(|r| depth(&walks, *r)))
            .expect("conflict has regions");
        let mut flagged = false;
        for (edge, (source, port, root)) in &edges {
            let set = root.map_or_else(|| regions_of(*source), |region| BTreeSet::from([region]));
            if !set.contains(&home) {
                flagged = true;
                result.diagnostics.push(Diagnostic::new(
                    DiagnosticCode::RegionEscape,
                    subject(function, *source, Some(port.clone()), Some(*edge)),
                    origin,
                ));
            }
        }
        if !flagged {
            result.diagnostics.push(Diagnostic::new(
                DiagnosticCode::RegionEscape,
                subject(function, *node, None, None),
                origin,
            ));
        }
    }

    // Structured boundaries and loop bounds.
    for node in body.nodes.values() {
        let definition = node.definition.id.as_str();
        if definition == "core/while"
            && !node
                .config
                .get("max_iterations")
                .and_then(serde_json::Value::as_u64)
                .is_some_and(|limit| limit > 0)
        {
            result.diagnostics.push(Diagnostic::new(
                DiagnosticCode::LoopUnbounded,
                subject(function, node.id, None, None),
                origin,
            ));
        }
        if definition == "core/switch"
            && resolved
                .get(&node.id)
                .is_some_and(|signature| !switch_cases_valid(node, signature))
        {
            result.diagnostics.push(Diagnostic::new(
                DiagnosticCode::SwitchCase,
                subject(function, node.id, None, None),
                origin,
            ));
        }
        let regions = regions_of(node.id);
        match definition {
            "core/break" | "core/continue"
                if regions.iter().any(|r| walks[*r].enclosing_loop.is_none()) =>
            {
                result.diagnostics.push(Diagnostic::new(
                    DiagnosticCode::BreakOutsideLoop,
                    subject(function, node.id, None, None),
                    origin,
                ));
            }
            "core/return"
                if regions.iter().any(|r| {
                    let mut cursor = Some(*r);
                    while let Some(region) = cursor {
                        if walks[region].kind == RegionKind::Fork {
                            return true;
                        }
                        cursor = walks[region].parent;
                    }
                    false
                }) =>
            {
                result.diagnostics.push(Diagnostic::new(
                    DiagnosticCode::RegionEscape,
                    subject(function, node.id, None, None),
                    origin,
                ));
            }
            _ => {}
        }
    }

    check_zones(
        document,
        resolved,
        body,
        function,
        error_edges,
        origin,
        result,
    );

    // Unreachable exec nodes, except those that unresolved nodes may feed.
    let mut maybe = BTreeSet::new();
    let mut stack: Vec<_> = local_incomplete.iter().copied().collect();
    while let Some(node) = stack.pop() {
        if maybe.insert(node) {
            stack.extend(raw.get(&node).into_iter().flatten().copied());
        }
    }
    for node in body.nodes.values() {
        if let Some(signature) = resolved.get(&node.id)
            && is_exec_node(signature)
            && !membership.contains_key(&node.id)
            && !maybe.contains(&node.id)
        {
            result.index.unreachable.insert(node.id);
            result.diagnostics.push(Diagnostic::new(
                DiagnosticCode::Unreachable,
                subject(function, node.id, None, None),
                origin,
            ));
        }
    }

    for node in &local_incomplete {
        result.incomplete_nodes.insert(*node);
        result.diagnostics.push(Diagnostic::new(
            DiagnosticCode::DependencyUnavailable,
            subject(None, *node, None, None),
            origin,
        ));
    }

    // Publish the index; nodes reached by several regions have no owner.
    let roots: Vec<_> = walks.iter().map(|walk| walk.root.clone()).collect();
    for (position, walk) in walks.iter().enumerate() {
        result.index.regions.insert(
            walk.root.clone(),
            Region {
                kind: walk.kind,
                parent: walk.parent.map(|parent| roots[parent].clone()),
                enclosing_loop: walk.enclosing_loop.map(|region| roots[region].clone()),
                loop_depth: walk.loop_depth,
                nodes: membership
                    .iter()
                    .filter(|(_, set)| set.len() == 1 && set.contains(&position))
                    .map(|(node, _)| *node)
                    .collect(),
            },
        );
    }
    for (node, set) in &membership {
        if let [only] = set.iter().copied().collect::<Vec<_>>()[..] {
            result.index.owners.insert(*node, roots[only].clone());
        }
    }
}

/// `config.cases` of a `core/switch` (11 §8.4): one value for every `case` instance and no
/// other entries, all distinct, of the JSON kind of the key type (integers, `bool` and
/// `string`; other key types are only checked for presence and uniqueness).
fn switch_cases_valid(node: &crate::document::NodeInstance, signature: &SignatureSnapshot) -> bool {
    let case = "case".parse().expect("port id is valid");
    let order: &[u16] = node
        .variadic
        .get(&case)
        .map_or(&[], |state| state.order.as_slice());
    let empty = serde_json::Map::new();
    let cases = match node.config.get("cases") {
        None => &empty,
        Some(value) => match value.as_object() {
            Some(cases) => cases,
            None => return false,
        },
    };
    if cases.len() != order.len() {
        return false;
    }
    let key = signature
        .ports
        .iter()
        .find(|port| port.port_id.as_str() == "key")
        .and_then(|port| port.ty.as_ref())
        .map(|ty| ty.as_str());
    let mut seen: Vec<&serde_json::Value> = Vec::new();
    for instance in order {
        let Some(value) = cases.get(&instance.to_string()) else {
            return false;
        };
        let kind_ok = match key {
            Some("core/i32@1" | "core/i64@1" | "core/u32@1" | "core/u64@1") => {
                value.is_i64() || value.is_u64()
            }
            Some("core/bool@1") => value.is_boolean(),
            Some("core/string@1") => value.is_string(),
            _ => true,
        };
        if !kind_ok || seen.contains(&value) {
            return false;
        }
        seen.push(value);
    }
    true
}

/// The zone family and side of a zone intrinsic (`core/map-begin` is `("map", false)`).
fn zone_of(definition: &str) -> Option<(&str, bool)> {
    let name = definition.strip_prefix("core/")?;
    let (family, side) = name.split_once('-')?;
    if !matches!(family, "repeat" | "map" | "fold" | "filter") {
        return None;
    }
    match side {
        "begin" => Some((family, false)),
        "end" => Some((family, true)),
        _ => None,
    }
}

/// A data edge: id, source node and port, target node.
type DataEdge = (EdgeId, NodeId, PortKey, NodeId);

/// Nodes reachable from `start` along `next`, never expanding out of `stop`.
fn reach(start: NodeId, stop: NodeId, next: &BTreeMap<NodeId, Vec<NodeId>>) -> BTreeSet<NodeId> {
    let mut seen = BTreeSet::from([start]);
    let mut stack = vec![start];
    while let Some(node) = stack.pop() {
        if node == stop && node != start {
            continue;
        }
        for target in next.get(&node).into_iter().flatten() {
            if seen.insert(*target) {
                stack.push(*target);
            }
        }
    }
    seen
}

/// Zone pairing, purity, closure and nesting of one body (11 §8.4). The zone body is the
/// data cone between a begin output and its end input.
fn check_zones(
    document: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    body: &GraphBody,
    function: Option<FunctionId>,
    error_edges: &BTreeSet<EdgeId>,
    origin: &HostOrigin,
    result: &mut RegionValidation,
) {
    let mut faults: BTreeSet<(NodeId, Option<PortKey>, Option<EdgeId>)> = BTreeSet::new();
    // (family, zone id) -> (begins, ends)
    let mut groups: BTreeMap<(String, String), (Vec<NodeId>, Vec<NodeId>)> = BTreeMap::new();
    for node in body.nodes.values() {
        let Some((family, end)) = zone_of(node.definition.id.as_str()) else {
            continue;
        };
        match node.config.get("zone").and_then(serde_json::Value::as_str) {
            Some(zone) if !zone.is_empty() => {
                let group = groups
                    .entry((family.to_owned(), zone.to_owned()))
                    .or_default();
                if end { &mut group.1 } else { &mut group.0 }.push(node.id);
            }
            _ => {
                faults.insert((node.id, None, None));
            }
        }
    }
    let mut pairs = Vec::new();
    for (begins, ends) in groups.values() {
        if let ([begin], [end]) = (&begins[..], &ends[..]) {
            pairs.push((*begin, *end));
        } else {
            faults.extend(begins.iter().chain(ends).map(|node| (*node, None, None)));
        }
    }
    if pairs.is_empty() {
        emit_zone_faults(faults, function, origin, result);
        return;
    }

    let mut edges: Vec<DataEdge> = Vec::new();
    for edge in body.edges.values() {
        if error_edges.contains(&edge.id)
            || !body.nodes.contains_key(&edge.from.node)
            || !body.nodes.contains_key(&edge.to.node)
        {
            continue;
        }
        if let (Ok(from), Ok(to)) = (
            lookup_port(document, resolved, &edge.from),
            lookup_port(document, resolved, &edge.to),
        ) && from.direction == PortDirection::Output
            && from.kind == PortKind::Data
            && to.direction == PortDirection::Input
            && to.kind == PortKind::Data
        {
            edges.push((
                edge.id,
                edge.from.node,
                edge.from.port.clone(),
                edge.to.node,
            ));
        }
    }
    edges.sort();
    let mut forward: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    let mut backward: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for (_, from, _, to) in &edges {
        forward.entry(*from).or_default().push(*to);
        backward.entry(*to).or_default().push(*from);
    }

    let bodies: Vec<BTreeSet<NodeId>> = pairs
        .iter()
        .map(|(begin, end)| {
            let after = reach(*begin, *end, &forward);
            let before = reach(*end, *begin, &backward);
            after
                .intersection(&before)
                .copied()
                .filter(|node| node != begin && node != end)
                .collect()
        })
        .collect();

    for ((begin, end), inside) in pairs.iter().zip(&bodies) {
        for node in inside {
            let pure = resolved
                .get(node)
                .is_none_or(|sig| matches!(sig.kind, NodeKind::Pure | NodeKind::Volatile));
            if !pure {
                faults.insert((*node, None, None));
            }
        }
        for (id, from, port, to) in &edges {
            let source_inside = inside.contains(from) || from == begin;
            if source_inside && !inside.contains(to) && to != end {
                faults.insert((*from, Some(port.clone()), Some(*id)));
            }
        }
        for ((other_begin, other_end), _) in pairs.iter().zip(&bodies) {
            if other_begin == begin {
                continue;
            }
            if inside.contains(other_begin) != inside.contains(other_end) {
                faults.insert((*other_begin, None, None));
            }
        }
    }
    emit_zone_faults(faults, function, origin, result);
}

fn emit_zone_faults(
    faults: BTreeSet<(NodeId, Option<PortKey>, Option<EdgeId>)>,
    function: Option<FunctionId>,
    origin: &HostOrigin,
    result: &mut RegionValidation,
) {
    for (node, port, edge) in faults {
        result.diagnostics.push(Diagnostic::new(
            DiagnosticCode::ZoneStructure,
            subject(function, node, port, edge),
            origin,
        ));
    }
}

fn depth(walks: &[Walk], mut region: usize) -> usize {
    let mut depth = 0;
    while let Some(parent) = walks[region].parent {
        depth += 1;
        region = parent;
    }
    depth
}
