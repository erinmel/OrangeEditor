//! When a session's background thread may start and must stop, without polling.

use std::sync::{Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Whether a session's background work may begin and whether it must end, shared by the
/// session and its thread. Waiting on it wakes as soon as either changes, so a thread
/// does not sleep in small steps to notice them.
#[derive(Debug, Default)]
pub struct Lifecycle {
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Debug, Default, Clone, Copy)]
struct State {
    started: bool,
    stopped: bool,
}

impl Lifecycle {
    pub fn new() -> Self {
        Self::default()
    }

    /// A flow attached its triggers: the work may begin.
    pub fn start(&self) {
        self.update(|state| state.started = true);
    }

    /// The session ended: the work must stop. Stopping is final.
    pub fn stop(&self) {
        self.update(|state| state.stopped = true);
    }

    pub fn started(&self) -> bool {
        self.state().started
    }

    pub fn stopped(&self) -> bool {
        self.state().stopped
    }

    /// Waits until the work is started or stopped; `true` when it may begin.
    pub fn wait_started(&self) -> bool {
        let mut state = self.lock();
        while !state.started && !state.stopped {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        !state.stopped
    }

    /// Waits `time`, or less if the work is stopped first; `true` when still running.
    pub fn rest(&self, time: Duration) -> bool {
        let deadline = Instant::now() + time;
        let mut state = self.lock();
        while !state.stopped {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            state = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        !state.stopped
    }

    fn update(&self, change: impl FnOnce(&mut State)) {
        change(&mut self.lock());
        self.changed.notify_all();
    }

    fn state(&self) -> State {
        *self.lock()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn waiting_wakes_on_start_and_stop_not_on_a_timer() {
        let life = Arc::new(Lifecycle::new());
        let waiter = {
            let life = Arc::clone(&life);
            thread::spawn(move || life.wait_started())
        };
        life.start();
        assert!(waiter.join().unwrap());

        let stopped = Arc::new(Lifecycle::new());
        let waiter = {
            let stopped = Arc::clone(&stopped);
            thread::spawn(move || stopped.wait_started())
        };
        stopped.stop();
        assert!(!waiter.join().unwrap());
    }

    #[test]
    fn resting_ends_early_when_stopped() {
        let life = Arc::new(Lifecycle::new());
        assert!(life.rest(Duration::from_millis(1)));
        let resting = {
            let life = Arc::clone(&life);
            thread::spawn(move || {
                let began = Instant::now();
                (life.rest(Duration::from_secs(60)), began.elapsed())
            })
        };
        life.stop();
        let (running, waited) = resting.join().unwrap();
        assert!(!running);
        assert!(waited < Duration::from_secs(30));
        assert!(life.stopped() && !life.rest(Duration::from_secs(1)));
    }
}
