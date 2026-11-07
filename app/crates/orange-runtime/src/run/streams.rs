//! Affine, run-owned pull-stream ABI. Adapters initiate bounded work and return;
//! pending work completes through an identity-checked host call, never by polling.
use super::*;
use orange_model::id::TypeExpr;
use orange_model::value::shape::ValueShape;
use std::sync::atomic::AtomicBool;
mod bridge;
mod graph;
pub(in crate::run) use bridge::Bridges;
pub use bridge::{
    PushAdmission, PushStream, PushStreamSource, StreamBranchStats, StreamBridge,
    StreamBridgeStats, StreamBufferOptions, TeeStreams,
};
pub use graph::EffectStreamRefused;
pub(in crate::run) use graph::{GraphStreams, PendingRead, PreparedRead, ReadMode, StreamCursor};
pub use orange_model::document::StreamOverflow;

/// A trusted nonblocking producer. `next` may return one item or initiate one
/// asynchronous read. It must never prefetch an unbounded source or block the actor.
/// The embedding host owns workers and delivers Pending results with `complete_stream`.
pub trait PullStream: Send + Sync + 'static {
    fn next(&mut self, demand: StreamDemand) -> Result<StreamStep, KernelFault>;
    fn close(self: Box<Self>, reason: CloseReason);
}
/// A generation-checked stream identity. Copying it never creates a second consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamHandle {
    resource: ResourceHandle,
}
/// Unique consuming authority; only `claim_stream` constructs it, once per stream.
#[derive(Debug)]
pub struct StreamConsumer {
    handle: StreamHandle,
    finished: bool,
}
/// Identity for one pending pull; a later pull can never accept this one's result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamNextId {
    handle: StreamHandle,
    sequence: u64,
}
/// One demand and a monotonic lifetime signal. Debug never formats producer data.
#[derive(Debug, Clone)]
pub struct StreamDemand {
    pub id: StreamNextId,
    closed: Arc<AtomicBool>,
}
impl StreamDemand {
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}
/// Producer result. `Pending` is resumed by a host completion, never repeated polls.
pub enum StreamStep {
    Item(Value),
    End,
    Pending,
}
impl std::fmt::Debug for StreamStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Item(_) => "Item(..)",
            Self::End => "End",
            Self::Pending => "Pending",
        })
    }
}
/// Public read outcome. Repeated reads while pending return the same demand ID.
pub enum StreamRead {
    Item(Value),
    End,
    Pending(StreamNextId),
    Failed(KernelFault),
}
/// A bounded collection either yields the whole typed list, suspends, or needs
/// another host slice. Refusal/failure never returns a partial list.
pub enum StreamCollectPoll {
    Complete(Value),
    Pending(StreamNextId),
    Working,
    Failed(KernelFault),
}
impl std::fmt::Debug for StreamCollectPoll {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Complete(_) => "Complete(..)",
            Self::Pending(_) => "Pending(..)",
            Self::Working => "Working",
            Self::Failed(_) => "Failed(..)",
        })
    }
}
impl std::fmt::Debug for StreamRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Item(_) => "Item(..)",
            Self::End => "End",
            Self::Pending(_) => "Pending(..)",
            Self::Failed(_) => "Failed(..)",
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum StreamError {
    #[error("The stream queue is full; retry when the consumer frees space.")]
    Full,
    #[error("The stream queue overflowed.")]
    Overflow,
    #[error("The stream source sequence must be nonzero and strictly increasing.")]
    Sequence,
    #[error("The stream belongs to another run.")]
    WrongRun,
    #[error("The stream is closed or its demand is stale.")]
    Stale,
    #[error("The stream already has a consumer.")]
    Claimed,
    #[error("The stream item violates its checked type.")]
    Contract,
    #[error("The stream exceeds a configured bound.")]
    Limit,
    #[error("The run is not accepting stream work.")]
    NotRunning,
}
impl StreamError {
    pub const fn message_key(self) -> &'static str {
        match self {
            Self::Full => "core/stream-full",
            Self::Overflow => "core/stream-queue-overflow",
            Self::Sequence => "core/stream-sequence",
            Self::WrongRun => "core/stream-wrong-run",
            Self::Stale => "core/stream-stale",
            Self::Claimed => "core/stream-claimed",
            Self::Contract => "core/stream-contract",
            Self::Limit => "core/stream-limit",
            Self::NotRunning => "core/stream-not-running",
        }
    }
}
/// Refusal preserves ownership of an unacquired producer; it was never closed.
pub struct StreamRefused {
    pub error: StreamError,
    pub producer: Box<dyn PullStream>,
}
impl std::fmt::Debug for StreamRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamRefused")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}
struct Pending {
    sequence: u64,
    result: Option<Result<Option<Value>, KernelFault>>,
}
struct Collection {
    max: usize,
    items: Vec<Value>,
    retained: Tally,
}
struct StreamResource {
    bridge: Option<bridge::BranchLink>,
    producer: Option<Box<dyn PullStream>>,
    shape: ValueShape,
    claimed: bool,
    graph_consumer: Option<graph::ConsumerKey>,
    next: u64,
    pending: Option<Pending>,
    retained: Tally,
    closed: Arc<AtomicBool>,
    terminal: Arc<std::sync::atomic::AtomicU8>,
    collection: Option<Collection>,
}
impl RunResource for StreamResource {
    fn close(mut self: Box<Self>, reason: CloseReason) {
        self.closed.store(true, Ordering::Relaxed);
        let _ = self
            .terminal
            .compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed);
        if let Some(producer) = self.producer.take() {
            producer.close(reason);
        }
    }
}
impl Drop for StreamResource {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
        let _ = self
            .terminal
            .compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed);
        if let Some(producer) = self.producer.take() {
            producer.close(CloseReason::RunDropped);
        }
    }
}
impl Run {
    /// Owns a producer without reading it. One checked item is the largest buffer
    /// this ABI retains, independently of source length. `max_resources` bounds streams.
    pub fn open_stream(
        &mut self,
        item: &TypeExpr,
        producer: Box<dyn PullStream>,
    ) -> Result<StreamHandle, StreamRefused> {
        let refuse = |error, producer| Err(StreamRefused { error, producer });
        if self.pump_open || self.state != RunState::Running {
            return refuse(StreamError::NotRunning, producer);
        }
        self.insert_stream(item, producer)
    }
    fn insert_stream(
        &mut self,
        item: &TypeExpr,
        producer: Box<dyn PullStream>,
    ) -> Result<StreamHandle, StreamRefused> {
        let refuse = |error, producer| Err(StreamRefused { error, producer });
        let Some(shape) = self
            .plan
            .as_ref()
            .and_then(|p| ValueShape::of_in(item, p.data().snapshot().types().as_ref()))
        else {
            return refuse(StreamError::Contract, producer);
        };
        if self.resources.len() >= self.options.limits.max_resources {
            return refuse(StreamError::Limit, producer);
        }
        let resource = StreamResource {
            bridge: None,
            producer: Some(producer),
            shape,
            claimed: false,
            graph_consumer: None,
            next: 0,
            pending: None,
            retained: Tally::new(Some(self.budget.clone()), 0),
            closed: Arc::new(AtomicBool::new(false)),
            terminal: Arc::new(std::sync::atomic::AtomicU8::new(0)),
            collection: None,
        };
        match self
            .resources
            .insert(item, Box::new(resource), self.options.limits.max_resources)
        {
            Ok(id) => Ok(StreamHandle {
                resource: ResourceHandle::for_run(self.id, id),
            }),
            Err(refused) => {
                let mut any: Box<dyn std::any::Any> = refused.resource;
                let producer = any
                    .downcast_mut::<StreamResource>()
                    .and_then(|r| r.producer.take());
                match producer {
                    Some(producer) => refuse(StreamError::Limit, producer),
                    None => unreachable!("only a StreamResource was inserted"),
                }
            }
        }
    }
    fn stream_resource(
        &mut self,
        handle: StreamHandle,
    ) -> Result<&mut StreamResource, StreamError> {
        if !handle.resource.belongs_to(self.id) {
            return Err(StreamError::WrongRun);
        }
        let kind = self
            .resource_kind(handle.resource)
            .map_err(|_| StreamError::Stale)?
            .clone();
        self.resource_mut::<StreamResource>(handle.resource, &kind)
            .map_err(|_| StreamError::Stale)
    }
    pub fn claim_stream(&mut self, handle: StreamHandle) -> Result<StreamConsumer, StreamError> {
        if self.pump_open || self.state != RunState::Running {
            return Err(StreamError::NotRunning);
        }
        self.claim_stream_inner(handle)
    }
    fn claim_stream_inner(&mut self, handle: StreamHandle) -> Result<StreamConsumer, StreamError> {
        let resource = self.stream_resource(handle)?;
        if resource.claimed {
            return Err(StreamError::Claimed);
        }
        resource.claimed = true;
        Ok(StreamConsumer {
            handle,
            finished: false,
        })
    }
    /// Consumes one item. A pending call never invokes the producer again until
    /// that exact demand is completed and its result consumed.
    pub fn next_stream(
        &mut self,
        consumer: &mut StreamConsumer,
    ) -> Result<StreamRead, StreamError> {
        if self.pump_open {
            return Err(StreamError::NotRunning);
        }
        if !consumer.finished && self.stream_resource(consumer.handle)?.collection.is_some() {
            return Err(StreamError::Claimed);
        }
        self.next_stream_inner(consumer)
    }
    fn next_stream_inner(
        &mut self,
        consumer: &mut StreamConsumer,
    ) -> Result<StreamRead, StreamError> {
        if !consumer.handle.resource.belongs_to(self.id) {
            return Err(StreamError::WrongRun);
        }
        if !matches!(self.state, RunState::Running | RunState::Draining) {
            return Err(StreamError::NotRunning);
        }
        if consumer.finished {
            return Ok(StreamRead::End);
        }
        let limit = self.options.limits.max_stream_item_bytes;
        let resource = self.stream_resource(consumer.handle)?;
        if let Some(pending) = &mut resource.pending {
            let id = StreamNextId {
                handle: consumer.handle,
                sequence: pending.sequence,
            };
            let Some(result) = pending.result.take() else {
                return Ok(StreamRead::Pending(id));
            };
            resource.pending = None;
            let _ = resource.retained.try_set(0);
            return match result {
                Ok(Some(value)) => Ok(StreamRead::Item(value)),
                Ok(None) => {
                    resource.terminal.store(2, Ordering::Relaxed);
                    self.close_stream(consumer)?;
                    Ok(StreamRead::End)
                }
                Err(fault) => {
                    self.close_stream(consumer)?;
                    Ok(StreamRead::Failed(fault))
                }
            };
        }
        let sequence = resource.next.checked_add(1).ok_or(StreamError::Limit)?;
        resource.next = sequence;
        let id = StreamNextId {
            handle: consumer.handle,
            sequence,
        };
        let demand = StreamDemand {
            id,
            closed: resource.closed.clone(),
        };
        // Publish the pending identity before calling trusted code. If it panics,
        // this demand cannot be repeated; run-owned closing still releases it.
        resource.pending = Some(Pending {
            sequence,
            result: None,
        });
        let link = resource.bridge;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if let Some(link) = link {
                return self.read_bridge(link, demand);
            }
            let resource = self.stream_resource(consumer.handle)?;
            resource
                .producer
                .as_mut()
                .ok_or(StreamError::Stale)
                .map(|producer| producer.next(demand))
        }));
        let result = match result {
            Ok(result) => result?,
            Err(payload) => {
                let _ = self.close_stream(consumer);
                std::panic::resume_unwind(payload);
            }
        };
        let resource = self.stream_resource(consumer.handle)?;
        match result {
            Ok(StreamStep::Pending) => Ok(StreamRead::Pending(id)),
            Ok(StreamStep::Item(value)) => {
                if let Err(error) = resource.shape.checked_charge(&value, limit) {
                    let error = match error {
                        orange_model::value::shape::ValueCheckError::InvalidType => {
                            StreamError::Contract
                        }
                        _ => StreamError::Limit,
                    };
                    self.close_stream(consumer)?;
                    return Err(error);
                }
                resource.pending = None;
                Ok(StreamRead::Item(value))
            }
            Ok(StreamStep::End) => {
                resource.terminal.store(2, Ordering::Relaxed);
                self.close_stream(consumer)?;
                Ok(StreamRead::End)
            }
            Err(fault) => {
                self.close_stream(consumer)?;
                if fault.is_valid() {
                    Ok(StreamRead::Failed(fault))
                } else {
                    Err(StreamError::Contract)
                }
            }
        }
    }
    /// Admit one typed item/end/fault. Refusals neither consume the token nor publish
    /// a partial result, so the producer can retry a quota refusal.
    pub fn complete_stream(
        &mut self,
        id: StreamNextId,
        outcome: Result<Option<&Value>, KernelFault>,
    ) -> Result<(), StreamError> {
        if !id.handle.resource.belongs_to(self.id) {
            return Err(StreamError::WrongRun);
        }
        if self.pump_open || !matches!(self.state, RunState::Running | RunState::Draining) {
            return Err(StreamError::NotRunning);
        }
        if self.stream_resource(id.handle)?.bridge.is_some() {
            return Err(StreamError::Contract);
        }
        self.complete_stream_inner(id, outcome)
    }
    fn complete_stream_inner(
        &mut self,
        id: StreamNextId,
        outcome: Result<Option<&Value>, KernelFault>,
    ) -> Result<(), StreamError> {
        if !id.handle.resource.belongs_to(self.id) {
            return Err(StreamError::WrongRun);
        }
        if !matches!(self.state, RunState::Running | RunState::Draining) {
            return Err(StreamError::NotRunning);
        }
        let limit = self.options.limits.max_stream_item_bytes;
        let resource = self.stream_resource(id.handle)?;
        let pending = resource
            .pending
            .as_ref()
            .filter(|p| p.sequence == id.sequence && p.result.is_none())
            .ok_or(StreamError::Stale)?;
        let _ = pending;
        let bytes = match outcome {
            Ok(Some(value)) => {
                resource
                    .shape
                    .checked_charge(value, limit)
                    .map_err(|e| match e {
                        orange_model::value::shape::ValueCheckError::InvalidType => {
                            StreamError::Contract
                        }
                        _ => StreamError::Limit,
                    })?
            }
            Err(fault) if fault.is_valid() => std::mem::size_of::<KernelFault>(),
            Err(_) => return Err(StreamError::Contract),
            Ok(None) => 0,
        };
        if bytes > limit {
            return Err(StreamError::Limit);
        }
        if !resource.retained.try_set(bytes) {
            return Err(StreamError::Limit);
        }
        resource.pending.as_mut().ok_or(StreamError::Stale)?.result =
            Some(outcome.map(|v| v.cloned()));
        self.graph_streams.wake(id);
        self.bridges.wake_source(id.handle.resource.id());
        Ok(())
    }
    /// Idempotent for this consumer; closes/cancels the producer exactly once.
    pub fn close_stream(&mut self, consumer: &mut StreamConsumer) -> Result<(), StreamError> {
        if !consumer.handle.resource.belongs_to(self.id) {
            return Err(StreamError::WrongRun);
        }
        if consumer.finished {
            return Ok(());
        }
        match self.close_resource(consumer.handle.resource) {
            Ok(()) | Err(ResourceError::StaleHandle) => {}
            Err(_) => return Err(StreamError::Stale),
        }
        consumer.finished = true;
        Ok(())
    }
    /// Collects at most `max_items` under run byte/item limits, consuming at most
    /// `budget` reads. The bound is fixed for the lifetime of this collection.
    pub fn collect_stream(
        &mut self,
        consumer: &mut StreamConsumer,
        max_items: usize,
        budget: &mut usize,
    ) -> Result<StreamCollectPoll, StreamError> {
        if self.pump_open || !matches!(self.state, RunState::Running | RunState::Draining) {
            return Err(StreamError::NotRunning);
        }
        self.collect_stream_inner(consumer, max_items, budget)
    }
    fn collect_stream_inner(
        &mut self,
        consumer: &mut StreamConsumer,
        max_items: usize,
        budget: &mut usize,
    ) -> Result<StreamCollectPoll, StreamError> {
        if max_items > self.options.limits.max_stream_collect_items {
            return Err(StreamError::Limit);
        }
        if consumer.finished {
            return Err(StreamError::Stale);
        }
        let aggregate = self.budget.clone();
        let limit = self.options.limits.max_stream_collect_bytes;
        let resource = self.stream_resource(consumer.handle)?;
        let shape = resource.shape.clone();
        let mut collection = match resource.collection.take() {
            Some(collection) => collection,
            None => {
                let mut retained = Tally::new(Some(aggregate), 0);
                let header = orange_model::value::shape::VALUE_CHARGE;
                if limit < header || !retained.try_set(header) {
                    return Err(StreamError::Limit);
                }
                Collection {
                    max: max_items,
                    items: Vec::new(),
                    retained,
                }
            }
        };
        if collection.max != max_items {
            self.stream_resource(consumer.handle)?.collection = Some(collection);
            return Err(StreamError::Contract);
        }
        while *budget > 0 {
            *budget -= 1;
            match self.next_stream_inner(consumer) {
                Ok(StreamRead::Item(value)) => {
                    let bytes = match shape.checked_charge(&value, limit) {
                        Ok(bytes) => bytes,
                        Err(_) => {
                            self.close_stream(consumer)?;
                            return Err(StreamError::Limit);
                        }
                    };
                    let total = collection
                        .retained
                        .get()
                        .checked_add(bytes)
                        .filter(|n| *n <= limit);
                    if collection.items.len() == collection.max
                        || total.is_none_or(|n| !collection.retained.try_set(n))
                        || collection.items.try_reserve(1).is_err()
                    {
                        self.close_stream(consumer)?;
                        return Err(StreamError::Limit);
                    }
                    collection.items.push(value);
                }
                Ok(StreamRead::Pending(id)) => {
                    self.stream_resource(consumer.handle)?.collection = Some(collection);
                    return Ok(StreamCollectPoll::Pending(id));
                }
                Ok(StreamRead::End) => {
                    let mut builder = orange_model::value::ListBuilder::new(Some(&shape));
                    builder
                        .reserve(collection.items.len())
                        .map_err(|_| StreamError::Limit)?;
                    for value in collection.items {
                        builder.push(value).map_err(|_| StreamError::Contract)?;
                    }
                    return Ok(StreamCollectPoll::Complete(Value::List(Arc::new(
                        builder.finish(),
                    ))));
                }
                Ok(StreamRead::Failed(fault)) => return Ok(StreamCollectPoll::Failed(fault)),
                Err(error) => {
                    let _ = self.close_stream(consumer);
                    return Err(error);
                }
            }
        }
        self.stream_resource(consumer.handle)?.collection = Some(collection);
        Ok(StreamCollectPoll::Working)
    }
}
