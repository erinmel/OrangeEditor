use super::*;
use crate::declarations::{AUDIO_SILENCE_DB as SILENCE_DB, audio_level_fraction};

const MIN_HZ: f32 = 20.0;
const MAX_HZ: f32 = 24_000.0;
const SPECTRUM_PLOT_TOP: f32 = 29.0;
const SPECTRUM_PLOT_BOTTOM: f32 = 185.0;
pub(super) const NOISE_CHART: f32 = 204.0;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BandEdit {
    Add([u32; 2]),
    Change(u16, Option<serde_json::Value>),
}

pub(super) fn combined(node: &NodeView) -> bool {
    node.definition == "org.orange.audio/multiband-spectrum"
}

/// The band a splitter output belongs to: `band_stream#3` is band 3.
pub(super) fn band_of(port: &str) -> Option<u16> {
    port.strip_prefix("band_stream#")?.parse().ok()
}

/// Where band `band` of a splitter starts, in Hz.
pub(super) fn band_low(node: &NodeView, band: u16) -> Option<f64> {
    node.settings["bands"][band.to_string()]["low_hz"].as_f64()
}

/// Where a splitter card at `rect` draws its plot.
pub(super) fn plot_rect(rect: Rect, zoom: f32) -> Rect {
    Rect::from_min_max(
        rect.min + Vec2::new(CARD_PADDING + 24.0, BODY_TOP + SPECTRUM_PLOT_TOP) * zoom,
        Pos2::new(
            rect.right() - CARD_PADDING * zoom,
            rect.top() + (BODY_TOP + SPECTRUM_PLOT_BOTTOM) * zoom,
        ),
    )
}

/// Where `hz` is across a plot, from 0 at its left to 1 at its right.
pub(super) fn plot_fraction(hz: f64) -> f32 {
    #[expect(clippy::cast_possible_truncation, reason = "screen positions")]
    let hz = hz as f32;
    ((hz / MIN_HZ).ln() / (MAX_HZ / MIN_HZ).ln()).clamp(0.0, 1.0)
}

pub(super) fn is_noise(node: &NodeView) -> bool {
    node.definition == "org.orange.audio/noise-level"
}

pub(super) fn chart_height(node: &NodeView, open: bool) -> f32 {
    if combined(node) {
        if open {
            SPECTRUM_PLOT_BOTTOM + 22.0
        } else {
            SPECTRUM_PLOT_TOP
        }
    } else {
        0.0
    }
}

pub(super) fn primary_band(node: &NodeView, port: &PortView) -> Option<u16> {
    if !combined(node) || port.input {
        return None;
    }
    port.id
        .strip_prefix("band_stream#")?
        .parse::<u16>()
        .ok()
        .filter(|band| node.settings["bands"][band.to_string()].is_object())
}

#[derive(Clone, Copy, Default)]
struct Selection {
    start: Option<f32>,
    range: Option<[f32; 2]>,
}

fn db_y(plot: Rect, db: f64) -> f32 {
    plot.bottom() - plot.height() * audio_level_fraction(db)
}

fn trace(painter: &egui::Painter, plot: Rect, samples: &[f64], zoom: f32, color: Color32) {
    let normalized: Vec<_> = samples
        .iter()
        .map(|db| crate::declarations::audio_level_fraction(*db))
        .collect();
    orange_ui_controls::paint_line(
        painter,
        plot,
        &normalized,
        Stroke::new((1.5 * zoom).max(1.0), color),
    );
}

fn select_range(
    ui: &mut egui::Ui,
    plot: Rect,
    id: egui::Id,
    controls: bool,
) -> (Selection, Option<[f32; 2]>) {
    let mut selection: Selection = ui.data_mut(|data| data.get_temp(id).unwrap_or_default());
    let mut completed = None;
    if controls {
        let drag = ui
            .interact(plot, id, Sense::drag())
            .on_hover_text(t("spectrum-drag-hint"));
        let fraction = |x: f32| ((x - plot.left()) / plot.width()).clamp(0.0, 1.0);
        if drag.drag_started()
            && let Some(pos) = ui.input(|input| input.pointer.press_origin())
        {
            selection.start = Some(fraction(pos.x));
            selection.range = None;
        }
        if (drag.dragged() || drag.drag_stopped())
            && let (Some(start), Some(pos)) = (
                selection.start,
                ui.input(|input| input.pointer.interact_pos()),
            )
        {
            let end = fraction(pos.x);
            selection.range = (((start - end).abs() * plot.width()) >= 1.0)
                .then_some([start.min(end), start.max(end)]);
        }
        if drag.drag_stopped() {
            completed = selection.range;
            selection = Selection::default();
        }
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            selection = Selection::default();
            completed = None;
        }
        ui.data_mut(|data| data.insert_temp(id, selection));
    }
    (selection, completed)
}

