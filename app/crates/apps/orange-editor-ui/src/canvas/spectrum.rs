use super::*;

/// The height of a control in a chart body, in graph units.
const FIELD_HEIGHT: f32 = 20.0;

const MIN_HZ: f32 = 20.0;
const MAX_HZ: f32 = 24_000.0;
const SPECTRUM_PLOT_TOP: f32 = 29.0;
const SPECTRUM_PLOT_BOTTOM: f32 = 185.0;
const SPECTRUM_CONTROLS_TOP: f32 = SPECTRUM_PLOT_BOTTOM + 22.0;
const BAND_CONTROLS_HEIGHT: f32 = 124.0;
const BAND_THRESHOLD_ROW: f32 = 44.0;
const NOISE_CHART: f32 = 214.0;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BandEdit {
    Add([u32; 2]),
    Change(u16, Option<serde_json::Value>),
    Move(u16, usize),
}

pub(super) fn combined(node: &NodeView) -> bool {
    matches!(
        node.definition.as_str(),
        "org.orange.audio/spectrum" | "org.orange.audio/multiband-spectrum"
    )
}

pub(super) fn is_noise(node: &NodeView) -> bool {
    node.definition == "org.orange.audio/noise-level"
}

pub(super) fn chart_height(node: &NodeView) -> f32 {
    if combined(node) {
        let editing = node.settings["bands"]
            .as_object()
            .is_some_and(|bands| !bands.is_empty());
        SPECTRUM_CONTROLS_TOP
            + if !editing {
                28.0
            } else if stream_bands(node)
                && !node.settings["bands"].as_object().is_some_and(|bands| {
                    bands
                        .values()
                        .any(|band| band["legacy_level"].as_bool() == Some(true))
                })
            {
                BAND_CONTROLS_HEIGHT
            } else {
                BAND_CONTROLS_HEIGHT + BAND_THRESHOLD_ROW
            }
    } else if is_noise(node) {
        NOISE_CHART
    } else {
        0.0
    }
}

fn stream_bands(node: &NodeView) -> bool {
    node.variadic.iter().any(|port| port.port == "band_stream")
}

#[derive(Clone, Copy, Default)]
struct Selection {
    start: Option<f32>,
    range: Option<[f32; 2]>,
}

fn trace(painter: &egui::Painter, plot: Rect, samples: &[f64], zoom: f32, color: Color32) {
    #[expect(
        clippy::cast_precision_loss,
        reason = "the spectrum has a bounded bucket count"
    )]
    let bucket_width = plot.width() / samples.len() as f32;
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "screen positions"
    )]
    let points = samples
        .iter()
        .enumerate()
        .map(|(index, db)| {
            let x = plot.left() + (index as f32 + 0.5) * bucket_width;
            let level = ((*db as f32 + 90.0) / 90.0).clamp(0.0, 1.0);
            Pos2::new(x, plot.bottom() - plot.height() * level)
        })
        .collect();
    painter
        .with_clip_rect(painter.clip_rect().intersect(plot))
        .add(egui::Shape::line(
            points,
            Stroke::new((1.5 * zoom).max(1.0), color),
        ));
}

