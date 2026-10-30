//! Read-only node metadata, provider, signature, and generic resolution (WU-00d.2a).

use crate::catalog::intrinsic_id as ids;
use std::collections::{BTreeMap, BTreeSet};

use std::borrow::Cow;
use std::cell::OnceCell;

use crate::catalog::{
    CallInterfaceError, ContractError, NodeContract, ProviderKey, RegistrySnapshot, TypeArgs,
    body_rigid_params, call_interface, is_converter_special_form, is_function_marker, is_intrinsic,
};
use crate::document::{
    BodyRef, DefinitionRef, GraphDocument, NodeKind, OutputSource, PortKind, PortRef,
    SignatureSnapshot, Suspends, SuspensionContractIssue,
    suspension_contract_issue as classify_suspension_contract,
};
use crate::error::IdError;
use crate::id::{FunctionId, NodeId, PortId, PortKey, SignatureKey};
use crate::index::DocIndex;
use crate::inference::{InferenceIssue, infer_components};
use crate::signature;
use crate::type_term::TypeTerm;
use crate::types::{TypeRegistry, TypeResolveError};

use super::diagnostic::{
    Diagnostic, DiagnosticCode, HostOrigin, Severity, Subject, sort_diagnostics,
};
use super::scope::Scope;

/// Results of resolving nodes against immutable host metadata.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodeValidation {
    pub diagnostics: Vec<Diagnostic>,
    /// Current semantic projections for nodes with resolved metadata and complete types.
    pub resolved: BTreeMap<NodeId, SignatureSnapshot>,
    /// Nodes whose definition, provider, or type information prevented full resolution.
    pub incomplete_nodes: BTreeSet<NodeId>,
}

impl NodeValidation {
    /// True only when every node resolved and no errors were diagnosed.
    pub fn is_valid(&self) -> bool {
        self.incomplete_nodes.is_empty()
            && self
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity != Severity::Error)
    }
}

/// Accepted executable facts for graph-local callees, produced by the validation pipeline from
/// complete, conformant function summaries. Standalone node validation supplies none.
#[derive(Debug, Clone, Default)]
pub(crate) struct CallEvidence {
    pub(crate) accepted: BTreeMap<FunctionId, (NodeKind, Suspends)>,
    /// Whether an unaccepted callee is explained by the function pass rather than unknown.
    pub(crate) pipeline: bool,
}

struct NodePass<'a> {
    calls: &'a CallEvidence,
    document: &'a GraphDocument,
    index: &'a DocIndex,
    registry: &'a RegistrySnapshot,
    eligible: &'a BTreeSet<NodeId>,
    ids: &'a [NodeId],
    origin: &'a HostOrigin,
}

/// Resolves node contracts, exact providers, fresh generic arguments, and signatures.
/// The document is never changed. Intrinsic metadata and provider availability are
/// declarative host inputs; package code is not loaded or executed.
pub fn validate_nodes(document: &GraphDocument, registry: &RegistrySnapshot) -> NodeValidation {
    let index = DocIndex::build(document);
    validate_nodes_with(
        document,
        registry,
        &CallEvidence::default(),
        Scope::All,
        &index,
        None,
    )
}

pub(crate) fn validate_nodes_with(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    calls: &CallEvidence,
    scope: Scope,
    index: &DocIndex,
    components: Option<&mut ComponentCache>,
) -> NodeValidation {
    validate_node_ids(
        document,
        registry,
        calls,
        &node_ids(document, scope),
        index,
        components,
    )
}

/// Node resolution of `ids` (ascending) alone. A node's result equals the whole-scope
/// result when `ids` holds its generic component and every data neighbour of it.
pub(crate) fn validate_node_ids(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    calls: &CallEvidence,
    ids: &[NodeId],
    index: &DocIndex,
    components: Option<&mut ComponentCache>,
) -> NodeValidation {
    let origin = registry.origin();
    let mut report = NodeValidation::default();
    let mut diagnostics = Vec::new();
    let eligible = resolve_contracts(
        document,
        registry,
        calls,
        ids,
        origin,
        &mut report,
        &mut diagnostics,
    );
    let pass = NodePass {
        calls,
        document,
        index,
        registry,
        eligible: &eligible,
        ids,
        origin,
    };
    let fresh_args = infer_fresh_args(&pass, components, &mut report, &mut diagnostics);
    project_nodes(&pass, fresh_args, &mut report, &mut diagnostics);

    sort_diagnostics(&mut diagnostics);
    report.diagnostics = diagnostics;
    report
}

