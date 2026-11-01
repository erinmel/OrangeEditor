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

/// Above this many changed items, [`ValidationAnalysis::patch`] rebuilds the stretch of the
/// published list they span rather than inserting and removing items one by one, each of
/// which shifts the rest of the list.
const SPLICE_LIMIT: usize = 1;

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
    /// Moves a retained analysis to a new semantic state that a scoped session update
    /// proved differs only in some kept diagnostic lists. Each sorted `(old, new)` pair
    /// replaces one list; an old one stays when `base`, the other passes'
    /// sorted diagnostics, also reports it. Returns the subjects whose lists changed.
    pub(super) fn patch(
        &mut self,
        document_state: u64,
        base: &[Diagnostic],
        inputs: &[(Vec<Diagnostic>, Vec<Diagnostic>)],
    ) -> BTreeSet<Subject> {
        use super::diagnostic::diagnostic_order;
        self.document_state = document_state;
        let all = &mut self.report.diagnostics;
        let mut changed = BTreeSet::new();
        for (old, new) in inputs {
            // Both lists are sorted, so one merge finds what each lacks.
            let (mut gone, mut came) = (Vec::new(), Vec::new());
            let (mut i, mut j) = (0, 0);
            while i < old.len() || j < new.len() {
                let order = match (old.get(i), new.get(j)) {
                    (Some(a), Some(b)) => diagnostic_order(a, b),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    _ => std::cmp::Ordering::Greater,
                };
                match order {
                    std::cmp::Ordering::Less => {
                        gone.push(&old[i]);
                        i += 1;
                    }
                    std::cmp::Ordering::Greater => {
                        came.push(&new[j]);
                        j += 1;
                    }
                    std::cmp::Ordering::Equal => {
                        let item = &old[i];
                        while old.get(i) == Some(item) {
                            i += 1;
                        }
                        while new.get(j) == Some(item) {
                            j += 1;
                        }
                    }
                }
            }
            // An item another kept list also reports stays published.
            gone.retain(|item| {
                base.binary_search_by(|probe| diagnostic_order(probe, item))
                    .is_err()
            });
            changed.extend(gone.iter().chain(&came).map(|item| item.subject.clone()));
            if gone.len() + came.len() <= SPLICE_LIMIT {
                for item in gone {
                    if let Ok(at) = all.binary_search_by(|probe| diagnostic_order(probe, item)) {
                        all.remove(at);
                    }
                }
                for item in came {
                    if let Err(at) = all.binary_search_by(|probe| diagnostic_order(probe, item)) {
                        all.insert(at, item.clone());
                    }
                }
            } else {
                // Many changes: find each one's place by binary search and rebuild only
                // the stretch between the first and the last, so the rest of the list is
                // neither compared nor copied, only shifted. Both lists are sorted, so the
                // positions come out in order.
                let find =
                    |item: &Diagnostic| all.binary_search_by(|probe| diagnostic_order(probe, item));
                let drops: Vec<usize> = gone.iter().filter_map(|item| find(item).ok()).collect();
                let inserts: Vec<(usize, &Diagnostic)> = came
                    .iter()
                    .filter_map(|item| find(item).err().map(|at| (at, *item)))
                    .collect();
                let first = drops
                    .first()
                    .into_iter()
                    .chain(inserts.first().map(|(at, _)| at));
                let last = drops.last().map(|at| at + 1).into_iter();
                let last = last.chain(inserts.last().map(|(at, _)| *at)).max();
                let (Some(&lo), Some(hi)) = (first.min(), last) else {
                    continue;
                };
                let stretch: Vec<Diagnostic> = all.drain(lo..hi).collect();
                let mut rebuilt = Vec::with_capacity(stretch.len() + inserts.len());
                let mut drop = drops.into_iter().peekable();
                let mut insert = inserts.into_iter().peekable();
                for (at, item) in (lo..).zip(stretch) {
                    while let Some((_, next)) = insert.next_if(|(position, _)| *position == at) {
                        rebuilt.push(next.clone());
                    }
                    if drop.next_if(|position| *position == at).is_none() {
                        rebuilt.push(item);
                    }
                }
                rebuilt.extend(insert.map(|(_, item)| item.clone()));
                all.splice(lo..lo, rebuilt);
            }
        }
        changed
    }
    /// Replaces the region index after a scoped exec edit recomputed it.
    pub(super) fn set_regions(&mut self, regions: super::regions::RegionIndex) {
        self.report.regions = regions;
    }
    /// Replaces one function's summary after a scoped edit rechecked it.
    pub(super) fn set_summary(
        &mut self,
        function: crate::id::FunctionId,
        summary: Option<super::functions::FunctionSummary>,
    ) {
        match summary {
            Some(summary) => self.report.functions.insert(function, summary),
            None => self.report.functions.remove(&function),
        };
    }
    /// The resolved signatures, for a scoped session update that adds or removes nodes.
    pub(super) fn resolved_mut(
        &mut self,
    ) -> &mut BTreeMap<NodeId, crate::document::SignatureSnapshot> {
        &mut self.report.resolved
    }
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
    let admitted = match prerequisite_report(document, registry, &leaf_effects, &options) {
        Ok(admitted) => admitted,
        Err(report) => return *report,
    };
    run_supported_analysis(document, registry, &leaf_effects, &options, None, admitted)
        .map(|(analysis, _)| analysis)
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

