//! Input level measurement.
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
            self.push_power(sample * sample, &mut window);
        }
    }

    pub(crate) fn push_power(&mut self, power: f64, window: &mut impl FnMut(f64)) {
        self.sum += power;
        self.count += 1;
        if self.count == self.size {
            window(level_db(self.sum, self.count));
            self.sum = 0.0;
            self.count = 0;
        }
    }
}

/// A full-band window on the analyzer worker. Keeps the aggregate trigger level
/// and channel sample peaks distinct, even when one channel briefly clips.
pub(crate) struct ChannelMeter {
    frames: usize,
    count: usize,
    channel_count: f64,
    channels: Vec<ChannelPower>,
}

#[derive(Default)]
struct ChannelPower {
    energy: f64,
    peak: f64,
    clipped: bool,
}

impl ChannelPower {
    fn push(&mut self, sample: f32) {
        let sample = f64::from(sample);
        let squared = sample * sample;
        self.energy += squared;
        self.peak = self.peak.max(squared);
    }
}

pub(crate) struct ChannelWindow {
    pub(crate) level: f64,
    pub(crate) rms: Box<[f64]>,
    pub(crate) peak: Box<[f64]>,
    pub(crate) clipped: Box<[bool]>,
}

impl ChannelMeter {
    pub(crate) fn new(frames: usize, channels: u16) -> Self {
        let channels = channels.max(1);
        Self {
            frames: frames.max(1),
            count: 0,
            channel_count: f64::from(channels),
            channels: (0..channels).map(|_| ChannelPower::default()).collect(),
        }
    }

    pub(crate) fn push(&mut self, samples: &[f32], mut report: impl FnMut(ChannelWindow)) {
        let width = self.channels.len();
        let mut samples = &samples[..samples.len() / width * width];
        while !samples.is_empty() {
            let frames = (self.frames - self.count).min(samples.len() / width);
            let (block, rest) = samples.split_at(frames * width);
            samples = rest;
            match self.channels.as_mut_slice() {
                [mono] => {
                    for &sample in block {
                        mono.push(sample);
                    }
                }
                [left, right] => {
                    for &[a, b] in block.as_chunks::<2>().0 {
                        left.push(a);
                        right.push(b);
                    }
                }
                channels => {
                    for frame in block.chunks_exact(width) {
                        for (&sample, channel) in frame.iter().zip(channels.iter_mut()) {
                            channel.push(sample);
                        }
                    }
                }
            }
            self.count += frames;
            if self.count == self.frames {
                report(self.finish());
                self.count = 0;
            }
        }
    }

    fn finish(&mut self) -> ChannelWindow {
        let mut total = 0.0;
        let (rms, peak): (Vec<_>, Vec<_>) = self
            .channels
            .iter_mut()
            .map(|channel| {
                let energy = std::mem::take(&mut channel.energy);
                let peak = std::mem::take(&mut channel.peak);
                total += energy;
                channel.clipped |= peak >= 1.0;
                (level_db(energy, self.count), level_db(peak, 1))
            })
            .unzip();
        let level = if let [mono] = rms.as_slice() {
            *mono
        } else {
            level_db(total / self.channel_count, self.count)
        };
        ChannelWindow {
            level,
            rms: rms.into(),
            peak: peak.into(),
            clipped: self
                .channels
                .iter()
                .map(|channel| channel.clipped)
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_peaks_do_not_change_the_aggregate_rms_trigger_reading() {
        let mut meter = ChannelMeter::new(4, 2);
        let mut window = None;
        meter.push(&[1.25, -0.25, 0.0, -0.25], |_| panic!("incomplete window"));
        meter.push(&[0.0, -0.25, 0.0, -0.25], |measured| {
            window = Some(measured)
        });
        let window = window.unwrap();
        assert!(window.level < -6.0);
        assert!(window.peak[0] > 0.0 && window.rms[0] < 0.0);
        assert_eq!(window.rms[1], window.peak[1]);
        assert_eq!(window.clipped.as_ref(), [true, false]);
        assert!((window.level - level_db(1.8125, 8)).abs() < 1e-12);
        meter.push(&[0.0; 8], |window| {
            assert_eq!(window.rms.as_ref(), [SILENCE_DB; 2]);
            assert_eq!(window.peak.as_ref(), [SILENCE_DB; 2]);
            assert_eq!(window.clipped.as_ref(), [true, false]);
        });
    }

    #[test]
    fn multichannel_windows_cross_push_boundaries_and_measure_each_complete_frame() {
        let mut meter = ChannelMeter::new(2, 3);
        let mut windows = Vec::new();
        meter.push(&[0.0, 0.0, 0.5], |_| panic!("incomplete window"));
        meter.push(&[-1.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], |window| {
            windows.push(window);
        });
        assert_eq!(windows.len(), 2);
        let first = &windows[0];
        assert!((first.level - level_db(1.5, 6)).abs() < 1e-12);
        assert_eq!(first.clipped.as_ref(), [true, false, false]);
        assert_eq!(
            first.peak.as_ref(),
            [0.0, level_db(0.25, 1), level_db(0.25, 1)]
        );
        assert_eq!(windows[1].level, SILENCE_DB);
        assert_eq!(windows[1].clipped.as_ref(), [true, false, false]);
    }

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
