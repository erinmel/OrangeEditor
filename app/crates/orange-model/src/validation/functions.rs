//! Graph-local function effect and execution-kind summaries.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::catalog::{ReferenceKind, RegistrySnapshot, intrinsic, is_intrinsic};
use crate::document::{
    GraphDocument, NodeKind, SignatureSnapshot, Suspends, SuspensionContractIssue, VarScope,
    suspension_contract_issue,
};
use crate::edit::DeclId;
use crate::id::{FunctionId, NodeId, VariableId};
pub use crate::variable::WriteOperation;

use super::diagnostic::{Diagnostic, DiagnosticCode, HostOrigin, Subject, sort_diagnostics};
use super::nodes::NodeValidation;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct VariableWrite {
    pub variable: VariableId,
    pub operation: WriteOperation,
}

/// Fresh host-supplied leaf effects for the exact immutable document/config/provider/lock
/// snapshot being analyzed. `Known(vec![])` is explicit proof of no writes. These facts
/// must not be reused after edits, rebinding, or lock changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeafEffects {
    Known(Vec<VariableWrite>),
    GraphBodyUnavailable,
}

/// Fresh resolved signatures and leaf effects bound to one document content state.
///
/// The embedding host constructs this evidence only after resolving metadata for the exact
/// document/config/provider/lock snapshot. Its private state token prevents reuse after a
/// transaction changes that document; a clone of the unchanged document retains the token.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionEvidence {
    document_state: u64,
    resolved: BTreeMap<NodeId, SignatureSnapshot>,
    leaf_effects: BTreeMap<NodeId, LeafEffects>,
}

impl FunctionEvidence {
    /// Binds host-resolved metadata to the document's current content state.
    ///
    /// Calling this constructor is the host's declaration that both maps were freshly
    /// resolved for `document`; it does not authenticate providers or metadata sources.
    pub fn new(
        document: &GraphDocument,
        resolved: BTreeMap<NodeId, SignatureSnapshot>,
        leaf_effects: BTreeMap<NodeId, LeafEffects>,
    ) -> Self {
        Self {
            document_state: document.state_id(),
            resolved,
            leaf_effects,
        }
    }

