//! Segment-stable, budgeted root Watches. No graph strand runs during evaluation.
use super::events::EventAdmission;
use super::*;

#[derive(Clone, Copy)]
enum Reaction {
    Watch(usize),
    Change(usize),
}

#[derive(Default)]
pub(super) struct Watches {
    data: Option<DataActivation>,
    previous: Vec<Option<Value>>,
    faults: Vec<Option<ConstantFault>>,
    cache: Option<Tally>,
    sequences: Vec<u64>,
    pending: VecDeque<Reaction>,
    changed: Vec<(usize, Value, Value)>,
    staged: Option<Tally>,
    active: bool,
    cause: Option<(u64, u16)>,
    evaluations: u64,
}
impl Watches {
    fn replace_fault(
        &mut self,
        index: usize,
        fault: Option<ConstantFault>,
        limit: usize,
    ) -> Result<(), RunError> {
        let bytes = |fault: &ConstantFault| {
            fault
                .fault
                .code
                .retained_bytes()
                .saturating_add(fault.fault.message_key.retained_bytes())
        };
        let cache = self.cache.as_mut().ok_or(RunError::InvalidState)?;
        let total = cache
            .get()
            .checked_sub(self.faults[index].as_ref().map_or(0, bytes))
            .and_then(|n| n.checked_add(fault.as_ref().map_or(0, bytes)))
            .filter(|n| {
                n.checked_add(self.staged.as_ref().map_or(0, Tally::get))
                    .is_some_and(|n| n <= limit)
            })
            .ok_or(RunError::ResourceLimit)?;
        if !cache.try_set(total) {
            return Err(RunError::AggregateLimit);
        }
        self.faults[index] = fault;
        Ok(())
    }
    pub fn new(
        plan: &ControlPlan,
        prepared: Option<Arc<PreparedData>>,
        revision: u64,
        seed: u64,
        options: DataOptions,
        budget: Arc<Budget>,
    ) -> Result<Self, RunError> {
        if plan.watches().is_empty() {
            return Ok(Self::default());
        }
        Ok(Self {
            data: Some(DataActivation::from_prepared(
                prepared.ok_or(RunError::InvalidState)?,
                revision,
                seed,
                u64::MAX,
                options,
                Some(budget.clone()),
            )?),
            previous: vec![None; plan.watches().len()],
            cache: Some(Tally::new(Some(budget.clone()), 0)),
            faults: vec![None; plan.watches().len()],
            staged: Some(Tally::new(Some(budget), 0)),
            changed: Vec::new(),
            sequences: vec![0; plan.watches().len()],
            pending: (0..plan.watches().len()).map(Reaction::Watch).collect(),
            active: false,
            cause: None,
            evaluations: 0,
        })
    }
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
    pub fn finish_segment(&mut self) {
        self.changed.clear();
        if let Some(staged) = &mut self.staged {
            let _ = staged.try_set(0);
        }
    }
    pub fn schedule(
        &mut self,
        plan: &ControlPlan,
        changed: &[(usize, Value, Value)],
        cause: (u64, u16),
        limit: usize,
    ) -> bool {
        if changed.is_empty() {
            return true;
        }
        debug_assert!(self.pending.is_empty());
        let bytes = changed
            .iter()
            .try_fold(0usize, |sum, (v, value, previous)| {
                sum.checked_add(plan.variables()[*v].charge(value)?)?
                    .checked_add(plan.variables()[*v].charge(previous)?)
            });
        let Some(bytes) = bytes.filter(|n| {
            n.checked_add(self.cache.as_ref().map_or(0, Tally::get))
                .is_some_and(|total| total <= limit)
        }) else {
            return false;
        };
        let Some(staged) = &mut self.staged else {
            return false;
        };
        if !staged.try_set(bytes) {
            return false;
        }
        self.changed = changed.to_vec();
        self.cause = Some(cause);
        let mut reactions = Vec::new();
        for (i, watch) in plan.watches().iter().enumerate() {
            if watch
                .variables()
                .iter()
                .any(|v| changed.iter().any(|(id, _, _)| id == v))
            {
                reactions.push((watch.handler().order(), watch.node(), Reaction::Watch(i)));
            }
        }
        for (i, change) in plan.changes().iter().enumerate() {
            if changed.iter().any(|(v, _, _)| *v == change.variable()) {
                reactions.push((change.handler().order(), change.node(), Reaction::Change(i)));
            }
        }
        reactions.sort_by_key(|(order, node, _)| (*order, *node));
        self.pending
            .extend(reactions.into_iter().map(|(_, _, r)| r));
        true
    }
    pub fn clear(&mut self) {
        let evaluations = self.evaluations;
        *self = Self::default();
        self.evaluations = evaluations;
    }
}
impl Run {
    /// Number of consumption epochs actually opened for affected Watches.
    pub fn watch_evaluations(&self) -> u64 {
        self.watches.evaluations
    }
    /// Last graph fault of a Watch, cleared by its next successful evaluation.
    /// Faults suppress delivery and preserve the previous successful baseline.
    pub fn watch_fault(&self, node: NodeId) -> Option<&ConstantFault> {
        let index = self
            .plan
            .as_ref()?
            .watches()
            .iter()
            .position(|w| w.node() == node)?;
        self.watches.faults.get(index)?.as_ref()
    }
    pub(super) fn evaluate_watch(
        &mut self,
        plan: &ControlPlan,
        budget: &mut usize,
    ) -> Result<(), RunError> {
        let reaction = *self.watches.pending.front().ok_or(RunError::InvalidState)?;
        if let Reaction::Change(index) = reaction {
            *budget -= 1;
            self.watches.pending.pop_front();
            let change = &plan.changes()[index];
            let (_, value, previous) = self
                .watches
                .changed
                .iter()
                .find(|(v, _, _)| *v == change.variable())
                .ok_or(RunError::InvalidState)?;
            let payload =
                super::changes::pair(plan, change.variable(), value.clone(), previous.clone())
                    .ok_or(RunError::ResourceLimit)?;
            let (cause, depth) = self.watches.cause.ok_or(RunError::InvalidState)?;
            if let Some((depth, sequence)) = depth
                .checked_add(1)
                .filter(|d| *d <= self.options.max_cause_depth)
                .zip(self.changes.sequences[index].checked_add(1))
            {
                let admission = EventAdmission {
                    event: plan.events().len() + plan.manuals().len() + plan.timers().len() + index,
                    origin: EventOrigin::Change,
                    cause: Some(cause),
                    cause_depth: depth,
                    source_seq: sequence,
                    wall_ns: None,
                    graph_source: None,
                    tick: None,
                };
                match self.admit_event(plan, &payload, admission) {
                    Ok(_) => self.changes.sequences[index] = sequence,
                    Err(_) => self.events.count_overflow(),
                }
            } else {
                self.events.count_overflow();
            }
            return Ok(());
        }
        let Reaction::Watch(index) = reaction else {
            return Err(RunError::InvalidState);
        };
        let watch = &plan.watches()[index];
        let data = self.watches.data.as_mut().ok_or(RunError::InvalidState)?;
        if !self.watches.active {
            *budget -= 1;
            data.begin_epoch()?;
            self.watches.evaluations = self.watches.evaluations.saturating_add(1);
            self.watches.active = true;
        }
        let outcome = loop {
            match data.poll_host_export(watch.root(), budget)? {
                HostPoll::Pending => return Ok(()),
                HostPoll::Input(input) => {
                    let HostSource::Variable(variable) = plan.host_sources()[input] else {
                        return Err(RunError::InvalidState);
                    };
                    let value = self.variables[self.layout.variables[variable]]
                        .as_ref()
                        .ok_or(RunError::InvalidState)?;
                    data.set_input(input, value.clone())?;
                }
                HostPoll::Ready(Some(export)) => {
                    break export.outcome.cloned().map_err(Clone::clone);
                }
                HostPoll::Ready(None) => return Err(RunError::InvalidState),
            }
        };
        self.watches.active = false;
        self.watches.pending.pop_front();
        let value = match outcome {
            Ok(value) => value,
            Err(fault) => {
                self.watches.replace_fault(
                    index,
                    Some(fault),
                    self.options.limits.max_state_bytes,
                )?;
                return Ok(());
            }
        };
        self.watches
            .replace_fault(index, None, self.options.limits.max_state_bytes)?;
        let previous = &self.watches.previous[index];
        let fires = previous.as_ref().is_some_and(|previous| {
            if watch.rising() {
                *previous == Value::Bool(false) && value == Value::Bool(true)
            } else {
                watch.ty().equal_values(previous, &value) == Some(false)
            }
        });
        let charge = watch
            .ty()
            .checked_charge(&value, self.options.limits.max_state_bytes)
            .map_err(|_| RunError::ResourceLimit)?;
        let old = previous
            .as_ref()
            .and_then(|v| watch.ty().checked_charge(v, usize::MAX).ok())
            .unwrap_or(0);
        let cache = self.watches.cache.as_mut().ok_or(RunError::InvalidState)?;
        let total = cache
            .get()
            .checked_sub(old)
            .and_then(|n| n.checked_add(charge))
            .filter(|n| {
                n.checked_add(self.watches.staged.as_ref().map_or(0, Tally::get))
                    .is_some_and(|total| total <= self.options.limits.max_state_bytes)
            })
            .ok_or(RunError::ResourceLimit)?;
        if !cache.try_set(total) {
            return Err(RunError::AggregateLimit);
        }
        self.watches.previous[index] = Some(value.clone());
        if !fires || self.state != RunState::Running {
            return Ok(());
        }
        let Some((cause, depth)) = self.watches.cause else {
            return Ok(());
        };
        let Some(depth) = depth
            .checked_add(1)
            .filter(|d| *d <= self.options.max_cause_depth)
        else {
            self.events.count_overflow();
            return Ok(());
        };
        let Some(sequence) = self.watches.sequences[index].checked_add(1) else {
            self.events.count_overflow();
            return Ok(());
        };
        let admission = EventAdmission {
            event: plan.events().len()
                + plan.manuals().len()
                + plan.timers().len()
                + plan.changes().len()
                + index,
            origin: EventOrigin::Watch,
            cause: Some(cause),
            cause_depth: depth,
            source_seq: sequence,
            wall_ns: None,
            graph_source: None,
            tick: None,
        };
        match self.admit_event(plan, &value, admission) {
            Ok(_) => self.watches.sequences[index] = sequence,
            Err(_) => self.events.count_overflow(),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orange_compiler::FaultText;

    #[test]
    fn last_watch_fault_text_is_admitted_transactionally_and_released_on_success() {
        let node = "10000000-0000-4000-8000-000000000001".parse().unwrap();
        let code: Arc<str> = Arc::from("acme/Failed");
        let weak = Arc::downgrade(&code);
        let fault = ConstantFault {
            source: node,
            fault: KernelFault {
                code: FaultText::Shared(code),
                message_key: "acme/failed".into(),
                message: "The watch failed.",
            },
        };
        let bytes = fault.fault.code.retained_bytes();
        let budget = Budget::new(bytes);
        let mut watches = Watches {
            faults: vec![None],
            cache: Some(Tally::new(Some(budget.clone()), 0)),
            ..Default::default()
        };
        assert_eq!(
            watches.replace_fault(0, Some(fault.clone()), bytes - 1),
            Err(RunError::ResourceLimit)
        );
        assert!(watches.faults[0].is_none());
        assert_eq!(budget.used(), 0);
        watches.replace_fault(0, Some(fault), bytes).unwrap();
        assert_eq!(budget.used(), bytes);
        watches.replace_fault(0, None, bytes).unwrap();
        assert_eq!(budget.used(), 0);
        assert!(weak.upgrade().is_none());
    }
}