/// A session analysis: the result, its local work, whether the index was rebuilt, and what
/// a later scoped update may build on.
pub(super) type Indexed = (
    ValidationAnalysis,
    super::local::LocalWork,
    bool,
    Option<super::scoped::Retained>,
);

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
) -> Indexed {
    let admitted = match prerequisite_report(document, registry, effects, &options) {
        Ok(admitted) => admitted,
        Err(report) => return (*report, Default::default(), false, None),
    };
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
    let (result, retained) = run_supported_analysis(
        document,
        registry,
        effects,
        &options,
        index.as_ref().map(|index| (index, &mut *local)),
        admitted,
    )
    .unwrap_or_else(|code| (rejected_analysis(document, registry, code), None));
    local.record_pass_work(&mut work);
    (result, work, rebuilt, retained)
}

/// Shared rejection boundary: neither route prepares retained state or copies effects
/// before current cancellation, admission quotas and domain support are checked.
fn prerequisite_report(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    leaf_effects: &BTreeMap<NodeId, LeafEffects>,
    options: &AnalysisOptions<'_>,
) -> Result<super::limits::Admitted, Box<ValidationAnalysis>> {
    let admitted = options
        .admit(document, registry, leaf_effects)
        .map_err(|code| Box::new(rejected_analysis(document, registry, code)))?;
    let domains = validate_body_domains(document, registry.origin());
    options
        .checkpoint()
        .map_err(|code| Box::new(rejected_analysis(document, registry, code)))?;
    if domains.unsupported_bodies.is_empty() {
        Ok(admitted)
    } else {
        Err(Box::new(domain_blocked_analysis(
            document,
            registry,
            domains.diagnostics,
        )))
    }
}

