//! Inert host commands and complete-once actor continuations. No executor runs here.
use super::retained::Hold;
use super::*;
use std::sync::atomic::AtomicU8;

/// Non-reused operation identity bound to one process-unique run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EffectId {
    run: u64,
    sequence: u64,
}

/// Read-only lifetime signal for a host adapter. Cancellation is monotonic and
/// signals intent; the host remains responsible for cancelling its external work.
#[derive(Clone)]
pub struct EffectLease(Arc<AtomicU8>);
impl EffectLease {
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed) == 1
    }
    pub fn is_finished(&self) -> bool {
        self.0.load(Ordering::Relaxed) != 0
    }
}
impl std::fmt::Debug for EffectLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectLease")
            .field("finished", &self.is_finished())
            .finish()
    }
}
/// One command, handed to the host once. Debug omits inputs. The host may move
/// this to its own bounded worker; taking it never executes graph work.
pub struct EffectRequest {
    pub id: EffectId,
    pub node: NodeId,
    pub route: u64,
    /// Exact host context pinned at compile time, including the executable generation.
    pub context: Arc<dyn std::any::Any + Send + Sync>,
    pub inputs: Arc<[Value]>,
    pub lease: EffectLease,
}
impl std::fmt::Debug for EffectRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectRequest")
            .field("id", &self.id)
            .field("node", &self.node)
            .field("route", &self.route)
            .field("input_count", &self.inputs.len())
            .finish()
    }
}
/// Payload-free refusal. A refusal leaves the pending operation and quotas unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum EffectError {
    #[error("The effect belongs to another run.")]
    WrongRun,
    #[error("The effect is retired, completed or was never issued.")]
    Stale,
    #[error("The run is not accepting completions.")]
    NotRunning,
    #[error("The effect result violates its typed contract.")]
    Contract,
    #[error("The effect result exceeds a retained-value limit.")]
    Limit,
}
impl EffectError {
    pub const fn message_key(self) -> &'static str {
        match self {
            Self::WrongRun => "core/effect-wrong-run",
            Self::Stale => "core/effect-stale",
            Self::NotRunning => "core/effect-not-running",
            Self::Contract => "core/effect-contract",
            Self::Limit => "core/effect-limit",
        }
    }
}
pub(super) struct PreparedEffect {
    pub owner: ControlIx,
    values: Arc<[Value]>,
    hold: Hold,
}
pub(super) struct PendingEffect {
    pub token: u64,
    pub strand: Strand,
    pub owner: ControlIx,
    inputs: Arc<[Value]>,
    hold: Hold,
    lease: EffectLease,
    issued: bool,
    completion: Option<EffectCompletion>,
}
struct EffectCompletion {
    outcome: Result<Vec<Value>, KernelFault>,
    hold: Hold,
}
#[derive(Default)]
pub(super) struct Effects {
    next: u64,
    by_id: BTreeMap<u64, PendingEffect>,
    by_strand: BTreeMap<StrandId, u64>,
    ready: VecDeque<u64>,
    commands: VecDeque<u64>,
    bytes: usize,
}
impl Effects {
    pub fn len(&self) -> usize {
        self.by_id.len()
    }
    pub fn has_ready(&self) -> bool {
        !self.ready.is_empty()
    }
    pub fn clear(&mut self) {
        for pending in self.by_id.values() {
            pending.lease.0.store(1, Ordering::Relaxed);
        }
        self.by_id.clear();
        self.by_strand.clear();
        self.ready.clear();
        self.commands.clear();
        self.bytes = 0;
    }
    fn remove(&mut self, id: u64) -> Option<PendingEffect> {
        let pending = self.by_id.remove(&id)?;
        self.by_strand.remove(&pending.strand.id);
        self.ready.retain(|seq| *seq != id);
        self.commands.retain(|seq| *seq != id);
        self.bytes -= pending.hold.bytes + pending.completion.as_ref().map_or(0, |c| c.hold.bytes);
        Some(pending)
    }
}
impl Run {
    /// Transfers one request to the host. A blocked worker cannot block the run actor.
    pub fn take_effect(&mut self) -> Option<EffectRequest> {
        if self.pump_open || !matches!(self.state, RunState::Running | RunState::Draining) {
            return None;
        }
        let sequence = self.effects.commands.pop_front()?;
        let pending = self.effects.by_id.get_mut(&sequence)?;
        let ControlOperation::Effect { effect, .. } =
            self.plan.as_ref()?.nodes()[pending.owner.index()].operation()
        else {
            return None;
        };
        pending.issued = true;
        Some(EffectRequest {
            id: EffectId {
                run: self.id,
                sequence,
            },
            node: self.plan.as_ref()?.nodes()[pending.owner.index()].node(),
            route: effect.binding().route,
            context: effect.binding().context.clone(),
            inputs: pending.inputs.clone(),
            lease: pending.lease.clone(),
        })
    }
    /// Admits a typed result without executing its continuation. A bad result can
    /// be corrected and retried with the same ID; an accepted result cannot.
    pub fn complete_effect(
        &mut self,
        id: EffectId,
        outcome: Result<&[Value], KernelFault>,
    ) -> Result<(), EffectError> {
        if id.run != self.id {
            return Err(EffectError::WrongRun);
        }
        if self.pump_open || !matches!(self.state, RunState::Running | RunState::Draining) {
            return Err(EffectError::NotRunning);
        }
        let pending = self
            .effects
            .by_id
            .get_mut(&id.sequence)
            .ok_or(EffectError::Stale)?;
        if !pending.issued || pending.completion.is_some() {
            return Err(EffectError::Stale);
        }
        let plan = self.plan.as_ref().ok_or(EffectError::NotRunning)?;
        let ControlOperation::Effect { effect, .. } =
            plan.nodes()[pending.owner.index()].operation()
        else {
            return Err(EffectError::Contract);
        };
        let mut bytes = 0usize;
        if let Err(fault) = outcome {
            bytes = plan
                .data()
                .fault_charge(&ConstantFault {
                    source: plan.nodes()[pending.owner.index()].node(),
                    fault,
                })
                .ok_or(EffectError::Contract)?;
        }
        if let Ok(values) = outcome {
            if values.len() != effect.outputs().len() {
                return Err(EffectError::Contract);
            }
            for (ty, value) in effect.outputs().iter().zip(values) {
                let charge = ty
                    .checked_charge(
                        value,
                        self.options.limits.max_effect_bytes.saturating_sub(bytes),
                    )
                    .map_err(|e| {
                        if matches!(e, orange_model::value::shape::ValueCheckError::InvalidType) {
                            EffectError::Contract
                        } else {
                            EffectError::Limit
                        }
                    })?;
                bytes = bytes.checked_add(charge).ok_or(EffectError::Limit)?;
            }
        }
        if self
            .effects
            .bytes
            .checked_add(bytes)
            .is_none_or(|b| b > self.options.limits.max_effect_bytes)
        {
            return Err(EffectError::Limit);
        }
        let shared = self
            .activations
            .get(pending.strand.sequence)
            .and_then(Option::as_deref)
            .ok_or(EffectError::Stale)?;
        let hold = Hold::new(shared, bytes, 0, self.options.limits.max_state_bytes)
            .map_err(|_| EffectError::Limit)?;
        pending.completion = Some(EffectCompletion {
            outcome: outcome.map(<[Value]>::to_vec),
            hold,
        });
        self.effects.bytes += bytes;
        self.effects.ready.push_back(id.sequence);
        Ok(())
    }
    pub(super) fn park_effect(
        &mut self,
        strand: Strand,
        prepared: PreparedEffect,
    ) -> Result<(), RunError> {
        let sequence = self
            .effects
            .next
            .checked_add(1)
            .ok_or(RunError::ResourceLimit)?;
        self.effects.next = sequence;
        self.effects.bytes += prepared.hold.bytes;
        self.effects.by_strand.insert(strand.id, sequence);
        self.effects.by_id.insert(
            sequence,
            PendingEffect {
                token: sequence,
                strand,
                owner: prepared.owner,
                inputs: prepared.values,
                hold: prepared.hold,
                lease: EffectLease(Arc::new(AtomicU8::new(0))),
                issued: false,
                completion: None,
            },
        );
        self.effects.commands.push_back(sequence);
        Ok(())
    }
    pub(super) fn take_effect_for_cancel(&mut self, strand: StrandId) -> Option<PendingEffect> {
        let pending = self.effects.remove(*self.effects.by_strand.get(&strand)?)?;
        pending.lease.0.store(1, Ordering::Relaxed);
        Some(pending)
    }
    pub(super) fn resume_effect(&mut self, plan: &ControlPlan) -> Result<(), RunError> {
        let sequence = self
            .effects
            .ready
            .front()
            .copied()
            .ok_or(RunError::InvalidState)?;
        let mut pending = self
            .effects
            .remove(sequence)
            .ok_or(RunError::InvalidState)?;
        pending.lease.0.store(2, Ordering::Relaxed);
        let completion = pending.completion.take().ok_or(RunError::InvalidState)?;
        let mut shared = self
            .activations
            .get_mut(pending.strand.sequence)
            .and_then(Option::take)
            .ok_or(RunError::InvalidState)?;
        drop(completion.hold);
        drop(pending.hold);
        let node = &plan.nodes()[pending.owner.index()];
        let result = (|| {
            let step = match completion.outcome {
                Ok(values) => {
                    Self::publish_effect_values(
                        plan,
                        &self.layout,
                        &mut shared,
                        pending.owner,
                        Some(&values),
                        self.options.limits.max_state_bytes,
                    )?;
                    (node.then(), ControlOutcome::Then, None)
                }
                Err(fault) => failure(node, fault),
            };
            self.complete(plan, &mut pending.strand, &mut shared, pending.owner, step)
        })();
        let strand = pending.strand;
        *self
            .activations
            .get_mut(strand.sequence)
            .ok_or(RunError::InvalidState)? = Some(shared);
        result?;
        self.ready.push_back(strand.id, strand);
        Ok(())
    }
    pub(super) fn execute_effect(
        &mut self,
        plan: &ControlPlan,
        strand: &mut Strand,
        shared: &mut ActivationState,
        owner: ControlIx,
        budget: &mut usize,
    ) -> Result<Option<Step>, RunError> {
        let node = &plan.nodes()[owner.index()];
        let ControlOperation::Effect { effect, demand } = node.operation() else {
            return Err(RunError::InvalidState);
        };
        if strand.pull.active.is_none() {
            Self::publish_effect_values(
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
            let Some(value) = self.cells().consume_inputs(
                plan,
                strand,
                shared,
                demand.roots()[index],
                demand.inputs(),
                budget,
            )?
            else {
                return Ok(None);
            };
            match value {
                FoldedValue::Value(value) => strand.pull.call_values.push(value),
                FoldedValue::Fault(fault) => return Ok(Some(failed(node, fault))),
            }
            if index + 1 < demand.roots().len() {
                strand.pull.active = Some(PendingPull {
                    root: demand.roots()[index + 1],
                    input: None,
                });
            }
        }
        let values = std::mem::take(&mut strand.pull.call_values);
        strand.pull.reset();
        shared.pull_owner = None;
        if let Some(action) = &effect.binding().action {
            let mut outputs = vec![Value::None; effect.outputs().len()];
            let outcome = action.execute(&values, &mut outputs);
            let step = match outcome {
                Err(fault) if fault.is_valid() => failure(node, fault),
                Err(_) => return Err(DataError::KernelContract { node: node.node() }.into()),
                Ok(()) => {
                    for (ty, value) in effect.outputs().iter().zip(&outputs) {
                        ty.checked_charge(value, self.options.limits.max_state_bytes)
                            .map_err(|_| DataError::KernelContract { node: node.node() })?;
                    }
                    Self::publish_effect_values(
                        plan,
                        &self.layout,
                        shared,
                        owner,
                        Some(&outputs),
                        self.options.limits.max_state_bytes,
                    )?;
                    (node.then(), ControlOutcome::Then, None)
                }
            };
            strand.pull.keep_values(values);
            return Ok(Some(step));
        }
        if self.effects.len() >= self.options.limits.max_pending_effects {
            return Err(RunError::ResourceLimit);
        }
        let bytes = super::function::values_charge(
            plan,
            demand,
            &values,
            self.options.limits.max_effect_bytes,
        )?;
        if self
            .effects
            .bytes
            .checked_add(bytes)
            .is_none_or(|b| b > self.options.limits.max_effect_bytes)
        {
            return Err(RunError::ResourceLimit);
        }
        let hold = Hold::new(shared, bytes, 0, self.options.limits.max_state_bytes)?;
        strand.transfer = Transfer::Effect(PreparedEffect {
            owner,
            values: values.into(),
            hold,
        });
        Ok(Some((node.then(), ControlOutcome::Suspended, None)))
    }
    fn publish_effect_values(
        plan: &ControlPlan,
        layout: &StateLayout,
        shared: &mut ActivationState,
        owner: ControlIx,
        values: Option<&[Value]>,
        limit: usize,
    ) -> Result<(), RunError> {
        Self::publish_values(
            plan,
            layout,
            shared,
            owner,
            |source| match source {
                HostSource::EffectOutput(_, i) => values.and_then(|v| v.get(i)).cloned(),
                _ => None,
            },
            limit,
        )
    }
}
impl Drop for Effects {
    fn drop(&mut self) {
        self.clear();
    }
}
