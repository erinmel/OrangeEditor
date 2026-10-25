//! Headless edit analysis with private freshness evidence and conservative fallbacks.
use super::local::{LocalValidation, LocalWork};
use super::{
    AnalysisLimits, AnalysisOptions, Diagnostic, LeafEffects, Subject, ValidationAnalysis,
};
use crate::catalog::RegistrySnapshot;
use crate::document::{GraphDocument, SignatureSnapshot};
use crate::edit::{ChangeSet, DocCommand};
use crate::error::TxError;
use crate::id::NodeId;
use crate::index::DocIndex;
use crate::inference::{InferencePlan, plan_edit_indexed};
use std::collections::{BTreeMap, BTreeSet};

/// Attributed update work. Input cardinalities are not total pass visits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct EditStats {
    pub work: SessionWork,
    /// Distinct input nodes when analysis ran; zero for a no-op.
    pub input_nodes: usize,
    /// Distinct input edges when analysis ran; zero for a no-op.
    pub input_edges: usize,
    /// Full orchestration attempts, including rejection. Global passes remain full.
    pub full_analysis_runs: usize,
    pub index_rebuilt: bool,
    /// Actual node-local validation computations and reused results.
    pub local: LocalWork,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SessionWork {
    None,
    /// Valid semantic edit; admission and global passes still run in full.
    Recompute,
    Rebuild(RebuildReason),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RebuildReason {
    Created,
    StateMismatch,
    RegistryChanged,
    LimitsChanged,
    EffectsChanged,
    Stale,
    Cancelled,
    GlobalChange,
}

/// Analysis beside one document. Supply every committed change, including undo/redo.
/// Missing/foreign receipts rebuild. Layout preserves semantic evidence; semantic
/// edits reuse proven local units within the same full-analysis pipeline.
#[derive(Clone)]
pub struct AnalysisSession {
    analysis: ValidationAnalysis,
    effects: BTreeMap<NodeId, LeafEffects>,
    limits: AnalysisLimits,
    revision: u64,
    content_state: u64,
    stale: bool,
    index: Option<DocIndex>,
    local: LocalValidation,
    subjects: BTreeMap<Subject, Vec<usize>>,
    changed_subjects: BTreeSet<Subject>,
    changed_nodes: BTreeSet<NodeId>,
    stats: EditStats,
}
impl std::fmt::Debug for AnalysisSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalysisSession")
            .field("semantic_state", &self.analysis.document_state())
            .field("registry_generation", &self.analysis.registry_generation())
            .field("diagnostics", &self.analysis.report().diagnostics.len())
            .field("stale", &self.stale)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}
