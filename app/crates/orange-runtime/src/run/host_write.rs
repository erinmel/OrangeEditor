//! A host's own write to a run variable, between two pumps.
use super::{Run, RunError, RunState};
use orange_model::Value;
use orange_model::document::VarScope;
use orange_model::id::VariableId;

impl Run {
    /// Sets a run-scoped variable from outside the graph, as an editor does when the
    /// user changes a value. The run must be running or paused, `id` must name one of
    /// its run variables and `value` must have that variable's type; the state budget
    /// applies as for a graph write.
    ///
    /// Graph work sees the value at its next read. An On Variable Changed entry
    /// watching the variable fires as for a graph write: at once when no graph
    /// segment is in progress, otherwise at that segment's end, coalesced with its own
    /// writes. A paused run takes the value and fires nothing.
    pub fn set_variable(&mut self, id: VariableId, value: Value) -> Result<(), RunError> {
        if !matches!(self.state, RunState::Running | RunState::Paused) {
            return Err(RunError::InvalidState);
        }
        let plan = self.plan.clone().ok_or(RunError::InvalidState)?;
        let index = plan
            .variables()
            .iter()
            .position(|cell| cell.id() == id && cell.scope() == VarScope::Run)
            .ok_or(RunError::InvalidState)?;
        let cell_plan = &plan.variables()[index];
        if !cell_plan.ty().accepts(&value) {
            return Err(RunError::InvalidState);
        }
        let new = cell_plan.charge(&value).ok_or(RunError::InvalidState)?;
        let cell = self.layout.variables[index];
        let old = self.variables[cell]
            .as_ref()
            .and_then(|current| cell_plan.charge(current))
            .unwrap_or(16);
        let bytes = self
            .state_bytes
            .checked_sub(old)
            .and_then(|bytes| bytes.checked_add(new))
            .filter(|bytes| *bytes <= self.options.limits.max_state_bytes)
            .ok_or(RunError::ResourceLimit)?;
        let version = self.versions[index]
            .checked_add(1)
            .ok_or(RunError::ResourceLimit)?;

        let own_segment = self.segment_owner.is_none();
        if own_segment {
            // The write is a segment of its own, caused by nothing in the graph.
            self.end_segment(&plan);
            self.changes.open((0, 0));
        }
        self.state_bytes = bytes;
        self.changes.capture(index, self.variables[cell].as_ref());
        self.variables[cell] = Some(value);
        self.versions[index] = version;
        if own_segment {
            self.end_segment(&plan);
        }
        Ok(())
    }
}
