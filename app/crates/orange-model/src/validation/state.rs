//! Parallel write conflicts between Fork branches (11 §11.2).
//!
//! Two branches of one Fork that write the same local or run variable report
//! `E-WRITE-CONFLICT` on the Fork, unless the variable declares last-writer-wins or every
//! such write is the same commutative `update-variable` operation. Min/Max qualify only for
//! language-owned ordered scalars. A function call writes every run variable its summary
//! lists; the callee's locals never conflict. Nested regions of a branch belong to it.
//! A register reached by two branches is already `E-REGION-AMBIGUOUS`; its storage and
//! initializer checks use the fresh resolved cell type. `W-STALE-CAPTURE` uses the
//! conservative dataflow approximation below, rather than proving exact suspension order.

use crate::catalog::intrinsic_id as ids;
use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::catalog::{ReferenceKind, RegistrySnapshot};
use crate::document::{
    BodyRef, ConflictPolicy, GraphDocument, NodeInstance, ReentrancyMode, SignatureSnapshot,
    VarScope,
};
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
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
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

    let mut diagnostics = validate_declarations(document, registry);
    for (body_ref, body) in document.bodies_with_refs() {
        validate_registers(body_ref, body, registry, resolved, &mut diagnostics);
        for node in body.nodes.values() {
            if node
                .activation
                .as_ref()
                .is_some_and(|policy| policy.mode == ReentrancyMode::Parallel)
            {
                let mut members = BTreeSet::new();
                for root in regions.regions.keys().filter(|root| root.node == node.id) {
                    members.extend(region_members(regions, &children, root));
                }
                stale_captures(
                    body_ref,
                    body,
                    document,
                    registry,
                    resolved,
                    &members,
                    &mut diagnostics,
                );
            }
            let Some(roots) = branches.get(&node.id).filter(|roots| roots.len() > 1) else {
                continue;
            };
            let mut per_branch = Vec::new();
            for root in roots {
                let mut writes = Writes::new();
                for member in region_members(regions, &children, root) {
                    if let Some(member) = body.nodes.get(&member) {
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

/// Register rules (11 §8.4, §12): the cell type may not contain a stream (`E-STREAM-STORE`);
/// `scope` is absent, `run` or `activation`; `initial`, when present, is a literal of the cell
/// type that decodes with the ordinary codec and is no handle or stream
/// (`E-REGISTER-CONFIG`, `E-HANDLE-LITERAL`). Two branches reaching one register are already
/// `E-REGION-AMBIGUOUS`, so no write-conflict rule is needed.
fn validate_registers(
    body_ref: BodyRef,
    body: &crate::document::GraphBody,
    registry: &RegistrySnapshot,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let (origin, types) = (registry.origin(), registry.types().as_ref());
    for node in body.nodes.values() {
        if node.definition.id.as_str() != ids::REGISTER
            || node.provider.is_some()
            || node.definition.version != 1
            || node.config_version != 1
        {
            continue;
        }
        let mut subject = Subject::new(Some(node.id), None, None);
        if let BodyRef::Function(function) = body_ref {
            subject = subject.with_function(function);
        }
        let mut emit = |code| diagnostics.push(Diagnostic::new(code, subject.clone(), origin));
        let cell = resolved.get(&node.id).and_then(|signature| {
            signature
                .ports
                .iter()
                .find(|port| port.port_id.as_str() == "current")?
                .ty
                .as_ref()
        });
        let resources = cell.and_then(|ty| super::streams::resources(ty, types));
        if resources.is_some_and(|found| found.stream) {
            emit(DiagnosticCode::StreamStore);
        }
        if node
            .config
            .get("scope")
            .is_some_and(|scope| !matches!(scope.as_str(), Some("run" | "activation")))
        {
            emit(DiagnosticCode::RegisterConfig);
        }
        let Some(initial) = node.config.get("initial") else {
            continue;
        };
        if resources.is_some_and(|found| found.handle || found.stream) {
            emit(DiagnosticCode::HandleLiteral);
            continue;
        }
        let valid = serde_json::from_value::<crate::document::literal::Literal>(initial.clone())
            .is_ok_and(|literal| {
                cell.is_none_or(|ty| literal.ty == *ty)
                    && crate::value::literal_codec::decode(&literal, types).is_ok()
            });
        if !valid {
            emit(DiagnosticCode::RegisterConfig);
        }
    }
}

/// Nodes owned by a region and all regions nested below it, iteratively.
fn region_members(
    regions: &RegionIndex,
    children: &BTreeMap<&RegionRoot, Vec<&RegionRoot>>,
    root: &RegionRoot,
) -> BTreeSet<NodeId> {
    let mut members = BTreeSet::new();
    let mut stack = vec![root];
    while let Some(current) = stack.pop() {
        if let Some(region) = regions.regions.get(current) {
            members.extend(region.nodes.iter().copied());
        }
        stack.extend(children.get(current).into_iter().flatten().copied());
    }
    members
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
        ids::SET_VARIABLE | ids::UPDATE_VARIABLE if core => {
            let Ok(variable) = super::variables::variable_reference(registry, node) else {
                return;
            };
            let operation = if definition == ids::SET_VARIABLE {
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
        ids::CALL if core => {
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

/// Variable declaration rules: a local variable is never persistent, and an initializer must
/// carry the declared type and decode with the ordinary literal codec. Resource-typed
/// initializers are reported by `validate_streams` instead.
pub(super) fn validate_declarations(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
) -> Vec<Diagnostic> {
    let (origin, types) = (registry.origin(), registry.types().as_ref());
    let mut diagnostics = Vec::new();
    for (id, decl) in document.variables() {
        let subject = Subject::default().with_variable(*id);
        if decl.persistent && decl.scope == VarScope::Local {
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::VariablePersistent,
                subject.clone(),
                origin,
            ));
        }
        let Some(init) = &decl.init else {
            continue;
        };
        let Ok(term) = TypeTerm::parse(&decl.ty) else {
            continue;
        };
        if term.core("handle").is_some() || term.core("stream").is_some() {
            continue;
        }
        if init.ty != decl.ty || crate::value::literal_codec::decode(init, types).is_err() {
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::LiteralType,
                subject,
                origin,
            ));
        }
    }
    diagnostics
}

/// `W-STALE-CAPTURE` (11 §10.1, strictest reading of "captured before a suspension"): in a
/// region of a `parallel` entry that contains a possibly suspending node, a `set-variable`
/// of V whose value depends on a `get-variable` of V through the output of an earlier exec
/// node (an action or async capture) warns. A direct Get-to-Set path and `update-variable`
/// are atomic and never warn. Captures through other variables are not tracked.
fn stale_captures(
    body_ref: BodyRef,
    body: &crate::document::GraphBody,
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    members: &BTreeSet<NodeId>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    use crate::document::{NodeKind, Suspends};
    let suspends = members.iter().any(|node| {
        resolved
            .get(node)
            .is_some_and(|s| s.suspension() != Some(Suspends::Never))
    });
    if !suspends {
        return;
    }
    let mut incoming: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for edge in body.edges.values() {
        incoming
            .entry(edge.to.node)
            .or_default()
            .push(edge.from.node);
    }
    let variable_of = |node: &NodeInstance| {
        (node.provider.is_none() && node.definition.version == 1 && node.config_version == 1)
            .then(|| super::variables::variable_reference(registry, node).ok())
            .flatten()
    };
    for id in members {
        let Some(write) = body.nodes.get(id) else {
            continue;
        };
        if write.definition.id.as_str() != ids::SET_VARIABLE {
            continue;
        }
        let Some(variable) = variable_of(write) else {
            continue;
        };
        if !document
            .variables()
            .get(&variable)
            .is_some_and(|decl| matches!(decl.scope, VarScope::Local | VarScope::Run))
        {
            continue;
        }
        let mut seen = BTreeSet::new();
        let mut stack: Vec<_> = incoming
            .get(id)
            .into_iter()
            .flatten()
            .map(|n| (*n, false))
            .collect();
        let mut stale = false;
        while let Some((current, crossed)) = stack.pop() {
            if !seen.insert((current, crossed)) {
                continue;
            }
            let Some(node) = body.nodes.get(&current) else {
                continue;
            };
            if crossed
                && node.definition.id.as_str() == ids::GET_VARIABLE
                && variable_of(node) == Some(variable)
            {
                stale = true;
                break;
            }
            let exec = resolved.get(&current).is_some_and(|s| {
                matches!(
                    s.kind,
                    NodeKind::Action | NodeKind::Async | NodeKind::Iterator
                )
            });
            stack.extend(
                incoming
                    .get(&current)
                    .into_iter()
                    .flatten()
                    .map(|n| (*n, crossed || exec)),
            );
        }
        if stale {
            let mut subject = Subject::new(Some(*id), None, None).with_variable(variable);
            if let BodyRef::Function(function) = body_ref {
                subject = subject.with_function(function);
            }
            diagnostics.push(Diagnostic::new(
                DiagnosticCode::StaleCapture,
                subject,
                registry.origin(),
            ));
        }
    }
}
