//! Variable and state cells a step may write, apart from the scheduler.
use super::function::{Frames, find_frame};
use super::*;

/// Ids the current actor segment wrote, each once. These are write intents,
/// not Eq-based change notifications: storing an equal value still counts.
#[derive(Default)]
pub(super) struct SegmentWrites {
    dirty: Vec<bool>,
    indices: Vec<usize>,
    variables: Vec<VariableId>,
    dirty_states: Vec<bool>,
    state_indices: Vec<usize>,
    states: Vec<NodeId>,
}
impl SegmentWrites {
    pub(super) fn new(variables: usize, states: usize) -> Self {
        Self {
            dirty: vec![false; variables],
            dirty_states: vec![false; states],
            ..Self::default()
        }
    }
    /// A new segment owner starts with no writes.
    pub(super) fn open(&mut self) {
        for index in self.indices.drain(..) {
            self.dirty[index] = false;
        }
        self.variables.clear();
        for index in self.state_indices.drain(..) {
            self.dirty_states[index] = false;
        }
        self.states.clear();
    }
    pub(super) fn variable(&mut self, index: usize, id: VariableId) {
        if !self.dirty[index] {
            self.dirty[index] = true;
            self.indices.push(index);
            self.variables.push(id);
        }
    }
    /// `source` is the dense state source, so repeated call sites mark once.
    fn state(&mut self, source: usize, id: NodeId) {
        if !self.dirty_states[source] {
            self.dirty_states[source] = true;
            self.state_indices.push(source);
            self.states.push(id);
        }
    }
    pub(super) fn variables(&self) -> &[VariableId] {
        &self.variables
    }
    pub(super) fn states(&self) -> &[NodeId] {
        &self.states
    }
}