fn select_range(
    ui: &mut egui::Ui,
    plot: Rect,
    id: egui::Id,
    controls: bool,
) -> (Selection, Option<f32>, Option<[f32; 2]>) {
    let mut selection: Selection = ui.data_mut(|data| data.get_temp(id).unwrap_or_default());
    let mut clicked = None;
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
            if completed.is_none() {
                clicked = ui
                    .input(|input| input.pointer.interact_pos())
                    .map(|pos| fraction(pos.x));
            }
            selection = Selection::default();
        }
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            selection = Selection::default();
            completed = None;
        }
        ui.data_mut(|data| data.insert_temp(id, selection));
    }
    (selection, clicked, completed)
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
    band: u16,
    saved: serde_json::Value,
    pending: serde_json::Value,
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "bounded frequency plot"
)]
pub(super) fn draw_combined(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    node: &NodeView,
    rect: Rect,
    zoom: f32,
    samples: Option<&[f64]>,
    controls: bool,
) -> Option<BandEdit> {
    let plot = Rect::from_min_max(
        rect.min + Vec2::new(CARD_PADDING + 24.0, BODY_TOP + SPECTRUM_PLOT_TOP) * zoom,
        Pos2::new(
            rect.right() - CARD_PADDING * zoom,
            rect.top() + (BODY_TOP + SPECTRUM_PLOT_BOTTOM) * zoom,
        ),
    );
    let id = ui.scope_id().with(("spectrum", node.id));
    let colors = Palette::of(ui);
    let trace_id = id.with("trace-visible");
    let mut trace_visible = ui
        .data(|data| data.get_temp::<bool>(trace_id))
        .unwrap_or(true);
    if zoom >= CONTROLS_ZOOM {
        let toggle = Rect::from_min_size(
            rect.min + Vec2::new(CARD_PADDING, BODY_TOP + 2.0) * zoom,
            Vec2::new(140.0, 22.0) * zoom,
        );
        ui.scope_builder(UiBuilder::new().max_rect(toggle).id_salt(trace_id), |ui| {
            super::fields::style_controls(ui, zoom);
            if ui
                .add_enabled(
                    controls,
                    egui::Checkbox::new(&mut trace_visible, t("spectrum-live-trace")),
                )
                .on_hover_text(t("spectrum-live-trace-help"))
                .changed()
            {
                ui.data_mut(|data| data.insert_temp(trace_id, trace_visible));
            }
        });
    }
    let bands = node.settings["bands"].as_object();
    let ids: Vec<u16> = node
        .ports
        .iter()
        .filter_map(|port| {
            port.id
                .strip_prefix(if stream_bands(node) {
                    "band_stream#"
                } else {
                    "band_level#"
                })?
                .parse()
                .ok()
        })
        .filter(|band: &u16| bands.is_some_and(|bands| bands.contains_key(&band.to_string())))
        .collect();
    let settings = |band: u16| bands.and_then(|bands| bands.get(&band.to_string()));
    let mut selected = ui
        .data(|data| data.get_temp::<u16>(id.with("selected")))
        .filter(|id| ids.contains(id))
        .or_else(|| ids.first().copied());
    let (selection, clicked, completed) = select_range(ui, plot, id.with("range"), controls);
    let fraction = |hz: f32| ((hz / MIN_HZ).ln() / (MAX_HZ / MIN_HZ).ln()).clamp(0.0, 1.0);
    let hertz = |ratio: f32| (MIN_HZ * (MAX_HZ / MIN_HZ).powf(ratio)).round() as u32;
    let valid = |[low, high]: &[u32; 2]| low < high && *high <= MAX_HZ as u32;
    let added = completed
        .map(|range| range.map(hertz))
        .filter(|range| ids.len() < 12 && valid(range))
        .map(BandEdit::Add);
    if let Some(hz) = clicked.map(|ratio| f64::from(hertz(ratio)))
        && let Some(band) = ids.iter().copied().find(|&band| {
            settings(band).is_some_and(|s| {
                matches!((s["low_hz"].as_f64(), s["high_hz"].as_f64()),
                (Some(low), Some(high)) if (low..=high).contains(&hz))
            })
        })
    {
        selected = Some(band);
    }
    painter.rect_filled(plot, 4.0 * zoom, colors.background);
    let font = FontId::proportional(10.5 * zoom);
    for db in [0, -30, -60, -90] {
        let y = plot.top() - (f64::from(db) / 90.0) as f32 * plot.height();
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
            let color = if selected == Some(band) {
                colors.selected
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
    if trace_visible && let Some(samples) = samples.filter(|samples| samples.len() > 1) {
        trace(painter, plot, samples, zoom, colors.selected);
    } else if trace_visible {
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
    let range = selection.range.map(|range| range.map(hertz)).filter(valid);
    if zoom < CONTROLS_ZOOM {
        return added;
    }
    let area = Rect::from_min_max(
        Pos2::new(
            rect.left() + CARD_PADDING * zoom,
            rect.top() + (BODY_TOP + SPECTRUM_CONTROLS_TOP) * zoom,
        ),
        Pos2::new(
            plot.right(),
            rect.top() + (BODY_TOP + chart_height(node)) * zoom,
        ),
    );
    ui.scope_builder(UiBuilder::new().max_rect(area).id_salt(id), |ui| {
        ui.set_clip_rect(ui.clip_rect().intersect(area));
        super::fields::style_controls(ui, zoom);
        if !controls {
            ui.disable();
        }
        ui.horizontal(|ui| {
            if !ids.is_empty() {
                egui::ComboBox::from_id_salt("band")
                    .selected_text(
                        selected
                            .and_then(settings)
                            .and_then(|s| s["name"].as_str())
                            .unwrap_or_default(),
                    )
                    .width(112.0 * zoom)
                    .wrap_mode(egui::TextWrapMode::Truncate)
                    .show_ui(ui, |ui| {
                        for band in ids.iter().copied() {
                            let name = settings(band)
                                .and_then(|s| s["name"].as_str())
                                .unwrap_or_default();
                            ui.selectable_value(&mut selected, Some(band), name);
                        }
                    });
            }
            let hint = if let Some([low, high]) = range {
                format!("{low}-{high} Hz")
            } else if ids.len() >= 12 {
                t("spectrum-band-limit").to_owned()
            } else {
                t("spectrum-drag-hint").to_owned()
            };
            ui.add(egui::Label::new(egui::RichText::new(hint).weak()).truncate());
        });
        let band = selected?;
        let saved = settings(band)?;
        let has_rules = !stream_bands(node) || saved["legacy_level"].as_bool() == Some(true);
        ui.data_mut(|data| data.insert_temp(id.with("selected"), band));
        let mut draft = ui
            .data(|data| data.get_temp::<BandDraft>(id.with("draft")))
            .filter(|draft| draft.band == band && draft.saved == *saved)
            .unwrap_or_else(|| BandDraft {
                band,
                saved: saved.clone(),
                pending: saved.clone(),
            });
        let mut name = draft.pending["name"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let mut mode = draft.pending["mode"].as_str().unwrap_or("above").to_owned();
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut name)
                    .char_limit(60)
                    .desired_width(145.0 * zoom)
                    .hint_text(t("name")),
            );
            if has_rules {
                egui::ComboBox::from_id_salt("direction")
                    .width(95.0 * zoom)
                    .truncate()
                    .selected_text(direction(&mode))
                    .show_ui(ui, |ui| {
                        for candidate in ["above", "below", "both"] {
                            ui.selectable_value(
                                &mut mode,
                                candidate.to_owned(),
                                direction(candidate),
                            );
                        }
                    });
            }
        });
        draft.pending["name"] = name.into();
        if mode != saved["mode"].as_str().unwrap_or("above") {
            draft.pending["mode"] = mode.into();
        } else if saved["mode"].is_null() {
            let _ = draft
                .pending
                .as_object_mut()
                .map(|fields| fields.remove("mode"));
        } else {
            draft.pending["mode"] = saved["mode"].clone();
        }
        ui.columns(2, |columns| {
            number_field(
                &mut columns[0],
                &mut draft.pending,
                "low_hz",
                20.0,
                20.0..=24_000.0,
                " Hz",
                t("spectrum-low-hz"),
            );
            number_field(
                &mut columns[1],
                &mut draft.pending,
                "high_hz",
                120.0,
                20.0..=24_000.0,
                " Hz",
                t("spectrum-high-hz"),
            );
        });
        if has_rules {
            ui.columns(3, |columns| {
                number_field(
                    &mut columns[0],
                    &mut draft.pending,
                    "threshold_db",
                    -35.0,
                    -120.0..=0.0,
                    " dBFS",
                    t("audio-setting-threshold-db"),
                );
                number_field(
                    &mut columns[1],
                    &mut draft.pending,
                    "hold_ms",
                    150.0,
                    0.0..=10_000.0,
                    " ms",
                    t("audio-setting-hold-ms"),
                );
                number_field(
                    &mut columns[2],
                    &mut draft.pending,
                    "reset_db",
                    4.0,
                    0.0..=24.0,
                    " dB",
                    t("audio-reset-margin"),
                );
            });
        }
        let valid = draft.pending["low_hz"].as_f64() < draft.pending["high_hz"].as_f64();
        let position = ids.iter().position(|&id| id == band)?;
        let edit = ui
            .horizontal(|ui| {
                let up = band_move_button(
                    ui,
                    zoom,
                    position > 0,
                    crate::icons::MOVE_UP,
                    t("spectrum-move-band-up"),
                );
                let down = band_move_button(
                    ui,
                    zoom,
                    position + 1 < ids.len(),
                    crate::icons::MOVE_DOWN,
                    t("spectrum-move-band-down"),
                );
                let apply = ui
                    .add_enabled(
                        valid && draft.pending != draft.saved,
                        egui::Button::new(t("spectrum-apply")),
                    )
                    .clicked();
                let remove = ui.button(t("spectrum-remove-band")).clicked();
                if apply {
                    Some(BandEdit::Change(band, Some(draft.pending.clone())))
                } else if remove {
                    Some(BandEdit::Change(band, None))
                } else if up {
                    Some(BandEdit::Move(band, position - 1))
                } else if down {
                    Some(BandEdit::Move(band, position + 1))
                } else {
                    None
                }
            })
            .inner;
        ui.data_mut(|data| data.insert_temp(id.with("draft"), draft));
        edit
    })
    .inner
    .or(added)
}

