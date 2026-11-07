//! Waiting strands, ordered by wake time and then by arrival. The one owner of the timer
//! order and of the strand-to-timer directory, which must change together.
use super::Timer;
use super::ids::IdMap;
use super::ids::StrandId;
use std::collections::BTreeMap;

/// Wake time, then a sequence that keeps equal wake times in arrival order.
pub(super) type TimerKey = (u64, u64);

#[derive(Default)]
pub(super) struct Timers {
    next: u64,
    by_strand: IdMap<StrandId, TimerKey>,
    by_awaiter: IdMap<u64, TimerKey>,
    /// The armed tick of each timer entry, by its index in `ControlPlan::timers()`.
    by_entry: BTreeMap<usize, TimerKey>,
    order: BTreeMap<TimerKey, TimerTarget>,
}

/// Timer ownership remains unique: ordinary waits own their strand here, while an
/// Await timeout records only its registration token (the waiter table owns the strand)
/// and a timer entry only its index and scheduled time (the run owns its cursor).
#[expect(
    clippy::large_enum_variant,
    reason = "Boxing would add a per-Wait allocation to the existing parked-strand path."
)]
pub(super) enum TimerTarget {
    Wait(Timer),
    Await { run: u64, token: u64 },
    Entry { run: u64, timer: usize, at: u64 },
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
        if self.order.len() >= max {
            return Err(TimerLimit);
        }
        let key = self.next_key(at)?;
        self.by_strand.insert(timer.activation.id, key);
        self.order.insert(key, TimerTarget::Wait(timer));
        Ok(())
    }
    pub fn schedule_await(
        &mut self,
        at: u64,
        run: u64,
        token: u64,
        max: usize,
    ) -> Result<TimerKey, TimerLimit> {
        if self.order.len() >= max {
            return Err(TimerLimit);
        }
        let key = self.next_key(at)?;
        self.by_awaiter.insert(token, key);
        self.order.insert(key, TimerTarget::Await { run, token });
        Ok(key)
    }
    /// Arms timer entry `timer`'s next tick at `at`.
    pub fn schedule_entry(
        &mut self,
        at: u64,
        run: u64,
        timer: usize,
        max: usize,
    ) -> Result<(), TimerLimit> {
        if self.order.len() >= max {
            return Err(TimerLimit);
        }
        let key = self.next_key(at)?;
        self.by_entry.insert(timer, key);
        self.order
            .insert(key, TimerTarget::Entry { run, timer, at });
        Ok(())
    }
    /// Armed timer entries; they are schedule, not parked work.
    pub fn entry_count(&self) -> usize {
        self.by_entry.len()
    }
    /// Disarms every timer entry: a stopping run admits no further ticks.
    pub fn remove_entries(&mut self) {
        for key in std::mem::take(&mut self.by_entry).into_values() {
            self.order.remove(&key);
        }
    }
    fn next_key(&mut self, at: u64) -> Result<TimerKey, TimerLimit> {
        let sequence = self.next;
        self.next = sequence.checked_add(1).ok_or(TimerLimit)?;
        Ok((at, sequence))
    }
    /// Removes and returns the earliest timer if it is due at `now`.
    pub fn pop_due(&mut self, now: u64) -> Option<TimerTarget> {
        if !self.is_due(now) {
            return None;
        }
        let (_, timer) = self.order.pop_first()?;
        match &timer {
            TimerTarget::Wait(timer) => {
                self.by_strand.remove(&timer.activation.id);
            }
            TimerTarget::Await { token, .. } => {
                self.by_awaiter.remove(token);
            }
            TimerTarget::Entry { timer, .. } => {
                self.by_entry.remove(timer);
            }
        }
        Some(timer)
    }
    /// Detaches `strand`'s timer from the directory; [`remove`](Self::remove) takes it out
    /// of the order. Cancellation collects keys first so it can retire them in order.
    pub fn detach(&mut self, strand: StrandId) -> Option<TimerKey> {
        self.by_strand.remove(&strand)
    }
    pub fn detach_await(&mut self, token: u64) -> Option<TimerKey> {
        self.by_awaiter.remove(&token)
    }
    pub fn remove(&mut self, key: TimerKey) -> Option<TimerTarget> {
        self.order.remove(&key)
    }
    pub fn clear(&mut self) {
        self.order.clear();
        self.by_strand.clear();
        self.by_awaiter.clear();
        self.by_entry.clear();
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
