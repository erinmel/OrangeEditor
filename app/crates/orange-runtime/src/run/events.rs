//! Run-scoped event fan-out, bounded reentrancy and FIFO delivery ownership.
//! No handler executes in Emit.
use super::*;
use orange_model::document::{OverflowPolicy, ReentrancyMode};
use orange_model::id::EventDeclId;
use orange_model::value::shape::ValueShape;
use std::collections::BTreeSet;
use ticks::{Tick, TickCursor};

const LIMIT: KernelFault = KernelFault {
    code: "core/LimitExceeded",
    message_key: "core/event-limit-exceeded",
    message: "The event exceeds a run delivery limit.",
};
const STOPPING: KernelFault = KernelFault {
    code: "core/RunStopping",
    message_key: "core/event-run-stopping",
    message: "The run is stopping and cannot accept an event.",
};
const DEPTH: KernelFault = KernelFault {
    code: "core/RunawayEventChain",
    message_key: "core/event-cause-depth",
    message: "The event causal depth limit was reached.",
};

/// Saturating handler and waiter delivery counters, with exact current occupancy.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RunEventStats {
    pub accepted: u64,
    pub delivered: u64,
    /// Evicted or deliberately skipped deliveries, including busy Drop and replaced latest.
    pub dropped: u64,
    /// Capacity overflow outcomes and limit refusals; a busy Drop is not an overflow.
    pub overflows: u64,
    pub pending: usize,
    pub pending_bytes: usize,
    pub live_payload_bytes: usize,
    pub activation_high_water: usize,
}
/// Identifies whether an accepted event came from a graph node, a registered host source,
/// the host manual-trigger call or a timer entry's schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventOrigin {
    Graph {
        node: NodeId,
        scope: usize,
    },
    Host {
        source: HostSourceId,
    },
    /// `Run::trigger`: no source registration; `source_seq` counts the entry's triggers.
    Manual,
    /// A timer entry's schedule; `source_seq` is the tick's ordinal, so a gap is missed ticks.
    Timer,
}

/// What an accepted delivery targets: a checked run-scope event declaration, consumed by
/// its On Event handlers and Await waiters, one root `core/on-manual` entry, or one root
/// `core/on-timer`/`core/on-timeout` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventTarget {
    Declared(EventDeclId),
    Manual(NodeId),
    Timer(NodeId),
}

/// Last successfully ingested event. Contains no payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventMetadata {
    pub ingestion_seq: u64,
    pub target: EventTarget,
    pub origin: EventOrigin,
    pub source_seq: u64,
    pub cause: Option<u64>,
    pub cause_depth: u16,
    pub monotonic_ns: u64,
    pub wall_ns: Option<i64>,
}

/// Run-local host-source generation. IDs are never reused during a run.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostSourceId(u64);
impl HostSourceId {
    pub const fn get(self) -> u64 {
        self.0
    }
}
impl std::fmt::Debug for HostSourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("HostSourceId").field(&self.0).finish()
    }
}

/// Opaque registration token bound to one run and one checked event.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RunEventSource {
    run: u64,
    id: HostSourceId,
    event: EventDeclId,
}
impl RunEventSource {
    pub const fn id(self) -> HostSourceId {
        self.id
    }
    pub const fn event(self) -> EventDeclId {
        self.event
    }
}
impl std::fmt::Debug for RunEventSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunEventSource")
            .field("run", &self.run)
            .field("id", &self.id)
            .field("event", &self.event)
            .finish()
    }
}

/// Bounded, payload-free refusal from the host ingress boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum IngressError {
    #[error("The event source belongs to another run.")]
    WrongRun,
    #[error("The run is stopping and cannot accept an event.")]
    NotRunning,
    #[error("The event is not present in the checked run plan.")]
    UnknownEvent,
    #[error("The run has reached its active host source limit.")]
    SourceLimit,
    #[error("The host source identity space is exhausted.")]
    IdentityExhausted,
    #[error("The host event source is revoked or unknown.")]
    RevokedSource,
    #[error("The source sequence must be nonzero and strictly increasing.")]
    SequenceNotIncreasing,
    #[error("The event payload does not match its checked type.")]
    InvalidPayload,
    #[error("The event exceeds a run delivery limit.")]
    DeliveryLimit,
    #[error("The node is not a manual entry in the checked run plan.")]
    UnknownEntry,
}
impl IngressError {
    pub const fn message_key(self) -> &'static str {
        match self {
            Self::WrongRun => "core/ingress-wrong-run",
            Self::NotRunning => "core/event-run-stopping",
            Self::UnknownEvent => "core/ingress-unknown-event",
            Self::SourceLimit => "core/ingress-source-limit",
            Self::IdentityExhausted => "core/ingress-identity-exhausted",
            Self::RevokedSource => "core/ingress-source-revoked",
            Self::SequenceNotIncreasing => "core/ingress-sequence-invalid",
            Self::InvalidPayload => "core/ingress-payload-invalid",
            Self::DeliveryLimit => "core/event-limit-exceeded",
            Self::UnknownEntry => "core/ingress-unknown-entry",
        }
    }
}

