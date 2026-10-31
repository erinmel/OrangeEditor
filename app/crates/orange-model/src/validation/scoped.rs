//! Scoped session updates (V12b). A change that edits only literals, type arguments or type
//! pins of root-body nodes, or only adds or removes root-body data edges, re-checks what it
//! can reach and patches the retained report, provided every result it re-derives is
//! unchanged apart from input diagnostics. Anything else runs the full pipeline (16 §5).
//!
//! Literals are read by admission, inference and the input check alone, pins by admission
//! and inference, and persisted type arguments by admission only. Inference reads literals
//! and pins through a node's generic component, and resolution also reads whether each data
//! neighbour of a component resolves. So re-resolving the edited nodes, their neighbours
//! and those nodes' components, with every neighbour of that set available, reproduces the
//! whole-document result for all of them; no other node can read the edit. A data edge is
//! also read by the connection checks, data cycles, availability and export cones, which
//! [`edge_update`] checks for the edge alone.
use std::collections::{BTreeMap, BTreeSet};

use super::acceptance::Accepted;
use super::inputs::node_input_diagnostics;
use super::limits::Admitted;
use super::nodes::{CallEvidence, validate_node_ids};
use super::{AnalysisOptions, Diagnostic, Severity, ValidationAnalysis, sort_diagnostics};
use crate::catalog::RegistrySnapshot;
use crate::document::{
    BodyRef, Edge, GraphDocument, NodeKind, PortDirection, PortKind, SignatureSnapshot,
};
use crate::edit::{ChangeSet, NodeEdit};
use crate::id::NodeId;
use crate::index::DocIndex;

/// What the last full analysis kept for scoped updates.
#[derive(Clone)]
pub(super) struct Retained {
    pub admitted: Admitted,
    /// The callee acceptance the final node resolution used.
    pub calls: Accepted,
    /// Node resolution diagnostics per subject node, sorted.
    pub node_diagnostics: BTreeMap<NodeId, Vec<Diagnostic>>,
    /// Nodes node resolution left incomplete.
    pub node_incomplete: BTreeSet<NodeId>,
    /// Input diagnostics per node, sorted; nodes without any are absent.
    pub inputs: BTreeMap<NodeId, Vec<Diagnostic>>,
    /// Configuration diagnostics per node, likewise.
    pub config: BTreeMap<NodeId, Vec<Diagnostic>>,
    /// Every other published diagnostic, sorted and deduplicated.
    pub base: Vec<Diagnostic>,
    /// The report had no error, incomplete node, zone or possibly suspending node: the
    /// conditions under which an edge edit can be checked locally. Scoped updates keep it.
    pub clean: bool,
}

/// `diagnostics` grouped by subject node and sorted, or `None` if one has no node.
pub(super) fn by_node(diagnostics: &[Diagnostic]) -> Option<BTreeMap<NodeId, Vec<Diagnostic>>> {
    let mut nodes = BTreeMap::<NodeId, Vec<Diagnostic>>::new();
    for diagnostic in diagnostics {
        nodes
            .entry(diagnostic.subject.node?)
            .or_default()
            .push(diagnostic.clone());
    }
    for list in nodes.values_mut() {
        sort_diagnostics(list);
    }
    Some(nodes)
}

/// A proven scoped update: each edited node's old and new input diagnostics, then its old
/// and new configuration diagnostics when its configuration changed, and the work it did.
pub(super) struct Scoped {
    pub inputs: Vec<(Vec<Diagnostic>, Vec<Diagnostic>)>,
    /// Nodes re-resolved, including neighbours read only for availability.
    pub resolved: usize,
    pub components: usize,
    /// Nodes added or removed, whose signatures the update published or withdrew.
    pub changed_nodes: BTreeSet<NodeId>,
}

