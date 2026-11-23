use egui::{Align2, Color32, FontId, Pos2, Rect, Response, Sense, Stroke, Ui, pos2, vec2};

use crate::{MeterOrientation, UiTokens};

pub const MIN_DBFS: f32 = -60.0;

#[derive(Clone, Copy, Debug)]
pub struct ChannelLevel {
    pub rms: f32,
    pub peak: f32,
    pub held: f32,
    pub clipped: bool,
}

impl Default for ChannelLevel {
    fn default() -> Self {
        Self {
            rms: MIN_DBFS,
            peak: MIN_DBFS,
            held: MIN_DBFS,
            clipped: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum MeterStyle {
    Continuous,
    Segmented,
}

/// Visual ballistics. Ingest block measurements in dBFS, with elapsed wall time
/// in seconds. Clip state stays latched until the host resets it.
#[derive(Clone, Copy, Debug, Default)]
pub struct PeakBallistics {
    pub level: ChannelLevel,
    hold_remaining: f32,
}

impl PeakBallistics {
    pub fn advance(&mut self, rms: f32, peak: f32, elapsed: f32) {
        self.advance_in(rms, peak, elapsed, MIN_DBFS);
    }

    fn at_floor(floor: f32) -> Self {
        Self {
            level: ChannelLevel {
                rms: floor,
                peak: floor,
                held: floor,
                clipped: false,
            },
            hold_remaining: 0.0,
        }
    }

    fn advance_in(&mut self, rms: f32, peak: f32, elapsed: f32, floor: f32) {
        let elapsed = elapsed.max(0.0);
        self.level.clipped |= peak >= 0.0;
        let peak = peak.clamp(floor, 0.0);
        let rms = rms.clamp(floor, peak);
        self.level.peak = peak.max(self.level.peak - 28.0 * elapsed);
        self.level.rms = rms
            .max(self.level.rms - 38.0 * elapsed)
            .min(self.level.peak);
        if peak >= self.level.held {
            self.level.held = peak;
            self.hold_remaining = 0.65;
        } else {
            let decay_time = (elapsed - self.hold_remaining).max(0.0);
            self.hold_remaining = (self.hold_remaining - elapsed).max(0.0);
            self.level.held = (self.level.held - 18.0 * decay_time).max(self.level.peak);
        }
    }
}

pub fn amplitude_dbfs(amplitude: f32) -> f32 {
    20.0 * amplitude.abs().max(1.0e-12).log10()
}

/// Stereo PPM display. The supplied channel labels and reset labels are
/// host-localized. Clicking a clip LED clears only its channel.
pub fn stereo_meter(
    ui: &mut Ui,
    tokens: UiTokens,
    orientation: MeterOrientation,
    style: MeterStyle,
    channels: &mut [ChannelLevel; 2],
    names: [&str; 2],
    reset_labels: [&str; 2],
) -> Response {
    let vertical = orientation == MeterOrientation::Vertical;
    let h = tokens.density.row_height();
    let size = if vertical {
        vec2(82.0, h * 6.0 + 16.0)
    } else {
        vec2(ui.available_width(), h * 2.15)
    };
    let (outer, response) = ui.allocate_exact_size(size, Sense::hover());
    if !ui.is_rect_visible(outer) {
        return response;
    }
    let theme = tokens.theme;
    let pixel = ui.ctx().pixels_per_point().recip();
    let lanes = if vertical {
        Rect::from_min_max(
            outer.min + vec2(0.0, 18.0),
            pos2(outer.left() + h, outer.bottom() - 16.0),
        )
    } else {
        Rect::from_min_max(outer.min + vec2(22.0, 3.0), outer.max - vec2(19.0, 17.0))
    };
    let painter = ui.painter_at(outer);
    let font = FontId::monospace(9.5);
    let colors = theme.meter_zone_inks();
    let face = MeterFace {
        painter: &painter,
        tokens,
        orientation,
        style,
        floor: MIN_DBFS,
        zoom: 1.0,
    };
    for (index, level) in channels.iter_mut().enumerate() {
        let lane = if vertical {
            let mid = lanes.center().x;
            if index == 0 {
                Rect::from_min_max(lanes.min, pos2(mid - pixel * 0.5, lanes.bottom()))
            } else {
                Rect::from_min_max(pos2(mid + pixel * 0.5, lanes.top()), lanes.max)
            }
        } else {
            let mid = lanes.center().y;
            if index == 0 {
                Rect::from_min_max(lanes.min, pos2(lanes.right(), mid - pixel * 0.5))
            } else {
                Rect::from_min_max(pos2(lanes.left(), mid + pixel * 0.5), lanes.max)
            }
        };
        face.lane(lane, *level);
        let label = if vertical {
            lane.center_bottom() + vec2(0.0, 8.0)
        } else {
            lane.left_center() - vec2(11.0, 0.0)
        };
        painter.text(
            label,
            Align2::CENTER_CENTER,
            names[index],
            font.clone(),
            theme.text_primary(),
        );
        let center = if vertical {
            lane.center_top() - vec2(0.0, 8.0)
        } else {
            lane.right_center() + vec2(9.0, 0.0)
        };
        let hit = Rect::from_center_size(center, vec2(15.0, 15.0));
        let reset = ui.interact(hit, response.id.with(index), Sense::click());
        reset.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, reset.enabled(), reset_labels[index])
        });
        if reset.clicked() {
            level.clipped = false;
        }
        painter.circle_filled(
            center,
            4.0,
            if level.clipped {
                colors[2]
            } else {
                theme.bg_canvas().lerp_to_gamma(colors[2], 0.25)
            },
        );
        theme.focus(ui, &reset);
        reset.on_hover_text(reset_labels[index]);
    }
    face.scale(lanes, STEREO_TICKS);
    response
}

