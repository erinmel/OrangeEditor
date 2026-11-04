//! Explicit provider rebinds (04 "Hot update/removal", ADR-07, 12 §8): a package update
//! never moves a document's nodes to another provider on its own. A rebind is requested,
//! previewed and checked on a copy before it can be committed.
//!
//! [`plan_rebind`] selects the affected nodes, builds the edit from the ordinary
//! [`DocCommand::PutPackageLock`] and [`DocCommand::Rebind`] commands, applies it to a copy
//! and validates the copy. Only an edit that introduces no new error yields a plan;
//! [`apply_rebind`] commits that plan as one transaction.

use std::collections::BTreeSet;

use thiserror::Error;

use super::{Committed, DocCommand, PlanApplyError, Preconditions};
use crate::catalog::{ProviderKey, RegistrySnapshot};
use crate::document::{
    DefinitionRef, Extensions, GraphDocument, PackageLockEntry, ProviderBinding,
};
use crate::error::TxError;
use crate::id::NodeId;
use crate::validation::{Diagnostic, Severity, validate};

/// Which existing bindings a rebind moves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebindSelection {
    /// Every node whose binding, with the document's lock, names exactly this provider.
    Provider(ProviderKey),
    /// Every node of this definition version that has a provider binding.
    Definition(DefinitionRef),
}

/// Move the bindings `from` selects to the exact provider `to`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebindRequest {
    pub from: RebindSelection,
    pub to: ProviderKey,
}

/// The planner's answer for a well-formed request. Only `Ready` can be committed.
#[derive(Debug, Clone, PartialEq)]
pub enum RebindOutcome {
    Ready(RebindPlan),
    /// The rebind would introduce these error diagnostics; nothing is committed.
    Rejected {
        affected: Vec<NodeId>,
        errors: Vec<Diagnostic>,
    },
}

/// A checked rebind: the affected nodes and the commands that rebind them. Valid only for
/// the document state and registry snapshot it records.
#[derive(Debug, Clone, PartialEq)]
pub struct RebindPlan {
    /// Rebound nodes, root body first, each body in document order.
    pub affected: Vec<NodeId>,
    commands: Vec<DocCommand>,
    preconditions: Preconditions,
}

impl RebindPlan {
    /// The target's package lock, then the rebind of every affected node.
    pub fn commands(&self) -> &[DocCommand] {
        &self.commands
    }
}

/// A request that cannot be planned at all. The document is unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RebindError {
    /// The target is not a provider of this registry snapshot; existing bindings, and the
    /// placeholders of nodes whose provider is missing, are kept.
    #[error("unknown provider `{}/{}`", .0.package, .0.id)]
    UnknownProvider(Box<ProviderKey>),
    /// No binding matches the selection, or every match is already bound to the target.
    #[error("no node binding matches the rebind selection")]
    NothingToRebind,
    /// The edit itself is refused, for example the document locks the target package
    /// version to a different content digest.
    #[error(transparent)]
    Edit(#[from] TxError),
}

/// Plans an explicit rebind (ADR-07).
///
/// The edit adds the target's package lock and rebinds the selected nodes; earlier lock
/// entries stay. It is applied to a copy and the copy is validated against `registry`: an
/// error diagnostic absent before the edit, keyed by code and subject, rejects the rebind.
/// Errors the document already had do not, so a node left unavailable by a removed
/// provider can be rebound to a compatible one. The document is never modified.
pub fn plan_rebind(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    request: RebindRequest,
) -> Result<RebindOutcome, RebindError> {
    let RebindRequest { from, to } = request;
    if registry.provider(&to).is_none() {
        return Err(RebindError::UnknownProvider(Box::new(to)));
    }
    let affected: Vec<NodeId> = document
        .bodies()
        .flat_map(|body| body.nodes.values())
        .filter(|node| {
            let Some(binding) = &node.provider else {
                return false;
            };
            let current = ProviderKey::of(document, binding);
            let selected = match &from {
                RebindSelection::Provider(key) => current.as_ref() == Some(key),
                RebindSelection::Definition(definition) => node.definition == *definition,
            };
            selected && current.as_ref() != Some(&to)
        })
        .map(|node| node.id)
        .collect();
    if affected.is_empty() {
        return Err(RebindError::NothingToRebind);
    }
    let commands = vec![
        DocCommand::PutPackageLock(PackageLockEntry {
            id: to.package.clone(),
            version: to.version.clone(),
            content_digest: to.content_digest.clone(),
            extensions: Extensions::new(),
        }),
        DocCommand::Rebind {
            nodes: affected.clone(),
            provider: ProviderBinding {
                package: to.package,
                version: to.version,
                id: to.id,
                extensions: Extensions::new(),
            },
        },
    ];
    let mut copy = document.clone();
    let _ = copy.apply(commands.clone())?;
    let before = validate(document, registry).diagnostics;
    let errors = new_errors(&before, validate(&copy, registry).diagnostics);
    if !errors.is_empty() {
        return Ok(RebindOutcome::Rejected { affected, errors });
    }
    Ok(RebindOutcome::Ready(RebindPlan {
        affected,
        commands,
        preconditions: Preconditions::capture(document, registry),
    }))
}

/// Error diagnostics of `after` whose code and subject `before` did not report.
fn new_errors(before: &[Diagnostic], after: Vec<Diagnostic>) -> Vec<Diagnostic> {
    let known: BTreeSet<_> = before
        .iter()
        .filter(|item| item.severity == Severity::Error)
        .map(|item| (item.code, &item.subject))
        .collect();
    after
        .into_iter()
        .filter(|item| {
            item.severity == Severity::Error && !known.contains(&(item.code, &item.subject))
        })
        .collect()
}

/// Commits `plan` as one transaction: one revision bump and one undo step.
///
/// The document content state and the registry snapshot must still be the ones the plan
/// was checked against; a stale plan is rejected without changing the document.
pub fn apply_rebind(
    document: &mut GraphDocument,
    registry: &RegistrySnapshot,
    plan: RebindPlan,
) -> Result<Committed, PlanApplyError> {
    plan.preconditions.check(document, registry)?;
    Ok(document.apply(plan.commands)?)
}