/// The run cells a step may write, borrowed apart from the scheduler, regions
/// and trace. Every write admits its whole byte charge before it stores, so a
/// rejected write leaves cells, bytes, versions and segment writes unchanged.
pub(super) struct Cells<'r> {
    pub layout: &'r StateLayout,
    pub limit: usize,
    pub variables: &'r mut [Option<Value>],
    pub states: &'r mut [Option<Value>],
    /// Retained bytes of run-scope variables and state.
    pub bytes: &'r mut usize,
    pub versions: &'r mut [u64],
    pub frames: &'r mut Frames,
    pub segment: &'r mut SegmentWrites,
}
impl Run {
    pub(super) fn cells(&mut self) -> Cells<'_> {
        Cells {
            layout: &self.layout,
            limit: self.options.limits.max_state_bytes,
            variables: &mut self.variables,
            states: &mut self.states,
            bytes: &mut self.state_bytes,
            versions: &mut self.versions,
            frames: &mut self.frames,
            segment: &mut self.segment,
        }
    }
}
impl Cells<'_> {
    /// Gate, Do Once, Flip Flop, Register and Set. `value` is the consumed
    /// demand: Register's store and Set have one, the others act without.
    pub(super) fn step(
        &mut self,
        plan: &ControlPlan,
        target: ExecTarget,
        shared: &mut ActivationState,
        function: Option<FrameId>,
        value: Option<Value>,
    ) -> Result<Step, RunError> {
        let node = &plan.nodes()[target.index()];
        let contract = DataError::KernelContract { node: node.node() };
        Ok(match (node.operation(), value) {
            (ControlOperation::Gate { state }, None) => {
                let Some(Value::Bool(open)) = self.layout.state(plan, self.states, shared, *state)
                else {
                    return Err(contract.into());
                };
                let open = *open;
                let next = match target.input() {
                    ExecInput::Enter if open => {
                        return Ok((node.then(), ControlOutcome::Then, None));
                    }
                    ExecInput::Enter => return Ok((None, ControlOutcome::Suppressed, None)),
                    ExecInput::Open => true,
                    ExecInput::Close => false,
                    ExecInput::Toggle => !open,
                    _ => return Err(RunError::InvalidState),
                };
                self.write_state(plan, shared, *state, Some(Value::Bool(next)))?;
                (None, ControlOutcome::StateChanged, None)
            }
            (ControlOperation::DoOnce { state }, None) => {
                if target.input() == ExecInput::Reset {
                    self.write_state(plan, shared, *state, Some(Value::Bool(false)))?;
                    (None, ControlOutcome::StateChanged, None)
                } else if matches!(
                    self.layout.state(plan, self.states, shared, *state),
                    Some(Value::Bool(false))
                ) {
                    self.write_state(plan, shared, *state, Some(Value::Bool(true)))?;
                    (node.then(), ControlOutcome::Then, None)
                } else {
                    (None, ControlOutcome::Suppressed, None)
                }
            }
            (ControlOperation::FlipFlop { state, a, b }, None) => {
                let Some(Value::Bool(previous)) =
                    self.layout.state(plan, self.states, shared, *state)
                else {
                    return Err(contract.into());
                };
                let select_a = !previous;
                self.write_state(plan, shared, *state, Some(Value::Bool(select_a)))?;
                if select_a {
                    (*a, ControlOutcome::FlipA, None)
                } else {
                    (*b, ControlOutcome::FlipB, None)
                }
            }
            // Reset demands nothing; a store consumed `next`.
            (ControlOperation::Register { state, .. }, value) => {
                let value = value.or_else(|| plan.states()[*state].initial().cloned());
                self.write_state(plan, shared, *state, value)?;
                (node.then(), ControlOutcome::Then, None)
            }
            (ControlOperation::Set { variable, .. }, Some(value)) => {
                self.set_variable(plan, shared, function, node, *variable, value)?;
                (node.then(), ControlOutcome::Then, None)
            }
            _ => return Err(RunError::InvalidState),
        })
    }
    pub(super) fn write_state(
        &mut self,
        plan: &ControlPlan,
        shared: &mut ActivationState,
        index: usize,
        value: Option<Value>,
    ) -> Result<(), RunError> {
        let metadata = &plan.states()[index];
        let (store, retained) = if metadata.scope() == VarScope::Local {
            (shared.states.as_mut_slice(), &mut shared.local_bytes)
        } else {
            (&mut *self.states, &mut *self.bytes)
        };
        let cell = self.layout.states[index];
        let old = store[cell]
            .as_ref()
            .and_then(|v| metadata.charge(v))
            .unwrap_or(16);
        let new = match &value {
            Some(value) => metadata.charge(value).ok_or(DataError::KernelContract {
                node: metadata.id(),
            })?,
            None => 16,
        };
        let bytes = retained
            .checked_sub(old)
            .and_then(|b| b.checked_add(new))
            .filter(|b| {
                b.checked_add(if metadata.scope() == VarScope::Local {
                    shared
                        .retained_bytes
                        .saturating_add(shared.owned_bytes.load(Ordering::Relaxed))
                } else {
                    0
                })
                .is_some_and(|total| total <= self.limit)
            })
            .ok_or(RunError::ResourceLimit)?;
        *retained = bytes;
        store[cell] = value;
        self.segment
            .state(self.layout.state_sources[index], metadata.id());
        Ok(())
    }
    /// Set: a function-scope local writes its frame; others the activation or run cell.
    pub(super) fn set_variable(
        &mut self,
        plan: &ControlPlan,
        shared: &mut ActivationState,
        function: Option<FrameId>,
        node: &ControlNode,
        variable: usize,
        value: Value,
    ) -> Result<(), RunError> {
        let var = &plan.variables()[variable];
        if var.scope() == VarScope::Local && node.scope() != 0 {
            let frame = find_frame(self.frames, function, node.scope())?;
            return self.write_frame_local(plan, shared, frame, variable, value);
        }
        let (store, retained) = if var.scope() == VarScope::Local {
            (shared.locals.as_mut_slice(), &mut shared.local_bytes)
        } else {
            (&mut *self.variables, &mut *self.bytes)
        };
        let cell = self.layout.variables[variable];
        let old = store[cell]
            .as_ref()
            .and_then(|v| var.charge(v))
            .unwrap_or(16);
        let new = var
            .charge(&value)
            .ok_or(DataError::KernelContract { node: node.node() })?;
        let bytes = retained
            .checked_sub(old)
            .and_then(|b| b.checked_add(new))
            .filter(|b| {
                b.checked_add(if var.scope() == VarScope::Local {
                    shared
                        .retained_bytes
                        .saturating_add(shared.owned_bytes.load(Ordering::Relaxed))
                } else {
                    0
                })
                .is_some_and(|total| total <= self.limit)
            })
            .ok_or(RunError::ResourceLimit)?;
        let version = self.versions[variable]
            .checked_add(1)
            .ok_or(RunError::ResourceLimit)?;
        *retained = bytes;
        store[cell] = Some(value);
        self.versions[variable] = version;
        self.segment.variable(variable, var.id());
        Ok(())
    }
}
