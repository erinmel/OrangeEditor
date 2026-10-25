//! Validation kept current beside one document across committed edits (V12a).
//!
//! An [`AnalysisSession`] owns the combined analysis of a document and applies each
//! committed [`ChangeSet`] to it. Its result always equals [`analyze_with_options`] for the
//! same document, registry, leaf effects and limits; that full pass is the oracle and the
//! fallback. This first slice recomputes the whole analysis for every semantic change.
//! Later slices recompute only the units a change dirties, without changing results.

use std::collections::BTreeMap;

use crate::catalog::RegistrySnapshot;
use crate::document::{GraphDocument, SignatureSnapshot};
use crate::edit::ChangeSet;
use crate::id::NodeId;

use super::{
    AnalysisLimits, AnalysisOptions, Diagnostic, DiagnosticCode, LeafEffects, ValidationAnalysis,
    analyze_with_options,
};

/// What one session update did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct EditStats {
    pub work: SessionWork,
    /// Nodes, across all bodies, whose results were recomputed.
    pub nodes_visited: usize,
    /// Edges, across all bodies, whose results were recomputed.
    pub edges_visited: usize,
}

/// How a session brought its analysis up to date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SessionWork {
    /// Nothing needed recomputing: a layout-only change only rebinds the result to the
    /// document's new state, and unchanged leaf effects keep the result as it is.
    None,
    /// The change was applied by recomputing affected results. V12a recomputes all of them.
    Recompute,
    /// The session discarded its results and reran the full analysis.
    Rebuild(RebuildReason),
}

/// Why a session reran the full analysis instead of applying a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RebuildReason {
    /// The session was just created.
    Created,
    /// The change does not start at the revision the session analyzed, or does not end at
    /// the document's current revision.
    StateMismatch,
    /// The registry is a different snapshot generation.
    RegistryChanged,
    /// The admission limits differ from the ones the session analyzed with.
    LimitsChanged,
    /// The host replaced the leaf effects.
    EffectsChanged,
    /// The previous analysis was cancelled, so its result does not describe the document.
    Stale,
}

/// Combined validation of one document, kept current across committed edits.
///
/// The host passes every committed change, in commit order: edits, undo and redo alike.
/// A change the session cannot apply (a skipped change, another registry generation, other
/// limits) makes it rerun the full analysis, never return a stale result.
#[derive(Debug, Clone)]
pub struct AnalysisSession {
    analysis: ValidationAnalysis,
    effects: BTreeMap<NodeId, LeafEffects>,
    limits: AnalysisLimits,
    revision: u64,
    stale: bool,
    stats: EditStats,
}

impl AnalysisSession {
    /// Runs the full analysis of `document`.
    pub fn new(
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        effects: BTreeMap<NodeId, LeafEffects>,
        options: AnalysisOptions<'_>,
    ) -> Self {
        let mut session = Self {
            analysis: analyze_with_options(document, registry, effects.clone(), options),
            effects,
            limits: options.limits,
            revision: document.revision(),
            stale: false,
            stats: full(document, SessionWork::Rebuild(RebuildReason::Created)),
        };
        session.stale = cancelled(&session.analysis);
        session
    }

    /// Brings the analysis up to date with `document` after the committed `change`.
    pub fn update(
        &mut self,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        change: &ChangeSet,
        options: AnalysisOptions<'_>,
    ) -> &EditStats {
        let rebuild = if self.stale {
            Some(RebuildReason::Stale)
        } else if registry.generation() != self.analysis.registry_generation() {
            Some(RebuildReason::RegistryChanged)
        } else if options.limits != self.limits {
            Some(RebuildReason::LimitsChanged)
        } else if change.from_revision != self.revision || change.to_revision != document.revision()
        {
            Some(RebuildReason::StateMismatch)
        } else {
            None
        };
        match rebuild {
            Some(reason) => self.rerun(document, registry, options, SessionWork::Rebuild(reason)),
            None if change.layout_only => {
                // Layout never affects validation (G-36), so only the bound state moves.
                // Layout preserves the semantic identity already bound to the analysis.
                self.revision = document.revision();
                self.stats = EditStats {
                    work: SessionWork::None,
                    nodes_visited: 0,
                    edges_visited: 0,
                };
            }
            None => self.rerun(document, registry, options, SessionWork::Recompute),
        }
        &self.stats
    }

    /// Replaces the host's leaf effects, rerunning the analysis if they changed.
    pub fn set_leaf_effects(
        &mut self,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        effects: BTreeMap<NodeId, LeafEffects>,
        options: AnalysisOptions<'_>,
    ) -> &EditStats {
        if effects == self.effects && !self.stale {
            self.stats = EditStats {
                work: SessionWork::None,
                nodes_visited: 0,
                edges_visited: 0,
            };
        } else {
            self.effects = effects;
            let reason = if self.stale {
                RebuildReason::Stale
            } else {
                RebuildReason::EffectsChanged
            };
            self.rerun(document, registry, options, SessionWork::Rebuild(reason));
        }
        &self.stats
    }

    fn rerun(
        &mut self,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        options: AnalysisOptions<'_>,
        work: SessionWork,
    ) {
        self.analysis = analyze_with_options(document, registry, self.effects.clone(), options);
        self.limits = options.limits;
        self.revision = document.revision();
        self.stale = cancelled(&self.analysis);
        self.stats = full(document, work);
    }

    /// The current analysis, bound to the document state and registry generation it
    /// describes, exactly as [`analyze_with_options`] would return it.
    pub fn analysis(&self) -> &ValidationAnalysis {
        &self.analysis
    }

    /// An owned copy of the current analysis, for consumers such as the compiler that need
    /// one. This copies the whole report; editors should read through the session instead.
    pub fn materialize(&self) -> ValidationAnalysis {
        self.analysis.clone()
    }

    /// The current diagnostics, in report order.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.analysis.report().diagnostics
    }

    /// The current semantic projection of `node`, if it resolved completely.
    pub fn resolved(&self, node: NodeId) -> Option<&SignatureSnapshot> {
        self.analysis.report().resolved.get(&node)
    }

    /// What the most recent creation, update or effects change did.
    pub fn stats(&self) -> &EditStats {
        &self.stats
    }

    /// True if the analysis describes `document` against `registry` and was not cancelled.
    pub fn is_current(&self, document: &GraphDocument, registry: &RegistrySnapshot) -> bool {
        !self.stale
            && self.analysis.document_state() == document.semantic_state_id()
            && self.analysis.registry_generation() == registry.generation()
    }
}

fn full(document: &GraphDocument, work: SessionWork) -> EditStats {
    let (nodes, edges) = document
        .bodies()
        .fold((0usize, 0usize), |(nodes, edges), body| {
            (nodes + body.nodes.len(), edges + body.edges.len())
        });
    EditStats {
        work,
        nodes_visited: nodes,
        edges_visited: edges,
    }
}

/// A cancelled analysis holds only the cancellation diagnostic; it describes no document.
fn cancelled(analysis: &ValidationAnalysis) -> bool {
    analysis
        .report()
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == DiagnosticCode::AnalysisCancelled)
}