fn node_ids(document: &GraphDocument, scope: Scope) -> Vec<NodeId> {
    let mut ids: Vec<_> = scope
        .bodies(document)
        .into_iter()
        .flat_map(|(_, body)| body.nodes.keys().copied())
        .collect();
    ids.sort();
    ids
}

fn resolve_contracts(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    calls: &CallEvidence,
    ids: &[NodeId],
    origin: &HostOrigin,
    report: &mut NodeValidation,
    diagnostics: &mut Vec<Diagnostic>,
) -> BTreeSet<NodeId> {
    let mut eligible = BTreeSet::new();
    for id in ids {
        let Some(node) = document.node(*id) else {
            continue;
        };
        if preflight_node(document, node, registry, calls, origin, report, diagnostics) {
            eligible.insert(*id);
        }
    }
    eligible
}

fn preflight_node(
    document: &GraphDocument,
    node: &crate::document::NodeInstance,
    registry: &RegistrySnapshot,
    calls: &CallEvidence,
    origin: &HostOrigin,
    report: &mut NodeValidation,
    diagnostics: &mut Vec<Diagnostic>,
) -> bool {
    if node.definition.id.as_str() == ids::CALL {
        let code = match call_interface(document, node, registry) {
            Err(CallInterfaceError::Identity) => DiagnosticCode::IntrinsicUnsupported,
            Err(CallInterfaceError::TargetMissing) => DiagnosticCode::CallTargetMissing,
            Err(CallInterfaceError::TargetUnknown) => DiagnosticCode::CallTargetUnknown,
            Err(CallInterfaceError::DomainUnsupported) => DiagnosticCode::DomainUnsupported,
            Err(CallInterfaceError::InvalidInterface) => DiagnosticCode::FunctionInterface,
            Ok(interface) => {
                let accepted = interface
                    .and_then(|interface| calls.accepted.get(&interface.function).copied());
                if let Some((kind, suspends)) = accepted {
                    match suspension_contract_issue(kind, call_suspends(kind, suspends)) {
                        None => return true,
                        Some(code) => code,
                    }
                } else if calls.pipeline {
                    // The callee's own pass explains why it is not an accepted contract.
                    DiagnosticCode::DependencyUnavailable
                } else {
                    // A type-only interface cannot produce execution metadata alone.
                    DiagnosticCode::FunctionMetadataUnavailable
                }
            }
        };
        unresolved(node.id, code, origin, report, diagnostics);
        return false;
    }
    if is_function_marker(node.definition.id.as_str())
        && node.provider.is_none()
        && node.definition.version == 1
        && node.config_version == 1
    {
        return match marker_contract(document, node) {
            Ok(_) => true,
            Err(code) => {
                unresolved(node.id, code, origin, report, diagnostics);
                false
            }
        };
    }
    if is_converter_special_form(node.definition.id.as_str()) {
        if node
            .signature
            .as_ref()
            .is_none_or(|key| !document.signatures().contains_key(key))
        {
            unresolved(
                node.id,
                DiagnosticCode::SignatureMissing,
                origin,
                report,
                diagnostics,
            );
            return false;
        }
        match registry.converter_contract(document, node) {
            Ok(contract) => {
                if let Some(code) = suspension_contract_issue(contract.kind, contract.suspends) {
                    unresolved(node.id, code, origin, report, diagnostics);
                    return false;
                }
                return true;
            }
            Err(error) => {
                let code = contract_code(error);
                unresolved(node.id, code, origin, report, diagnostics);
                return false;
            }
        }
    }
    if is_intrinsic(node.definition.id.as_str()) {
        if node.provider.is_some() {
            unresolved(
                node.id,
                DiagnosticCode::ProviderUnavailable,
                origin,
                report,
                diagnostics,
            );
            return false;
        }
        if node.definition.version != 1 || node.config_version != 1 {
            unresolved(
                node.id,
                DiagnosticCode::ProviderUnavailable,
                origin,
                report,
                diagnostics,
            );
            return false;
        }
        let contract = match registry.contract(document, node) {
            Ok(Some(contract)) => contract,
            Ok(None) => {
                unresolved(
                    node.id,
                    DiagnosticCode::IntrinsicUnsupported,
                    origin,
                    report,
                    diagnostics,
                );
                return false;
            }
            Err(error) => {
                let code = contract_code(error);
                unresolved(node.id, code, origin, report, diagnostics);
                return false;
            }
        };
        if let Some(code) = suspension_contract_issue(contract.kind, contract.suspends) {
            unresolved(node.id, code, origin, report, diagnostics);
            return false;
        }
        check_activation(node, contract.kind, true, origin, diagnostics);
        return true;
    }

    let missing_signature = node
        .signature
        .as_ref()
        .is_none_or(|key| !document.signatures().contains_key(key));
    if missing_signature {
        unresolved(
            node.id,
            DiagnosticCode::SignatureMissing,
            origin,
            report,
            diagnostics,
        );
        return false;
    }
    let Some(binding) = &node.provider else {
        unresolved(
            node.id,
            DiagnosticCode::ProviderMissing,
            origin,
            report,
            diagnostics,
        );
        return false;
    };
    let Some(provider) = ProviderKey::of(document, binding) else {
        unresolved(
            node.id,
            DiagnosticCode::PackageLockMissing,
            origin,
            report,
            diagnostics,
        );
        return false;
    };
    let contract = match registry.contract(document, node) {
        Ok(Some(contract)) => contract,
        Ok(None) => {
            unresolved(
                node.id,
                DiagnosticCode::ProviderUnavailable,
                origin,
                report,
                diagnostics,
            );
            return false;
        }
        Err(error) => {
            unresolved(node.id, contract_code(error), origin, report, diagnostics);
            return false;
        }
    };
    if let Some(code) = suspension_contract_issue(contract.kind, contract.suspends) {
        unresolved(node.id, code, origin, report, diagnostics);
        return false;
    }
    if !registry.provides(&node.definition, node.config_version, &provider) {
        unresolved(
            node.id,
            DiagnosticCode::ProviderUnavailable,
            origin,
            report,
            diagnostics,
        );
        return false;
    }
    check_activation(node, contract.kind, false, origin, diagnostics);
    true
}