fn highlight(painter: &egui::Painter, plot: Rect, [low, high]: [f32; 2], color: Color32) {
    let at = |f: f32| plot.left() + plot.width() * f;
    let range = Rect::from_min_max(
        Pos2::new(at(low), plot.top()),
        Pos2::new(at(high), plot.bottom()),
    );
    painter.rect_filled(range, 0.0, color.gamma_multiply(0.20));
    painter.vline(
        range.left(),
        range.y_range(),
        Stroke::new(1.0, color.gamma_multiply(0.75)),
    );
    painter.vline(
        range.right(),
        range.y_range(),
        Stroke::new(1.0, color.gamma_multiply(0.75)),
    );
}

#[derive(Clone)]
struct BandDraft {
    saved: serde_json::Value,
    pending: serde_json::Value,
}

pub(super) fn chart_toggle(
    ui: &egui::Ui,
    painter: &egui::Painter,
    rect: Rect,
    zoom: f32,
    id: egui::Id,
    open: bool,
) -> bool {
    let at = rect.min + Vec2::new(CARD_PADDING, BODY_TOP + 13.0) * zoom;
    let button = Rect::from_min_size(
        at - Vec2::new(0.0, 11.0) * zoom,
        Vec2::new(164.0, 22.0) * zoom,
    );
    let response = ui
        .interact(button, id, Sense::click())
        .on_hover_text(t("spectrum-live-trace-help"));
    let colors = Palette::of(ui);
    let tint = if response.hovered() {
        colors.selected
    } else {
        colors.text
    };
    chart_heading(painter, rect, zoom, open, tint);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::Role::Button,
            ui.is_enabled(),
            open,
            t("spectrum-live-trace"),
        )
    });
    response.clicked()
}

pub(super) fn chart_heading(
    painter: &egui::Painter,
    rect: Rect,
    zoom: f32,
    open: bool,
    tint: Color32,
) {
    let at = rect.min + Vec2::new(CARD_PADDING, BODY_TOP + 13.0) * zoom;
    painter.text(
        at,
        Align2::LEFT_CENTER,
        if open {
            crate::icons::COLLAPSE
        } else {
            crate::icons::EXPAND
        },
        FontId::new(15.0 * zoom, crate::icons::family()),
        tint,
    );
    painter.text(
        at + Vec2::new(19.0 * zoom, 0.0),
        Align2::LEFT_CENTER,
        t("spectrum-live-trace"),
        FontId::proportional(12.0 * zoom),
        tint,
    );
}

