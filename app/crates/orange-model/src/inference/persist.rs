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

use std::collections::BTreeSet;

use super::component::{ComponentInference, InferenceIssue, infer_components};
use crate::catalog::{ReferenceKind, RegistrySnapshot};
use crate::document::GraphDocument;
use crate::edit::{ChangeSet, Committed, DeclId, DocCommand};
use crate::error::TxError;
use crate::id::{EdgeId, NodeId};
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
///
/// Fails with the transaction error `edit` itself would produce.
pub fn plan_edit(
    document: &GraphDocument,
    edit: impl IntoIterator<Item = DocCommand>,
    registry: &RegistrySnapshot,
) -> Result<InferencePlan, TxError> {
    let mut commands: Vec<DocCommand> = edit.into_iter().collect();
    let mut after = document.clone();
    let change = after.apply(commands.clone())?.change_set;
    let touched = touched_nodes(document, &after, &change, registry);
    let index = DocIndex::build(&after);
    let mut issues = Vec::new();
    for component in infer_components(&after, &index, touched, registry) {
        commands.extend(component_updates(&after, registry, &component));
        issues.extend(component.issues);
    }
    Ok(InferencePlan { commands, issues })
}

/// Commits `edit` and its inferred updates as one transaction.
///
/// On error nothing changes, revision included. See [`plan_edit`] for what is inferred.
pub fn apply_inferred(
    document: &mut GraphDocument,
    edit: impl IntoIterator<Item = DocCommand>,
    registry: &RegistrySnapshot,
) -> Result<InferredCommit, TxError> {
    let plan = plan_edit(document, edit, registry)?;
    let committed = document.apply(plan.commands)?;
    Ok(InferredCommit {
        committed,
        issues: plan.issues,
    })
}

/// Surviving nodes whose inference inputs the transaction changed. Removed edges are looked
/// up in `before`, because their endpoints are gone from `after`. Changed edges contribute
/// endpoints from both documents, including when a disconnect/connect reuses an ID. A
/// replaced function body reports only its function, so all of its nodes count as touched.
fn touched_nodes(
    before: &GraphDocument,
    after: &GraphDocument,
    change: &ChangeSet,
    registry: &RegistrySnapshot,
) -> BTreeSet<NodeId> {
    let endpoints = |document: &GraphDocument, edges: &BTreeSet<EdgeId>| {
        edges
            .iter()
            .filter_map(|id| document.edge(*id))
            .flat_map(|edge| [edge.from.node, edge.to.node])
            .collect::<Vec<_>>()
    };
    let mut touched: BTreeSet<NodeId> = change
        .nodes
        .added
        .iter()
        .chain(&change.nodes.changed)
        .copied()
        .collect();
    touched.extend(endpoints(after, &change.edges.added));
    touched.extend(endpoints(before, &change.edges.removed));
    touched.extend(endpoints(before, &change.edges.changed));
    touched.extend(endpoints(after, &change.edges.changed));
    for decl in change.decls.added.iter().chain(&change.decls.changed) {
        if let DeclId::Function(function) = decl
            && let Some(function) = after.functions().get(function)
        {
            touched.extend(function.body.nodes.keys().copied());
        }
    }
    let variable_changes = change
        .decls
        .added
        .iter()
        .chain(&change.decls.changed)
        .chain(&change.decls.removed)
        .filter_map(|decl| match decl {
            DeclId::Variable(id) => Some(*id),
            DeclId::Event(_) | DeclId::Function(_) => None,
        })
        .collect::<BTreeSet<_>>();
    if !variable_changes.is_empty() {
        // This correctness-first scan uses the registry's typed reference parser in both
        // states. The document stores no registry-specific reverse index, and removing a
        // declaration still finds consumers through their persisted typed reference.
        for document in [before, after] {
            let consumers = document
                .bodies()
                .flat_map(|body| body.nodes.values())
                .filter(|node| {
                    registry.references(node).any(|(field, parsed)| {
                        field.kind == ReferenceKind::Variable
                            && matches!(parsed, Ok(DeclId::Variable(id)) if variable_changes.contains(&id))
                    })
                })
                .map(|node| node.id)
                .collect::<BTreeSet<_>>();
            touched.extend(consumers.iter().copied());
            // On removal, a contextual consumer no longer has a contract, so inference
            // cannot discover a type-variable edge from that endpoint. Seed the opposite
            // endpoint as well; it then solves without stale snapshot evidence and becomes
            // ambiguous unless another live source exists.
            for body in document.bodies() {
                touched.extend(body.edges.values().filter_map(|edge| {
                    if consumers.contains(&edge.from.node) {
                        Some(edge.to.node)
                    } else if consumers.contains(&edge.to.node) {
                        Some(edge.from.node)
                    } else {
                        None
                    }
                }));
            }
        }
    }
    touched.retain(|id| after.node(*id).is_some());
    touched
}

/// Commands bringing the component's nodes in `document` to the inferred state. Nodes
/// already in that state, or without a template, produce none.
fn component_updates(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    component: &ComponentInference,
) -> Vec<DocCommand> {
    let mut commands = Vec::new();
    for &id in &component.nodes {
        let Some(node) = document.node(id) else {
            continue;
        };
        let Ok(Some(template)) = registry.contract(document, node) else {
            continue;
        };
        let args = component.type_args.get(&id).cloned().unwrap_or_default();
        let snapshot = project_with_suspension(
            template.kind,
            template.suspends,
            template.ports.iter().cloned(),
            &args,
        );
        let key = semantic_key(&snapshot);
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
