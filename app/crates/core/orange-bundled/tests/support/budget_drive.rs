//! Drive a run in one-unit slices and watch its aggregate nominal reservation.
//!
//! The charges are nominal bytes, so tests locate the boundary by search: the smallest
//! aggregate limit that lets the fixture finish must finish it, and one byte less must
//! be refused with the typed aggregate error. That also covers peaks inside a slice.
use orange_runtime::{Run, RunError, RunState};

pub const HUGE: usize = 1 << 20;

/// Pump until idle, advancing virtual time to timers. Every slice must stay within
/// `limit`. A refusal faults the run, whose cleanup retires its activations, so it
/// may only lower the reservation.
pub fn drive(run: &mut Run, limit: usize) -> Result<(), RunError> {
    for _ in 0..100_000 {
        if run.state() == RunState::Stopped {
            return Ok(());
        }
        let progress = run.pump(1)?;
        assert!(run.retention().reserved_bytes <= limit);
        if !progress.ready && run.event_stats().pending == 0 {
            match progress.next_wakeup {
                Some(at) => run.advance_to(at).unwrap(),
                None => return Ok(()),
            }
        }
    }
    panic!("fixture did not become idle");
}

/// Stop and dispose, then require that every reserved byte was returned.
pub fn release(mut run: Run) {
    let _ = run.stop();
    if run.state() == RunState::Stopped {
        run.dispose().unwrap();
    }
    assert_eq!(
        run.retention().reserved_bytes,
        0,
        "capacity was not returned"
    );
}

/// Start under `limit` and drive to idle, then release. The run is torn down either way.
pub fn attempt(
    make: &impl Fn(usize) -> Result<Run, RunError>,
    limit: usize,
) -> Result<(), RunError> {
    let mut run = make(limit)?;
    let result = drive(&mut run, limit);
    release(run);
    result
}

/// `make(limit)` builds a run under that aggregate limit. Finds the smallest limit
/// that completes the fixture (which must be non-zero and below `HUGE`) and returns
/// the refusal one byte below it.
pub fn boundary(make: impl Fn(usize) -> Result<Run, RunError>) -> RunError {
    attempt(&make, HUGE).expect("a generous budget completes the run");
    let (mut refused, mut fits) = (0, HUGE);
    while fits - refused > 1 {
        let middle = refused + (fits - refused) / 2;
        match attempt(&make, middle) {
            Ok(()) => fits = middle,
            Err(_) => refused = middle,
        }
    }
    assert!(
        fits > 0 && attempt(&make, 0).is_err(),
        "the fixture reserved nothing"
    );
    attempt(&make, fits).expect("the smallest admitting limit completes the run");
    attempt(&make, fits - 1).expect_err("one byte below it is refused")
}
