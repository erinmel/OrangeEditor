//! The run's bounded control trace. Recording is the only write, so step code that may
//! trace borrows this alone instead of the run.
use super::RunEvent;
use std::collections::VecDeque;

#[derive(Default)]
pub(super) struct Trace {
    events: VecDeque<RunEvent>,
    dropped: u64,
}

impl Trace {
    /// Appends `event`, dropping the oldest once `max` events are kept.
    pub fn record(&mut self, event: RunEvent, max: usize) {
        if max == 0 {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        if self.events.len() == max {
            self.events.pop_front();
            self.dropped = self.dropped.saturating_add(1);
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
    }
}