pub(crate) fn contract_code(error: ContractError) -> DiagnosticCode {
    match error {
        ContractError::VariableUnknown => DiagnosticCode::VariableUnknown,
        ContractError::EventUnknown => DiagnosticCode::EventUnknown,
        ContractError::VariableScope => DiagnosticCode::VariableScope,
        ContractError::SharedVariableScope => DiagnosticCode::SharedVariableScope,
        ContractError::UpdateOperation => DiagnosticCode::UpdateOperation,
        ContractError::UpdateUnsupported => DiagnosticCode::UpdateUnsupported,
        ContractError::UpdateConstraint => DiagnosticCode::ConstraintUnsatisfied,
        ContractError::TypeMismatch => DiagnosticCode::TypeMismatch,
        ContractError::TypeComplexity => DiagnosticCode::TypeComplexity,
        ContractError::ForEachMode => DiagnosticCode::ForEachMode,
        ContractError::ConverterConfig => DiagnosticCode::ConverterConfig,
        ContractError::ConverterEntry => DiagnosticCode::ConverterEntry,
        ContractError::ConverterOptions => DiagnosticCode::ConverterOptions,
        ContractError::ConverterProvider => DiagnosticCode::ProviderUnavailable,
        ContractError::ConverterPackageLockMissing => DiagnosticCode::PackageLockMissing,
    }
}

fn suspension_contract_issue(
    kind: crate::document::NodeKind,
    suspends: Option<Suspends>,
) -> Option<DiagnosticCode> {
    classify_suspension_contract(kind, suspends).map(|issue| match issue {
        SuspensionContractIssue::Missing => DiagnosticCode::SuspendsMissing,
        SuspensionContractIssue::Invalid(_) => DiagnosticCode::SuspendsInvalid,
    })
}