#[expect(clippy::too_many_arguments, reason = "one output row's drawing state")]
pub(super) fn band_row(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    node: &NodeView,
    row: &card::Row<'_>,
    rect: Rect,
    zoom: f32,
    controls: bool,
    band: u16,
) -> Option<BandEdit> {
    row.right?;
    let colors = Palette::of(ui);
    let at = Pos2::new(
        rect.right() - (CARD_PADDING + 9.0) * zoom,
        rect.top() + row.anchor() * zoom,
    );
    let id = ui.scope_id().with(("band", node.id, band));
    let remove = ui
        .interact(
            Rect::from_center_size(at, Vec2::splat(18.0 * zoom)),
            id.with("remove"),
            if controls {
                Sense::click()
            } else {
                Sense::hover()
            },
        )
        .on_hover_text(t("spectrum-remove-band"));
    painter.text(
        at,
        Align2::CENTER_CENTER,
        crate::icons::CLOSE,
        FontId::new(14.0 * zoom, crate::icons::family()),
        if remove.hovered() && controls {
            colors.error
        } else {
            mix(colors.weak, colors.text, 0.55)
        },
    );
    remove.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Button,
            ui.is_enabled() && controls,
            t("spectrum-remove-band"),
        )
    });
    if remove.clicked() && controls {
        return Some(BandEdit::Change(band, None));
    }
    if !controls {
        return None;
    }

    let saved = &node.settings["bands"][band.to_string()];
    let mut draft = ui
        .data(|data| data.get_temp::<BandDraft>(id))
        .filter(|edit| edit.saved == *saved)
        .unwrap_or_else(|| BandDraft {
            saved: saved.clone(),
            pending: saved.clone(),
        });
    let mut commit = false;
    let slot = Rect::from_min_max(
        Pos2::new(rect.right() - 294.0 * zoom, at.y - 11.0 * zoom),
        Pos2::new(at.x - 14.0 * zoom, at.y + 11.0 * zoom),
    );
    ui.scope_builder(UiBuilder::new().max_rect(slot).id_salt(id), |ui| {
        ui.set_clip_rect(ui.clip_rect().intersect(slot));
        fields::style_controls(ui, zoom);
        ui.spacing_mut().item_spacing.x = 4.0 * zoom;
        ui.horizontal(|ui| {
            let mut name = draft.pending["name"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let response = ui
                .add(
                    egui::TextEdit::singleline(&mut name)
                        .desired_width(88.0 * zoom)
                        .char_limit(60)
                        .hint_text(t("name")),
                )
                .on_hover_text(t("name"));
            draft.pending["name"] = name.into();
            commit |= response.lost_focus();
            ui.spacing_mut().interact_size.x = 73.0 * zoom;
            ui.visuals_mut().widgets.inactive.weak_bg_fill = colors.background;
            commit |= number_field(
                ui,
                &mut draft.pending,
                "low_hz",
                20.0,
                20.0..=24_000.0,
                " Hz",
                t("spectrum-low-hz"),
            );
            commit |= number_field(
                ui,
                &mut draft.pending,
                "high_hz",
                120.0,
                20.0..=24_000.0,
                " Hz",
                t("spectrum-high-hz"),
            );
        });
    });
    if commit
        && draft.pending["low_hz"].as_f64() < draft.pending["high_hz"].as_f64()
        && draft.pending != draft.saved
    {
        ui.data_mut(|data| data.remove::<BandDraft>(id));
        return Some(BandEdit::Change(band, Some(draft.pending)));
    }
    ui.data_mut(|data| data.insert_temp(id, draft));
    None
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_arguments,
    reason = "bounded frequency plot and one chart's drawing state"
)]
pub(super) fn draw_combined(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    node: &NodeView,
    rect: Rect,
    zoom: f32,
    samples: Option<&[f64]>,
    controls: bool,
    pointed: Option<u16>,
) -> Option<BandEdit> {
    let plot = plot_rect(rect, zoom);
    let id = ui.scope_id().with(("spectrum", node.id));
    let colors = Palette::of(ui);
    let bands = node.settings["bands"].as_object();
    let ids: Vec<u16> = node
        .ports
        .iter()
        .filter_map(|port| port.id.strip_prefix("band_stream#")?.parse().ok())
        .filter(|band: &u16| bands.is_some_and(|bands| bands.contains_key(&band.to_string())))
        .collect();
    let settings = |band: u16| bands.and_then(|bands| bands.get(&band.to_string()));
    let full = ids.len() >= 12;
    if full {
        ui.interact(plot, id.with("full"), Sense::hover())
            .on_hover_text(t("spectrum-band-limit"));
    }
    let (selection, completed) = select_range(ui, plot, id.with("range"), controls && !full);
    let fraction = |hz: f32| ((hz / MIN_HZ).ln() / (MAX_HZ / MIN_HZ).ln()).clamp(0.0, 1.0);
    let hertz = |ratio: f32| (MIN_HZ * (MAX_HZ / MIN_HZ).powf(ratio)).round() as u32;
    let valid = |[low, high]: &[u32; 2]| low < high && *high <= MAX_HZ as u32;
    let added = completed
        .map(|range| range.map(hertz))
        .filter(|range| ids.len() < 12 && valid(range))
        .map(BandEdit::Add);
    painter.rect_filled(plot, 4.0 * zoom, colors.background);
    let font = FontId::proportional(10.5 * zoom);
    for db in [0, -30, -60, -90, -120] {
        let y = db_y(plot, f64::from(db));
        painter.hline(plot.x_range(), y, Stroke::new(1.0, colors.grid));
        painter.text(
            Pos2::new(plot.left() - 4.0 * zoom, y),
            Align2::RIGHT_CENTER,
            db.to_string(),
            font.clone(),
            colors.weak,
        );
    }
    for band in ids.iter().copied() {
        if let Some(settings) = settings(band)
            && let (Some(low), Some(high)) =
                (settings["low_hz"].as_f64(), settings["high_hz"].as_f64())
        {
            let color = if pointed == Some(band) {
                colors.exec
            } else {
                colors.weak
            };
            highlight(
                painter,
                plot,
                [fraction(low as f32), fraction(high as f32)],
                color,
            );
        }
    }
    if let Some(range) = selection.range {
        highlight(painter, plot, range, colors.exec);
    }
    let resized = if controls {
        resize_edges(
            ui,
            painter,
            plot,
            zoom,
            id.with("edges"),
            &ids,
            &settings,
            &colors,
        )
    } else {
        None
    };
    for (hz, label) in [
        (20.0, "20"),
        (100.0, "100"),
        (500.0, "500"),
        (1_000.0, "1k"),
        (5_000.0, "5k"),
        (10_000.0, "10k"),
    ] {
        let x = plot.left() + plot.width() * fraction(hz);
        painter.vline(x, plot.y_range(), Stroke::new(1.0, colors.grid));
        painter.text(
            Pos2::new(x, plot.bottom() + 4.0 * zoom),
            Align2::LEFT_TOP,
            label,
            font.clone(),
            colors.weak,
        );
    }
    painter.text(
        plot.right_bottom() + Vec2::new(0.0, 4.0 * zoom),
        Align2::RIGHT_TOP,
        "Hz",
        font.clone(),
        colors.weak,
    );
    if let Some(samples) = samples.filter(|samples| samples.len() > 1) {
        trace(painter, plot, samples, zoom, colors.selected);
    } else {
        painter.text(
            plot.center(),
            Align2::CENTER_CENTER,
            if node
                .ports
                .iter()
                .any(|p| p.input && p.id == "stream" && p.connected)
            {
                t("spectrum-await-audio")
            } else {
                t("spectrum-connect")
            },
            font,
            colors.weak,
        );
    }
    if controls
        && let Some(pointer) = ui
            .input(|input| input.pointer.hover_pos())
            .filter(|pos| plot.contains(*pos))
    {
        let hz = hertz((pointer.x - plot.left()) / plot.width());
        painter.vline(pointer.x, plot.y_range(), Stroke::new(zoom, colors.weak));
        painter.text(
            Pos2::new(plot.right(), plot.top() - 5.0 * zoom),
            Align2::RIGHT_BOTTOM,
            format!("{hz} Hz"),
            FontId::proportional(11.5 * zoom),
            colors.text,
        );
    }
    resized.or(added)
}