const STEREO_TICKS: &[(i16, &str)] = &[
    (-60, "-60"),
    (-54, "-54"),
    (-48, "-48"),
    (-42, "-42"),
    (-36, "-36"),
    (-30, "-30"),
    (-24, "-24"),
    (-18, "-18"),
    (-12, "-12"),
    (-6, "-6"),
    (0, "0"),
];
const MONO_TICKS: &[(i16, &str)] = &[
    (-120, "-120"),
    (-100, "-100"),
    (-80, "-80"),
    (-60, "-60"),
    (-40, "-40"),
    (-30, "-30"),
    (-20, "-20"),
    (-10, "-10"),
    (0, "0"),
];

struct MeterFace<'a> {
    painter: &'a egui::Painter,
    tokens: UiTokens,
    orientation: MeterOrientation,
    style: MeterStyle,
    floor: f32,
    zoom: f32,
}

impl MeterFace<'_> {
    fn lane(&self, lane: Rect, level: ChannelLevel) {
        paint_lane(
            self.painter,
            lane,
            self.tokens.theme,
            self.orientation,
            self.style,
            self.floor,
            level,
        );
    }

    fn scale(&self, lanes: Rect, ticks: &[(i16, &str)]) {
        paint_scale(
            self.painter,
            lanes,
            self.tokens.theme,
            self.orientation,
            self.floor,
            ticks,
            self.zoom,
        );
    }
}

pub const RMS_FLOOR_DBFS: f32 = -120.0;

#[derive(Clone, Copy)]
struct RmsMotion {
    ballistics: PeakBallistics,
    time: f64,
}

/// Single-channel RMS version of the shared meter. A missing reading clears the
/// lane immediately. The optional target is a document value supplied by the caller.
#[derive(Debug, Clone, Copy)]
pub struct RmsMeter {
    pub lane: Rect,
    pub zoom: f32,
    pub reading: Option<f32>,
    pub target: Option<(f32, Color32)>,
}