fn infer_fresh_args(
    pass: &NodePass<'_>,
    cache: Option<&mut ComponentCache>,
    report: &mut NodeValidation,
    diagnostics: &mut Vec<Diagnostic>,
) -> BTreeMap<NodeId, TypeArgs> {
    let components = match cache {
        Some(cache) => cache.infer(pass.document, pass.index, pass.ids, pass.registry),
        None => infer_components(
            pass.document,
            pass.index,
            pass.ids.iter().copied(),
            pass.registry,
        ),
    };
    let mut fresh_args = BTreeMap::new();
    for component in components {
        let incomplete = component.nodes.iter().any(|node| {
            !pass.eligible.contains(node)
                || touches_unresolved_data_neighbor(
                    pass.document,
                    pass.index,
                    pass.registry,
                    pass.eligible,
                    *node,
                )
        });
        if incomplete {
            // Ineligible members already carry their own diagnostic; the rest are blocked.
            for node in &component.nodes {
                report.incomplete_nodes.insert(*node);
                if pass.eligible.contains(node) {
                    diagnostics.push(Diagnostic::new(
                        DiagnosticCode::DependencyUnavailable,
                        Subject::new(Some(*node), None, None),
                        pass.origin,
                    ));
                }
            }
            continue;
        }
        report_component_issues(&component, pass.eligible, pass.origin, report, diagnostics);
        if component
            .issues
            .iter()
            .any(|issue| matches!(issue, InferenceIssue::Unification(_)))
        {
            report.incomplete_nodes.extend(
                component
                    .nodes
                    .iter()
                    .filter(|node| pass.eligible.contains(node))
                    .copied(),
            );
            continue;
        }
        collect_component_args(&component, pass, report, &mut fresh_args);
    }
    fresh_args
}

fn report_component_issues(
    component: &crate::inference::ComponentInference,
    eligible: &BTreeSet<NodeId>,
    origin: &HostOrigin,
    report: &mut NodeValidation,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for issue in &component.issues {
        let nodes = issue_node(issue)
            .filter(|node| component.nodes.contains(node))
            .map(|node| vec![node])
            .unwrap_or_else(|| component.nodes.iter().copied().collect());
        for node in nodes.into_iter().filter(|node| eligible.contains(node)) {
            diagnostics.push(Diagnostic::new(
                inference_code(issue),
                Subject::new(Some(node), None, None),
                origin,
            ));
            if !matches!(issue, InferenceIssue::NoSpecialization { .. }) {
                report.incomplete_nodes.insert(node);
            }
        }
    }
}

fn collect_component_args(
    component: &crate::inference::ComponentInference,
    pass: &NodePass<'_>,
    report: &mut NodeValidation,
    fresh_args: &mut BTreeMap<NodeId, TypeArgs>,
) {
    for id in &component.nodes {
        if !pass.eligible.contains(id) {
            continue;
        }
        let Some(node) = pass.document.node(*id) else {
            continue;
        };
        let Some(contract) = pass.contract(node) else {
            continue;
        };
        let Some(args) = component.type_args.get(id) else {
            continue;
        };
        let complete = args.len() == contract.type_params.len()
            && contract
                .type_params
                .iter()
                .all(|param| args.contains_key(&param.name));
        if !complete {
            report.incomplete_nodes.insert(*id);
            continue;
        }
        let invalid = component.issues.iter().any(|issue| {
            issue_node(issue) == Some(*id)
                && !matches!(issue, InferenceIssue::NoSpecialization { .. })
        });
        if !invalid {
            fresh_args.insert(*id, args.clone());
        }
    }
}

/// Port-level failures of one projection, in port order.
type Projection = Result<SignatureSnapshot, Vec<(PortId, DiagnosticCode)>>;

/// Projections of registry-borrowed contracts, keyed by definition, solved arguments and
/// body (whose rigid parameters scope type validation). Nodes of one definition with the same
/// arguments share an entry, and its semantic key once needed, instead of re-parsing every
/// port type and re-hashing the snapshot. Contextual contracts, built per node from config
/// or declarations, are projected per node.
type ProjectionCache =
    BTreeMap<(DefinitionRef, TypeArgs, Option<BodyRef>), (Projection, OnceCell<SignatureKey>)>;

