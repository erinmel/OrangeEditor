//! Listening to part of a sound: a band filter, the level of that band, and the events
//! made from it.
//!
//! Analyzers read an input's shared capture ring with their own cursor, so any number
//! of them listen to one device opened once, each with its own band and thresholds.
//! They run on their own threads, never on the capture callback.

use crate::level::{Meter, SILENCE_DB, level_db};
use crate::stream::Ring;
use serde_json::{Map, Value as Json};
use std::f64::consts::PI;
use std::time::Duration;

/// One second-order filter section (RBJ cookbook), in transposed direct form II.
#[derive(Debug, Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    /// Butterworth (Q = 1/√2) low-pass or high-pass at `hz` for `rate`.
    fn new(rate: f64, hz: f64, high_pass: bool) -> Self {
        let w = 2.0 * PI * hz / rate;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        let (b0, b1, b2) = if high_pass {
            ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0)
        } else {
            ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0)
        };
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    /// Constant-peak band-pass section centred between the two edges. Widen each
    /// section so two cascaded sections reach -3 dB at the prewarped edges.
    fn band_pass(rate: f64, low: f64, high: f64) -> Self {
        let low = (PI * low / rate).tan();
        let high = (PI * high / rate).tan();
        let centre_squared = low * high;
        let bandwidth = (high - low) / (2.0_f64.sqrt() - 1.0).sqrt();
        let a0 = 1.0 + bandwidth + centre_squared;
        Self {
            b0: bandwidth / a0,
            b1: 0.0,
            b2: -bandwidth / a0,
            a1: 2.0 * (centre_squared - 1.0) / a0,
            a2: (1.0 - bandwidth + centre_squared) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }
}

/// The part of the spectrum an analyzer hears: everything above `low` and below `high`
/// hertz. Zero leaves that side open; a band at or above half the rate is open too.
#[derive(Debug, Clone)]
pub(crate) struct Band {
    high_pass: Option<Biquad>,
    low_pass: Option<Biquad>,
}

impl Band {
    pub(crate) fn isolated(rate: u32, low: f64, high: f64) -> Self {
        if high >= f64::from(rate) / 2.0 {
            return Self::new(rate, low, high);
        }
        let section = Biquad::band_pass(f64::from(rate), low, high);
        Self {
            high_pass: Some(section),
            low_pass: Some(section),
        }
    }
    pub(crate) fn new(rate: u32, low_hz: f64, high_hz: f64) -> Self {
        let rate = f64::from(rate.max(1));
        let nyquist = rate / 2.0;
        let open = |hz: f64| !(hz > 0.0 && hz < nyquist * 0.95);
        Self {
            high_pass: (!open(low_hz)).then(|| Biquad::new(rate, low_hz, true)),
            low_pass: (!open(high_hz)).then(|| Biquad::new(rate, high_hz, false)),
        }
    }

    pub(crate) fn process(&mut self, sample: f64) -> f64 {
        let mut y = sample;
        if let Some(stage) = &mut self.high_pass {
            y = stage.process(y);
        }
        if let Some(stage) = &mut self.low_pass {
            y = stage.process(y);
        }
        y
    }
}

/// Settings every analyzer reads: its band and its measurement window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Listening {
    pub low_hz: f64,
    pub high_hz: f64,
    pub window: Duration,
}

/// A number setting, or `default` when it is absent.
pub(crate) fn number(
    settings: &Map<String, Json>,
    name: &str,
    default: f64,
) -> Result<f64, String> {
    match settings.get(name) {
        None | Some(Json::Null) => Ok(default),
        Some(value) => value
            .as_f64()
            .filter(|number| number.is_finite())
            .ok_or_else(|| format!("`{name}` must be a number")),
    }
}

impl Listening {
    /// The band (`low_hz`, `high_hz`, 0 for open) and window (`window_ms`, 5 to 1000)
    /// of an analyzer's settings, with `defaults` for those it leaves out.
    pub(crate) fn of(
        settings: &Map<String, Json>,
        defaults: (f64, f64, f64),
    ) -> Result<Self, String> {
        let low_hz = number(settings, "low_hz", defaults.0)?;
        let high_hz = number(settings, "high_hz", defaults.1)?;
        let window_ms = number(settings, "window_ms", defaults.2)?;
        if low_hz < 0.0 || high_hz < 0.0 || (high_hz > 0.0 && low_hz >= high_hz) {
            return Err("the band needs 0 <= low_hz < high_hz, or 0 to leave a side open".into());
        }
        if !(5.0..=1_000.0).contains(&window_ms) {
            return Err("`window_ms` must be from 5 to 1000".into());
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "checked to be 5 to 1000"
        )]
        let window = Duration::from_millis(window_ms as u64);
        Ok(Self {
            low_hz,
            high_hz,
            window,
        })
    }

    /// Frames in one window at `rate`.
    pub(crate) fn frames(&self, rate: u32) -> usize {
        let frames = u128::from(rate) * self.window.as_millis() / 1_000;
        usize::try_from(frames).unwrap_or(usize::MAX).max(1)
    }
}

/// Turns interleaved samples into band levels. Filtering and power measurement
/// precede the channel average so opposite-phase signals remain audible to the meter.
#[derive(Debug, Clone)]
pub(crate) struct BandMeter {
    bands: Vec<Band>,
    meter: Meter,
}

impl BandMeter {
    pub(crate) fn new(ring: &Ring, listening: &Listening) -> Self {
        Self {
            bands: channel_bands(ring, listening),
            meter: Meter::new(listening.frames(ring.rate)),
        }
    }

