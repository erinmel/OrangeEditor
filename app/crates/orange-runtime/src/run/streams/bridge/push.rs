use super::*;
impl Run {
    /// Create a bounded push source and one affine stream without producing data.
    pub fn open_push_stream(
        &mut self,
        item: &TypeExpr,
        options: StreamBufferOptions,
    ) -> Result<PushStream, StreamError> {
        if self.pump_open || self.state != RunState::Running {
            return Err(StreamError::NotRunning);
        }
        self.check_buffer_options(options)?;
        let shape = self
            .plan
            .as_ref()
            .and_then(|p| ValueShape::of_in(item, p.data().snapshot().types().as_ref()))
            .ok_or(StreamError::Contract)?;
        if self.resources.len() >= self.options.limits.max_resources {
            return Err(StreamError::Limit);
        }
        let id = self.bridges.next.checked_add(1).ok_or(StreamError::Limit)?;
        let branch = self.bridge_branch(id, 0, item, &shape, None)?;
        let bridge = StreamBridge {
            run: self.id,
            sequence: id,
        };
        let result = PushStream {
            source: PushStreamSource {
                bridge,
                closed: branch.closed.clone(),
            },
            stream: branch.handle,
        };
        self.bridges.hubs.insert(
            id,
            Hub {
                item: item.clone(),
                shape,
                options,
                source: None,
                source_wait: None,
                source_sequence: 0,
                staged: None,
                retained: Tally::new(Some(self.budget.clone()), 0),
                terminal: None,
                branches: vec![branch],
                depth: 0,
            },
        );
        self.bridges.next = id;
        Ok(result)
    }
    fn check_push_source(
        &self,
        source: &PushStreamSource,
        admitting: bool,
    ) -> Result<(), StreamError> {
        if source.bridge.run != self.id {
            return Err(StreamError::WrongRun);
        }
        if self.pump_open
            || !matches!(self.state, RunState::Running | RunState::Draining)
            || (admitting && self.state != RunState::Running)
        {
            return Err(StreamError::NotRunning);
        }
        if source.is_closed() {
            return Err(StreamError::Stale);
        }
        let hub = self
            .bridges
            .hubs
            .get(&source.bridge.sequence)
            .ok_or(StreamError::Stale)?;
        if hub.source.is_some() || hub.terminal.is_some() {
            return Err(StreamError::Stale);
        }
        Ok(())
    }
    /// Admit one typed item in host call order. A Full/type/quota refusal changes
    /// no queue or sequence, so the caller can correct/retry the same sequence.
    /// Deliberate drop accepts the sequence; Fault overflow terminates the bridge.
    pub fn push_stream(
        &mut self,
        source: &PushStreamSource,
        sequence: u64,
        value: &Value,
    ) -> Result<PushAdmission, StreamError> {
        self.check_push_source(source, true)?;
        self.reconcile_stream_bridges();
        let id = source.bridge.sequence;
        let hub = self.bridges.hubs.get_mut(&id).ok_or(StreamError::Stale)?;
        if sequence == 0 || sequence <= hub.source_sequence {
            return Err(StreamError::Sequence);
        }
        let bytes = hub
            .shape
            .checked_charge(
                value,
                self.options
                    .limits
                    .max_stream_item_bytes
                    .min(hub.options.max_bytes),
            )
            .map_err(|error| {
                if error == orange_model::value::shape::ValueCheckError::InvalidType {
                    StreamError::Contract
                } else {
                    StreamError::Limit
                }
            })?;
        let result = hub.fanout(value, bytes, &self.budget);
        match result {
            Ok(_) => hub.source_sequence = sequence,
            Err(StreamError::Overflow) => hub.fail(OVERFLOW),
            _ => {}
        }
        self.bridges.schedule(id);
        result
    }
    /// Stop admission and let the consumer drain accepted items before End.
    pub fn finish_push_stream(&mut self, source: &PushStreamSource) -> Result<(), StreamError> {
        self.check_push_source(source, false)?;
        let id = source.bridge.sequence;
        self.bridges
            .hubs
            .get_mut(&id)
            .ok_or(StreamError::Stale)?
            .terminal = Some(Ok(()));
        self.bridges.schedule(id);
        Ok(())
    }
    /// Stop admission, discard unread queues and wake a safe producer fault.
    pub fn fail_push_stream(
        &mut self,
        source: &PushStreamSource,
        fault: KernelFault,
    ) -> Result<(), StreamError> {
        self.check_push_source(source, false)?;
        if !fault.is_valid() {
            return Err(StreamError::Contract);
        }
        let id = source.bridge.sequence;
        self.bridges
            .hubs
            .get_mut(&id)
            .ok_or(StreamError::Stale)?
            .fail(fault);
        self.bridges.schedule(id);
        Ok(())
    }
}
