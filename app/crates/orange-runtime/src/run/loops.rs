//! Structured loops and sequences. A step pushes a cursor and requests body
//! entry, or names the loop region a Break/Continue unwinds; the cursor then
//! resumes when the body region completes. Run state is only read here.
use super::*;

const LOOP_BUDGET_EXCEEDED: KernelFault = KernelFault {
    code: "core/LoopBudgetExceeded",
    message_key: "core/loop-budget-exceeded",
    message: "The loop iteration budget is exhausted.",
};

impl Run {
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
                    self.admit_cursor(activation)?;
                    activation.transfer = Transfer::EnterBody(Some(ix));
                    activation.stack.push(Cursor::Count {
                        owner: ix,
                        next: 1,
                        count,
                    });
                    Self::publish_index(plan, &self.layout, shared, ix, Some(0));
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
        let region = self.loop_region(activation.region, owner)?;
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
        if activation.stack.len() == self.options.limits.max_region_depth {
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
            Cursor::ForEach(cursor) => self.resume_iteration(plan, activation, shared, cursor)?,
            Cursor::While { owner, iterations } => {
                activation.pc = Some(owner.into());
                activation.reenter = Some((owner, iterations));
                activation.yielded = true;
            }
            Cursor::Count { owner, next, count } => {
                let node = &plan.nodes()[owner.index()];
                let ControlOperation::Count {
                    body, completed, ..
                } = node.operation()
                else {
                    return Err(RunError::InvalidState);
                };
                if next < count {
                    activation.transfer = Transfer::EnterBody(Some(owner));
                    activation.stack.push(Cursor::Count {
                        owner,
                        next: next + 1,
                        count,
                    });
                    Self::publish_index(plan, &self.layout, shared, owner, Some(next));
                    activation.pc = *body;
                    activation.yielded = true;
                    self.record(RunEvent {
                        activation: activation.sequence,
                        node: node.node(),
                        outcome: ControlOutcome::Body,
                        fault: None,
                    });
                } else {
                    Self::publish_index(plan, &self.layout, shared, owner, None);
                    activation.pc = *completed;
                    self.record(RunEvent {
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