struct HostSource {
    event: usize,
    last_seq: u64,
}
pub(super) struct EventAdmission {
    pub event: usize,
    pub origin: EventOrigin,
    pub cause: Option<u64>,
    pub cause_depth: u16,
    pub source_seq: u64,
    pub wall_ns: Option<i64>,
    pub graph_source: Option<usize>,
    /// A timer tick's output values; its payload is `Unit`.
    pub tick: Option<Tick>,
}
struct Envelope {
    metadata: EventMetadata,
    payload: Value,
    tick: Option<Tick>,
    charge: usize,
}
struct Handler {
    entry: ControlIx,
    capacity: usize,
    overflow: OverflowPolicy,
    mode: ReentrancyMode,
    max_parallel: usize,
    pending: VecDeque<Envelope>,
    /// Root identities remain owned through descendant/cleanup retirement. The existing
    /// singleton set keeps serial handlers allocation-free and can identify a Replace
    /// target without scanning unrelated activations.
    active_roots: IdSet<u64>,
    /// A Replace request cancels its old root once. Further admitted latest envelopes
    /// must not cancel that root's cleanup again before full retirement.
    cancel_requested: Option<u64>,
}
pub(super) struct AwaitCompletion {
    pub(super) metadata: Option<EventMetadata>,
    pub(super) payload: Option<Value>,
    pub(super) charge: usize,
    pub(super) timed_out: bool,
}
pub(super) struct Awaiter {
    pub(super) token: u64,
    pub(super) event: usize,
    pub(super) owner: ControlIx,
    pub(super) strand: Strand,
    pub(super) timer: Option<TimerKey>,
    pub(super) completion: Option<AwaitCompletion>,
}
#[derive(Default)]
struct Awaiters {
    next: u64,
    by_event: Vec<BTreeSet<u64>>,
    by_token: BTreeMap<u64, Awaiter>,
    by_strand: super::ids::IdMap<StrandId, u64>,
    ready: VecDeque<u64>,
}
impl Awaiters {
    fn with_events(count: usize) -> Self {
        Self {
            by_event: vec![BTreeSet::new(); count],
            ..Self::default()
        }
    }
    fn token(&mut self) -> Result<u64, KernelFault> {
        let token = self.next;
        self.next = token.checked_add(1).ok_or(LIMIT)?;
        Ok(token)
    }
    fn insert(
        &mut self,
        token: u64,
        event: usize,
        owner: ControlIx,
        strand: Strand,
        timer: Option<TimerKey>,
    ) -> Result<(), RunError> {
        let index = self.by_event.get_mut(event).ok_or(RunError::InvalidState)?;
        if self.by_token.contains_key(&token) || self.by_strand.contains_key(&strand.id) {
            return Err(RunError::InvalidState);
        }
        index.insert(token);
        self.by_strand.insert(strand.id, token);
        self.by_token.insert(
            token,
            Awaiter {
                token,
                event,
                owner,
                strand,
                timer,
                completion: None,
            },
        );
        Ok(())
    }
    fn matching(&self, event: usize) -> impl Iterator<Item = u64> + '_ {
        self.by_event
            .get(event)
            .into_iter()
            .flat_map(|tokens| tokens.iter().copied())
    }
    fn accept(&mut self, token: u64, completion: AwaitCompletion) -> Result<(), RunError> {
        let waiter = self
            .by_token
            .get_mut(&token)
            .ok_or(RunError::InvalidState)?;
        if waiter.completion.is_some() {
            return Err(RunError::InvalidState);
        }
        self.by_event[waiter.event].remove(&token);
        waiter.timer = None;
        waiter.completion = Some(completion);
        self.ready.push_back(token);
        Ok(())
    }
    fn timeout(&mut self, token: u64) -> Result<bool, RunError> {
        let Some(waiter) = self.by_token.get_mut(&token) else {
            return Ok(false);
        };
        if waiter.completion.is_some() {
            return Ok(false);
        }
        self.by_event[waiter.event].remove(&token);
        waiter.timer = None;
        waiter.completion = Some(AwaitCompletion {
            metadata: None,
            payload: None,
            charge: 0,
            timed_out: true,
        });
        self.ready.push_back(token);
        Ok(true)
    }
    fn pop_ready(&mut self) -> Option<(u64, Awaiter)> {
        let token = self.ready.pop_front()?;
        let waiter = self.by_token.remove(&token)?;
        self.by_strand.remove(&waiter.strand.id);
        Some((token, waiter))
    }
    fn detach_strand(&mut self, strand: StrandId) -> Option<Awaiter> {
        let token = self.by_strand.remove(&strand)?;
        let waiter = self.by_token.remove(&token)?;
        self.by_event[waiter.event].remove(&token);
        if waiter.completion.is_some() {
            self.ready.retain(|pending| *pending != token);
        }
        Some(waiter)
    }
    fn len(&self) -> usize {
        self.by_event.iter().map(BTreeSet::len).sum()
    }
    fn has_ready(&self) -> bool {
        !self.ready.is_empty()
    }
    fn clear(&mut self) {
        self.by_token.clear();
        self.by_strand.clear();
        self.ready.clear();
        for index in &mut self.by_event {
            index.clear();
        }
    }
    fn release(&mut self) {
        let next = self.next;
        *self = Self {
            next,
            ..Self::default()
        };
    }
    fn discard_completions(&mut self, stats: &mut RunEventStats) {
        for token in self.ready.drain(..) {
            if let Some(waiter) = self.by_token.get_mut(&token)
                && let Some(completion) = waiter.completion.take()
                && completion.metadata.is_some()
            {
                stats.pending = stats.pending.saturating_sub(1);
                stats.pending_bytes = stats.pending_bytes.saturating_sub(completion.charge);
                stats.live_payload_bytes =
                    stats.live_payload_bytes.saturating_sub(completion.charge);
                stats.dropped = stats.dropped.saturating_add(1);
            }
        }
    }
}
impl Handler {
    fn has_slot(&self) -> bool {
        self.active_roots.len() < self.max_parallel
    }
    fn drops_busy(&self) -> bool {
        self.mode == ReentrancyMode::Drop
            && (!self.pending.is_empty() || !self.active_roots.is_empty())
    }
    fn replacement_target(&self) -> Option<u64> {
        if self.mode != ReentrancyMode::Replace {
            return None;
        }
        self.active_roots
            .iter()
            .next()
            .filter(|root| self.cancel_requested != Some(*root))
    }
}
/// Payload shape and target of one delivery channel: checked events, then manual entries,
/// then timer entries.
fn channel(plan: &ControlPlan, channel: usize) -> Option<(Option<&ValueShape>, EventTarget)> {
    if let Some(event) = plan.events().get(channel) {
        return Some((
            Some(event.payload_type()),
            EventTarget::Declared(event.id()),
        ));
    }
    let channel = channel - plan.events().len();
    if let Some(manual) = plan.manuals().get(channel) {
        return Some((manual.payload_type(), EventTarget::Manual(manual.node())));
    }
    let timer = plan.timers().get(channel - plan.manuals().len())?;
    Some((None, EventTarget::Timer(timer.node())))
}
pub(super) struct ActivationTemplate {
    pub prepared: Arc<PreparedData>,
    pub locals: Vec<Option<Value>>,
    pub states: Vec<Option<Value>>,
    pub local_bytes: usize,
    pub frame_storage: usize,
    pub template_storage: usize,
    pub source_revision: u64,
}
pub(super) struct Events {
    handlers: Vec<Handler>,
    /// Only handlers with an available root slot and pending deliveries are ready.
    ready: BTreeSet<(u64, usize)>,
    /// Reservation ownership is independent of a temporarily checked-out frame.
    active: BTreeMap<u64, (usize, usize)>,
    /// Successful Replace admissions, in emission/handler order. Each root is queued
    /// once until retirement, so this is bounded by active roots, not emission history.
    cancel_pending: VecDeque<u64>,
    /// Handler indices per channel: checked events first, then one channel per root manual
    /// entry at `events.len() + manual index`, then one per root timer entry.
    by_event: Vec<Vec<usize>>,
    source_sequences: Vec<u64>,
    /// Accepted trigger count per manual entry, used as its source sequence.
    manual_sequences: Vec<u64>,
    /// Schedule cursor per timer entry.
    ticks: Vec<TickCursor>,
    host_sources: BTreeMap<HostSourceId, HostSource>,
    next_host_source_id: Option<u64>,
    ingestion: u64,
    last: Option<EventMetadata>,
    stats: RunEventStats,
    static_starts: u64,
    static_roots: u64,
    live_stop_frames: usize,
    waiters: Awaiters,
}
impl Events {
    pub fn new(plan: &ControlPlan) -> Self {
        let mut handlers = Vec::new();
        let mut add = |handler: &orange_compiler::EventHandler| {
            handlers.push(Handler {
                entry: handler.entry(),
                capacity: handler.capacity(),
                overflow: handler.overflow(),
                mode: handler.mode(),
                max_parallel: handler.max_parallel(),
                pending: VecDeque::new(),
                active_roots: IdSet::default(),
                cancel_requested: None,
            });
            handlers.len() - 1
        };
        let mut by_event: Vec<Vec<usize>> = plan
            .events()
            .iter()
            .map(|metadata| metadata.handlers().iter().map(&mut add).collect())
            .collect();
        by_event.extend(
            plan.manuals()
                .iter()
                .map(|manual| vec![add(manual.handler())]),
        );
        by_event.extend(plan.timers().iter().map(|timer| vec![add(timer.handler())]));
        Self {
            handlers,
            ready: BTreeSet::new(),
            active: BTreeMap::new(),
            cancel_pending: VecDeque::new(),
            by_event,
            source_sequences: if plan.events().is_empty() {
                Vec::new()
            } else {
                vec![0; plan.nodes().len()]
            },
            manual_sequences: vec![0; plan.manuals().len()],
            ticks: vec![TickCursor::default(); plan.timers().len()],
            host_sources: BTreeMap::new(),
            next_host_source_id: Some(1),
            ingestion: 0,
            last: None,
            stats: RunEventStats::default(),
            static_starts: plan.starts().len() as u64,
            static_roots: (plan.starts().len() + plan.stops().len()) as u64,
            live_stop_frames: plan.stops().len(),
            waiters: Awaiters::with_events(plan.events().len()),
        }
    }
    fn remove_ready(&mut self, handler: usize) {
        if let Some(front) = self.handlers[handler].pending.front() {
            self.ready.remove(&(front.metadata.ingestion_seq, handler));
        }
    }
    fn make_ready(&mut self, handler: usize) {
        let queue = &self.handlers[handler];
        if queue.has_slot()
            && let Some(front) = queue.pending.front()
        {
            self.ready.insert((front.metadata.ingestion_seq, handler));
        }
    }
    fn finish(&mut self, activation: u64) -> Result<(), RunError> {
        if let Some((handler, charge)) = self.active.remove(&activation) {
            if !self.handlers[handler].active_roots.remove(&activation) {
                return Err(RunError::InvalidState);
            }
            if self.handlers[handler].cancel_requested == Some(activation) {
                self.handlers[handler].cancel_requested = None;
            }
            self.stats.live_payload_bytes = self
                .stats
                .live_payload_bytes
                .checked_sub(charge)
                .ok_or(RunError::InvalidState)?;
            self.make_ready(handler);
        }
        Ok(())
    }
    pub fn pending(&self) -> usize {
        self.stats.pending
    }
    pub(super) fn tick_cursor(&self, timer: usize) -> Result<TickCursor, RunError> {
        self.ticks.get(timer).copied().ok_or(RunError::InvalidState)
    }
    pub(super) fn set_tick_admitted(&mut self, timer: usize, ordinal: u64) -> Result<(), RunError> {
        self.ticks
            .get_mut(timer)
            .ok_or(RunError::InvalidState)?
            .admitted = ordinal;
        Ok(())
    }
    pub(super) fn set_tick_next(&mut self, timer: usize, ordinal: u64) -> Result<(), RunError> {
        self.ticks
            .get_mut(timer)
            .ok_or(RunError::InvalidState)?
            .next = ordinal;
        Ok(())
    }
    pub(super) fn count_overflow(&mut self) {
        self.stats.overflows = self.stats.overflows.saturating_add(1);
    }
    pub fn clear(&mut self) {
        // Await completions share aggregate pending counters with handler deliveries.
        // Refund them first so the handler-only queue reset below runs exactly once.
        self.waiters.discard_completions(&mut self.stats);
        self.stats.dropped = self.stats.dropped.saturating_add(self.stats.pending as u64);
        self.stats.live_payload_bytes = self
            .stats
            .live_payload_bytes
            .saturating_sub(self.stats.pending_bytes);
        self.stats.pending = 0;
        self.stats.pending_bytes = 0;
        self.ready.clear();
        // Whole-run cancellation will visit every root itself. Do not cancel cleanup
        // a second time by replaying a targeted Replace request afterward.
        self.cancel_pending.clear();
        self.host_sources.clear();
        for handler in &mut self.handlers {
            handler.pending.clear();
        }
    }
    pub fn abort(&mut self) {
        self.clear();
        self.waiters.clear();
        self.active.clear();
        self.stats.live_payload_bytes = 0;
        for handler in &mut self.handlers {
            handler.active_roots.clear();
            handler.cancel_requested = None;
        }
    }
    pub fn release(&mut self, high_water: usize) {
        self.abort();
        self.stats.activation_high_water = self.stats.activation_high_water.max(high_water);
        self.handlers = Vec::new();
        self.ready = BTreeSet::new();
        self.by_event = Vec::new();
        self.waiters.release();
        self.source_sequences = Vec::new();
        self.manual_sequences = Vec::new();
        self.ticks = Vec::new();
        self.host_sources = BTreeMap::new();
        self.cancel_pending = VecDeque::new();
        self.live_stop_frames = 0;
    }
}
struct Disposition {
    handler: usize,
    remove: usize,
    skip: bool,
    overflow: bool,
    charge: usize,
    cancel: Option<u64>,
}
impl Run {
    pub(super) fn host_source_count(&self) -> usize {
        self.events.host_sources.len()
    }
    pub(super) fn has_replace_cancellation(&self) -> bool {
        !self.events.cancel_pending.is_empty()
    }
    pub(super) fn close_host_sources(&mut self) {
        self.events.host_sources.clear();
    }
    pub fn event_stats(&self) -> RunEventStats {
        RunEventStats {
            activation_high_water: self
                .activations
                .high_water()
                .max(self.events.stats.activation_high_water),
            ..self.events.stats
        }
    }
    pub fn last_event(&self) -> Option<EventMetadata> {
        self.events.last
    }
    /// Register one bounded host source for an event in this run's checked plan.
    pub fn register_event_source(
        &mut self,
        event: EventDeclId,
    ) -> Result<RunEventSource, IngressError> {
        // An unwound pump leaves `Interrupted` for pump/advance_to to report once;
        // a refused host call must not consume or otherwise mutate that state.
        if self.pump_open || self.state != RunState::Running {
            return Err(IngressError::NotRunning);
        }
        let plan = self.plan.as_ref().ok_or(IngressError::UnknownEvent)?;
        let event_index = plan
            .events()
            .iter()
            .position(|checked| checked.id() == event)
            .ok_or(IngressError::UnknownEvent)?;
        if self.events.host_sources.len() >= self.options.limits.max_host_sources {
            return Err(IngressError::SourceLimit);
        }
        let id = HostSourceId(
            self.events
                .next_host_source_id
                .ok_or(IngressError::IdentityExhausted)?,
        );
        self.events.next_host_source_id = id.0.checked_add(1);
        self.events.host_sources.insert(
            id,
            HostSource {
                event: event_index,
                last_seq: 0,
            },
        );
        Ok(RunEventSource {
            run: self.id,
            id,
            event,
        })
    }