fn run_supported_analysis(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
    leaf_effects: &BTreeMap<NodeId, LeafEffects>,
    options: &AnalysisOptions<'_>,
    retained: Option<(&DocIndex, &mut super::local::LocalValidation)>,
    admitted: super::limits::Admitted,
) -> Result<(ValidationAnalysis, Option<super::scoped::Retained>), DiagnosticCode> {
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
            .map(super::local::LocalValidation::passes),
        options,
    )?;
    let Settled {
        passes:
            BodyPasses {
                projections,
                nodes,
                args,
                connections,
                data_cycles,
                regions,
                control,
                input_diagnostics,
            },
        calls,
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
            .map(super::local::LocalValidation::passes),
        options,
    )?;

    let ports = super::ports::PortResolver::resume(document, &nodes.resolved, projections);
    // A session also keeps the lazy cones the pass found free of requirements (V12c).
    let mut free = BTreeSet::new();
    let availability = if local.is_some() {
        super::availability::validate_availability_noting_free(
            document,
            &ports,
            &connections,
            &regions.index,
            registry,
            &mut free,
        )
    } else {
        super::availability::validate_availability_with_ports(
            document,
            &ports,
            &connections,
            &regions.index,
            registry,
        )
    };
    options.checkpoint()?;
    // Scoped updates may take the inference solution as kept only while no published type
    // can exceed the unifier's expansion budget (16, V12f "Kept solutions").
    let bounded = local.is_some() && super::scoped::types_bounded(nodes.resolved.values());
    let node_parts = local.is_some().then(|| {
        (
            super::scoped::by_node(&nodes.diagnostics),
            nodes.incomplete_nodes.clone(),
        )
    });
    // Node diagnostics stay apart too, so a scoped update can add or drop a node's.
    let node_diagnostics = nodes.diagnostics;
    let mut diagnostics = Vec::new();
    diagnostics.extend(super::inputs::validate_type_locks(document, registry));
    match &local {
        Some(local) => diagnostics.extend(local.declaration_diagnostics().cloned()),
        None => diagnostics.extend(validate_declaration_types(document, registry)),
    }
    options.checkpoint()?;
    // Control-flow results (availability, state, regions and control cycles) stay apart,
    // so a scoped exec edit can replace them together.
    let mut flow = availability.diagnostics;
    diagnostics.extend(super::exports::validate_exports_with_ports(
        document,
        &ports,
        registry,
        &regions.index.zones,
    ));
    options.checkpoint()?;
    // Kept apart, like the input diagnostics, so a scoped update can replace one node's.
    let config_diagnostics: Vec<Diagnostic> = match &local {
        Some(local) => local.config_diagnostics().cloned().collect(),
        None => validate_config(document, registry),
    };
    options.checkpoint()?;
    diagnostics.extend(connections.diagnostics);
    // Each function's own function-pass and conformance diagnostics, which a scoped edit
    // inside its body must leave unchanged.
    let function_diagnostics = local.is_some().then(|| {
        let mut groups = BTreeMap::<crate::id::FunctionId, Vec<Diagnostic>>::new();
        for item in functions.diagnostics.iter().chain(&conformance.diagnostics) {
            if let Some(function) = item.subject.function {
                groups.entry(function).or_default().push(item.clone());
            }
        }
        for list in groups.values_mut() {
            sort_diagnostics(list);
            list.dedup();
        }
        groups
    });
    diagnostics.extend(functions.diagnostics);
    diagnostics.extend(conformance.diagnostics);
    diagnostics.extend(super::streams::validate_streams_with_ports(
        document, &ports, registry,
    ));
    options.checkpoint()?;
    let state = validate_state(
        document,
        registry,
        &nodes.resolved,
        &regions.index,
        &functions.summaries,
        leaf_effects,
    );
    options.checkpoint()?;
    let explained_nodes: BTreeSet<_> = diagnostics
        .iter()
        .chain(&flow)
        .chain(&state)
        .chain(&node_diagnostics)
        .chain(&input_diagnostics)
        .chain(&config_diagnostics)
        .filter(|item| item.severity == Severity::Error)
        .filter_map(|item| item.subject.node)
        .collect();
    let unexplained = |item: &Diagnostic| {
        item.code != super::DiagnosticCode::DependencyUnavailable
            || item
                .subject
                .node
                .is_none_or(|node| !explained_nodes.contains(&node))
    };
    diagnostics.extend(data_cycles.diagnostics.into_iter().filter(unexplained));
    flow.extend(
        regions
            .diagnostics
            .into_iter()
            .chain(control.diagnostics)
            .filter(unexplained),
    );
    // The session keeps the input diagnostics apart from the rest, so a literal edit
    // can replace one node's inputs and merge again (16 §5.6).
    let mut retained = match (
        node_parts,
        super::scoped::by_node(&input_diagnostics),
        super::scoped::by_node(&config_diagnostics),
    ) {
        (Some((Some(node_diagnostics), node_incomplete)), Some(inputs), Some(config)) => {
            let mut base = diagnostics.clone();
            sort_diagnostics(&mut base);
            base.dedup();
            Some(super::scoped::Retained {
                admitted,
                calls: calls.clone(),
                node_diagnostics,
                node_incomplete,
                inputs,
                base,
                config,
                functions: function_diagnostics.clone().unwrap_or_default(),
                flow: sorted(flow.clone()),
                state: sorted(state.clone()),
                complete: false,
                clean: false,
                isolable: false,
                zones_checked: false,
                quiet: false,
                orders: BTreeMap::new(),
                args,
                bounded,
                free,
            })
        }
        _ => None,
    };
    diagnostics.extend(flow);
    diagnostics.extend(state);
    diagnostics.extend(node_diagnostics);
    diagnostics.extend(input_diagnostics);
    diagnostics.extend(config_diagnostics);
    sort_diagnostics(&mut diagnostics);
    diagnostics.dedup();

    let mut incomplete_nodes = nodes.incomplete_nodes;
    incomplete_nodes.extend(connections.incomplete_nodes);
    incomplete_nodes.extend(data_cycles.incomplete_nodes);
    incomplete_nodes.extend(regions.incomplete_nodes);
    incomplete_nodes.extend(control.incomplete_nodes);
    options.checkpoint()?;
    if let Some(retained) = &mut retained {
        use crate::document::Suspends;
        retained.complete =
            incomplete_nodes.is_empty() && functions.incomplete_functions.is_empty();
        // Every zone marker belongs to a checked pair, so each zone's body is closed (V12c).
        // A body with a zone fault publishes none and has an error anyway.
        let markers = document
            .bodies()
            .flat_map(|body| body.nodes.values())
            .filter(|node| super::zones::ZoneKind::marker(node.definition.id.as_str()).is_some())
            .count();
        retained.zones_checked = markers == 2 * regions.index.zones.iter().len();
        retained.clean = retained.complete
            && !super::scoped::blocking_error(retained, &BTreeSet::new())
            && retained.zones_checked;
        retained.isolable = functions.incomplete_functions.is_empty()
            && !super::scoped::blocking_error(retained, &incomplete_nodes)
            && retained.zones_checked;
        retained.quiet = nodes
            .resolved
            .values()
            .all(|signature| signature.suspension() == Some(Suspends::Never));
        // The root body's data order, which scoped structure edits there keep, built here
        // rather than by the first of them; function bodies build theirs when first edited.
        if retained.clean || retained.isolable {
            let order = super::order::TopoOrder::build(
                document,
                &ports,
                crate::BodyRef::Root,
                &regions.index.zones,
            );
            retained.orders.insert(crate::BodyRef::Root, order);
        }
    }
    drop(ports);
    let analysis = ValidationAnalysis {
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
    };
    Ok((analysis, retained))
}

fn sorted(mut diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    sort_diagnostics(&mut diagnostics);
    diagnostics.dedup();
    diagnostics
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
