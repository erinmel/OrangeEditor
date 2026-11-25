use super::*;
use crate::declarations::audio_level_fraction;
use orange_ui_controls::{
    UiTokens,
    audio_meter::{ChannelReading, RmsMeter},
    eq_plot::{PlotAxes, frequency_fraction, paint_cached_trace},
};

const MIN_HZ: f32 = 20.0;
const MAX_HZ: f32 = 24_000.0;

#[derive(Debug, Clone, Copy)]
pub(crate) enum ChannelLevels {
    Mono(ChannelReading),
    Stereo([ChannelReading; 2]),
    Multichannel { peak: f32, clipped: bool },
}

impl ChannelLevels {
    #[expect(clippy::cast_possible_truncation, reason = "audio display only")]
    pub(crate) fn from_value(value: &orange_host::Value) -> Option<Self> {
        use orange_host::Value;
        let Value::Record(record) = value else {
            return None;
        };
        if record.ty.as_str() != "org.orange.audio/channel-levels@1" {
            return None;
        }
        let [
            Some(Value::List(rms)),
            Some(Value::List(peak)),
            Some(Value::List(clipped)),
        ] = &record.fields[..]
        else {
            return None;
        };
        if rms.len() != peak.len() || rms.len() != clipped.len() {
            return None;
        }
        let reading = |index| {
            let (Some(Value::F64(rms)), Some(Value::F64(peak)), Some(Value::Bool(clipped))) =
                (rms.get(index), peak.get(index), clipped.get(index))
            else {
                return None;
            };
            (rms.is_finite() && peak.is_finite()).then_some(ChannelReading {
                rms: rms as f32,
                peak: peak as f32,
                clipped,
            })
        };
        match rms.len() {
            0 => None,
            1 => Some(Self::Mono(reading(0)?)),
            2 => Some(Self::Stereo([reading(0)?, reading(1)?])),
            n => {
                let (peak, clipped) =
                    (0..n).try_fold((-120.0_f32, false), |(peak, clipped), index| {
                        let channel = reading(index)?;
                        Some((peak.max(channel.peak), clipped || channel.clipped))
                    })?;
                Some(Self::Multichannel { peak, clipped })
            }
        }
    }
}
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
        && has_stream_input(node)
        && node.ports.iter().any(|port| {
            !port.input
                && !port.exec
                && port.id == "spectrum"
                && port.ty.as_deref() == Some("core/list@1<core/f64@1>")
        })
}

/// Where the band described by a splitter port starts, in Hz.
pub(super) fn band_low(node: &NodeView, port: &str) -> Option<f64> {
    let key = port.strip_prefix("band_stream#")?;
    node.settings["bands"][key]["low_hz"].as_f64()
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
    frequency_fraction(hz, MIN_HZ, MAX_HZ)
}

/// A node whose definition asks for the level meter and has the stream input and level
/// output it draws.
pub(crate) fn is_noise(node: &NodeView) -> bool {
    node.controls.visual == Some(orange_host::controls::Visual::LevelMeter)
        && has_stream_input(node)
        && node.ports.iter().filter(|port| port.input).count() == 1
        && node.ports.iter().any(|port| {
            !port.input
                && !port.exec
                && port.id == "level"
                && port.ty.as_deref() == Some("core/f64@1")
        })
}

fn has_stream_input(node: &NodeView) -> bool {
    node.ports.iter().any(|port| {
        port.input
            && !port.exec
            && port.id == "stream"
            && port.ty.as_deref() == Some("org.orange.audio/stream@1")
    })
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

fn trace(painter: &egui::Painter, id: egui::Id, plot: Rect, samples: &[f64], stroke: Stroke) {
    paint_cached_trace(painter, id, plot, stroke, samples, || {
        let len = samples.len();
        #[expect(clippy::cast_precision_loss, reason = "bounded display buckets")]
        samples
            .iter()
            .enumerate()
            .map(|(index, db)| {
                Pos2::new(
                    egui::lerp(plot.x_range(), (index as f32 + 0.5) / len as f32),
                    db_y(plot, *db),
                )
            })
            .collect()
    });
}

pub(crate) fn audio_status(node: &NodeView, running: bool, has_reading: bool) -> &'static str {
    audio_status_labels(node, running, has_reading).0
}

