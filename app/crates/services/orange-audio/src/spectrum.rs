//! Bounded, logarithmic spectrum snapshots on a worker, outside capture callbacks.
//!
//! A snapshot reads the latest samples, so consecutive snapshots overlap. Buckets below
//! about 800 Hz come from a long transform, which resolves a few hertz; the others come
//! from a short one, which follows changes faster. Both use a Blackman-Harris window,
//! whose sidelobes stay below -92 dB, so a tone does not leak into distant buckets. A
//! tone's level is corrected for where it falls between Fourier bins.
use crate::SILENCE_DB;
use std::f64::consts::TAU;

/// Frames of the short transform.
const SHORT: usize = 4096;
/// Frames of the long transform, which is also the history a snapshot needs. Twice the
/// short one: about 6 Hz bins at 48 kHz, while a change still shows within about 85 ms.
pub(crate) const FRAMES: usize = 8192;
/// Buckets per snapshot: 1/24 octave from 20 Hz to 24 kHz.
pub(crate) const BUCKETS: usize = 246;
const MIN_HZ: f64 = 20.0;
const MAX_HZ: f64 = 24_000.0;

/// A Blackman-Harris windowed transform of one size, with every buffer allocated once.
struct Transform {
    window: Vec<f64>,
    /// Sum of the window, which scales a bin to the peak amplitude of a sine.
    gain: f64,
    twiddles: Vec<(f64, f64)>,
    reversed: Vec<u32>,
    data: Vec<(f64, f64)>,
    /// Squared single-sided peak amplitude of each bin, the loudest channel's.
    power: Vec<f64>,
}

#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "transform sizes and indices are small"
)]
impl Transform {
    fn new(size: usize) -> Self {
        let window: Vec<f64> = (0..size)
            .map(|i| {
                let x = TAU * i as f64 / size as f64;
                0.358_75 - 0.488_29 * x.cos() + 0.141_28 * (2.0 * x).cos()
                    - 0.011_68 * (3.0 * x).cos()
            })
            .collect();
        let bits = size.ilog2();
        Self {
            gain: window.iter().sum(),
            window,
            twiddles: (0..size / 2)
                .map(|i| {
                    let (sin, cos) = (-TAU * i as f64 / size as f64).sin_cos();
                    (cos, sin)
                })
                .collect(),
            reversed: (0..size)
                .map(|i| (i.reverse_bits() >> (usize::BITS - bits)) as u32)
                .collect(),
            data: vec![(0.0, 0.0); size],
            power: vec![0.0; size / 2],
        }
    }

    fn size(&self) -> usize {
        self.data.len()
    }

    /// Transforms the last frames of interleaved `samples`. Channels go through the
    /// transform two at a time, one as the real part and one as the imaginary part.
    fn run(&mut self, samples: &[f32], channels: usize) {
        let size = self.size();
        let frames = &samples[samples.len() - size * channels..];
        self.power.fill(0.0);
        for first in (0..channels).step_by(2) {
            let second = (first + 1 < channels).then_some(first + 1);
            for (index, frame) in frames.chunks_exact(channels).enumerate() {
                let w = self.window[index];
                let imag = second.map_or(0.0, |channel| f64::from(frame[channel]) * w);
                self.data[self.reversed[index] as usize] = (f64::from(frame[first]) * w, imag);
            }
            let mut span = 2;
            while span <= size {
                let stride = size / span;
                for chunk in self.data.chunks_exact_mut(span) {
                    let (left, right) = chunk.split_at_mut(span / 2);
                    for (index, (a, b)) in left.iter_mut().zip(right).enumerate() {
                        let (c, s) = self.twiddles[index * stride];
                        let (real, imag) = (b.0 * c - b.1 * s, b.0 * s + b.1 * c);
                        *b = (a.0 - real, a.1 - imag);
                        *a = (a.0 + real, a.1 + imag);
                    }
                }
                span *= 2;
            }
            // Z = X + iY for real X and Y: X[k] = (Z[k] + conj Z[N-k]) / 2 and
            // Y[k] = (Z[k] - conj Z[N-k]) / 2i. A sine of amplitude A gives |X| = A·gain/2.
            let scale = 4.0 / (self.gain * self.gain) / 4.0;
            for k in 1..size / 2 {
                let (zr, zi) = self.data[k];
                let (wr, wi) = self.data[size - k];
                let x = (zr + wr).mul_add(zr + wr, (zi - wi) * (zi - wi));
                let mut loudest = x;
                if second.is_some() {
                    let y = (zi + wi).mul_add(zi + wi, (zr - wr) * (zr - wr));
                    loudest = loudest.max(y);
                }
                self.power[k] = self.power[k].max(loudest * scale);
            }
        }
    }
}

/// Measures snapshots. Each one stands alone, so the line rises and falls as fast as
/// the sound does.
pub(crate) struct Analyzer {
    short: Transform,
    long: Transform,
}

