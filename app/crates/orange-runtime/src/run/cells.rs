//! Variable and state cells a step may write, apart from the scheduler.
use super::function::{Frames, find_frame, frame_value};
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

/// The run cells a step may read and write, borrowed apart from the scheduler, regions
/// and trace. Data demand is consumed through it too, since it reads only cells. Every
/// write admits its whole byte charge before it stores, so a rejected write leaves cells,
/// bytes, versions and segment writes unchanged.
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

/// Data consumption reads only cells, frames and the layout, so it runs on `Cells`.
impl Cells<'_> {
    pub(super) fn consume(
        &self,
        plan: &ControlPlan,
        activation: &mut Strand,
        shared: &mut ActivationState,
        root: usize,
        budget: &mut usize,
    ) -> Result<Option<FoldedValue>, RunError> {
        let inputs = plan.inputs_for(root).ok_or(RunError::InvalidState)?;
        self.consume_inputs(plan, activation, shared, root, inputs, budget)
    }
    pub(super) fn consume_inputs(
        &self,
        plan: &ControlPlan,
        activation: &mut Strand,
        shared: &mut ActivationState,
        root: usize,
        inputs: &[usize],
        budget: &mut usize,
    ) -> Result<Option<FoldedValue>, RunError> {
        // Snapshot: a pull's first poll opens one data epoch and copies every host
        // input it may read; later polls resume that epoch, so cell writes by other
        // strands meanwhile cannot change what this demand observes.
        if activation.pull.active.is_none() {
            shared.data.begin_epoch()?;
            shared.pull_owner = Some(activation.id);
            activation.pull.begin(root);
            for &i in inputs {
                let source = &plan.host_sources()[i];
                let value = match source {
                    HostSource::Iteration(owner, projection) => {
                        let frame =
                            IterationFrame::find(activation.iteration_context.as_deref(), *owner)
                                .ok_or(DataError::MissingInput {
                                node: plan.nodes()[owner.index()].node(),
                            })?;
                        Some(FoldedValue::Value(
                            frame.value(*projection).ok_or(RunError::InvalidState)?,
                        ))
                    }
                    HostSource::FrameLocal { scope, variable } => {
                        let frame = find_frame(self.frames, activation.function, *scope)?;
                        let value = frame_value(self.frames, plan, frame, *variable)?.cloned();
                        Some(
                            value
                                .map(FoldedValue::Value)
                                .unwrap_or_else(|| FunctionFrame::uninitialized(plan, i)),
                        )
                    }
                    HostSource::Argument { scope, index } => {
                        let frame = find_frame(self.frames, activation.function, *scope)?;
                        Some(FoldedValue::Value(
                            self.frames
                                .arguments(frame)?
                                .get(*index)
                                .ok_or(RunError::InvalidState)?
                                .clone(),
                        ))
                    }
                    HostSource::State(index) => {
                        Some(match self.layout.state(plan, self.states, shared, *index) {
                            Some(value) => FoldedValue::Value(value.clone()),
                            None => FoldedValue::Fault(ConstantFault {
                                source: plan.states()[*index].id(),
                                fault: KernelFault {
                                    code: "core/UninitializedRegister",
                                    message_key: "core/uninitialized-register",
                                    message: "The register has not been initialized.",
                                },
                            }),
                        })
                    }
                    HostSource::Variable(index) | HostSource::CallLocal(index) => {
                        let var = &plan.variables()[*index];
                        let cell = if matches!(source, HostSource::CallLocal(_)) {
                            // Volatile bodies cannot mutate locals; immutable initial values
                            // implement fresh call-local reads without mutable frame allocation.
                            var.initial()
                        } else if var.scope() == VarScope::Local {
                            shared.locals[self.layout.variables[*index]].as_ref()
                        } else {
                            self.variables[self.layout.variables[*index]].as_ref()
                        };
                        Some(match cell {
                            Some(value) => FoldedValue::Value(value.clone()),
                            None => FoldedValue::Fault(ConstantFault {
                                source: plan.data().nodes()[plan.data().host_inputs()[i].index()]
                                    .node(),
                                fault: KernelFault {
                                    code: "core/UninitializedVariable",
                                    message_key: "core/uninitialized-variable",
                                    message: "The variable has not been initialized.",
                                },
                            }),
                        })
                    }
                    HostSource::Error(_)
                    | HostSource::Index(_)
                    | HostSource::Value(_)
                    | HostSource::Update(_, _)
                    | HostSource::CallOutput(_, _) => {
                        self.layout.outputs[i].and_then(|slot| shared.outputs[slot].clone())
                    }
                };
                activation.pull.inputs.push(value);
            }
        }
        if activation
            .pull
            .active
            .as_ref()
            .is_none_or(|pull| pull.root != root)
        {
            return Err(RunError::InvalidState);
        }
        loop {
            if let Some(index) = activation.pull.active.as_ref().and_then(|pull| pull.input) {
                if *budget == 0 {
                    return Ok(None);
                }
                *budget -= 1;
                let offset = inputs
                    .binary_search(&index)
                    .map_err(|_| RunError::InvalidState)?;
                let value = activation.pull.inputs[offset].clone().ok_or_else(|| {
                    RunError::Data(DataError::MissingInput {
                        node: plan.data().nodes()[plan.data().host_inputs()[index].index()].node(),
                    })
                })?;
                shared.data.set_input_outcome(index, value)?;
                activation
                    .pull
                    .active
                    .as_mut()
                    .ok_or(RunError::InvalidState)?
                    .input = None;
            }
            match shared.data.poll_host_export(root, budget)? {
                HostPoll::Pending => return Ok(None),
                HostPoll::Input(index) => {
                    activation
                        .pull
                        .active
                        .as_mut()
                        .ok_or(RunError::InvalidState)?
                        .input = Some(index)
                }
                HostPoll::Ready(Some(value)) => {
                    return Ok(Some(match value.outcome {
                        Ok(value) => FoldedValue::Value(value.clone()),
                        Err(fault) => FoldedValue::Fault(fault.clone()),
                    }));
                }
                HostPoll::Ready(None) => return Err(RunError::InvalidState),
            }
        }
    }
}
