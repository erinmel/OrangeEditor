//! The combined entry points composing the currently implemented passes.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::RegistrySnapshot;
use crate::document::{GraphDocument, PortRef, SignatureSnapshot};
use crate::id::{FunctionId, NodeId};

use super::{
    Diagnostic, FunctionEvidence, FunctionSummary, LeafEffects, Severity, analyze_functions,
    sort_diagnostics, validate_connections, validate_data_cycles, validate_inputs, validate_nodes,
};

/// A validation pass or contract category that must be covered before compilation can be ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ValidationCheck {
    NodeResolution,
    InputValues,
    Connections,
    FunctionSummaries,
    DefinitionAndConfig,
    BodyDomains,
    ContextualContracts,
    FunctionRestrictions,
    Regions,
    DataCycles,
    ControlCycles,
    ValueAvailability,
    StateRules,
    StreamRules,
    EntryPoliciesAndExports,
}

const REQUIRED_CHECKS: &[ValidationCheck] = &[
    ValidationCheck::NodeResolution,
    ValidationCheck::InputValues,
    ValidationCheck::Connections,
    ValidationCheck::FunctionSummaries,
    ValidationCheck::DefinitionAndConfig,
    ValidationCheck::BodyDomains,
    ValidationCheck::ContextualContracts,
    ValidationCheck::FunctionRestrictions,
    ValidationCheck::Regions,
    ValidationCheck::DataCycles,
    ValidationCheck::ControlCycles,
    ValidationCheck::ValueAvailability,
    ValidationCheck::StateRules,
    ValidationCheck::StreamRules,
    ValidationCheck::EntryPoliciesAndExports,
];

const COVERED_CHECKS: &[ValidationCheck] = &[
    ValidationCheck::NodeResolution,
    ValidationCheck::InputValues,
    ValidationCheck::Connections,
    ValidationCheck::FunctionSummaries,
    ValidationCheck::DataCycles,
];

/// Checks required by the current control-domain compile-precondition contract.
///
/// A different execution domain needs its own explicit validation contract; this list does not
/// certify that a foreign domain is supported.
pub fn required_checks() -> &'static [ValidationCheck] {
    REQUIRED_CHECKS
}

/// Combined result of the currently implemented passes over one document and registry snapshot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ValidationReport {
    /// All pass diagnostics, sorted by [`sort_diagnostics`]; identical records emitted by
    /// more than one pass appear once.
    pub diagnostics: Vec<Diagnostic>,
    /// Current semantic projections for nodes that resolved completely.
    pub resolved: BTreeMap<NodeId, SignatureSnapshot>,
    /// Nodes whose resolution or connection checks could not complete; each has a
    /// diagnostic that names it.
    pub incomplete_nodes: BTreeSet<NodeId>,
    /// Summaries of functions whose analysis completed.
    pub functions: BTreeMap<FunctionId, FunctionSummary>,
    /// Functions whose summary could not be established.
    pub incomplete_functions: BTreeSet<FunctionId>,
    /// Closed concrete-port witnesses for cyclic data-dependency components.
    pub data_cycle_paths: Vec<Vec<PortRef>>,
}

impl ValidationReport {
    /// True only when nothing is incomplete and no error was diagnosed in the passes this
    /// report ran. This is pass-local status, not complete validation or compile approval.
    pub fn is_valid(&self) -> bool {
        self.incomplete_nodes.is_empty()
            && self.incomplete_functions.is_empty()
            && self
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity != Severity::Error)
    }
}

/// Report plus private evidence of which checks ran and which inputs it analyzed.
///
/// Only [`analyze`] and [`analyze_with_effects`] can produce this value. Extracting or
/// inspecting its report cannot be used to manufacture fresh analysis evidence.
///
/// ```compile_fail
/// use orange_model::ValidationAnalysis;
/// use orange_model::ValidationReport;
/// let _ = ValidationAnalysis {
///     report: ValidationReport::default(),
///     document_state: 0,
///     registry_generation: 0,
/// };
/// ```
#[derive(Debug, Clone)]
pub struct ValidationAnalysis {
    report: ValidationReport,
    document_state: u64,
    registry_generation: u64,
}

impl ValidationAnalysis {
    /// Immutable pass-local report.
    pub fn report(&self) -> &ValidationReport {
        &self.report
    }

    /// Consume the evidence wrapper and retain its ordinary report only.
    pub fn into_report(self) -> ValidationReport {
        self.report
    }

