//! Atomic typed variable actions and ordered multi-input demand.
use super::cells::Cells;
use super::function::{find_frame, frame_value};
use super::*;
use std::cmp::Ordering;

struct UpdateOperands {
    first: Value,
    second: Option<Value>,
}
enum UpdatePoll {
    Pending,
    Fault(ConstantFault),
    Ready(UpdateOperands),
}
struct UpdateCommit<'a> {
    frame: Option<u64>,
    owner: ControlIx,
    variable: usize,
    old: &'a Value,
    new: Value,
    write: bool,
    compare: bool,
}
impl Run {
    fn consume_update(
        &self,
        plan: &ControlPlan,
        activation: &mut Strand,
        shared: &mut ActivationState,
        demand: &UpdateDemand,
        budget: &mut usize,
    ) -> Result<UpdatePoll, RunError> {
        loop {
            let index = activation.pull.completed_len;
            let root = *demand.roots().get(index).ok_or(RunError::InvalidState)?;
            let Some(result) =
                self.consume_inputs(plan, activation, shared, root, demand.inputs(), budget)?
            else {
                return Ok(UpdatePoll::Pending);
            };
            match result {
                FoldedValue::Fault(fault) => return Ok(UpdatePoll::Fault(fault)),
                FoldedValue::Value(value) => {
                    *activation
                        .pull
                        .completed
                        .get_mut(index)
                        .ok_or(RunError::InvalidState)? = Some(value);
                    activation.pull.completed_len += 1;
                }
            }
            if activation.pull.completed_len == demand.roots().len() {
                let first = activation.pull.completed[0]
                    .take()
                    .ok_or(RunError::InvalidState)?;
                let second = activation.pull.completed[1].take();
                return Ok(UpdatePoll::Ready(UpdateOperands { first, second }));
            }
            // Same epoch and union snapshot; only the requested export changes.
            activation.pull.active = Some(PendingPull {
                root: demand.roots()[index + 1],
                input: None,
            });
        }
    }

    /// `None` while the ordered demand is pending; otherwise the node's step.
    pub(super) fn execute_update(
        &mut self,
        plan: &ControlPlan,
        activation: &mut Strand,
        shared: &mut ActivationState,
        owner: ControlIx,
        budget: &mut usize,
    ) -> Result<Option<Step>, RunError> {
        let node = &plan.nodes()[owner.index()];
        let ControlOperation::Update {
            variable,
            operation,
            demand,
        } = node.operation()
        else {
            return Err(RunError::InvalidState);
        };
        let (variable, operation) = (*variable, *operation);
        let metadata = &plan.variables()[variable];
        let cell = self.layout.variables[variable];
        let limit = self.options.limits.max_state_bytes;
        let frame = if metadata.scope() == VarScope::Local && node.scope() != 0 {
            Some(find_frame(&self.frames, activation.function, node.scope())?)
        } else {
            None
        };
        let current = if let Some(frame) = frame {
            frame_value(&self.frames, plan, frame, variable)?
        } else if metadata.scope() == VarScope::Local {
            shared.locals[cell].as_ref()
        } else {
            self.variables[cell].as_ref()
        };
        let update = match current.cloned() {
            None => Err(failure(
                node,
                KernelFault {
                    code: "core/UninitializedVariable",
                    message_key: "core/uninitialized-variable",
                    message: "The variable has not been initialized.",
                },
            )),
            Some(old) => match self.consume_update(plan, activation, shared, demand, budget)? {
                UpdatePoll::Pending => return Ok(None),
                UpdatePoll::Fault(fault) => Err(failed(node, fault)),
                UpdatePoll::Ready(inputs) => {
                    compute(metadata.ty(), operation, &old, &inputs, limit)
                        .map(|result| (old, result))
                        .map_err(|fault| failure(node, fault))
                }
            },
        };
        let (old, (new, write)) = match update {
            Ok(update) => update,
            Err(step) => {
                // A failed update clears its value outputs before its Failed edge.
                Self::publish_match_value(plan, &self.layout, shared, owner, None, limit)?;
                return Ok(Some(step));
            }
        };
        self.cells().publish_update(
            plan,
            shared,
            UpdateCommit {
                frame,
                owner,
                variable,
                old: &old,
                new,
                write,
                compare: matches!(operation, UpdateOperation::CompareAndSet),
            },
        )?;
        Ok(Some((node.then(), ControlOutcome::Then, None)))
    }
}

