//! The run's bounded control trace. Recording is the only write, so step code that may
//! trace borrows this alone instead of the run.
use super::RunEvent;
use crate::budget::{Budget, Tally};
use std::collections::VecDeque;
use std::sync::Arc;

/// Position in one run's bounded trace. Start with the default for each new run.
#[derive(Debug, Default, Clone, Copy)]
pub struct TraceCursor {
    admitted: u64,
    refused: u64,
}

pub(super) struct Trace {
    events: VecDeque<RunEvent>,
    evicted: u64,
    position: TraceCursor,
    text: Tally,
}

impl Trace {
    pub(crate) fn new(budget: Arc<Budget>) -> Self {
        Self {
            events: VecDeque::new(),
            evicted: 0,
            position: TraceCursor::default(),
            text: Tally::new(Some(budget), 0),
        }
    }
    fn text_bytes(event: &RunEvent) -> usize {
        let route = event
            .route
            .as_ref()
            .map_or(0, |port| port.port().as_str().len());
        route.saturating_add(event.fault.as_ref().map_or(0, |fault| {
            fault
                .fault
                .code
                .retained_bytes()
                .saturating_add(fault.fault.message_key.retained_bytes())
        }))
    }
    /// Appends `event`, dropping the oldest once `max` events are kept.
    pub(crate) fn record(&mut self, event: RunEvent, max: usize) {
        if max == 0 {
            self.refuse();
            return;
        }
        if self.events.len() == max {
            if let Some(old) = self.events.pop_front() {
                self.text.set(self.text.get() - Self::text_bytes(&old));
            }
            self.evicted = self.evicted.saturating_add(1);
        }
        let Some(bytes) = self.text.get().checked_add(Self::text_bytes(&event)) else {
            self.refuse();
            return;
        };
        if !self.text.try_set(bytes) {
            self.refuse();
            return;
        }
        self.events.push_back(event);
        self.position.admitted = self.position.admitted.saturating_add(1);
    }
    fn refuse(&mut self) {
        self.position.refused = self.position.refused.saturating_add(1);
    }
    pub(crate) fn events(&self) -> &VecDeque<RunEvent> {
        &self.events
    }
    pub(crate) fn dropped(&self) -> u64 {
        self.evicted.saturating_add(self.position.refused)
    }
    pub(crate) fn since(&self, cursor: &mut TraceCursor) -> (u64, impl Iterator<Item = &RunEvent>) {
        let previous = std::mem::replace(cursor, self.position);
        let admitted = self.position.admitted.saturating_sub(previous.admitted);
        let refused = self.position.refused.saturating_sub(previous.refused);
        let kept = admitted.min(self.events.len() as u64) as usize;
        let lost = refused.saturating_add(admitted - kept as u64);
        (lost, self.events.iter().skip(self.events.len() - kept))
    }
    /// Drops the kept events and releases their storage; the dropped count stays.
    pub(crate) fn release(&mut self) {
        self.events = VecDeque::new();
        self.text.set(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ControlOutcome;
    use orange_compiler::{ConstantFault, FaultText, KernelFault};

    #[test]
    fn cursor_counts_refusals_and_evictions_without_repeating_old_events() {
        let mut trace = Trace::new(Budget::new(0));
        let mut cursor = TraceCursor::default();
        let event = |activation, route| RunEvent {
            route,
            activation,
            node: "10000000-0000-4000-8000-000000000001".parse().unwrap(),
            outcome: ControlOutcome::Then,
            fault: None,
        };
        trace.record(event(1, None), 2);
        let (lost, events) = trace.since(&mut cursor);
        assert_eq!(lost, 0);
        assert_eq!(
            events.map(|event| event.activation).collect::<Vec<_>>(),
            [1]
        );

        trace.record(event(2, Some("then".parse().unwrap())), 2);
        let (lost, events) = trace.since(&mut cursor);
        assert_eq!((lost, events.count()), (1, 0));

        for activation in 3..=5 {
            trace.record(event(activation, None), 2);
        }
        let (lost, events) = trace.since(&mut cursor);
        assert_eq!(lost, 1);
        assert_eq!(
            events.map(|event| event.activation).collect::<Vec<_>>(),
            [4, 5]
        );
    }

    #[test]
    fn shared_trace_text_obeys_the_run_budget_and_releases_on_eviction() {
        let node = "10000000-0000-4000-8000-000000000001".parse().unwrap();
        let code: Arc<str> = Arc::from("acme/Failed");
        let weak = Arc::downgrade(&code);
        let event = RunEvent {
            route: None,
            activation: 1,
            node,
            outcome: ControlOutcome::Failed,
            fault: Some(ConstantFault {
                source: node,
                fault: KernelFault {
                    code: FaultText::Shared(code),
                    message_key: "acme/failed".into(),
                    message: "The node failed.",
                },
            }),
        };
        let bytes = Trace::text_bytes(&event);
        let budget = Budget::new(bytes);
        let mut trace = Trace::new(budget.clone());
        trace.record(event.clone(), 2);
        assert_eq!(budget.used(), bytes);
        trace.record(event, 2);
        assert_eq!(trace.events().len(), 1);
        assert_eq!(trace.dropped(), 1);
        let static_event = RunEvent {
            route: None,
            activation: 2,
            node,
            outcome: ControlOutcome::Then,
            fault: None,
        };
        trace.record(static_event, 1);
        assert_eq!(budget.used(), 0);
        assert!(weak.upgrade().is_none());
        trace.release();
        assert!(trace.events().is_empty());
    }
}