/// The edges of each band on the chart: dragged sideways, an edge moves and the band
/// keeps its new size when let go. Returns the change once the drag ends.
#[expect(clippy::too_many_arguments, reason = "one chart's drawing state")]
fn resize_edges<'a>(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    plot: Rect,
    zoom: f32,
    id: egui::Id,
    bands: &[u16],
    settings: &dyn Fn(u16) -> Option<&'a serde_json::Value>,
    colors: &Palette,
) -> Option<BandEdit> {
    let fraction = plot_fraction;
    let hertz = |x: f32| {
        let ratio = ((x - plot.left()) / plot.width()).clamp(0.0, 1.0);
        f64::from((MIN_HZ * (MAX_HZ / MIN_HZ).powf(ratio)).round())
    };
    let mut done = None;
    for &band in bands {
        let Some(saved) = settings(band) else {
            continue;
        };
        let (Some(low), Some(high)) = (saved["low_hz"].as_f64(), saved["high_hz"].as_f64()) else {
            continue;
        };
        for (edge, hz) in [("low_hz", low), ("high_hz", high)] {
            let x = plot.left() + plot.width() * fraction(hz);
            let grip = Rect::from_center_size(
                Pos2::new(x, plot.center().y),
                Vec2::new(8.0 * zoom, plot.height()),
            );
            let handle = ui
                .interact(grip, id.with((band, edge)), Sense::drag())
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
            let dragged_to = handle
                .interact_pointer_pos()
                .filter(|_| handle.dragged() || handle.drag_stopped())
                .map(|at| hertz(at.x));
            if handle.hovered() || handle.dragged() {
                let shown = dragged_to.unwrap_or(hz);
                let at = plot.left() + plot.width() * fraction(shown);
                painter.vline(at, plot.y_range(), Stroke::new(2.0 * zoom, colors.exec));
                let _ = painter.text(
                    Pos2::new(at, plot.top() - 5.0 * zoom),
                    Align2::CENTER_BOTTOM,
                    format!("{shown:.0} Hz"),
                    FontId::proportional(11.5 * zoom),
                    colors.text,
                );
            }
            if handle.drag_stopped()
                && let Some(to) = dragged_to
            {
                let (new_low, new_high) = if edge == "low_hz" {
                    (to, high)
                } else {
                    (low, to)
                };
                if new_low < new_high && (to - hz).abs() >= 1.0 {
                    let mut changed = saved.clone();
                    changed[edge] = serde_json::Value::from(to);
                    done = Some(BandEdit::Change(band, Some(changed)));
                }
            }
        }
    }
    done
}

