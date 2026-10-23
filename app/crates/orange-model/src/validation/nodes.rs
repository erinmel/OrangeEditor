//! Read-only node metadata, provider, signature, and generic resolution (WU-00d.2a).

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::{
    ContractError, NodeContract, ProviderKey, RegistrySnapshot, TypeArgs,
    is_converter_special_form, is_intrinsic,
};
use crate::document::{
    GraphDocument, PortKind, PortRef, SignatureSnapshot, Suspends, SuspensionContractIssue,
    suspension_contract_issue as classify_suspension_contract,
};
use crate::error::IdError;
use crate::id::{NodeId, PortKey};
use crate::index::DocIndex;
use crate::inference::{InferenceIssue, infer_components};
use crate::signature;
use crate::type_term::TypeTerm;
use crate::types::{TypeRegistry, TypeResolveError};

use super::diagnostic::{
    Diagnostic, DiagnosticCode, HostOrigin, Severity, Subject, sort_diagnostics,
};

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

struct NodePass<'a> {
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
    let origin = registry.origin();
    let mut report = NodeValidation::default();
    let mut diagnostics = Vec::new();
    let ids = node_ids(document);
    let eligible = resolve_contracts(
        document,
        registry,
        &ids,
        origin,
        &mut report,
        &mut diagnostics,
    );
    let index = DocIndex::build(document);
    let pass = NodePass {
        document,
        index: &index,
        registry,
        eligible: &eligible,
        ids: &ids,
        origin,
    };
    let fresh_args = infer_fresh_args(&pass, &mut report, &mut diagnostics);
    project_nodes(&pass, fresh_args, &mut report, &mut diagnostics);

    sort_diagnostics(&mut diagnostics);
    report.diagnostics = diagnostics;
    report
}

fn node_ids(document: &GraphDocument) -> Vec<NodeId> {
    let mut ids: Vec<_> = document
        .bodies()
        .flat_map(|body| body.nodes.keys().copied())
        .collect();
    ids.sort();
    ids
}

fn resolve_contracts(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
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
        if preflight_node(document, node, registry, origin, report, diagnostics) {
            eligible.insert(*id);
        }
    }
    eligible
}

fn preflight_node(
    document: &GraphDocument,
    node: &crate::document::NodeInstance,
    registry: &RegistrySnapshot,
    origin: &HostOrigin,
    report: &mut NodeValidation,
    diagnostics: &mut Vec<Diagnostic>,
) -> bool {
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
    true
}

pub(crate) fn contract_code(error: ContractError) -> DiagnosticCode {
    match error {
        ContractError::VariableUnknown => DiagnosticCode::VariableUnknown,
        ContractError::VariableScope => DiagnosticCode::VariableScope,
        ContractError::UpdateOperation => DiagnosticCode::UpdateOperation,
        ContractError::UpdateUnsupported => DiagnosticCode::UpdateUnsupported,
        ContractError::UpdateConstraint => DiagnosticCode::ConstraintUnsatisfied,
        ContractError::TypeMismatch => DiagnosticCode::TypeMismatch,
        ContractError::TypeComplexity => DiagnosticCode::TypeComplexity,
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
    report: &mut NodeValidation,
    diagnostics: &mut Vec<Diagnostic>,
) -> BTreeMap<NodeId, TypeArgs> {
    let components = infer_components(
        pass.document,
        pass.index,
        pass.ids.iter().copied(),
        pass.registry,
    );
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
        collect_component_args(
            &component,
            pass.document,
            pass.registry,
            pass.eligible,
            report,
            &mut fresh_args,
        );
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
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    eligible: &BTreeSet<NodeId>,
    report: &mut NodeValidation,
    fresh_args: &mut BTreeMap<NodeId, TypeArgs>,
) {
    for id in &component.nodes {
        if !eligible.contains(id) {
            continue;
        }
        let Some(node) = document.node(*id) else {
            continue;
        };
        let Ok(Some(contract)) = registry.contract(document, node) else {
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

fn project_nodes(
    pass: &NodePass<'_>,
    mut fresh_args: BTreeMap<NodeId, TypeArgs>,
    report: &mut NodeValidation,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for id in pass.ids {
        if !pass.eligible.contains(id) || report.incomplete_nodes.contains(id) {
            continue;
        }
        let Some(node) = pass.document.node(*id) else {
            continue;
        };
        let Ok(Some(contract)) = pass.registry.contract(pass.document, node) else {
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
        let stored = node
            .signature
            .as_ref()
            .and_then(|key| pass.document.signatures().get(key));
        if let Some(projected) =
            project_node(pass, *id, &contract, &args, stored, report, diagnostics)
        {
            report.resolved.insert(*id, projected);
        }
    }
}

fn project_node(
    pass: &NodePass<'_>,
    id: NodeId,
    contract: &NodeContract,
    args: &TypeArgs,
    stored: Option<&SignatureSnapshot>,
    report: &mut NodeValidation,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<SignatureSnapshot> {
    let mut ports = contract.ports.clone();
    for port in &mut ports {
        let Some(ty) = &port.ty else { continue };
        match signature::try_substitute_params(ty, args) {
            Ok(resolved) => port.ty = Some(resolved),
            Err(error) => {
                diagnostics.push(port_type_diagnostic(
                    id,
                    port.port_id.clone(),
                    substitution_error_code(&error),
                    pass.origin,
                ));
                report.incomplete_nodes.insert(id);
                return None;
            }
        }
    }
    let projected = signature::project_with_suspension(
        contract.kind,
        contract.suspends,
        ports,
        &BTreeMap::new(),
    );
    if !validate_projected_types(
        id,
        &projected,
        pass.registry.types(),
        pass.origin,
        report,
        diagnostics,
    ) {
        return None;
    }
    if stored.is_some_and(|stored| signature::compare(stored, &projected).is_some()) {
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
    Some(projected)
}

fn validate_projected_types(
    id: NodeId,
    snapshot: &SignatureSnapshot,
    types: &TypeRegistry,
    origin: &HostOrigin,
    report: &mut NodeValidation,
    diagnostics: &mut Vec<Diagnostic>,
) -> bool {
    let mut valid = true;
    for port in &snapshot.ports {
        let Some(ty) = &port.ty else { continue };
        let code = match TypeTerm::parse(ty) {
            Ok(term) if term.contains_param() => Some(DiagnosticCode::TypeMismatch),
            Ok(term) => types.validate(&term).err().map(resolve_error_code),
            Err(IdError::TypeTooDeep { .. }) => Some(DiagnosticCode::TypeComplexity),
            Err(_) => Some(DiagnosticCode::TypeMismatch),
        };
        if let Some(code) = code {
            diagnostics.push(port_type_diagnostic(id, port.port_id.clone(), code, origin));
            valid = false;
        }
    }
    if !valid {
        report.incomplete_nodes.insert(id);
    }
    valid
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
