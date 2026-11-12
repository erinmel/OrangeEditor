//! Input level measurement and the threshold that turns it into one event.
//!
//! Everything here is arithmetic on values the capture callback already holds, so it
//! runs on that callback without allocating or waiting.

/// The level reported for silence, in dBFS.
pub const SILENCE_DB: f64 = -120.0;

/// RMS level of a block in dBFS: 0.0 is a full-scale square wave, silence is
/// [`SILENCE_DB`].
pub(crate) fn level_db(sum_of_squares: f64, samples: usize) -> f64 {
    if samples == 0 {
        return SILENCE_DB;
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "window sizes are far below 2^52"
    )]
    let mean = sum_of_squares / samples as f64;
    if mean <= 0.0 || !mean.is_finite() {
        return SILENCE_DB;
    }
    (10.0 * mean.log10()).max(SILENCE_DB)
}

/// Fires once when the level reaches `threshold`, and again only after it has fallen
/// to `release` or below.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Gate {
    pub threshold: f64,
    pub release: f64,
    armed: bool,
}
impl Gate {
    pub(crate) const fn new(threshold: f64, release: f64) -> Self {
        Self {
            threshold,
            release,
            armed: true,
        }
    }
    pub(crate) fn step(&mut self, level: f64) -> bool {
        if self.armed && level >= self.threshold {
            self.armed = false;
            return true;
        }
        if level <= self.release {
            self.armed = true;
        }
        false
    }
}

/// Groups captured samples into fixed windows and measures each.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Meter {
    size: usize,
    sum: f64,
    count: usize,
}
impl Meter {
    pub(crate) fn new(size: usize) -> Self {
        Self {
            size: size.max(1),
            sum: 0.0,
            count: 0,
        }
    }
    /// Adds samples, calling `window` with the level of each window they complete.
    pub(crate) fn push(&mut self, samples: impl Iterator<Item = f32>, mut window: impl FnMut(f64)) {
        for sample in samples {
            let sample = f64::from(sample);
            self.sum += sample * sample;
            self.count += 1;
            if self.count == self.size {
                window(level_db(self.sum, self.count));
                self.sum = 0.0;
                self.count = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_are_rms_decibels_with_a_floor() {
        assert_eq!(level_db(0.0, 100), SILENCE_DB);
        assert_eq!(level_db(1.0, 0), SILENCE_DB);
        assert!(level_db(100.0, 100).abs() < 1e-9);
        // Half amplitude is about -6.02 dB.
        assert!((level_db(0.25 * 100.0, 100) + 6.0206).abs() < 1e-3);
        assert_eq!(level_db(1e-40, 100), SILENCE_DB);
    }

    #[test]
    fn the_gate_fires_once_until_the_level_falls_back() {
        let mut gate = Gate::new(-20.0, -30.0);
        let fired: Vec<bool> = [-50.0, -19.0, -10.0, -25.0, -15.0, -31.0, -20.0]
            .into_iter()
            .map(|level| gate.step(level))
            .collect();
        // -25 is between the two limits, so -15 does not fire again; -31 rearms.
        assert_eq!(fired, [false, true, false, false, false, false, true]);
    }

    #[test]
    fn the_meter_reports_each_full_window() {
        let mut meter = Meter::new(4);
        let mut levels = Vec::new();
        meter.push([1.0, 1.0, 1.0].into_iter(), |level| levels.push(level));
        assert!(levels.is_empty());
        meter.push([1.0, 0.0, 0.0, 0.0, 0.0, 0.5].into_iter(), |level| {
            levels.push(level);
        });
        assert_eq!(levels.len(), 2);
        assert!(levels[0].abs() < 1e-9);
        assert_eq!(levels[1], SILENCE_DB);
    }
}
