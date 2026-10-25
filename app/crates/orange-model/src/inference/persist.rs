//! Persists inference results as document commands (11 §5.5, 00c.4b).
//!
//! [`plan_edit`] is analysis: it applies the originating edit to a scratch copy, infers the
//! generic components that edit touched, and returns the edit followed by the commands that
//! bring each component's `type_args` and signature references up to date. The document is
//! not modified. [`apply_inferred`] commits that list as one transaction, so one undo
//! restores the edit and its inferred bindings together. Storage commands stay structural
//! and loading never infers.
//!
//! Inference issues never block a commit: an ambiguous or conflicting component is still a
//! representable document. It simply gets no `type_args` for the affected parameters, and its
//! signature keeps `$name` for them. The issues are returned for the caller to report.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use super::component::{ComponentInference, InferenceIssue, infer_components};
use crate::catalog::{RegistrySnapshot, TypeArgs, call_interface};
use crate::document::{DefinitionRef, GraphDocument, SignatureSnapshot};
use crate::edit::{ChangeSet, Committed, DeclId, DocCommand};
use crate::error::TxError;
use crate::id::{EdgeId, NodeId, SignatureKey};
use crate::index::DocIndex;
use crate::signature::{project_with_suspension, semantic_key};

/// An edit plus the commands that persist its inference result.
///
/// Valid only for the document it was planned against.
#[derive(Debug, Clone, PartialEq)]
pub struct InferencePlan {
    /// The originating commands, then the inferred updates in component and node order.
    pub commands: Vec<DocCommand>,
    /// Outcomes of every inferred component, in the same order.
    pub issues: Vec<InferenceIssue>,
}

/// A committed edit together with the inference outcomes it produced.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct InferredCommit {
    pub committed: Committed,
    pub issues: Vec<InferenceIssue>,
}

/// Plans `edit` and the inference updates it implies, without modifying `document`.
///
/// Only the generic components the edit touched are inspected (nodes it added or changed,
/// old and new ends of every edge it added, changed or removed, and every node of a function
/// body it added or replaced). Their nodes are updated to the solved `type_args`, removing entries that
/// are now ambiguous or invalid, and to the signature projected from them. `type_pins`,
/// provider bindings and nodes outside those components are never written.
/// Type-only calls persist arguments without a signature; touched calls that lose their
/// generic interface clear their cache. Contextual port loss also re-infers old neighbours.
///
/// Fails with the transaction error `edit` itself would produce.
pub fn plan_edit(
    document: &GraphDocument,
    edit: impl IntoIterator<Item = DocCommand>,
    registry: &RegistrySnapshot,
) -> Result<InferencePlan, TxError> {
    let mut scratch = document.clone();
    let mut index = DocIndex::build(&scratch);
    plan_edit_indexed(&mut scratch, &mut index, edit, registry)
}

/// Plans against a retained index without cloning the whole document.
/// Stale indexes rebuild on entry. Trial edits restore document content, revision
/// and identity. An unwinding observer invalidates the index before resuming panic.
pub fn plan_edit_indexed(
    document: &mut GraphDocument,
    index: &mut DocIndex,
    edit: impl IntoIterator<Item = DocCommand>,
    registry: &RegistrySnapshot,
) -> Result<InferencePlan, TxError> {
    if !index.is_current(document) {
        *index = DocIndex::build(document);
    }
    plan_in_place(document, index, edit, registry)
}

/// Reusable trial-edit boundary. The document journal rolls back before the
/// original index is restored; unwinds cannot leave a falsely current index.
pub(crate) fn scratch_indexed<R>(
    document: &mut GraphDocument,
    index: &mut DocIndex,
    commands: impl IntoIterator<Item = DocCommand>,
    observe: impl FnOnce(&GraphDocument, &DocIndex, &ChangeSet) -> R,
) -> Result<R, TxError> {
    reset_index_on_unwind(index, |index| {
        let result = document.scratch(commands, |after, change| {
            index.update(after, change);
            (observe(after, index, change), change.clone())
        })?;
        index.update(document, &result.1.reversed());
        Ok(result.0)
    })
}

