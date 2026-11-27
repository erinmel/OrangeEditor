use std::sync::Arc;

use egui::{Align2, FontId, Rect, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};

use crate::{
    UiTokens,
    frequency::{Biquad, EqBand, FilterKind},
};

const MIN_HZ: f32 = 20.0;
const MAX_HZ: f32 = 20_000.0;
const GAIN: f32 = 18.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EqView {
    Eq,
    Spectrum,
    Overlay,
}

/// Shared logarithmic placement for response curves, FFT traces and band editing.
pub fn frequency_fraction(hz: f32, lower: f32, upper: f32) -> f32 {
    ((hz / lower).ln() / (upper / lower).ln()).clamp(0.0, 1.0)
}

pub const FREQUENCY_TICKS: &[(f32, &str)] = &[
    (20.0, "20"),
    (50.0, "50"),
    (100.0, "100"),
    (200.0, "200"),
    (500.0, "500"),
    (1_000.0, "1k"),
    (2_000.0, "2k"),
    (5_000.0, "5k"),
    (10_000.0, "10k"),
    (20_000.0, "20k"),
];

/// The common plot surface, axes and typography. Callers provide the data
/// range and place graph-specific overlays and interaction handles.
#[derive(Debug)]
pub struct PlotAxes {
    pub plot: Rect,
    pub theme: crate::theme::Theme,
    pub zoom: f32,
}

impl PlotAxes {
    /// The plot area. Square, like the grid and data drawn to its edges.
    pub fn frame(&self, painter: &egui::Painter, outer: Rect) {
        painter.rect(
            outer,
            0.0,
            self.theme.bg_canvas(),
            self.theme.border_subtle(),
            StrokeKind::Inside,
        );
    }

    pub fn frequency(&self, painter: &egui::Painter, range: [f32; 2], unit: Option<&str>) {
        let [lower, upper] = range;
        let mut previous = f32::NEG_INFINITY;
        for &(hz, label) in FREQUENCY_TICKS {
            if hz < lower || hz > upper {
                continue;
            }
            let x = egui::lerp(self.plot.x_range(), frequency_fraction(hz, lower, upper));
            painter.vline(x, self.plot.y_range(), self.theme.border_subtle());
            let next_to_unit = unit.is_some() && self.plot.right() - x < 20.0 * self.zoom;
            if !next_to_unit && x - previous >= 26.0 * self.zoom {
                painter.text(
                    pos2(x, self.plot.bottom() + 4.0 * self.zoom),
                    Align2::CENTER_TOP,
                    label,
                    self.font(),
                    self.theme.text_muted,
                );
                previous = x;
            }
        }
        if let Some(unit) = unit {
            painter.text(
                self.plot.right_bottom() + vec2(0.0, 4.0 * self.zoom),
                Align2::RIGHT_TOP,
                unit,
                self.font(),
                self.theme.text_muted,
            );
        }
    }

    pub fn db(
        &self,
        painter: &egui::Painter,
        range: [f32; 2],
        ticks: &[(f32, &str)],
        right: bool,
        grid: bool,
    ) {
        let [lower, upper] = range;
        for &(db, label) in ticks {
            let y = egui::lerp(
                self.plot.y_range(),
                ((upper - db) / (upper - lower)).clamp(0.0, 1.0),
            );
            if grid {
                painter.hline(
                    self.plot.x_range(),
                    y,
                    if db == 0.0 {
                        self.theme.border_strong
                    } else {
                        self.theme.border_subtle()
                    },
                );
            }
            let (x, anchor, color) = if right {
                (
                    self.plot.right() + 3.0 * self.zoom,
                    Align2::LEFT_CENTER,
                    self.theme.info,
                )
            } else {
                (
                    self.plot.left() - 4.0 * self.zoom,
                    Align2::RIGHT_CENTER,
                    self.theme.text_muted,
                )
            };
            painter.text(pos2(x, y), anchor, label, self.font(), color);
        }
    }

    fn font(&self) -> FontId {
        FontId::monospace(10.0 * self.zoom)
    }
}

struct TraceMesh<T> {
    clip: Rect,
    stroke: Stroke,
    scale: f32,
    options: egui::epaint::TessellationOptions,
    samples: Vec<T>,
    mesh: Arc<egui::Mesh>,
}

