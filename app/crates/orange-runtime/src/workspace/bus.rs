//! Exclusively borrowed, typed workspace fan-out with lifetime-bounded deliveries.
use super::*;
use orange_compiler::{ControlOperation, ControlPlan};
use orange_model::document::{EventScope, OverflowPolicy};
use orange_model::id::{EventDeclId, NodeId};
use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;

static NEXT_BUS: AtomicU64 = AtomicU64::new(1);
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkspaceBusId(u64);
fn identity() -> Result<WorkspaceBusId, WorkspaceBusError> {
    let mut old = NEXT_BUS.load(Ordering::Relaxed);
    loop {
        let next = old
            .checked_add(1)
            .ok_or(WorkspaceBusError::IdentityExhausted)?;
        match NEXT_BUS.compare_exchange_weak(old, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return Ok(WorkspaceBusId(old)),
            Err(current) => old = current,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum WorkspaceBusError {
    #[error("The workspace event bus violates its checked contract.")]
    Contract,
    #[error("The workspace event schema is incompatible.")]
    SchemaMismatch,
    #[error("The event bus or delivery belongs to another workspace or run.")]
    WrongOwner,
    #[error("The run has no matching workspace bus attachment.")]
    NotAttached,
    #[error("The workspace event lease has closed.")]
    Closed,
    #[error("The workspace event exceeds a configured bound.")]
    Limit,
    #[error("The workspace event identity space is exhausted.")]
    IdentityExhausted,
    #[error("The workspace event causal depth limit was reached.")]
    CauseDepth,
    #[error("The workspace delivery is older than an accepted delivery.")]
    SequenceNotIncreasing,
    #[error("The run is not accepting workspace events.")]
    NotRunning,
}
impl WorkspaceBusError {
    pub const fn message_key(self) -> &'static str {
        match self {
            Self::Contract => "core/workspace-bus-contract",
            Self::SchemaMismatch => "core/workspace-bus-schema-mismatch",
            Self::WrongOwner => "core/workspace-bus-wrong-owner",
            Self::NotAttached => "core/workspace-bus-not-attached",
            Self::Closed => "core/workspace-bus-closed",
            Self::Limit => "core/workspace-bus-limit",
            Self::IdentityExhausted => "core/workspace-bus-identity-exhausted",
            Self::CauseDepth => "core/event-cause-depth",
            Self::SequenceNotIncreasing => "core/workspace-bus-sequence-not-increasing",
            Self::NotRunning => "core/event-run-stopping",
        }
    }
    pub(crate) fn fault(self) -> KernelFault {
        KernelFault {
            code: match self {
                Self::CauseDepth => "core/RunawayEventChain",
                Self::NotRunning | Self::Closed => "core/RunStopping",
                Self::Limit | Self::IdentityExhausted => "core/LimitExceeded",
                _ => "core/WorkspaceBusFailure",
            },
            message_key: self.message_key(),
            message: "The workspace event could not be admitted.",
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct WorkspaceBusLimits {
    pub max_events: usize,
    pub max_schema_bytes: usize,
    pub max_endpoints: usize,
    pub max_queue_items: usize,
    pub max_queue_bytes: usize,
    pub max_payload_bytes: usize,
    /// Includes queued and host-drained deliveries until their last owner drops.
    pub max_retained_items: usize,
    pub max_retained_bytes: usize,
    pub max_cause_depth: u16,
}
impl Default for WorkspaceBusLimits {
    fn default() -> Self {
        Self {
            max_events: 4096,
            max_schema_bytes: 16 * 1024 * 1024,
            max_endpoints: 1024,
            max_queue_items: 256,
            max_queue_bytes: 1024 * 1024,
            max_payload_bytes: 1024 * 1024,
            max_retained_items: 4096,
            max_retained_bytes: 64 * 1024 * 1024,
            max_cause_depth: 64,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct WorkspaceQueue {
    pub capacity: usize,
    pub max_bytes: usize,
    pub overflow: OverflowPolicy,
}
impl Default for WorkspaceQueue {
    fn default() -> Self {
        Self {
            capacity: 256,
            max_bytes: 1024 * 1024,
            overflow: OverflowPolicy::Fault,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceBusRetention {
    pub reserved_items: usize,
    pub limit_items: usize,
    pub reserved_bytes: usize,
    pub limit_bytes: usize,
}
#[derive(Clone)]
pub struct WorkspaceBusMeter {
    bytes: Arc<Budget>,
    items: Arc<Budget>,
}
impl WorkspaceBusMeter {
    pub fn retention(&self) -> WorkspaceBusRetention {
        WorkspaceBusRetention {
            reserved_items: self.items.used(),
            limit_items: self.items.limit(),
            reserved_bytes: self.bytes.used(),
            limit_bytes: self.bytes.limit(),
        }
    }
}
impl std::fmt::Debug for WorkspaceBusMeter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.retention().fmt(f)
    }
}
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceBusStats {
    pub accepted: u64,
    pub enqueued: u64,
    pub taken: u64,
    pub dropped: u64,
    pub refused: u64,
    pub endpoints: usize,
    pub queued: usize,
    pub queued_bytes: usize,
    pub ingestion_seq: u64,
}
/// Source time is descriptive; it never advances the recipient's virtual clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceEnvelope {
    pub bus: WorkspaceBusId,
    pub workspace: WorkspaceId,
    pub event: EventDeclId,
    pub source_run: u64,
    pub target_run: u64,
    pub source_node: NodeId,
    pub source_scope: usize,
    pub source_seq: u64,
    pub ingestion_seq: u64,
    pub cause: u64,
    pub cause_depth: u16,
    pub source_monotonic_ns: u64,
}
/// Source facts for EventMetadata, whose other fields hold the target event,
/// cause, depth and source sequence. Unique bus IDs identify the host workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceOrigin {
    pub bus: WorkspaceBusId,
    pub source_run: u64,
    pub source_node: NodeId,
    pub source_scope: usize,
    pub ingestion_seq: u64,
    pub source_monotonic_ns: u64,
}
impl WorkspaceEnvelope {
    pub fn origin(self) -> WorkspaceOrigin {
        WorkspaceOrigin {
            bus: self.bus,
            source_run: self.source_run,
            source_node: self.source_node,
            source_scope: self.source_scope,
            ingestion_seq: self.ingestion_seq,
            source_monotonic_ns: self.source_monotonic_ns,
        }
    }
}
struct Event {
    shape: ValueShape,
}
struct Schema {
    id: WorkspaceBusId,
    workspace: WorkspaceId,
    events: BTreeMap<EventDeclId, Event>,
    _registry: Arc<RegistrySnapshot>,
    meter: WorkspaceBusMeter,
    _hold: Tally,
}
pub(crate) struct Endpoint {
    schema: Arc<Schema>,
    pub run: u64,
    /// Plan event index and whether this run has a handler or potential waiter.
    pub events: BTreeMap<EventDeclId, (usize, bool)>,
    closed: AtomicBool,
    _hold: Tally,
}
impl Endpoint {
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
    fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
    }
}
pub(crate) struct BusLease {
    pub state: Arc<Endpoint>,
    pub last_ingestion: u64,
}
impl BusLease {
    pub fn close(&self) {
        self.state.close();
    }
    pub fn matches(&self, bus: &WorkspaceBus) -> bool {
        self.state.schema.id == bus.id()
    }
}
impl Drop for BusLease {
    fn drop(&mut self) {
        self.close();
    }
}
struct Record {
    state: Arc<Endpoint>,
    queue: VecDeque<WorkspaceDelivery>,
    bytes: usize,
    options: WorkspaceQueue,
}
/// One authenticated, single-use delivery. Host-drained messages remain charged.
///
/// ```compile_fail
/// fn duplicate(message: orange_runtime::WorkspaceDelivery) { let _ = message.clone(); }
/// ```
pub struct WorkspaceDelivery {
    pub(crate) target: Arc<Endpoint>,
    pub(crate) envelope: WorkspaceEnvelope,
    pub(crate) payload: Value,
    charge: usize,
    _bytes: Tally,
    _item: Tally,
}
impl WorkspaceDelivery {
    pub fn envelope(&self) -> WorkspaceEnvelope {
        self.envelope
    }
    pub fn payload(&self) -> &Value {
        &self.payload
    }
}
impl std::fmt::Debug for WorkspaceDelivery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceDelivery")
            .field("envelope", &self.envelope)
            .finish()
    }
}
#[derive(Debug)]
pub struct WorkspaceDeliveryRefused {
    pub error: WorkspaceBusError,
    pub delivery: WorkspaceDelivery,
}
pub struct WorkspaceBus {
    schema: Arc<Schema>,
    records: BTreeMap<u64, Record>,
    limits: WorkspaceBusLimits,
    stats: WorkspaceBusStats,
}
impl std::fmt::Debug for WorkspaceBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceBus")
            .field("id", &self.id())
            .field("stats", &self.stats())
            .field("meter", &self.meter())
            .finish()
    }
}
fn reserve(budget: &Arc<Budget>, bytes: usize) -> Result<Tally, WorkspaceBusError> {
    let mut hold = Tally::new(Some(budget.clone()), 0);
    if !hold.try_set(bytes) {
        return Err(WorkspaceBusError::Limit);
    }
    Ok(hold)
}
impl WorkspaceBus {
    pub fn new(
        document: &GraphDocument,
        registry: Arc<RegistrySnapshot>,
        workspace: &WorkspaceSnapshot,
        limits: WorkspaceBusLimits,
    ) -> Result<Self, WorkspaceBusError> {
        workspace
            .check_schema(document, &registry)
            .map_err(|_| WorkspaceBusError::SchemaMismatch)?;
        let mut events = BTreeMap::new();
        let mut bytes = std::mem::size_of::<Schema>();
        if bytes > limits.max_schema_bytes {
            return Err(WorkspaceBusError::Limit);
        }
        for declaration in document
            .events()
            .values()
            .filter(|e| e.scope == EventScope::Workspace)
        {
            if events.len() >= limits.max_events {
                return Err(WorkspaceBusError::Limit);
            }
            let shape = ValueShape::of_in(&declaration.payload, registry.types().as_ref())
                .ok_or(WorkspaceBusError::Contract)?;
            bytes = bytes
                .checked_add(
                    super::schema::shape_bytes(&shape, limits.max_schema_bytes).map_err(|e| {
                        if e == WorkspaceError::Limit {
                            WorkspaceBusError::Limit
                        } else {
                            WorkspaceBusError::Contract
                        }
                    })?,
                )
                .and_then(|n| n.checked_add(std::mem::size_of::<(EventDeclId, Event)>()))
                .filter(|n| *n <= limits.max_schema_bytes)
                .ok_or(WorkspaceBusError::Limit)?;
            events.insert(declaration.id, Event { shape });
        }
        let meter = WorkspaceBusMeter {
            bytes: Budget::new(limits.max_retained_bytes),
            items: Budget::new(limits.max_retained_items),
        };
        let hold = reserve(&meter.bytes, bytes)?;
        Ok(Self {
            schema: Arc::new(Schema {
                id: identity()?,
                workspace: workspace.id(),
                events,
                _registry: registry,
                meter,
                _hold: hold,
            }),
            records: BTreeMap::new(),
            limits,
            stats: WorkspaceBusStats::default(),
        })
    }
    pub fn id(&self) -> WorkspaceBusId {
        self.schema.id
    }
    pub fn workspace(&self) -> WorkspaceId {
        self.schema.workspace
    }
    pub fn meter(&self) -> WorkspaceBusMeter {
        self.schema.meter.clone()
    }
    pub fn stats(&self) -> WorkspaceBusStats {
        let mut stats = self.stats;
        stats.endpoints = self
            .records
            .values()
            .filter(|r| !r.state.is_closed())
            .count();
        stats.queued = self.records.values().map(|r| r.queue.len()).sum();
        stats.queued_bytes = self.records.values().map(|r| r.bytes).sum();
        stats
    }
    /// Physical retirement happens on this owning actor; logical close is immediate.
    pub fn reap(&mut self) {
        self.records.retain(|_, r| !r.state.is_closed());
    }
    pub fn revoke_run(&mut self, run: u64) -> bool {
        let found = self.records.get(&run).is_some_and(|r| {
            r.state.close();
            true
        });
        self.reap();
        found
    }
    pub(crate) fn register(
        &mut self,
        run: u64,
        workspace: WorkspaceId,
        plan: &ControlPlan,
        options: WorkspaceQueue,
    ) -> Result<BusLease, WorkspaceBusError> {
        self.reap();
        if workspace != self.workspace() {
            return Err(WorkspaceBusError::WrongOwner);
        }
        if self.records.contains_key(&run) {
            return Err(WorkspaceBusError::Contract);
        }
        if self.records.len() >= self.limits.max_endpoints {
            return Err(WorkspaceBusError::Limit);
        }
        if options.capacity == 0
            || options.max_bytes == 0
            || options.capacity > self.limits.max_queue_items
            || options.max_bytes > self.limits.max_queue_bytes
            || options.overflow == OverflowPolicy::Backpressure
        {
            return Err(WorkspaceBusError::Contract);
        }
        let mut events = BTreeMap::new();
        for (index, event) in plan
            .events()
            .iter()
            .enumerate()
            .filter(|(_, e)| e.scope() == EventScope::Workspace)
        {
            if !self
                .schema
                .events
                .get(&event.id())
                .is_some_and(|e| &e.shape == event.payload_type())
            {
                return Err(WorkspaceBusError::SchemaMismatch);
            }
            let receives=!event.handlers().is_empty() || plan.nodes().iter().any(|n|
                matches!(n.operation(),ControlOperation::AwaitEvent {event,..} if *event==index));
            events.insert(event.id(), (index, receives));
        }
        let bytes = events
            .len()
            .checked_mul(std::mem::size_of::<(EventDeclId, (usize, bool))>())
            .and_then(|n| {
                n.checked_add(std::mem::size_of::<Endpoint>() + std::mem::size_of::<Record>())
            })
            .ok_or(WorkspaceBusError::Limit)?;
        let hold = reserve(&self.schema.meter.bytes, bytes)?;
        let state = Arc::new(Endpoint {
            schema: self.schema.clone(),
            run,
            events,
            closed: AtomicBool::new(false),
            _hold: hold,
        });
        self.records.insert(
            run,
            Record {
                state: state.clone(),
                queue: VecDeque::new(),
                bytes: 0,
                options,
            },
        );
        Ok(BusLease {
            state,
            last_ingestion: 0,
        })
    }
    pub(crate) fn take(
        &mut self,
        lease: &BusLease,
    ) -> Result<Option<WorkspaceDelivery>, WorkspaceBusError> {
        self.reap();
        if !lease.matches(self) {
            return Err(WorkspaceBusError::WrongOwner);
        }
        if lease.state.is_closed() {
            return Err(WorkspaceBusError::Closed);
        }
        let record = self
            .records
            .get_mut(&lease.state.run)
            .ok_or(WorkspaceBusError::Closed)?;
        if !Arc::ptr_eq(&record.state, &lease.state) {
            return Err(WorkspaceBusError::WrongOwner);
        }
        let Some(message) = record.queue.pop_front() else {
            return Ok(None);
        };
        record.bytes -= message.charge;
        self.stats.taken = self.stats.taken.saturating_add(1);
        Ok(Some(message))
    }
    pub(crate) fn emit(
        &mut self,
        lease: &BusLease,
        payload: &Value,
        emission: Emission,
    ) -> Result<(), WorkspaceBusError> {
        self.reap();
        let result = self.emit_inner(lease, payload, emission);
        if result.is_err() {
            self.stats.refused = self.stats.refused.saturating_add(1);
        }
        result
    }
    fn emit_inner(
        &mut self,
        lease: &BusLease,
        payload: &Value,
        e: Emission,
    ) -> Result<(), WorkspaceBusError> {
        if !lease.matches(self) {
            return Err(WorkspaceBusError::WrongOwner);
        }
        if lease.state.is_closed() {
            return Err(WorkspaceBusError::Closed);
        }
        if !lease.state.events.contains_key(&e.event) {
            return Err(WorkspaceBusError::SchemaMismatch);
        }
        if e.depth == 0 || e.depth > self.limits.max_cause_depth {
            return Err(WorkspaceBusError::CauseDepth);
        }
        if e.source_seq == 0 {
            return Err(WorkspaceBusError::Contract);
        }
        let shape = &self
            .schema
            .events
            .get(&e.event)
            .ok_or(WorkspaceBusError::SchemaMismatch)?
            .shape;
        let charge = shape
            .checked_charge(payload, self.limits.max_payload_bytes)
            .map_err(|error| match error {
                ValueCheckError::ResourceLimit => WorkspaceBusError::Limit,
                _ => WorkspaceBusError::Contract,
            })?;
        let ingestion = self
            .stats
            .ingestion_seq
            .checked_add(1)
            .ok_or(WorkspaceBusError::IdentityExhausted)?;
        let mut changes = Vec::new();
        // Complete fan-out and new leases are staged before any queue eviction.
        for (&run, record) in &self.records {
            if run == lease.state.run || !record.state.events.get(&e.event).is_some_and(|(_, r)| *r)
            {
                continue;
            }
            let options = record.options;
            if charge > options.max_bytes {
                return Err(WorkspaceBusError::Limit);
            }
            let fits = |removed: usize, bytes: usize| {
                record.queue.len() - removed < options.capacity
                    && bytes
                        .checked_add(charge)
                        .is_some_and(|n| n <= options.max_bytes)
            };
            let (mut remove, mut bytes, mut skip) = (0, record.bytes, false);
            if !fits(0, bytes) {
                match options.overflow {
                    OverflowPolicy::Fault | OverflowPolicy::Backpressure => {
                        return Err(WorkspaceBusError::Limit);
                    }
                    OverflowPolicy::DropNewest => skip = true,
                    OverflowPolicy::CoalesceLatest => {
                        remove = record.queue.len();
                        bytes = 0;
                    }
                    OverflowPolicy::DropOldest => {
                        for old in &record.queue {
                            if fits(remove, bytes) {
                                break;
                            }
                            remove += 1;
                            bytes -= old.charge;
                        }
                    }
                }
            }
            let message = if skip {
                None
            } else {
                if !fits(remove, bytes) {
                    return Err(WorkspaceBusError::Limit);
                }
                let held = charge
                    .checked_add(std::mem::size_of::<WorkspaceDelivery>())
                    .ok_or(WorkspaceBusError::Limit)?;
                let item = reserve(&self.schema.meter.items, 1)?;
                let hold = reserve(&self.schema.meter.bytes, held)?;
                Some(WorkspaceDelivery {
                    target: record.state.clone(),
                    payload: payload.clone(),
                    charge,
                    _item: item,
                    _bytes: hold,
                    envelope: WorkspaceEnvelope {
                        bus: self.id(),
                        workspace: self.workspace(),
                        event: e.event,
                        source_run: lease.state.run,
                        target_run: run,
                        source_node: e.node,
                        source_scope: e.scope,
                        source_seq: e.source_seq,
                        ingestion_seq: ingestion,
                        cause: e.cause,
                        cause_depth: e.depth,
                        source_monotonic_ns: e.now,
                    },
                })
            };
            changes.push((run, remove, skip, message));
        }
        for (run, remove, skip, message) in changes {
            let record = self
                .records
                .get_mut(&run)
                .ok_or(WorkspaceBusError::Contract)?;
            for _ in 0..remove {
                if let Some(old) = record.queue.pop_front() {
                    record.bytes -= old.charge;
                }
            }
            self.stats.dropped = self
                .stats
                .dropped
                .saturating_add(remove as u64 + u64::from(skip));
            if let Some(message) = message {
                record.bytes += message.charge;
                record.queue.push_back(message);
                self.stats.enqueued = self.stats.enqueued.saturating_add(1);
            }
        }
        self.stats.ingestion_seq = ingestion;
        self.stats.accepted = self.stats.accepted.saturating_add(1);
        Ok(())
    }
}
impl Drop for WorkspaceBus {
    fn drop(&mut self) {
        for record in self.records.values() {
            record.state.close();
        }
    }
}
pub(crate) struct Emission {
    pub event: EventDeclId,
    pub node: NodeId,
    pub scope: usize,
    pub source_seq: u64,
    pub cause: u64,
    pub depth: u16,
    pub now: u64,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exhausted_bus_sequence_refuses_without_wrapping_or_reserving_messages() {
        let registry = orange_model::catalog::RegistryBuilder::core(
            orange_model::validation::HostOrigin::new(
                "0.0.0",
                format!("sha256:{}", "a".repeat(64)),
            )
            .unwrap(),
        )
        .build()
        .unwrap();
        let event: EventDeclId = "40000000-0000-4000-8000-000000000001".parse().unwrap();
        let meter = WorkspaceBusMeter {
            bytes: Budget::new(4096),
            items: Budget::new(4),
        };
        let schema = Arc::new(Schema {
            id: identity().unwrap(),
            workspace: super::super::identity().unwrap(),
            events: BTreeMap::from([(
                event,
                Event {
                    shape: ValueShape::U64,
                },
            )]),
            _registry: registry,
            _hold: reserve(&meter.bytes, 32).unwrap(),
            meter: meter.clone(),
        });
        let lease = BusLease {
            state: Arc::new(Endpoint {
                schema: schema.clone(),
                run: 1,
                events: BTreeMap::from([(event, (0, false))]),
                closed: AtomicBool::new(false),
                _hold: reserve(&meter.bytes, 32).unwrap(),
            }),
            last_ingestion: 0,
        };
        let mut bus = WorkspaceBus {
            schema,
            records: BTreeMap::new(),
            limits: WorkspaceBusLimits::default(),
            stats: WorkspaceBusStats {
                ingestion_seq: u64::MAX,
                ..WorkspaceBusStats::default()
            },
        };
        let before = meter.retention();
        let emission = Emission {
            event,
            node: "10000000-0000-4000-8000-000000000001".parse().unwrap(),
            scope: 0,
            source_seq: 1,
            cause: 0,
            depth: 1,
            now: 0,
        };
        assert_eq!(
            bus.emit(&lease, &Value::U64(1), emission),
            Err(WorkspaceBusError::IdentityExhausted)
        );
        assert_eq!(bus.stats().ingestion_seq, u64::MAX);
        assert_eq!(bus.stats().accepted, 0);
        assert_eq!(meter.retention(), before);
    }
}
