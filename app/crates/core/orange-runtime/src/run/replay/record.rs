use super::*;
impl RecordedRun {
    pub fn pump(&mut self, budget: usize) -> Result<RunProgress, ReplayError> {
        let slot = self.session()?.begin(Entry::Pump {
            budget,
            result: None,
        })?;
        let result = self.run.pump(budget);
        self.session()?.set(
            slot,
            Entry::Pump {
                budget,
                result: Some(result),
            },
        )?;
        Ok(result?)
    }
    pub fn advance_to(&mut self, now: u64) -> Result<(), ReplayError> {
        let slot = self
            .session()?
            .begin(Entry::Advance { now, result: None })?;
        let result = self.run.advance_to(now);
        self.session()?.set(
            slot,
            Entry::Advance {
                now,
                result: Some(result),
            },
        )?;
        Ok(result?)
    }
    pub fn request_stop(&mut self, mode: StopMode) -> Result<(), ReplayError> {
        let slot = self.session()?.begin(Entry::Stop { mode, result: None })?;
        let result = self.run.request_stop(mode);
        self.session()?.set(
            slot,
            Entry::Stop {
                mode,
                result: Some(result),
            },
        )?;
        Ok(result?)
    }
    /// Abandon the recording and request immediate stop independently of journal
    /// capacity. The host drives any remaining bounded cleanup on the returned Run.
    pub fn abort(mut self) -> Result<Run, RunError> {
        self.run.replay = None;
        if !matches!(self.run.state(), RunState::Stopped | RunState::Disposed) {
            self.run.stop()?;
        }
        Ok(self.run)
    }
    pub fn register_event_source(
        &mut self,
        event: EventDeclId,
    ) -> Result<RunEventSource, ReplayError> {
        let slot = self.session()?.begin(Entry::Register {
            event,
            result: None,
        })?;
        let result = self.run.register_event_source(event);
        self.session()?.set(
            slot,
            Entry::Register {
                event,
                result: Some(result.map(|s| s.id())),
            },
        )?;
        Ok(result?)
    }
    pub fn revoke_event_source(&mut self, token: &RunEventSource) -> Result<bool, ReplayError> {
        let source = Source::of(*token, self.run.id());
        let slot = self.session()?.begin(Entry::Revoke {
            source,
            result: None,
        })?;
        let result = self.run.revoke_event_source(token);
        self.session()?.set(
            slot,
            Entry::Revoke {
                source,
                result: Some(result),
            },
        )?;
        Ok(result?)
    }
    pub fn ingest_event(
        &mut self,
        token: &RunEventSource,
        sequence: u64,
        payload: &Value,
        wall: Option<i64>,
    ) -> Result<EventMetadata, ReplayError> {
        let source = Source::of(*token, self.run.id());
        let empty = ReplayPayloadRef {
            namespace: 0,
            key: 0,
        };
        let slot = self.session()?.begin(Entry::Ingest {
            source,
            sequence,
            payload: empty,
            wall,
            result: None,
        })?;
        let reference = self.session()?.capture(std::slice::from_ref(payload))?;
        let result = self.run.ingest_event(token, sequence, payload, wall);
        self.session()?.set(
            slot,
            Entry::Ingest {
                source,
                sequence,
                payload: reference,
                wall,
                result: Some(result),
            },
        )?;
        Ok(result?)
    }
    pub fn trigger(
        &mut self,
        node: NodeId,
        payload: Option<&Value>,
        wall: Option<i64>,
    ) -> Result<EventMetadata, ReplayError> {
        let slot = self.session()?.begin(Entry::Trigger {
            node,
            payload: None,
            wall,
            result: None,
        })?;
        let reference = payload
            .map(|v| self.session()?.capture(std::slice::from_ref(v)))
            .transpose()?;
        let result = self.run.trigger(node, payload, wall);
        self.session()?.set(
            slot,
            Entry::Trigger {
                node,
                payload: reference,
                wall,
                result: Some(result),
            },
        )?;
        Ok(result?)
    }
    pub fn take_effect(&mut self) -> Result<Option<EffectRequest>, ReplayError> {
        let slot = self.session()?.begin(Entry::Take { result: None })?;
        let result = self.run.take_effect();
        let saved = result.as_ref().map(|r| (r.id.sequence, r.node, r.route));
        self.session()?.set(
            slot,
            Entry::Take {
                result: Some(saved),
            },
        )?;
        Ok(result)
    }
    pub fn complete_effect(
        &mut self,
        id: EffectId,
        outcome: Result<&[Value], KernelFault>,
    ) -> Result<(), ReplayError> {
        // Reserve first, then capture. An archive refusal invalidates this
        // recording; no completion has been admitted to the actor yet.
        const SAFE: KernelFault = KernelFault {
            code: orange_compiler::FaultText::Static("core/InternalFault"),
            message_key: orange_compiler::FaultText::Static("core/replay-incomplete"),
            message: "The recording is incomplete.",
        };
        let foreign = id.run != self.run.id();
        let slot = self.session()?.begin(Entry::Complete {
            sequence: id.sequence,
            foreign,
            outcome: Err(SAFE),
            result: None,
        })?;
        let saved = match &outcome {
            Ok(values) => Ok(self.session()?.capture(values)?),
            Err(fault) if fault.is_valid() => Err(fault.clone()),
            Err(_) => {
                self.session()?.failed = true;
                return Err(ReplayError::Effect(EffectError::Contract));
            }
        };
        let result = self.run.complete_effect(id, outcome);
        self.session()?.set(
            slot,
            Entry::Complete {
                sequence: id.sequence,
                foreign,
                outcome: saved,
                result: Some(result),
            },
        )?;
        Ok(result?)
    }
}
