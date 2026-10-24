//! Checked reads of fully folded pure exports. This does not execute a scheduler.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use orange_model::catalog::RegistrySnapshot;
use orange_model::{GraphDocument, Value};
use thiserror::Error;

use crate::{ConstantFault, Export, FoldedValue, PlanType, PurePlan, SlotSource};

/// Bounds the exported result set; cancellation is polled at entry and each read.
/// It does not interrupt trusted compilation callbacks or bound caller-owned copies.
#[derive(Debug, Clone, Copy)]
pub struct EvaluationOptions<'a> {
    pub max_exports: usize,
    pub cancel: Option<&'a AtomicBool>,
}
impl Default for EvaluationOptions<'_> {
    fn default() -> Self {
        Self {
            max_exports: 100_000,
            cancel: None,
        }
    }
}

/// Failure to start/read an evaluation. A graph's constant fault is instead an
/// [`EvaluatedExport::outcome`], with its original producer and error envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum EvaluationError {
    #[error("The plan belongs to another document state.")]
    StaleDocument,
    #[error("The plan belongs to another registry snapshot.")]
    StaleRegistry,
    #[error("The exported result set exceeds the host limit.")]
    ResourceLimit,
    #[error("Evaluation was cancelled.")]
    Cancelled,
}
impl EvaluationError {
    pub const fn message_key(self) -> &'static str {
        match self {
            Self::StaleDocument => "core/evaluate-stale-document",
            Self::StaleRegistry => "core/evaluate-stale-registry",
            Self::ResourceLimit => "core/evaluate-resource-limit",
            Self::Cancelled => "core/evaluate-cancelled",
        }
    }
}
fn checkpoint(cancel: Option<&AtomicBool>) -> Result<(), EvaluationError> {
    // This flag communicates cancellation only, not associated memory publication.
    if cancel.is_some_and(|signal| signal.load(Ordering::Relaxed)) {
        Err(EvaluationError::Cancelled)
    } else {
        Ok(())
    }
}

/// Borrowed typed result: source identifies the exported port, while a fault retains
/// its original failing producer. Values remain immutable and owned by the plan.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EvaluatedExport<'a> {
    pub id: &'a str,
    pub ty: PlanType,
    pub source: &'a SlotSource,
    pub outcome: Result<&'a Value, &'a ConstantFault>,
}

/// Checked frozen view of pure constants, retaining the plan's pinned registry.
/// Later document edits/catalogue publication do not mutate an already started view;
/// beginning a new view against changed inputs requires fresh compilation.
/// No kernels run and no frame, output buffer, or result values are allocated here.
///
/// ```compile_fail
/// let evaluation = orange_compiler::PureEvaluation::default();
/// ```
#[derive(Debug)]
#[must_use]
pub struct PureEvaluation<'a> {
    plan: &'a PurePlan,
    document_revision: u64,
    cancel: Option<&'a AtomicBool>,
}
impl PurePlan {
    /// Begin reading the fully folded pure-export result set after checking freshness.
    /// Pass the host's current registry at entry. An existing view keeps its old snapshot.
    pub fn evaluate<'a>(
        &'a self,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        options: EvaluationOptions<'a>,
    ) -> Result<PureEvaluation<'a>, EvaluationError> {
        checkpoint(options.cancel)?;
        if self.state != document.state_id() {
            return Err(EvaluationError::StaleDocument);
        }
        if self.snapshot.generation() != registry.generation() {
            return Err(EvaluationError::StaleRegistry);
        }
        if self.exports.len() > options.max_exports {
            return Err(EvaluationError::ResourceLimit);
        }
        checkpoint(options.cancel)?;
        Ok(PureEvaluation {
            plan: self,
            document_revision: document.revision(),
            cancel: options.cancel,
        })
    }
}
impl<'a> PureEvaluation<'a> {
    /// Compilation revision; it can differ from the entry revision after exact undo.
    pub fn source_revision(&self) -> u64 {
        self.plan.revision
    }
    pub fn document_revision(&self) -> u64 {
        self.document_revision
    }
    pub fn snapshot(&self) -> &Arc<RegistrySnapshot> {
        &self.plan.snapshot
    }
    pub fn len(&self) -> usize {
        self.plan.exports.len()
    }
    pub fn is_empty(&self) -> bool {
        self.plan.exports.is_empty()
    }
    /// Read in persisted export order. Each item polls cancellation; already returned
    /// results are immutable. This is a read stream, not an atomic batch publication.
    pub fn exports(
        &self,
    ) -> impl ExactSizeIterator<Item = Result<EvaluatedExport<'a>, EvaluationError>> + '_ {
        self.plan.exports.iter().map(|export| self.read(export))
    }
    /// Checked indexed read; an out-of-range index is `Ok(None)`.
    pub fn export(&self, index: usize) -> Result<Option<EvaluatedExport<'a>>, EvaluationError> {
        match self.plan.exports.get(index) {
            Some(export) => self.read(export).map(Some),
            None => {
                checkpoint(self.cancel)?;
                Ok(None)
            }
        }
    }
    fn read(&self, export: &'a Export) -> Result<EvaluatedExport<'a>, EvaluationError> {
        checkpoint(self.cancel)?;
        let slot = export.slot.index();
        // Slot construction and all plan arrays are private to checked compilation.
        let outcome = match &self.plan.constants[slot] {
            FoldedValue::Value(value) => Ok(value),
            FoldedValue::Fault(fault) => Err(fault),
        };
        Ok(EvaluatedExport {
            id: &export.id,
            ty: self.plan.types[slot],
            source: &self.plan.sources[slot],
            outcome,
        })
    }
}