fn project_nodes(
    pass: &NodePass<'_>,
    mut fresh_args: BTreeMap<NodeId, TypeArgs>,
    report: &mut NodeValidation,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut cache = ProjectionCache::new();
    // Semantic keys of stored snapshots, by document key: many nodes share one snapshot.
    let mut stored_keys = BTreeMap::new();
    for id in pass.ids {
        if !pass.eligible.contains(id) || report.incomplete_nodes.contains(id) {
            continue;
        }
        let Some(node) = pass.document.node(*id) else {
            continue;
        };
        let Some(contract) = pass.contract(node) else {
            continue;
        };
        let args = if contract.type_params.is_empty() {
            BTreeMap::new()
        } else if let Some(args) = fresh_args.remove(id) {
            args
        } else {
            report.incomplete_nodes.insert(*id);
            continue;
        };
        let body = pass.document.locate_node(*id);
        let stored = node
            .signature
            .as_ref()
            .and_then(|key| pass.document.signatures().get_key_value(key));
        let (projection, drifted) = match &contract {
            Cow::Borrowed(shared) => {
                let (projection, key) = cache
                    .entry((node.definition.clone(), args, body))
                    .or_insert_with_key(|(_, args, body)| {
                        (project_contract(pass, shared, args, *body), OnceCell::new())
                    });
                let drifted = projection.as_ref().is_ok_and(|projected| {
                    drifted(&mut stored_keys, stored, || {
                        key.get_or_init(|| signature::semantic_key(projected))
                            .clone()
                    })
                });
                (projection.clone(), drifted)
            }
            Cow::Owned(contextual) => {
                let projection = project_contract(pass, contextual, &args, body);
                let drifted = projection.as_ref().is_ok_and(|projected| {
                    drifted(&mut stored_keys, stored, || {
                        signature::semantic_key(projected)
                    })
                });
                (projection, drifted)
            }
        };
        match projection {
            Ok(projected) => {
                if drifted {
                    report_drift(pass, *id, report, diagnostics);
                }
                report.resolved.insert(*id, projected);
            }
            Err(failures) => {
                for (port, code) in failures {
                    diagnostics.push(port_type_diagnostic(*id, port, code, pass.origin));
                }
                report.incomplete_nodes.insert(*id);
            }
        }
    }
}

/// Substitutes `args` into `contract` and validates every resulting port type. The first
/// substitution failure stops; type validation reports every invalid port.
fn project_contract(
    pass: &NodePass<'_>,
    contract: &NodeContract,
    args: &TypeArgs,
    body: Option<BodyRef>,
) -> Projection {
    let mut ports = contract.ports.clone();
    for port in &mut ports {
        let Some(ty) = &port.ty else { continue };
        match signature::try_substitute_params(ty, args) {
            Ok(resolved) => port.ty = Some(resolved),
            Err(error) => {
                return Err(vec![(
                    port.port_id.clone(),
                    substitution_error_code(&error),
                )]);
            }
        }
    }
    let projected = signature::project_with_suspension(
        contract.kind,
        contract.suspends,
        ports,
        &BTreeMap::new(),
    );
    let rigid: Vec<String> = body_rigid_params(pass.document, body)
        .iter()
        .map(|param| param.name.clone())
        .collect();
    let failures = projected_type_failures(&projected, pass.registry.types(), &rigid);
    if failures.is_empty() {
        Ok(projected)
    } else {
        Err(failures)
    }
}

/// Whether the stored snapshot differs semantically from the projection, which is exactly
/// when [`signature::compare`] reports drift. Keys are computed only when a snapshot is stored.
fn drifted(
    stored_keys: &mut BTreeMap<SignatureKey, SignatureKey>,
    stored: Option<(&SignatureKey, &SignatureSnapshot)>,
    projected_key: impl FnOnce() -> SignatureKey,
) -> bool {
    let Some((key, snapshot)) = stored else {
        return false;
    };
    let stored = stored_keys
        .entry(key.clone())
        .or_insert_with(|| signature::semantic_key(snapshot));
    *stored != projected_key()
}

fn report_drift(
    pass: &NodePass<'_>,
    id: NodeId,
    report: &mut NodeValidation,
    diagnostics: &mut Vec<Diagnostic>,
) {
    diagnostics.push(Diagnostic::new(
        DiagnosticCode::SignatureDrift,
        Subject::new(Some(id), None, None),
        pass.origin,
    ));
    if pass
        .document
        .node(id)
        .is_some_and(|node| is_converter_special_form(node.definition.id.as_str()))
    {
        report.incomplete_nodes.insert(id);
    }
}

