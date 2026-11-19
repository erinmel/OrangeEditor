//! Bounded, logarithmic spectrum snapshots on a worker, outside capture callbacks.
use crate::SILENCE_DB;
use std::f64::consts::TAU;

pub(crate) const FRAMES: usize = 4096;
const BUCKETS: usize = 96;
const MIN_HZ: f64 = 20.0;
const MAX_HZ: f64 = 24_000.0;

pub(crate) struct Analyzer {
    window: Vec<f64>,
    twiddles: Vec<(f64, f64)>,
    bins: Vec<(f64, f64)>,
}

#[expect(
    clippy::cast_precision_loss,
    reason = "FFT and channel indices are small"
)]
impl Analyzer {
    pub(crate) fn new() -> Self {
        Self {
            window: (0..FRAMES)
                .map(|i| (1.0 - (TAU * i as f64 / FRAMES as f64).cos()) / 2.0)
                .collect(),
            twiddles: (0..FRAMES / 2)
                .map(|i| {
                    let (sin, cos) = (-TAU * i as f64 / FRAMES as f64).sin_cos();
                    (cos, sin)
                })
                .collect(),
            bins: vec![(0.0, 0.0); FRAMES],
        }
    }

    pub(crate) fn measure(
        &mut self,
        samples: &[f32],
        channels: usize,
        rate: u32,
    ) -> [f64; BUCKETS] {
        let mut power = [None::<f64>; BUCKETS];
        for channel in 0..channels {
            for (index, frame) in samples.chunks_exact(channels).enumerate() {
                let reversed = index.reverse_bits() >> (usize::BITS - FRAMES.ilog2());
                self.bins[reversed] = (f64::from(frame[channel]) * self.window[index], 0.0);
            }
            let mut size = 2;
            while size <= FRAMES {
                for chunk in self.bins.chunks_exact_mut(size) {
                    let (left, right) = chunk.split_at_mut(size / 2);
                    for (index, (a, b)) in left.iter_mut().zip(right).enumerate() {
                        let (c, s) = self.twiddles[index * (FRAMES / size)];
                        let (real, imag) = (b.0 * c - b.1 * s, b.0 * s + b.1 * c);
                        *b = (a.0 - real, a.1 - imag);
                        *a = (a.0 + real, a.1 + imag);
                    }
                }
                size *= 2;
            }
            for (i, &(real, imag)) in self.bins.iter().enumerate().take(FRAMES / 2).skip(1) {
                let hz = i as f64 * f64::from(rate) / FRAMES as f64;
                if (MIN_HZ..MAX_HZ).contains(&hz) {
                    let slot = &mut power[bucket(hz)];
                    let measured = real.mul_add(real, imag * imag);
                    *slot = Some(slot.map_or(measured, |previous| previous.max(measured)));
                }
            }
        }
        // Some narrow logarithmic buckets do not contain a Fourier bin. Fill
        // those between measurements so the chart doesn't draw false nulls.
        let mut previous = None;
        for next in 0..power.len() {
            let Some(right) = power[next] else { continue };
            if let Some((left, value)) = previous {
                let span = (next - left) as f64;
                for (offset, slot) in power[left + 1..next].iter_mut().enumerate() {
                    let fraction = (offset + 1) as f64 / span;
                    *slot = Some(value * (1.0 - fraction) + right * fraction);
                }
            }
            previous = Some((next, right));
        }
        power.map(|value| {
            (10.0 * (value.unwrap_or_default() * (4.0 / FRAMES as f64).powi(2)).log10())
                .max(SILENCE_DB)
        })
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

    #[test]
    fn finds_live_tones_even_when_stereo_channels_have_opposite_phase() {
        let samples: Vec<_> = (0u16..4096)
            .flat_map(|i| {
                let sample = (std::f32::consts::TAU * 1000.0 * f32::from(i) / 48_000.0).sin();
                [sample, -sample]
            })
            .collect();
        let values = Analyzer::new().measure(&samples, 2, 48_000);
        assert!(values[bucket(1000.0)] > -2.0);
        assert!(values[bucket(100.0)] < -50.0);
    }
}
