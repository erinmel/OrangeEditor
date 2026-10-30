//! The combined entry points composing the currently implemented passes.

use std::collections::{BTreeMap, BTreeSet};

use crate::catalog::RegistrySnapshot;
use crate::document::{GraphDocument, PortRef, SignatureSnapshot};
use crate::id::{FunctionId, NodeId};

use crate::index::DocIndex;

use super::acceptance::{BodyPasses, Settled, accept_functions, settle};
use super::{
    AnalysisOptions, Diagnostic, DiagnosticCode, FunctionSummary, LeafEffects, Severity, Subject,
    sort_diagnostics, validate_body_domains, validate_config, validate_declaration_types,
    validate_state,
};

/// A validation pass or contract category that must be covered before compilation can be ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ValidationCheck {
    DeclarationTypes,
    /// Compiler-owned folding and preservation of constant errors (`W-CONST-ERROR`).
    ConstantEvaluation,
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
    ValidationCheck::DeclarationTypes,
    ValidationCheck::ConstantEvaluation,
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
    ValidationCheck::DeclarationTypes,
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

const DOMAIN_ONLY_CHECKS: &[ValidationCheck] = &[ValidationCheck::BodyDomains];

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
    /// Region ownership and loop nesting for later availability and control-cycle passes.
    pub regions: super::RegionIndex,
    /// Closed node witnesses for control cycles that lack guaranteed suspension.
    pub control_cycle_paths: Vec<Vec<NodeId>>,
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
/// Only the combined `analyze*` entry points can produce this value. Extracting or
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
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationAnalysis {
    report: ValidationReport,
    document_state: u64,
    registry_generation: u64,
    covered: &'static [ValidationCheck],
}

impl ValidationAnalysis {
    /// Read-only semantic content identity, restored by undo/redo. Layout edits do not
    /// invalidate this analysis; compare with [`GraphDocument::semantic_state_id`].
    pub fn document_state(&self) -> u64 {
        self.document_state
    }

    /// Read-only identity of the analyzed immutable registry.
    pub fn registry_generation(&self) -> u64 {
        self.registry_generation
    }

