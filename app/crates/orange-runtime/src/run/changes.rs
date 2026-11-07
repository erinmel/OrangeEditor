//! Variable-changed notifications (C06): a run variable watched by a root
//! `core/on-variable-changed` entry keeps its value from before the first write of each
//! segment. At segment end the run compares it with the final value under the type's host
//! `eq` and admits one coalesced delivery per watching entry, in handler order (11 §9.2).
use super::events::{EventAdmission, EventOrigin};
use super::{Run, RunState};
use orange_compiler::{ChangeProjection, ControlPlan, HostSource};
use orange_model::Value;
use orange_model::value::ListBuilder;
use std::sync::Arc;

#[derive(Default)]
pub(super) struct Changes {
    /// Per plan variable: some entry watches it, so its writes capture a previous value.
    watched: Box<[bool]>,
    /// Per plan variable: the value before the segment's first write, once captured.
    previous: Box<[Option<Option<Value>>]>,
    /// Captured variables in first-write order, so ending a segment visits only writes.
    touched: Vec<usize>,
    /// Admitted deliveries per change entry, used as its source sequence.
    pub(super) sequences: Box<[u64]>,
    /// The segment owner's activation sequence and causal depth.
    cause: (u64, u16),
}
impl Changes {
    pub(super) fn new(plan: &ControlPlan) -> Self {
        let mut watched = vec![false; plan.variables().len()];
        for change in plan.changes() {
            watched[change.variable()] = true;
        }
        for watch in plan.watches() {
            for &variable in watch.variables() {
                watched[variable] = true;
            }
        }
        Self {
            watched: watched.into(),
            previous: vec![None; plan.variables().len()].into(),
            touched: Vec::new(),
            sequences: vec![0; plan.changes().len()].into(),
            cause: (0, 0),
        }
    }
    /// Called before a run variable is stored; only the first write of a segment counts.
    pub(super) fn capture(&mut self, variable: usize, current: Option<&Value>) {
        if self.watched.get(variable) == Some(&true) && self.previous[variable].is_none() {
            self.previous[variable] = Some(current.cloned());
            self.touched.push(variable);
        }
    }
    pub(super) fn open(&mut self, cause: (u64, u16)) {
        self.cause = cause;
    }
    pub(super) fn release(&mut self) {
        *self = Self::default();
    }
}

/// `[value, previous]` as the channel's `list<T>` payload.
pub(super) fn pair(
    plan: &ControlPlan,
    variable: usize,
    value: Value,
    previous: Value,
) -> Option<Value> {
    let mut list = ListBuilder::new(Some(plan.variables()[variable].ty()));
    list.reserve(2).ok()?;
    list.push(value).ok()?;
    list.push(previous).ok()?;
    Some(Value::List(Arc::new(list.finish())))
}

impl Run {
    /// Ends the current segment: compare every captured variable and admit one delivery
    /// per changed watched variable and entry. Refusals count as overflows and never fault
    /// the run, because no graph node asked for the notification.
    pub(super) fn end_segment(&mut self, plan: &ControlPlan) {
        if self.changes.touched.is_empty() {
            return;
        }
        let touched = std::mem::take(&mut self.changes.touched);
        let mut changed = Vec::new();
        for &variable in &touched {
            let Some(previous) = self.changes.previous[variable].take() else {
                continue;
            };
            let current = &self.variables[self.layout.variables[variable]];
            if let (Some(previous), Some(current)) = (previous, current)
                && plan.variables()[variable]
                    .ty()
                    .equal_values(current, &previous)
                    == Some(false)
            {
                changed.push((variable, current.clone(), previous));
            }
        }
        self.changes.touched = touched;
        self.changes.touched.clear();
        // Accepted graph work only: Draining and cancellation take no new triggers.
        if changed.is_empty() || self.state != RunState::Running {
            return;
        }
        let (cause, depth) = self.changes.cause;
        if !plan.watches().is_empty() {
            if !self.watches.schedule(
                plan,
                &changed,
                (cause, depth),
                self.options.limits.max_state_bytes,
            ) {
                self.events.count_overflow();
            }
            return;
        }
        let depth = depth
            .checked_add(1)
            .filter(|depth| *depth <= self.options.max_cause_depth);
        let first = plan.events().len() + plan.manuals().len() + plan.timers().len();
        for (entry, change) in plan.changes().iter().enumerate() {
            let Some((variable, value, previous)) = changed
                .iter()
                .find(|(variable, _, _)| *variable == change.variable())
            else {
                continue;
            };
            let admitted = depth.zip(self.changes.sequences[entry].checked_add(1));
            let payload = pair(plan, *variable, value.clone(), previous.clone());
            let (Some((depth, sequence)), Some(payload)) = (admitted, payload) else {
                self.events.count_overflow();
                continue;
            };
            let admission = EventAdmission {
                event: first + entry,
                origin: EventOrigin::Change,
                cause: Some(cause),
                cause_depth: depth,
                source_seq: sequence,
                wall_ns: None,
                graph_source: None,
                tick: None,
            };
            match self.admit_event(plan, &payload, admission) {
                Ok(_) => self.changes.sequences[entry] = sequence,
                Err(_) => self.events.count_overflow(),
            }
        }
    }
}

/// The output a delivered change publishes for one host source.
pub(super) fn project(payload: &Value, source: HostSource) -> Option<Value> {
    let Value::List(items) = payload else {
        return None;
    };
    match source {
        HostSource::Change(_, ChangeProjection::Value) => items.get(0),
        HostSource::Change(_, ChangeProjection::Previous) => items.get(1),
        _ => None,
    }
}
