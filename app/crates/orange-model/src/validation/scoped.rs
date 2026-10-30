//! Scoped session updates (V12b). A change that edits only literals of root-body nodes, or
//! only adds or removes root-body data edges, re-checks what it can reach and patches the
//! retained report, provided every result it re-derives is unchanged apart from input
//! diagnostics. Anything else runs the full pipeline (16 §5).
//!
//! Literals are read by admission, inference and the input check alone. Inference reads
//! them through a node's generic component, and resolution also reads whether each data
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
use crate::document::{BodyRef, Edge, GraphDocument, PortDirection, PortKind};
use crate::edit::{EdgeEdit, LiteralEdit};
use crate::id::{EdgeId, NodeId};
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

/// A proven scoped update: each edited node's old and new input diagnostics, and the
/// work it did.
pub(super) struct Scoped {
    pub inputs: Vec<(Vec<Diagnostic>, Vec<Diagnostic>)>,
    /// Nodes re-resolved, including neighbours read only for availability.
    pub resolved: usize,
    pub components: usize,
}

/// Re-checks a literal edit against `retained`, which it updates only on success. `None`
/// means the edit is not provably local and the caller must run the full analysis.
pub(super) fn literal_update(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    options: &AnalysisOptions<'_>,
    index: &DocIndex,
    analysis: &ValidationAnalysis,
    retained: &mut Retained,
    edit: &LiteralEdit,
) -> Option<Scoped> {
    options.checkpoint().ok()?;
    let mut removed = Admitted::default();
    let mut added = Admitted::default();
    for (id, before) in &edit.before {
        if document.locate_node(*id) != Some(BodyRef::Root) {
            return None;
        }
        let node = document.node(*id)?;
        removed = removed.add(Admitted::of_literals(before.values()))?;
        added = added.add(Admitted::of_literals(node.inputs.values()))?;
    }
    let admitted = retained.admitted.replace(removed, added)?;
    if !options.fits(admitted) {
        return None;
    }
    let touched: BTreeSet<NodeId> = edit.before.keys().copied().collect();
    let (resolved, components) =
        resolution_unchanged(document, registry, index, analysis, retained, &touched)?;
    let inputs = recheck_inputs(document, registry, index, analysis, retained, &touched)?;
    options.checkpoint().ok()?;
    Some(commit(retained, admitted, inputs, resolved, components))
}

/// Re-checks an edit that only added or removed edges. Every edge must join two data ports
/// in the root body of a clean report. Such an edge then reaches only node resolution, the
/// connection checks of an added edge and its ports, its target's inputs, data cycles,
/// availability and export cones, and each of those is checked for the edge alone.
#[expect(
    clippy::too_many_arguments,
    reason = "the added edges come from the change set, the removed ones from the journal"
)]
pub(super) fn edge_update(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    options: &AnalysisOptions<'_>,
    index: &DocIndex,
    analysis: &ValidationAnalysis,
    retained: &mut Retained,
    added: &BTreeSet<EdgeId>,
    edit: &EdgeEdit,
) -> Option<Scoped> {
    options.checkpoint().ok()?;
    if !retained.clean {
        return None;
    }
    let root = document.root();
    let mut edges: Vec<(&Edge, bool)> = Vec::with_capacity(added.len() + edit.removed.len());
    for id in added {
        edges.push((root.edges.get(id)?, true));
    }
    for (body, edge) in edit.removed.values() {
        if *body != BodyRef::Root {
            return None;
        }
        edges.push((edge, false));
    }
    let mut removed_charge = Admitted::default();
    let mut added_charge = Admitted::default();
    let mut touched = BTreeSet::new();
    let mut targets = BTreeSet::new();
    for (edge, is_added) in &edges {
        if !root.nodes.contains_key(&edge.from.node) || !root.nodes.contains_key(&edge.to.node) {
            return None;
        }
        let charge = Admitted::of_edge(edge);
        if *is_added {
            added_charge = added_charge.add(charge)?;
        } else {
            removed_charge = removed_charge.add(charge)?;
        }
        touched.insert(edge.from.node);
        touched.insert(edge.to.node);
        targets.insert(edge.to.node);
    }
    let admitted = retained.admitted.replace(removed_charge, added_charge)?;
    if !options.fits(admitted) {
        return None;
    }
    let (resolved, components) =
        resolution_unchanged(document, registry, index, analysis, retained, &touched)?;

    let report = analysis.report();
    let ports = super::ports::PortResolver::new(document, &report.resolved);
    let origin = registry.origin();
    for (edge, is_added) in &edges {
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
            return None;
        }
        let downstream = data_downstream(document, &ports, index, edge.to.node);
        if document
            .exports()
            .iter()
            .any(|export| downstream.contains(&export.node))
        {
            return None;
        }
        if *is_added {
            // Acyclic before, so a new cycle must lead from the target back to the source.
            if downstream.contains(&edge.from.node)
                || !super::connections::added_data_edge_is_clean(
                    document, &ports, index, edge, origin,
                )
            {
                return None;
            }
        } else if report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.subject.edge == Some(edge.id))
        {
            return None;
        }
    }
    drop(ports);
    let inputs = recheck_inputs(document, registry, index, analysis, retained, &targets)?;
    options.checkpoint().ok()?;
    Some(commit(retained, admitted, inputs, resolved, components))
}

/// Re-resolves `touched`, their neighbours and those nodes' generic components, with every
/// neighbour of that set available, and confirms each result equals the retained one.
/// Returns how many nodes were resolved and how many components they span.
fn resolution_unchanged(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    index: &DocIndex,
    analysis: &ValidationAnalysis,
    retained: &Retained,
    touched: &BTreeSet<NodeId>,
) -> Option<(usize, usize)> {
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
    for id in &affected {
        if nodes.resolved.get(id) != report.resolved.get(id)
            || nodes.incomplete_nodes.contains(id) != retained.node_incomplete.contains(id)
            || fresh.get(id).unwrap_or(&empty)
                != retained.node_diagnostics.get(id).unwrap_or(&empty)
        {
            return None;
        }
    }
    Some((ids.len(), components.len()))
}

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
    let has_error = |list: &[Diagnostic]| list.iter().any(|d| d.severity == Severity::Error);
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