/// Plans `edit` against `document` by applying it, reading the result and undoing it, so
/// the document and `index` are left exactly as found. Cost follows the edit and the
/// components it touches, not the document.
fn plan_in_place(
    document: &mut GraphDocument,
    index: &mut DocIndex,
    edit: impl IntoIterator<Item = DocCommand>,
    registry: &RegistrySnapshot,
) -> Result<InferencePlan, TxError> {
    let mut commands: Vec<DocCommand> = edit.into_iter().collect();
    // Removed and replaced edges and nodes exist only in the original document, so their
    // neighbours are read there, from the change the edit makes.
    let change = document.scratch(commands.clone(), |_, change| change.clone())?;
    let mut touched = touched_before(document, &change, registry);
    let planned = scratch_indexed(document, index, commands.clone(), |after, index, change| {
        let mut updates = Vec::new();
        let mut issues = Vec::new();
        touched.extend(touched_after(after, change, registry));
        touched.retain(|id| after.node(*id).is_some());
        let mut inferred_nodes = BTreeSet::new();
        for component in infer_components(after, index, touched.iter().copied(), registry) {
            inferred_nodes.extend(component.nodes.iter().copied());
            updates.extend(component_updates(after, registry, &component));
            issues.extend(component.issues);
        }
        // Calls whose function interface disappeared or became concrete are no longer
        // generic inference nodes. Clear only their stale argument cache; their historical
        // signature field and all type pins remain untouched.
        for id in touched
            .iter()
            .copied()
            .filter(|id| !inferred_nodes.contains(id))
        {
            let Some(node) = after.node(id) else { continue };
            if is_function_call(node) && !node.type_args.is_empty() {
                updates.push(DocCommand::SetTypeArgs {
                    node: id,
                    args: Default::default(),
                });
            }
        }
        (updates, issues)
    });
    let (updates, issues) = planned?;
    commands.extend(updates);
    Ok(InferencePlan { commands, issues })
}

/// Runs `plan` with `index`. If it unwinds, `index` may describe trial content under the
/// document's unchanged state, so it is replaced by one that is never current and the
/// next indexed edit rebuilds it.
fn reset_index_on_unwind<R>(index: &mut DocIndex, plan: impl FnOnce(&mut DocIndex) -> R) -> R {
    match catch_unwind(AssertUnwindSafe(|| plan(&mut *index))) {
        Ok(result) => result,
        Err(panic) => {
            *index = DocIndex::default();
            resume_unwind(panic)
        }
    }
}

/// Commits `edit` and its inferred updates as one transaction.
///
/// On error nothing changes, revision included. See [`plan_edit`] for what is inferred.
/// Builds an index of `document`; use [`apply_inferred_indexed`] to keep one between edits.
pub fn apply_inferred(
    document: &mut GraphDocument,
    edit: impl IntoIterator<Item = DocCommand>,
    registry: &RegistrySnapshot,
) -> Result<InferredCommit, TxError> {
    let mut index = DocIndex::build(document);
    apply_inferred_indexed(document, &mut index, edit, registry)
}

/// [`apply_inferred`] for a caller that keeps a [`DocIndex`] of `document`. An index that is
/// not current (the document changed without it, for example by an undo) is rebuilt first.
/// The index is updated with the commit and, like the document, unchanged on error. A small
/// edit costs in proportion to the generic components it touches.
pub fn apply_inferred_indexed(
    document: &mut GraphDocument,
    index: &mut DocIndex,
    edit: impl IntoIterator<Item = DocCommand>,
    registry: &RegistrySnapshot,
) -> Result<InferredCommit, TxError> {
    if !index.is_current(document) {
        *index = DocIndex::build(document);
    }
    let plan = plan_in_place(document, index, edit, registry)?;
    let committed = document.apply(plan.commands)?;
    index.update(document, &committed.change_set);
    Ok(InferredCommit {
        committed,
        issues: plan.issues,
    })
}

/// Surviving nodes of the original document whose inference inputs the transaction changed.
/// Removed edges are looked up here, because their endpoints are gone after the edit. Changed
/// edges contribute endpoints from both states, including when a disconnect/connect reuses an
/// ID. A config edit can remove a contextual node's ports before inference can traverse
/// them, so incident neighbours are seeded from both states, just as for declaration removal.
fn touched_before(
    before: &GraphDocument,
    change: &ChangeSet,
    registry: &RegistrySnapshot,
) -> BTreeSet<NodeId> {
    let mut touched = BTreeSet::new();
    touched.extend(edge_endpoints(before, &change.edges.removed));
    touched.extend(edge_endpoints(before, &change.edges.changed));
    touch_neighbours(before, &change.nodes.changed, &mut touched);
    touch_declaration_consumers(before, change, registry, &mut touched);
    touched
}

/// The edited document's side of [`touched_before`]. A replaced function body reports only
/// its function, so all of its nodes count as touched.
fn touched_after(
    after: &GraphDocument,
    change: &ChangeSet,
    registry: &RegistrySnapshot,
) -> BTreeSet<NodeId> {
    let mut touched: BTreeSet<NodeId> = change
        .nodes
        .added
        .iter()
        .chain(&change.nodes.changed)
        .copied()
        .collect();
    touched.extend(edge_endpoints(after, &change.edges.added));
    touched.extend(edge_endpoints(after, &change.edges.changed));
    touch_neighbours(after, &change.nodes.changed, &mut touched);
    for decl in change.decls.added.iter().chain(&change.decls.changed) {
        if let DeclId::Function(function) = decl
            && let Some(function) = after.functions().get(function)
        {
            touched.extend(function.body.nodes.keys().copied());
        }
    }
    touch_declaration_consumers(after, change, registry, &mut touched);
    touched
}

