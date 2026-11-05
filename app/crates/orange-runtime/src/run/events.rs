//! Run-scoped event fan-out and FIFO handler ownership. No handler executes in Emit.
use super::*;
use orange_model::document::OverflowPolicy;
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
    pub dropped: u64,
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
    pending: VecDeque<Envelope>,
    active: bool,
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
    /// Only idle handlers with pending deliveries are in this ordered set.
    ready: BTreeSet<(u64, usize)>,
    /// Reservation ownership is independent of a temporarily checked-out frame.
    active: BTreeMap<u64, (usize, usize)>,
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
                            pending: VecDeque::new(),
                            active: false,
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
        if !queue.active
            && let Some(front) = queue.pending.front()
        {
            self.ready.insert((front.metadata.ingestion_seq, handler));
        }
    }
    fn finish(&mut self, activation: u64) -> Result<(), RunError> {
        if let Some((handler, charge)) = self.active.remove(&activation) {
            self.stats.live_payload_bytes = self
                .stats
                .live_payload_bytes
                .checked_sub(charge)
                .ok_or(RunError::InvalidState)?;
            self.handlers[handler].active = false;
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
        for handler in &mut self.handlers {
            handler.pending.clear();
        }
    }
    pub fn abort(&mut self) {
        self.clear();
        self.active.clear();
        self.stats.live_payload_bytes = 0;
        for handler in &mut self.handlers {
            handler.active = false;
        }
    }
    pub fn release(&mut self, high_water: usize) {
        self.abort();
        self.stats.activation_high_water = self.stats.activation_high_water.max(high_water);
        self.handlers = Vec::new();
        self.ready = BTreeSet::new();
        self.by_event = Vec::new();
        self.source_sequences = Vec::new();
        self.live_stop_frames = 0;
    }
}
struct Disposition {
    handler: usize,
    remove: usize,
    skip: bool,
    overflow: bool,
    charge: usize,
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
            let overflow = queue.pending.len() >= queue.capacity;
            let (remove, skip) = if overflow {
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
            let retained = charge
                .checked_mul(plan.value_targets(queue.entry).len())
                .ok_or(LIMIT)?;
            let delivery_charge = charge.max(retained);
            if !skip {
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
    pub(super) fn dispatch_event(&mut self, plan: &ControlPlan) -> Result<bool, RunError> {
        if !self.event_dispatchable() {
            return Ok(false);
        }
        let (_, handler) = *self.events.ready.first().ok_or(RunError::InvalidState)?;
        let queue = &self.events.handlers[handler];
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
        queue.active = true;
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
