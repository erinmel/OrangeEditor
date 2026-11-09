//! The run's bounded control trace. Recording is the only write, so step code that may
//! trace borrows this alone instead of the run.
use super::RunEvent;
use crate::budget::{Budget, Tally};
use std::collections::VecDeque;
use std::sync::Arc;

pub(super) struct Trace {
    events: VecDeque<RunEvent>,
    dropped: u64,
    text: Tally,
}

impl Trace {
    pub fn new(budget: Arc<Budget>) -> Self {
        Self {
            events: VecDeque::new(),
            dropped: 0,
            text: Tally::new(Some(budget), 0),
        }
    }
    fn text_bytes(event: &RunEvent) -> usize {
        event.fault.as_ref().map_or(0, |fault| {
            fault
                .fault
                .code
                .retained_bytes()
                .saturating_add(fault.fault.message_key.retained_bytes())
        })
    }
    /// Appends `event`, dropping the oldest once `max` events are kept.
    pub fn record(&mut self, event: RunEvent, max: usize) {
        if max == 0 {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        if self.events.len() == max {
            if let Some(old) = self.events.pop_front() {
                self.text.set(self.text.get() - Self::text_bytes(&old));
            }
            self.dropped = self.dropped.saturating_add(1);
        }
        let Some(bytes) = self.text.get().checked_add(Self::text_bytes(&event)) else {
            self.dropped = self.dropped.saturating_add(1);
            return;
        };
        if !self.text.try_set(bytes) {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.events.push_back(event);
    }
    pub fn events(&self) -> &VecDeque<RunEvent> {
        &self.events
    }
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
    /// Drops the kept events and releases their storage; the dropped count stays.
    pub fn release(&mut self) {
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
    fn shared_trace_text_obeys_the_run_budget_and_releases_on_eviction() {
        let node = "10000000-0000-4000-8000-000000000001".parse().unwrap();
        let code: Arc<str> = Arc::from("acme/Failed");
        let weak = Arc::downgrade(&code);
        let event = RunEvent {
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