    /// Revoke a source generation. Already accepted deliveries keep their ownership.
    pub fn revoke_event_source(&mut self, source: &RunEventSource) -> Result<bool, IngressError> {
        if source.run != self.id {
            return Err(IngressError::WrongRun);
        }
        Ok(self.events.host_sources.remove(&source.id).is_some())
    }

    /// Admit one host event synchronously; graph work remains deferred to `pump`.
    pub fn ingest_event(
        &mut self,
        source: &RunEventSource,
        source_seq: u64,
        payload: &Value,
        wall_ns: Option<i64>,
    ) -> Result<EventMetadata, IngressError> {
        if source.run != self.id {
            return Err(IngressError::WrongRun);
        }
        if self.pump_open || self.state != RunState::Running {
            return Err(IngressError::NotRunning);
        }
        let plan = self.plan.as_ref().ok_or(IngressError::RevokedSource)?;
        let (event_index, last_seq) = self
            .events
            .host_sources
            .get(&source.id)
            .filter(|entry| {
                plan.events()
                    .get(entry.event)
                    .is_some_and(|event| event.id() == source.event)
            })
            .map(|entry| (entry.event, entry.last_seq))
            .ok_or(IngressError::RevokedSource)?;
        if source_seq == 0 || source_seq <= last_seq {
            return Err(IngressError::SequenceNotIncreasing);
        }
        let plan = plan.clone();
        let checked = plan
            .events()
            .get(event_index)
            .ok_or(IngressError::RevokedSource)?;
        checked
            .payload_type()
            .checked_charge(payload, usize::MAX)
            .map_err(|_| IngressError::InvalidPayload)?;
        let accepted = self.admit_event(
            &plan,
            payload,
            EventAdmission {
                event: event_index,
                origin: EventOrigin::Host { source: source.id },
                cause: None,
                cause_depth: 0,
                source_seq,
                wall_ns,
                graph_source: None,
                tick: None,
            },
        );
        let metadata = match accepted {
            Ok(metadata) => metadata,
            Err(LIMIT) => {
                self.events.stats.overflows = self.events.stats.overflows.saturating_add(1);
                return Err(IngressError::DeliveryLimit);
            }
            Err(_) => return Err(IngressError::DeliveryLimit),
        };
        if let Some(entry) = self.events.host_sources.get_mut(&source.id) {
            entry.last_seq = source_seq;
        }
        Ok(metadata)
    }
    /// Admit one host trigger of a root `core/on-manual` entry. Like `ingest_event`, this
    /// is synchronous bounded admission only; the handler root starts in a later `pump`.
    /// `payload` is required exactly when the entry declares a payload type.
    pub fn trigger(
        &mut self,
        node: NodeId,
        payload: Option<&Value>,
        wall_ns: Option<i64>,
    ) -> Result<EventMetadata, IngressError> {
        if self.pump_open || self.state != RunState::Running {
            return Err(IngressError::NotRunning);
        }
        let plan = self
            .plan
            .as_ref()
            .ok_or(IngressError::UnknownEntry)?
            .clone();
        let manual = plan
            .manuals()
            .iter()
            .position(|manual| manual.node() == node)
            .ok_or(IngressError::UnknownEntry)?;
        let payload = match (plan.manuals()[manual].payload_type(), payload) {
            (Some(shape), Some(payload)) => {
                shape
                    .checked_charge(payload, usize::MAX)
                    .map_err(|_| IngressError::InvalidPayload)?;
                payload
            }
            (None, None) => &Value::Unit,
            _ => return Err(IngressError::InvalidPayload),
        };
        let source_seq = self.events.manual_sequences[manual]
            .checked_add(1)
            .ok_or(IngressError::DeliveryLimit)?;
        let accepted = self.admit_event(
            &plan,
            payload,
            EventAdmission {
                event: plan.events().len() + manual,
                origin: EventOrigin::Manual,
                cause: None,
                cause_depth: 0,
                source_seq,
                wall_ns,
                graph_source: None,
                tick: None,
            },
        );
        let metadata = match accepted {
            Ok(metadata) => metadata,
            Err(LIMIT) => {
                self.events.stats.overflows = self.events.stats.overflows.saturating_add(1);
                return Err(IngressError::DeliveryLimit);
            }
            Err(_) => return Err(IngressError::DeliveryLimit),
        };
        self.events.manual_sequences[manual] = source_seq;
        Ok(metadata)
    }
    pub(super) fn waiter_count(&self) -> usize {
        self.events.waiters.len()
    }
    pub(super) fn next_waiter_token(&mut self) -> Result<u64, KernelFault> {
        self.events.waiters.token()
    }
    pub(super) fn register_waiter(
        &mut self,
        token: u64,
        event: usize,
        owner: ControlIx,
        strand: Strand,
        timer: Option<TimerKey>,
    ) -> Result<(), RunError> {
        self.events
            .waiters
            .insert(token, event, owner, strand, timer)
    }
    pub(super) fn take_waiter_for_cancel(&mut self, strand: StrandId) -> Option<Awaiter> {
        let waiter = self.events.waiters.detach_strand(strand)?;
        if let Some(completion) = &waiter.completion
            && completion.metadata.is_some()
        {
            self.events.stats.pending = self.events.stats.pending.saturating_sub(1);
            self.events.stats.pending_bytes = self
                .events
                .stats
                .pending_bytes
                .saturating_sub(completion.charge);
            self.events.stats.live_payload_bytes = self
                .events
                .stats
                .live_payload_bytes
                .saturating_sub(completion.charge);
            self.events.stats.dropped = self.events.stats.dropped.saturating_add(1);
        }
        Some(waiter)
    }
    pub(super) fn complete_waiter_timeout(&mut self, token: u64) -> Result<bool, RunError> {
        self.events.waiters.timeout(token)
    }
    pub(super) fn pop_completed_waiter(&mut self) -> Option<Awaiter> {
        self.events.waiters.pop_ready().map(|(_, waiter)| waiter)
    }
    pub(super) fn pending_await_completions(&self) -> bool {
        self.events.waiters.has_ready()
    }
    pub(super) fn published_await_delivery(&mut self, completion: &AwaitCompletion) {
        if completion.metadata.is_some() {
            self.events.stats.pending = self.events.stats.pending.saturating_sub(1);
            self.events.stats.pending_bytes = self
                .events
                .stats
                .pending_bytes
                .saturating_sub(completion.charge);
            self.events.stats.live_payload_bytes = self
                .events
                .stats
                .live_payload_bytes
                .saturating_sub(completion.charge);
            self.events.stats.delivered = self.events.stats.delivered.saturating_add(1);
        }
    }
    pub(super) fn dropped_await_delivery(&mut self, completion: &AwaitCompletion) {
        if completion.metadata.is_some() {
            self.events.stats.pending = self.events.stats.pending.saturating_sub(1);
            self.events.stats.pending_bytes = self
                .events
                .stats
                .pending_bytes
                .saturating_sub(completion.charge);
            self.events.stats.live_payload_bytes = self
                .events
                .stats
                .live_payload_bytes
                .saturating_sub(completion.charge);
            self.events.stats.dropped = self.events.stats.dropped.saturating_add(1);
        }
    }
    pub(super) fn event_waiter_count(&self) -> usize {
        self.waiter_count()
    }
    fn event_frame_room(&self) -> bool {
        let Some(template) = &self.event_template else {
            return false;
        };
        self.activations
            .len()
            .saturating_sub(self.events.live_stop_frames)
            < self.options.limits.max_activations
            && self
                .activations
                .len()
                .checked_add(1)
                .and_then(|n| n.checked_mul(template.frame_storage))
                .and_then(|n| n.checked_add(template.template_storage))
                .is_some_and(|n| n <= self.options.limits.max_frame_storage_bytes)
            && self.regions.has_room(1, 1, &self.options.limits)
    }
    pub(super) fn event_dispatchable(&self) -> bool {
        !self.events.ready.is_empty()
            && self.event_frame_room()
            && matches!(self.state, RunState::Running | RunState::Draining)
    }
    pub(super) fn emit_event(
        &mut self,
        plan: &ControlPlan,
        activation: &mut Strand,
        owner: ControlIx,
        event: usize,
        payload: Value,
    ) -> Result<(), KernelFault> {
        let result = self.enqueue_event(plan, activation, owner, event, payload);
        if result == Err(LIMIT) {
            self.events.stats.overflows = self.events.stats.overflows.saturating_add(1);
        }
        result
    }
    fn enqueue_event(
        &mut self,
        plan: &ControlPlan,
        activation: &mut Strand,
        owner: ControlIx,
        event: usize,
        payload: Value,
    ) -> Result<(), KernelFault> {
        if self.state != RunState::Running {
            return Err(STOPPING);
        }
        let depth = activation
            .cause_depth
            .checked_add(1)
            .filter(|d| *d <= self.options.max_cause_depth)
            .ok_or(DEPTH)?;
        let source_seq = self
            .events
            .source_sequences
            .get(owner.index())
            .and_then(|n| n.checked_add(1))
            .ok_or(LIMIT)?;
        self.admit_event(
            plan,
            &payload,
            EventAdmission {
                event,
                origin: EventOrigin::Graph {
                    node: plan.nodes()[owner.index()].node(),
                    scope: plan.nodes()[owner.index()].scope(),
                },
                cause: Some(activation.sequence),
                cause_depth: depth,
                source_seq,
                wall_ns: None,
                graph_source: Some(owner.index()),
                tick: None,
            },
        )?;
        Ok(())
    }