fn edge_endpoints(document: &GraphDocument, edges: &BTreeSet<EdgeId>) -> Vec<NodeId> {
    edges
        .iter()
        .filter_map(|id| document.edge(*id))
        .flat_map(|edge| [edge.from.node, edge.to.node])
        .collect()
}

/// Nodes of `document` that reference a typed declaration the transaction changed, and their
/// neighbours.
fn touch_declaration_consumers(
    document: &GraphDocument,
    change: &ChangeSet,
    registry: &RegistrySnapshot,
    touched: &mut BTreeSet<NodeId>,
) {
    let typed_declaration_changes = change
        .decls
        .added
        .iter()
        .chain(&change.decls.changed)
        .chain(&change.decls.removed)
        .copied()
        .collect::<BTreeSet<_>>();
    if typed_declaration_changes.is_empty() {
        return;
    }
    // This correctness-first scan uses the registry's typed reference parser in both
    // states. The document stores no registry-specific reverse index, and removing a
    // declaration still finds consumers through their persisted typed reference.
    let consumers = document
        .bodies()
        .flat_map(|body| body.nodes.values())
        .filter(|node| {
            registry.references(node).any(|(_, parsed)| {
                parsed.is_ok_and(|decl| typed_declaration_changes.contains(&decl))
            })
        })
        .map(|node| node.id)
        .collect::<BTreeSet<_>>();
    touched.extend(consumers.iter().copied());
    // When a contextual port contract disappears or changes shape, inference may
    // no longer discover a type-variable edge from that endpoint. Seed the
    // opposite endpoint as well; it then solves from remaining current evidence.
    touch_neighbours(document, &consumers, touched);
}

fn touch_neighbours(
    document: &GraphDocument,
    nodes: &BTreeSet<NodeId>,
    touched: &mut BTreeSet<NodeId>,
) {
    if nodes.is_empty() {
        return;
    }
    for body in document.bodies() {
        touched.extend(body.edges.values().filter_map(|edge| {
            if nodes.contains(&edge.from.node) {
                Some(edge.to.node)
            } else if nodes.contains(&edge.to.node) {
                Some(edge.from.node)
            } else {
                None
            }
        }));
    }
}

/// Commands bringing the component's nodes in `document` to the inferred state. Nodes
/// already in that state, or without a template, produce none.
fn component_updates(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    component: &ComponentInference,
) -> Vec<DocCommand> {
    let mut commands = Vec::new();
    // Nodes of one registry-declared definition with the same arguments share one projected
    // snapshot and semantic key; contextual contracts are projected per node.
    let mut projected: BTreeMap<(DefinitionRef, TypeArgs), (SignatureSnapshot, SignatureKey)> =
        BTreeMap::new();
    for &id in &component.nodes {
        let Some(node) = document.node(id) else {
            continue;
        };
        if is_function_call(node) {
            let Some(interface) = call_interface(document, node, registry).ok().flatten() else {
                continue;
            };
            if interface.type_params.is_empty() {
                continue;
            }
            let args = component.type_args.get(&id).cloned().unwrap_or_default();
            if node.type_args != args {
                commands.push(DocCommand::SetTypeArgs { node: id, args });
            }
            continue;
        }
        let Ok(Some(template)) = registry.contract(document, node) else {
            continue;
        };
        let args = component.type_args.get(&id).cloned().unwrap_or_default();
        let project = || {
            let snapshot = project_with_suspension(
                template.kind,
                template.suspends,
                template.ports.iter().cloned(),
                &args,
            );
            let key = semantic_key(&snapshot);
            (snapshot, key)
        };
        let (snapshot, key) = match &template {
            Cow::Borrowed(_) => projected
                .entry((node.definition.clone(), args.clone()))
                .or_insert_with(project)
                .clone(),
            Cow::Owned(_) => project(),
        };
        if node.type_args != args {
            commands.push(DocCommand::SetTypeArgs { node: id, args });
        }
        if node.signature.as_ref() != Some(&key) {
            commands.push(DocCommand::PutSignature {
                key: key.clone(),
                snapshot,
            });
            commands.push(DocCommand::SetSignature { node: id, key });
        }
    }
    commands
}

fn is_function_call(node: &crate::document::NodeInstance) -> bool {
    node.definition.id.as_str() == "core/call"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unwinding_plan_leaves_an_index_that_is_never_current() {
        let document = GraphDocument::new("20000000-0000-4000-8000-000000000001".parse().unwrap());
        let mut index = DocIndex::build(&document);
        assert!(index.is_current(&document));
        assert_eq!(reset_index_on_unwind(&mut index, |_| 7), 7);
        assert!(index.is_current(&document));
        let unwound = catch_unwind(AssertUnwindSafe(|| {
            reset_index_on_unwind(&mut index, |_| -> () { panic!("inference failure") })
        }));
        assert!(unwound.is_err());
        assert!(!index.is_current(&document));
    }
}
