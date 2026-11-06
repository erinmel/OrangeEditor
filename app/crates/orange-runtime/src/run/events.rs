//! Run-scoped event fan-out, bounded reentrancy and FIFO delivery ownership.
//! No handler executes in Emit.
use super::*;
use orange_model::document::{OverflowPolicy, ReentrancyMode};
use std::collections::BTreeSet;

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

/// Saturating per-handler delivery counters, with exact current queue occupancy.
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
/// Last successfully ingested graph emission. Contains no payload or host wall clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventMetadata {
    pub ingestion_seq: u64,
    pub source: NodeId,
    pub source_scope: usize,
    pub source_seq: u64,
    pub cause: u64,
    pub cause_depth: u16,
    pub monotonic_ns: u64,
}
struct Envelope {
    metadata: EventMetadata,
    payload: Value,
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
    by_event: Vec<Vec<usize>>,
    source_sequences: Vec<u64>,
    ingestion: u64,
    last: Option<EventMetadata>,
    stats: RunEventStats,
    static_starts: u64,
    static_roots: u64,
    live_stop_frames: usize,
}
impl Events {
    pub fn new(plan: &ControlPlan) -> Self {
        let mut handlers = Vec::new();
        let by_event = plan
            .events()
            .iter()
            .map(|metadata| {
                metadata
                    .handlers()
                    .iter()
                    .map(|handler| {
                        let index = handlers.len();
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
                        index
                    })
                    .collect()
            })
            .collect();
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
            ingestion: 0,
            last: None,
            stats: RunEventStats::default(),
            static_starts: plan.starts().len() as u64,
            static_roots: (plan.starts().len() + plan.stops().len()) as u64,
            live_stop_frames: plan.stops().len(),
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
    pub fn clear(&mut self) {
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
        for handler in &mut self.handlers {
            handler.pending.clear();
        }
    }
    pub fn abort(&mut self) {
        self.clear();
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
        self.source_sequences = Vec::new();
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
        activation: &Strand,
        shared: &ActivationState,
        owner: ControlIx,
        event: usize,
        payload: Value,
    ) -> Result<(), KernelFault> {
        let result = self.enqueue_event(plan, activation, shared, owner, event, payload);
        if result == Err(LIMIT) {
            self.events.stats.overflows = self.events.stats.overflows.saturating_add(1);
        }
        result
    }
    fn enqueue_event(
        &mut self,
        plan: &ControlPlan,
        activation: &Strand,
        shared: &ActivationState,
        owner: ControlIx,
        event: usize,
        payload: Value,
    ) -> Result<(), KernelFault> {
        if self.state != RunState::Running {
            return Err(STOPPING);
        }
        let depth = shared
            .cause_depth
            .checked_add(1)
            .filter(|d| *d <= self.options.max_cause_depth)
            .ok_or(DEPTH)?;
        let metadata = plan.events().get(event).ok_or(LIMIT)?;
        let charge = metadata
            .payload_type()
            .checked_charge(&payload, usize::MAX)
            .map_err(|_| LIMIT)?;
        let ingestion = self.events.ingestion.checked_add(1).ok_or(LIMIT)?;
        let source_seq = self
            .events
            .source_sequences
            .get(owner.index())
            .and_then(|n| n.checked_add(1))
            .ok_or(LIMIT)?;
        let mut pending = self.events.stats.pending;
        let mut bytes = self.events.stats.pending_bytes;
        let mut live = self.events.stats.live_payload_bytes;
        let mut changes = Vec::with_capacity(self.events.by_event[event].len());
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
            source: plan.nodes()[owner.index()].node(),
            source_scope: plan.nodes()[owner.index()].scope(),
            source_seq,
            cause: activation.sequence,
            cause_depth: depth,
            monotonic_ns: self.now,
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
        self.events.stats.pending = pending;
        self.events.stats.pending_bytes = bytes;
        self.events.stats.live_payload_bytes = live;
        self.events.ingestion = ingestion;
        self.events.source_sequences[owner.index()] = source_seq;
        self.events.last = Some(origin);
        self.regions.charge(self.events.by_event[event].len());
        Ok(())
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
            cause_depth: envelope.metadata.cause_depth,
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
        Self::publish_match_value(
            plan,
            &self.layout,
            &mut frame,
            entry,
            Some(envelope.payload.clone()),
            self.options.limits.max_state_bytes,
        )?;
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
