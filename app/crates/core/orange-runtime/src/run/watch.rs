//! Segment-stable, budgeted root Watches, and the arguments of sources armed by the
//! host (11 §10.5). No graph strand runs during evaluation.
use super::events::EventAdmission;
use super::*;
use orange_model::catalog::{NumericWatch, WatchCondition};
use std::collections::BTreeSet;

/// The activation and causal depth a reaction descends from; none for a host-published
/// live value.
type Cause = Option<(u64, u16)>;

#[derive(Clone, Copy)]
enum Reaction {
    Watch(usize, Cause),
    Change {
        entry: usize,
        value_index: usize,
    },
    /// Evaluate the arguments of [`ControlPlan::sources`]`[i]`.
    Source(usize),
}

struct ChangeBatch {
    values: Vec<(usize, Value, Value)>,
    cause: (u64, u16),
    bytes: usize,
    remaining: usize,
}

#[derive(Default)]
struct Crossing {
    armed: bool,
    since: Option<(u64, Cause)>,
    last_fired: Option<u64>,
}

impl Crossing {
    fn observe(
        &mut self,
        test: &NumericWatch,
        value: &Value,
        baseline: bool,
        now: u64,
        cause: Cause,
        delays: (u64, u64),
    ) -> Result<(Option<Cause>, Option<u64>), RunError> {
        let matches = test.matches(value);
        if !baseline {
            self.armed = !matches;
            return Ok((None, None));
        }
        if !self.armed {
            self.armed = test.released(value);
            return Ok((None, None));
        }
        if !matches {
            self.since = None;
            return Ok((None, None));
        }
        let (since, cause) = *self.since.get_or_insert((now, cause));
        let (hold, cooldown) = delays;
        let held = since.checked_add(hold).ok_or(RunError::ResourceLimit)?;
        let cooled = self
            .last_fired
            .map_or(Some(0), |last| last.checked_add(cooldown))
            .ok_or(RunError::ResourceLimit)?;
        let due = held.max(cooled);
        if now < due {
            return Ok((None, Some(due)));
        }
        self.since = None;
        self.last_fired = Some(now);
        self.armed = false;
        Ok((Some(cause), None))
    }
}

/// The values a source's data inputs have now, for the host to arm its provider with.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceArguments {
    pub node: NodeId,
    /// In the order of [`orange_compiler::ControlSource::ports`].
    pub values: Vec<Value>,
}

