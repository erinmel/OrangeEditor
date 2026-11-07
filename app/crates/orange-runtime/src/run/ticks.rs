//! Timer entries (11 §10.2): each root `core/on-timer`/`core/on-timeout` keeps one entry in
//! the run's timer order. When it is due, the actor admits the entry's due ticks through
//! the shared handler admission and re-arms the next scheduled tick. The schedule is
//! `offset + k * interval` on the run's virtual clock, so it never drifts.
use super::events::{EventAdmission, EventOrigin};
use super::*;
use orange_model::catalog::{MissedTicks, TimerFirst, TimerSchedule};

/// The values one delivered tick publishes on its entry's outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Tick {
    pub scheduled_at: i64,
    pub missed: u64,
}

/// Where one timer entry is in its schedule.
#[derive(Debug, Clone, Copy)]
pub(super) struct TickCursor {
    /// Ordinal (from one) of the next scheduled tick; it is that tick's `source_seq`.
    pub next: u64,
    /// Ordinal of the last admitted tick, zero before the first.
    pub admitted: u64,
}
impl Default for TickCursor {
    fn default() -> Self {
        Self {
            next: 1,
            admitted: 0,
        }
    }
}

/// Scheduled time of the first tick, as an offset from the run clock's zero.
fn first_at(schedule: TimerSchedule) -> u64 {
    match schedule {
        TimerSchedule::Interval {
            first: TimerFirst::Immediate,
            ..
        } => 0,
        TimerSchedule::Interval {
            interval_ns,
            first: TimerFirst::AfterInterval,
            ..
        } => interval_ns.get(),
        TimerSchedule::Timeout { delay_ns } => delay_ns,
    }
}

/// Latest scheduled time representable as a `core/duration@1`.
const HORIZON: u64 = i64::MAX as u64;

impl Run {
    /// Arms the first tick of every timer entry when the run enters Running.
    pub(super) fn arm_timer_entries(&mut self) -> Result<(), RunError> {
        let plan = self.plan.as_ref().ok_or(RunError::InvalidState)?.clone();
        for (index, timer) in plan.timers().iter().enumerate() {
            self.timers
                .schedule_entry(
                    first_at(timer.schedule()),
                    self.id,
                    index,
                    self.options.limits.max_timers,
                )
                .map_err(|_| RunError::ResourceLimit)?;
        }
        Ok(())
    }

    /// Admits the ticks of timer entry `index` that are due now, oldest first, then
    /// re-arms its next scheduled tick. `at` is the due tick's scheduled time. Returns the
    /// number of admission attempts, which the pump charges as work.
    ///
    /// `missed` on a delivered tick counts the scheduled ticks since the previous admitted
    /// one that were not admitted: late ticks the missed-tick policy drops, and ticks a
    /// delivery limit refused. Handler policy drops after admission are event statistics.
    pub(super) fn fire_timer_entry(&mut self, index: usize, at: u64) -> Result<usize, RunError> {
        if self.state != RunState::Running {
            return Ok(1);
        }
        let plan = self.plan.as_ref().ok_or(RunError::InvalidState)?.clone();
        let schedule = plan
            .timers()
            .get(index)
            .ok_or(RunError::InvalidState)?
            .schedule();
        let cursor = self.events.tick_cursor(index)?;
        // Due ticks: every scheduled time from `at` up to now, within the duration range.
        let horizon = self.now.min(HORIZON);
        let (interval, due, policy) = match schedule {
            TimerSchedule::Interval {
                interval_ns,
                missed,
                ..
            } => {
                let interval = interval_ns.get();
                let elapsed = horizon.checked_sub(at).ok_or(RunError::InvalidState)?;
                (interval, elapsed / interval + 1, missed)
            }
            TimerSchedule::Timeout { .. } => (0, 1, MissedTicks::Coalesce),
        };
        let last = cursor
            .next
            .checked_add(due - 1)
            .ok_or(RunError::ResourceLimit)?;
        // At most `horizon`, so neither this nor the earlier tick times overflow.
        let latest_at = at + (due - 1) * interval;
        let count = match policy {
            MissedTicks::Coalesce => 1,
            MissedTicks::Skip => u64::from(latest_at == self.now),
            MissedTicks::CatchUp(max) => due.min(u64::from(max.get())),
        };
        let event = plan.events().len() + plan.manuals().len() + index;
        let mut attempts = 0usize;
        for ordinal in (last + 1 - count)..=last {
            let scheduled_at = latest_at - (last - ordinal) * interval;
            let admitted = self.events.tick_cursor(index)?.admitted;
            let tick = Tick {
                scheduled_at: i64::try_from(scheduled_at).map_err(|_| RunError::InvalidState)?,
                missed: ordinal - admitted - 1,
            };
            attempts = attempts.saturating_add(1);
            let admission = EventAdmission {
                event,
                origin: EventOrigin::Timer,
                cause: None,
                cause_depth: 0,
                source_seq: ordinal,
                wall_ns: None,
                graph_source: None,
                tick: Some(tick),
            };
            if self.admit_event(&plan, &Value::Unit, admission).is_ok() {
                self.events.set_tick_admitted(index, ordinal)?;
            } else {
                // A refused tick is not a run fault: count the overflow and report the
                // tick as missed on the next admitted one.
                self.events.count_overflow();
            }
        }
        self.events.set_tick_next(index, last + 1)?;
        if let TimerSchedule::Interval { .. } = schedule
            && let Some(next_at) = latest_at.checked_add(interval).filter(|t| *t <= HORIZON)
        {
            self.timers
                .schedule_entry(next_at, self.id, index, self.options.limits.max_timers)
                .map_err(|_| RunError::ResourceLimit)?;
        }
        Ok(attempts.max(1))
    }
}
