//! One owner for a run and one bounded mailbox for everything that feeds it.
//!
//! A `Run` is driven through `&mut`, so sockets, hotkeys, timers and effect workers on
//! other threads need a single place where their input meets the run. That place
//! decides cross-source order and loss before the run's own guarantees begin, so it
//! is provided once here instead of by every embedder.
//!
//! The driver starts no thread and reads no clock. The host calls [`RunDriver::turn`]
//! from the thread that owns the run and supplies its own monotonic time.
use crate::run::{
    EffectError, EffectId, EventMetadata, HostSourceId, IngressError, Run, RunError,
    RunEventSource, RunProgress, RunState, SourceOptions, StopMode,
};
use orange_compiler::KernelFault;
use orange_model::Value;
use orange_model::id::{EventDeclId, NodeId};
use std::collections::BTreeMap;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};

/// One thing a host thread asks of the run. Applied in mailbox arrival order.
#[derive(Debug)]
pub enum HostInput {
    /// An event from a source registered through [`RunDriver::register_source`]. The
    /// driver assigns the source sequence, so senders need no shared counter.
    Event {
        source: RunEventSource,
        payload: Value,
        wall_ns: Option<i64>,
    },
    /// A trigger of a root `core/on-manual` entry.
    Trigger {
        node: NodeId,
        payload: Option<Value>,
        wall_ns: Option<i64>,
    },
    /// The result of an effect the host took from the run.
    EffectDone {
        id: EffectId,
        outcome: Result<Vec<Value>, KernelFault>,
    },
    Pause,
    Resume,
    Stop(StopMode),
}

/// A send that the mailbox did not take. The input is handed back unchanged.
#[derive(Debug)]
pub enum InboxError {
    /// The mailbox is at capacity; the sender may retry, drop or tell its peer.
    Full(HostInput),
    /// The driver is gone.
    Closed(HostInput),
}

/// Cloneable, thread-safe sending side of a run's mailbox. Sending never blocks.
#[derive(Debug, Clone)]
pub struct RunInbox(SyncSender<HostInput>);
impl RunInbox {
    pub fn send(&self, input: HostInput) -> Result<(), InboxError> {
        self.0.try_send(input).map_err(|error| match error {
            TrySendError::Full(input) => InboxError::Full(input),
            TrySendError::Disconnected(input) => InboxError::Closed(input),
        })
    }
}

/// Why the run did not take an input that reached it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusedReason {
    Ingress(IngressError),
    Effect(EffectError),
    Run(RunError),
}

/// An input the run refused, handed back so the host can retry it or answer its peer.
#[derive(Debug)]
pub struct Refused {
    pub input: HostInput,
    pub reason: RefusedReason,
}

/// What one [`RunDriver::turn`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriverTurn {
    /// Inputs the run took.
    pub applied: usize,
    /// Inputs handed back through the refusal callback.
    pub refused: usize,
    /// More inputs were waiting when the turn's input bound was reached.
    pub inbox_pending: bool,
    pub progress: RunProgress,
}

