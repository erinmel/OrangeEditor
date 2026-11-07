//! Reference-only, bounded host command journals for an exact pinned plan.
use super::*;
use orange_compiler::ActionKernel;
use orange_model::id::EventDeclId;
use orange_model::value::shape::ValueShape;
mod playback;
mod record;

/// Host-owned archive identity. These numbers carry no authorization by themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayPayloadRef {
    pub namespace: u64,
    pub key: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ReplayArchiveError {
    #[error("The host policy excludes this payload from replay.")]
    Excluded,
    #[error("The replay payload reference is unavailable.")]
    Missing,
    #[error("The replay archive exceeds its configured bound.")]
    Limit,
}
/// Explicit host sensitivity/integrity/storage boundary. Methods must be bounded
/// and nonblocking. The archive owns its data and quotas outside Run retention.
/// `capture` must preserve exact values, or return Excluded; redacting a value
/// cannot preserve replay semantics. `resolve` must authenticate immutable refs.
pub trait ReplayArchive: Send + Sync + 'static {
    fn capture(&self, values: &[Value]) -> Result<ReplayPayloadRef, ReplayArchiveError>;
    fn resolve(&self, reference: ReplayPayloadRef) -> Result<Arc<[Value]>, ReplayArchiveError>;
}
#[derive(Debug, Clone, Copy)]
pub struct ReplayOptions {
    pub max_entries: usize,
    /// Fixed entry storage only; archive and pinned compiled plan are host owned.
    pub max_bytes: usize,
}
impl Default for ReplayOptions {
    fn default() -> Self {
        Self {
            max_entries: 8192,
            max_bytes: 2 * 1024 * 1024,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ReplayError {
    #[error("The replay journal is full; the operation was not started.")]
    Full,
    #[error("This plan or option is outside the supported replay profile.")]
    Unsupported,
    #[error("The recording is incomplete or was interrupted.")]
    Incomplete,
    #[error("Replay diverged from its recorded command or outcome.")]
    Diverged,
    #[error("{0}")]
    Archive(#[from] ReplayArchiveError),
    #[error("{0}")]
    Run(#[from] RunError),
    #[error("{0}")]
    Ingress(#[from] IngressError),
    #[error("{0}")]
    Effect(#[from] EffectError),
}
impl ReplayError {
    pub const fn message_key(self) -> &'static str {
        match self {
            Self::Full => "core/replay-full",
            Self::Unsupported => "core/replay-unsupported",
            Self::Incomplete => "core/replay-incomplete",
            Self::Diverged => "core/replay-diverged",
            Self::Archive(_) => "core/replay-payload-unavailable",
            Self::Run(e) => e.message_key(),
            Self::Ingress(e) => e.message_key(),
            Self::Effect(e) => e.message_key(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayStats {
    pub version: u32,
    pub entries: usize,
    pub reserved_bytes: usize,
    pub full_refusals: u64,
    pub incomplete: bool,
}
type SavedOutcome = Result<ReplayPayloadRef, KernelFault>;
#[derive(Clone, Copy)]
struct Source {
    id: HostSourceId,
    event: EventDeclId,
    foreign: bool,
}
impl Source {
    fn of(source: RunEventSource, run: u64) -> Self {
        Self {
            id: source.id,
            event: source.event,
            foreign: source.run != run,
        }
    }
    fn token(self, run: u64) -> RunEventSource {
        RunEventSource {
            run: if self.foreign { 0 } else { run },
            id: self.id,
            event: self.event,
        }
    }
}
#[derive(Clone, Copy)]
enum Entry {
    Pump {
        budget: usize,
        result: Option<Result<RunProgress, RunError>>,
    },
    Advance {
        now: u64,
        result: Option<Result<(), RunError>>,
    },
    Stop {
        mode: StopMode,
        result: Option<Result<(), RunError>>,
    },
    Register {
        event: EventDeclId,
        result: Option<Result<HostSourceId, IngressError>>,
    },
    Revoke {
        source: Source,
        result: Option<Result<bool, IngressError>>,
    },
    Ingest {
        source: Source,
        sequence: u64,
        payload: ReplayPayloadRef,
        wall: Option<i64>,
        result: Option<Result<EventMetadata, IngressError>>,
    },
    Trigger {
        node: NodeId,
        payload: Option<ReplayPayloadRef>,
        wall: Option<i64>,
        result: Option<Result<EventMetadata, IngressError>>,
    },
    Take {
        result: Option<Option<(u64, NodeId, u64)>>,
    },
    Complete {
        sequence: u64,
        foreign: bool,
        outcome: SavedOutcome,
        result: Option<Result<(), EffectError>>,
    },
    Action {
        node: NodeId,
        outcome: Option<SavedOutcome>,
    },
}
enum Mode {
    Record {
        entries: Vec<Entry>,
        max: usize,
        full: u64,
    },
    Replay {
        entries: Arc<[Entry]>,
        cursor: usize,
    },
}
pub(super) struct Session {
    mode: Mode,
    archive: Arc<dyn ReplayArchive>,
    failed: bool,
}
impl Session {
    fn begin(&mut self, entry: Entry) -> Result<usize, ReplayError> {
        if self.failed {
            return Err(ReplayError::Incomplete);
        }
        let Mode::Record { entries, max, full } = &mut self.mode else {
            return Err(ReplayError::Diverged);
        };
        if entries.len() == *max {
            *full = full.saturating_add(1);
            return Err(ReplayError::Full);
        }
        let index = entries.len();
        entries.push(entry);
        Ok(index)
    }
    fn set(&mut self, index: usize, entry: Entry) {
        if let Mode::Record { entries, .. } = &mut self.mode {
            entries[index] = entry;
        }
    }
    fn next(&mut self) -> Result<Option<Entry>, ReplayError> {
        if self.failed {
            return Err(ReplayError::Incomplete);
        }
        let Mode::Replay { entries, cursor } = &mut self.mode else {
            return Err(ReplayError::Diverged);
        };
        let entry = entries.get(*cursor).copied();
        *cursor += usize::from(entry.is_some());
        Ok(entry)
    }
    fn capture(&mut self, values: &[Value]) -> Result<ReplayPayloadRef, ReplayError> {
        self.archive.capture(values).map_err(|e| {
            self.failed = true;
            ReplayError::Archive(e)
        })
    }
    fn resolve(&self, reference: ReplayPayloadRef) -> Result<Arc<[Value]>, ReplayError> {
        Ok(self.archive.resolve(reference)?)
    }
    pub(super) fn action(
        &mut self,
        node: NodeId,
        action: &dyn ActionKernel,
        inputs: &[Value],
        outputs: &mut [Value],
        shapes: &[ValueShape],
        limit: usize,
    ) -> Result<Result<(), KernelFault>, RunError> {
        // Reserve before a callback can cause a side effect. Incomplete placeholders
        // also make a caught provider/archive panic ineligible for replay.
        let result = (|| {
            let outcome = match self.mode {
                Mode::Record { .. } => {
                    let slot = self.begin(Entry::Action {
                        node,
                        outcome: None,
                    })?;
                    let outcome = action.execute(inputs, outputs);
                    let saved = match outcome {
                        Ok(()) => {
                            check_outputs(shapes, outputs, limit)?;
                            Ok(self.capture(outputs)?)
                        }
                        Err(fault) if fault.is_valid() => Err(fault),
                        Err(_) => return Err(ReplayError::Diverged),
                    };
                    self.set(
                        slot,
                        Entry::Action {
                            node,
                            outcome: Some(saved),
                        },
                    );
                    return Ok(outcome);
                }
                Mode::Replay { .. } => match self.next()? {
                    Some(Entry::Action {
                        node: expected,
                        outcome: Some(outcome),
                    }) if expected == node => outcome,
                    _ => return Err(ReplayError::Diverged),
                },
            };
            match outcome {
                Err(fault) => Ok(Err(fault)),
                Ok(reference) => {
                    let values = self.resolve(reference)?;
                    check_outputs(shapes, &values, limit)?;
                    outputs.clone_from_slice(&values);
                    Ok(Ok(()))
                }
            }
        })();
        result.map_err(|_: ReplayError| {
            self.failed = true;
            RunError::ReplayFailed
        })
    }
    fn stats(&self) -> ReplayStats {
        let (entries, capacity, full) = match &self.mode {
            Mode::Record { entries, full, .. } => (entries.len(), entries.capacity(), *full),
            Mode::Replay { entries, .. } => (entries.len(), entries.len(), 0),
        };
        ReplayStats {
            version: 1,
            entries,
            reserved_bytes: capacity * std::mem::size_of::<Entry>(),
            full_refusals: full,
            incomplete: self.failed || self.incomplete_entries(),
        }
    }
    fn incomplete_entries(&self) -> bool {
        let entries = match &self.mode {
            Mode::Record { entries, .. } => entries.as_slice(),
            Mode::Replay { entries, .. } => entries,
        };
        entries.iter().any(|entry| match entry {
            Entry::Pump { result, .. } => result.is_none(),
            Entry::Advance { result, .. } | Entry::Stop { result, .. } => result.is_none(),
            Entry::Register { result, .. } => result.is_none(),
            Entry::Revoke { result, .. } => result.is_none(),
            Entry::Ingest { result, .. } | Entry::Trigger { result, .. } => result.is_none(),
            Entry::Take { result } => result.is_none(),
            Entry::Complete { result, .. } => result.is_none(),
            Entry::Action { outcome, .. } => outcome.is_none(),
        })
    }
}
fn check_outputs(shapes: &[ValueShape], values: &[Value], limit: usize) -> Result<(), ReplayError> {
    if shapes.len() != values.len() {
        return Err(ReplayError::Diverged);
    }
    for (shape, value) in shapes.iter().zip(values) {
        // Match the ordinary Action contract, including unprojected outputs.
        // Publication still admits the actual state/aggregate growth atomically.
        shape
            .checked_charge(value, limit)
            .map_err(|_| ReplayError::Diverged)?;
    }
    Ok(())
}
/// A recording owns its Run and exposes only immutable inspection. Mutations must
/// pass through the journal. No serialization or live resource recording is implied.
pub struct RecordedRun {
    run: Run,
    plan: Arc<ControlPlan>,
    seed: u64,
    options: RunOptions,
}
impl std::fmt::Debug for RecordedRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordedRun")
            .field("state", &self.run.state())
            .field("stats", &self.stats())
            .finish()
    }
}
/// Sealed command history for this exact immutable compiled plan/provider snapshot.
/// It owns references/metadata, never an archive or captured item values.
pub struct ReplayLog {
    plan: Arc<ControlPlan>,
    seed: u64,
    options: RunOptions,
    entries: Arc<[Entry]>,
    stats: ReplayStats,
}
impl std::fmt::Debug for ReplayLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplayLog")
            .field("stats", &self.stats)
            .field("graph", &self.plan.data().graph_id())
            .field("entropy", &crate::ENTROPY_ALGORITHM)
            .finish()
    }
}
/// Read-only replay result. It cannot expose pending requests to an executor or
/// continue the replay as a live run; dropping it performs ordinary run cleanup.
///
/// ```compile_fail
/// fn resend(replayed: &orange_runtime::ReplayedRun) {
///     replayed.inspect().take_effect();
/// }
/// ```
#[derive(Debug)]
pub struct ReplayedRun {
    run: Run,
}
/// Resumable replay; each `step` executes at most one saved host command, retaining
/// that command's original pump work budget. Inspection cannot issue effects.
#[derive(Debug)]
pub struct ReplayPlayback {
    run: Run,
    step_open: bool,
}
impl ReplayedRun {
    pub fn inspect(&self) -> &Run {
        &self.run
    }
}
impl RecordedRun {
    pub fn inspect(&self) -> &Run {
        &self.run
    }
    fn session(&mut self) -> Result<&mut Session, ReplayError> {
        self.run.replay.as_mut().ok_or(ReplayError::Incomplete)
    }
    pub fn stats(&self) -> ReplayStats {
        self.run
            .replay
            .as_ref()
            .map(Session::stats)
            .unwrap_or(ReplayStats {
                version: 1,
                entries: 0,
                reserved_bytes: 0,
                full_refusals: 0,
                incomplete: true,
            })
    }
    pub fn start(
        plan: Arc<ControlPlan>,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        seed: u64,
        options: RunOptions,
        limits: ReplayOptions,
        archive: Arc<dyn ReplayArchive>,
    ) -> Result<Self, ReplayError> {
        // Cancellation must have a recorded actor boundary; a live atomic token
        // cannot supply one. Resources/streams require an adapter-specific replay.
        if options.data.cancel.is_some()
            || plan
                .data()
                .slot_types()
                .iter()
                .any(|s| matches!(s, ValueShape::Handle(_) | ValueShape::Stream(_)))
            || plan.nodes().iter().any(|n| match n.operation() {
                ControlOperation::Effect { effect, .. } => effect
                    .outputs()
                    .iter()
                    .any(|s| matches!(s, ValueShape::Handle(_) | ValueShape::Stream(_))),
                ControlOperation::CloseHandle { .. }
                | ControlOperation::NextItem { .. }
                | ControlOperation::Collect { .. }
                | ControlOperation::ForEachItem { .. }
                | ControlOperation::CloseStream { .. }
                | ControlOperation::Tee { .. } => true,
                _ => false,
            })
        {
            return Err(ReplayError::Unsupported);
        }
        let bytes = limits
            .max_entries
            .checked_mul(std::mem::size_of::<Entry>())
            .ok_or(ReplayError::Full)?;
        if bytes > limits.max_bytes {
            return Err(ReplayError::Full);
        }
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(limits.max_entries)
            .map_err(|_| ReplayError::Full)?;
        if entries
            .capacity()
            .checked_mul(std::mem::size_of::<Entry>())
            .is_none_or(|n| n > limits.max_bytes)
        {
            return Err(ReplayError::Full);
        }
        let mut run = Run::start(plan.clone(), document, registry, seed, options.clone())?;
        run.replay = Some(Session {
            mode: Mode::Record {
                entries,
                max: limits.max_entries,
                full: 0,
            },
            archive,
            failed: false,
        });
        Ok(Self {
            run,
            plan,
            seed,
            options,
        })
    }
    /// Seal a complete prefix and return the live original Run separately. The
    /// journal does not claim that the original Run has reached Stopped.
    pub fn finish(mut self) -> Result<(Run, ReplayLog), ReplayError> {
        let session = self.run.replay.take().ok_or(ReplayError::Incomplete)?;
        let mut stats = session.stats();
        if self.run.pump_open || stats.incomplete {
            return Err(ReplayError::Incomplete);
        }
        let Mode::Record { entries, .. } = session.mode else {
            return Err(ReplayError::Incomplete);
        };
        let entries: Arc<[Entry]> = entries.into();
        stats.reserved_bytes = entries.len() * std::mem::size_of::<Entry>();
        Ok((
            self.run,
            ReplayLog {
                plan: self.plan,
                seed: self.seed,
                options: self.options,
                entries,
                stats,
            },
        ))
    }
}
