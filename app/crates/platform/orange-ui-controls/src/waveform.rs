//! Pixel-bounded audio waveform drawing. File previews can cache an envelope
//! once and reuse it at different plot widths.
use egui::{Color32, Mesh, Painter, Rect, Shape, Stroke, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaveformPeak {
    pub min: f32,
    pub max: f32,
}

impl WaveformPeak {
    fn sample(sample: f32) -> Self {
        let sample = if sample.is_finite() {
            sample.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        Self {
            min: sample,
            max: sample,
        }
    }

    fn union(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Waveform<'a> {
    Samples(&'a [f32]),
    Envelope(&'a [WaveformPeak]),
}

impl Waveform<'_> {
    fn len(self) -> usize {
        match self {
            Self::Samples(samples) => samples.len(),
            Self::Envelope(peaks) => peaks.len(),
        }
    }

    fn peak(self, range: std::ops::Range<usize>) -> WaveformPeak {
        match self {
            Self::Samples(samples) => samples[range]
                .iter()
                .copied()
                .map(WaveformPeak::sample)
                .reduce(WaveformPeak::union),
            Self::Envelope(peaks) => peaks[range]
                .iter()
                .map(|peak| WaveformPeak::sample(peak.min).union(WaveformPeak::sample(peak.max)))
                .reduce(WaveformPeak::union),
        }
        .unwrap_or_else(|| WaveformPeak::sample(0.0))
    }
}

fn buckets(length: usize, count: usize) -> impl Iterator<Item = std::ops::Range<usize>> {
    let base = length / count.max(1);
    let remainder = length % count.max(1);
    let mut start = 0;
    let mut extra = 0;
    (0..count).map(move |_| {
        extra += remainder;
        let wider = extra >= count;
        extra -= usize::from(wider) * count;
        let end = start + base + usize::from(wider);
        let range = start..end;
        start = end;
        range
    })
}

pub fn waveform_envelope(samples: &[f32], max_columns: usize) -> Vec<WaveformPeak> {
    let columns = max_columns.min(samples.len());
    buckets(samples.len(), columns)
        .map(|bucket| Waveform::Samples(samples).peak(bucket))
        .collect()
}

/// Draws a line when all raw samples fit, or a signed min/max envelope at display resolution.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "sample indices and physical widths become screen coordinates"
)]
pub fn paint_waveform(painter: &Painter, plot: Rect, data: Waveform<'_>, ink: Color32) {
    if !plot.is_positive() || !plot.is_finite() || !plot.size().is_finite() || data.len() == 0 {
        return;
    }
    let pixel = painter.ctx().pixels_per_point().recip();
    // Keep mesh size bounded even when a host gives us a huge offscreen plot.
    let max_columns = (plot.width() / pixel).ceil().clamp(1.0, 16_384.0) as usize;
    let y = |sample| plot.center().y - sample * plot.height() * 0.47;
    if let Waveform::Samples(samples) = data
        && (2..=max_columns).contains(&samples.len())
    {
        let points = samples
            .iter()
            .enumerate()
            .map(|(index, &sample)| {
                let x = egui::lerp(plot.x_range(), index as f32 / (samples.len() - 1) as f32);
                pos2(x, y(WaveformPeak::sample(sample).min))
            })
            .collect();
        painter.add(Shape::line(points, Stroke::new(1.1, ink)));
        return;
    }

    let columns = data.len().min(max_columns);
    let width = plot.width() / columns as f32;
    let mut mesh = Mesh::default();
    mesh.reserve_vertices(columns * 4);
    mesh.reserve_triangles(columns * 2);
    for (index, bucket) in buckets(data.len(), columns).enumerate() {
        let peak = data.peak(bucket);
        let top = y(peak.max);
        let bottom = y(peak.min);
        mesh.add_colored_rect(
            Rect::from_center_size(
                pos2(
                    plot.left() + (index as f32 + 0.5) * width,
                    f32::midpoint(top, bottom),
                ),
                vec2(width, (bottom - top).max(pixel)),
            ),
            ink,
        );
    }
    painter.add(Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_uneven_buckets_cover_the_input_and_empty_limits_are_safe() {
        let length = 200_003;
        let mut last = 0;
        for range in buckets(length, 110_001) {
            assert_eq!(range.start, last);
            assert!((1..=2).contains(&range.len()));
            last = range.end;
        }
        assert_eq!(last, length);
        assert!(waveform_envelope(&[0.5], 0).is_empty());
    }

    #[test]
    fn malformed_envelopes_are_normalized_before_rendering() {
        let data = Waveform::Envelope(&[
            WaveformPeak {
                min: 3.0,
                max: -4.0,
            },
            WaveformPeak {
                min: f32::NAN,
                max: f32::INFINITY,
            },
        ]);
        assert_eq!(
            data.peak(0..1),
            WaveformPeak {
                min: -1.0,
                max: 1.0
            }
        );
        assert_eq!(data.peak(1..2), WaveformPeak { min: 0.0, max: 0.0 });
    }

    #[test]
    fn envelope_preserves_asymmetric_impulses_and_tail() {
        let samples = [0.0, -0.75, 0.3, 0.1, 0.2, f32::NAN, 1.0];
        assert_eq!(
            waveform_envelope(&samples, 2),
            [
                WaveformPeak {
                    min: -0.75,
                    max: 0.3
                },
                WaveformPeak { min: 0.0, max: 1.0 },
            ]
        );
        assert!(waveform_envelope(&[], 32).is_empty());
    }
}