/// Reuses a tessellated line until its samples or display geometry change.
/// Include caller-specific sampling parameters in the ID, such as the FFT rate.
pub fn paint_cached_trace<T: Clone + PartialEq + Send + Sync + 'static>(
    painter: &egui::Painter,
    id: egui::Id,
    clip: Rect,
    stroke: Stroke,
    samples: &[T],
    points: impl FnOnce() -> Vec<egui::Pos2>,
) {
    if samples.len() < 2 || !painter.clip_rect().intersects(clip) {
        return;
    }
    let ctx = painter.ctx();
    let scale = ctx.pixels_per_point();
    let options = ctx.tessellation_options(|opts| *opts);
    let cached = ctx.data(|data| data.get_temp::<Arc<TraceMesh<T>>>(id));
    let cached = cached
        .filter(|cached| {
            cached.clip == clip
                && cached.stroke == stroke
                && cached.scale == scale
                && cached.options == options
                && cached.samples == samples
        })
        .unwrap_or_else(|| {
            let mut mesh = egui::Mesh::default();
            egui::epaint::Tessellator::new(scale, options, [0, 0], Vec::new())
                .tessellate_shape(Shape::line(points(), stroke), &mut mesh);
            let cached = Arc::new(TraceMesh {
                clip,
                stroke,
                scale,
                options,
                samples: samples.to_vec(),
                mesh: mesh.into(),
            });
            ctx.data_mut(|data| data.insert_temp(id, cached.clone()));
            cached
        });
    painter
        .with_clip_rect(clip)
        .add(Shape::mesh(cached.mesh.clone()));
}

/// Interactive response and analyzer display. Supply contiguous FFT bins in
/// dBFS from DC up to (but excluding) Nyquist. An empty spectrum is allowed.
#[derive(Debug)]
pub struct EqPlot<'a> {
    pub bands: &'a mut [EqBand],
    pub selected: &'a mut usize,
    pub band_labels: &'a [&'a str],
    pub view: EqView,
    pub bins: &'a [f32],
    pub sample_rate: f32,
}