impl Cells<'_> {
    /// Admit every charge (cell, projections, frame hold, version) first; the
    /// stores after that point cannot fail, so a rejected update changes nothing.
    fn publish_update(
        &mut self,
        plan: &ControlPlan,
        shared: &mut ActivationState,
        commit: UpdateCommit<'_>,
    ) -> Result<(), RunError> {
        let UpdateCommit {
            frame,
            owner,
            variable,
            old,
            new,
            write,
            compare,
        } = commit;
        let metadata = &plan.variables()[variable];
        let local = metadata.scope() == VarScope::Local;
        let old_charge = metadata.charge(old).ok_or(RunError::InvalidState)?;
        let new_charge = metadata.charge(&new).ok_or(RunError::InvalidState)?;
        let bytes = if frame.is_some() {
            // Function-frame bytes are reserved with the complete output delta below.
            old_charge
        } else if local {
            shared.local_bytes
        } else {
            *self.bytes
        }
        .checked_sub(old_charge)
        .and_then(|n| n.checked_add(new_charge))
        .ok_or(RunError::ResourceLimit)?;
        let exchanged = Value::Bool(write);
        let projection = |i: usize| -> Result<&Value, RunError> {
            Ok(match plan.host_sources()[i] {
                HostSource::Update(_, UpdateProjection::Old) => old,
                HostSource::Update(_, UpdateProjection::New) => &new,
                HostSource::Update(_, UpdateProjection::Exchanged) if compare => &exchanged,
                _ => return Err(RunError::InvalidState),
            })
        };
        let mut removed = 0usize;
        let mut added = 0usize;
        for &i in plan.value_targets(owner) {
            let slot = plan.data().nodes()[plan.data().host_inputs()[i].index()].outputs()[0];
            let charge = |v: &Value| {
                plan.data()
                    .value_charge(slot, v)
                    .ok_or(RunError::InvalidState)
            };
            let cell = self.layout.outputs[i].ok_or(RunError::InvalidState)?;
            if let Some(FoldedValue::Value(value)) = &shared.outputs[cell] {
                removed = removed
                    .checked_add(charge(value)?)
                    .ok_or(RunError::ResourceLimit)?;
            }
            added = added
                .checked_add(charge(projection(i)?)?)
                .ok_or(RunError::ResourceLimit)?;
        }
        let retained = shared
            .retained_bytes
            .checked_sub(removed)
            .and_then(|n| n.checked_add(added))
            .ok_or(RunError::ResourceLimit)?;
        let locals = if local && frame.is_none() {
            bytes
        } else {
            shared.local_bytes
        };
        if frame.is_none()
            && ((!local && bytes > self.limit)
                || locals
                    .checked_add(retained)
                    .and_then(|n| {
                        n.checked_add(
                            shared
                                .owned_bytes
                                .load(std::sync::atomic::Ordering::Relaxed),
                        )
                    })
                    .is_none_or(|n| n > self.limit))
        {
            return Err(RunError::ResourceLimit);
        }
        let frame_write = if let Some(frame) = frame {
            // Also check unchanged CAS so projections cannot bypass the combined budget.
            let reservation = self.prepare_frame_write(
                plan,
                shared,
                super::function::FrameWriteSpec {
                    frame,
                    variable,
                    value: &new,
                    retained,
                    write,
                },
            )?;
            Some((frame, reservation))
        } else {
            None
        };
        let version = if write && frame.is_none() {
            self.versions[variable]
                .checked_add(1)
                .ok_or(RunError::ResourceLimit)?
        } else {
            self.versions[variable]
        };
        // All admission is complete. Subsequent stores are infallible typed copies.
        for &i in plan.value_targets(owner) {
            let value = projection(i)?.clone();
            shared.outputs[self.layout.outputs[i].expect("checked execution output")] =
                Some(FoldedValue::Value(value));
        }
        shared.retained_bytes = retained;
        if write {
            let cell = self.layout.variables[variable];
            if let Some((frame, reservation)) = frame_write {
                self.commit_frame_write(plan, frame, variable, new, reservation);
            } else if local {
                shared.locals[cell] = Some(new);
                shared.local_bytes = bytes;
            } else {
                self.variables[cell] = Some(new);
                *self.bytes = bytes;
            }
            if frame.is_none() {
                self.versions[variable] = version;
            }
            self.segment.variable(variable, metadata.id());
        }
        Ok(())
    }
}