fn band_move_button(ui: &mut egui::Ui, zoom: f32, enabled: bool, icon: char, label: &str) -> bool {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(crate::icons::text(icon).size(14.0 * zoom))
            .min_size(Vec2::new(26.0, FIELD_HEIGHT) * zoom),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled() && enabled, label)
    });
    response.on_hover_text(label).clicked()
}

fn number_field(
    ui: &mut egui::Ui,
    settings: &mut serde_json::Value,
    key: &str,
    default: f64,
    range: std::ops::RangeInclusive<f64>,
    suffix: &str,
    hint: &str,
) {
    let mut value = settings[key].as_f64().unwrap_or(default);
    let edited = ui
        .vertical(|ui| {
            ui.add(egui::Label::new(egui::RichText::new(hint).weak()).truncate())
                .on_hover_text(hint);
            ui.add(egui::DragValue::new(&mut value).range(range).suffix(suffix))
                .changed()
        })
        .inner;
    if edited {
        settings[key] = value.into();
    }
}

pub(super) fn direction(mode: &str) -> &'static str {
    match mode {
        "below" => t("audio-goes-below"),
        "both" => t("audio-both-directions"),
        _ => t("audio-goes-above"),
    }
}

const METER_FLOOR: f64 = -120.0;
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
            fill: METER_FLOOR,
            peak: METER_FLOOR,
            peak_until: now,
            last: now,
        }
    }

    fn advance(&mut self, reading: Option<f64>, now: f64) -> bool {
        let target = reading.unwrap_or(METER_FLOOR).clamp(METER_FLOOR, 0.0);
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

fn meter_gradient(bar: Rect, y: impl Fn(f64) -> f32, brightness: f32) -> egui::Mesh {
    let mut mesh = egui::Mesh::default();
    for (index, (db, red, green, blue)) in (0_u32..).zip([
        (-120.0, 22, 154, 82),
        (-24.0, 53, 217, 100),
        (-18.0, 242, 211, 65),
        (-12.0, 255, 190, 50),
        (-7.0, 244, 82, 66),
        (0.0, 255, 61, 76),
    ]) {
        let row = y(db);
        let color = Color32::from_rgb(red, green, blue).gamma_multiply(brightness);
        let next = index * 2;
        mesh.colored_vertex(Pos2::new(bar.left(), row), color);
        mesh.colored_vertex(Pos2::new(bar.right(), row), color);
        if next != 0 {
            mesh.add_triangle(next - 2, next - 1, next);
            mesh.add_triangle(next, next - 1, next + 1);
        }
    }
    mesh
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "meter scale is bounded to screen points"
)]
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
            current = ((f64::from((bar.bottom() - pointer.y) / bar.height()) * 120.0) - 120.0)
                .clamp(-120.0, 0.0)
                .round();
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
    let y =
        |db: f64| bar.bottom() - ((db.clamp(-120.0, 0.0) + 120.0) / 120.0) as f32 * bar.height();
    let channel = |low, high| {
        Rect::from_min_max(
            Pos2::new(bar.left(), y(high)),
            Pos2::new(bar.right(), y(low)),
        )
    };
    painter.rect_filled(bar, 0.0, colors.background);
    painter.add(egui::Shape::mesh(meter_gradient(bar, y, 0.23)));
    if motion.fill > METER_FLOOR {
        let clip = painter
            .clip_rect()
            .intersect(channel(METER_FLOOR, motion.fill));
        painter
            .with_clip_rect(clip)
            .add(egui::Shape::mesh(meter_gradient(bar, y, 1.0)));
    }
    if motion.peak > motion.fill + 0.5 {
        painter.hline(
            bar.x_range(),
            y(motion.peak),
            Stroke::new(1.5 * zoom, Color32::from_gray(240)),
        );
    }
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
    let left = bar.right() + 53.0 * zoom;
    painter.text(
        Pos2::new(left, bar.top() + 22.0 * zoom),
        Align2::LEFT_CENTER,
        reading.map_or_else(
            || t("noise-await-audio").to_owned(),
            |db| format!("{db:.1} dBFS"),
        ),
        FontId::proportional(13.0 * zoom),
        colors.text,
    );
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(meter.fill, METER_FLOOR);
        assert_eq!(meter.peak, METER_FLOOR);
    }
}