impl EqPlot<'_> {
    pub fn show(self, ui: &mut Ui, tokens: UiTokens) {
        self.show_visible(ui, tokens);
    }

    /// Returns true when the plot intersects the viewport.
    pub fn show_visible(self, ui: &mut Ui, tokens: UiTokens) -> bool {
        if !self.sample_rate.is_finite() || self.sample_rate <= 2.0 * MIN_HZ {
            return false;
        }
        let max_hz = MAX_HZ.min(self.sample_rate * 0.499);
        if max_hz <= MIN_HZ {
            return false;
        }
        let theme = tokens.theme;
        let height = 8.0 * tokens.density.row_height() + 32.0;
        let (outer, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        if !ui.is_rect_visible(outer) || outer.width() < 76.0 {
            return false;
        }
        let plot = Rect::from_min_max(outer.min + vec2(32.0, 13.0), outer.max - vec2(30.0, 23.0));
        let hz_at = |fraction| MIN_HZ * (max_hz / MIN_HZ).powf(fraction);
        let x = |hz: f32| egui::lerp(plot.x_range(), frequency_fraction(hz, MIN_HZ, max_hz));
        let y = |db: f32| egui::lerp(plot.y_range(), ((GAIN - db) / (2.0 * GAIN)).clamp(0.0, 1.0));
        let spectrum_y = |db: f32| egui::lerp(plot.y_range(), (-db / 84.0).clamp(0.0, 1.0));
        let point = |band: EqBand| {
            let frequency = if band.frequency.is_finite() {
                band.frequency.clamp(MIN_HZ, max_hz)
            } else {
                MIN_HZ
            };
            pos2(
                x(frequency),
                y(if band.kind == FilterKind::HighPass {
                    -3.0
                } else {
                    band.gain_db
                }),
            )
        };
        if self.view != EqView::Spectrum {
            for (index, band) in self.bands.iter_mut().enumerate() {
                let handle = ui.interact(
                    Rect::from_center_size(point(*band), vec2(23.0, 23.0)).intersect(outer),
                    response.id.with(index),
                    Sense::click_and_drag(),
                );
                handle.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::Role::Button,
                        handle.enabled(),
                        self.band_labels.get(index).copied().unwrap_or_default(),
                    )
                });
                if handle.clicked() || handle.drag_started() {
                    *self.selected = index;
                }
                if handle.hovered() {
                    // Shift and the wheel change the band's width.
                    let direction = if ui.input(|input| input.modifiers.shift) {
                        crate::take_wheel(ui)
                    } else {
                        0.0
                    };
                    if direction != 0.0 {
                        band.q = (band.q * if direction > 0.0 { 1.1 } else { 1.0 / 1.1 })
                            .clamp(0.1, 18.0);
                        *self.selected = index;
                    }
                }
                if handle.dragged()
                    && let Some(pointer) = handle.interact_pointer_pos()
                {
                    band.frequency =
                        hz_at(((pointer.x - plot.left()) / plot.width()).clamp(0.0, 1.0));
                    if band.kind != FilterKind::HighPass {
                        band.gain_db = egui::lerp(
                            GAIN..=-GAIN,
                            ((pointer.y - plot.top()) / plot.height()).clamp(0.0, 1.0),
                        );
                    }
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                }
            }
        }
        let painter = ui.painter_at(outer);
        let font = FontId::monospace(10.0);
        let axes = PlotAxes {
            plot,
            theme,
            zoom: 1.0,
        };
        axes.frame(&painter, outer);
        if self.view != EqView::Spectrum {
            axes.db(
                &painter,
                [-GAIN, GAIN],
                &[
                    (-18., "-18"),
                    (-12., "-12"),
                    (-6., "-6"),
                    (0., "0"),
                    (6., "+6"),
                    (12., "+12"),
                    (18., "+18"),
                ],
                false,
                true,
            );
        }
        if self.view != EqView::Eq {
            axes.db(
                &painter,
                [-84., 0.],
                &[
                    (-84., "-84"),
                    (-60., "-60"),
                    (-36., "-36"),
                    (-12., "-12"),
                    (0., "0"),
                ],
                true,
                self.view == EqView::Spectrum,
            );
        }
        axes.frequency(&painter, [MIN_HZ, max_hz], None);
        let painter = painter.with_clip_rect(plot.expand(7.0).intersect(outer));
        if self.view != EqView::Eq {
            let columns = (plot.width() * ui.ctx().pixels_per_point()).ceil().max(1.0) as usize;
            let id = response.id.with("fft-columns");
            let key = (self.bins.len(), self.sample_rate.to_bits(), columns);
            let cached = ui.ctx().data(|data| data.get_temp::<Arc<BinLayout>>(id));
            let layout = cached
                .filter(|layout| layout.key == key)
                .unwrap_or_else(|| {
                    let layout = Arc::new(BinLayout::new(key));
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(id, layout.clone()));
                    layout
                });
            paint_cached_trace(
                &painter,
                response.id.with(("fft-trace", self.sample_rate.to_bits())),
                plot.expand(7.0),
                Stroke::new(1.3, theme.info),
                self.bins,
                || {
                    layout
                        .groups
                        .iter()
                        .filter_map(|(fraction, range)| {
                            let peak = self.bins[range.clone()]
                                .iter()
                                .copied()
                                .filter(|v| v.is_finite())
                                .fold(f32::NEG_INFINITY, f32::max);
                            peak.is_finite().then(|| {
                                pos2(egui::lerp(plot.x_range(), *fraction), spectrum_y(peak))
                            })
                        })
                        .collect()
                },
            );
        }
        if self.view != EqView::Spectrum {
            let count = (plot.width() * ui.ctx().pixels_per_point())
                .ceil()
                .clamp(128.0, 2048.0) as u16;
            let id = response.id.with("response-curve");
            let cached = cached_curve(ui.ctx(), id, self.bands, self.sample_rate, count);
            paint_cached_trace(
                &painter,
                id.with("mesh"),
                plot.expand(7.0),
                Stroke::new(2.0, theme.accent_primary()),
                &cached.values,
                || {
                    cached
                        .values
                        .iter()
                        .map(|&(i, db)| {
                            let t = f32::from(i) / f32::from(count);
                            pos2(egui::lerp(plot.x_range(), t), y(db))
                        })
                        .collect()
                },
            );
            if let Some(pointer) = response.hover_pos().filter(|p| plot.contains(*p)) {
                let hz = hz_at((pointer.x - plot.left()) / plot.width());
                let db: f32 = cached
                    .filters
                    .iter()
                    .map(|filter| filter.magnitude_db(hz, self.sample_rate))
                    .sum();
                painter.vline(pointer.x, plot.y_range(), theme.border_strong);
                painter.circle_filled(pos2(pointer.x, y(db)), 2.5, theme.accent_primary());
                painter.text(
                    plot.right_top() + vec2(-5.0, 5.0),
                    Align2::RIGHT_TOP,
                    format!("{hz:.0} Hz / {db:+.1} dB"),
                    font.clone(),
                    theme.text_primary(),
                );
            }
            for (index, band) in self.bands.iter().enumerate() {
                let at = point(*band);
                let fill = if index == *self.selected {
                    theme.accent_primary()
                } else {
                    theme.bg_elevated()
                };
                let ink = if index == *self.selected {
                    theme.on_primary
                } else {
                    theme.text_primary()
                };
                painter.circle_filled(at, 8.0, fill);
                painter.circle_stroke(at, 8.0, Stroke::new(1.0, theme.accent_primary()));
                painter.text(
                    at,
                    Align2::CENTER_CENTER,
                    (index + 1).to_string(),
                    font.clone(),
                    ink,
                );
            }
        }
        true
    }
}