fn internal() -> KernelFault {
    KernelFault {
        code: "core/InternalFault",
        message_key: "core/kernel-input-contract",
        message: "Update inputs violate the resolved contract.",
    }
}
fn compute(
    ty: &orange_model::value::shape::ValueShape,
    operation: UpdateOperation,
    old: &Value,
    inputs: &UpdateOperands,
    limit: usize,
) -> Result<(Value, bool), KernelFault> {
    if matches!(operation, UpdateOperation::CompareAndSet) {
        let expected = &inputs.first;
        let value = inputs.second.as_ref().ok_or_else(internal)?;
        let exchanged = ty.equal_values(old, expected).ok_or_else(internal)?;
        return Ok((
            if exchanged {
                value.clone()
            } else {
                old.clone()
            },
            exchanged,
        ));
    }
    if inputs.second.is_some() {
        return Err(internal());
    }
    let value = &inputs.first;
    let result = match operation {
        UpdateOperation::Numeric {
            operation,
            scalar,
            overflow,
            float_policy,
        } => orange_model::scalar::evaluate(
            operation,
            scalar,
            overflow,
            float_policy,
            &[old.clone(), value.clone()],
        )
        .map_err(|error| KernelFault {
            code: error.code(),
            message_key: error.message_key(),
            message: match error {
                orange_model::scalar::ScalarError::NumericOverflow => {
                    "Numeric arithmetic overflowed."
                }
                orange_model::scalar::ScalarError::DivideByZero => {
                    "Division requires a nonzero divisor."
                }
                orange_model::scalar::ScalarError::NonFinite => {
                    "Finite arithmetic requires finite inputs and results."
                }
                orange_model::scalar::ScalarError::InputContract => {
                    "Update inputs violate the resolved contract."
                }
            },
        })?,
        UpdateOperation::Min | UpdateOperation::Max => {
            macro_rules! compare {($($v:ident),*)=>{match(old,value){$((Value::$v(a),Value::$v(b))=>a.cmp(b),)*_=>return Err(internal())}};}
            let order = compare!(Bool, I32, I64, U32, U64, Str, Duration, Timestamp);
            if (matches!(operation, UpdateOperation::Min) && order == Ordering::Greater)
                || (matches!(operation, UpdateOperation::Max) && order == Ordering::Less)
            {
                value.clone()
            } else {
                old.clone()
            }
        }
        UpdateOperation::And | UpdateOperation::Or | UpdateOperation::Xor => {
            macro_rules! bitwise {($($v:ident),*)=>{match(old,value){$((Value::$v(a),Value::$v(b))=>Value::$v(match operation {UpdateOperation::And=>a&b,UpdateOperation::Or=>a|b,_=>a^b}),)*_=>return Err(internal())}};}
            bitwise!(Bool, I32, I64, U32, U64)
        }
        UpdateOperation::Append => {
            let failure = || KernelFault {
                code: "core/LimitExceeded",
                message_key: "core/update-limit-exceeded",
                message: "The updated value exceeds its state limit.",
            };
            let a = ty.checked_charge(old, limit).map_err(|_| failure())?;
            let b = ty.checked_charge(value, limit).map_err(|_| failure())?;
            let bytes = a
                .checked_add(b)
                .and_then(|n| n.checked_sub(16))
                .filter(|n| *n <= limit)
                .ok_or_else(failure)?;
            match (old, value) {
                (Value::Str(a), Value::Str(b)) => {
                    if b.is_empty() {
                        return Ok((old.clone(), true));
                    }
                    if a.is_empty() {
                        return Ok((value.clone(), true));
                    }
                    let mut result = String::new();
                    result
                        .try_reserve_exact(bytes - 16)
                        .map_err(|_| failure())?;
                    result.push_str(a);
                    result.push_str(b);
                    Value::Str(Arc::new(result))
                }
                (Value::Bytes(a), Value::Bytes(b)) => {
                    if b.is_empty() {
                        return Ok((old.clone(), true));
                    }
                    if a.is_empty() {
                        return Ok((value.clone(), true));
                    }
                    let mut result = Vec::new();
                    result
                        .try_reserve_exact(bytes - 16)
                        .map_err(|_| failure())?;
                    result.extend_from_slice(a);
                    result.extend_from_slice(b);
                    Value::Bytes(Arc::new(result))
                }
                (Value::List(a), Value::List(b)) => {
                    if b.is_empty() {
                        return Ok((old.clone(), true));
                    }
                    if a.is_empty() {
                        return Ok((value.clone(), true));
                    }
                    Value::List(Arc::new(a.concat(b).ok_or_else(failure)?))
                }
                _ => return Err(internal()),
            }
        }
        UpdateOperation::CompareAndSet => unreachable!("handled before single operand"),
    };
    Ok((result, true))
}