impl RmsMeter {
    pub fn show(self, ui: &Ui, painter: &egui::Painter, id: egui::Id, tokens: UiTokens) {
        if !ui.is_rect_visible(self.lane) {
            return;
        }
        let floor = RMS_FLOOR_DBFS;
        let now = ui.input(|input| input.time);
        let level = if let Some(value) = self.reading.filter(|value| value.is_finite()) {
            let value = value.clamp(floor, 0.0);
            let mut state = ui
                .data(|data| data.get_temp::<RmsMotion>(id))
                .unwrap_or(RmsMotion {
                    ballistics: PeakBallistics::at_floor(floor),
                    time: now,
                });
            #[expect(
                clippy::cast_possible_truncation,
                reason = "elapsed display time is bounded"
            )]
            let elapsed = (now - state.time).clamp(0.0, 10.0) as f32;
            state.ballistics.advance_in(value, value, elapsed, floor);
            state.time = now;
            let level = state.ballistics.level;
            if level.peak > value || level.held > level.peak {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(16));
            }
            ui.data_mut(|data| data.insert_temp(id, state));
            level
        } else {
            ui.data_mut(|data| data.remove::<RmsMotion>(id));
            PeakBallistics::at_floor(floor).level
        };
        let face = MeterFace {
            painter,
            tokens,
            orientation: MeterOrientation::Vertical,
            style: MeterStyle::Segmented,
            floor,
            zoom: self.zoom,
        };
        face.lane(self.lane, level);
        face.scale(self.lane, MONO_TICKS);
        if let Some((db, ink)) = self.target {
            let y = meter_position(self.lane, MeterOrientation::Vertical, floor, db);
            let stroke = Stroke::new(1.5 * self.zoom, ink);
            painter.hline(
                self.lane.left() - 4.0 * self.zoom..=self.lane.right() + 5.0 * self.zoom,
                y,
                stroke,
            );
            painter.circle_filled(
                pos2(self.lane.right() + 5.0 * self.zoom, y),
                2.5 * self.zoom,
                ink,
            );
        }
    }

    pub fn db_at(self, point: Pos2) -> f32 {
        let fraction = ((point.y - self.lane.top()) / self.lane.height()).clamp(0.0, 1.0);
        RMS_FLOOR_DBFS * fraction
    }
}

fn meter_position(lane: Rect, orientation: MeterOrientation, floor: f32, db: f32) -> f32 {
    let fraction = ((db - floor) / -floor).clamp(0.0, 1.0);
    match orientation {
        MeterOrientation::Vertical => egui::lerp(lane.y_range(), 1.0 - fraction),
        MeterOrientation::Horizontal => egui::lerp(lane.x_range(), fraction),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "shared meter lane geometry and appearance"
)]
fn paint_lane(
    painter: &egui::Painter,
    lane: Rect,
    theme: crate::theme::Theme,
    orientation: MeterOrientation,
    style: MeterStyle,
    floor: f32,
    level: ChannelLevel,
) {
    let vertical = orientation == MeterOrientation::Vertical;
    let pixel = painter.ctx().pixels_per_point().recip();
    let scale = |db| meter_position(lane, orientation, floor, db);
    let colors = theme.meter_zone_inks();
    let position = |lane: Rect, db| {
        if vertical {
            pos2(lane.center().x, scale(db))
        } else {
            pos2(scale(db), lane.center().y)
        }
    };
    let interval = |start, end| {
        let a: Pos2 = position(lane, start);
        let b: Pos2 = position(lane, end);
        if vertical {
            Rect::from_min_max(pos2(lane.left(), b.y), pos2(lane.right(), a.y))
        } else {
            Rect::from_min_max(pos2(a.x, lane.top()), pos2(b.x, lane.bottom()))
        }
    };
    for ((start, end), color) in [(floor, -20.0), (-20.0, -9.0), (-9.0, 0.0)]
        .into_iter()
        .zip(colors)
    {
        painter.rect_filled(
            interval(start, end),
            0,
            theme.bg_canvas().lerp_to_gamma(color, 0.22),
        );
        if level.peak > start {
            painter.rect_filled(interval(start, level.peak.min(end)), 0, color);
        }
    }
    if let MeterStyle::Segmented = style {
        let step = if floor < MIN_DBFS { 4.0 } else { 2.0 };
        let mut db = -step;
        while db > floor {
            let tick = position(lane, db);
            db -= step;
            let stroke = Stroke::new(pixel, theme.bg_canvas());
            if vertical {
                painter.hline(lane.x_range(), tick.y, stroke);
            } else {
                painter.vline(tick.x, lane.y_range(), stroke);
            }
        }
    }
    let inset = if vertical {
        vec2(lane.width() * 0.27, 0.0)
    } else {
        vec2(0.0, lane.height() * 0.3)
    };
    if level.rms > floor {
        painter.rect_filled(
            interval(floor, level.rms).shrink2(inset),
            0,
            Color32::from_white_alpha(if theme.dark { 150 } else { 195 }),
        );
    }
    if level.held > floor {
        let tick = position(lane, level.held);
        let stroke = Stroke::new(2.0 * pixel, theme.text_primary());
        if vertical {
            painter.hline(lane.x_range(), tick.y, stroke);
        } else {
            painter.vline(tick.x, lane.y_range(), stroke);
        }
    }
}

