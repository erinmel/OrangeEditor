//! Structured loops and sequences. A step pushes a cursor and requests body
//! entry, or names the loop region a Break/Continue unwinds; the cursor then
//! resumes when the body region completes. Steps borrow [`Loops`], so they read the
//! layout, limits and region tree and append to the trace, and reach nothing else.
use super::*;

const LOOP_BUDGET_EXCEEDED: KernelFault = KernelFault {
    code: orange_compiler::FaultText::Static("core/LoopBudgetExceeded"),
    message_key: orange_compiler::FaultText::Static("core/loop-budget-exceeded"),
    message: "The loop iteration budget is exhausted.",
};

const ZERO_RANGE_STEP: KernelFault = KernelFault {
    code: orange_compiler::FaultText::Static("core/ZeroRangeStep"),
    message_key: orange_compiler::FaultText::Static("core/zero-range-step"),
    message: "The range step must be nonzero.",
};

fn in_range(index: i64, end: i64, step: i64) -> bool {
    if step > 0 { index < end } else { index > end }
}

fn next_index(index: i64, end: i64, step: i64) -> Option<i64> {
    index
        .checked_add(step)
        .filter(|&next| in_range(next, end, step))
}

/// What loop and sequence steps may touch: the layout, limits and region tree to read and
/// the trace to append to. Scheduling, cells, frames, timers and activations stay out of
/// reach.
pub(super) struct Loops<'r> {
    pub layout: &'r StateLayout,
    pub limits: &'r RunLimits,
    pub regions: &'r Regions,
    pub trace: &'r mut Trace,
}
impl Run {
    pub(super) fn loops(&mut self) -> Loops<'_> {
        Loops {
            layout: &self.layout,
            limits: &self.options.limits,
            regions: &self.regions,
            trace: &mut self.trace,
        }
    }

    pub(super) fn execute_range(
        &mut self,
        plan: &ControlPlan,
        strand: &mut Strand,
        shared: &mut ActivationState,
        owner: ControlIx,
        budget: &mut usize,
    ) -> Result<Option<Step>, RunError> {
        let node = &plan.nodes()[owner.index()];
        let ControlOperation::ForRange {
            demand,
            body,
            completed,
            max_iterations,
        } = node.operation()
        else {
            return Err(RunError::InvalidState);
        };
        while let Some(&root) = demand.roots().get(strand.pull.call_values.len()) {
            let Some(result) =
                self.cells()
                    .consume_inputs(plan, strand, shared, root, demand.inputs(), budget)?
            else {
                return Ok(None);
            };
            match result {
                FoldedValue::Value(value) => strand.pull.call_values.push(value),
                FoldedValue::Fault(fault) => return Ok(Some(failed(node, fault))),
            }
            if let Some(&root) = demand.roots().get(strand.pull.call_values.len()) {
                strand.pull.active = Some(PendingPull { root, input: None });
            }
        }
        let [Value::I64(start), Value::I64(end), Value::I64(step)] = strand.pull.call_values[..]
        else {
            return Err(DataError::KernelContract { node: node.node() }.into());
        };
        if step == 0 {
            return Ok(Some(failure(node, ZERO_RANGE_STEP)));
        }
        if !in_range(start, end, step) {
            return Ok(Some((*completed, ControlOutcome::Completed, None)));
        }
        if start.abs_diff(end).div_ceil(step.unsigned_abs()) > *max_iterations {
            return Ok(Some(failure(node, LOOP_BUDGET_EXCEEDED)));
        }
        self.loops()
            .begin_indexed(plan, strand, shared, owner, (start, end, step))?;
        Ok(Some((*body, ControlOutcome::Body, None)))
    }
}
impl Loops<'_> {
    pub(super) fn record(&mut self, event: RunEvent) {
        self.trace.record(event, self.limits.max_trace);
    }
    fn begin_indexed(
        &self,
        plan: &ControlPlan,
        strand: &mut Strand,
        shared: &mut ActivationState,
        owner: ControlIx,
        (start, end, step): (i64, i64, i64),
    ) -> Result<(), RunError> {
        self.admit_cursor(strand)?;
        strand.transfer = Transfer::EnterBody(Some(owner));
        strand.stack.push(Cursor::Indexed {
            owner,
            next: next_index(start, end, step),
            end,
            step,
        });
        Run::publish_index(plan, self.layout, shared, owner, Some(start))
    }
    pub(super) fn loop_step(
        &self,
        plan: &ControlPlan,
        ix: ControlIx,
        activation: &mut Strand,
        shared: &mut ActivationState,
        value: Option<Value>,
    ) -> Result<Step, RunError> {
        let node = &plan.nodes()[ix.index()];
        let contract = DataError::KernelContract { node: node.node() };
        Ok(match (node.operation(), value) {
            (
                ControlOperation::ForEach {
                    items,
                    body,
                    completed,
                },
                Some(value),
            ) => {
                if iteration::collection_len(&value).ok_or(contract)? == 0 {
                    return Ok((*completed, ControlOutcome::Completed, None));
                }
                self.admit_cursor(activation)?;
                let context = activation.iteration_context.clone();
                let cursor = self.begin_iteration(plan, shared, ix, *items, value, context)?;
                activation.stack.push(Cursor::ForEach(Box::new(cursor)));
                activation.transfer = Transfer::EnterBody(Some(ix));
                (*body, ControlOutcome::Body, None)
            }
            (
                ControlOperation::While {
                    body,
                    completed,
                    max_iterations,
                    ..
                },
                Some(value),
            ) => {
                let iterations = activation
                    .reenter
                    .filter(|(owner, _)| *owner == ix)
                    .map_or(0, |(_, n)| n);
                match value {
                    Value::Bool(false) => (*completed, ControlOutcome::Completed, None),
                    Value::Bool(true) if iterations >= *max_iterations => {
                        failure(node, LOOP_BUDGET_EXCEEDED)
                    }
                    Value::Bool(true) => {
                        self.admit_cursor(activation)?;
                        activation.transfer = Transfer::EnterBody(Some(ix));
                        activation.stack.push(Cursor::While {
                            owner: ix,
                            iterations: iterations + 1,
                        });
                        (*body, ControlOutcome::Body, None)
                    }
                    _ => return Err(contract.into()),
                }
            }
            (
                ControlOperation::Count {
                    body, completed, ..
                },
                Some(value),
            ) => match value {
                Value::I64(count) if count < 0 => failure(node, KernelFault::NEGATIVE_COUNT),
                Value::I64(0) => (*completed, ControlOutcome::Completed, None),
                Value::I64(count) => {
                    self.begin_indexed(plan, activation, shared, ix, (0, count, 1))?;
                    (*body, ControlOutcome::Body, None)
                }
                _ => return Err(contract.into()),
            },
            (ControlOperation::Sequence { steps }, None) => {
                self.admit_cursor(activation)?;
                activation.transfer = Transfer::EnterBody(None);
                activation
                    .stack
                    .push(Cursor::Sequence { owner: ix, next: 1 });
                (
                    steps.first().copied().flatten(),
                    ControlOutcome::Step(0),
                    None,
                )
            }
            (ControlOperation::Break | ControlOperation::Continue, None) => {
                self.unwind_step(node, activation)?
            }
            _ => return Err(RunError::InvalidState),
        })
    }
    /// Break and Continue request an unwind of their nearest owning loop region.
    fn unwind_step(&self, node: &ControlNode, activation: &mut Strand) -> Result<Step, RunError> {
        let owner = node.loop_owner().ok_or(RunError::InvalidState)?;
        let region = self.regions.loop_region(activation.region, owner)?;
        let is_break = matches!(node.operation(), ControlOperation::Break);
        activation.transfer = Transfer::Unwind { region, is_break };
        let outcome = if is_break {
            ControlOutcome::Break
        } else {
            ControlOutcome::Continue
        };
        Ok((None, outcome, None))
    }
    /// The region depth limit bounds a strand's cursor stack.
    fn admit_cursor(&self, activation: &Strand) -> Result<(), RunError> {
        if activation.stack.len() == self.limits.max_region_depth {
            return Err(RunError::ResourceLimit);
        }
        Ok(())
    }
    pub(super) fn resume_cursor(
        &mut self,
        plan: &ControlPlan,
        activation: &mut Strand,
        shared: &mut ActivationState,
    ) -> Result<(), RunError> {
        let Some(cursor) = activation.stack.pop() else {
            return Ok(());
        };
        match cursor {
            Cursor::Stream(cursor) => {
                let work = streams::PreparedRead::new(
                    cursor.owner,
                    cursor.reference,
                    streams::ReadMode::Iterate {
                        index: cursor.next,
                        parent: activation.iteration_context.clone(),
                    },
                    shared,
                    self.limits.max_state_bytes,
                )?;
                activation.transfer = Transfer::Stream(work);
                activation.yielded = true;
            }
            Cursor::ForEach(cursor) => self.resume_iteration(plan, activation, shared, cursor)?,
            Cursor::While { owner, iterations } => {
                activation.pc = Some(owner.into());
                activation.reenter = Some((owner, iterations));
                activation.yielded = true;
            }
            Cursor::Indexed {
                owner,
                next,
                end,
                step,
            } => {
                let node = &plan.nodes()[owner.index()];
                let (body, completed) = match node.operation() {
                    ControlOperation::Count {
                        body, completed, ..
                    }
                    | ControlOperation::ForRange {
                        body, completed, ..
                    } => (*body, *completed),
                    _ => return Err(RunError::InvalidState),
                };
                if let Some(index) = next {
                    activation.transfer = Transfer::EnterBody(Some(owner));
                    activation.stack.push(Cursor::Indexed {
                        owner,
                        next: next_index(index, end, step),
                        end,
                        step,
                    });
                    Run::publish_index(plan, self.layout, shared, owner, Some(index))?;
                    activation.pc = body;
                    activation.yielded = true;
                    self.record(RunEvent {
                        route: None,
                        activation: activation.sequence,
                        node: node.node(),
                        outcome: ControlOutcome::Body,
                        fault: None,
                    });
                } else {
                    Run::publish_index(plan, self.layout, shared, owner, None)?;
                    activation.pc = completed;
                    self.record(RunEvent {
                        route: None,
                        activation: activation.sequence,
                        node: node.node(),
                        outcome: ControlOutcome::Completed,
                        fault: None,
                    });
                }
            }
            Cursor::Sequence { owner, next } => {
                let ControlOperation::Sequence { steps } = plan.nodes()[owner.index()].operation()
                else {
                    return Err(RunError::InvalidState);
                };
                if next < steps.len() {
                    activation.transfer = Transfer::EnterBody(None);
                    activation.pc = steps[next];
                    activation.stack.push(Cursor::Sequence {
                        owner,
                        next: next + 1,
                    });
                    self.record(RunEvent {
                        route: None,
                        activation: activation.sequence,
                        node: plan.nodes()[owner.index()].node(),
                        outcome: ControlOutcome::Step(next),
                        fault: None,
                    });
                }
            }
        }
        Ok(())
    }
}