    /// Measures `samples`, calling `window` with the level of each completed window.
    pub(crate) fn push(&mut self, samples: &[f32], mut window: impl FnMut(f64)) {
        for frame in samples.chunks_exact(self.bands.len()) {
            self.meter
                .push_power(frame_power(frame, &mut self.bands), &mut window);
        }
    }
}

fn channel_bands(ring: &Ring, listening: &Listening) -> Vec<Band> {
    vec![
        Band::new(ring.rate, listening.low_hz, listening.high_hz);
        usize::from(ring.channels.max(1))
    ]
}

#[expect(clippy::cast_precision_loss, reason = "a frame has few channels")]
fn frame_power(frame: &[f32], bands: &mut [Band]) -> f64 {
    frame
        .iter()
        .zip(bands)
        .map(|(&sample, band)| band.process(f64::from(sample)).powi(2))
        .sum::<f64>()
        / frame.len() as f64
}

/// Fires on a sudden rise of a band's level: a hit, such as a kick drum. A window is a
/// hit when its level is at least `floor` and stands `rise` decibels above the recent
/// average, the last hit was more than `gap` windows ago, and the level has fallen
/// back toward the average since then, so a held note is one hit.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Onsets {
    floor: f64,
    rise: f64,
    gap: u32,
    since: u32,
    armed: bool,
    /// The recent average energy, smoothed over about [`AVERAGE_WINDOWS`].
    average: f64,
}

/// Windows the onset average reaches back over.
const AVERAGE_WINDOWS: f64 = 30.0;

impl Onsets {
    /// `sensitivity` from 0 (only strong hits) to 1 (faint ones): the rise needed falls
    /// from 12 to 3 dB.
    pub(crate) fn new(floor: f64, sensitivity: f64, gap: u32) -> Self {
        Self {
            floor,
            rise: 12.0 - 9.0 * sensitivity.clamp(0.0, 1.0),
            gap,
            since: gap,
            armed: true,
            average: SILENCE_DB,
        }
    }

    pub(crate) fn step(&mut self, level: f64) -> bool {
        self.since = self.since.saturating_add(1);
        let above = level - self.average;
        if above < self.rise / 2.0 {
            self.armed = true;
        }
        let hit = self.armed && level >= self.floor && above >= self.rise && self.since > self.gap;
        // The average follows in energy, so a loud window counts as loud.
        let energy = |db: f64| 10f64.powf(db / 10.0);
        let mixed = energy(self.average) + (energy(level) - energy(self.average)) / AVERAGE_WINDOWS;
        self.average = level_db(mixed, 1);
        if hit {
            self.since = 0;
            self.armed = false;
        }
        hit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f64, rate: u32, seconds: f64) -> Vec<f32> {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "test sizes"
        )]
        let count = (f64::from(rate) * seconds) as usize;
        (0..count)
            .map(|n| {
                #[expect(
                    clippy::cast_precision_loss,
                    clippy::cast_possible_truncation,
                    reason = "test tone"
                )]
                let sample = (2.0 * PI * hz * n as f64 / f64::from(rate)).sin() as f32 * 0.5;
                sample
            })
            .collect()
    }

    fn band_level(samples: &[f32], low: f64, high: f64) -> f64 {
        let mut band = Band::new(48_000, low, high);
        let tail = samples.len() / 2;
        let mut sum = 0.0;
        for (index, sample) in samples.iter().enumerate() {
            let y = band.process(f64::from(*sample));
            if index >= tail {
                sum += y * y;
            }
        }
        level_db(sum, samples.len() - tail)
    }

    #[test]
    fn a_band_keeps_what_is_inside_and_cuts_what_is_outside() {
        let bass = tone(80.0, 48_000, 0.5);
        let hiss = tone(5_000.0, 48_000, 0.5);
        let open = band_level(&bass, 0.0, 0.0);
        // A half-scale sine is about -9 dBFS.
        assert!((open + 9.03).abs() < 0.2, "{open}");
        assert!((band_level(&bass, 45.0, 120.0) - open).abs() < 3.5);
        assert!(band_level(&hiss, 45.0, 120.0) < open - 40.0);
        assert!(band_level(&bass, 2_000.0, 0.0) < open - 40.0);
    }

    #[test]
    fn listening_settings_have_defaults_and_limits() {
        let none = Map::new();
        let kick = Listening::of(&none, (45.0, 120.0, 10.0)).unwrap();
        assert_eq!((kick.low_hz, kick.high_hz), (45.0, 120.0));
        assert_eq!(kick.frames(48_000), 480);
        for bad in [
            serde_json::json!({"low_hz": 200, "high_hz": 100}),
            serde_json::json!({"low_hz": -1}),
            serde_json::json!({"window_ms": 1}),
            serde_json::json!({"high_hz": "loud"}),
        ] {
            let Json::Object(settings) = bad else {
                unreachable!()
            };
            assert!(Listening::of(&settings, (0.0, 0.0, 50.0)).is_err());
        }
    }

    #[test]
    fn onsets_fire_on_sudden_rises_once_per_gap() {
        let mut onsets = Onsets::new(-50.0, 0.5, 5);
        let quiet = -60.0;
        let mut hits = Vec::new();
        for window in 0..60 {
            // A hit every 20 windows over a quiet bed, each lasting three windows.
            let level = if window % 20 < 3 && window >= 10 {
                -10.0
            } else {
                quiet
            };
            if onsets.step(level) {
                hits.push(window);
            }
        }
        assert_eq!(hits, [20, 40]);
        // A sustained loud sound gives one hit, not one per window.
        let mut held = Onsets::new(-50.0, 0.5, 2);
        let fired = (0..40).filter(|_| held.step(-10.0)).count();
        assert_eq!(fired, 1);
    }
}
