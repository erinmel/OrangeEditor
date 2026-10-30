//! Actor-owned function invocations. Regions keep frames alive through cancellation cleanup.
use super::cells::Cells;
use super::retained::Hold;
use super::*;

pub(super) struct FunctionFrame {
    scope: usize,
    parent: Option<FrameId>,
    pub arguments: Vec<Value>,
    locals: Vec<LocalCell>,
    hold: Hold,
}
/// One frame-local cell and its write version.
struct LocalCell {
    value: Option<Value>,
    version: u64,
}
/// A frame's argument and local buffers. Retired frames return them empty to
/// `Run::spare_frames`, so a steady call loop reuses them instead of allocating.
#[derive(Default)]
pub(super) struct FrameStorage {
    arguments: Vec<Value>,
    locals: Vec<LocalCell>,
}
/// Enough spare storage for typical call depth; deeper recursion allocates.
const SPARE_FRAMES: usize = 16;
pub(super) struct PendingCall {
    owner: ControlIx,
    scope: usize,
    storage: FrameStorage,
}
pub(super) enum FunctionReturn {
    Values {
        values: Vec<Value>,
        hold: Option<Hold>,
    },
    Fault(ConstantFault),
}
impl FunctionReturn {
    pub fn empty() -> Self {
        Self::Values {
            values: Vec::new(),
            hold: None,
        }
    }
}
impl FunctionFrame {
    pub fn uninitialized(plan: &ControlPlan, input: usize) -> FoldedValue {
        FoldedValue::Fault(ConstantFault {
            source: plan.data().nodes()[plan.data().host_inputs()[input].index()].node(),
            fault: KernelFault {
                code: "core/UninitializedVariable",
                message_key: "core/uninitialized-variable",
                message: "The variable has not been initialized.",
            },
        })
    }
}
pub(super) struct FrameWriteSpec<'a> {
    pub frame: FrameId,
    pub variable: usize,
    pub value: &'a Value,
    pub retained: usize,
    pub write: bool,
}
pub(super) struct FrameWrite {
    slot: usize,
    hold: Hold,
    version: u64,
}
pub(super) type Frames = BTreeMap<FrameId, FunctionFrame>;
/// The nearest frame of `scope` on the strand's lexical frame chain.
pub(super) fn find_frame(
    frames: &Frames,
    mut id: Option<FrameId>,
    scope: usize,
) -> Result<FrameId, RunError> {
    while let Some(current) = id {
        let frame = frames.get(&current).ok_or(RunError::InvalidState)?;
        if frame.scope == scope {
            return Ok(current);
        }
        id = frame.parent;
    }
    Err(RunError::InvalidState)
}
/// One frame lookup, then the local's slot in its scope's sorted table.
fn frame_slot<'a>(
    frames: &'a Frames,
    plan: &ControlPlan,
    frame: FrameId,
    variable: usize,
) -> Result<(&'a FunctionFrame, usize), RunError> {
    let frame = frames.get(&frame).ok_or(RunError::InvalidState)?;
    let slot = plan
        .function(frame.scope)
        .ok_or(RunError::InvalidState)?
        .locals()
        .binary_search(&variable)
        .map_err(|_| RunError::InvalidState)?;
    Ok((frame, slot))
}
pub(super) fn frame_value<'a>(
    frames: &'a Frames,
    plan: &ControlPlan,
    frame: FrameId,
    variable: usize,
) -> Result<Option<&'a Value>, RunError> {
    let (frame, slot) = frame_slot(frames, plan, frame, variable)?;
    Ok(frame.locals[slot].value.as_ref())
}
/// A frame-local write is prepared (charged, hold reserved, version bumped)
/// before any store, then committed infallibly; a rejected write changes nothing.
impl Cells<'_> {
    pub(super) fn prepare_frame_write(
        &self,
        plan: &ControlPlan,
        shared: &ActivationState,
        spec: FrameWriteSpec<'_>,
    ) -> Result<FrameWrite, RunError> {
        let FrameWriteSpec {
            frame,
            variable,
            value,
            retained,
            write,
        } = spec;
        let (frame, slot) = frame_slot(self.frames, plan, frame, variable)?;
        let metadata = &plan.variables()[variable];
        let old = frame.locals[slot]
            .value
            .as_ref()
            .and_then(|v| metadata.charge(v))
            .unwrap_or(16);
        let new = metadata
            .checked_charge(value, self.limit)
            .map_err(|_| RunError::ResourceLimit)?;
        let bytes = frame
            .hold
            .bytes
            .checked_sub(old)
            .and_then(|n| n.checked_add(new))
            .ok_or(RunError::ResourceLimit)?;
        let hold = Hold::with_retained(shared, bytes, frame.hold.bytes, retained, self.limit)?;
        let version = if write {
            frame.locals[slot]
                .version
                .checked_add(1)
                .ok_or(RunError::ResourceLimit)?
        } else {
            frame.locals[slot].version
        };
        Ok(FrameWrite {
            slot,
            hold,
            version,
        })
    }
    pub(super) fn commit_frame_write(
        &mut self,
        plan: &ControlPlan,
        frame: FrameId,
        variable: usize,
        value: Value,
        reservation: FrameWrite,
    ) {
        let FrameWrite {
            slot,
            hold,
            version,
        } = reservation;
        #[expect(
            clippy::expect_used,
            reason = "`prepare_frame_write` found this frame and nothing ran in between"
        )]
        let frame = self
            .frames
            .get_mut(&frame)
            .expect("reserved actor-owned function frame");
        frame.hold = hold;
        frame.locals[slot] = LocalCell {
            value: Some(value),
            version,
        };
        self.segment
            .variable(variable, plan.variables()[variable].id());
    }
    pub(super) fn write_frame_local(
        &mut self,
        plan: &ControlPlan,
        shared: &ActivationState,
        frame: FrameId,
        variable: usize,
        value: Value,
    ) -> Result<(), RunError> {
        let reservation = self.prepare_frame_write(
            plan,
            shared,
            FrameWriteSpec {
                frame,
                variable,
                value: &value,
                retained: shared.retained_bytes,
                write: true,
            },
        )?;
        self.commit_frame_write(plan, frame, variable, value, reservation);
        Ok(())
    }
}
impl Run {
    pub(super) fn retire_frame(&mut self, id: FrameId, activation: u64) -> Result<(), RunError> {
        let frame = self.frames.remove(&id).ok_or(RunError::InvalidState)?;
        let plan = self.plan.as_ref().ok_or(RunError::InvalidState)?;
        let layout = plan.function(frame.scope).ok_or(RunError::InvalidState)?;
        let shared = self
            .activations
            .get_mut(activation as usize)
            .and_then(Option::as_mut)
            .ok_or(RunError::InvalidState)?;
        let mut removed = 0usize;
        for &input in layout.projections() {
            let cell = self.layout.outputs[input].ok_or(RunError::InvalidState)?;
            if matches!(
                plan.host_sources()[input],
                HostSource::Value(_) | HostSource::Update(_, _) | HostSource::CallOutput(_, _)
            ) && let Some(FoldedValue::Value(value)) = &shared.outputs[cell]
            {
                let slot =
                    plan.data().nodes()[plan.data().host_inputs()[input].index()].outputs()[0];
                removed = removed
                    .checked_add(
                        plan.data()
                            .value_charge(slot, value)
                            .ok_or(RunError::InvalidState)?,
                    )
                    .ok_or(RunError::ResourceLimit)?;
            }
        }
        let retained = shared
            .retained_bytes
            .checked_sub(removed)
            .ok_or(RunError::InvalidState)?;
        for &input in layout.projections() {
            shared.outputs[self.layout.outputs[input].ok_or(RunError::InvalidState)?] = None;
        }
        shared.retained_bytes = retained;
        let FunctionFrame {
            mut arguments,
            mut locals,
            ..
        } = frame;
        if self.spare_frames.len() < SPARE_FRAMES {
            arguments.clear();
            locals.clear();
            self.spare_frames.push(FrameStorage { arguments, locals });
        }
        Ok(())
    }
    pub(super) fn execute_function(
        &mut self,
        plan: &ControlPlan,
        strand: &mut Strand,
        shared: &mut ActivationState,
        owner: ControlIx,
        budget: &mut usize,
    ) -> Result<(), RunError> {
        let node = &plan.nodes()[owner.index()];
        let (demand, scope) = match node.operation() {
            ControlOperation::Call { scope, demand } => (demand, Some(*scope)),
            ControlOperation::Return { demand } => (demand, None),
            _ => return Err(RunError::InvalidState),
        };
        if scope.is_some() && strand.pull.active.is_none() {
            Self::publish_call_results(
                plan,
                &self.layout,
                shared,
                owner,
                None,
                self.options.limits.max_state_bytes,
            )?;
        }
        while strand.pull.call_values.len() < demand.roots().len() {
            let index = strand.pull.call_values.len();
            let root = demand.roots()[index];
            let Some(result) =
                self.cells()
                    .consume_inputs(plan, strand, shared, root, demand.inputs(), budget)?
            else {
                return Ok(());
            };
            match result {
                FoldedValue::Value(value) => strand.pull.call_values.push(value),
                FoldedValue::Fault(fault) => {
                    self.complete(
                        plan,
                        strand,
                        shared,
                        owner,
                        (node.failed(), ControlOutcome::Failed, Some(fault)),
                    )?;
                    return Ok(());
                }
            }
            if strand.pull.call_values.len() < demand.roots().len() {
                strand.pull.active = Some(PendingPull {
                    root: demand.roots()[index + 1],
                    input: None,
                });
            }
        }
        if let Some(scope) = scope {
            // Arguments move into spare frame storage; the pull scratch keeps
            // its capacity for this strand and the callee it lends to.
            let mut storage = self.spare_frames.pop().unwrap_or_default();
            storage.arguments.append(&mut strand.pull.call_values);
            strand.transfer = Transfer::Call(PendingCall {
                owner,
                scope,
                storage,
            });
        } else {
            // The buffer travels with the result and returns to the caller's scratch.
            let values = std::mem::take(&mut strand.pull.call_values);
            let bytes =
                Self::values_charge(plan, demand, &values, self.options.limits.max_state_bytes)?;
            let hold = Hold::new(shared, bytes, 0, self.options.limits.max_state_bytes)?;
            strand.transfer = Transfer::Return(FunctionReturn::Values {
                values,
                hold: Some(hold),
            });
            self.record(RunEvent {
                activation: strand.sequence,
                node: node.node(),
                outcome: ControlOutcome::Then,
                fault: None,
            });
        }
        strand.pull.reset();
        shared.pull_owner = None;
        Ok(())
    }
    fn values_charge(
        plan: &ControlPlan,
        demand: &UpdateDemand,
        values: &[Value],
        limit: usize,
    ) -> Result<usize, RunError> {
        let mut bytes = 0usize;
        for (&root, value) in demand.roots().iter().zip(values) {
            let charge = plan
                .data()
                .checked_value_charge(
                    plan.data().exports()[root].slot,
                    value,
                    limit.saturating_sub(bytes),
                )
                .map_err(|_| RunError::ResourceLimit)?;
            bytes = bytes.checked_add(charge).ok_or(RunError::ResourceLimit)?;
        }
        Ok(bytes)
    }
    pub(super) fn enter_function(
        &mut self,
        mut caller: Strand,
        call: PendingCall,
    ) -> Result<(), RunError> {
        let plan = self.plan.as_ref().ok_or(RunError::InvalidState)?.clone();
        let layout = plan.function(call.scope).ok_or(RunError::InvalidState)?;
        let depth = self.regions.get(caller.region)?.depth + 1;
        if depth > self.options.limits.max_region_depth
            || !self.regions.has_room(1, 1, &self.options.limits)
        {
            return self.fail(RunError::ResourceLimit);
        }
        let shared = self.activations[caller.sequence as usize]
            .as_deref()
            .ok_or(RunError::InvalidState)?;
        let ControlOperation::Call { demand, .. } = plan.nodes()[call.owner.index()].operation()
        else {
            return Err(RunError::InvalidState);
        };
        let mut bytes = Self::values_charge(
            &plan,
            demand,
            &call.storage.arguments,
            self.options.limits.max_state_bytes,
        )?;
        for &variable in layout.locals() {
            let metadata = &plan.variables()[variable];
            bytes = bytes
                .checked_add(
                    metadata
                        .initial()
                        .and_then(|v| metadata.charge(v))
                        .unwrap_or(16),
                )
                .ok_or(RunError::ResourceLimit)?;
        }
        let hold = Hold::new(shared, bytes, 0, self.options.limits.max_state_bytes)?;
        let FrameStorage {
            arguments,
            mut locals,
        } = call.storage;
        locals.extend(layout.locals().iter().map(|&i| LocalCell {
            value: plan.variables()[i].initial().cloned(),
            version: 0,
        }));
        let region = self.regions.mint_region()?;
        let mut child = self.child_strand(&caller, region)?;
        child.id = caller.id;
        child.pull = caller.pull.lend();
        child.function = Some(FrameId::of_region(region));
        child.pc = layout.entry();
        caller.pc = None;
        self.frames.insert(
            FrameId::of_region(region),
            FunctionFrame {
                scope: call.scope,
                parent: caller.function,
                arguments,
                locals,
                hold,
            },
        );
        self.regions.open(
            region,
            Region {
                frames: None,
                retired_cursor: None,
                parent: Some(caller.region),
                activation: caller.sequence,
                depth,
                live: IdSet::one(child.id),
                children: IdSet::default(),
                continuation: Some(caller),
                kind: RegionKind::Function {
                    owner: call.owner,
                    result: None,
                },
            },
        )?;
        self.ready.push_front(child.id, child);
        Ok(())
    }
    pub(super) fn leave_function(
        &mut self,
        strand: Strand,
        result: FunctionReturn,
    ) -> Result<(), RunError> {
        let region = RegionId::of_frame(strand.function.ok_or(RunError::InvalidState)?);
        let frame = self.regions.get_mut(region)?;
        let RegionKind::Function {
            result: pending, ..
        } = &mut frame.kind
        else {
            return Err(RunError::InvalidState);
        };
        if pending.is_some() {
            return Err(RunError::InvalidState);
        }
        *pending = Some(result);
        if let Some(caller) = frame.continuation.as_mut() {
            caller.pull.recycle(strand.pull);
        }
        self.cancel_region(region)?;
        self.complete_regions(region)
    }
    pub(super) fn finish_function(
        &mut self,
        caller: &mut Strand,
        owner: ControlIx,
        result: FunctionReturn,
    ) -> Result<(), RunError> {
        let plan = self.plan.as_ref().ok_or(RunError::InvalidState)?.clone();
        let ControlOperation::Call { scope, .. } = plan.nodes()[owner.index()].operation() else {
            return Err(RunError::InvalidState);
        };
        if let FunctionReturn::Values { values, .. } = &result
            && values.len()
                != plan
                    .function(*scope)
                    .ok_or(RunError::InvalidState)?
                    .outputs()
        {
            return Err(RunError::InvalidState);
        }
        let mut shared = self.activations[caller.sequence as usize]
            .take()
            .ok_or(RunError::InvalidState)?;
        let node = &plan.nodes()[owner.index()];
        let result = match result {
            FunctionReturn::Values { values, hold } => {
                drop(hold);
                let published = Self::publish_call_results(
                    &plan,
                    &self.layout,
                    &mut shared,
                    owner,
                    Some(&values),
                    self.options.limits.max_state_bytes,
                );
                caller.pull.keep_values(values);
                published.map(|()| (node.then(), ControlOutcome::Then, None))
            }
            FunctionReturn::Fault(fault) => {
                Ok((node.failed(), ControlOutcome::Failed, Some(fault)))
            }
        };
        let result =
            result.and_then(|outcome| self.complete(&plan, caller, &mut shared, owner, outcome));
        self.activations[caller.sequence as usize] = Some(shared);
        result
    }
    fn publish_call_results(
        plan: &ControlPlan,
        layout: &StateLayout,
        shared: &mut ActivationState,
        owner: ControlIx,
        values: Option<&[Value]>,
        limit: usize,
    ) -> Result<(), RunError> {
        let mut removed = 0usize;
        let mut added = 0usize;
        for &input in plan.value_targets(owner) {
            let HostSource::CallOutput(_, index) = plan.host_sources()[input] else {
                return Err(RunError::InvalidState);
            };
            let slot = plan.data().nodes()[plan.data().host_inputs()[input].index()].outputs()[0];
            let cell = layout.outputs[input].ok_or(RunError::InvalidState)?;
            if let Some(FoldedValue::Value(value)) = &shared.outputs[cell] {
                removed = removed
                    .checked_add(
                        plan.data()
                            .value_charge(slot, value)
                            .ok_or(RunError::InvalidState)?,
                    )
                    .ok_or(RunError::ResourceLimit)?;
            }
            if let Some(values) = values {
                let value = values.get(index).ok_or(RunError::InvalidState)?;
                added = added
                    .checked_add(
                        plan.data()
                            .checked_value_charge(slot, value, limit.saturating_sub(added))
                            .map_err(|_| RunError::ResourceLimit)?,
                    )
                    .ok_or(RunError::ResourceLimit)?;
            }
        }
        let retained = shared
            .retained_bytes
            .checked_sub(removed)
            .and_then(|n| n.checked_add(added))
            .ok_or(RunError::ResourceLimit)?;
        if retained
            .checked_add(shared.local_bytes)
            .and_then(|n| n.checked_add(shared.owned_bytes.load(Ordering::Relaxed)))
            .is_none_or(|n| n > limit)
        {
            return Err(RunError::ResourceLimit);
        }
        for &input in plan.value_targets(owner) {
            let HostSource::CallOutput(_, index) = plan.host_sources()[input] else {
                unreachable!("checked call projection")
            };
            shared.outputs[layout.outputs[input].ok_or(RunError::InvalidState)?] =
                values.map(|values| FoldedValue::Value(values[index].clone()));
        }
        shared.retained_bytes = retained;
        Ok(())
    }
}
