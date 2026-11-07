//! Direct graph stream values and actor-owned parked consumers.
use super::*;
use crate::run::retained::Hold;
use orange_model::value::HandleId;
mod actor;

/// An unsuccessful resource-result admission returns the producer unclosed.
pub struct EffectStreamRefused {
    pub error: EffectError,
    pub producer: Box<dyn PullStream>,
}
impl std::fmt::Debug for EffectStreamRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectStreamRefused")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::run::streams) struct ConsumerKey {
    activation: u64,
    function: Option<FrameId>,
    owner: ControlIx,
}
pub(in crate::run) enum ReadMode {
    Next {
        boxed: bool,
    },
    Collect {
        max: usize,
    },
    Iterate {
        index: u64,
        parent: Option<Arc<IterationFrame>>,
    },
}
pub(in crate::run) struct PreparedRead {
    pub owner: ControlIx,
    reference: Value,
    mode: ReadMode,
    hold: Option<Hold>,
}
impl PreparedRead {
    pub fn new(
        owner: ControlIx,
        reference: Value,
        mode: ReadMode,
        shared: &ActivationState,
        limit: usize,
    ) -> Result<Self, RunError> {
        Ok(Self {
            owner,
            reference,
            mode,
            hold: Some(Hold::new(shared, 16, 0, limit)?),
        })
    }
}
pub(in crate::run) struct StreamCursor {
    pub owner: ControlIx,
    pub frame: Arc<IterationFrame>,
    pub reference: Value,
    pub next: u64,
}
pub(in crate::run) struct PendingRead {
    pub token: u64,
    pub strand: Strand,
    pub owner: ControlIx,
    work: PreparedRead,
    demand: Option<StreamNextId>,
    revoked: bool,
}
#[derive(Default)]
pub(in crate::run) struct GraphStreams {
    next: u64,
    reads: BTreeMap<u64, PendingRead>,
    strands: BTreeMap<StrandId, u64>,
    waiting: BTreeMap<(HandleId, u64), u64>,
    ready: VecDeque<u64>,
}
impl GraphStreams {
    pub fn len(&self) -> usize {
        self.reads.len()
    }
    pub fn has_ready(&self) -> bool {
        !self.ready.is_empty()
    }
    pub fn clear(&mut self) {
        self.reads.clear();
        self.strands.clear();
        self.waiting.clear();
        self.ready.clear();
    }
    pub fn wake(&mut self, id: StreamNextId) {
        if let Some(&token) = self.waiting.get(&(id.handle.resource.id(), id.sequence))
            && !self.ready.contains(&token)
        {
            self.ready.push_back(token);
        }
    }
    pub fn revoke(&mut self, id: HandleId) {
        for (&token, read) in &mut self.reads {
            if matches!(&read.work.reference,Value::Stream(value) if value.id()==id) {
                read.revoked = true;
                if !self.ready.contains(&token) {
                    self.ready.push_back(token);
                }
            }
        }
    }
    fn insert(&mut self, read: PendingRead) {
        let token = read.token;
        if let Some(id) = read.demand {
            self.waiting
                .insert((id.handle.resource.id(), id.sequence), token);
        } else {
            self.ready.push_back(token);
        }
        self.strands.insert(read.strand.id, token);
        self.reads.insert(token, read);
    }
    fn remove(&mut self, token: u64) -> Option<PendingRead> {
        let read = self.reads.remove(&token)?;
        self.strands.remove(&read.strand.id);
        if let Some(id) = read.demand {
            self.waiting.remove(&(id.handle.resource.id(), id.sequence));
        }
        self.ready.retain(|id| *id != token);
        Some(read)
    }
}
impl Run {
    /// Expose a host stream without claiming or pulling it. A graph consumer must
    /// still claim its affine authority; cloning this alias grants no extra claim.
    pub fn stream_value(&mut self, handle: StreamHandle) -> Result<Value, StreamError> {
        let terminal = self.stream_resource(handle)?.terminal.clone();
        self.issue_resource_value(handle.resource, Some(terminal))
            .map_err(|_| StreamError::Limit)
    }
    /// Admit an Async's sole direct stream result, owned by its dynamic region.
    /// Refusal always returns the original producer unclosed; a valid pending
    /// effect can retry a corrected type/quota refusal.
    pub fn complete_effect_stream(
        &mut self,
        id: EffectId,
        item: &TypeExpr,
        producer: Box<dyn PullStream>,
    ) -> Result<(), EffectStreamRefused> {
        let refuse = |error, producer| Err(EffectStreamRefused { error, producer });
        let region = match self.effect_reference_target(id, item, true) {
            Ok(region) => region,
            Err(error) => return refuse(error, producer),
        };
        let handle = match self.insert_stream(item, producer) {
            Ok(handle) => handle,
            Err(refused) => {
                return refuse(
                    if refused.error == StreamError::Contract {
                        EffectError::Contract
                    } else {
                        EffectError::Limit
                    },
                    refused.producer,
                );
            }
        };
        let result = (|| {
            self.resources
                .set_owner(handle.resource.id(), Some(region))
                .map_err(|_| EffectError::Contract)?;
            let value = self.stream_value(handle).map_err(|_| EffectError::Limit)?;
            self.complete_effect(id, Ok(&[value]))
        })();
        match result {
            Ok(()) => Ok(()),
            Err(error) => {
                let resource = self
                    .resources
                    .remove(handle.resource.id())
                    .unwrap_or_else(|_| unreachable!("unpublished stream remains live"));
                let mut resource: Box<dyn std::any::Any> = resource;
                let producer = resource
                    .downcast_mut::<StreamResource>()
                    .and_then(|r| r.producer.take())
                    .unwrap_or_else(|| unreachable!("only a StreamResource was inserted"));
                refuse(error, producer)
            }
        }
    }
    pub(in crate::run) fn park_stream_read(
        &mut self,
        strand: Strand,
        work: PreparedRead,
    ) -> Result<(), RunError> {
        let token = self
            .graph_streams
            .next
            .checked_add(1)
            .ok_or(RunError::ResourceLimit)?;
        self.graph_streams.next = token;
        self.graph_streams.insert(PendingRead {
            token,
            owner: work.owner,
            strand,
            work,
            demand: None,
            revoked: false,
        });
        Ok(())
    }
    pub(in crate::run) fn take_stream_read_for_cancel(
        &mut self,
        strand: StrandId,
    ) -> Option<PendingRead> {
        let token = *self.graph_streams.strands.get(&strand)?;
        self.graph_streams.remove(token)
    }
    pub(in crate::run) fn cancel_stream_read(
        &mut self,
        read: &PendingRead,
    ) -> Result<(), RunError> {
        self.close_graph_resource(&read.work.reference, read.strand.region)
            .map_err(|_| RunError::InvalidState)
    }
}
fn stream_fault(error: StreamError) -> KernelFault {
    KernelFault {
        code: match error {
            StreamError::Limit => "core/LimitExceeded",
            StreamError::Contract => "core/InternalFault",
            StreamError::NotRunning => "core/RunStopping",
            _ => "core/StaleHandle",
        },
        message_key: error.message_key(),
        message: "The stream is unavailable or exceeds its configured bounds.",
    }
}