fn projected_type_failures(
    snapshot: &SignatureSnapshot,
    types: &TypeRegistry,
    rigid: &[String],
) -> Vec<(PortId, DiagnosticCode)> {
    snapshot
        .ports
        .iter()
        .filter_map(|port| {
            let ty = port.ty.as_ref()?;
            let code = match TypeTerm::parse(ty) {
                Ok(term) => types
                    .validate_scoped(&term, rigid)
                    .err()
                    .map(|error| match error {
                        TypeResolveError::UnresolvedParameter { .. } => {
                            DiagnosticCode::TypeMismatch
                        }
                        error => resolve_error_code(error),
                    }),
                Err(IdError::TypeTooDeep { .. }) => Some(DiagnosticCode::TypeComplexity),
                Err(_) => Some(DiagnosticCode::TypeMismatch),
            }?;
            Some((port.port_id.clone(), code))
        })
        .collect()
}

fn port_type_diagnostic(
    node: NodeId,
    port: crate::id::PortId,
    code: DiagnosticCode,
    origin: &HostOrigin,
) -> Diagnostic {
    Diagnostic::new(
        code,
        Subject::new(Some(node), Some(PortKey::new(port, None)), None),
        origin,
    )
}

fn resolve_error_code(error: TypeResolveError) -> DiagnosticCode {
    match error {
        TypeResolveError::TooDeep { .. } | TypeResolveError::DerivationTooDeep { .. } => {
            DiagnosticCode::TypeComplexity
        }
        _ => DiagnosticCode::TypeMismatch,
    }
}

fn substitution_error_code(error: &signature::SignatureSubstitutionError) -> DiagnosticCode {
    use signature::SignatureSubstitutionError as Error;
    match error {
        Error::TooDeep => DiagnosticCode::TypeComplexity,
        Error::InvalidSource { source, .. }
        | Error::InvalidReplacement { source, .. }
        | Error::InvalidResult { source, .. } => match source {
            IdError::TypeTooDeep { .. } => DiagnosticCode::TypeComplexity,
            _ => DiagnosticCode::TypeMismatch,
        },
    }
}

fn touches_unresolved_data_neighbor(
    document: &GraphDocument,
    index: &DocIndex,
    registry: &RegistrySnapshot,
    eligible: &BTreeSet<NodeId>,
    node: NodeId,
) -> bool {
    index.incident(node).any(|edge_id| {
        let Some(edge) = document.edge(edge_id) else {
            return false;
        };
        let (port, other_node) = if edge.from.node == node {
            (&edge.from, edge.to.node)
        } else {
            (&edge.to, edge.from.node)
        };
        if eligible.contains(&other_node) {
            return false;
        }
        // The generic endpoint determines whether type equations can depend on the
        // unavailable neighbour. Its own metadata is available; the opposite port may
        // be implicit or belong to an unavailable definition.
        port_kind(document, registry, port) == Some(PortKind::Data)
    })
}

fn port_kind(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    port: &PortRef,
) -> Option<PortKind> {
    let node = document.node(port.node)?;
    let contract = registry.contract(document, node).ok()?;
    let ports = match &contract {
        Some(contract) => contract.ports.as_slice(),
        None => document
            .signatures()
            .get(node.signature.as_ref()?)?
            .ports
            .as_slice(),
    };
    ports
        .iter()
        .find(|candidate| candidate.port_id == *port.port.port())
        .map(|candidate| candidate.kind)
}

