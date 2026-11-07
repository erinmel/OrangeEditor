use super::*;
const CONTRACT: KernelFault = KernelFault {
    code: "core/InternalFault",
    message_key: "core/stream-contract",
    message: "The stream item violates its checked type.",
};
impl Run {
    fn drive_bridge(&mut self, hub: &mut Hub) {
        if hub.terminal.is_some() {
            return;
        }
        if hub.staged.is_none()
            && !hub.blocked()
            && hub.branches.iter().any(|b| b.live() && b.waiting.is_some())
            && let Some(source) = hub.source.as_mut()
        {
            let outcome = self.next_stream_inner(source);
            match outcome {
                Ok(StreamRead::Pending(id)) => {
                    hub.source_wait = Some(id);
                    return;
                }
                Ok(StreamRead::Item(value)) => {
                    hub.source_wait = None;
                    match hub
                        .shape
                        .checked_charge(&value, self.options.limits.max_stream_item_bytes)
                    {
                        Ok(bytes)
                            if bytes <= hub.options.max_bytes && hub.retained.try_set(bytes) =>
                        {
                            hub.source_sequence = hub.source_sequence.saturating_add(1);
                            hub.staged = Some((value, bytes));
                        }
                        _ => hub.fail(LIMIT),
                    }
                }
                Ok(StreamRead::End) => {
                    hub.source_wait = None;
                    hub.terminal = Some(Ok(()));
                }
                Ok(StreamRead::Failed(fault)) => hub.fail(fault),
                Err(StreamError::Limit) => hub.fail(LIMIT),
                Err(StreamError::Contract) => hub.fail(CONTRACT),
                Err(_) => hub.fail(REVOKED),
            }
        }
        if let Some((value, bytes)) = hub.staged.as_ref().map(|(v, b)| (v.clone(), *b)) {
            match hub.fanout(&value, bytes, &self.budget) {
                Ok(_) => {}
                Err(StreamError::Full) => {}
                Err(StreamError::Overflow) => hub.fail(OVERFLOW),
                Err(_) => hub.fail(LIMIT),
            }
        }
        if matches!(hub.terminal, Some(Err(_)))
            && let Some(source) = hub.source.as_mut()
        {
            // All source handles were created by this Run. A source already
            // retired by its own End/fault is an idempotent close here.
            let _ = self.close_stream(source);
        }
    }
    pub(in crate::run::streams) fn read_bridge(
        &mut self,
        link: BranchLink,
        demand: StreamDemand,
    ) -> Result<Result<StreamStep, KernelFault>, StreamError> {
        self.reconcile_stream_bridges();
        let mut hub = self
            .bridges
            .hubs
            .remove(&link.bridge)
            .ok_or(StreamError::Stale)?;
        let result = (|| {
            let branch = hub
                .branches
                .get_mut(link.index)
                .filter(|b| b.live())
                .ok_or(StreamError::Stale)?;
            branch.waiting = Some(demand.id);
            if branch.queue.is_empty() && hub.terminal.is_none() {
                self.drive_bridge(&mut hub);
            }
            let branch = &mut hub.branches[link.index];
            if let Some(value) = branch.pop() {
                branch.waiting = None;
                return Ok(Ok(StreamStep::Item(value)));
            }
            match hub.terminal {
                Some(Ok(())) => {
                    branch.waiting = None;
                    Ok(Ok(StreamStep::End))
                }
                Some(Err(fault)) => {
                    branch.waiting = None;
                    Ok(Err(fault))
                }
                None => Ok(Ok(StreamStep::Pending)),
            }
        })();
        self.bridges.hubs.insert(link.bridge, hub);
        self.bridges.schedule(link.bridge);
        self.reconcile_stream_bridges();
        result
    }
    /// Core terminal failures use the existing pending enum and safe static fault
    /// envelope without a new payload reservation, so revocation can progress at quota.
    fn finish_bridge_waiter(
        &mut self,
        id: StreamNextId,
        fault: KernelFault,
    ) -> Result<(), StreamError> {
        let resource = self.stream_resource(id.handle)?;
        let pending = resource
            .pending
            .as_mut()
            .filter(|p| p.sequence == id.sequence && p.result.is_none())
            .ok_or(StreamError::Stale)?;
        pending.result = Some(Err(fault));
        self.graph_streams.wake(id);
        self.bridges.wake_source(id.handle.resource.id());
        Ok(())
    }
    pub(in crate::run) fn pump_stream_bridge(&mut self) -> Result<(), RunError> {
        self.reconcile_stream_bridges();
        let Some(id) = self.bridges.ready.pop_front() else {
            return Ok(());
        };
        let Some(mut hub) = self.bridges.hubs.remove(&id) else {
            return Ok(());
        };
        self.drive_bridge(&mut hub);
        let mut deliveries = Vec::with_capacity(2);
        for branch in &mut hub.branches {
            if !branch.live() {
                continue;
            }
            let Some(demand) = branch.waiting else {
                continue;
            };
            let outcome = if let Some(value) = branch.pop() {
                Some(Ok(Some(value)))
            } else {
                hub.terminal.map(|t| t.map(|()| None))
            };
            if let Some(outcome) = outcome {
                branch.waiting = None;
                deliveries.push((demand, outcome));
            }
        }
        self.bridges.hubs.insert(id, hub);
        for (demand, outcome) in deliveries {
            match outcome {
                Ok(value) => self.complete_stream_inner(demand, Ok(value.as_ref())),
                Err(fault) => self.finish_bridge_waiter(demand, fault),
            }
            .map_err(|_| RunError::InvalidState)?;
        }
        self.bridges.schedule(id);
        self.reconcile_stream_bridges();
        Ok(())
    }
    /// Run-owned source ownership survives returned branch scopes. Reaping closes
    /// upstream after the last branch, refunds queues, and never retains history.
    pub(in crate::run) fn reconcile_stream_bridges(&mut self) {
        if self.bridges.reconciling {
            return;
        }
        self.bridges.reconciling = true;
        let mut panic = None;
        loop {
            let ids: Vec<_> = self.bridges.hubs.keys().copied().collect();
            let mut changed = false;
            let mut close = Vec::new();
            for id in ids {
                let mut retire = false;
                if let Some(hub) = self.bridges.hubs.get_mut(&id) {
                    for branch in &mut hub.branches {
                        if !branch.live() && !branch.closed_seen {
                            branch.closed_seen = true;
                            branch.discard();
                            branch.waiting = None;
                            changed = true;
                        }
                    }
                    if hub.branches.iter().all(|b| !b.live()) {
                        retire = true;
                    } else if hub.terminal.is_none()
                        && hub
                            .source
                            .as_ref()
                            .is_some_and(|s| !self.resources.contains(s.handle.resource.id()))
                    {
                        hub.fail(REVOKED);
                        changed = true;
                    }
                }
                if retire {
                    if let Some(hub) = self.bridges.remove(id)
                        && let Some(source) = hub.source
                    {
                        close.push(source.handle.resource);
                    }
                    changed = true;
                } else {
                    self.bridges.schedule(id);
                }
            }
            for source in close {
                if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = self.close_resource(source);
                })) {
                    panic.get_or_insert(payload);
                }
            }
            if !changed {
                break;
            }
        }
        self.bridges.reconciling = false;
        if let Some(payload) = panic
            && !std::thread::panicking()
        {
            std::panic::resume_unwind(payload);
        }
    }
    /// Revoke both branches and any unpublished results. Source tokens become stale.
    pub fn revoke_stream_bridge(&mut self, bridge: StreamBridge) -> Result<(), StreamError> {
        if bridge.run != self.id {
            return Err(StreamError::WrongRun);
        }
        let Some(hub) = self.bridges.hubs.get(&bridge.sequence) else {
            return Ok(());
        };
        let handles: Vec<_> = hub
            .branches
            .iter()
            .filter(|b| b.live())
            .map(|b| b.handle.resource)
            .collect();
        for handle in handles {
            self.close_resource(handle)
                .map_err(|_| StreamError::Stale)?;
        }
        self.reconcile_stream_bridges();
        Ok(())
    }
}