fn number_field(
    ui: &mut egui::Ui,
    settings: &mut serde_json::Value,
    key: &str,
    default: f64,
    range: std::ops::RangeInclusive<f64>,
    suffix: &str,
    hint: &str,
) -> bool {
    let mut value = settings[key].as_f64().unwrap_or(default);
    let response = ui
        .add(egui::DragValue::new(&mut value).range(range).suffix(suffix))
        .on_hover_text(hint);
    if response.changed() {
        settings[key] = value.into();
    }
    response.lost_focus() || response.drag_stopped() || (response.changed() && !response.dragged())
}

pub(super) fn direction(mode: &str) -> &'static str {
    match mode {
        "below" => t("audio-goes-below"),
        "both" => t("audio-both-directions"),
        _ => t("audio-goes-above"),
    }
}

const METER_RELEASE_DB_PER_SECOND: f64 = 55.0;
const METER_PEAK_HOLD_SECONDS: f64 = 0.55;

#[derive(Clone, Copy)]
struct MeterMotion {
    fill: f64,
    peak: f64,
    peak_until: f64,
    last: f64,
}

impl MeterMotion {
    fn new(now: f64) -> Self {
        Self {
            fill: SILENCE_DB,
            peak: SILENCE_DB,
            peak_until: now,
            last: now,
        }
    }

    fn advance(&mut self, reading: Option<f64>, now: f64) -> bool {
        let target = reading.unwrap_or(SILENCE_DB).clamp(SILENCE_DB, 0.0);
        let elapsed = (now - self.last).max(0.0);
        self.fill = target.max(self.fill - elapsed * METER_RELEASE_DB_PER_SECOND);
        if target >= self.peak {
            self.peak = target;
            self.peak_until = now + METER_PEAK_HOLD_SECONDS;
        } else {
            let release = (now - self.last.max(self.peak_until)).max(0.0);
            self.peak = self
                .fill
                .max(self.peak - release * METER_RELEASE_DB_PER_SECOND);
        }
        self.last = now;
        self.fill > target || self.peak > self.fill
    }
}

