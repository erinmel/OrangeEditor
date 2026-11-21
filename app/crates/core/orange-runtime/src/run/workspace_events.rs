//! Checked workspace bus attachment and deferred Run event ingestion.
use super::*;
use crate::{
    WorkspaceBus, WorkspaceBusError as Error, WorkspaceDelivery, WorkspaceDeliveryRefused,
    WorkspaceQueue,
};
use orange_model::document::EventScope;
impl Run {
    pub fn attach_workspace_bus(
        &mut self,
        bus: &mut WorkspaceBus,
        queue: WorkspaceQueue,
    ) -> Result<(), Error> {
        if self.pump_open || self.state != RunState::Running {
            return Err(Error::NotRunning);
        }
        if self.workspace_bus.is_some() {
            return Err(Error::Contract);
        }
        let workspace = self
            .workspace
            .as_ref()
            .ok_or(Error::NotAttached)?
            .snapshot
            .id();
        let lease = bus.register(
            self.id,
            workspace,
            self.plan.as_ref().ok_or(Error::NotRunning)?,
            queue,
        )?;
        self.workspace_bus = Some(lease);
        Ok(())
    }
    /// Drain one FIFO delivery, retaining the bus's item/byte leases in the result.
    pub fn take_workspace_delivery(
        &self,
        bus: &mut WorkspaceBus,
    ) -> Result<Option<WorkspaceDelivery>, Error> {
        bus.take(self.workspace_bus.as_ref().ok_or(Error::NotAttached)?)
    }
    /// Admit without executing graph work. A refusal returns the same message.
    pub fn ingest_workspace(
        &mut self,
        delivery: WorkspaceDelivery,
    ) -> Result<EventMetadata, Box<WorkspaceDeliveryRefused>> {
        let result = (|| {
            if self.pump_open || !self.accepts_input() {
                return Err(Error::NotRunning);
            }
            let lease = self.workspace_bus.as_ref().ok_or(Error::NotAttached)?;
            if !Arc::ptr_eq(&lease.state, &delivery.target)
                || delivery.envelope.target_run != self.id
            {
                return Err(Error::WrongOwner);
            }
            if lease.state.is_closed() {
                return Err(Error::Closed);
            }
            let e = delivery.envelope;
            if e.ingestion_seq <= lease.last_ingestion {
                return Err(Error::SequenceNotIncreasing);
            }
            if e.cause_depth == 0 || e.cause_depth > self.options.max_cause_depth {
                return Err(Error::CauseDepth);
            }
            let index = lease
                .state
                .events
                .get(&e.event)
                .filter(|(_, receive)| *receive)
                .map(|(i, _)| *i)
                .ok_or(Error::SchemaMismatch)?;
            let plan = self.plan.as_ref().ok_or(Error::NotRunning)?.clone();
            let metadata = self
                .admit_event(
                    &plan,
                    &delivery.payload,
                    events::EventAdmission {
                        event: index,
                        origin: EventOrigin::Workspace(e.origin()),
                        cause: Some(e.cause),
                        cause_depth: e.cause_depth,
                        source_seq: e.source_seq,
                        wall_ns: None,
                        graph_source: None,
                        tick: None,
                    },
                )
                .map_err(|_| Error::Limit)?;
            self.workspace_bus
                .as_mut()
                .ok_or(Error::NotAttached)?
                .last_ingestion = e.ingestion_seq;
            Ok(metadata)
        })();
        result.map_err(|error| Box::new(WorkspaceDeliveryRefused { error, delivery }))
    }
    pub(super) fn emit_workspace(
        &mut self,
        plan: &ControlPlan,
        activation: &Strand,
        owner: ControlIx,
        event: usize,
        payload: &Value,
        bus: Option<&mut WorkspaceBus>,
    ) -> Result<(), KernelFault> {
        if self.state != RunState::Running {
            return Err(Error::NotRunning.fault());
        }
        if plan.events()[event].scope() != EventScope::Workspace {
            return Err(Error::Contract.fault());
        }
        let depth = activation
            .cause_depth
            .checked_add(1)
            .filter(|n| *n <= self.options.max_cause_depth)
            .ok_or(Error::CauseDepth.fault())?;
        let sequence = self
            .events
            .source_sequences
            .get(owner.index())
            .and_then(|n| n.checked_add(1))
            .ok_or(Error::IdentityExhausted.fault())?;
        let lease = self
            .workspace_bus
            .as_ref()
            .ok_or(Error::NotAttached.fault())?;
        bus.ok_or(Error::NotAttached.fault())?
            .emit(
                lease,
                payload,
                crate::workspace::Emission {
                    event: plan.events()[event].id(),
                    node: plan.nodes()[owner.index()].node(),
                    scope: plan.nodes()[owner.index()].scope(),
                    source_seq: sequence,
                    cause: activation.sequence,
                    depth,
                    now: self.now,
                },
            )
            .map_err(Error::fault)?;
        self.events.source_sequences[owner.index()] = sequence;
        Ok(())
    }
}