fn paint_scale(
    painter: &egui::Painter,
    lanes: Rect,
    theme: crate::theme::Theme,
    orientation: MeterOrientation,
    floor: f32,
    ticks: &[(i16, &str)],
    zoom: f32,
) {
    let vertical = orientation == MeterOrientation::Vertical;
    let pixel = painter.ctx().pixels_per_point().recip();
    let font = FontId::monospace(9.5 * zoom);
    for &(db, label) in ticks {
        let at = meter_position(lanes, orientation, floor, f32::from(db));
        let stroke = Stroke::new(pixel, theme.text_muted);
        if vertical {
            painter.hline(
                lanes.right() + zoom..=lanes.right() + 5.0 * zoom,
                at,
                stroke,
            );
            painter.text(
                pos2(lanes.right() + 8.0 * zoom, at),
                Align2::LEFT_CENTER,
                label,
                font.clone(),
                theme.text_muted,
            );
        } else {
            painter.vline(at, lanes.bottom()..=lanes.bottom() + 4.0 * zoom, stroke);
            if lanes.width() >= 280.0
                || db % 12 == 0 && lanes.width() >= 150.0
                || f32::from(db) == floor
                || db == 0
            {
                painter.text(
                    pos2(at, lanes.bottom() + 5.0 * zoom),
                    Align2::CENTER_TOP,
                    label,
                    font.clone(),
                    theme.text_muted,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_meter_shares_ballistics_at_the_trigger_floor_and_clears_on_disconnect() {
        let ctx = egui::Context::default();
        let lane = Rect::from_min_size(pos2(8.0, 8.0), vec2(40.0, 120.0));
        let id = egui::Id::unique("rms-test");
        for (index, reading) in [Some(-70.0), Some(-100.0), None].into_iter().enumerate() {
            let time = [0.0, 0.2, 0.3][index];
            let mut state = None;
            ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    let meter = RmsMeter {
                        lane,
                        zoom: 1.0,
                        reading,
                        target: Some((-35.0, Color32::YELLOW)),
                    };
                    assert_eq!(meter.db_at(lane.left_top()), 0.0);
                    assert_eq!(meter.db_at(lane.left_bottom()), RMS_FLOOR_DBFS);
                    meter.show(ui, ui.painter(), id, UiTokens::resolve(&ctx));
                    state = ui.data(|data| data.get_temp::<RmsMotion>(id));
                },
            )
            .drop_without_applying_deltas();
            match index {
                0 => assert_eq!(state.unwrap().ballistics.level.rms, -70.0),
                1 => {
                    let level = state.unwrap().ballistics.level;
                    assert!(-100.0 < level.rms && level.rms < -70.0);
                    assert_eq!(level.held, -70.0);
                }
                _ => assert!(state.is_none()),
            }
        }
    }

    #[test]
    fn clip_latches_and_hold_decays_after_the_hold_window() {
        let mut meter = PeakBallistics::default();
        meter.advance(-12.0, 1.0, 0.01);
        assert!(meter.level.clipped);
        assert_eq!(meter.level.held, 0.0);
        meter.advance(-55.0, -42.0, 0.5);
        assert_eq!(meter.level.held, 0.0);
        meter.advance(-55.0, -42.0, 0.5);
        assert!(meter.level.held < 0.0 && meter.level.held >= meter.level.peak);
        assert!(meter.level.clipped);
    }
}
