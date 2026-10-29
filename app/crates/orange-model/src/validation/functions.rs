//! Graph-local function effect and execution-kind summaries.

use crate::catalog::intrinsic_id as ids;
use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::catalog::{
    NodeContract, ReferenceKind, RegistrySnapshot, VariableAccess, intrinsic, is_intrinsic,
};
use crate::document::{
    GraphDocument, NodeKind, SignatureSnapshot, Suspends, SuspensionContractIssue, VarScope,
    suspension_contract_issue,
};
use crate::edit::DeclId;
use crate::id::{FunctionId, NodeId, VariableId};
use crate::type_term::TypeTerm;
use crate::types::TypeResolveError;
pub use crate::variable::WriteOperation;

use super::diagnostic::{Diagnostic, DiagnosticCode, HostOrigin, Subject, sort_diagnostics};
use super::nodes::NodeValidation;

/// Graph-local call targets: each function's call nodes and the function each one calls.
pub(crate) type CallGraph = BTreeMap<FunctionId, BTreeMap<NodeId, FunctionId>>;

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
    let context = SummaryContext {
        document,
        resolved: &evidence.resolved,
        leaf_effects: &evidence.leaf_effects,
        registry,
        calls: &calls,
        cyclic: &cyclic,
    };
    for function_id in order {
        summarize_function(&context, function_id, &mut result);
    }
    result.incomplete_functions.extend(cyclic.iter().copied());
    result.diagnostics = sort_diagnostics_owned(result.diagnostics);
    result
}

/// Inputs shared by every function summarized in one analysis.
pub(crate) struct SummaryContext<'a> {
    pub(crate) document: &'a GraphDocument,
    pub(crate) resolved: &'a BTreeMap<NodeId, SignatureSnapshot>,
    pub(crate) leaf_effects: &'a BTreeMap<NodeId, LeafEffects>,
    pub(crate) registry: &'a RegistrySnapshot,
    pub(crate) calls: &'a CallGraph,
    pub(crate) cyclic: &'a BTreeSet<FunctionId>,
}

