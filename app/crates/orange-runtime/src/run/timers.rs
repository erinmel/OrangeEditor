//! Waiting strands, ordered by wake time and then by arrival. The one owner of the timer
//! order and of the strand-to-timer directory, which must change together.
use super::Timer;
use super::ids::StrandId;
use std::collections::{BTreeMap, HashMap};

/// Wake time, then a sequence that keeps equal wake times in arrival order.
pub(super) type TimerKey = (u64, u64);

#[derive(Default)]
pub(super) struct Timers {
    next: u64,
    by_strand: HashMap<StrandId, TimerKey>,
    order: BTreeMap<TimerKey, Timer>,
}

/// [`Timers::schedule`] refused a timer: the run's timer limit or sequence is exhausted.
pub(super) struct TimerLimit;

impl Timers {
    pub fn len(&self) -> usize {
        self.order.len()
    }
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
    /// The earliest wake time.
    pub fn next_at(&self) -> Option<u64> {
        self.order.first_key_value().map(|((at, _), _)| *at)
    }
    pub fn is_due(&self, now: u64) -> bool {
        self.next_at().is_some_and(|at| at <= now)
    }
    /// Parks `timer` until `at`, unless the run already holds `max` timers.
    pub fn schedule(&mut self, at: u64, timer: Timer, max: usize) -> Result<(), TimerLimit> {
        if self.order.len() == max {
            return Err(TimerLimit);
        }
        let sequence = self.next;
        self.next = sequence.checked_add(1).ok_or(TimerLimit)?;
        self.by_strand.insert(timer.activation.id, (at, sequence));
        self.order.insert((at, sequence), timer);
        Ok(())
    }
    /// Removes and returns the earliest timer if it is due at `now`.
    pub fn pop_due(&mut self, now: u64) -> Option<Timer> {
        if !self.is_due(now) {
            return None;
        }
        let (_, timer) = self.order.pop_first()?;
        self.by_strand.remove(&timer.activation.id);
        Some(timer)
    }
    /// Detaches `strand`'s timer from the directory; [`remove`](Self::remove) takes it out
    /// of the order. Cancellation collects keys first so it can retire them in order.
    pub fn detach(&mut self, strand: StrandId) -> Option<TimerKey> {
        self.by_strand.remove(&strand)
    }
    pub fn remove(&mut self, key: TimerKey) -> Option<Timer> {
        self.order.remove(&key)
    }
    pub fn clear(&mut self) {
        self.order.clear();
        self.by_strand.clear();
    }
    /// Drops all timers and releases their storage.
    pub fn release(&mut self) {
        let next = self.next;
        *self = Self {
            next,
            ..Self::default()
        };
    }
}
