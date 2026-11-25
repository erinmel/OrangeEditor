use std::f64::consts::TAU;

pub const FFT_SIZE: usize = 2048;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilterKind {
    LowShelf,
    Bell,
    HighPass,
    HighShelf,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EqBand {
    pub frequency: f32,
    pub gain_db: f32,
    pub q: f32,
    pub kind: FilterKind,
}

#[derive(Clone, Copy, Debug)]
pub struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
}

impl Biquad {
    /// RBJ Audio EQ Cookbook. Shelf Q varies its transition slope.
    pub fn new(sample_rate: f32, band: EqBand) -> Self {
        assert!(sample_rate.is_finite() && sample_rate > 0.0);
        let rate = f64::from(sample_rate);
        let frequency = f64::from(band.frequency).clamp(rate * 0.000_001, rate * 0.499_999);
        let amplitude = 10.0_f64.powf(f64::from(band.gain_db.clamp(-36.0, 36.0)) / 40.0);
        let (sin, cos) = (TAU * frequency / rate).sin_cos();
        let alpha = sin / (2.0 * f64::from(band.q.clamp(0.1, 18.0)));
        let (b, a) = match band.kind {
            FilterKind::Bell => (
                [1.0 + alpha * amplitude, -2.0 * cos, 1.0 - alpha * amplitude],
                [1.0 + alpha / amplitude, -2.0 * cos, 1.0 - alpha / amplitude],
            ),
            FilterKind::HighPass => (
                [(1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0],
                [1.0 + alpha, -2.0 * cos, 1.0 - alpha],
            ),
            FilterKind::LowShelf | FilterKind::HighShelf => {
                let high = band.kind == FilterKind::HighShelf;
                let direction = if high { 1.0 } else { -1.0 };
                let sum = amplitude + 1.0;
                let diff = amplitude - 1.0;
                let knee = 2.0 * amplitude.sqrt() * alpha;
                let bc = sum + direction * diff * cos;
                let ac = sum - direction * diff * cos;
                (
                    [
                        amplitude * (bc + knee),
                        -direction * 2.0 * amplitude * (diff + direction * sum * cos),
                        amplitude * (bc - knee),
                    ],
                    [
                        ac + knee,
                        direction * 2.0 * (diff - direction * sum * cos),
                        ac - knee,
                    ],
                )
            }
        };
        Self {
            b: b.map(|value| value / a[0]),
            a: [a[1] / a[0], a[2] / a[0]],
        }
    }

    pub fn magnitude_db(self, frequency: f32, sample_rate: f32) -> f32 {
        let (sin, cos) = (TAU * f64::from(frequency) / f64::from(sample_rate)).sin_cos();
        (10.0 * self.power_ratio(sin, cos).log10()) as f32
    }

    fn power_ratio(self, sin: f64, cos: f64) -> f64 {
        let power = |[first, middle, last]: [f64; 3]| {
            ((first + last) * cos + middle).powi(2) + ((first - last) * sin).powi(2)
        };
        power(self.b).max(1.0e-30) / power([1.0, self.a[0], self.a[1]])
    }

    pub(crate) fn cascade_db(filters: &[Self], sin: f64, cos: f64) -> f32 {
        // Bound each product to keep arbitrarily long user-provided cascades representable.
        let mut total = 0.0;
        for chunk in filters.chunks(4) {
            total += 10.0
                * chunk
                    .iter()
                    .map(|f| f.power_ratio(sin, cos))
                    .product::<f64>()
                    .log10();
        }
        total as f32
    }

    /// Apply this filter with caller-owned transposed direct-form II state.
    pub fn process(self, sample: f32, state: &mut [f64; 2]) -> f32 {
        let sample = f64::from(sample);
        let output = self.b[0] * sample + state[0];
        state[0] = self.b[1] * sample - self.a[0] * output + state[1];
        state[1] = self.b[2] * sample - self.a[1] * output;
        output as f32
    }
}

/// A windowed, allocation-free transform after initialization. Bin i is
/// sample_rate * i / FFT_SIZE. The output is single-sided peak amplitude dBFS.
#[derive(Debug)]
pub struct Spectrum {
    window: Vec<f64>,
    twiddles: Vec<(f64, f64)>,
    data: Vec<(f64, f64)>,
}

impl Default for Spectrum {
    fn default() -> Self {
        Self {
            window: (0..FFT_SIZE)
                .map(|i| 0.5 - 0.5 * (TAU * i as f64 / FFT_SIZE as f64).cos())
                .collect(),
            twiddles: (0..FFT_SIZE / 2)
                .map(|i| {
                    let (sin, cos) = (-TAU * i as f64 / FFT_SIZE as f64).sin_cos();
                    (cos, sin)
                })
                .collect(),
            data: vec![(0.0, 0.0); FFT_SIZE],
        }
    }
}

impl Spectrum {
    pub fn measure(&mut self, samples: &[f32; FFT_SIZE], output: &mut [f32; FFT_SIZE / 2]) {
        for (i, (&sample, &window)) in samples.iter().zip(&self.window).enumerate() {
            self.data[i.reverse_bits() >> (usize::BITS - FFT_SIZE.ilog2())] =
                (f64::from(sample) * window, 0.0);
        }
        let mut size = 2;
        while size <= FFT_SIZE {
            let stride = FFT_SIZE / size;
            for chunk in self.data.chunks_exact_mut(size) {
                let (left, right) = chunk.split_at_mut(size / 2);
                for (i, (a, b)) in left.iter_mut().zip(right).enumerate() {
                    let (cos, sin) = self.twiddles[i * stride];
                    let (real, imag) = (b.0 * cos - b.1 * sin, b.0 * sin + b.1 * cos);
                    *b = (a.0 - real, a.1 - imag);
                    *a = (a.0 + real, a.1 + imag);
                }
            }
            size *= 2;
        }
        for (i, (out, &(real, imag))) in output.iter_mut().zip(&self.data).enumerate() {
            let scale = if i == 0 { 2.0 } else { 4.0 };
            *out = super::audio_meter::amplitude_dbfs(
                ((real * real + imag * imag).sqrt() * scale / FFT_SIZE as f64) as f32,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combined_response_matches_individual_filters_over_multiple_chunks() {
        let filters = [
            FilterKind::Bell,
            FilterKind::HighShelf,
            FilterKind::LowShelf,
            FilterKind::HighPass,
            FilterKind::Bell,
        ]
        .map(|kind| {
            Biquad::new(
                48000.0,
                EqBand {
                    frequency: 1950.0,
                    gain_db: 11.0,
                    q: 3.5,
                    kind,
                },
            )
        });
        for frequency in [20.0, 220.0, 1950.0, 5000.0, 20000.0] {
            let (sin, cos) = (TAU * f64::from(frequency) / 48000.0).sin_cos();
            let reference: f32 = filters
                .iter()
                .map(|f| f.magnitude_db(frequency, 48000.0))
                .sum();
            assert!((Biquad::cascade_db(&filters, sin, cos) - reference).abs() < 0.0001);
        }
    }

    #[test]
    fn filter_endpoints_and_fft_calibration() {
        for kind in [
            FilterKind::LowShelf,
            FilterKind::Bell,
            FilterKind::HighPass,
            FilterKind::HighShelf,
        ] {
            let band = EqBand {
                frequency: 1_000.0,
                gain_db: 6.0,
                q: 0.707,
                kind,
            };
            let filter = Biquad::new(48_000.0, band);
            let low = filter.magnitude_db(20.0, 48_000.0);
            let high = filter.magnitude_db(20_000.0, 48_000.0);
            match kind {
                FilterKind::LowShelf => assert!((low - 6.0).abs() < 0.1 && high.abs() < 0.1),
                FilterKind::HighShelf => assert!(low.abs() < 0.1 && (high - 6.0).abs() < 0.1),
                FilterKind::Bell => {
                    assert!((filter.magnitude_db(1_000.0, 48_000.0) - 6.0).abs() < 0.001)
                }
                FilterKind::HighPass => assert!(low < -50.0 && high.abs() < 0.1),
            }
        }
        let input =
            std::array::from_fn(|i| (TAU * 32.0 * i as f64 / FFT_SIZE as f64).sin() as f32 * 0.5);
        let mut output = [0.0; FFT_SIZE / 2];
        Spectrum::default().measure(&input, &mut output);
        assert!((output[32] + 6.0206).abs() < 0.01);
        assert!(output[40] < -100.0);
    }
}