    pub(super) fn admit_event(
        &mut self,
        plan: &ControlPlan,
        payload: &Value,
        admission: EventAdmission,
    ) -> Result<EventMetadata, KernelFault> {
        let EventAdmission {
            event,
            origin: event_origin,
            cause,
            cause_depth,
            source_seq,
            wall_ns,
            graph_source,
            tick,
        } = admission;
        let (payload_type, target) = channel(plan, event).ok_or(LIMIT)?;
        let charge = match payload_type {
            Some(shape) => shape
                .checked_charge(payload, usize::MAX)
                .map_err(|_| LIMIT)?,
            None if matches!(payload, Value::Unit) => 0,
            None => return Err(LIMIT),
        };
        let ingestion = self.events.ingestion.checked_add(1).ok_or(LIMIT)?;
        let mut pending = self.events.stats.pending;
        let mut bytes = self.events.stats.pending_bytes;
        let mut live = self.events.stats.live_payload_bytes;
        let mut changes = Vec::with_capacity(self.events.by_event[event].len());
        let waiter_tokens: Vec<_> = self.events.waiters.matching(event).collect();
        let mut waiter_charges = Vec::with_capacity(waiter_tokens.len());
        for &token in &waiter_tokens {
            let waiter = self.events.waiters.by_token.get(&token).ok_or(LIMIT)?;
            let retained = charge
                .checked_mul(plan.value_targets(waiter.owner).len())
                .ok_or(LIMIT)?;
            let delivery_charge = charge.max(retained);
            pending = pending.checked_add(1).ok_or(LIMIT)?;
            bytes = bytes.checked_add(delivery_charge).ok_or(LIMIT)?;
            live = live.checked_add(delivery_charge).ok_or(LIMIT)?;
            if self.event_template.as_ref().is_none_or(|template| {
                template
                    .local_bytes
                    .checked_add(retained)
                    .is_none_or(|n| n > self.options.limits.max_state_bytes)
            }) {
                return Err(LIMIT);
            }
            waiter_charges.push((token, delivery_charge));
        }
        for &handler in &self.events.by_event[event] {
            let queue = &self.events.handlers[handler];
            // A busy Drop target is a deliberate policy loss, not a capacity overflow.
            // Type/causal checks above still apply, but no payload is retained for it.
            let drops_busy = queue.drops_busy();
            let replaces = queue.mode == ReentrancyMode::Replace;
            let overflow = !replaces && !drops_busy && queue.pending.len() >= queue.capacity;
            let (remove, skip) = if replaces {
                // Replace always retains only the latest pending envelope. Its intrinsic
                // coalescing supersedes the configured capacity overflow selector.
                (queue.pending.len(), false)
            } else if drops_busy {
                (0, true)
            } else if overflow {
                match queue.overflow {
                    OverflowPolicy::DropNewest => (0, true),
                    OverflowPolicy::DropOldest => (1, false),
                    OverflowPolicy::CoalesceLatest => (queue.pending.len(), false),
                    OverflowPolicy::Fault | OverflowPolicy::Backpressure => {
                        return Err(LIMIT);
                    }
                }
            } else {
                (0, false)
            };
            for old in queue.pending.iter().take(remove) {
                pending = pending.checked_sub(1).ok_or(LIMIT)?;
                bytes = bytes.checked_sub(old.charge).ok_or(LIMIT)?;
                live = live.checked_sub(old.charge).ok_or(LIMIT)?;
            }
            let mut delivery_charge = 0;
            if !skip {
                let retained = charge
                    .checked_mul(plan.value_targets(queue.entry).len())
                    .ok_or(LIMIT)?;
                delivery_charge = charge.max(retained);
                pending = pending.checked_add(1).ok_or(LIMIT)?;
                bytes = bytes.checked_add(delivery_charge).ok_or(LIMIT)?;
                live = live.checked_add(delivery_charge).ok_or(LIMIT)?;
                let template = self.event_template.as_ref().ok_or(LIMIT)?;
                if template
                    .local_bytes
                    .checked_add(retained)
                    .is_none_or(|n| n > self.options.limits.max_state_bytes)
                {
                    return Err(LIMIT);
                }
                if self.options.limits.max_activations == 0
                    || self.options.limits.max_strands == 0
                    || self.options.limits.max_regions == 0
                {
                    return Err(LIMIT);
                }
            }
            changes.push(Disposition {
                handler,
                remove,
                skip,
                overflow,
                charge: delivery_charge,
                cancel: queue.replacement_target(),
            });
        }
        if pending > self.options.limits.max_pending_events
            || bytes > self.options.limits.max_pending_event_bytes
            || live > self.options.limits.max_live_event_payload_bytes
        {
            return Err(LIMIT);
        }
        let origin = EventMetadata {
            ingestion_seq: ingestion,
            target,
            origin: event_origin,
            source_seq,
            cause,
            cause_depth,
            monotonic_ns: self.now,
            wall_ns,
        };
        // Every refusal above precedes mutation of any target queue or identity counter.
        // Retire every eviction before retaining a replacement, so commit does not
        // temporarily exceed the admitted total payload/item counts.
        for change in &changes {
            self.events.remove_ready(change.handler);
            let queue = &mut self.events.handlers[change.handler];
            for _ in 0..change.remove {
                queue.pending.pop_front();
            }
        }
        for change in changes {
            let queue = &mut self.events.handlers[change.handler];
            self.events.stats.dropped = self
                .events
                .stats
                .dropped
                .saturating_add(change.remove as u64 + u64::from(change.skip));
            self.events.stats.overflows = self
                .events
                .stats
                .overflows
                .saturating_add(u64::from(change.overflow));
            if !change.skip {
                queue.pending.push_back(Envelope {
                    metadata: origin,
                    payload: payload.clone(),
                    tick,
                    charge: change.charge,
                });
                self.events.stats.accepted = self.events.stats.accepted.saturating_add(1);
            }
            if let Some(root) = change.cancel {
                queue.cancel_requested = Some(root);
                self.events.cancel_pending.push_back(root);
            }
            self.events.make_ready(change.handler);
        }
        for (token, delivery_charge) in waiter_charges {
            let timer_key = self.timers.detach_await(token);
            if let Some(key) = timer_key {
                let Some(TimerTarget::Await {
                    run,
                    token: timer_token,
                }) = self.timers.remove(key)
                else {
                    return Err(LIMIT);
                };
                if run != self.id || timer_token != token {
                    return Err(LIMIT);
                }
            }
            self.events
                .waiters
                .accept(
                    token,
                    AwaitCompletion {
                        metadata: Some(origin),
                        payload: Some(payload.clone()),
                        charge: delivery_charge,
                        timed_out: false,
                    },
                )
                .map_err(|_| LIMIT)?;
            self.events.stats.accepted = self.events.stats.accepted.saturating_add(1);
        }
        self.events.stats.pending = pending;
        self.events.stats.pending_bytes = bytes;
        self.events.stats.live_payload_bytes = live;
        self.events.ingestion = ingestion;
        if let Some(owner) = graph_source {
            self.events.source_sequences[owner] = source_seq;
        }
        self.events.last = Some(origin);
        self.regions.charge(
            self.events.by_event[event]
                .len()
                .saturating_add(waiter_tokens.len()),
        );
        Ok(origin)
    }
    /// Apply Replace only at an actor-owned boundary: the emitting strand and its shared
    /// frame must already be restored, even when it targets its own root from a function
    /// or Fork child. Cancellation schedules existing bounded cleanup; it runs no nodes.
    pub(super) fn cancel_replaced_roots(&mut self) -> Result<(), RunError> {
        if !matches!(self.state, RunState::Running | RunState::Draining) {
            self.events.cancel_pending.clear();
            return Ok(());
        }
        while let Some(activation) = self.events.cancel_pending.pop_front() {
            let root = RegionId::root(activation);
            // A terminal self-Emit can naturally retire its old root during restoration.
            // Identities never reuse, so such a request cannot target its replacement.
            if self.regions.contains(root) {
                self.cancel_region(root)?;
                self.complete_regions(root)?;
            }
        }
        Ok(())
    }
    pub(super) fn dispatch_event(&mut self, plan: &ControlPlan) -> Result<bool, RunError> {
        if !self.event_dispatchable() {
            return Ok(false);
        }
        let (_, handler) = *self.events.ready.first().ok_or(RunError::InvalidState)?;
        let queue = &self.events.handlers[handler];
        if !queue.has_slot() {
            return Err(RunError::InvalidState);
        }
        let entry = queue.entry;
        let envelope = queue.pending.front().ok_or(RunError::InvalidState)?;
        let template = self.event_template.as_ref().ok_or(RunError::InvalidState)?;
        let (sequence, strand_id, region) = self.regions.mint_root()?;
        let mut local_bytes = Tally::new(Some(self.budget.clone()), 0);
        if !local_bytes.try_set(template.local_bytes) {
            return Err(RunError::AggregateLimit);
        }
        let mut frame = Box::new(ActivationState {
            budget: self.budget.clone(),
            retained_bytes: Tally::new(Some(self.budget.clone()), 0),
            owned_bytes: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            cleanup_scope: None,
            cleanup_remaining: self.options.max_cleanup_operations,
            pull_owner: None,
            data: DataActivation::from_prepared(
                template.prepared.clone(),
                template.source_revision,
                self.seed,
                sequence,
                self.options.data.clone(),
                Some(self.budget.clone()),
            )?,
            locals: template.locals.clone(),
            states: template.states.clone(),
            local_bytes,
            outputs: vec![None; self.layout.output_count],
        });
        match envelope.tick {
            Some(tick) => Self::publish_tick(
                plan,
                &self.layout,
                &mut frame,
                entry,
                tick,
                self.options.limits.max_state_bytes,
            )?,
            None => Self::publish_match_value(
                plan,
                &self.layout,
                &mut frame,
                entry,
                Some(envelope.payload.clone()),
                self.options.limits.max_state_bytes,
            )?,
        }
        self.activations.insert(
            sequence,
            frame,
            self.options
                .limits
                .max_activations
                .saturating_add(self.events.live_stop_frames),
        )?;
        self.events.remove_ready(handler);
        let queue = &mut self.events.handlers[handler];
        let envelope = queue.pending.pop_front().ok_or(RunError::InvalidState)?;
        if !queue.active_roots.insert(sequence) {
            return Err(RunError::InvalidState);
        }
        self.events
            .active
            .insert(sequence, (handler, envelope.charge));
        self.events.stats.pending -= 1;
        self.events.stats.pending_bytes -= envelope.charge;
        self.events.stats.delivered = self.events.stats.delivered.saturating_add(1);
        self.regions.open(
            region,
            Region {
                frames: None,
                retired_cursor: None,
                parent: None,
                activation: sequence,
                depth: 0,
                max_cause_depth: envelope.metadata.cause_depth,
                live: IdSet::one(strand_id),
                children: IdSet::default(),
                continuation: None,
                kind: RegionKind::Root,
            },
        )?;
        let strand = Strand {
            function: None,
            transfer: Transfer::None,
            cleanup_deadline: None,
            iteration_context: None,
            id: strand_id,
            region,
            sequence,
            entry,
            cause_depth: envelope.metadata.cause_depth,
            pc: Some(entry.into()),
            pull: PullState::default(),
            stack: vec![],
            yielded: false,
            reenter: None,
            cleanup: false,
            cleanup_remaining: 0,
        };
        self.ready.push_back(strand_id, strand);
        // Parallel may admit another pending root before the first one retires. Only
        // the queue head participates, preserving global ingestion/handler ordering.
        self.events.make_ready(handler);
        Ok(true)
    }
    pub(super) fn retire_activation(&mut self, id: u64) -> Result<(), RunError> {
        let frame = self.activations.remove(id)?;
        if frame.is_none() && self.last_error != Some(RunError::Interrupted) {
            return Err(RunError::InvalidState);
        }
        self.events.finish(id)?;
        if id >= self.events.static_starts && id < self.events.static_roots {
            self.events.live_stop_frames = self
                .events
                .live_stop_frames
                .checked_sub(1)
                .ok_or(RunError::InvalidState)?;
        }
        Ok(())
    }
}
