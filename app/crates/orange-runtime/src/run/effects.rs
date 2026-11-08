//! Inert host commands and complete-once actor continuations. No executor runs here.
use super::retained::Hold;
use super::*;
use std::sync::atomic::AtomicU8;

/// Non-reused operation identity bound to one process-unique run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EffectId {
    pub(super) run: u64,
    pub(super) sequence: u64,
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
    revoked: Option<KernelFault>,
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
    pub(in crate::run) fn reserve_shared_command(&self) -> Result<Option<Tally>, SharedError> {
        if self.pump_open || !matches!(self.state, RunState::Running | RunState::Draining) {
            return Ok(None);
        }
        let Some(plan) = &self.plan else {
            return Ok(None);
        };
        let pending = self
            .effects
            .commands
            .iter()
            .filter_map(|id| self.effects.by_id.get(id))
            .find(|p| {
                matches!(plan.nodes()[p.owner.index()].operation(),
                ControlOperation::Effect { effect, .. } if effect.shared().is_some())
            });
        let Some(pending) = pending else {
            return Ok(None);
        };
        let bytes = pending
            .hold
            .bytes
            .checked_add(std::mem::size_of::<shared::RequestData>())
            .ok_or(SharedError::Limit)?;
        let mut hold = Tally::new(Some(self.budget.clone()), 0);
        if !hold.try_set(bytes) {
            return Err(SharedError::Limit);
        }
        Ok(Some(hold))
    }
    pub(in crate::run) fn revoke_resource_effects(&mut self, id: orange_model::value::HandleId) {
        for (&sequence, pending) in &mut self.effects.by_id {
            let matches = |value: &Value| matches!(value,Value::Handle(handle) | Value::Stream(handle) if handle.id()==id);
            let uses = pending.inputs.iter().any(matches)
                || pending
                    .completion
                    .as_ref()
                    .is_some_and(|c| c.outcome.as_ref().is_ok_and(|v| v.iter().any(matches)));
            if !uses || pending.revoked.is_some() {
                continue;
            }
            if let Some(completion) = pending.completion.take() {
                self.effects.bytes -= completion.hold.bytes;
            }
            pending.revoked = Some(resources::resource_fault(ResourceError::StaleHandle));
            pending.lease.0.store(1, Ordering::Relaxed);
            self.effects.commands.retain(|id| *id != sequence);
            if !self.effects.ready.contains(&sequence) {
                self.effects.ready.push_back(sequence);
            }
        }
    }
    pub(in crate::run) fn effect_resource_target(
        &self,
        id: EffectId,
        kind: &orange_model::id::TypeExpr,
    ) -> Result<RegionId, EffectError> {
        self.effect_reference_target(id, kind, false)
    }
    pub(in crate::run) fn effect_reference_target(
        &self,
        id: EffectId,
        kind: &orange_model::id::TypeExpr,
        stream: bool,
    ) -> Result<RegionId, EffectError> {
        if id.run != self.id {
            return Err(EffectError::WrongRun);
        }
        if self.pump_open || !matches!(self.state, RunState::Running | RunState::Draining) {
            return Err(EffectError::NotRunning);
        }
        let pending = self
            .effects
            .by_id
            .get(&id.sequence)
            .ok_or(EffectError::Stale)?;
        if !pending.issued || pending.completion.is_some() || pending.revoked.is_some() {
            return Err(EffectError::Stale);
        }
        let ControlOperation::Effect { effect, .. } =
            self.plan.as_ref().ok_or(EffectError::NotRunning)?.nodes()[pending.owner.index()]
                .operation()
        else {
            return Err(EffectError::Contract);
        };
        let valid = if stream {
            matches!(effect.outputs(),[orange_model::value::shape::ValueShape::Stream(expected)] if expected==kind)
        } else {
            matches!(effect.outputs(),[orange_model::value::shape::ValueShape::Handle(expected)] if expected==kind)
        };
        if !valid {
            return Err(EffectError::Contract);
        }
        Ok(pending.strand.region)
    }
    /// Transfers one request to the host. A blocked worker cannot block the run actor.
    pub fn take_effect(&mut self) -> Option<EffectRequest> {
        self.take_effect_kind(false)
    }
    pub(in crate::run) fn take_effect_kind(&mut self, shared: bool) -> Option<EffectRequest> {
        if self.pump_open || !matches!(self.state, RunState::Running | RunState::Draining) {
            return None;
        }
        let plan = self.plan.as_ref()?;
        let position = self.effects.commands.iter().position(|id| {
            self.effects.by_id.get(id).is_some_and(|p| {
                matches!(plan.nodes()[p.owner.index()].operation(),
                ControlOperation::Effect { effect, .. } if effect.shared().is_some() == shared)
            })
        })?;
        let sequence = self.effects.commands.remove(position)?;
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
        self.complete_effect_kind(id, outcome, false)
    }
    pub(in crate::run) fn complete_effect_kind(
        &mut self,
        id: EffectId,
        outcome: Result<&[Value], KernelFault>,
        shared: bool,
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
            .get(&id.sequence)
            .ok_or(EffectError::Stale)?;
        if !pending.issued || pending.completion.is_some() || pending.revoked.is_some() {
            return Err(EffectError::Stale);
        }
        let plan = self.plan.as_ref().ok_or(EffectError::NotRunning)?;
        let ControlOperation::Effect { effect, .. } =
            plan.nodes()[pending.owner.index()].operation()
        else {
            return Err(EffectError::Contract);
        };
        let mut bytes = 0usize;
        if effect.shared().is_some() != shared {
            return Err(EffectError::Contract);
        }
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
                self.check_graph_resource(value, pending.strand.region)
                    .map_err(|_| EffectError::Contract)?;
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
        self.effects
            .by_id
            .get_mut(&id.sequence)
            .ok_or(EffectError::Stale)?
            .completion = Some(EffectCompletion {
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
                revoked: None,
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
        let _ = pending
            .lease
            .0
            .compare_exchange(0, 2, Ordering::Relaxed, Ordering::Relaxed);
        let completion = pending.completion.take();
        let outcome = match (pending.revoked.take(), completion) {
            (Some(fault), _) => Err(fault),
            (None, Some(completion)) => {
                drop(completion.hold);
                match completion.outcome {
                    Ok(values) => match values.iter().find_map(|value| {
                        self.check_graph_resource(value, pending.strand.region)
                            .err()
                    }) {
                        Some(error) => Err(resources::resource_fault(error)),
                        None => Ok(values),
                    },
                    fault => fault,
                }
            }
            (None, None) => return Err(RunError::InvalidState),
        };
        let mut shared = self
            .activations
            .get_mut(pending.strand.sequence)
            .and_then(Option::take)
            .ok_or(RunError::InvalidState)?;
        drop(pending.hold);
        let node = &plan.nodes()[pending.owner.index()];
        let result = (|| {
            let step = match outcome {
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
        if let Some(error) = values
            .iter()
            .find_map(|value| self.check_graph_resource(value, strand.region).err())
        {
            return Ok(Some(failure(node, resources::resource_fault(error))));
        }
        if let Some(action) = &effect.binding().action {
            let mut outputs = vec![Value::None; effect.outputs().len()];
            let outcome = if let Some(session) = &mut self.replay {
                session.action(
                    node.node(),
                    action.as_ref(),
                    &values,
                    &mut outputs,
                    effect.outputs(),
                    self.options.limits.max_state_bytes,
                )?
            } else {
                action.execute(&values, &mut outputs)
            };
            let step = match outcome {
                Err(fault) if fault.is_valid() => failure(node, fault),
                Err(_) => return Err(DataError::KernelContract { node: node.node() }.into()),
                Ok(()) => {
                    for (ty, value) in effect.outputs().iter().zip(&outputs) {
                        ty.checked_charge(value, self.options.limits.max_state_bytes)
                            .map_err(|_| DataError::KernelContract { node: node.node() })?;
                        if let Err(error) = self.check_graph_resource(value, strand.region) {
                            return Ok(Some(failure(node, resources::resource_fault(error))));
                        }
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
    pub(super) fn publish_effect_values(
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