    /// Binds the projections produced by [`validate_nodes`](super::validate_nodes) for
    /// `document` and the host-supplied `leaf_effects`. Leaf effects are never derived
    /// from the registry: a package leaf without an entry leaves its function incomplete
    /// (11 §14).
    pub fn from_nodes(
        document: &GraphDocument,
        nodes: &NodeValidation,
        leaf_effects: BTreeMap<NodeId, LeafEffects>,
    ) -> Self {
        Self::new(document, nodes.resolved.clone(), leaf_effects)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSummary {
    pub kind: NodeKind,
    /// Conservative function-level summary: any possibly suspending node yields Maybe.
    pub suspends: Suspends,
    pub run_writes: BTreeMap<VariableId, BTreeSet<WriteOperation>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FunctionAnalysis {
    pub diagnostics: Vec<Diagnostic>,
    pub summaries: BTreeMap<FunctionId, FunctionSummary>,
    pub incomplete_functions: BTreeSet<FunctionId>,
    /// Directed closed paths; each path starts at its least rotation and repeats its first ID.
    pub recursion_paths: Vec<Vec<FunctionId>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum KindRank {
    Pure,
    Volatile,
    Action,
    Async,
}

impl KindRank {
    fn kind(self) -> NodeKind {
        match self {
            Self::Pure => NodeKind::Pure,
            Self::Volatile => NodeKind::Volatile,
            Self::Action => NodeKind::Action,
            Self::Async => NodeKind::Async,
        }
    }

    fn max(self, other: Self) -> Self {
        if self >= other { self } else { other }
    }
}

/// Analyzes graph-local functions without executing package code or changing the document.
/// `evidence` must have been freshly constructed for this exact immutable document and its
/// current config, provider, and lock identities. A mismatched document state produces stale
/// metadata diagnostics and no summaries.
pub fn analyze_functions(
    document: &GraphDocument,
    evidence: &FunctionEvidence,
    registry: &RegistrySnapshot,
) -> FunctionAnalysis {
    let origin = registry.origin();
    let mut result = FunctionAnalysis::default();
    if evidence.document_state != document.state_id() {
        for function_id in document.functions().keys().copied() {
            result.incomplete_functions.insert(function_id);
            result.diagnostics.push(Diagnostic::new(
                DiagnosticCode::MetadataStale,
                Subject::new(None, None, None).with_function(function_id),
                origin,
            ));
        }
        sort_diagnostics(&mut result.diagnostics);
        return result;
    }

    let resolved = &evidence.resolved;
    let leaf_effects = &evidence.leaf_effects;
    let calls = call_graph(document, registry, &mut result);
    let (cycles, cyclic) = find_cycles(&calls);
    for path in &cycles {
        let mut diagnostic = Diagnostic::new(
            DiagnosticCode::Recursion,
            Subject::new(None, None, None).with_function(path[0]),
            origin,
        );
        diagnostic.severity = super::diagnostic::Severity::Error;
        result.diagnostics.push(diagnostic);
    }
    result.recursion_paths = cycles;

    result.incomplete_functions.extend(cyclic.iter().copied());
    let order = callee_first_order(&calls, &cyclic);
    for function_id in order {
        let Some(function) = document.functions().get(&function_id) else {
            continue;
        };
        let mut rank = KindRank::Pure;
        let mut may_suspend = false;
        let mut complete = !result.incomplete_functions.contains(&function_id);
        let mut writes = BTreeMap::<VariableId, BTreeSet<WriteOperation>>::new();

        for node in function.body.nodes.values() {
            let definition = node.definition.id.as_str();
            if is_intrinsic(definition)
                && (node.provider.is_some()
                    || node.definition.version != 1
                    || node.config_version != 1)
            {
                emit_at(
                    &mut result,
                    DiagnosticCode::FunctionMetadataUnavailable,
                    function_id,
                    node.id,
                    origin,
                );
                complete = false;
                continue;
            }
            if is_marker(node) {
                continue;
            }
            if definition == "core/call" {
                let Some(callee) = calls
                    .get(&function_id)
                    .and_then(|items| items.get(&node.id))
                else {
                    complete = false;
                    continue;
                };
                let Some(summary) = result.summaries.get(callee) else {
                    complete = false;
                    continue;
                };
                rank = rank.max(rank_of(summary.kind));
                may_suspend |= summary.suspends != Suspends::Never;
                merge_writes(&mut writes, &summary.run_writes);
                continue;
            }
            if definition == "core/get-variable" {
                let variable = super::variables::variable_reference(registry, node).ok();
                if variable.is_none_or(|id| !document.variables().contains_key(&id)) {
                    emit_at(
                        &mut result,
                        DiagnosticCode::VariableUnknown,
                        function_id,
                        node.id,
                        origin,
                    );
                    complete = false;
                    continue;
                }
            }
            if is_intrinsic(definition) && definition != "core/call" {
                let Some((kind, suspends)) = intrinsic(definition).and_then(|rule| rule.summary)
                else {
                    emit_at(
                        &mut result,
                        DiagnosticCode::FunctionMetadataUnavailable,
                        function_id,
                        node.id,
                        origin,
                    );
                    complete = false;
                    continue;
                };
                rank = rank.max(rank_of(kind));
                may_suspend |= suspends != Suspends::Never;
                if is_variable_write(definition) {
                    if definition == "core/update-variable" {
                        match registry.contract(document, node) {
                            Ok(Some(_)) => {}
                            Ok(None) => {
                                emit_at(
                                    &mut result,
                                    DiagnosticCode::FunctionMetadataUnavailable,
                                    function_id,
                                    node.id,
                                    origin,
                                );
                                complete = false;
                                continue;
                            }
                            Err(error) => {
                                emit_at(
                                    &mut result,
                                    super::nodes::contract_code(error),
                                    function_id,
                                    node.id,
                                    origin,
                                );
                                complete = false;
                                continue;
                            }
                        }
                    }
                    match core_variable_write(registry, node, definition) {
                        Ok(write) => {
                            if document.variables().get(&write.variable).is_none() {
                                emit_at(
                                    &mut result,
                                    DiagnosticCode::VariableUnknown,
                                    function_id,
                                    node.id,
                                    origin,
                                );
                                complete = false;
                            } else if document.variables()[&write.variable].scope
                                == VarScope::Workspace
                            {
                                emit_at(
                                    &mut result,
                                    DiagnosticCode::FunctionMetadataUnavailable,
                                    function_id,
                                    node.id,
                                    origin,
                                );
                                complete = false;
                            } else if document.variables()[&write.variable].scope == VarScope::Run {
                                writes
                                    .entry(write.variable)
                                    .or_default()
                                    .insert(write.operation);
                            }
                        }
                        Err(code) => {
                            emit_at(&mut result, code, function_id, node.id, origin);
                            complete = false;
                        }
                    }
                }
                continue;
            }

            let Some(snapshot) = resolved.get(&node.id) else {
                emit_at(
                    &mut result,
                    DiagnosticCode::FunctionMetadataUnavailable,
                    function_id,
                    node.id,
                    origin,
                );
                complete = false;
                continue;
            };
            if snapshot.kind == NodeKind::Entry {
                emit_at(
                    &mut result,
                    DiagnosticCode::EntryInFunction,
                    function_id,
                    node.id,
                    origin,
                );
                complete = false;
                continue;
            }
            if let Some(issue) = suspension_contract_issue(snapshot.kind, snapshot.suspends) {
                let code = match issue {
                    SuspensionContractIssue::Missing => DiagnosticCode::SuspendsMissing,
                    SuspensionContractIssue::Invalid(_) => DiagnosticCode::SuspendsInvalid,
                };
                emit_at(&mut result, code, function_id, node.id, origin);
                complete = false;
                continue;
            }
            let suspension = snapshot.suspension().unwrap_or(Suspends::Never);
            rank = rank.max(rank_of(snapshot.kind));
            may_suspend |= suspension != Suspends::Never;

            if matches!(
                leaf_effects.get(&node.id),
                Some(LeafEffects::GraphBodyUnavailable)
            ) {
                emit_at(
                    &mut result,
                    DiagnosticCode::FunctionMetadataUnavailable,
                    function_id,
                    node.id,
                    origin,
                );
                complete = false;
                continue;
            }
            if matches!(snapshot.kind, NodeKind::Pure | NodeKind::Volatile)
                && matches!(leaf_effects.get(&node.id), Some(LeafEffects::Known(items)) if !items.is_empty())
            {
                emit_at(
                    &mut result,
                    DiagnosticCode::FunctionMetadataUnavailable,
                    function_id,
                    node.id,
                    origin,
                );
                complete = false;
                continue;
            }
            match snapshot.kind {
                NodeKind::Pure | NodeKind::Volatile => {}
                _ => match leaf_effects.get(&node.id) {
                    Some(LeafEffects::Known(items)) => {
                        for item in items {
                            if document.variables().get(&item.variable).is_none() {
                                emit_at(
                                    &mut result,
                                    DiagnosticCode::VariableUnknown,
                                    function_id,
                                    node.id,
                                    origin,
                                );
                                complete = false;
                            } else if document.variables()[&item.variable].scope == VarScope::Run {
                                writes
                                    .entry(item.variable)
                                    .or_default()
                                    .insert(item.operation);
                            }
                        }
                    }
                    Some(LeafEffects::GraphBodyUnavailable) | None => {
                        emit_at(
                            &mut result,
                            DiagnosticCode::FunctionMetadataUnavailable,
                            function_id,
                            node.id,
                            origin,
                        );
                        complete = false;
                    }
                },
            }
        }

        let inferred = rank.kind();
        if complete && function.kind != inferred {
            emit(
                &mut result,
                DiagnosticCode::FunctionKind,
                function_id,
                origin,
            );
        }
        if !cyclic.contains(&function_id) && complete {
            result.summaries.insert(
                function_id,
                FunctionSummary {
                    kind: inferred,
                    suspends: if may_suspend {
                        Suspends::Maybe
                    } else {
                        Suspends::Never
                    },
                    run_writes: writes,
                },
            );
        } else {
            result.incomplete_functions.insert(function_id);
        }
    }
    result.incomplete_functions.extend(cyclic.iter().copied());
    result.diagnostics = sort_diagnostics_owned(result.diagnostics);
    result
}

fn call_graph(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    result: &mut FunctionAnalysis,
) -> BTreeMap<FunctionId, BTreeMap<NodeId, FunctionId>> {
    let origin = registry.origin();
    let mut graph = BTreeMap::new();
    for (id, function) in document.functions() {
        let mut edges = BTreeMap::new();
        for node in function
            .body
            .nodes
            .values()
            .filter(|n| n.definition.id.as_str() == "core/call")
        {
            if node.provider.is_some() || node.definition.version != 1 || node.config_version != 1 {
                emit_at(
                    result,
                    DiagnosticCode::FunctionMetadataUnavailable,
                    *id,
                    node.id,
                    origin,
                );
                result.incomplete_functions.insert(*id);
                continue;
            }
            let target = match registry.reference(node, ReferenceKind::Function) {
                Ok(DeclId::Function(target)) => Some(target),
                _ => None,
            };
            match target {
                None => {
                    emit_at(
                        result,
                        DiagnosticCode::CallTargetMissing,
                        *id,
                        node.id,
                        origin,
                    );
                    result.incomplete_functions.insert(*id);
                }
                Some(target) if !document.functions().contains_key(&target) => {
                    emit_at(
                        result,
                        DiagnosticCode::CallTargetUnknown,
                        *id,
                        node.id,
                        origin,
                    );
                    result.incomplete_functions.insert(*id);
                }
                Some(target) => {
                    edges.insert(node.id, target);
                }
            }
        }
        graph.insert(*id, edges);
    }
    graph
}

fn is_marker(node: &crate::document::NodeInstance) -> bool {
    node.provider.is_none()
        && node.config_version == 1
        && matches!(
            node.definition.id.as_str(),
            "core/function-entry" | "core/function-outputs" | "core/return"
        )
}

fn core_variable_write(
    registry: &RegistrySnapshot,
    node: &crate::document::NodeInstance,
    definition: &str,
) -> Result<VariableWrite, DiagnosticCode> {
    let variable = super::variables::variable_reference(registry, node)?;
    let operation = if definition == "core/set-variable" {
        Some(WriteOperation::Set)
    } else {
        node.config
            .get("operation")
            .and_then(Value::as_str)
            .and_then(WriteOperation::parse_update)
    }
    .ok_or(DiagnosticCode::UpdateOperation)?;
    Ok(VariableWrite {
        variable,
        operation,
    })
}

fn is_variable_write(id: &str) -> bool {
    matches!(id, "core/set-variable" | "core/update-variable")
}
fn rank_of(kind: NodeKind) -> KindRank {
    match kind {
        NodeKind::Pure => KindRank::Pure,
        NodeKind::Volatile => KindRank::Volatile,
        NodeKind::Action => KindRank::Action,
        NodeKind::Async | NodeKind::Iterator => KindRank::Async,
        NodeKind::Entry => KindRank::Action,
    }
}
fn merge_writes(
    target: &mut BTreeMap<VariableId, BTreeSet<WriteOperation>>,
    source: &BTreeMap<VariableId, BTreeSet<WriteOperation>>,
) {
    for (id, ops) in source {
        target.entry(*id).or_default().extend(ops);
    }
}
fn emit(result: &mut FunctionAnalysis, code: DiagnosticCode, id: FunctionId, origin: &HostOrigin) {
    result.diagnostics.push(Diagnostic::new(
        code,
        Subject::new(None, None, None).with_function(id),
        origin,
    ));
}

fn emit_at(
    result: &mut FunctionAnalysis,
    code: DiagnosticCode,
    id: FunctionId,
    node: NodeId,
    origin: &HostOrigin,
) {
    result.diagnostics.push(Diagnostic::new(
        code,
        Subject::new(Some(node), None, None).with_function(id),
        origin,
    ));
}

fn find_cycles(
    graph: &BTreeMap<FunctionId, BTreeMap<NodeId, FunctionId>>,
) -> (Vec<Vec<FunctionId>>, BTreeSet<FunctionId>) {
    let adjacency: BTreeMap<_, Vec<_>> = graph
        .iter()
        .map(|(id, edges)| {
            (
                *id,
                edges
                    .values()
                    .copied()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
            )
        })
        .collect();
    let mut visited = BTreeSet::new();
    let mut finish = Vec::with_capacity(graph.len());
    for start in graph.keys().copied() {
        if visited.contains(&start) {
            continue;
        }
        visited.insert(start);
        let mut stack = vec![(start, 0usize)];
        while let Some((current, cursor)) = stack.last_mut() {
            let next = adjacency
                .get(current)
                .and_then(|edges| edges.get(*cursor))
                .copied();
            if let Some(next) = next {
                *cursor += 1;
                if visited.insert(next) {
                    stack.push((next, 0));
                }
            } else {
                let (done, _) = stack.pop().expect("nonempty traversal stack");
                finish.push(done);
            }
        }
    }
    let mut reverse: BTreeMap<FunctionId, BTreeSet<FunctionId>> =
        graph.keys().map(|id| (*id, BTreeSet::new())).collect();
    for (from, targets) in &adjacency {
        for to in targets {
            reverse.entry(*to).or_default().insert(*from);
        }
    }
    visited.clear();
    let mut witnesses = Vec::new();
    let mut cyclic = BTreeSet::new();
    for start in finish.into_iter().rev() {
        if !visited.insert(start) {
            continue;
        }
        let mut component = BTreeSet::new();
        let mut stack = vec![start];
        while let Some(current) = stack.pop() {
            component.insert(current);
            for next in reverse.get(&current).into_iter().flatten() {
                if visited.insert(*next) {
                    stack.push(*next);
                }
            }
        }
        let is_cycle = component.len() > 1
            || adjacency
                .get(&start)
                .is_some_and(|targets| targets.contains(&start));
        if is_cycle {
            cyclic.extend(component.iter().copied());
            if let Some(witness) = cycle_witness(&component, &adjacency) {
                witnesses.push(witness);
            }
        }
    }
    witnesses.sort();
    (witnesses, cyclic)
}

fn cycle_witness(
    component: &BTreeSet<FunctionId>,
    adjacency: &BTreeMap<FunctionId, Vec<FunctionId>>,
) -> Option<Vec<FunctionId>> {
    let start = *component.iter().next()?;
    if adjacency
        .get(&start)
        .is_some_and(|targets| targets.contains(&start))
    {
        return Some(vec![start, start]);
    }
    let first = adjacency
        .get(&start)
        .into_iter()
        .flatten()
        .find(|id| component.contains(id))?;
    let mut parents = BTreeMap::new();
    let mut seen = BTreeSet::from([*first]);
    let mut queue = std::collections::VecDeque::from([*first]);
    while let Some(current) = queue.pop_front() {
        if adjacency
            .get(&current)
            .is_some_and(|targets| targets.contains(&start))
        {
            let mut reverse_path = vec![current];
            let mut cursor = current;
            while cursor != *first {
                cursor = parents[&cursor];
                reverse_path.push(cursor);
            }
            reverse_path.reverse();
            let mut path = vec![start];
            path.extend(reverse_path);
            path.push(start);
            return Some(path);
        }
        for next in adjacency
            .get(&current)
            .into_iter()
            .flatten()
            .filter(|id| component.contains(id))
        {
            if seen.insert(*next) {
                parents.insert(*next, current);
                queue.push_back(*next);
            }
        }
    }
    None
}

fn callee_first_order(
    graph: &BTreeMap<FunctionId, BTreeMap<NodeId, FunctionId>>,
    cyclic: &BTreeSet<FunctionId>,
) -> Vec<FunctionId> {
    let mut pending: BTreeSet<_> = graph
        .keys()
        .copied()
        .filter(|id| !cyclic.contains(id))
        .collect();
    let mut dependency_count = BTreeMap::new();
    let mut dependents: BTreeMap<FunctionId, BTreeSet<FunctionId>> = BTreeMap::new();
    for id in &pending {
        let deps: BTreeSet<_> = graph
            .get(id)
            .into_iter()
            .flat_map(|edges| edges.values().copied())
            .filter(|callee| pending.contains(callee))
            .collect();
        dependency_count.insert(*id, deps.len());
        for dependency in deps {
            dependents.entry(dependency).or_default().insert(*id);
        }
    }
    let mut ready: BTreeSet<_> = dependency_count
        .iter()
        .filter_map(|(id, count)| (*count == 0).then_some(*id))
        .collect();
    let mut result = Vec::new();
    while let Some(id) = ready.pop_first() {
        pending.remove(&id);
        result.push(id);
        for dependent in dependents.get(&id).into_iter().flatten() {
            let count = dependency_count
                .get_mut(dependent)
                .expect("known dependent");
            *count -= 1;
            if *count == 0 {
                ready.insert(*dependent);
            }
        }
    }
    result
}

fn sort_diagnostics_owned(mut diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    sort_diagnostics(&mut diagnostics);
    diagnostics
}