pub(super) fn draw_noise(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    node: &NodeView,
    rect: Rect,
    zoom: f32,
    reading: Option<f64>,
    controls: bool,
) -> Option<f64> {
    let colors = Palette::of(ui);
    let bar = Rect::from_min_size(
        rect.min + Vec2::new(21.0, BODY_TOP + 10.0) * zoom,
        Vec2::new(40.0, 185.0) * zoom,
    );
    let id = ui.scope_id().with(("noise-threshold", node.id));
    let motion_id = id.with("motion");
    let now = ui.input(|input| input.time);
    let mut motion = ui
        .data(|data| data.get_temp::<MeterMotion>(motion_id))
        .unwrap_or_else(|| MeterMotion::new(now));
    if motion.advance(reading, now) {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));
    }
    ui.data_mut(|data| data.insert_temp(motion_id, motion));
    let threshold = node.settings["threshold_db"].as_f64().unwrap_or(-35.0);
    let mut current = ui
        .data(|data| data.get_temp::<f64>(id))
        .unwrap_or(threshold);
    let mut changed = None;
    if controls {
        let drag = ui
            .interact(
                bar.expand2(Vec2::new(12.0, 2.0) * zoom),
                id,
                Sense::click_and_drag(),
            )
            .on_hover_text(t("noise-drag-threshold"));
        if (drag.dragged() || drag.clicked() || drag.drag_stopped())
            && let Some(pointer) = drag.interact_pointer_pos()
        {
            let fraction = f64::from((bar.bottom() - pointer.y) / bar.height()).clamp(0.0, 1.0);
            current = (SILENCE_DB * (1.0 - fraction)).round();
            ui.data_mut(|data| data.insert_temp(id, current));
        }
        if drag.drag_stopped() || drag.clicked() {
            ui.data_mut(|data| {
                data.remove::<f64>(id);
            });
            if current != threshold {
                changed = Some(current);
            }
        }
    }
    let y = |db| db_y(bar, db);
    let fill = crate::declarations::audio_level_fraction(motion.fill);
    let peak = crate::declarations::audio_level_fraction(motion.peak);
    painter.rect_filled(bar, 0.0, colors.background);
    orange_ui_controls::paint_vertical_meter(
        painter,
        bar,
        fill,
        peak,
        &[
            orange_ui_controls::MeterStop {
                fraction: crate::declarations::audio_level_fraction(SILENCE_DB),
                color: Color32::from_rgb(22, 154, 82),
            },
            orange_ui_controls::MeterStop {
                fraction: crate::declarations::audio_level_fraction(-24.0),
                color: Color32::from_rgb(53, 217, 100),
            },
            orange_ui_controls::MeterStop {
                fraction: crate::declarations::audio_level_fraction(-18.0),
                color: Color32::from_rgb(242, 211, 65),
            },
            orange_ui_controls::MeterStop {
                fraction: crate::declarations::audio_level_fraction(-12.0),
                color: Color32::from_rgb(255, 190, 50),
            },
            orange_ui_controls::MeterStop {
                fraction: crate::declarations::audio_level_fraction(-7.0),
                color: Color32::from_rgb(244, 82, 66),
            },
            orange_ui_controls::MeterStop {
                fraction: crate::declarations::audio_level_fraction(0.0),
                color: Color32::from_rgb(255, 61, 76),
            },
        ],
        Color32::from_gray(240),
        0.23,
    );
    let font = FontId::proportional(10.5 * zoom);
    for db in [0, -10, -20, -30, -40, -60, -80, -100, -120] {
        let y = y(f64::from(db));
        painter.hline(
            bar.right()..=bar.right() + 4.0 * zoom,
            y,
            Stroke::new(1.0, colors.weak),
        );
        painter.text(
            Pos2::new(bar.right() + 7.0 * zoom, y),
            Align2::LEFT_CENTER,
            db.to_string(),
            font.clone(),
            colors.weak,
        );
    }
    painter.hline(
        bar.left() - 5.0 * zoom..=bar.right() + 6.0 * zoom,
        y(current),
        Stroke::new(2.0, colors.exec),
    );
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spectrum_and_meter_keep_quiet_levels_above_the_silence_floor() {
        let plot = Rect::from_min_size(Pos2::ZERO, Vec2::splat(120.0));
        assert_eq!(db_y(plot, 0.0), 0.0);
        assert_eq!(db_y(plot, -60.0), 60.0);
        assert_eq!(db_y(plot, -105.0), 105.0);
        assert_eq!(db_y(plot, orange_audio::SILENCE_DB), plot.bottom());
    }

    #[test]
    fn meter_rises_immediately_and_decays_with_a_held_peak() {
        let mut meter = MeterMotion::new(0.0);
        assert!(!meter.advance(Some(-3.0), 0.0));
        assert_eq!(meter.fill, -3.0);
        assert!(meter.advance(Some(-80.0), 0.2));
        assert_eq!(meter.fill, -14.0);
        assert_eq!(meter.peak, -3.0);
        assert!(meter.advance(Some(-80.0), 0.8));
        assert!(meter.fill < meter.peak && meter.peak < -3.0);
        assert!(!meter.advance(None, 3.0));
        assert_eq!(meter.fill, SILENCE_DB);
        assert_eq!(meter.peak, SILENCE_DB);
    }
}