/// Re-checks a literal, type argument or pin edit against `retained`, which it updates only on success. `None`
/// means the edit is not provably local and the caller must run the full analysis.
pub(super) fn node_update(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    options: &AnalysisOptions<'_>,
    index: &DocIndex,
    analysis: &ValidationAnalysis,
    retained: &mut Retained,
    edit: &NodeEdit,
) -> Option<Scoped> {
    options.checkpoint().ok()?;
    let mut removed = Admitted::default();
    let mut added = Admitted::default();
    for (id, before) in &edit.before {
        if document.locate_node(*id) != Some(BodyRef::Root) {
            return None;
        }
        let node = document.node(*id)?;
        // Structural passes read configuration of core intrinsics only (Fork, Switch,
        // While, zones, variables, registers); a package node's reaches admission, its
        // configuration check and its contract.
        if (before.config != node.config || before.config_version != node.config_version)
            && node.provider.is_none()
        {
            return None;
        }
        removed = removed.add(Admitted::of_node_fields(
            &before.config,
            &before.inputs,
            &before.type_args,
            &before.type_pins,
        ))?;
        added = added.add(Admitted::of_node_fields(
            &node.config,
            &node.inputs,
            &node.type_args,
            &node.type_pins,
        ))?;
    }
    let admitted = retained.admitted.replace(removed, added)?;
    if !options.fits(admitted) {
        return None;
    }
    let touched: BTreeSet<NodeId> = edit.before.keys().copied().collect();
    let (resolved, components, _) = resolution_unchanged(
        document,
        registry,
        index,
        analysis,
        retained,
        &touched,
        &BTreeSet::new(),
    )?;
    let inputs = recheck_inputs(document, registry, index, analysis, retained, &touched)?;
    let mut config = Vec::new();
    let mut schemas = super::config::ConfigSchemas::default();
    for (id, before) in &edit.before {
        let node = document.node(*id)?;
        if before.config == node.config && before.config_version == node.config_version {
            continue;
        }
        let list: Vec<Diagnostic> =
            super::config::validate_node_config(BodyRef::Root, node, registry, &mut schemas)
                .into_iter()
                .collect();
        if has_error(&list) != has_error(retained.config.get(id).map_or(&[], Vec::as_slice)) {
            return None;
        }
        config.push((*id, list));
    }
    options.checkpoint().ok()?;
    let mut scoped = commit(retained, admitted, inputs, resolved, components);
    for (id, list) in config {
        let old = if list.is_empty() {
            retained.config.remove(&id)
        } else {
            retained.config.insert(id, list.clone())
        };
        scoped.inputs.push((old.unwrap_or_default(), list));
    }
    Some(scoped)
}

fn has_error(list: &[Diagnostic]) -> bool {
    list.iter().any(|d| d.severity == Severity::Error)
}

/// Re-checks an edit that only added or removed whole nodes and edges, in the root body of
/// a clean report. Every added or removed node must be a pure package node whose ports are
/// all typed data ports and that no export names; such a node is read only by its own
/// resolution, inputs and configuration, its edges and admission. Every edge must join two
/// data ports; it reaches only node resolution, the connection checks of an added edge and
/// its ports, its target's inputs, data cycles, availability and export cones, and each of
/// those is checked for the edge alone. A removed edge or node in a clean report can only
/// take requirements and cycles away.
pub(super) fn structure_update(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    options: &AnalysisOptions<'_>,
    index: &DocIndex,
    analysis: &mut ValidationAnalysis,
    retained: &mut Retained,
    change: &ChangeSet,
) -> Option<Scoped> {
    options.checkpoint().ok()?;
    let edit = change.structure_edit()?;
    if !retained.clean {
        return None;
    }
    let root = document.root();
    let added_nodes = &change.nodes().added;
    let exported = |id: &NodeId| document.exports().iter().any(|export| export.node == *id);
    let mut removed_charge = Admitted::default();
    let mut added_charge = Admitted::default();
    for id in added_nodes {
        let node = root.nodes.get(id)?;
        if node.provider.is_none() || exported(id) {
            return None;
        }
        added_charge = added_charge.add(Admitted::of_node(document, registry, node))?;
    }
    for (id, (body, node)) in &edit.removed_nodes {
        if *body != BodyRef::Root
            || node.provider.is_none()
            || exported(id)
            || !pure_data(analysis.report().resolved.get(id)?)
        {
            return None;
        }
        removed_charge = removed_charge.add(Admitted::of_node(document, registry, node))?;
    }
    let added_edges = &change.edges().added;
    let mut edges: Vec<(&Edge, bool)> =
        Vec::with_capacity(added_edges.len() + edit.removed_edges.len());
    for id in added_edges {
        edges.push((root.edges.get(id)?, true));
    }
    for (body, edge) in edit.removed_edges.values() {
        if *body != BodyRef::Root {
            return None;
        }
        edges.push((edge, false));
    }
    let mut touched: BTreeSet<NodeId> = added_nodes.clone();
    let mut targets: BTreeSet<NodeId> = added_nodes.clone();
    for (edge, is_added) in &edges {
        for end in [edge.from.node, edge.to.node] {
            if root.nodes.contains_key(&end) {
                touched.insert(end);
            } else if *is_added {
                return None;
            }
        }
        if root.nodes.contains_key(&edge.to.node) {
            targets.insert(edge.to.node);
        }
        let charge = Admitted::of_edge(edge);
        if *is_added {
            added_charge = added_charge.add(charge)?;
        } else {
            removed_charge = removed_charge.add(charge)?;
        }
    }
    let admitted = retained.admitted.replace(removed_charge, added_charge)?;
    if !options.fits(admitted) {
        return None;
    }
    let (resolved, components, fresh) = resolution_unchanged(
        document,
        registry,
        index,
        analysis,
        retained,
        &touched,
        added_nodes,
    )?;
    if fresh.values().any(|(signature, _)| !pure_data(signature)) {
        return None;
    }

    // Stage the new node set's signatures so the edge and input checks see it; a failed
    // check restores the published ones before the caller runs the full analysis.
    let staged: Vec<(NodeId, SignatureSnapshot)> = edit
        .removed_nodes
        .keys()
        .filter_map(|id| analysis.resolved_mut().remove(id).map(|old| (*id, old)))
        .collect();
    for (id, (signature, _)) in &fresh {
        analysis.resolved_mut().insert(*id, signature.clone());
    }
    let checked = edges_are_local(document, registry, index, analysis, &edges)
        .then(|| recheck_inputs(document, registry, index, analysis, retained, &targets))
        .flatten()
        .and_then(|inputs| {
            let mut schemas = super::config::ConfigSchemas::default();
            let mut config = Vec::new();
            for id in added_nodes {
                let list: Vec<Diagnostic> = super::config::validate_node_config(
                    BodyRef::Root,
                    document.node(*id)?,
                    registry,
                    &mut schemas,
                )
                .into_iter()
                .collect();
                if has_error(&list) {
                    return None;
                }
                config.push((*id, list));
            }
            options.checkpoint().ok()?;
            Some((inputs, config))
        });
    let Some((inputs, config)) = checked else {
        for id in fresh.keys() {
            analysis.resolved_mut().remove(id);
        }
        analysis.resolved_mut().extend(staged);
        return None;
    };

    let mut scoped = commit(retained, admitted, inputs, resolved, components);
    for (id, list) in config {
        if !list.is_empty() {
            retained.config.insert(id, list.clone());
            scoped.inputs.push((Vec::new(), list));
        }
    }
    for (id, (_, list)) in fresh {
        if !list.is_empty() {
            retained.node_diagnostics.insert(id, list.clone());
            scoped.inputs.push((Vec::new(), list));
        }
    }
    for id in edit.removed_nodes.keys() {
        retained.node_incomplete.remove(id);
        for map in [
            &mut retained.node_diagnostics,
            &mut retained.inputs,
            &mut retained.config,
        ] {
            if let Some(old) = map.remove(id) {
                scoped.inputs.push((old, Vec::new()));
            }
        }
    }
    scoped.changed_nodes = added_nodes
        .iter()
        .chain(edit.removed_nodes.keys())
        .copied()
        .collect();
    Some(scoped)
}

