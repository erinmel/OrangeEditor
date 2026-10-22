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

use super::component::{ComponentInference, InferenceIssue, TemplateCatalog, infer_components};
use crate::document::GraphDocument;
use crate::edit::{ChangeSet, Committed, DeclId, DocCommand};
use crate::error::TxError;
use crate::id::{EdgeId, NodeId};
use crate::index::DocIndex;
use crate::signature::{project, semantic_key};
use crate::types::TypeRegistry;

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
    templates: &TemplateCatalog,
    types: &TypeRegistry,
) -> Result<InferencePlan, TxError> {
    let mut commands: Vec<DocCommand> = edit.into_iter().collect();
    let mut after = document.clone();
    let change = after.apply(commands.clone())?.change_set;
    let touched = touched_nodes(document, &after, &change);
    let index = DocIndex::build(&after);
    let mut issues = Vec::new();
    for component in infer_components(&after, &index, touched, templates, types) {
        commands.extend(component_updates(&after, templates, &component));
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
    templates: &TemplateCatalog,
    types: &TypeRegistry,
) -> Result<InferredCommit, TxError> {
    let plan = plan_edit(document, edit, templates, types)?;
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
    touched.retain(|id| after.node(*id).is_some());
    touched
}

/// Commands bringing the component's nodes in `document` to the inferred state. Nodes
/// already in that state, or without a template, produce none.
fn component_updates(
    document: &GraphDocument,
    templates: &TemplateCatalog,
    component: &ComponentInference,
) -> Vec<DocCommand> {
    let mut commands = Vec::new();
    for &id in &component.nodes {
        let Some(node) = document.node(id) else {
            continue;
        };
        let Some(template) = templates.get(&node.definition) else {
            continue;
        };
        let args = component.type_args.get(&id).cloned().unwrap_or_default();
        let snapshot = project(template.kind, template.ports.iter().cloned(), &args);
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