/// Summarizes one function into `result`. Its callees must already have been summarized,
/// so callers process functions callee first.
pub(crate) fn summarize_function(
    context: &SummaryContext<'_>,
    function_id: FunctionId,
    result: &mut FunctionAnalysis,
) {
    let Some(function) = context.document.functions().get(&function_id) else {
        return;
    };
    let mut summarizer = Summarizer {
        context,
        function_id,
        origin: context.registry.origin(),
        result,
        writes: BTreeMap::new(),
    };
    let mut rank = KindRank::Pure;
    let mut may_suspend = false;
    let mut complete = !summarizer
        .result
        .incomplete_functions
        .contains(&function_id);
    for node in function.body.nodes.values() {
        match summarizer.node(node) {
            Ok(Some((kind, suspends))) => {
                rank = rank.max(rank_of(kind));
                may_suspend |= suspends != Suspends::Never;
            }
            Ok(None) => {}
            Err(Incomplete) => complete = false,
        }
    }
    let Summarizer {
        result,
        writes,
        origin,
        ..
    } = summarizer;
    let inferred = rank.kind();
    if complete && function.kind != inferred {
        emit(result, DiagnosticCode::FunctionKind, function_id, origin);
    }
    if !context.cyclic.contains(&function_id) && complete {
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

/// A node that leaves its function's summary incomplete; its diagnostics are emitted.
struct Incomplete;

/// One function's summary under construction. An incomplete node discards nothing
/// else: the kind, suspension and writes matter only while the function stays complete.
struct Summarizer<'a, 'r> {
    context: &'a SummaryContext<'a>,
    function_id: FunctionId,
    origin: &'a HostOrigin,
    result: &'r mut FunctionAnalysis,
    writes: BTreeMap<VariableId, BTreeSet<WriteOperation>>,
}

impl Summarizer<'_, '_> {
    fn report(&mut self, node: NodeId, code: DiagnosticCode) {
        emit_at(self.result, code, self.function_id, node, self.origin);
    }

    fn fail(&mut self, node: NodeId, code: DiagnosticCode) -> Incomplete {
        self.report(node, code);
        Incomplete
    }

    fn record_run_write(&mut self, variable: VariableId, operation: WriteOperation) {
        self.writes.entry(variable).or_default().insert(operation);
    }

    /// The kind and suspension one body node contributes; None for structure markers.
    fn node(
        &mut self,
        node: &crate::document::NodeInstance,
    ) -> Result<Option<(NodeKind, Suspends)>, Incomplete> {
        let SummaryContext {
            document,
            registry,
            calls,
            ..
        } = *self.context;
        let definition = node.definition.id.as_str();
        // Documented host entries (11 §10) are never allowed in a body, even before this
        // core version registers their contracts; an unsupported entry must not hide the rule.
        if UNREGISTERED_HOST_ENTRIES.contains(&definition) {
            return Err(self.fail(node.id, DiagnosticCode::EntryInFunction));
        }
        if is_intrinsic(definition)
            && (node.provider.is_some() || node.definition.version != 1 || node.config_version != 1)
        {
            return Err(self.fail(node.id, DiagnosticCode::FunctionMetadataUnavailable));
        }
        // A function's return (or end) cancels strands a detaching fork left running
        // (11 §14). The fork stays legal, so this is a warning and not incompleteness.
        if definition == ids::FORK
            && node.config.get("join").and_then(serde_json::Value::as_str) == Some("detach")
        {
            self.report(node.id, DiagnosticCode::DetachInFunction);
        }
        if is_marker(node) {
            return Ok(None);
        }
        if definition == ids::CALL {
            let summary = calls
                .get(&self.function_id)
                .and_then(|items| items.get(&node.id))
                .and_then(|callee| self.result.summaries.get(callee))
                .ok_or(Incomplete)?;
            merge_writes(&mut self.writes, &summary.run_writes);
            return Ok(Some((summary.kind, summary.suspends)));
        }
        if definition == ids::GET_VARIABLE {
            let variable = super::variables::variable_reference(registry, node).ok();
            if variable.is_none_or(|id| !document.variables().contains_key(&id)) {
                return Err(self.fail(node.id, DiagnosticCode::VariableUnknown));
            }
        }
        if intrinsic(definition).is_some_and(|rule| rule.event_access.is_some())
            && let Err(error) = registry.contract(document, node)
        {
            return Err(self.fail(node.id, super::nodes::contract_code(error)));
        }
        if is_intrinsic(definition) {
            return self.intrinsic(node).map(Some);
        }
        self.leaf(node).map(Some)
    }

    fn intrinsic(
        &mut self,
        node: &crate::document::NodeInstance,
    ) -> Result<(NodeKind, Suspends), Incomplete> {
        let SummaryContext {
            document, registry, ..
        } = *self.context;
        let rule = intrinsic(node.definition.id.as_str());
        let Some((kind, suspends)) = rule.and_then(|rule| rule.summary) else {
            return Err(self.fail(node.id, DiagnosticCode::FunctionMetadataUnavailable));
        };
        if kind == NodeKind::Entry {
            return Err(self.fail(node.id, DiagnosticCode::EntryInFunction));
        }
        let Some(
            access @ (VariableAccess::Write
            | VariableAccess::Update
            | VariableAccess::WriteShared
            | VariableAccess::UpdateShared),
        ) = rule.and_then(|rule| rule.variable_access)
        else {
            return Ok((kind, suspends));
        };
        let shared = matches!(
            access,
            VariableAccess::WriteShared | VariableAccess::UpdateShared
        );
        if access != VariableAccess::Write {
            let contract = match registry.contract(document, node) {
                Ok(Some(contract)) => contract,
                Ok(None) => {
                    return Err(self.fail(node.id, DiagnosticCode::FunctionMetadataUnavailable));
                }
                Err(error) => return Err(self.fail(node.id, super::nodes::contract_code(error))),
            };
            if access == VariableAccess::WriteShared
                && let Err(code) = validate_shared_variable_type(registry, &contract)
            {
                return Err(self.fail(node.id, code));
            }
        }
        let write =
            core_variable_write(registry, node, access).map_err(|code| self.fail(node.id, code))?;
        match document.variables().get(&write.variable).map(|v| v.scope) {
            None => return Err(self.fail(node.id, DiagnosticCode::VariableUnknown)),
            Some(VarScope::Workspace) if !shared => {
                return Err(self.fail(node.id, DiagnosticCode::FunctionMetadataUnavailable));
            }
            Some(VarScope::Run) if !shared => {
                self.record_run_write(write.variable, write.operation)
            }
            Some(_) => {}
        }
        Ok((kind, suspends))
    }

    /// A package or graph-body leaf, summarized from its fresh signature and effects.
    fn leaf(
        &mut self,
        node: &crate::document::NodeInstance,
    ) -> Result<(NodeKind, Suspends), Incomplete> {
        let SummaryContext {
            document,
            resolved,
            leaf_effects,
            ..
        } = *self.context;
        let Some(snapshot) = resolved.get(&node.id) else {
            return Err(self.fail(node.id, DiagnosticCode::FunctionMetadataUnavailable));
        };
        if snapshot.kind == NodeKind::Entry {
            return Err(self.fail(node.id, DiagnosticCode::EntryInFunction));
        }
        if snapshot.ports.iter().any(|port| {
            port.kind == crate::document::PortKind::Event
                && port.direction == crate::document::PortDirection::Output
        }) {
            return Err(self.fail(node.id, DiagnosticCode::SubscriptionInFunction));
        }
        if let Some(issue) = suspension_contract_issue(snapshot.kind, snapshot.suspends) {
            let code = match issue {
                SuspensionContractIssue::Missing => DiagnosticCode::SuspendsMissing,
                SuspensionContractIssue::Invalid(_) => DiagnosticCode::SuspendsInvalid,
            };
            return Err(self.fail(node.id, code));
        }
        let contribution = (
            snapshot.kind,
            snapshot.suspension().unwrap_or(Suspends::Never),
        );
        let pure = matches!(snapshot.kind, NodeKind::Pure | NodeKind::Volatile);
        let items = match leaf_effects.get(&node.id) {
            Some(LeafEffects::Known(items)) => items,
            None if pure => return Ok(contribution),
            Some(LeafEffects::GraphBodyUnavailable) | None => {
                return Err(self.fail(node.id, DiagnosticCode::FunctionMetadataUnavailable));
            }
        };
        if pure {
            if items.is_empty() {
                return Ok(contribution);
            }
            return Err(self.fail(node.id, DiagnosticCode::FunctionMetadataUnavailable));
        }
        let mut complete = true;
        for item in items {
            match document.variables().get(&item.variable).map(|v| v.scope) {
                // Every unknown variable is reported, not only the first.
                None => {
                    self.report(node.id, DiagnosticCode::VariableUnknown);
                    complete = false;
                }
                Some(VarScope::Run) => self.record_run_write(item.variable, item.operation),
                Some(_) => {}
            }
        }
        if complete {
            Ok(contribution)
        } else {
            Err(Incomplete)
        }
    }
}

pub(crate) fn call_graph(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    result: &mut FunctionAnalysis,
) -> CallGraph {
    let origin = registry.origin();
    let mut graph = BTreeMap::new();
    for (id, function) in document.functions() {
        let mut edges = BTreeMap::new();
        for node in function
            .body
            .nodes
            .values()
            .filter(|n| n.definition.id.as_str() == ids::CALL)
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
            ids::FUNCTION_ENTRY | ids::FUNCTION_OUTPUTS | ids::RETURN
        )
}

fn core_variable_write(
    registry: &RegistrySnapshot,
    node: &crate::document::NodeInstance,
    access: VariableAccess,
) -> Result<VariableWrite, DiagnosticCode> {
    let variable = super::variables::variable_reference(registry, node)?;
    let operation = match access {
        VariableAccess::Write | VariableAccess::WriteShared => WriteOperation::Set,
        VariableAccess::Update | VariableAccess::UpdateShared => node
            .config
            .get("operation")
            .and_then(Value::as_str)
            .and_then(WriteOperation::parse_update)
            .ok_or(DiagnosticCode::UpdateOperation)?,
        VariableAccess::Read => return Err(DiagnosticCode::FunctionMetadataUnavailable),
    };
    Ok(VariableWrite {
        variable,
        operation,
    })
}

fn validate_shared_variable_type(
    registry: &RegistrySnapshot,
    contract: &NodeContract,
) -> Result<(), DiagnosticCode> {
    let ty = contract
        .ports
        .iter()
        .find(|port| {
            port.direction == crate::document::PortDirection::Input
                && port.kind == crate::document::PortKind::Data
        })
        .and_then(|port| port.ty.as_ref())
        .ok_or(DiagnosticCode::FunctionMetadataUnavailable)?;
    let term = TypeTerm::parse(ty).map_err(|error| match error {
        crate::error::IdError::TypeTooDeep { .. } => DiagnosticCode::TypeComplexity,
        _ => DiagnosticCode::TypeMismatch,
    })?;
    registry
        .types()
        .validate(&term)
        .map_err(|error| match error {
            TypeResolveError::TooDeep { .. } | TypeResolveError::DerivationTooDeep { .. } => {
                DiagnosticCode::TypeComplexity
            }
            _ => DiagnosticCode::TypeMismatch,
        })
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

/// Host entry identities from the 11 §10 table that the core registry does not yet define.
const UNREGISTERED_HOST_ENTRIES: &[&str] = &[
    ids::ON_START,
    ids::ON_STOP,
    "core/on-manual",
    "core/on-timer",
    "core/on-timeout",
    "core/on-variable-changed",
    "core/on-watch",
];

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

pub(crate) fn find_cycles(
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
    let ordered = adjacency
        .iter()
        .map(|(id, edges)| (*id, edges.iter().copied().collect()))
        .collect();
    let mut witnesses = Vec::new();
    let mut cyclic = BTreeSet::new();
    for component in super::graph::strongly_connected_components(&ordered) {
        #[expect(
            clippy::expect_used,
            reason = "strongly connected components are never empty"
        )]
        let start = *component.first().expect("components are nonempty");
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

pub(crate) fn callee_first_order(
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
            #[expect(
                clippy::expect_used,
                reason = "every dependent was counted when the graph was built"
            )]
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