fn audio_status_labels(
    node: &NodeView,
    running: bool,
    has_reading: bool,
) -> (&'static str, &'static str) {
    if !node
        .ports
        .iter()
        .any(|p| p.input && p.id == "stream" && p.connected)
    {
        (t("spectrum-connect"), t("audio-display-no-input"))
    } else if !running {
        (t("audio-display-preview"), t("audio-display-preview-short"))
    } else if has_reading {
        (t("audio-display-live"), t("audio-display-live"))
    } else {
        (t("spectrum-await-audio"), t("audio-display-waiting-short"))
    }
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
    running: bool,
) -> Option<BandEdit> {
    let plot = plot_rect(rect, zoom);
    if !ui.is_rect_visible(plot) {
        return None;
    }
    let id = ui.scope_id().with(("spectrum", node.id));
    let colors = Palette::of(ui);
    let bands = node.ports.iter().filter_map(|port| {
        let key = port.id.strip_prefix("band_stream#")?;
        let band = key.parse::<u16>().ok()?;
        let settings = node.settings["bands"].get(key)?;
        Some((band, settings))
    });
    let full = bands.clone().count() >= 12;
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
        .filter(|range| !full && valid(range))
        .map(BandEdit::Add);
    let theme = UiTokens::for_surface(ui, orange_ui_controls::Surface::Card).theme;
    let axes = PlotAxes { plot, theme, zoom };
    axes.frame(painter, plot.expand(0.5 * zoom));
    axes.db(
        painter,
        [-120.0, 0.0],
        &[
            (0.0, "0"),
            (-30.0, "-30"),
            (-60.0, "-60"),
            (-90.0, "-90"),
            (-120.0, "-120"),
        ],
        false,
        true,
    );
    axes.frequency(painter, [MIN_HZ, MAX_HZ], Some("Hz"));
    let font = FontId::monospace(10.0 * zoom);
    for (band, settings) in bands.clone() {
        if let (Some(low), Some(high)) = (settings["low_hz"].as_f64(), settings["high_hz"].as_f64())
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
        resize_edges(ui, painter, plot, zoom, id.with("edges"), bands, &colors)
    } else {
        None
    };
    let samples = samples.filter(|samples| samples.len() > 1);
    let status = audio_status(node, running, samples.is_some());
    if let Some(samples) = samples {
        trace(
            painter,
            id.with("trace"),
            plot,
            samples,
            Stroke::new((1.3 * zoom).max(1.0), theme.info),
        );
    }
    painter.text(
        if samples.is_some() {
            plot.left_top() + Vec2::splat(5.0 * zoom)
        } else {
            plot.center()
        },
        if samples.is_some() {
            Align2::LEFT_TOP
        } else {
            Align2::CENTER_CENTER
        },
        status,
        font,
        colors.weak,
    );
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
fn resize_edges<'a>(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    plot: Rect,
    zoom: f32,
    id: egui::Id,
    bands: impl Iterator<Item = (u16, &'a serde_json::Value)>,
    colors: &Palette,
) -> Option<BandEdit> {
    let fraction = plot_fraction;
    let hertz = |x: f32| {
        let ratio = ((x - plot.left()) / plot.width()).clamp(0.0, 1.0);
        f64::from((MIN_HZ * (MAX_HZ / MIN_HZ).powf(ratio)).round())
    };
    let mut done = None;
    for (band, saved) in bands {
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

pub(crate) fn direction(mode: &str) -> &'static str {
    match mode {
        "below" => t("audio-goes-below"),
        "both" => t("audio-both-directions"),
        _ => t("audio-goes-above"),
    }
}

#[expect(clippy::too_many_arguments, reason = "one meter’s drawing state")]
pub(super) fn draw_noise(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    node: &NodeView,
    rect: Rect,
    zoom: f32,
    readings: (Option<f64>, Option<ChannelLevels>),
    controls: bool,
    running: bool,
) -> Option<f64> {
    let (reading, channels) = readings;
    let colors = Palette::of(ui);
    let bar = Rect::from_min_size(
        rect.min + Vec2::new(17.0, BODY_TOP + 18.0) * zoom,
        Vec2::new(49.0, 163.0) * zoom,
    );
    let id = ui.scope_id().with(("noise-threshold", node.id));
    #[expect(
        clippy::cast_possible_truncation,
        reason = "bounded display measurements"
    )]
    let mut meter = RmsMeter {
        lane: bar,
        zoom,
        reading: reading.map(|db| db as f32),
        target: None,
    };
    let threshold = crate::properties::ThresholdEdit::for_node(node);
    let mut changed = None;
    if controls && let Some(threshold) = threshold {
        let mut drag = ui
            .interact(
                bar.expand2(Vec2::new(8.0, 2.0) * zoom),
                id,
                Sense::click_and_drag(),
            )
            .on_hover_text(t("noise-drag-threshold"));
        let mut value = threshold.shown(ui.ctx());
        if (drag.dragged() || drag.clicked() || drag.drag_stopped())
            && let Some(pointer) = drag.interact_pointer_pos()
        {
            value = f64::from(meter.db_at(pointer).round());
            drag.mark_changed();
        }
        changed = threshold.update(&drag, value);
    }
    if let Some(threshold) = threshold {
        #[expect(clippy::cast_possible_truncation, reason = "display coordinate")]
        let dbfs = changed.unwrap_or_else(|| threshold.shown(ui.ctx())) as f32;
        meter.target = Some((dbfs, colors.exec));
    }
    let stereo = match channels {
        Some(ChannelLevels::Stereo([left, right])) => Some([("L", left), ("R", right)]),
        _ => None,
    };
    let single = match channels {
        Some(ChannelLevels::Mono(reading)) => Some((t("meter-mono"), reading)),
        Some(ChannelLevels::Multichannel { peak, clipped }) => meter
            .reading
            .map(|rms| (t("meter-mix"), ChannelReading { rms, peak, clipped })),
        _ => None,
    };
    let lanes = stereo
        .as_ref()
        .map_or(single.as_slice(), |pair| pair.as_slice());
    let meter_id = id.with("meter");
    let tokens = UiTokens::for_surface(ui, orange_ui_controls::Surface::Card);
    let preview = reading.is_none() && lanes.is_empty();
    if preview {
        meter.show_stereo_preview(ui, painter, meter_id, tokens, ["L", "R"]);
    } else {
        meter.show_channels(ui, painter, meter_id, tokens, lanes);
    }
    if preview || !lanes.is_empty() {
        let lights = Rect::from_min_max(bar.min - Vec2::new(0.0, 15.0) * zoom, bar.right_top());
        ui.interact(lights, id.with("clip-status"), Sense::hover())
            .on_hover_text(t("meter-clip-hint"));
    }
    let (status, short) = audio_status_labels(node, running, reading.is_some());
    let caption = Rect::from_min_size(
        Pos2::new(bar.left() - 4.0 * zoom, bar.bottom() + 14.0 * zoom),
        Vec2::new(97.0, 17.0) * zoom,
    );
    ui.interact(caption, id.with("status"), Sense::hover())
        .on_hover_text(status);
    fields::field_text_in(painter, caption, zoom, short, colors.weak, None);

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
    fn profile_trace_geometry() {
        use std::time::Instant;
        let ctx = egui::Context::default();
        let mut results: [Vec<f64>; 2] = Default::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            for (mode, times) in results.iter_mut().enumerate() {
                let left = Rect::from_min_size(Pos2::ZERO, Vec2::new(420.0, 156.0));
                let right = left.translate(Vec2::new(440.0, 0.0));
                let mut samples = [-45.0; 96];
                for frame in 0..600 {
                    if mode == 1 {
                        for (bin, value) in (0..96).zip(&mut samples) {
                            *value = -20.0 - f64::from((bin + frame) % 73);
                        }
                    }
                    let now = Instant::now();
                    for (id, plot) in [left, right].into_iter().enumerate() {
                        trace(
                            ui.painter(),
                            ui.make_persistent_id(id),
                            plot,
                            &samples,
                            Stroke::new(1.5, Color32::WHITE),
                        );
                    }
                    if frame >= 40 {
                        times.push(now.elapsed().as_secs_f64() * 1000.0);
                    }
                }
            }
        });
        output.drop_without_applying_deltas();
        for (mode, times) in ["unchanged", "new samples"].into_iter().zip(&mut results) {
            times.sort_by(f64::total_cmp);
            eprintln!(
                "two traces {mode}: median {:.4} ms p95 {:.4} ms",
                times[times.len() / 2],
                times[times.len() * 95 / 100]
            );
        }
    }
}