struct BinLayout {
    key: (usize, u32, usize),
    groups: Vec<(f32, std::ops::Range<usize>)>,
}

impl BinLayout {
    fn new(key: (usize, u32, usize)) -> Self {
        let (bins, rate, columns) = key;
        let rate = f32::from_bits(rate);
        let max = MAX_HZ.min(rate * 0.499);
        let scale = (max / MIN_HZ).ln().recip();
        let mut groups: Vec<(f32, std::ops::Range<usize>)> = Vec::new();
        let mut previous = None;
        for index in 1..bins {
            let hz = index as f32 * (rate * 0.5 / bins as f32);
            if hz > max {
                break;
            }
            if hz < MIN_HZ {
                continue;
            }
            let fraction = (hz / MIN_HZ).ln() * scale;
            let column = (fraction * columns as f32) as usize;
            if previous == Some(column)
                && let Some((_, range)) = groups.last_mut()
            {
                range.end = index + 1;
            } else {
                groups.push((fraction, index..index + 1));
                previous = Some(column);
            }
        }
        Self { key, groups }
    }
}

struct FrequencyGrid {
    rate: f32,
    count: u16,
    points: Vec<(f64, f64)>,
}

struct ResponseCurve {
    bands: Vec<EqBand>,
    rate: f32,
    count: u16,
    filters: Vec<Biquad>,
    values: Vec<(u16, f32)>,
}

