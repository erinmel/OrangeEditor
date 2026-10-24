//! Parallel write conflicts between Fork branches (11 §11.2).
//!
//! Two branches of one Fork that write the same local or run variable report
//! `E-WRITE-CONFLICT` on the Fork, unless the variable declares last-writer-wins or every
//! such write is the same commutative `update-variable` operation. Min/Max qualify only for
//! language-owned ordered scalars. A function call writes every run variable its summary
//! lists; the callee's locals never conflict. Nested regions of a branch belong to it.
//! Registers have no registered contract, and a register reached by two branches is already
//! `E-REGION-AMBIGUOUS`. `W-STALE-CAPTURE` needs suspension-aware dataflow and is not
//! implemented here.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::catalog::{ReferenceKind, RegistrySnapshot};
use crate::document::{BodyRef, ConflictPolicy, GraphDocument, NodeInstance, VarScope};
use crate::edit::DeclId;
use crate::id::{FunctionId, NodeId, VariableId};
use crate::type_term::TypeTerm;
use crate::variable::WriteOperation;

use super::diagnostic::{Diagnostic, DiagnosticCode, Subject, sort_diagnostics};
use super::functions::{FunctionSummary, LeafEffects};
use super::regions::{RegionIndex, RegionKind, RegionRoot};

type Writes = BTreeMap<VariableId, BTreeSet<WriteOperation>>;

/// Checks every Fork against the region index. Read-only; nodes or functions without
/// accepted metadata contribute no writes (their own diagnostics keep the report invalid).
pub fn validate_state(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    regions: &RegionIndex,
    summaries: &BTreeMap<FunctionId, FunctionSummary>,
    leaf_effects: &BTreeMap<NodeId, LeafEffects>,
) -> Vec<Diagnostic> {
    let origin = registry.origin();
    let mut children: BTreeMap<&RegionRoot, Vec<&RegionRoot>> = BTreeMap::new();
    let mut branches: BTreeMap<NodeId, Vec<&RegionRoot>> = BTreeMap::new();
    for (root, region) in &regions.regions {
        if let Some(parent) = &region.parent {
            children.entry(parent).or_default().push(root);
        }
        if region.kind == RegionKind::Fork {
            branches.entry(root.node).or_default().push(root);
        }
    }

    let mut diagnostics = Vec::new();
    for (body_ref, body) in document.bodies_with_refs() {
        for node in body.nodes.values() {
            let Some(roots) = branches.get(&node.id).filter(|roots| roots.len() > 1) else {
                continue;
            };
            let mut per_branch = Vec::new();
            for root in roots {
                let mut writes = Writes::new();
                let mut stack = vec![*root];
                while let Some(current) = stack.pop() {
                    let Some(region) = regions.regions.get(current) else {
                        continue;
                    };
                    for member in &region.nodes {
                        if let Some(member) = body.nodes.get(member) {
                            collect_writes(
                                document,
                                registry,
                                summaries,
                                leaf_effects,
                                member,
                                &mut writes,
                            );
                        }
                    }
                    stack.extend(children.get(current).into_iter().flatten().copied());
                }
                per_branch.push(writes);
            }
            let mut variables = BTreeSet::new();
            for writes in &per_branch {
                variables.extend(writes.keys().copied());
            }
            for variable in variables {
                let writing: Vec<_> = per_branch
                    .iter()
                    .filter_map(|writes| writes.get(&variable))
                    .collect();
                if writing.len() < 2 || compatible(document, variable, &writing) {
                    continue;
                }
                let mut subject = Subject::new(Some(node.id), None, None).with_variable(variable);
                if let BodyRef::Function(function) = body_ref {
                    subject = subject.with_function(function);
                }
                diagnostics.push(Diagnostic::new(
                    DiagnosticCode::WriteConflict,
                    subject,
                    origin,
                ));
            }
        }
    }
    sort_diagnostics(&mut diagnostics);
    diagnostics
}

fn compatible(
    document: &GraphDocument,
    variable: VariableId,
    writing: &[&BTreeSet<WriteOperation>],
) -> bool {
    let Some(decl) = document.variables().get(&variable) else {
        return true;
    };
    if decl.conflict == ConflictPolicy::LastWriterWins {
        return true;
    }
    let operations: BTreeSet<_> = writing.iter().flat_map(|ops| ops.iter().copied()).collect();
    let mut iter = operations.iter();
    let (Some(&operation), None) = (iter.next(), iter.next()) else {
        return false;
    };
    match operation {
        WriteOperation::Add | WriteOperation::And | WriteOperation::Or | WriteOperation::Xor => {
            true
        }
        WriteOperation::Min | WriteOperation::Max => TypeTerm::parse(&decl.ty)
            .ok()
            .as_ref()
            .and_then(TypeTerm::named)
            .is_some_and(|named| {
                named.namespace == "core"
                    && named.major == 1
                    && named.args.is_empty()
                    && matches!(
                        named.name.as_str(),
                        "i32"
                            | "i64"
                            | "u32"
                            | "u64"
                            | "bool"
                            | "string"
                            | "duration"
                            | "timestamp"
                    )
            }),
        _ => false,
    }
}

fn collect_writes(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    summaries: &BTreeMap<FunctionId, FunctionSummary>,
    leaf_effects: &BTreeMap<NodeId, LeafEffects>,
    node: &NodeInstance,
    writes: &mut Writes,
) {
    let definition = node.definition.id.as_str();
    let core = node.provider.is_none() && node.definition.version == 1 && node.config_version == 1;
    match definition {
        "core/set-variable" | "core/update-variable" if core => {
            let Ok(variable) = super::variables::variable_reference(registry, node) else {
                return;
            };
            let operation = if definition == "core/set-variable" {
                Some(WriteOperation::Set)
            } else {
                node.config
                    .get("operation")
                    .and_then(Value::as_str)
                    .and_then(WriteOperation::parse_update)
            };
            if let (Some(operation), Some(decl)) = (operation, document.variables().get(&variable))
                && matches!(decl.scope, VarScope::Local | VarScope::Run)
            {
                writes.entry(variable).or_default().insert(operation);
            }
        }
        "core/call" if core => {
            if let Ok(DeclId::Function(callee)) = registry.reference(node, ReferenceKind::Function)
                && let Some(summary) = summaries.get(&callee)
            {
                for (variable, operations) in &summary.run_writes {
                    writes.entry(*variable).or_default().extend(operations);
                }
            }
        }
        _ if definition.starts_with("core/") && node.provider.is_none() => {}
        _ => {
            if let Some(LeafEffects::Known(items)) = leaf_effects.get(&node.id) {
                for item in items {
                    if document
                        .variables()
                        .get(&item.variable)
                        .is_some_and(|decl| matches!(decl.scope, VarScope::Local | VarScope::Run))
                    {
                        writes
                            .entry(item.variable)
                            .or_default()
                            .insert(item.operation);
                    }
                }
            }
        }
    }
}