    /// Whether semantic validation permits the compiler to begin lowering and folding.
    /// Constant evaluation remains required before a completed plan can be published.
    pub fn is_ready_for_lowering(
        &self,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
    ) -> bool {
        self.is_current(document, registry)
            && self.report.is_valid()
            && self
                .missing_checks()
                .all(|check| check == ValidationCheck::ConstantEvaluation)
    }

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
        self.covered.iter().copied()
    }

    /// Required checks not executed by this analysis invocation.
    pub fn missing_checks(&self) -> impl Iterator<Item = ValidationCheck> + '_ {
        REQUIRED_CHECKS
            .iter()
            .copied()
            .filter(|check| !self.covered.contains(check))
    }

    /// Whether this analysis belongs to the document content and registry snapshot supplied.
    pub fn is_current(&self, document: &GraphDocument, registry: &RegistrySnapshot) -> bool {
        self.document_state == document.semantic_state_id()
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
/// Checks body domains before running node resolution, input, connection, function and
/// data-cycle passes on the same resolution. Unsupported domains stop the control passes.
/// This never changes the document. `leaf_effects` must be fresh for this exact document,
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
/// Unsupported body domains return before the control passes; coverage records that only
/// the domain prerequisite ran, and no resolved signatures or summaries are produced.
pub fn analyze_with_effects(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    leaf_effects: BTreeMap<NodeId, LeafEffects>,
) -> ValidationAnalysis {
    analyze_with_options(document, registry, leaf_effects, AnalysisOptions::default())
}

/// Combined validation with configurable admission quotas and cooperative cancellation.
/// A rejected/cancelled request returns one safe error, no projections and no pass coverage.
/// Standalone pass APIs do not apply these orchestration controls.
pub fn analyze_with_options(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    leaf_effects: BTreeMap<NodeId, LeafEffects>,
    options: AnalysisOptions<'_>,
) -> ValidationAnalysis {
    if let Some(report) = prerequisite_report(document, registry, &leaf_effects, &options) {
        return report;
    }
    run_supported_analysis(document, registry, &leaf_effects, &options, None)
        .unwrap_or_else(|code| rejected_analysis(document, registry, code))
}

fn rejected_analysis(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    code: DiagnosticCode,
) -> ValidationAnalysis {
    ValidationAnalysis {
        report: ValidationReport {
            diagnostics: vec![Diagnostic::new(code, Subject::default(), registry.origin())],
            ..ValidationReport::default()
        },
        document_state: document.semantic_state_id(),
        registry_generation: registry.generation(),
        covered: &[],
    }
}

/// The same full pipeline, with a retained index and proven node-local results.
/// Admission still scans semantic metadata; downstream passes still run in full.
pub(super) fn analyze_indexed(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    effects: &BTreeMap<NodeId, LeafEffects>,
    options: AnalysisOptions<'_>,
    index: &mut Option<DocIndex>,
    local: &mut super::local::LocalValidation,
    change: Option<&crate::edit::ChangeSet>,
) -> (ValidationAnalysis, super::local::LocalWork, bool) {
    if let Some(report) = prerequisite_report(document, registry, effects, &options) {
        return (report, Default::default(), false);
    }
    let rebuilt = match index {
        Some(index) if index.is_current(document) => false,
        _ => {
            if let (Some(retained), Some(change)) = (&mut *index, change)
                && retained.state_id() == change.from_state()
            {
                retained.update(document, change);
                false
            } else {
                *index = Some(DocIndex::build(document));
                true
            }
        }
    };
    let mut work = local.refresh(document, registry, change);
    let result = run_supported_analysis(
        document,
        registry,
        effects,
        &options,
        index.as_ref().map(|index| (index, &mut *local)),
    )
    .unwrap_or_else(|code| rejected_analysis(document, registry, code));
    local.record_input_work(&mut work);
    (result, work, rebuilt)
}

/// Shared rejection boundary: neither route prepares retained state or copies effects
/// before current cancellation, admission quotas and domain support are checked.
fn prerequisite_report(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    leaf_effects: &BTreeMap<NodeId, LeafEffects>,
    options: &AnalysisOptions<'_>,
) -> Option<ValidationAnalysis> {
    if let Err(code) = options.admit(document, registry, leaf_effects) {
        return Some(rejected_analysis(document, registry, code));
    }
    let domains = validate_body_domains(document, registry.origin());
    if let Err(code) = options.checkpoint() {
        return Some(rejected_analysis(document, registry, code));
    }
    (!domains.unsupported_bodies.is_empty())
        .then(|| domain_blocked_analysis(document, registry, domains.diagnostics))
}

fn run_supported_analysis(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    leaf_effects: &BTreeMap<NodeId, LeafEffects>,
    options: &AnalysisOptions<'_>,
    retained: Option<(&DocIndex, &mut super::local::LocalValidation)>,
) -> Result<ValidationAnalysis, DiagnosticCode> {
    let built_index;
    let (index, mut local) = match retained {
        Some((index, local)) => (index, Some(local)),
        None => {
            built_index = DocIndex::build(document);
            (&built_index, None)
        }
    };
    let seed = accept_functions(
        document,
        registry,
        leaf_effects,
        index,
        local
            .as_deref_mut()
            .map(super::local::LocalValidation::inputs),
        options,
    )?;
    let Settled {
        passes:
            BodyPasses {
                projections,
                nodes,
                connections,
                data_cycles,
                regions,
                control,
                input_diagnostics,
            },
        functions,
        conformance,
        ..
    } = settle(
        document,
        registry,
        leaf_effects,
        index,
        seed,
        local
            .as_deref_mut()
            .map(super::local::LocalValidation::inputs),
        options,
    )?;

    let ports = super::ports::PortResolver::resume(document, &nodes.resolved, projections);
    let availability = super::availability::validate_availability_with_ports(
        document,
        &ports,
        &connections,
        &regions.index,
        registry,
    );
    options.checkpoint()?;
    let mut diagnostics = nodes.diagnostics;
    diagnostics.extend(super::inputs::validate_type_locks(document, registry));
    match &local {
        Some(local) => diagnostics.extend(local.declaration_diagnostics().cloned()),
        None => diagnostics.extend(validate_declaration_types(document, registry)),
    }
    options.checkpoint()?;
    diagnostics.extend(availability.diagnostics);
    diagnostics.extend(super::exports::validate_exports_with_ports(
        document,
        &ports,
        registry,
        &regions.index.zones,
    ));
    options.checkpoint()?;
    match &local {
        Some(local) => diagnostics.extend(local.config_diagnostics().cloned()),
        None => diagnostics.extend(validate_config(document, registry)),
    }
    options.checkpoint()?;
    diagnostics.extend(input_diagnostics);
    diagnostics.extend(connections.diagnostics);
    diagnostics.extend(functions.diagnostics);
    diagnostics.extend(conformance.diagnostics);
    diagnostics.extend(super::streams::validate_streams_with_ports(
        document, &ports, registry,
    ));
    options.checkpoint()?;
    diagnostics.extend(validate_state(
        document,
        registry,
        &nodes.resolved,
        &regions.index,
        &functions.summaries,
        leaf_effects,
    ));
    options.checkpoint()?;
    let explained_nodes: BTreeSet<_> = diagnostics
        .iter()
        .filter(|item| item.severity == Severity::Error)
        .filter_map(|item| item.subject.node)
        .collect();
    diagnostics.extend(
        data_cycles
            .diagnostics
            .into_iter()
            .chain(regions.diagnostics)
            .chain(control.diagnostics)
            .filter(|item| {
                item.code != super::DiagnosticCode::DependencyUnavailable
                    || item
                        .subject
                        .node
                        .is_none_or(|node| !explained_nodes.contains(&node))
            }),
    );
    sort_diagnostics(&mut diagnostics);
    diagnostics.dedup();

    let mut incomplete_nodes = nodes.incomplete_nodes;
    incomplete_nodes.extend(connections.incomplete_nodes);
    incomplete_nodes.extend(data_cycles.incomplete_nodes);
    incomplete_nodes.extend(regions.incomplete_nodes);
    incomplete_nodes.extend(control.incomplete_nodes);
    options.checkpoint()?;
    drop(ports);
    Ok(ValidationAnalysis {
        report: ValidationReport {
            diagnostics,
            resolved: nodes.resolved,
            incomplete_nodes,
            functions: functions.summaries,
            incomplete_functions: functions.incomplete_functions,
            data_cycle_paths: data_cycles.cycle_paths,
            regions: regions.index,
            control_cycle_paths: control.cycle_paths,
        },
        document_state: document.semantic_state_id(),
        registry_generation: registry.generation(),
        covered: COVERED_CHECKS,
    })
}

/// A foreign domain cannot supply evidence for the control-domain passes. Keep this
/// prerequisite failure separate from ordinary node/provider resolution failures.
fn domain_blocked_analysis(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    mut diagnostics: Vec<Diagnostic>,
) -> ValidationAnalysis {
    let incomplete_nodes: BTreeSet<_> = document
        .bodies()
        .flat_map(|body| body.nodes.keys().copied())
        .collect();
    let incomplete_functions: BTreeSet<_> = document.functions().keys().copied().collect();
    for (body_ref, body) in document.bodies_with_refs() {
        for node in body.nodes.keys() {
            let subject = match body_ref {
                crate::BodyRef::Root => super::Subject::new(Some(*node), None, None),
                crate::BodyRef::Function(function) => {
                    super::Subject::new(Some(*node), None, None).with_function(function)
                }
            };
            diagnostics.push(Diagnostic::new(
                super::DiagnosticCode::DomainAnalysisBlocked,
                subject,
                registry.origin(),
            ));
        }
    }
    for function in &incomplete_functions {
        diagnostics.push(Diagnostic::new(
            super::DiagnosticCode::DomainAnalysisBlocked,
            super::Subject::default().with_function(*function),
            registry.origin(),
        ));
    }
    sort_diagnostics(&mut diagnostics);
    ValidationAnalysis {
        report: ValidationReport {
            diagnostics,
            incomplete_nodes,
            incomplete_functions,
            ..ValidationReport::default()
        },
        document_state: document.semantic_state_id(),
        registry_generation: registry.generation(),
        covered: DOMAIN_ONLY_CHECKS,
    }
}

#[cfg(test)]
mod thread_contract {
    use super::*;

    #[test]
    fn private_projection_cache_does_not_change_public_thread_boundaries() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<GraphDocument>();
        send_sync::<RegistrySnapshot>();
        send_sync::<ValidationReport>();
        send_sync::<ValidationAnalysis>();
    }
}
