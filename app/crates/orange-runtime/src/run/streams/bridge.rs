//! Finite per-consumer stream queues. All data and scheduling remain on the Run actor.
use super::*;
use orange_model::value::HandleId;
mod buffer;
mod drive;
mod push;

/// Queue limits exclude the one already accepted pull result owned by each consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamBufferOptions {
    pub capacity: usize,
    pub max_bytes: usize,
    pub overflow: StreamOverflow,
}
impl Default for StreamBufferOptions {
    fn default() -> Self {
        Self {
            capacity: 64,
            max_bytes: 1024 * 1024,
            overflow: StreamOverflow::Backpressure,
        }
    }
}
/// Opaque identity; bridge identities never repeat within their issuing run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamBridge {
    run: u64,
    sequence: u64,
}
impl StreamBridge {
    pub fn id(self) -> u64 {
        self.sequence
    }
}
/// A host's push authority and a monotonic consumer-closed signal. No queued data.
#[derive(Clone)]
pub struct PushStreamSource {
    bridge: StreamBridge,
    closed: Arc<AtomicBool>,
}
impl PushStreamSource {
    pub fn bridge(&self) -> StreamBridge {
        self.bridge
    }
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}
impl std::fmt::Debug for PushStreamSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PushStreamSource")
            .field("bridge", &self.bridge)
            .field("closed", &self.is_closed())
            .finish()
    }
}
#[derive(Debug)]
pub struct PushStream {
    pub source: PushStreamSource,
    pub stream: StreamHandle,
}
#[derive(Debug, Clone, Copy)]
pub struct TeeStreams {
    pub bridge: StreamBridge,
    pub left: StreamHandle,
    pub right: StreamHandle,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushAdmission {
    Accepted,
    Dropped,
}
/// Counts refer to accepted consumer handoff, including an unpublished pull result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamBranchStats {
    pub accepted: u64,
    pub delivered: u64,
    pub dropped: u64,
    pub queued_items: usize,
    pub queued_bytes: usize,
    pub closed: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamBridgeStats {
    pub bridge: StreamBridge,
    pub item: TypeExpr,
    pub source_sequence: u64,
    pub staged_bytes: usize,
    pub branches: [Option<StreamBranchStats>; 2],
}

#[derive(Debug, Clone, Copy)]
pub(in crate::run::streams) struct BranchLink {
    bridge: u64,
    index: usize,
}
struct Branch {
    handle: StreamHandle,
    closed: Arc<AtomicBool>,
    queue: VecDeque<(Value, usize)>,
    retained: Tally,
    waiting: Option<StreamNextId>,
    closed_seen: bool,
    accepted: u64,
    delivered: u64,
    dropped: u64,
}
impl Branch {
    fn live(&self) -> bool {
        !self.closed.load(Ordering::Relaxed)
    }
    fn stats(&self) -> StreamBranchStats {
        StreamBranchStats {
            accepted: self.accepted,
            delivered: self.delivered,
            dropped: self.dropped,
            queued_items: self.queue.len(),
            queued_bytes: self.retained.get(),
            closed: !self.live(),
        }
    }
    fn discard(&mut self) {
        self.dropped = self.dropped.saturating_add(self.queue.len() as u64);
        self.queue.clear();
        let _ = self.retained.try_set(0);
    }
    fn pop(&mut self) -> Option<Value> {
        let (value, bytes) = self.queue.pop_front()?;
        let _ = self.retained.try_set(self.retained.get() - bytes);
        self.delivered = self.delivered.saturating_add(1);
        Some(value)
    }
}
struct Hub {
    item: TypeExpr,
    shape: ValueShape,
    options: StreamBufferOptions,
    source: Option<StreamConsumer>,
    source_wait: Option<StreamNextId>,
    source_sequence: u64,
    staged: Option<(Value, usize)>,
    retained: Tally,
    terminal: Option<Result<(), KernelFault>>,
    branches: Vec<Branch>,
    depth: u8,
}
impl Hub {
    fn stats(&self, id: StreamBridge) -> StreamBridgeStats {
        StreamBridgeStats {
            bridge: id,
            item: self.item.clone(),
            source_sequence: self.source_sequence,
            staged_bytes: self.retained.get(),
            branches: std::array::from_fn(|i| self.branches.get(i).map(Branch::stats)),
        }
    }
    fn fail(&mut self, fault: KernelFault) {
        self.terminal = Some(Err(fault));
        self.staged = None;
        let _ = self.retained.try_set(0);
        for branch in &mut self.branches {
            branch.discard();
        }
    }
    fn blocked(&self) -> bool {
        self.options.overflow == StreamOverflow::Backpressure
            && self.branches.iter().any(|b| {
                b.live()
                    && (b.queue.len() >= self.options.capacity
                        || self.staged.as_ref().map_or(
                            b.retained.get() >= self.options.max_bytes,
                            |(_, bytes)| {
                                b.retained
                                    .get()
                                    .checked_add(*bytes)
                                    .is_none_or(|n| n > self.options.max_bytes)
                            },
                        ))
            })
    }
    fn can_progress(&self) -> bool {
        self.branches.iter().any(|b| {
            b.live() && b.waiting.is_some() && (!b.queue.is_empty() || self.terminal.is_some())
        }) || (self.source.is_some()
            && self.source_wait.is_none()
            && !self.blocked()
            && self
                .branches
                .iter()
                .any(|b| b.live() && b.waiting.is_some()))
    }
}
#[derive(Default)]
pub(in crate::run) struct Bridges {
    next: u64,
    hubs: BTreeMap<u64, Hub>,
    sources: BTreeMap<HandleId, u64>,
    ready: VecDeque<u64>,
    reconciling: bool,
}
impl Bridges {
    pub fn has_ready(&self) -> bool {
        !self.ready.is_empty()
    }
    pub fn clear(&mut self) {
        self.hubs.clear();
        self.sources.clear();
        self.ready.clear();
        self.reconciling = false;
    }
    fn wake(&mut self, id: u64) {
        if !self.ready.contains(&id) {
            self.ready.push_back(id);
        }
    }
    pub fn wake_source(&mut self, handle: HandleId) {
        if let Some(&id) = self.sources.get(&handle) {
            self.wake(id);
        }
    }
    fn schedule(&mut self, id: u64) {
        if self.hubs.get(&id).is_some_and(Hub::can_progress) {
            self.wake(id);
        }
    }
    fn remove(&mut self, id: u64) -> Option<Hub> {
        let hub = self.hubs.remove(&id)?;
        if let Some(source) = &hub.source {
            self.sources.remove(&source.handle.resource.id());
        }
        self.ready.retain(|n| *n != id);
        Some(hub)
    }
}
const OVERFLOW: KernelFault = KernelFault {
    code: "core/QueueOverflow",
    message_key: "core/stream-queue-overflow",
    message: "The stream queue is full.",
};
const LIMIT: KernelFault = KernelFault {
    code: "core/LimitExceeded",
    message_key: "core/stream-limit",
    message: "The stream exceeds a configured bound.",
};
const REVOKED: KernelFault = KernelFault {
    code: "core/StaleHandle",
    message_key: "core/stream-stale",
    message: "The stream is closed or its demand is stale.",
};

impl Run {
    pub(in crate::run) fn execute_stream_tee(
        &mut self,
        plan: &ControlPlan,
        strand: &Strand,
        shared: &mut ActivationState,
        owner: ControlIx,
        value: Value,
        options: StreamBufferOptions,
    ) -> Result<Step, RunError> {
        let node = &plan.nodes()[owner.index()];
        let handle = match self.graph_stream_handle(&value, strand.region) {
            Ok((resource, _)) => StreamHandle { resource },
            Err(error) => return Ok(failure(node, resources::resource_fault(error))),
        };
        let tee = match self.tee_stream_inner(handle, options, Some(strand.region)) {
            Ok(tee) => tee,
            Err(error) => return Ok(failure(node, super::graph::stream_fault(error))),
        };
        let left = self
            .stream_value(tee.left)
            .map_err(|_| RunError::AggregateLimit)?;
        let right = self
            .stream_value(tee.right)
            .map_err(|_| RunError::AggregateLimit)?;
        Self::publish_effect_values(
            plan,
            &self.layout,
            shared,
            owner,
            Some(&[left, right]),
            self.options.limits.max_state_bytes,
        )?;
        Ok((node.then(), ControlOutcome::Then, None))
    }
    fn check_buffer_options(&self, options: StreamBufferOptions) -> Result<(), StreamError> {
        if options.capacity == 0 || options.max_bytes == 0 {
            return Err(StreamError::Contract);
        }
        if options.capacity > self.options.limits.max_stream_bridge_items
            || options.max_bytes > self.options.limits.max_stream_bridge_bytes
        {
            return Err(StreamError::Limit);
        }
        Ok(())
    }
    fn bridge_branch(
        &mut self,
        id: u64,
        index: usize,
        item: &TypeExpr,
        shape: &ValueShape,
        owner: Option<RegionId>,
    ) -> Result<Branch, StreamError> {
        let closed = Arc::new(AtomicBool::new(false));
        let resource = StreamResource {
            bridge: Some(BranchLink { bridge: id, index }),
            producer: None,
            shape: shape.clone(),
            claimed: false,
            graph_consumer: None,
            next: 0,
            pending: None,
            retained: Tally::new(Some(self.budget.clone()), 0),
            closed: closed.clone(),
            terminal: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            collection: None,
        };
        let resource = self
            .resources
            .insert(item, Box::new(resource), self.options.limits.max_resources)
            .map_err(|_| StreamError::Limit)?;
        if self.resources.set_owner(resource, owner).is_err() {
            let _ = self.resources.remove(resource);
            return Err(StreamError::Stale);
        }
        Ok(Branch {
            handle: StreamHandle {
                resource: ResourceHandle::for_run(self.id, resource),
            },
            closed,
            queue: VecDeque::new(),
            retained: Tally::new(Some(self.budget.clone()), 0),
            waiting: None,
            closed_seen: false,
            accepted: 0,
            delivered: 0,
            dropped: 0,
        })
    }
    /// Split one unclaimed stream. Refusal acquires neither branch and preserves
    /// the input's consuming authority. Queues are finite and pull only on demand.
    pub fn tee_stream(
        &mut self,
        source: StreamHandle,
        options: StreamBufferOptions,
    ) -> Result<TeeStreams, StreamError> {
        if self.pump_open || self.state != RunState::Running {
            return Err(StreamError::NotRunning);
        }
        self.tee_stream_inner(source, options, None)
    }
    pub(in crate::run) fn tee_stream_inner(
        &mut self,
        source: StreamHandle,
        options: StreamBufferOptions,
        owner: Option<RegionId>,
    ) -> Result<TeeStreams, StreamError> {
        self.check_buffer_options(options)?;
        let item = self
            .resource_kind(source.resource)
            .map_err(|e| {
                if e == ResourceError::WrongRun {
                    StreamError::WrongRun
                } else {
                    StreamError::Stale
                }
            })?
            .clone();
        let resource = self.stream_resource(source)?;
        if resource.claimed {
            return Err(StreamError::Claimed);
        }
        let shape = resource.shape.clone();
        let parent = resource.bridge;
        let depth = parent
            .and_then(|p| self.bridges.hubs.get(&p.bridge).map(|h| h.depth))
            .unwrap_or(0)
            + 1;
        if depth > 32
            || self
                .resources
                .len()
                .checked_add(2)
                .is_none_or(|n| n > self.options.limits.max_resources)
        {
            return Err(StreamError::Limit);
        }
        let id = self.bridges.next.checked_add(1).ok_or(StreamError::Limit)?;
        let left = self.bridge_branch(id, 0, &item, &shape, owner)?;
        let right = match self.bridge_branch(id, 1, &item, &shape, owner) {
            Ok(right) => right,
            Err(error) => {
                let _ = self.resources.remove(left.handle.resource.id());
                return Err(error);
            }
        };
        let source = match self.claim_stream_inner(source) {
            Ok(source) => source,
            Err(error) => {
                let _ = self.resources.remove(left.handle.resource.id());
                let _ = self.resources.remove(right.handle.resource.id());
                return Err(error);
            }
        };
        if self
            .resources
            .set_owner(source.handle.resource.id(), None)
            .is_err()
        {
            let _ = self.resources.remove(left.handle.resource.id());
            let _ = self.resources.remove(right.handle.resource.id());
            return Err(StreamError::Stale);
        }
        let result = TeeStreams {
            bridge: StreamBridge {
                run: self.id,
                sequence: id,
            },
            left: left.handle,
            right: right.handle,
        };
        self.bridges.sources.insert(source.handle.resource.id(), id);
        self.bridges.hubs.insert(
            id,
            Hub {
                item,
                shape,
                options,
                source: Some(source),
                source_wait: None,
                source_sequence: 0,
                staged: None,
                retained: Tally::new(Some(self.budget.clone()), 0),
                terminal: None,
                branches: vec![left, right],
                depth,
            },
        );
        self.bridges.next = id;
        Ok(result)
    }
    /// Live metadata only. Retired bridges retain no history or payload.
    pub fn stream_bridge_stats(
        &self,
        bridge: StreamBridge,
    ) -> Result<StreamBridgeStats, StreamError> {
        if bridge.run != self.id {
            return Err(StreamError::WrongRun);
        }
        self.bridges
            .hubs
            .get(&bridge.sequence)
            .map(|h| h.stats(bridge))
            .ok_or(StreamError::Stale)
    }
    pub fn stream_bridges(&self) -> impl Iterator<Item = StreamBridgeStats> + '_ {
        self.bridges.hubs.iter().map(|(&sequence, hub)| {
            hub.stats(StreamBridge {
                run: self.id,
                sequence,
            })
        })
    }
}
