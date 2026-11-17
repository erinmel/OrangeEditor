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
    /// A selected catch-up window remains fixed until all its attempts are charged.
    pub pending: Option<TickWindow>,
}

/// The immutable clock window selected when an entry first became due.
#[derive(Debug, Clone, Copy)]
pub(super) struct TickWindow {
    pub next: u64,
    pub last: u64,
    pub latest_at: u64,
    pub interval: u64,
}
impl Default for TickCursor {
    fn default() -> Self {
        Self {
            next: 1,
            admitted: 0,
            pending: None,
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

    /// Performs at most one timer admission attempt. The pump checks cancellation at the
    /// next actor boundary, and the timer heap retains its original ordering key while a
    /// selected catch-up window is pending.
    ///
    /// `missed` on a delivered tick counts the scheduled ticks since the previous admitted
    /// one that were not admitted: late ticks the missed-tick policy drops, and ticks a
    /// delivery limit refused. Handler policy drops after admission are event statistics.
    pub(super) fn fire_timer_entry(
        &mut self,
        index: usize,
        at: u64,
        key: super::timers::TimerKey,
    ) -> Result<usize, RunError> {
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
        if let Some(window) = cursor.pending {
            return self.admit_timer_window_tick(index, at, key, window, schedule);
        }
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
        let (first, count) = match policy {
            MissedTicks::Coalesce => (last, 1),
            // One due tick was handled before the next came due, however late within
            // its interval. Several due ticks mean the run fell behind: drop them all.
            MissedTicks::Skip => (last, u64::from(due == 1)),
            MissedTicks::CatchUp(max) => {
                let count = due.min(u64::from(max.get()));
                (last + 1 - count, count)
            }
        };
        if count == 0 {
            self.events.set_tick_next(index, last + 1)?;
            self.rearm_timer_entry(index, latest_at, interval, schedule)?;
            return Ok(1);
        }
        let window = TickWindow {
            next: first,
            last,
            latest_at,
            interval,
        };
        self.events.set_tick_next(index, last + 1)?;
        self.admit_timer_window_tick(index, at, key, window, schedule)
    }

    fn admit_timer_window_tick(
        &mut self,
        index: usize,
        at: u64,
        key: super::timers::TimerKey,
        mut window: TickWindow,
        schedule: TimerSchedule,
    ) -> Result<usize, RunError> {
        let plan = self.plan.as_ref().ok_or(RunError::InvalidState)?.clone();
        let event = plan.events().len() + plan.manuals().len() + index;
        let ordinal = window.next;
        let scheduled_at = window.latest_at - (window.last - ordinal) * window.interval;
        let admitted = self.events.tick_cursor(index)?.admitted;
        let tick = Tick {
            scheduled_at: i64::try_from(scheduled_at).map_err(|_| RunError::InvalidState)?,
            missed: ordinal - admitted - 1,
        };
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
            self.events.count_overflow();
        }
        window.next = window.next.checked_add(1).ok_or(RunError::ResourceLimit)?;
        if window.next <= window.last {
            self.events.set_tick_pending(index, Some(window))?;
            self.timers.requeue_entry(key, self.id, index, at);
        } else {
            self.events.set_tick_pending(index, None)?;
            self.rearm_timer_entry(index, window.latest_at, window.interval, schedule)?;
        }
        Ok(1)
    }

    fn rearm_timer_entry(
        &mut self,
        index: usize,
        latest_at: u64,
        interval: u64,
        schedule: TimerSchedule,
    ) -> Result<(), RunError> {
        if matches!(schedule, TimerSchedule::Interval { .. })
            && let Some(next_at) = latest_at.checked_add(interval).filter(|t| *t <= HORIZON)
        {
            self.timers
                .schedule_entry(next_at, self.id, index, self.options.limits.max_timers)
                .map_err(|_| RunError::ResourceLimit)?;
        }
        Ok(())
    }
}