/// Entry activation metadata (11 §10.1, 12 §6). Strict reading: an entry must persist a
/// policy (host-owned entries) and no other kind may; `capacity` is at least 1; `max_parallel` is present, at
/// least 1, exactly for `parallel`; host-owned entries have no backpressure protocol.
fn check_activation(
    node: &crate::document::NodeInstance,
    kind: NodeKind,
    host_owned: bool,
    origin: &HostOrigin,
    diagnostics: &mut Vec<Diagnostic>,
) {
    use crate::document::{OverflowPolicy, ReentrancyMode};
    let valid = match (&node.activation, kind == NodeKind::Entry) {
        (None, false) => true,
        (Some(_), false) => false,
        // Package entries keep their established tolerance for an absent policy.
        (None, true) => !host_owned,
        (Some(policy), true) => {
            let parallel = policy.mode == ReentrancyMode::Parallel;
            policy.capacity >= 1
                && match policy.max_parallel {
                    Some(max) => parallel && max >= 1,
                    None => !parallel,
                }
                && !(host_owned && policy.overflow == OverflowPolicy::Backpressure)
        }
    };
    if !valid {
        diagnostics.push(Diagnostic::new(
            DiagnosticCode::ActivationPolicy,
            Subject::new(Some(node.id), None, None),
            origin,
        ));
    }
}

fn unresolved(
    node: NodeId,
    code: DiagnosticCode,
    origin: &HostOrigin,
    report: &mut NodeValidation,
    diagnostics: &mut Vec<Diagnostic>,
) {
    report.incomplete_nodes.insert(node);
    diagnostics.push(Diagnostic::new(
        code,
        Subject::new(Some(node), None, None),
        origin,
    ));
}

fn issue_node(issue: &InferenceIssue) -> Option<NodeId> {
    match issue {
        InferenceIssue::InvalidTerm { node, .. }
        | InferenceIssue::Ambiguous { node, .. }
        | InferenceIssue::UnknownType { node, .. }
        | InferenceIssue::ConstraintUnsatisfied { node, .. }
        | InferenceIssue::NoSpecialization { node, .. } => Some(*node),
        InferenceIssue::Unification(_) => None,
    }
}

fn inference_code(issue: &InferenceIssue) -> DiagnosticCode {
    match issue.code() {
        "E-TYPE-AMBIGUOUS" => DiagnosticCode::TypeAmbiguous,
        "E-TYPE-COMPLEXITY" => DiagnosticCode::TypeComplexity,
        "E-CONSTRAINT-UNSATISFIED" => DiagnosticCode::ConstraintUnsatisfied,
        "E-NO-SPECIALIZATION" => DiagnosticCode::NoSpecialization,
        _ => DiagnosticCode::TypeMismatch,
    }
}

impl NodePass<'_> {
    /// The execution contract of an eligible node: registry metadata, an accepted call, or a
    /// function marker derived from its function's declared interface.
    fn contract(&self, node: &crate::document::NodeInstance) -> Option<Cow<'_, NodeContract>> {
        match node.definition.id.as_str() {
            ids::CALL => self.call_contract(node).map(Cow::Owned),
            id if is_function_marker(id) => {
                marker_contract(self.document, node).ok().map(Cow::Owned)
            }
            _ => self.registry.contract(self.document, node).ok().flatten(),
        }
    }

    /// Projects an accepted call: kind and suspension from the callee summary, ports from the
    /// declared interface with a source for each output (the call computes or executes it).
    fn call_contract(&self, node: &crate::document::NodeInstance) -> Option<NodeContract> {
        let interface = call_interface(self.document, node, self.registry)
            .ok()
            .flatten()?;
        let (kind, suspends) = *self.calls.accepted.get(&interface.function)?;
        let source = if matches!(kind, NodeKind::Pure | NodeKind::Volatile) {
            OutputSource::Computed
        } else {
            OutputSource::Execution
        };
        let mut ports = interface.ports;
        for port in &mut ports {
            if port.direction == crate::document::PortDirection::Output {
                port.source = Some(source);
            }
        }
        Some(NodeContract {
            kind,
            suspends: call_suspends(kind, suspends),
            type_params: interface.type_params,
            ports,
        })
    }
}

fn call_suspends(kind: NodeKind, suspends: Suspends) -> Option<Suspends> {
    matches!(kind, NodeKind::Async | NodeKind::Iterator).then_some(suspends)
}

fn marker_contract(
    document: &GraphDocument,
    node: &crate::document::NodeInstance,
) -> Result<NodeContract, DiagnosticCode> {
    crate::catalog::marker_contract(document, node).map_err(|error| match error {
        CallInterfaceError::InvalidInterface => DiagnosticCode::FunctionInterface,
        _ => DiagnosticCode::IntrinsicUnsupported,
    })
}