#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "bounded bins and buckets"
)]
impl Analyzer {
    pub(crate) fn new() -> Self {
        Self {
            short: Transform::new(SHORT),
            long: Transform::new(FRAMES),
        }
    }

    /// One snapshot of the latest interleaved `samples` in dBFS peak amplitude, or
    /// `None` until a short transform's worth of frames is there.
    pub(crate) fn measure(
        &mut self,
        samples: &[f32],
        channels: usize,
        rate: u32,
    ) -> Option<Vec<f64>> {
        let channels = channels.max(1);
        let frames = samples.len() / channels;
        if frames < SHORT {
            return None;
        }
        let samples = &samples[..frames * channels];
        let rate = f64::from(rate);
        self.short.run(samples, channels);
        let long = frames >= FRAMES;
        if long {
            self.long.run(samples, channels);
        }
        let ratio = (MAX_HZ / MIN_HZ).powf(1.0 / BUCKETS as f64);
        let mut levels = vec![0.0f64; BUCKETS];
        for transform in [&self.long, &self.short] {
            if transform.size() == FRAMES && !long {
                continue;
            }
            let spacing = rate / transform.size() as f64;
            let power = &transform.power;
            let uses = |bucket: usize| {
                let low = MIN_HZ * ratio.powi(bucket as i32);
                // The long transform measures buckets narrower than two short bins.
                let narrow = low * (ratio - 1.0) < 2.0 * rate / SHORT as f64;
                (narrow && long) == (transform.size() == FRAMES)
            };
            for (bucket, level) in levels.iter_mut().enumerate().filter(|(b, _)| uses(*b)) {
                let low = MIN_HZ * ratio.powi(bucket as i32);
                let high = low * ratio;
                let first = ((low / spacing).ceil() as usize).max(1);
                let last = ((high / spacing).ceil() as usize).min(power.len());
                *level = if first < last {
                    power[first..last].iter().copied().fold(0.0, f64::max)
                } else {
                    // No bin falls in this narrow bucket: read the smooth line between
                    // the bins on either side of its centre.
                    let at = (low * high).sqrt() / spacing;
                    let below = (at.floor() as usize).clamp(1, power.len() - 1);
                    let above = (below + 1).min(power.len() - 1);
                    let fraction = (at - below as f64).clamp(0.0, 1.0);
                    let db = |p: f64| p.max(1e-30).ln();
                    (db(power[below]) * (1.0 - fraction) + db(power[above]) * fraction).exp()
                };
            }
            // A tone between bins reads low on both: estimate its true peak from the
            // log-parabola through the loudest bin and its neighbours, and credit it to
            // the bucket of the estimated frequency.
            for k in 2..power.len() - 1 {
                let (a, b, c) = (power[k - 1], power[k], power[k + 1]);
                if b <= a || b < c || b <= 0.0 {
                    continue;
                }
                let (a, b, c) = (a.max(1e-30).ln(), b.ln(), c.max(1e-30).ln());
                let curve = a - 2.0 * b + c;
                if curve >= 0.0 {
                    continue;
                }
                let offset = (0.5 * (a - c) / curve).clamp(-0.5, 0.5);
                let hz = (k as f64 + offset) * spacing;
                if !(MIN_HZ..MAX_HZ).contains(&hz) {
                    continue;
                }
                let bucket = bucket(hz);
                if uses(bucket) {
                    let peak = (b - 0.25 * (a - c) * offset).exp();
                    levels[bucket] = levels[bucket].max(peak);
                }
            }
        }
        Some(
            levels
                .iter()
                .map(|power| (10.0 * power.max(1e-30).log10()).max(SILENCE_DB))
                .collect(),
        )
    }
}

/// The latest interleaved samples, up to a fixed capacity.
pub(crate) struct History {
    data: Vec<f32>,
    next: usize,
    filled: usize,
    ordered: Vec<f32>,
}