fn cached_curve(
    ctx: &egui::Context,
    id: egui::Id,
    bands: &[EqBand],
    rate: f32,
    count: u16,
) -> Arc<ResponseCurve> {
    if let Some(curve) = ctx.data(|data| data.get_temp::<Arc<ResponseCurve>>(id))
        && curve.bands == bands
        && curve.rate == rate
        && curve.count == count
    {
        return curve;
    }
    let max_hz = MAX_HZ.min(rate * 0.499);
    let filters: Vec<_> = bands.iter().map(|&band| Biquad::new(rate, band)).collect();
    // Dragging a node changes coefficients, but leaves the logarithmic grid intact.
    let grid_id = id.with("unit-circle");
    let grid = ctx
        .data(|data| data.get_temp::<Arc<FrequencyGrid>>(grid_id))
        .filter(|grid| grid.rate == rate && grid.count == count)
        .unwrap_or_else(|| {
            let points = (0..=count)
                .map(|index| {
                    let hz = MIN_HZ * (max_hz / MIN_HZ).powf(f32::from(index) / f32::from(count));
                    (std::f64::consts::TAU * f64::from(hz) / f64::from(rate)).sin_cos()
                })
                .collect();
            let grid = Arc::new(FrequencyGrid {
                rate,
                count,
                points,
            });
            ctx.data_mut(|data| data.insert_temp(grid_id, grid.clone()));
            grid
        });
    // Stream the dense samples directly into the bounded-error polyline.
    let mut samples = grid
        .points
        .iter()
        .map(|&(sin, cos)| Biquad::cascade_db(&filters, sin, cos));
    let mut previous = samples.next().expect("frequency grid includes its origin");
    let mut anchor_db = previous;
    let mut values = vec![(0, previous)];
    let mut anchor = 0;
    let (mut low, mut high) = (f32::NEG_INFINITY, f32::INFINITY);
    for (i, db) in (1..=count).zip(samples) {
        let slope = (db - anchor_db) / f32::from(i - anchor);
        if slope < low || slope > high {
            anchor = i - 1;
            anchor_db = previous;
            values.push((anchor, previous));
            low = f32::NEG_INFINITY;
            high = f32::INFINITY;
        }
        let distance = f32::from(i - anchor);
        low = low.max((db - anchor_db - 0.025) / distance);
        high = high.min((db - anchor_db + 0.025) / distance);
        previous = db;
    }
    values.push((count, previous));
    let curve = Arc::new(ResponseCurve {
        bands: bands.to_vec(),
        rate,
        count,
        filters,
        values,
    });
    ctx.data_mut(|data| data.insert_temp(id, Arc::clone(&curve)));
    curve
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_trace_only_rebuilds_for_new_samples_or_geometry() {
        let context = egui::Context::default();
        let mut builds = 0;
        context
            .run_ui(egui::RawInput::default(), |ui| {
                let rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(180.0, 100.0));
                let id = ui.make_persistent_id("trace");
                for (plot, samples) in [
                    (rect, [0, 1]),
                    (rect, [0, 1]),
                    (rect, [1, 0]),
                    (rect.expand(2.0), [1, 0]),
                ] {
                    paint_cached_trace(
                        ui.painter(),
                        id,
                        plot,
                        Stroke::new(1.0, egui::Color32::WHITE),
                        &samples,
                        || {
                            builds += 1;
                            vec![plot.left_bottom(), plot.right_top()]
                        },
                    );
                }
            })
            .drop_without_applying_deltas();
        assert_eq!(builds, 3);
    }

    #[test]
    fn narrow_bell_simplification_preserves_sampled_response() {
        let bands = [EqBand {
            frequency: 2850.0,
            gain_db: 17.0,
            q: 18.0,
            kind: FilterKind::Bell,
        }];
        let curve = cached_curve(
            &egui::Context::default(),
            egui::Id::unique("peak"),
            &bands,
            48000.0,
            2048,
        );
        assert!(curve.values.len() < 1024);
        for pair in curve.values.windows(2) {
            let [(a, low), (b, high)] = [pair[0], pair[1]];
            for i in a..=b {
                let hz = MIN_HZ * (MAX_HZ / MIN_HZ).powf(f32::from(i) / 2048.0);
                let actual = curve.filters[0].magnitude_db(hz, 48000.0);
                let drawn = egui::lerp(low..=high, f32::from(i - a) / f32::from(b - a));
                assert!((actual - drawn).abs() < 0.0251);
            }
        }
    }

    #[test]
    fn spectrum_columns_cover_visible_bins_without_gaps() {
        let layout = BinLayout::new((1024, 48000_f32.to_bits(), 120));
        assert!(layout.groups.len() <= 121);
        assert_eq!(layout.groups.first().unwrap().1.start, 1);
        assert_eq!(layout.groups.last().unwrap().1.end, 854);
        for pair in layout.groups.windows(2) {
            assert_eq!(pair[0].1.end, pair[1].1.start);
        }
    }

    #[test]
    fn dragging_eq_handle_updates_frequency_and_gain() {
        let context = egui::Context::default();
        let mut bands = [EqBand {
            frequency: 1000.0,
            gain_db: 0.0,
            q: 1.2,
            kind: FilterKind::Bell,
        }];
        let mut selected = 0;
        let mut frame = |events| {
            let output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(600.0, 500.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    EqPlot {
                        bands: &mut bands,
                        selected: &mut selected,
                        band_labels: &["Bell"],
                        view: EqView::Eq,
                        bins: &[],
                        sample_rate: 48_000.0,
                    }
                    .show(ui, UiTokens::resolve(&context));
                },
            );
            let point = output.shapes.iter().find_map(|clipped| {
                if let Shape::Circle(circle) = &clipped.shape
                    && circle.radius == 8.0
                {
                    Some(circle.center)
                } else {
                    None
                }
            });
            output.drop_without_applying_deltas();
            (point, bands[0])
        };
        let point = frame(vec![]).0.unwrap();
        frame(vec![egui::Event::PointerMoved(point)]);
        frame(vec![egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }]);
        let (_, updated) = frame(vec![egui::Event::PointerMoved(point + vec2(60.0, -30.0))]);
        assert!(updated.frequency > 1500.0);
        assert!(updated.gain_db > 3.0);
        assert_eq!(updated.q, 1.2);
    }
}
