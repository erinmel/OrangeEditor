//! Instance-owned cancellation over a shared engine's epoch wakeups.
use crate::{GuestEngine, GuestError, GuestTrap};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use wasmtime::{Store, UpdateDeadline};

struct State {
    cancelled: AtomicBool,
    broadcast_seen: AtomicU64,
    #[cfg(test)]
    epochs: AtomicU64,
}

/// A monotonic cancellation handle for exactly one guest instance's lifetime.
/// Clones never retarget another instance. Cancellation is terminal; discard the
/// instance after Interrupted/Poisoned. The host must periodically advance the
/// engine's epochs while guest calls run. This cannot preempt a blocked host import.
#[derive(Clone)]
pub struct GuestCancellation {
    engine: GuestEngine,
    state: Arc<State>,
}
impl std::fmt::Debug for GuestCancellation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuestCancellation")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}
impl GuestCancellation {
    pub(crate) fn install<T: 'static>(
        engine: &GuestEngine,
        store: &mut Store<T>,
    ) -> Result<Self, GuestError> {
        let control = Self {
            engine: engine.clone(),
            state: Arc::new(State {
                cancelled: AtomicBool::new(false),
                broadcast_seen: AtomicU64::new(engine.broadcast.load(Ordering::SeqCst)),
                #[cfg(test)]
                epochs: AtomicU64::new(0),
            }),
        };
        let callback = control.clone();
        store.epoch_deadline_callback(move |_| {
            #[cfg(test)]
            callback.state.epochs.fetch_add(1, Ordering::SeqCst);
            Ok(if callback.interrupted() {
                UpdateDeadline::Interrupt
            } else {
                UpdateDeadline::Continue(1)
            })
        });
        control.arm(store)?;
        Ok(control)
    }

    /// Signal cancellation before waking epoch checks. Other instances sharing
    /// the engine continue unless separately cancelled or explicitly broadcast.
    /// A single wakeup can race deadline renewal; the host keeps advancing epochs.
    pub fn cancel(&self) {
        if !self.state.cancelled.swap(true, Ordering::SeqCst) {
            self.engine.engine.increment_epoch();
        }
    }
    /// Whether this handle received a local cancellation request. An engine-wide
    /// broadcast is reported by the instance's error, not by this local flag.
    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::SeqCst)
    }
    fn interrupted(&self) -> bool {
        self.is_cancelled()
            || self.engine.broadcast.load(Ordering::SeqCst)
                != self.state.broadcast_seen.load(Ordering::SeqCst)
    }
    pub(crate) fn arm<T>(&self, store: &mut Store<T>) -> Result<(), GuestError> {
        self.state.broadcast_seen.store(
            self.engine.broadcast.load(Ordering::SeqCst),
            Ordering::SeqCst,
        );
        store.set_epoch_deadline(1);
        // A cancellation/broadcast between arming and this check must not be
        // hidden by renewing the deadline after its epoch wakeup already occurred.
        if self.interrupted() {
            Err(GuestError::Trap(GuestTrap::Interrupted))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GuestInstance, GuestLimits};
    use std::time::{Duration, Instant};

    const SPIN: &str = r#"(component
      (core module $m
        (func (export "spin") (result i64) (loop $l br $l) i64.const 0)
        (func (export "value") (result i64) i64.const 42))
      (core instance $i (instantiate $m))
      (func (export "spin") (result s64) (canon lift (core func $i "spin")))
      (func (export "value") (result s64) (canon lift (core func $i "value"))))"#;

    struct Cleanup(Vec<GuestCancellation>);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            for control in &self.0 {
                control.cancel();
            }
        }
    }
    struct Ticker {
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl Ticker {
        fn new(engine: GuestEngine) -> Self {
            let stop = Arc::new(AtomicBool::new(false));
            let signal = stop.clone();
            let thread = std::thread::spawn(move || {
                while !signal.load(Ordering::SeqCst) {
                    engine.advance_epoch();
                    std::thread::yield_now();
                }
            });
            Self {
                stop,
                thread: Some(thread),
            }
        }
    }
    impl Drop for Ticker {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            self.thread.take().unwrap().join().unwrap();
        }
    }
    fn progressed(control: &GuestCancellation, previous: u64) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while control.state.epochs.load(Ordering::SeqCst) <= previous {
            assert!(
                Instant::now() < deadline,
                "guest did not reach its epoch callback"
            );
            // A wakeup is not a broadcast cancellation. Observing a callback proves
            // this store is executing guest code, without sleep-based assumptions.
            std::thread::yield_now();
        }
    }
    fn finished<T>(worker: &std::thread::ScopedJoinHandle<'_, T>) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !worker.is_finished() {
            assert!(Instant::now() < deadline, "cancelled guest did not retire");
            std::thread::yield_now();
        }
    }

    fn live_pair(broadcast: bool) {
        let engine = GuestEngine::new(GuestLimits {
            fuel_per_call: u64::MAX,
            ..Default::default()
        })
        .unwrap();
        let component = engine.compile(SPIN.as_bytes()).unwrap();
        let mut first = GuestInstance::new(&component).unwrap();
        let mut second = GuestInstance::new(&component).unwrap();
        let a = first.cancellation();
        let b = second.cancellation();
        let retired = GuestInstance::new(&component).unwrap().cancellation();
        // Test-only host ticker outlives scoped workers, including panic cleanup.
        let _ticker = Ticker::new(engine.clone());
        std::thread::scope(|scope| {
            // Dropped before the scoped threads are joined, including on panic.
            let _cleanup = Cleanup(vec![a.clone(), b.clone()]);
            let first = scope.spawn(move || first.call::<(), (i64,)>("spin", ()));
            let second = scope.spawn(move || second.call::<(), (i64,)>("spin", ()));
            progressed(&a, 0);
            progressed(&b, 0);
            let before = b.state.epochs.load(Ordering::SeqCst);
            retired.cancel();
            progressed(&b, before);
            if broadcast {
                engine.interrupt();
            } else {
                a.cancel();
            }
            finished(&first);
            assert_eq!(
                first.join().unwrap(),
                Err(GuestError::Trap(GuestTrap::Interrupted))
            );
            if !broadcast {
                // The other guest must still execute epoch callbacks after its
                // peer retired. Merely creating a healthy instance is insufficient.
                let before = b.state.epochs.load(Ordering::SeqCst);
                progressed(&b, before);
                assert!(!b.is_cancelled());
                b.cancel();
            }
            finished(&second);
            assert_eq!(
                second.join().unwrap(),
                Err(GuestError::Trap(GuestTrap::Interrupted))
            );
        });
    }

    #[test]
    fn scoped_cancellation_leaves_an_already_running_peer_usable() {
        live_pair(false);
    }

    #[test]
    fn explicit_engine_broadcast_still_interrupts_both_running_instances() {
        live_pair(true);
    }

    #[test]
    fn cancellation_before_entry_is_terminal_and_old_handles_never_retarget() {
        let engine = GuestEngine::new(GuestLimits::default()).unwrap();
        let component = engine.compile(SPIN.as_bytes()).unwrap();
        let mut first = GuestInstance::new(&component).unwrap();
        let old = first.cancellation();
        old.cancel();
        old.cancel();
        assert!(old.is_cancelled());
        assert_eq!(
            first.call::<(), (i64,)>("value", ()),
            Err(GuestError::Trap(GuestTrap::Interrupted))
        );
        assert!(first.is_poisoned());
        assert_eq!(
            first.call::<(), (i64,)>("value", ()),
            Err(GuestError::Poisoned)
        );
        drop(first);
        let mut next = GuestInstance::new(&component).unwrap();
        old.cancel();
        assert_eq!(next.call::<(), (i64,)>("value", ()), Ok((42,)));
        // Idle instances do not inherit a previous broadcast into their next call.
        engine.interrupt();
        assert_eq!(next.call::<(), (i64,)>("value", ()), Ok((42,)));
    }

    #[test]
    fn arming_checks_a_cancellation_even_after_its_wakeup_has_passed() {
        let engine = GuestEngine::new(GuestLimits::default()).unwrap();
        let mut store = Store::new(&engine.engine, ());
        let control = GuestCancellation::install(&engine, &mut store).unwrap();
        control.cancel();
        assert_eq!(
            control.arm(&mut store),
            Err(GuestError::Trap(GuestTrap::Interrupted))
        );
    }
}