/// A pure node whose ports are all typed data ports, outputs with a source: the data
/// cycle pass needs nothing else of it, and no control or region pass sees it.
fn pure_data(signature: &SignatureSnapshot) -> bool {
    signature.kind == NodeKind::Pure
        && signature.ports.iter().all(|port| {
            port.kind == PortKind::Data
                && port.ty.is_some()
                && (port.direction == PortDirection::Input || port.source.is_some())
        })
}

/// Whether each edge, with the staged signatures, can only change its target's inputs.
fn edges_are_local(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    index: &DocIndex,
    analysis: &ValidationAnalysis,
    edges: &[(&Edge, bool)],
) -> bool {
    let report = analysis.report();
    let ports = super::ports::PortResolver::new(document, &report.resolved);
    let origin = registry.origin();
    let root = document.root();
    edges.iter().all(|(edge, is_added)| {
        let published = report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.subject.edge == Some(edge.id));
        if !is_added
            && (!root.nodes.contains_key(&edge.from.node)
                || !root.nodes.contains_key(&edge.to.node))
        {
            // The edge went with a removed node, which was pure and clean.
            return !published;
        }
        let data = matches!(
            (ports.port(&edge.from), ports.port(&edge.to)),
            (Ok(from), Ok(to)) if from.kind == PortKind::Data
                && to.kind == PortKind::Data
                && from.direction == PortDirection::Output
                && to.direction == PortDirection::Input
        );
        if !data
            || super::availability::data_edge_requires(
                document,
                &ports,
                &report.regions,
                index,
                &edge.from,
            )
        {
            return false;
        }
        let downstream = data_downstream(document, &ports, index, edge.to.node);
        if document
            .exports()
            .iter()
            .any(|export| downstream.contains(&export.node))
        {
            return false;
        }
        if *is_added {
            // Acyclic before, so a new cycle must lead from the target back to the source.
            !downstream.contains(&edge.from.node)
                && super::connections::added_data_edge_is_clean(
                    document, &ports, index, edge, origin,
                )
        } else {
            !published
        }
    })
}