/// Each generic component's inference from an earlier analysis of the same session (16,
/// V12b). A component is solved from its nodes, the data edges incident to them and the
/// port types at their far ends, the registry and the document's declarations, so an
/// entry is dropped when the journal touches one of its nodes, boundary nodes or incident
/// edges, and the whole cache clears on any declaration or registry change. Nodes without
/// a valid entry are solved by the same inference.
#[derive(Default, Clone)]
pub(crate) struct ComponentCache {
    /// Keyed by the component's lowest node.
    components: BTreeMap<NodeId, CachedComponent>,
    of_node: BTreeMap<NodeId, NodeId>,
    pub(crate) computed: usize,
    pub(crate) hits: usize,
}

#[derive(Clone)]
struct CachedComponent {
    inference: crate::inference::ComponentInference,
    edges: BTreeSet<crate::id::EdgeId>,
    boundary: BTreeSet<NodeId>,
}

impl ComponentCache {
    pub(crate) fn clear(&mut self) {
        self.components.clear();
        self.of_node.clear();
    }

    pub(crate) fn len(&self) -> usize {
        self.components.len()
    }

    /// Drops every component that reads one of `nodes` or `edges`.
    pub(crate) fn forget(&mut self, nodes: &BTreeSet<NodeId>, edges: &BTreeSet<crate::id::EdgeId>) {
        if nodes.is_empty() && edges.is_empty() {
            return;
        }
        let stale: Vec<NodeId> = self
            .components
            .iter()
            .filter(|(_, cached)| {
                cached
                    .inference
                    .nodes
                    .iter()
                    .any(|node| nodes.contains(node))
                    || cached.boundary.iter().any(|node| nodes.contains(node))
                    || cached.edges.iter().any(|edge| edges.contains(edge))
            })
            .map(|(key, _)| *key)
            .collect();
        for key in stale {
            self.remove(key);
        }
    }

    fn remove(&mut self, key: NodeId) {
        if let Some(cached) = self.components.remove(&key) {
            for node in &cached.inference.nodes {
                self.of_node.remove(node);
            }
        }
    }

    /// The components of `ids`: valid entries as they are, the rest solved now.
    fn infer(
        &mut self,
        document: &GraphDocument,
        index: &DocIndex,
        ids: &[NodeId],
        registry: &RegistrySnapshot,
    ) -> Vec<crate::inference::ComponentInference> {
        let mut components = Vec::new();
        let mut known = BTreeSet::new();
        for id in ids {
            if known.contains(id) {
                continue;
            }
            if let Some(cached) = self
                .of_node
                .get(id)
                .and_then(|key| self.components.get(key))
            {
                self.hits += 1;
                known.extend(cached.inference.nodes.iter().copied());
                components.push(cached.inference.clone());
            }
        }
        let solved = crate::inference::component::infer_components_except(
            document,
            index,
            ids.iter().copied().filter(|id| !known.contains(id)),
            registry,
            &known,
        );
        for component in &solved {
            self.computed += 1;
            self.insert(document, index, component.clone());
        }
        components.extend(solved);
        components
    }

    fn insert(
        &mut self,
        document: &GraphDocument,
        index: &DocIndex,
        inference: crate::inference::ComponentInference,
    ) {
        let Some(&key) = inference.nodes.first() else {
            return;
        };
        let overlapping: BTreeSet<NodeId> = inference
            .nodes
            .iter()
            .filter_map(|node| self.of_node.get(node).copied())
            .collect();
        for stale in overlapping {
            self.remove(stale);
        }
        let mut edges = BTreeSet::new();
        let mut boundary = BTreeSet::new();
        for &node in &inference.nodes {
            let body = index.body_of(node).and_then(|body| document.body(body));
            for edge in index.incident(node) {
                edges.insert(edge);
                if let Some(edge) = body.and_then(|body| body.edges.get(&edge)) {
                    boundary.insert(edge.from.node);
                    boundary.insert(edge.to.node);
                }
            }
        }
        for node in &inference.nodes {
            self.of_node.insert(*node, key);
        }
        self.components.insert(
            key,
            CachedComponent {
                inference,
                edges,
                boundary,
            },
        );
    }
}