#[derive(Default)]
pub(super) struct Watches {
    data: Option<DataActivation>,
    previous: Vec<Option<Value>>,
    crossings: Vec<Crossing>,
    faults: Vec<Option<ConstantFault>>,
    cache: Option<Tally>,
    sequences: Vec<u64>,
    pending: VecDeque<Reaction>,
    changes: VecDeque<ChangeBatch>,
    staged: Option<Tally>,
    active: bool,
    evaluations: u64,
    /// A reaction for each watch and source is already queued, so a burst of live
    /// values re-evaluates it once.
    watch_queued: Vec<bool>,
    source_queued: Vec<bool>,
    /// The arguments last handed to the host, and the last fault of each source.
    arguments: Vec<Option<Vec<Value>>>,
    source_faults: Vec<Option<ConstantFault>>,
    /// Values of the source being evaluated, one per argument already read.
    partial: Vec<Value>,
    outbox: BTreeSet<usize>,
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
    pub(crate) fn new(
        plan: &ControlPlan,
        prepared: Option<Arc<PreparedData>>,
        revision: u64,
        seed: u64,
        options: DataOptions,
        budget: Arc<Budget>,
    ) -> Result<Self, RunError> {
        if plan.watches().is_empty() && plan.sources().is_empty() {
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
            crossings: (0..plan.watches().len())
                .map(|_| Crossing::default())
                .collect(),
            cache: Some(Tally::new(Some(budget.clone()), 0)),
            faults: vec![None; plan.watches().len()],
            staged: Some(Tally::new(Some(budget), 0)),
            changes: VecDeque::new(),
            sequences: vec![0; plan.watches().len()],
            pending: (0..plan.watches().len())
                .map(|i| Reaction::Watch(i, None))
                .chain((0..plan.sources().len()).map(Reaction::Source))
                .collect(),
            active: false,
            evaluations: 0,
            watch_queued: vec![true; plan.watches().len()],
            source_queued: vec![true; plan.sources().len()],
            arguments: vec![None; plan.sources().len()],
            source_faults: vec![None; plan.sources().len()],
            partial: Vec::new(),
            outbox: BTreeSet::new(),
        })
    }
    pub(crate) fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
    pub(crate) fn invalidate_reads(
        &mut self,
        plan: &ControlPlan,
        changed: &[(usize, Value, Value)],
    ) {
        if !self.active || changed.is_empty() {
            return;
        }
        let variables = match self.pending.front() {
            Some(Reaction::Watch(index, _)) => plan.watches()[*index].variables(),
            Some(Reaction::Source(index)) => plan.sources()[*index].variables(),
            _ => return,
        };
        if changed.iter().any(|(id, _, _)| variables.contains(id)) {
            // The queued reaction keeps its place. Its next slice starts a new
            // epoch, discarding any previously sampled arguments.
            self.active = false;
            self.partial.clear();
        }
    }
    pub(crate) fn schedule(
        &mut self,
        plan: &ControlPlan,
        changed: &[(usize, Value, Value)],
        cause: (u64, u16),
        limit: usize,
    ) -> bool {
        if changed.is_empty() {
            return true;
        }
        let mut reactions = Vec::new();
        let mut values = Vec::new();
        // Each segment has its own stamp and payloads. Only variables with
        // change-entry consumers occupy staging storage.
        for (variable, value, previous) in changed {
            let mut matched = false;
            for &entry in plan.variable_dependents(*variable).changes() {
                let change = &plan.changes()[entry];
                matched = true;
                reactions.push((
                    change.handler().order(),
                    change.node(),
                    Reaction::Change {
                        entry,
                        value_index: values.len(),
                    },
                ));
            }
            if matched {
                values.push((*variable, value.clone(), previous.clone()));
            }
        }
        if !reactions.is_empty() {
            let remaining = reactions.len();
            let Some(staged) = &mut self.staged else {
                return false;
            };
            let Some((bytes, total)) = values
                .iter()
                .try_fold(0usize, |sum, (v, value, previous)| {
                    sum.checked_add(plan.variables()[*v].charge(value)?)?
                        .checked_add(plan.variables()[*v].charge(previous)?)
                })
                .and_then(|bytes| Some((bytes, staged.get().checked_add(bytes)?)))
                .filter(|(_, total)| {
                    total
                        .checked_add(self.cache.as_ref().map_or(0, Tally::get))
                        .is_some_and(|total| total <= limit)
                })
            else {
                return false;
            };
            if !staged.try_set(total) {
                return false;
            }
            self.changes.push_back(ChangeBatch {
                values,
                cause,
                bytes,
                remaining,
            });
        }
        // Only the Watches that read a changed variable, each once, in plan order.
        let mut watches: Vec<usize> = changed
            .iter()
            .flat_map(|(variable, _, _)| plan.variable_dependents(*variable).watches())
            .copied()
            .collect();
        watches.sort_unstable();
        watches.dedup();
        for i in watches {
            let watch = &plan.watches()[i];
            if self.watch_queued[i] {
                // Already queued, perhaps by a live value with no cause: it keeps its
                // place but descends from the deepest write, so the causal-depth guard
                // still sees a chain the watch's own handler feeds.
                for reaction in &mut self.pending {
                    if let Reaction::Watch(index, queued) = reaction
                        && *index == i
                        && queued.is_none_or(|(_, depth)| depth < cause.1)
                    {
                        *queued = Some(cause);
                    }
                }
                continue;
            }
            self.watch_queued[i] = true;
            reactions.push((
                watch.handler().order(),
                watch.node(),
                Reaction::Watch(i, Some(cause)),
            ));
        }
        for i in Self::sources_reading(plan, changed) {
            if !self.source_queued[i] {
                self.source_queued[i] = true;
                reactions.push((0, plan.sources()[i].node(), Reaction::Source(i)));
            }
        }
        reactions.sort_by_key(|(order, node, _)| (*order, *node));
        self.pending
            .extend(reactions.into_iter().map(|(_, _, r)| r));
        true
    }
    /// Queue the watches and sources that read live value `live`, once each until
    /// they are evaluated.
    pub(crate) fn schedule_live(&mut self, plan: &ControlPlan, live: usize) {
        if self.data.is_none() {
            return;
        }
        let live = &plan.lives()[live];
        for &i in live.watch_dependents() {
            if !self.watch_queued[i] {
                self.watch_queued[i] = true;
                self.pending.push_back(Reaction::Watch(i, None));
            }
        }
        for &i in live.source_dependents() {
            if !self.source_queued[i] {
                self.source_queued[i] = true;
                self.pending.push_back(Reaction::Source(i));
            }
        }
    }
    pub(crate) fn schedule_sources(
        &mut self,
        plan: &ControlPlan,
        changed: &[(usize, Value, Value)],
    ) {
        for i in Self::sources_reading(plan, changed) {
            if !self.source_queued[i] {
                self.source_queued[i] = true;
                self.pending.push_back(Reaction::Source(i));
            }
        }
    }
    /// The sources whose arguments read a changed variable, each once, in plan order.
    fn sources_reading(plan: &ControlPlan, changed: &[(usize, Value, Value)]) -> Vec<usize> {
        let mut sources: Vec<usize> = changed
            .iter()
            .flat_map(|(variable, _, _)| plan.variable_dependents(*variable).sources())
            .copied()
            .collect();
        sources.sort_unstable();
        sources.dedup();
        sources
    }
    pub(crate) fn schedule_deadline(&mut self, index: usize) {
        if !self.watch_queued[index] {
            self.watch_queued[index] = true;
            self.pending.push_back(Reaction::Watch(index, None));
        }
    }
    pub(crate) fn clear(&mut self) {
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
    /// The arguments of the next source whose data inputs changed, for the host to arm
    /// or re-arm its provider with. Arguments equal to the last ones are not repeated.
    pub fn take_arguments(&mut self) -> Option<SourceArguments> {
        let index = self.watches.outbox.pop_first()?;
        Some(SourceArguments {
            node: self.plan.as_ref()?.sources()[index].node(),
            values: self.watches.arguments[index].clone()?,
        })
    }
    /// The last fault evaluating a source's arguments; the provider keeps the arguments
    /// it last had until the next success clears it.
    pub fn source_fault(&self, node: NodeId) -> Option<&ConstantFault> {
        let index = self
            .plan
            .as_ref()?
            .sources()
            .iter()
            .position(|s| s.node() == node)?;
        self.watches.source_faults.get(index)?.as_ref()
    }
    /// The value a live node last published, if any.
    pub fn live_value(&self, node: NodeId) -> Option<&Value> {
        let index = self.plan.as_ref()?.live_index(node)?;
        self.lives.get(index)?.as_ref()
    }
    /// The latest value published on a specific output.
    pub fn live_port_value(
        &self,
        node: NodeId,
        port: &orange_model::id::PortKey,
    ) -> Option<&Value> {
        let index = self.plan.as_ref()?.live_port_index(node, port)?;
        self.lives.get(index)?.as_ref()
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
        if let Reaction::Source(index) = reaction {
            return self.evaluate_source(plan, index, budget);
        }
        if let Reaction::Change {
            entry: index,
            value_index,
        } = reaction
        {
            *budget -= 1;
            self.watches.pending.pop_front();
            let batch = self
                .watches
                .changes
                .front_mut()
                .ok_or(RunError::InvalidState)?;
            let (variable, value, previous) = &batch.values[value_index];
            let payload = changes::pair(plan, *variable, value.clone(), previous.clone())
                .ok_or(RunError::ResourceLimit)?;
            let (cause, depth) = batch.cause;
            batch.remaining -= 1;
            if batch.remaining == 0 {
                let staged = self.watches.staged.as_mut().ok_or(RunError::InvalidState)?;
                staged.set(staged.get() - batch.bytes);
                self.watches.changes.pop_front();
            }
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
        let Reaction::Watch(index, cause) = reaction else {
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
        let Some(outcome) = poll_root(data, watch.root(), budget, |input| {
            root_value(plan, &self.variables, &self.layout, &self.lives, input)
        })?
        else {
            return Ok(());
        };
        self.watches.active = false;
        self.watches.pending.pop_front();
        self.watches.watch_queued[index] = false;
        let value = match outcome {
            Ok(value) => value,
            Err(fault) => {
                self.watches.crossings[index].since = None;
                self.timers
                    .set_watch(index, None, self.options.limits.max_timers)
                    .map_err(|_| RunError::ResourceLimit)?;
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
        if let WatchCondition::Deadband(threshold) = watch.condition()
            && (matches!(&value, Value::F32(sample) if !sample.is_finite())
                || matches!(&value, Value::F64(sample) if !sample.is_finite())
                || previous
                    .as_ref()
                    .is_some_and(|old| !exceeds_deadband(old, &value, threshold)))
        {
            return Ok(());
        }
        let firing = match watch.condition() {
            WatchCondition::Deadband(_) => previous.is_some().then_some(cause),
            WatchCondition::Changed => previous
                .as_ref()
                .is_some_and(|old| watch.ty().equal_values(old, &value) == Some(false))
                .then_some(cause),
            condition @ (WatchCondition::Rising | WatchCondition::Falling) => {
                let target = matches!(condition, WatchCondition::Rising);
                (previous == &Some(Value::Bool(!target)) && value == Value::Bool(target))
                    .then_some(cause)
            }
            WatchCondition::Numeric {
                test,
                hold_ns,
                cooldown_ns,
            } => {
                let (firing, deadline) = self.watches.crossings[index].observe(
                    test,
                    &value,
                    previous.is_some(),
                    self.now,
                    cause,
                    (*hold_ns, *cooldown_ns),
                )?;
                self.timers
                    .set_watch(index, deadline, self.options.limits.max_timers)
                    .map_err(|_| RunError::ResourceLimit)?;
                firing
            }
        };
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
        let Some(cause) = firing.filter(|_| self.state == RunState::Running) else {
            return Ok(());
        };
        // A live value is host input: its reactions start a new causal chain.
        let depth = cause.map_or(Some(0), |(_, depth)| depth.checked_add(1));
        let Some(depth) = depth.filter(|d| *d <= self.options.max_cause_depth) else {
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
            cause: cause.map(|(sequence, _)| sequence),
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

fn exceeds_deadband(old: &Value, new: &Value, threshold: &Value) -> bool {
    match (old, new, threshold) {
        (Value::I32(a), Value::I32(b), Value::I32(t)) => a.abs_diff(*b) > t.unsigned_abs(),
        (Value::I64(a), Value::I64(b), Value::I64(t)) => a.abs_diff(*b) > t.unsigned_abs(),
        (Value::U32(a), Value::U32(b), Value::U32(t)) => a.abs_diff(*b) > *t,
        (Value::U64(a), Value::U64(b), Value::U64(t)) => a.abs_diff(*b) > *t,
        (Value::F32(a), Value::F32(b), Value::F32(t)) => (a - b).abs() > *t,
        (Value::F64(a), Value::F64(b), Value::F64(t)) => (a - b).abs() > *t,
        _ => false,
    }
}

type RootOutcome = Result<Value, ConstantFault>;

fn poll_root(
    data: &mut DataActivation,
    root: usize,
    budget: &mut usize,
    mut read_input: impl FnMut(usize) -> Result<FoldedValue, RunError>,
) -> Result<Option<RootOutcome>, RunError> {
    loop {
        match data.poll_host_export(root, budget)? {
            HostPoll::Pending => return Ok(None),
            HostPoll::Input(input) => data.set_input_outcome(input, read_input(input)?)?,
            HostPoll::Ready(Some(export)) => {
                return Ok(Some(export.outcome.cloned().map_err(Clone::clone)));
            }
            HostPoll::Ready(None) => return Err(RunError::InvalidState),
        }
    }
}

/// A host input read at the root: a run variable, or a live value, which faults until
/// its provider has published one.
fn root_value(
    plan: &ControlPlan,
    variables: &[Option<Value>],
    layout: &StateLayout,
    lives: &[Option<Value>],
    input: usize,
) -> Result<FoldedValue, RunError> {
    match plan.host_sources()[input] {
        HostSource::Variable(variable) => variables[layout.variables[variable]]
            .clone()
            .map(FoldedValue::Value)
            .ok_or(RunError::InvalidState),
        HostSource::Live(live) => Ok(match &lives[live] {
            Some(value) => FoldedValue::Value(value.clone()),
            None => FoldedValue::Fault(no_value_yet(plan.lives()[live].node())),
        }),
        _ => Err(RunError::InvalidState),
    }
}

/// The fault of reading a live value its provider has not published yet.
pub(super) fn no_value_yet(node: NodeId) -> ConstantFault {
    ConstantFault {
        source: node,
        fault: KernelFault {
            code: orange_compiler::FaultText::Static("core/NoValueYet"),
            message_key: orange_compiler::FaultText::Static("core/no-value-yet"),
            message: "The live value has not been published yet.",
        },
    }
}

impl Run {
    /// Evaluates one source's arguments, every one in a single epoch, and hands them to
    /// the host when they differ from the last ones. A fault keeps the previous ones.
    fn evaluate_source(
        &mut self,
        plan: &ControlPlan,
        index: usize,
        budget: &mut usize,
    ) -> Result<(), RunError> {
        let source = &plan.sources()[index];
        let data = self.watches.data.as_mut().ok_or(RunError::InvalidState)?;
        if !self.watches.active {
            *budget -= 1;
            data.begin_epoch()?;
            self.watches.active = true;
            self.watches.partial.clear();
        }
        while self.watches.partial.len() < source.roots().len() {
            let root = source.roots()[self.watches.partial.len()];
            let Some(outcome) = poll_root(data, root, budget, |input| {
                root_value(plan, &self.variables, &self.layout, &self.lives, input)
            })?
            else {
                return Ok(());
            };
            match outcome {
                Ok(value) => self.watches.partial.push(value),
                Err(fault) => {
                    self.watches.active = false;
                    self.watches.pending.pop_front();
                    self.watches.source_queued[index] = false;
                    self.watches.partial.clear();
                    self.watches.source_faults[index] = Some(fault);
                    return Ok(());
                }
            }
        }
        self.watches.active = false;
        self.watches.pending.pop_front();
        self.watches.source_queued[index] = false;
        self.watches.source_faults[index] = None;
        let values = std::mem::take(&mut self.watches.partial);
        if self.watches.arguments[index].as_ref() == Some(&values) {
            return Ok(());
        }
        // Arguments are retained run state: charged like a watch's previous value.
        let limit = self.options.limits.max_state_bytes;
        let charge = |values: &[Value]| {
            values
                .iter()
                .zip(source.types())
                .try_fold(0usize, |sum, (value, ty)| {
                    sum.checked_add(ty.checked_charge(value, limit).ok()?)
                })
        };
        let new = charge(&values).ok_or(RunError::ResourceLimit)?;
        let old = self.watches.arguments[index]
            .as_deref()
            .and_then(charge)
            .unwrap_or(0);
        let cache = self.watches.cache.as_mut().ok_or(RunError::InvalidState)?;
        let total = cache
            .get()
            .checked_sub(old)
            .and_then(|n| n.checked_add(new))
            .filter(|n| *n <= limit)
            .ok_or(RunError::ResourceLimit)?;
        if !cache.try_set(total) {
            return Err(RunError::AggregateLimit);
        }
        self.watches.arguments[index] = Some(values);
        let _ = self.watches.outbox.insert(index);
        // A live value was published for the previous arguments: until the provider
        // armed with these publishes, it has no value, rather than a stale one.
        for &live in source.outputs() {
            let ty = plan.lives()[live].ty();
            let release = |value: &Value| ty.checked_charge(value, usize::MAX).unwrap_or(0);
            let mut freed = self
                .pending_lives
                .remove(&live)
                .flatten()
                .as_ref()
                .map_or(0, release);
            if let Some(old) = self.lives[live].take() {
                freed += release(&old);
                self.watches.schedule_live(plan, live);
            }
            let _ = self
                .live_bytes
                .try_set(self.live_bytes.get().saturating_sub(freed));
        }
        Ok(())
    }

    /// Stages a provider's publication for the next graph boundary. Each live node
    /// holds at most one pending publication. Returns whether the staged value changed.
    pub fn publish_live(&mut self, node: NodeId, value: Value) -> Result<bool, IngressError> {
        let plan = self.live_plan()?;
        let index = plan.live_index(node).ok_or(IngressError::UnknownLive)?;
        self.stage_live(&plan, index, Some(value))
    }

    /// Stage a continuous output of a source. None revokes an unavailable reading.
    pub fn publish_live_port(
        &mut self,
        node: NodeId,
        port: &orange_model::id::PortKey,
        value: Option<Value>,
    ) -> Result<bool, IngressError> {
        let plan = self.live_plan()?;
        let index = plan
            .live_port_index(node, port)
            .ok_or(IngressError::UnknownLive)?;
        self.stage_live(&plan, index, value)
    }

    fn live_plan(&self) -> Result<Arc<ControlPlan>, IngressError> {
        if !matches!(self.state, RunState::Running | RunState::Paused) {
            return Err(IngressError::NotRunning);
        }
        self.plan.clone().ok_or(IngressError::NotRunning)
    }

    fn stage_live(
        &mut self,
        plan: &ControlPlan,
        index: usize,
        value: Option<Value>,
    ) -> Result<bool, IngressError> {
        let ty = plan.lives()[index].ty();
        if value.as_ref().is_some_and(|value| !ty.accepts(value)) {
            return Err(IngressError::InvalidPayload);
        }
        let pending = self.pending_lives.get(&index);
        if pending.unwrap_or(&self.lives[index]) == &value {
            return Ok(false);
        }
        let limit = self.options.limits.max_state_bytes;
        let charge = value
            .as_ref()
            .map_or(Ok(0), |value| ty.checked_charge(value, limit))
            .map_err(|_| IngressError::DeliveryLimit)?;
        let old = pending
            .and_then(Option::as_ref)
            .and_then(|held| ty.checked_charge(held, usize::MAX).ok())
            .unwrap_or(0);
        let total = self
            .live_bytes
            .get()
            .checked_sub(old)
            .and_then(|n| n.checked_add(charge))
            .filter(|n| *n <= limit)
            .ok_or(IngressError::DeliveryLimit)?;
        if !self.live_bytes.try_set(total) {
            return Err(IngressError::DeliveryLimit);
        }
        let _ = self.pending_lives.insert(index, value);
        Ok(true)
    }

    pub(super) fn can_apply_live(&self) -> bool {
        !self.watches.active && !self.pending_lives.is_empty()
    }

    pub(super) fn apply_live(&mut self, plan: &ControlPlan) -> Result<(), RunError> {
        let (index, value) = self
            .pending_lives
            .pop_first()
            .ok_or(RunError::InvalidState)?;
        let changed = self.lives[index] != value;
        if let Some(old) = std::mem::replace(&mut self.lives[index], value) {
            let charge = plan.lives()[index]
                .ty()
                .checked_charge(&old, usize::MAX)
                .map_err(|_| RunError::InvalidState)?;
            let _ = self.live_bytes.try_set(self.live_bytes.get() - charge);
        }
        if changed {
            self.watches.schedule_live(plan, index);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orange_compiler::FaultText;

    #[test]
    fn deadband_distance_is_strict_and_handles_integer_extremes() {
        assert!(!exceeds_deadband(
            &Value::I32(-1),
            &Value::I32(1),
            &Value::I32(2)
        ));
        assert!(exceeds_deadband(
            &Value::I64(i64::MIN),
            &Value::I64(i64::MAX),
            &Value::I64(i64::MAX)
        ));
        assert!(!exceeds_deadband(
            &Value::U64(u64::MAX),
            &Value::U64(0),
            &Value::U64(u64::MAX)
        ));
        assert!(exceeds_deadband(
            &Value::F64(1.0),
            &Value::F64(0.49),
            &Value::F64(0.5)
        ));
    }

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