impl History {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            data: vec![0.0; capacity],
            next: 0,
            filled: 0,
            ordered: Vec::with_capacity(capacity),
        }
    }

    pub(crate) fn clear(&mut self) {
        self.next = 0;
        self.filled = 0;
    }

    pub(crate) fn push(&mut self, mut samples: &[f32]) {
        let capacity = self.data.len();
        if samples.len() > capacity {
            samples = &samples[samples.len() - capacity..];
        }
        while !samples.is_empty() {
            let count = (capacity - self.next).min(samples.len());
            self.data[self.next..self.next + count].copy_from_slice(&samples[..count]);
            self.next = (self.next + count) % capacity;
            self.filled = (self.filled + count).min(capacity);
            samples = &samples[count..];
        }
    }

    /// The samples held, oldest first.
    pub(crate) fn latest(&mut self) -> &[f32] {
        self.ordered.clear();
        if self.filled < self.data.len() {
            self.ordered.extend_from_slice(&self.data[..self.filled]);
        } else {
            self.ordered.extend_from_slice(&self.data[self.next..]);
            self.ordered.extend_from_slice(&self.data[..self.next]);
        }
        &self.ordered
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "a bounded frequency bucket index"
)]
fn bucket(hz: f64) -> usize {
    let position = (hz / MIN_HZ).ln() / (MAX_HZ / MIN_HZ).ln();
    ((position * BUCKETS as f64) as usize).min(BUCKETS - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn tone(hz: f64, amplitude: f64, frames: usize, channels: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                #[expect(clippy::cast_precision_loss, reason = "test frame index")]
                let sample = (amplitude * (TAU * hz * i as f64 / f64::from(RATE)).sin()) as f32;
                std::iter::repeat_n(sample, channels)
            })
            .collect()
    }

    fn measure(samples: &[f32], channels: usize) -> Vec<f64> {
        Analyzer::new().measure(samples, channels, RATE).unwrap()
    }

    #[test]
    fn finds_live_tones_even_when_stereo_channels_have_opposite_phase() {
        let samples: Vec<_> = tone(1000.0, 1.0, FRAMES, 1)
            .into_iter()
            .flat_map(|sample| [sample, -sample])
            .collect();
        let values = measure(&samples, 2);
        assert!(values[bucket(1000.0)] > -0.5, "{}", values[bucket(1000.0)]);
        assert!(values[bucket(100.0)] < -90.0);
    }

    #[test]
    fn a_tone_reads_its_level_across_the_range_and_wherever_it_falls_between_bins() {
        // -6.02 dBFS: a sine of amplitude 0.5.
        for hz in [
            25.0, 31.7, 50.0, 63.3, 100.0, 440.0, 997.0, 1000.0, 1001.3, 5000.0, 12_345.0, 19_000.0,
        ] {
            let values = measure(&tone(hz, 0.5, FRAMES, 1), 1);
            let level = values[bucket(hz)];
            assert!((level + 6.02).abs() < 0.3, "{hz} Hz read {level:.2} dBFS");
            let loudest = values.iter().copied().fold(f64::MIN, f64::max);
            assert!((loudest - level).abs() < 1e-9, "{hz} Hz peaked elsewhere");
        }
    }

    #[test]
    fn a_tone_does_not_leak_into_buckets_an_octave_away() {
        for hz in [40.0, 200.0, 1000.0, 8000.0] {
            let values = measure(&tone(hz, 1.0, FRAMES, 2), 2);
            for (index, level) in values.iter().enumerate() {
                let centre = MIN_HZ * (MAX_HZ / MIN_HZ).powf((index as f64 + 0.5) / BUCKETS as f64);
                if (centre / hz).log2().abs() > 1.0 {
                    assert!(
                        *level < -90.0,
                        "{hz} Hz showed {level:.1} dB at {centre:.0} Hz"
                    );
                }
            }
        }
    }

    #[test]
    fn a_short_history_measures_with_the_short_transform_only() {
        assert!(
            Analyzer::new()
                .measure(&tone(1000.0, 0.5, SHORT - 1, 1), 1, RATE)
                .is_none()
        );
        let values = measure(&tone(1000.0, 0.5, SHORT, 1), 1);
        assert!((values[bucket(1000.0)] + 6.02).abs() < 0.3);
    }

    #[test]
    fn a_snapshot_holds_nothing_from_the_one_before() {
        let mut analyzer = Analyzer::new();
        let loud = analyzer
            .measure(&tone(1000.0, 1.0, FRAMES, 1), 1, RATE)
            .unwrap();
        assert!(loud[bucket(1000.0)] > -0.3);
        let silent = analyzer.measure(&vec![0.0; FRAMES], 1, RATE).unwrap();
        assert!(silent.iter().all(|level| *level == SILENCE_DB));
    }

    #[test]
    fn a_new_tone_reads_its_level_once_the_short_window_holds_it() {
        // Silence, then a tone for the last short window: the upper buckets measure
        // only the short window, so the tone reads at full level straight away.
        let mut samples = vec![0.0; FRAMES - SHORT];
        samples.extend(tone(5000.0, 0.5, SHORT, 1));
        let values = measure(&samples, 1);
        assert!((values[bucket(5000.0)] + 6.02).abs() < 0.3);
    }

    #[test]
    fn history_keeps_the_latest_samples_in_order() {
        let mut history = History::new(4);
        history.push(&[1.0, 2.0, 3.0]);
        assert_eq!(history.latest(), [1.0, 2.0, 3.0]);
        history.push(&[4.0, 5.0]);
        assert_eq!(history.latest(), [2.0, 3.0, 4.0, 5.0]);
        history.push(&[6.0, 7.0, 8.0, 9.0, 10.0]);
        assert_eq!(history.latest(), [7.0, 8.0, 9.0, 10.0]);
    }
}
