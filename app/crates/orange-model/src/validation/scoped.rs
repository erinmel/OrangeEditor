//! Scoped session updates (V12b). A change that edits only literals of root-body nodes
//! re-checks the nodes those literals can reach and patches the retained report, provided
//! every result it re-derives for them is unchanged apart from input diagnostics. Anything
//! else runs the full pipeline (16 §5).
//!
//! Literals are read by admission, inference and the input check alone. Inference reads
//! them through a node's generic component, and resolution also reads whether each data
//! neighbour of a component resolves. So re-resolving the edited nodes, their neighbours
//! and those nodes' components, with every neighbour of that set available, reproduces the
//! whole-document result for all of them; no other node can read the edit.
use std::collections::{BTreeMap, BTreeSet};

use super::acceptance::Accepted;
use super::inputs::node_input_diagnostics;
use super::limits::Admitted;
use super::nodes::{CallEvidence, validate_node_ids};
use super::{AnalysisOptions, Diagnostic, Severity, ValidationAnalysis, sort_diagnostics};
use crate::catalog::RegistrySnapshot;
use crate::document::{BodyRef, GraphDocument};
use crate::edit::LiteralEdit;
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
    /// Every other published diagnostic, sorted and deduplicated.
    pub base: Vec<Diagnostic>,
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
        removed = removed.replace(Admitted::default(), Admitted::of_literals(before.values()))?;
        added = added.replace(
            Admitted::default(),
            Admitted::of_literals(node.inputs.values()),
        )?;
    }
    let admitted = retained.admitted.replace(removed, added)?;
    if !options.fits(admitted) {
        return None;
    }

    let touched: BTreeSet<NodeId> = edit.before.keys().copied().collect();
    let near = with_neighbours(document, index, &touched);
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

    // Signatures are unchanged, so only the edited nodes' input checks can differ. A
    // node gaining or losing an input error would change which dependency diagnostics
    // the report suppresses; that case runs in full.
    let has_error = |list: &[Diagnostic]| list.iter().any(|d| d.severity == Severity::Error);
    let mut inputs = Vec::with_capacity(touched.len());
    for id in &touched {
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
    options.checkpoint().ok()?;

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
    Some(Scoped {
        inputs,
        resolved: ids.len(),
        components: components.len(),
    })
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