impl AnalysisSession {
    pub fn new(
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        effects: BTreeMap<NodeId, LeafEffects>,
        options: AnalysisOptions<'_>,
    ) -> Self {
        let mut index = None;
        let mut local = LocalValidation::default();
        let (analysis, work, rebuilt) = super::report::analyze_indexed(
            document,
            registry,
            effects.clone(),
            options,
            &mut index,
            &mut local,
            None,
        );
        let subjects = subject_index(&analysis);
        Self {
            stale: incomplete(&analysis),
            changed_subjects: subjects.keys().cloned().collect(),
            changed_nodes: analysis.report().resolved.keys().copied().collect(),
            analysis,
            effects,
            limits: options.limits,
            revision: document.revision(),
            content_state: document.state_id(),
            index,
            local,
            subjects,
            stats: full(
                document,
                SessionWork::Rebuild(RebuildReason::Created),
                work,
                rebuilt,
            ),
        }
    }
    pub fn update(
        &mut self,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        change: &ChangeSet,
        options: AnalysisOptions<'_>,
    ) -> &EditStats {
        let reason = self
            .rebuild_reason(registry, options)
            .or_else(|| {
                (!change.is_committed()
                    || change.from_state() != self.content_state
                    || change.to_state() != document.state_id()
                    || change.from_semantic_state() != self.analysis.document_state()
                    || change.to_semantic_state() != document.semantic_state_id()
                    || change.from_revision() != self.revision
                    || change.to_revision() != document.revision())
                .then_some(RebuildReason::StateMismatch)
            })
            .or_else(|| change.global_dirty().then_some(RebuildReason::GlobalChange));
        if let Some(reason) = reason {
            self.rerun(
                document,
                registry,
                options,
                SessionWork::Rebuild(reason),
                None,
            );
        } else if change.layout_only() {
            if let Some(index) = &mut self.index {
                if index.state_id() == change.from_state() {
                    index.update(document, change);
                } else {
                    // A failed/foreign trial may invalidate only the index. Defer its
                    // rebuild instead of hiding a full scan in a layout no-op.
                    self.index = None;
                }
            }
            self.local.advance_layout(document, change);
            self.revision = document.revision();
            self.content_state = document.state_id();
            self.no_work();
        } else {
            self.rerun(
                document,
                registry,
                options,
                SessionWork::Recompute,
                Some(change),
            );
        }
        &self.stats
    }
    pub fn set_leaf_effects(
        &mut self,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        effects: BTreeMap<NodeId, LeafEffects>,
        options: AnalysisOptions<'_>,
    ) -> &EditStats {
        let reason = self
            .rebuild_reason(registry, options)
            .or_else(|| {
                (document.state_id() != self.content_state
                    || document.revision() != self.revision
                    || !self.analysis.is_current(document, registry))
                .then_some(RebuildReason::StateMismatch)
            })
            .or_else(|| (effects != self.effects).then_some(RebuildReason::EffectsChanged));
        if let Some(reason) = reason {
            self.effects = effects;
            self.rerun(
                document,
                registry,
                options,
                SessionWork::Rebuild(reason),
                None,
            );
        } else {
            self.no_work();
        }
        &self.stats
    }
    fn rebuild_reason(
        &self,
        registry: &RegistrySnapshot,
        options: AnalysisOptions<'_>,
    ) -> Option<RebuildReason> {
        if options.checkpoint().is_err() {
            Some(RebuildReason::Cancelled)
        } else if self.stale {
            Some(RebuildReason::Stale)
        } else if registry.generation() != self.analysis.registry_generation() {
            Some(RebuildReason::RegistryChanged)
        } else if options.limits != self.limits {
            Some(RebuildReason::LimitsChanged)
        } else {
            None
        }
    }
    fn rerun(
        &mut self,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        options: AnalysisOptions<'_>,
        work: SessionWork,
        change: Option<&ChangeSet>,
    ) {
        let (analysis, local, rebuilt) = super::report::analyze_indexed(
            document,
            registry,
            self.effects.clone(),
            options,
            &mut self.index,
            &mut self.local,
            change,
        );
        let subjects = subject_index(&analysis);
        self.changed_subjects.clear();
        for subject in self.subjects.keys().chain(subjects.keys()) {
            let old = self
                .subjects
                .get(subject)
                .into_iter()
                .flatten()
                .map(|&i| &self.analysis.report().diagnostics[i]);
            let new = subjects
                .get(subject)
                .into_iter()
                .flatten()
                .map(|&i| &analysis.report().diagnostics[i]);
            if !old.eq(new) {
                self.changed_subjects.insert(subject.clone());
            }
        }
        self.changed_nodes = self
            .analysis
            .report()
            .resolved
            .keys()
            .chain(analysis.report().resolved.keys())
            .filter(|node| {
                self.analysis.report().resolved.get(node) != analysis.report().resolved.get(node)
            })
            .copied()
            .collect();
        self.subjects = subjects;
        self.stale = incomplete(&analysis);
        self.analysis = analysis;
        self.limits = options.limits;
        self.revision = document.revision();
        self.content_state = document.state_id();
        self.stats = full(document, work, local, rebuilt);
    }
    fn no_work(&mut self) {
        self.changed_subjects.clear();
        self.changed_nodes.clear();
        self.stats = EditStats {
            work: SessionWork::None,
            input_nodes: 0,
            input_edges: 0,
            full_analysis_runs: 0,
            index_rebuilt: false,
            local: LocalWork::default(),
        };
    }
    /// Trial planning through the retained index. No analysis is published on failure.
    /// An unwinding planner invalidates its index; the next operation rebuilds it.
    pub fn plan_edit(
        &mut self,
        document: &mut GraphDocument,
        edit: impl IntoIterator<Item = DocCommand>,
        registry: &RegistrySnapshot,
    ) -> Result<InferencePlan, TxError> {
        let index = self.index.get_or_insert_with(|| DocIndex::build(document));
        plan_edit_indexed(document, index, edit, registry)
    }
    /// Absent if construction was rejected before indexing. Check freshness after host edits.
    pub fn index(&self) -> Option<&DocIndex> {
        self.index.as_ref()
    }
    pub fn analysis(&self) -> &ValidationAnalysis {
        &self.analysis
    }
    /// Copies the complete report; UI readers should use the borrowed views.
    pub fn materialize(&self) -> ValidationAnalysis {
        self.analysis.clone()
    }
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.analysis.report().diagnostics
    }
    pub fn diagnostics_for<'a>(
        &'a self,
        subject: &Subject,
    ) -> impl Iterator<Item = &'a Diagnostic> {
        self.subjects
            .get(subject)
            .into_iter()
            .flatten()
            .map(|&i| &self.analysis.report().diagnostics[i])
    }
    /// Subjects whose diagnostic lists changed, including removed diagnostics.
    pub fn changed_subjects(&self) -> &BTreeSet<Subject> {
        &self.changed_subjects
    }
    /// Resolved signatures that appeared, changed or disappeared.
    pub fn changed_nodes(&self) -> &BTreeSet<NodeId> {
        &self.changed_nodes
    }
    pub fn resolved(&self, node: NodeId) -> Option<&SignatureSnapshot> {
        self.analysis.report().resolved.get(&node)
    }
    pub fn stats(&self) -> &EditStats {
        &self.stats
    }
    pub fn is_current(&self, document: &GraphDocument, registry: &RegistrySnapshot) -> bool {
        !self.stale && self.analysis.is_current(document, registry)
    }
}
fn subject_index(analysis: &ValidationAnalysis) -> BTreeMap<Subject, Vec<usize>> {
    let mut subjects = BTreeMap::<Subject, Vec<usize>>::new();
    for (i, diagnostic) in analysis.report().diagnostics.iter().enumerate() {
        subjects
            .entry(diagnostic.subject.clone())
            .or_default()
            .push(i);
    }
    subjects
}
fn full(
    document: &GraphDocument,
    work: SessionWork,
    local: LocalWork,
    index_rebuilt: bool,
) -> EditStats {
    let (input_nodes, input_edges) = document.bodies().fold((0, 0), |(nodes, edges), body| {
        (nodes + body.nodes.len(), edges + body.edges.len())
    });
    EditStats {
        work,
        input_nodes,
        input_edges,
        full_analysis_runs: 1,
        index_rebuilt,
        local,
    }
}
fn incomplete(analysis: &ValidationAnalysis) -> bool {
    !analysis
        .covered_checks()
        .any(|check| check == super::ValidationCheck::NodeResolution)
}