/// Owns a run and its mailbox. Each turn is bounded in both inputs and graph work, so
/// neither a busy source nor a busy graph can keep the other from running.
#[derive(Debug)]
pub struct RunDriver {
    run: Run,
    inbox: Receiver<HostInput>,
    sequences: BTreeMap<HostSourceId, u64>,
    last_host_ns: u64,
}
impl RunDriver {
    /// `capacity` bounds queued inputs across all senders. `host_ns` is the host's
    /// monotonic time now; later turns pass later readings of the same clock.
    pub fn new(run: Run, capacity: usize, host_ns: u64) -> (Self, RunInbox) {
        let (sender, inbox) = sync_channel(capacity);
        let driver = Self {
            run,
            inbox,
            sequences: BTreeMap::new(),
            last_host_ns: host_ns,
        };
        (driver, RunInbox(sender))
    }
    pub fn run(&self) -> &Run {
        &self.run
    }
    /// Direct access for the owning thread: taking effects, inspection, streams and
    /// resources. Do not ingest events for a driver-registered source here; the
    /// driver owns that source's sequence.
    pub fn run_mut(&mut self) -> &mut Run {
        &mut self.run
    }
    pub fn into_run(self) -> Run {
        self.run
    }
    /// Register a host source whose sequence this driver assigns.
    pub fn register_source(
        &mut self,
        event: EventDeclId,
        options: SourceOptions,
    ) -> Result<RunEventSource, IngressError> {
        let source = self.run.register_event_source_with(event, options)?;
        self.sequences.insert(source.id(), 0);
        Ok(source)
    }
    /// One bounded step: let host time pass, apply at most `max_inputs` queued inputs
    /// in arrival order, then pump at most `budget` work items.
    ///
    /// Host time that passes while the run is paused is not shown to it, so the run
    /// resumes where it stopped. A run that has already stopped takes no input; each
    /// one is handed back as refused.
    pub fn turn(
        &mut self,
        host_ns: u64,
        max_inputs: usize,
        budget: usize,
        refused: &mut dyn FnMut(Refused),
    ) -> Result<DriverTurn, RunError> {
        let elapsed = host_ns
            .checked_sub(self.last_host_ns)
            .ok_or(RunError::ClockWentBackwards)?;
        self.last_host_ns = host_ns;
        if clocked(self.run.state()) {
            self.run
                .advance_to(self.run.now().saturating_add(elapsed))?;
        }
        let (mut applied, mut refusals, mut inbox_pending) = (0, 0, false);
        for taken in 0.. {
            if taken == max_inputs {
                inbox_pending = true;
                break;
            }
            let input = match self.inbox.try_recv() {
                Ok(input) => input,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            };
            match self.apply(&input) {
                Ok(()) => applied += 1,
                Err(reason) => {
                    refusals += 1;
                    refused(Refused { input, reason });
                }
            }
        }
        let progress = if clocked(self.run.state()) || self.run.state() == RunState::Paused {
            self.run.pump(budget)?
        } else {
            RunProgress {
                operations: 0,
                ready: false,
                next_wakeup: None,
            }
        };
        Ok(DriverTurn {
            applied,
            refused: refusals,
            inbox_pending,
            progress,
        })
    }
    fn apply(&mut self, input: &HostInput) -> Result<(), RefusedReason> {
        match input {
            HostInput::Event {
                source,
                payload,
                wall_ns,
            } => {
                let next = self
                    .sequences
                    .get(&source.id())
                    .and_then(|last| last.checked_add(1))
                    .ok_or(RefusedReason::Ingress(IngressError::RevokedSource))?;
                let accepted: Result<EventMetadata, _> =
                    self.run.ingest_event(source, next, payload, *wall_ns);
                match accepted {
                    Ok(_) => {
                        self.sequences.insert(source.id(), next);
                        Ok(())
                    }
                    Err(error) => {
                        if error == IngressError::RevokedSource {
                            self.sequences.remove(&source.id());
                        }
                        Err(RefusedReason::Ingress(error))
                    }
                }
            }
            HostInput::Trigger {
                node,
                payload,
                wall_ns,
            } => self
                .run
                .trigger(*node, payload.as_ref(), *wall_ns)
                .map(drop)
                .map_err(RefusedReason::Ingress),
            HostInput::EffectDone { id, outcome } => self
                .run
                .complete_effect(*id, outcome.as_deref().map_err(Clone::clone))
                .map_err(RefusedReason::Effect),
            HostInput::Pause => self.run.pause().map_err(RefusedReason::Run),
            HostInput::Resume => self.run.resume().map_err(RefusedReason::Run),
            HostInput::Stop(mode) => self.run.request_stop(*mode).map_err(RefusedReason::Run),
        }
    }
}

/// States in which the run observes time and can be pumped for graph work.
fn clocked(state: RunState) -> bool {
    matches!(
        state,
        RunState::Running | RunState::Draining | RunState::Cancelling
    )
}