    /// Checks actually executed by current combined validation.
    pub fn covered_checks(&self) -> impl Iterator<Item = ValidationCheck> + '_ {
        COVERED_CHECKS.iter().copied()
    }

    /// Required checks that combined validation has not implemented yet.
    pub fn missing_checks(&self) -> impl Iterator<Item = ValidationCheck> + '_ {
        REQUIRED_CHECKS
            .iter()
            .copied()
            .filter(|check| !COVERED_CHECKS.contains(check))
    }

    /// Whether this analysis belongs to the document content and registry snapshot supplied.
    pub fn is_current(&self, document: &GraphDocument, registry: &RegistrySnapshot) -> bool {
        self.document_state == document.state_id()
            && self.registry_generation == registry.generation()
    }

    /// Whether the current implementation has complete, successful compile-precondition checks.
    ///
    /// This remains false while any required category is absent from `covered_checks()`.
    pub fn is_ready_for_compilation(
        &self,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
    ) -> bool {
        self.is_current(document, registry)
            && self.report.is_valid()
            && self.missing_checks().next().is_none()
    }
}

/// Validates `document` against `registry` without supplying leaf effects.
///
/// A package leaf that is not pure or volatile has no known effects here, so its
/// function stays incomplete (`E-FUNCTION-METADATA-UNAVAILABLE`, 11 §14); missing leaf
/// facts are never treated as "no writes". Use [`validate_with_effects`] to supply them.
pub fn validate(document: &GraphDocument, registry: &RegistrySnapshot) -> ValidationReport {
    analyze(document, registry).into_report()
}

/// Runs the currently implemented combined validation passes and binds their evidence to inputs.
pub fn analyze(document: &GraphDocument, registry: &RegistrySnapshot) -> ValidationAnalysis {
    analyze_with_effects(document, registry, BTreeMap::new())
}

/// Validates `document` against `registry` with host-supplied leaf effects.
///
/// Runs node resolution, input, connection, function and data-cycle passes on the same resolution
/// and never changes the document. `leaf_effects` must be fresh for this exact document,
/// config, provider and lock snapshot; nodes without an entry make their function
/// incomplete (11 §14). Diagnostics are sorted and exact duplicates removed.
pub fn validate_with_effects(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    leaf_effects: BTreeMap<NodeId, LeafEffects>,
) -> ValidationReport {
    analyze_with_effects(document, registry, leaf_effects).into_report()
}

/// Runs combined validation with host-supplied leaf effects and retains private freshness evidence.
///
/// Freshness binds those effects to the caller's analysis invocation, but does not authenticate
/// their truth. The host remains responsible for supplying complete, exact effects for this
/// document, config, provider and lock snapshot.
pub fn analyze_with_effects(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    leaf_effects: BTreeMap<NodeId, LeafEffects>,
) -> ValidationAnalysis {
    let nodes = validate_nodes(document, registry);
    let connections = validate_connections(document, &nodes.resolved, registry);
    let data_cycles = validate_data_cycles(document, &nodes.resolved, &connections, registry);
    let evidence = FunctionEvidence::from_nodes(document, &nodes, leaf_effects);
    let functions = analyze_functions(document, &evidence, registry);

    let mut diagnostics = nodes.diagnostics;
    diagnostics.extend(validate_inputs(document, &nodes.resolved, registry));
    diagnostics.extend(connections.diagnostics);
    diagnostics.extend(functions.diagnostics);
    let explained_nodes: BTreeSet<_> = diagnostics
        .iter()
        .filter(|item| item.severity == Severity::Error)
        .filter_map(|item| item.subject.node)
        .collect();
    diagnostics.extend(data_cycles.diagnostics.into_iter().filter(|item| {
        item.code != super::DiagnosticCode::DependencyUnavailable
            || item
                .subject
                .node
                .is_none_or(|node| !explained_nodes.contains(&node))
    }));
    sort_diagnostics(&mut diagnostics);
    diagnostics.dedup();

    let mut incomplete_nodes = nodes.incomplete_nodes;
    incomplete_nodes.extend(connections.incomplete_nodes);
    incomplete_nodes.extend(data_cycles.incomplete_nodes);
    ValidationAnalysis {
        report: ValidationReport {
            diagnostics,
            resolved: nodes.resolved,
            incomplete_nodes,
            functions: functions.summaries,
            incomplete_functions: functions.incomplete_functions,
            data_cycle_paths: data_cycles.cycle_paths,
        },
        document_state: document.state_id(),
        registry_generation: registry.generation(),
    }
}
