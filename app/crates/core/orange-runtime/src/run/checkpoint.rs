//! Persistent variables bootstrap fresh actor state; no existing Run is mutated.
use super::*;
use crate::{CheckpointError as Error, CheckpointLimits, CheckpointScope, ValueCheckpoint};
impl Run {
    pub fn checkpoint(
        &self,
        document: &GraphDocument,
        selection: &[VariableId],
        limits: CheckpointLimits,
    ) -> Result<ValueCheckpoint, Error> {
        if self.state != RunState::Stopped
            || self.pump_open
            || self.last_error.is_some()
            || self.cleanup_truncated != 0
        {
            return Err(Error::Boundary);
        }
        let plan = self.plan.as_ref().ok_or(Error::Boundary)?;
        if !plan.data().is_current(document, plan.data().snapshot()) {
            return Err(Error::Incompatible);
        }
        ValueCheckpoint::capture(
            document,
            plan.data().snapshot(),
            CheckpointScope::Run,
            selection,
            limits,
            |id| {
                let (index, _) = plan
                    .variables()
                    .iter()
                    .enumerate()
                    .find(|(_, v)| v.id() == id && v.scope() == VarScope::Run)
                    .ok_or(Error::Selection)?;
                Ok(self.variables[self.layout.variables[index]].as_ref())
            },
        )
    }
    pub fn start_from_checkpoint(
        plan: Arc<ControlPlan>,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        seed: u64,
        options: RunOptions,
        checkpoint: &ValueCheckpoint,
    ) -> Result<Self, RunError> {
        Self::restore_start(plan, document, registry, seed, options, (None, checkpoint))
    }
    pub fn start_in_workspace_from_checkpoint(
        plan: Arc<ControlPlan>,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        seed: u64,
        options: RunOptions,
        snapshot: &crate::WorkspaceSnapshot,
        checkpoint: &ValueCheckpoint,
    ) -> Result<Self, RunError> {
        snapshot
            .check_schema(document, registry)
            .map_err(|_| Error::Incompatible)?;
        Self::restore_start(
            plan,
            document,
            registry,
            seed,
            options,
            (Some(snapshot), checkpoint),
        )
    }
    fn restore_start(
        plan: Arc<ControlPlan>,
        document: &GraphDocument,
        registry: &RegistrySnapshot,
        seed: u64,
        options: RunOptions,
        source: (Option<&crate::WorkspaceSnapshot>, &ValueCheckpoint),
    ) -> Result<Self, RunError> {
        if !plan.is_current(document, registry) {
            return Err(Error::Incompatible.into());
        }
        let values = source.1.restore(document, registry, CheckpointScope::Run)?;
        if values.keys().any(|id| {
            !plan
                .variables()
                .iter()
                .any(|v| v.id() == *id && v.scope() == VarScope::Run)
        }) {
            return Err(Error::Selection.into());
        }
        Self::start_workspace(
            plan,
            document,
            registry,
            seed,
            options,
            source.0,
            Some(&values),
        )
    }
}