/// Re-resolves `touched`, their neighbours and those nodes' generic components, with every
/// neighbour of that set available, and confirms each result equals the retained one.
/// Nodes in `added` are new: each must resolve without errors, and its results are returned.
/// Returns how many nodes were resolved, how many components they span and the new nodes'
/// results.
fn resolution_unchanged(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    index: &DocIndex,
    analysis: &ValidationAnalysis,
    retained: &Retained,
    touched: &BTreeSet<NodeId>,
    added: &BTreeSet<NodeId>,
) -> Option<(usize, usize, Fresh)> {
    let near = with_neighbours(document, index, touched);
    let components = crate::inference::component::component_nodes(
        document,
        index,
        near.iter().copied(),
        registry,
    );
    let mut affected = near;
    for component in &components {
        affected.extend(component.iter().copied());
    }
    let ids: Vec<NodeId> = with_neighbours(document, index, &affected)
        .into_iter()
        .collect();
    let calls = CallEvidence {
        accepted: retained.calls.clone(),
        pipeline: true,
    };
    let nodes = validate_node_ids(document, registry, &calls, &ids, index, None);
    let fresh = by_node(&nodes.diagnostics)?;
    let report = analysis.report();
    let empty = Vec::new();
    let mut fresh_added = Fresh::new();
    for id in &affected {
        if added.contains(id) {
            // A new node has nothing to compare with: it must resolve without errors.
            let list = fresh.get(id).cloned().unwrap_or_default();
            if nodes.incomplete_nodes.contains(id) || has_error(&list) {
                return None;
            }
            fresh_added.insert(*id, (nodes.resolved.get(id)?.clone(), list));
            continue;
        }
        if nodes.resolved.get(id) != report.resolved.get(id)
            || nodes.incomplete_nodes.contains(id) != retained.node_incomplete.contains(id)
            || fresh.get(id).unwrap_or(&empty)
                != retained.node_diagnostics.get(id).unwrap_or(&empty)
        {
            return None;
        }
    }
    if added.iter().any(|id| !fresh_added.contains_key(id)) {
        return None;
    }
    Some((ids.len(), components.len(), fresh_added))
}

/// Each added node's new signature and node diagnostics.
type Fresh = BTreeMap<NodeId, (SignatureSnapshot, Vec<Diagnostic>)>;

/// The current input diagnostics of `nodes`, whose signatures are unchanged. A node
/// gaining or losing an input error would change which dependency diagnostics the report
/// suppresses; that case runs in full.
fn recheck_inputs(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    index: &DocIndex,
    analysis: &ValidationAnalysis,
    retained: &Retained,
    nodes: &BTreeSet<NodeId>,
) -> Option<Vec<(NodeId, Vec<Diagnostic>)>> {
    let report = analysis.report();
    let empty = Vec::new();
    let mut inputs = Vec::with_capacity(nodes.len());
    for id in nodes {
        let node = document.node(*id)?;
        let mut list = match report.resolved.get(id) {
            Some(snapshot) => node_input_diagnostics(document, node, snapshot, registry, index),
            None => Vec::new(),
        };
        sort_diagnostics(&mut list);
        if has_error(&list) != has_error(retained.inputs.get(id).unwrap_or(&empty)) {
            return None;
        }
        inputs.push((*id, list));
    }
    Some(inputs)
}

/// Stores a proven update in `retained` and returns each node's old and new inputs.
fn commit(
    retained: &mut Retained,
    admitted: Admitted,
    inputs: Vec<(NodeId, Vec<Diagnostic>)>,
    resolved: usize,
    components: usize,
) -> Scoped {
    retained.admitted = admitted;
    let inputs = inputs
        .into_iter()
        .map(|(id, list)| {
            let old = if list.is_empty() {
                retained.inputs.remove(&id)
            } else {
                retained.inputs.insert(id, list.clone())
            };
            (old.unwrap_or_default(), list)
        })
        .collect();
    Scoped {
        inputs,
        resolved,
        components,
        changed_nodes: BTreeSet::new(),
    }
}

/// `node` and every node it feeds through data edges.
fn data_downstream(
    document: &GraphDocument,
    ports: &super::ports::PortResolver<'_>,
    index: &DocIndex,
    node: NodeId,
) -> BTreeSet<NodeId> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![node];
    while let Some(node) = stack.pop() {
        if !seen.insert(node) {
            continue;
        }
        for id in index.incident(node) {
            let Some(edge) = document.edge(id).filter(|edge| edge.from.node == node) else {
                continue;
            };
            if matches!(ports.port(&edge.from), Ok(port) if port.kind == PortKind::Data) {
                stack.push(edge.to.node);
            }
        }
    }
    seen
}

/// `nodes` and the other endpoint of every edge incident to one of them.
fn with_neighbours(
    document: &GraphDocument,
    index: &DocIndex,
    nodes: &BTreeSet<NodeId>,
) -> BTreeSet<NodeId> {
    let mut out = nodes.clone();
    for &node in nodes {
        for edge in index.incident(node) {
            if let Some(edge) = document.edge(edge) {
                out.insert(edge.from.node);
                out.insert(edge.to.node);
            }
        }
    }
    out
}
