//! The properties panel: what the selected node settles (the declarations it names,
//! its settings, the types of a generic node) and the values on its inputs.
use super::*;
use crate::controls::{Dropdown as _, Surface};
use orange_host::controls::{Control, Region, Scale, Widget};
use orange_host::intrinsic;
use orange_host::{Connection, EventTiming, NodeView, PortView, ReferenceKind};
use serde_json::{Value as Json, json};

const HTTP_METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

pub(crate) fn choose_http_method(ui: &mut egui::Ui, shown: &str) -> Option<&'static str> {
    let selected_color = ui.visuals().strong_text_color();
    let mut chosen = None;
    for method in HTTP_METHODS {
        let selected = shown == method;
        let mut label = egui::RichText::new(method);
        if selected {
            label = label.strong().color(selected_color);
        }
        if ui.selectable_label(selected, label).clicked() && !selected {
            chosen = Some(method);
        }
    }
    chosen
}

pub(crate) fn http_request(node: &NodeView) -> bool {
    node.definition == "org.orange.net/http-request"
}

pub(crate) fn http_request_variant(node: &NodeView) -> bool {
    matches!(
        node.definition.as_str(),
        "org.orange.net/http-request"
            | "org.orange.net/http-request-bytes"
            | "org.orange.net/http-request-json"
            | "org.orange.net/http-stream"
    )
}

pub(crate) fn input_ports(node: &NodeView) -> impl Iterator<Item = &PortView> {
    // A bridge's link port carries its binding, shown as the channel, not as an input.
    let inputs = node
        .ports
        .iter()
        .filter(|port| port.input && !port.exec && !palette::bridge_link_port(node, port));
    let first = |port: &&PortView| {
        node.definition == "org.orange.logging/log-message"
            && matches!(port.id.as_str(), "level" | "severity")
    };
    inputs
        .clone()
        .filter(first)
        .chain(inputs.filter(move |port| !first(port)))
}

pub(crate) fn input_caption(node: &NodeView, port: &PortView) -> String {
    if let Some(label) = specialized_port_caption(&node.definition, port) {
        return label.to_owned();
    }
    if node.definition == intrinsic::COALESCE {
        match port.id.as_str() {
            "value" => return t("optional-value-input").to_owned(),
            "fallback" => return t("if-missing-input").to_owned(),
            _ => {}
        }
    }
    log_part_label(node, &port.id)
        .or_else(|| port.label.clone())
        .or_else(|| input_label(node, &port.id).map(str::to_owned))
        .unwrap_or_else(|| port_label(&port.id))
}

pub(crate) fn specialized_port_caption(definition: &str, port: &PortView) -> Option<&'static str> {
    match (definition, port.input, port.id.as_str()) {
        (intrinsic::RISING_EDGE | intrinsic::FALLING_EDGE, true, "watch") => {
            Some(t("edge-condition"))
        }
        (intrinsic::CHANGE_BEYOND_TOLERANCE, true, "watch") => Some(t("port-value")),
        (intrinsic::INCREMENT_COUNTER | intrinsic::DECREMENT_COUNTER, true, "value") => {
            Some(t("counter-step"))
        }
        (intrinsic::INCREMENT_COUNTER | intrinsic::DECREMENT_COUNTER, false, "new") => {
            Some(t("counter-new-value"))
        }
        _ => None,
    }
}

pub(crate) fn log_part_label(node: &NodeView, id: &str) -> Option<String> {
    (node.definition == "org.orange.logging/log-message" && id.starts_with("argument#"))
        .then(|| {
            node.ports
                .iter()
                .filter(|port| port.id.starts_with("argument#"))
                .position(|port| port.id == id)
        })
        .flatten()
        .map(|index| tf("log-part", &[("number", (index + 1).into())]))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TextInput {
    Text,
    Message,
    LogMessage,
    Body,
    Headers,
    HeaderList,
    JsonPath,
}

impl TextInput {
    pub(crate) fn for_port(node: &NodeView, port: &PortView) -> Option<Self> {
        if node.paths.iter().any(|path| path.port == port.id)
            || is_folder_path(node, &port.id)
            || (http_request_variant(node) && port.id == "method")
            || (port.id == "text"
                && matches!(
                    node.definition.as_str(),
                    "org.orange.color/parse-hex" | "org.orange.time/parse-timestamp"
                ))
        {
            return None;
        }
        match port.id.as_str() {
            "content" if http_request(node) => Some(Self::Body),
            "path" | "content_type" if http_request_variant(node) => None,
            "message" if node.definition == "org.orange.logging/log-message" => {
                Some(Self::LogMessage)
            }
            id if node.definition == "org.orange.logging/log-message"
                && id.starts_with("argument#") =>
            {
                None
            }
            "headers" if http_request(node) => Some(Self::Headers),
            "headers" if http_request_variant(node) => Some(Self::HeaderList),
            "path"
                if node.definition == "org.orange.json/get-path"
                    || node.definition.starts_with("org.orange.json/read-") =>
            {
                Some(Self::JsonPath)
            }
            "text"
                if matches!(
                    node.definition.as_str(),
                    "org.orange.net/mqtt-publish"
                        | "org.orange.net/amqp-publish"
                        | "org.orange.net/ws-send"
                ) =>
            {
                Some(Self::Message)
            }
            _ if port.ty.as_deref() == Some("core/string@1") => Some(Self::Text),
            _ => None,
        }
    }

    pub(crate) fn hint(self) -> &'static str {
        t(match self {
            Self::Text => "text-input-hint",
            Self::Message => "message-input-hint",
            Self::LogMessage => "log-message-hint",
            Self::Body => "http-body-hint",
            Self::Headers | Self::HeaderList => "http-headers-hint",
            Self::JsonPath => "json-path-input-hint",
        })
    }

    pub(crate) fn literal(self, text: &str) -> String {
        if self == Self::HeaderList {
            let lines: Vec<_> = text
                .lines()
                .filter(|line| !line.trim().is_empty())
                .collect();
            json!({"type": "core/list@1<core/string@1>", "value": lines})
        } else {
            json!({"type": "core/string@1", "value": text})
        }
        .to_string()
    }
}

pub(crate) fn text_area_preview(text: &str, kind: TextInput) -> String {
    if kind == TextInput::HeaderList
        && let Ok(lines) = serde_json::from_str::<Vec<String>>(text)
    {
        return lines.join("\n");
    }
    serde_json::from_str::<String>(text).unwrap_or_else(|_| text.to_owned())
}

pub(crate) fn choice_caption<'a>(port: &PortView, name: &'a str) -> std::borrow::Cow<'a, str> {
    match port.ty.as_deref() {
        Some("org.orange.logging/severity@1") if name == "warning" => "WARN".into(),
        Some("org.orange.logging/level@1" | "org.orange.logging/severity@1") => {
            name.to_ascii_uppercase().into()
        }
        _ => name.into(),
    }
}

pub(crate) fn text_area_height(text: &str, width: f32, kind: TextInput) -> f32 {
    let chars_per_line = (width / 7.5).max(8.0) as usize;
    let lines = text
        .lines()
        .take(14)
        .map(|line| line.chars().count().div_ceil(chars_per_line).max(1))
        .sum::<usize>()
        .min(14);
    let minimum = if matches!(kind, TextInput::Text | TextInput::JsonPath) {
        60.0
    } else {
        82.0
    };
    (lines as f32 * 16.0 + 24.0).clamp(minimum, 248.0)
}

pub(crate) fn text_resize_grip(ui: &egui::Ui, rect: egui::Rect, id: egui::Id) -> f32 {
    let grip = ui
        .interact(rect, id, egui::Sense::DRAG)
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text(t("resize-text-area"));
    let color = if grip.hovered() || grip.dragged() {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().widgets.noninteractive.bg_stroke.color
    };
    ui.painter().hline(
        (rect.center().x - 12.0)..=(rect.center().x + 12.0),
        rect.center().y,
        egui::Stroke::new(2.0, color),
    );
    grip.drag_delta().y
}

/// Reads a literal or its definition default without the display preview's truncation.
pub(crate) fn stored_input_text(
    host: &Host,
    draft: &Draft,
    node: NodeId,
    name: &str,
) -> Option<String> {
    let document = draft.document();
    let node = document.node(node)?;
    let key: orange_host::PortKey = name.parse().ok()?;
    let encoded = if let Some(input) = node.inputs.get(&key) {
        serde_json::to_value(input).ok()?
    } else {
        let registry = host.registry_for(document);
        let contract = registry.contract(document, node).ok().flatten()?;
        let default = contract
            .ports
            .iter()
            .filter(|port| &port.port_id == key.port())
            .find_map(|port| port.default.as_ref())?;
        serde_json::to_value(default).ok()?
    };
    let value = encoded.as_object()?.get("value")?;
    match value {
        Json::String(text) => Some(text.clone()),
        Json::Array(lines) => Some(
            lines
                .iter()
                .filter_map(Json::as_str)
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    }
}

pub(crate) fn input_label(node: &NodeView, port: &str) -> Option<&'static str> {
    if http_request_variant(node) {
        return http_request_label(port);
    }
    (node.definition == "org.orange.logging/log-message" && matches!(port, "level" | "severity"))
        .then(|| t("log-level"))
}

pub(crate) fn input_color(port: &PortView) -> [u8; 4] {
    port.value
        .as_deref()
        .or(port.default.as_deref())
        .and_then(orange_host::parse_rgba_hex)
        .unwrap_or([0, 0, 0, 255])
}

pub(crate) fn pick_color(
    ui: &egui::Ui,
    response: &egui::Response,
    color: [u8; 4],
) -> Option<String> {
    let picked =
        orange_ui_controls::color_picker::popup(ui, response, color, true, color_picker_labels())?;
    Some(orange_host::rgba_hex(picked))
}

pub(crate) fn color_picker_labels() -> orange_ui_controls::color_picker::Labels<'static> {
    orange_ui_controls::color_picker::Labels {
        channels: [
            t("color-red"),
            t("color-green"),
            t("color-blue"),
            t("color-alpha"),
        ],
        hue: t("color-hue"),
        saturation_value: t("color-saturation-value"),
        recent: t("color-recent"),
        value: t("color-value"),
        invalid_value: t("color-value-invalid"),
        apply: t("color-apply"),
        cancel: t("cancel"),
    }
}

pub(crate) fn http_request_label(port: &str) -> Option<&'static str> {
    Some(t(match port {
        "method" => "http-method",
        "content" => "http-body",
        "path" => "http-path",
        "content_type" => "http-content-type",
        "headers" => "http-headers",
        "header_fields" => "http-typed-headers",
        "query" => "http-query-parameters",
        _ => return None,
    }))
}

/// Types offered for a value, besides those the document already uses.
pub const COMMON_TYPES: [&str; 6] = [
    "core/string@1",
    "core/i64@1",
    "core/f64@1",
    "core/bool@1",
    "core/bytes@1",
    "core/duration@1",
];

/// Nanoseconds in a millisecond: lengths of time are typed in milliseconds.
const NANOS_PER_MILLI: f64 = 1_000_000.0;

/// The range a slider or dial setting offers and how it spreads it, from the node's
/// control hint (ADR-33). The provider schema still owns the valid range.
#[derive(Clone, Copy)]
pub(crate) struct NumberRange {
    min: f64,
    max: f64,
    logarithmic: bool,
    /// The slider's notch style, when the hint names one.
    notch: Option<orange_ui_controls::NotchStyle>,
}

/// The hint of setting `name`, when it asks for `widget`.
pub(crate) fn setting_hint<'a>(
    node: &'a NodeView,
    name: &str,
    widget: Widget,
) -> Option<&'a Control> {
    node.controls
        .settings
        .get(name)
        .filter(|control| control.widget == widget)
}

/// The slider or dial hint of a numeric input, when it gives the range to offer:
/// an input has no schema bounds of its own (ADR-33).
pub(crate) fn input_hint<'a>(node: &'a NodeView, port: &PortView) -> Option<&'a Control> {
    let numeric = matches!(
        port.ty.as_deref(),
        Some(
            "core/f64@1" | "core/f32@1" | "core/i64@1" | "core/u64@1" | "core/i32@1" | "core/u32@1"
        )
    );
    node.controls.inputs.get(&port.id).filter(|control| {
        numeric
            && matches!(control.widget, Widget::Slider | Widget::Dial)
            && NumberRange::of_input(control).is_some()
    })
}

fn input_number_text(port: &PortView, value: f64) -> String {
    if matches!(
        port.ty.as_deref(),
        Some("core/i64@1" | "core/u64@1" | "core/i32@1" | "core/u32@1")
    ) {
        value.round().to_string()
    } else {
        value.to_string()
    }
}

pub(crate) fn advanced_input(node: &NodeView, port: &PortView) -> bool {
    node.controls
        .region_of(&port.id)
        .is_some_and(|(region, _)| region == Region::Advanced)
}

pub(crate) fn field_order(node: &NodeView, name: &str) -> (u8, usize, usize) {
    node.controls
        .regions
        .iter()
        .enumerate()
        .find_map(|(index, placement)| {
            let field = placement.fields.iter().position(|field| field == name)?;
            let region = match placement.region {
                Region::Header => 0,
                Region::Aside => 1,
                Region::Body => 2,
                Region::Advanced => 3,
            };
            Some((region, index, field))
        })
        .unwrap_or((2, usize::MAX, 0))
}

fn knob_style(control: &Control) -> orange_ui_controls::KnobStyle {
    use orange_host::controls::Knob;
    use orange_ui_controls::KnobStyle;
    match control.knob {
        None | Some(Knob::Arc) => KnobStyle::Arc,
        Some(Knob::Bipolar) => KnobStyle::Bipolar,
        Some(Knob::Notched) => KnobStyle::Notched,
        Some(Knob::Loop) => KnobStyle::Loop,
    }
}

fn notch_style(control: &Control) -> Option<orange_ui_controls::NotchStyle> {
    use orange_host::controls::Notch;
    control.notch.map(|notch| match notch {
        Notch::Pill => orange_ui_controls::NotchStyle::Pill,
        Notch::Tick => orange_ui_controls::NotchStyle::Tick,
        Notch::Dot => orange_ui_controls::NotchStyle::Dot,
    })
}

/// A number as the hint shows it: with its decimals, when the hint gives them.
pub(crate) fn hinted_number(control: Option<&Control>, value: f64) -> String {
    match control.and_then(|control| control.precision) {
        Some(digits) => format!("{value:.*}", usize::from(digits)),
        None => value.to_string(),
    }
}

/// The next whole value a stepper can offer within its declared limits.
pub(crate) fn stepper_value(control: &Control, schema: &Json, value: f64, sign: i8) -> Option<f64> {
    let [low, high] = control.range.unwrap_or_else(|| {
        [
            schema["minimum"].as_f64().unwrap_or(f64::MIN),
            schema["maximum"].as_f64().unwrap_or(f64::MAX),
        ]
    });
    let (low, high) = (low.ceil(), high.floor());
    let step = control.step.unwrap_or(1.0);
    if !low.is_finite()
        || !high.is_finite()
        || low > high
        || !step.is_finite()
        || step <= 0.0
        || !value.is_finite()
    {
        return None;
    }
    Some((value + f64::from(sign) * step).round().clamp(low, high))
}

/// The dial hint of setting `name`, when the node's definition asks for a dial.
pub(crate) fn dial_of<'a>(node: &'a NodeView, name: &str) -> Option<&'a Control> {
    node.controls
        .settings
        .get(name)
        .filter(|control| control.widget == Widget::Dial)
        .filter(|_| {
            node.schema
                .as_ref()
                .and_then(|schema| NumberRange::of(node, name, &schema["properties"][name]))
                .is_some()
        })
}

/// The step a dial moves by and the unit it shows.
#[expect(clippy::cast_possible_truncation, reason = "a display step")]
fn dial_face(dial: &Control) -> (Option<f32>, &str) {
    (
        dial.step.map(|step| step as f32),
        dial.unit.as_deref().unwrap_or_default(),
    )
}

#[derive(Clone, Copy)]
pub(crate) struct ThresholdEdit<'a> {
    control: Option<&'a Control>,
    node: NodeId,
    saved: f64,
    range: NumberRange,
}

#[derive(Clone, Copy)]
struct ThresholdPreview {
    owner: egui::Id,
    saved: f64,
    value: f64,
    frame: u64,
}

impl<'a> ThresholdEdit<'a> {
    /// The setting this editor shapes: the threshold of a noise node, in decibels.
    pub(crate) const SETTING: &'static str = "threshold_db";

    pub fn for_node(node: &'a NodeView) -> Option<Self> {
        if !canvas::spectrum::is_noise(node) {
            return None;
        }
        let schema = &node.schema.as_ref()?["properties"][Self::SETTING];
        if schema["type"] != "number" || schema["readOnly"] == true {
            return None;
        }
        let range = NumberRange::of(node, Self::SETTING, schema)?;
        let saved = node.settings[Self::SETTING]
            .as_f64()
            .or_else(|| schema["default"].as_f64())?;
        Some(Self {
            node: node.id,
            saved,
            range,
            control: node.controls.settings.get(Self::SETTING),
        })
    }

    fn key(self) -> egui::Id {
        egui::Id::unique(("orange-threshold-preview", self.node))
    }

    fn preview(self, ctx: &egui::Context) -> Option<ThresholdPreview> {
        let frame = ctx.cumulative_frame_nr();
        ctx.data(|data| data.get_temp::<ThresholdPreview>(self.key()))
            .filter(|draft| draft.saved == self.saved && frame.saturating_sub(draft.frame) <= 1)
    }

    pub fn shown(self, ctx: &egui::Context) -> f64 {
        self.preview(ctx).map_or(self.saved, |draft| draft.value)
    }

    pub fn update(self, response: &egui::Response, value: f64) -> Option<f64> {
        if !value.is_finite() {
            return None;
        }
        let ctx = &response.ctx;
        let owner = response.id;
        let value = value.clamp(self.range.min, self.range.max);
        if response.dragged() {
            if self.shown(ctx) != value {
                ctx.request_repaint();
            }
            let frame = ctx.cumulative_frame_nr();
            ctx.data_mut(|data| {
                data.insert_temp(
                    self.key(),
                    ThresholdPreview {
                        owner,
                        saved: self.saved,
                        value,
                        frame,
                    },
                )
            });
            return None;
        }
        let owns_preview = self.preview(ctx).is_some_and(|draft| draft.owner == owner);
        let commit = response.changed() || (owns_preview && response.drag_stopped());
        if commit || owns_preview {
            ctx.data_mut(|data| data.remove::<ThresholdPreview>(self.key()));
        }
        (commit && self.saved != value).then_some(value)
    }

    pub fn slider(self, ui: &mut egui::Ui, surface: Surface) -> Option<f64> {
        let mut fraction = self.range.fraction(self.shown(ui.ctx()));
        let defaults = self.range.marks();
        let marks = self.range.slider_marks(self.control, &defaults);
        let response = orange_ui_controls::marked_slider(ui, surface, &mut fraction, &marks);
        let value = (self.range.value(fraction) * 10.0).round() / 10.0;
        response.widget_info(|| {
            egui::WidgetInfo::slider(response.enabled(), value, t("audio-setting-threshold-db"))
        });
        self.update(&response, value)
    }

    pub fn number(self, ui: &mut egui::Ui) -> Option<f64> {
        let mut value = self.shown(ui.ctx());
        let response = ui
            .add(
                egui::DragValue::new(&mut value)
                    .range(self.range.min..=self.range.max)
                    .speed(0.25)
                    .max_decimals(3)
                    .update_while_editing(false)
                    .suffix(" dBFS"),
            )
            .accessible_name(t("audio-setting-threshold-db"));
        self.update(&response, value)
    }
}

impl NumberRange {
    /// The range of a setting whose definition asks for a slider or a dial: the hint's
    /// own range, else the schema's bounds.
    pub(crate) fn of(node: &NodeView, name: &str, schema: &Json) -> Option<Self> {
        let control = node.controls.settings.get(name)?;
        if !matches!(control.widget, Widget::Slider | Widget::Dial)
            || !matches!(schema["type"].as_str(), Some("number" | "integer"))
        {
            return None;
        }
        let [min, max] = control
            .range
            .or_else(|| Some([schema["minimum"].as_f64()?, schema["maximum"].as_f64()?]))?;
        Self::from_bounds(control, min, max)
    }

    /// The range a hinted input offers ([`input_hint`]).
    pub(crate) fn of_input(control: &Control) -> Option<Self> {
        let [min, max] = control.range?;
        Self::from_bounds(control, min, max)
    }

    fn from_bounds(control: &Control, min: f64, max: f64) -> Option<Self> {
        if !(min.is_finite() && max.is_finite() && min < max)
            || (control.scale == Scale::Log && min <= 0.0)
        {
            return None;
        }
        if control.widget == Widget::Dial && !Self::dial_bounds_fit(min, max) {
            return None;
        }
        Some(Self {
            min,
            max,
            logarithmic: control.scale == Scale::Log && min.ln() < max.ln(),
            notch: notch_style(control),
        })
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "the rotary control uses f32 bounds"
    )]
    fn dial_bounds_fit(min: f64, max: f64) -> bool {
        let (min, max) = (min as f32, max as f32);
        max > min && (max - min).is_finite()
    }

    fn style(self) -> orange_ui_controls::SliderStyle {
        orange_ui_controls::SliderStyle {
            notch: self.notch.unwrap_or_default(),
            ..Default::default()
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "normalized screen coordinate"
    )]
    fn fraction(self, value: f64) -> f32 {
        if value.is_nan() || value <= self.min {
            return 0.0;
        }
        if value >= self.max {
            return 1.0;
        }
        let span = self.max - self.min;
        let fraction = if self.logarithmic {
            (value.ln() - self.min.ln()) / (self.max.ln() - self.min.ln())
        } else if span.is_finite() {
            (value - self.min) / span
        } else {
            (value * 0.5 - self.min * 0.5) / (self.max * 0.5 - self.min * 0.5)
        };
        fraction.clamp(0.0, 1.0) as f32
    }

    fn value(self, fraction: f32) -> f64 {
        if fraction.is_nan() || fraction <= 0.0 {
            self.min
        } else if fraction >= 1.0 {
            self.max
        } else {
            let fraction = f64::from(fraction);
            let span = self.max - self.min;
            let value = if self.logarithmic {
                let min = self.min.ln();
                (min + fraction * (self.max.ln() - min)).exp()
            } else if span.is_finite() {
                self.min + fraction * span
            } else {
                (1.0 - fraction) * self.min + fraction * self.max
            };
            value.clamp(self.min, self.max)
        }
    }

    pub(crate) fn slider(
        self,
        ui: &mut egui::Ui,
        surface: Surface,
        id: egui::Id,
        value: f64,
        label: &str,
        control: Option<&Control>,
    ) -> Option<f64> {
        let id = id.with("audio-drag");
        let mut fraction = ui
            .data(|data| data.get_temp::<f32>(id))
            .unwrap_or_else(|| self.fraction(value));
        let defaults = self.marks();
        let marks = self.slider_marks(control, &defaults);
        let response = orange_ui_controls::SliderStyle::scope(ui, self.style(), |ui| {
            orange_ui_controls::marked_slider(ui, surface, &mut fraction, &marks)
        });
        let value = self.value(fraction);
        let value = control
            .and_then(|control| control.step)
            .map_or(value, |step| {
                if fraction == 0.0 || fraction == 1.0 {
                    value
                } else {
                    let snapped = self.min + ((value - self.min) / step).round() * step;
                    if snapped.is_finite() {
                        snapped.clamp(self.min, self.max)
                    } else {
                        value
                    }
                }
            });
        response.widget_info(|| egui::WidgetInfo::slider(response.enabled(), value, label));
        if response.dragged() {
            ui.data_mut(|data| data.insert_temp(id, fraction));
            return None;
        }
        ui.data_mut(|data| data.remove::<f32>(id));
        (response.changed() || response.drag_stopped()).then_some(value)
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "bounded audio controls use f32 for painting and retain the typed editor"
    )]
    /// A dial for the setting. `caption` is shown inside it; leave it empty where a
    /// label already sits beside the dial.
    #[allow(clippy::too_many_arguments, reason = "one dial's state and face")]
    pub(crate) fn dial(
        self,
        ui: &mut egui::Ui,
        surface: Surface,
        id: egui::Id,
        value: f64,
        dial: &Control,
        label: &str,
        caption: &str,
        diameter: Option<f32>,
    ) -> Option<f64> {
        let (step, unit) = dial_face(dial);
        let key = id.with("rotary-draft");
        let shown_value = if value.is_nan() {
            self.min
        } else {
            value.clamp(self.min, self.max)
        } as f32;
        let mut shown = ui.data(|d| d.get_temp::<f32>(key)).unwrap_or(shown_value);
        let response = ui
            .push_id(id, |ui| {
                orange_ui_controls::styled_dial(
                    ui,
                    surface,
                    &mut shown,
                    self.min as f32..=self.max as f32,
                    step,
                    orange_ui_controls::DialFace {
                        caption: dial.caption.as_ref().map_or(caption, |text| &text.fallback),
                        unit,
                        diameter,
                    },
                    orange_ui_controls::DialStyle {
                        knob: knob_style(dial),
                        precision: dial.precision,
                    },
                )
            })
            .inner;
        response
            .widget_info(|| egui::WidgetInfo::slider(response.enabled(), f64::from(shown), label));
        if response.dragged() {
            ui.data_mut(|d| d.insert_temp(key, shown));
            return None;
        }
        ui.data_mut(|d| d.remove::<f32>(key));
        (shown.is_finite()
            && shown != shown_value
            && (response.changed() || response.drag_stopped()))
        .then(|| f64::from(shown).clamp(self.min, self.max))
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "bounded audio display values"
    )]
    pub(crate) fn paint_dial(
        self,
        ui: &egui::Ui,
        rect: egui::Rect,
        value: f64,
        dial: &Control,
        caption: &str,
    ) {
        let (step, unit) = dial_face(dial);
        orange_ui_controls::styled_dial_preview(
            ui,
            rect,
            Surface::Card,
            value as f32,
            self.min as f32..=self.max as f32,
            step,
            orange_ui_controls::DialFace {
                caption: dial.caption.as_ref().map_or(caption, |text| &text.fallback),
                unit,
                diameter: None,
            },
            orange_ui_controls::DialStyle {
                knob: knob_style(dial),
                precision: dial.precision,
            },
        );
    }

    pub(crate) fn paint_slider(
        self,
        ui: &egui::Ui,
        rect: egui::Rect,
        value: f64,
        control: Option<&Control>,
    ) {
        let defaults = self.marks();
        let marks = self.slider_marks(control, &defaults);
        orange_ui_controls::SliderStyle::apply(ui.ctx(), self.style(), || {
            orange_ui_controls::marked_slider_preview(
                ui,
                rect,
                Surface::Card,
                self.fraction(value),
                &marks,
            );
        });
    }

    fn slider_marks<'a>(
        self,
        control: Option<&Control>,
        fallback: &'a [f32],
    ) -> std::borrow::Cow<'a, [f32]> {
        let Some(control) = control.filter(|control| !control.marks.is_empty()) else {
            return fallback.into();
        };
        control
            .marks
            .iter()
            .copied()
            .filter(|at| (self.min..=self.max).contains(at))
            .map(|at| self.fraction(at))
            .collect::<Vec<_>>()
            .into()
    }

    fn marks(self) -> [f32; 5] {
        if self.logarithmic {
            [20.0, 100.0, 1_000.0, 10_000.0, 20_000.0].map(|at| self.fraction(at))
        } else if self.min < 0.0 && self.max > 0.0 {
            [0.0, 0.25, self.fraction(0.0), 0.75, 1.0]
        } else {
            [0.0, 0.25, 0.5, 0.75, 1.0]
        }
    }
}

pub(crate) fn audio_setting_label(node: &NodeView, setting: &str) -> Option<&'static str> {
    if node.definition == "org.orange.audio/noise-level" {
        return Some(t(match setting {
            ThresholdEdit::SETTING => "audio-setting-threshold-db",
            "reset_db" => "audio-reset-margin",
            "hold_ms" => "audio-setting-hold-ms",
            "direction" => "when-mode",
            _ => return None,
        }));
    }
    if node.definition != "org.orange.audio/noise-gate" {
        return None;
    }
    let key = match setting {
        ThresholdEdit::SETTING => "audio-setting-threshold-db",
        "reset_db" => "noise-gate-reset",
        "attack_ms" => "noise-gate-attack",
        "hold_ms" => "audio-setting-hold-ms",
        "release_ms" => "noise-gate-release",
        _ => return None,
    };
    Some(t(key))
}

pub(crate) fn audio_trigger_help(node: &NodeView, name: &str) -> Option<&'static str> {
    if node.definition != "org.orange.audio/noise-level" {
        return None;
    }
    Some(t(match name {
        "hold_ms" => "audio-trigger-hold-help",
        "reset_db" => "audio-trigger-reset-help",
        _ => return None,
    }))
}

/// Sets the value at a JSON `pointer` (such as `/variable`) in `settings`, making
/// objects on the way; `None` removes it.
pub fn set_at(settings: &mut Json, pointer: &str, value: Option<Json>) {
    let mut keys: Vec<String> = pointer
        .split('/')
        .skip(1)
        .map(|key| key.replace("~1", "/").replace("~0", "~"))
        .collect();
    let Some(last) = keys.pop() else {
        return;
    };
    let mut at = settings;
    for key in keys {
        if !at[&key].is_object() {
            if value.is_none() {
                return;
            }
            at[&key] = json!({});
        }
        at = &mut at[&key];
    }
    match (at.as_object_mut(), value) {
        (Some(fields), Some(value)) => {
            let _ = fields.insert(last, value);
        }
        (Some(fields), None) => {
            let _ = fields.remove(&last);
        }
        (None, _) => {}
    }
}

/// How one setting is typed in, from its schema.
enum Field<'a> {
    Choice(Vec<&'a str>),
    Toggle,
    Whole,
    Number,
    WatchNumber {
        whole: bool,
    },
    /// A length of time, typed in milliseconds and kept as nanoseconds in digits.
    Time,
    Json,
    Text,
}

impl<'a> Field<'a> {
    fn of(schema: &'a Json) -> Self {
        if let Some(options) = schema["enum"].as_array() {
            return Self::Choice(options.iter().filter_map(Json::as_str).collect());
        }
        match (schema["type"].as_str(), schema["format"].as_str()) {
            (Some("boolean"), _) => Self::Toggle,
            (Some("integer"), _) => Self::Whole,
            (Some("number"), _) => Self::Number,
            (Some("string"), Some("duration-ns")) => Self::Time,
            (Some("object" | "array"), _) => Self::Json,
            _ => Self::Text,
        }
    }

    fn for_node(node: &NodeView, name: &str, schema: &'a Json) -> Self {
        if intrinsic::is_watch(&node.definition) && name != "mode" {
            return Self::WatchNumber {
                whole: matches!(name, "hold" | "cooldown"),
            };
        }
        Self::of(schema)
    }

    /// The setting as it is typed in.
    fn typed(&self, value: &Json) -> String {
        match (self, value) {
            (_, Json::Null) => String::new(),
            (Self::Time, Json::String(nanos)) => nanos
                .parse::<f64>()
                .map_or_else(|_| nanos.clone(), |n| (n / NANOS_PER_MILLI).to_string()),
            (_, Json::String(text)) => text.clone(),
            (_, other) => other.to_string(),
        }
    }

    /// The setting from what is typed, or why it cannot be.
    fn value(&self, typed: &str) -> Result<Json, String> {
        let typed = typed.trim();
        match self {
            Self::Whole => typed
                .parse::<i64>()
                .map(Json::from)
                .map_err(|_| tf("not-whole-number", &[("typed", typed.into())])),
            Self::Number => typed
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(Json::Number)
                .ok_or_else(|| tf("not-a-number", &[("typed", typed.into())])),
            Self::WatchNumber { whole } => watch_number_value(typed, *whole),
            Self::Time => typed
                .parse::<f64>()
                .ok()
                .filter(|millis| *millis >= 0.0)
                .map(|millis| (millis * NANOS_PER_MILLI).round())
                // 2^64 is exactly representable as f64; u64::MAX is not.
                .filter(|nanos| (0.0..18_446_744_073_709_551_616.0).contains(nanos))
                .map(|nanos| {
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "a non-negative length of time well inside u64"
                    )]
                    let nanos = nanos as u64;
                    json!(nanos.to_string())
                })
                .ok_or_else(|| tf("not-milliseconds", &[("typed", typed.into())])),
            Self::Json => {
                serde_json::from_str(typed).map_err(|_| t("invalid-setting-json").to_owned())
            }
            Self::Choice(_) | Self::Toggle | Self::Text => Ok(json!(typed)),
        }
    }
}

/// The key of a setting among the typed fields, apart from the inputs'.
fn setting_key(pointer: &str) -> String {
    format!("@{pointer}")
}

pub(crate) fn watch_fields(mode: Option<&str>) -> &'static [&'static str] {
    match mode {
        Some("deadband") => &["mode", "threshold"],
        Some("above" | "below") => &["mode", "threshold", "release", "hold", "cooldown"],
        Some("inside" | "outside") => &["mode", "low", "high", "hold", "cooldown"],
        _ => &["mode"],
    }
}

pub(crate) fn watch_number_value(typed: &str, whole: bool) -> Result<Json, String> {
    serde_json::from_str::<Json>(typed.trim())
        .ok()
        .filter(|value| {
            if whole {
                value.as_u64().is_some()
            } else {
                value.is_number()
            }
        })
        .ok_or_else(|| {
            tf(
                if whole {
                    "not-whole-number"
                } else {
                    "not-a-number"
                },
                &[("typed", typed.into())],
            )
        })
}

pub(crate) fn watch_field_entries(node: &NodeView) -> impl Iterator<Item = (&String, &Json)> {
    let properties = node
        .schema
        .as_ref()
        .and_then(|schema| schema["properties"].as_object());
    let fields: &[&str] = match node.definition.as_str() {
        intrinsic::ON_WATCH => watch_fields(node.settings["mode"].as_str()),
        intrinsic::CHANGE_BEYOND_TOLERANCE => &["threshold"],
        _ => &[],
    };
    fields
        .iter()
        .filter_map(move |name| properties?.get_key_value(*name))
}

pub(crate) fn watch_field_label(node: &NodeView, name: &str) -> &'static str {
    if node.definition == intrinsic::CHANGE_BEYOND_TOLERANCE && name == "threshold" {
        t("change-tolerance")
    } else {
        watch_label(name)
    }
}

pub(crate) fn watch_label(name: &str) -> &'static str {
    match name {
        "changed" => t("when-changed"),
        "rising" => t("when-rising"),
        "falling" => t("when-falling"),
        "above" => t("when-above"),
        "below" => t("when-below"),
        "inside" => t("when-inside"),
        "outside" => t("when-outside"),
        "deadband" => t("when-deadband"),
        "threshold" => t("when-threshold"),
        "release" => t("when-release"),
        "low" => t("when-low"),
        "high" => t("when-high"),
        "hold" => t("when-hold"),
        "cooldown" => t("when-cooldown"),
        _ => t("when-mode"),
    }
}

pub(crate) fn folder_path_label(node: &NodeView, label: String, folder: Option<&str>) -> String {
    match folder.filter(|_| node.definition.starts_with("org.orange.files/")) {
        Some(folder) => tf(
            "folder-path-label",
            &[("port", label.into()), ("folder", folder.into())],
        ),
        None => label,
    }
}

pub(crate) fn is_folder_path(node: &NodeView, port: &str) -> bool {
    node.definition.starts_with("org.orange.files/")
        && (matches!(port, "path" | "to") || node.paths.iter().any(|path| path.port == port))
}

pub(crate) fn connection_named<'a>(
    node: &NodeView,
    connections: &'a [Connection],
) -> Option<&'a Connection> {
    let id = node
        .references
        .iter()
        .filter(|(_, kind)| *kind == ReferenceKind::Connection)
        .find_map(|(pointer, _)| node.settings.pointer(pointer)?.as_str())?;
    connections
        .iter()
        .find(|connection| connection.id.as_deref() == Some(id))
}

impl Editor {
    /// The configured root for a folder-scoped file node's path picker.
    pub fn file_picker_root(&self, node: NodeId) -> Option<Option<String>> {
        self.file_picker_folder(node).map(|folder| {
            folder.and_then(|connection| connection.settings["path"].as_str().map(str::to_owned))
        })
    }

    /// The folder connection referenced by a file node. The outer option identifies
    /// folder-scoped file nodes, including drafts that need a connection.
    pub fn file_picker_folder(&self, node: NodeId) -> Option<Option<&Connection>> {
        let node = self.graph()?.nodes.iter().find(|view| view.id == node)?;
        if !node.definition.starts_with("org.orange.files/") {
            return None;
        }
        Some(self.named_connection(node))
    }

    /// Changes one setting of a node, keeping the others: `value` at the JSON
    /// `pointer`, or the setting taken off with `None`.
    pub fn set_setting(
        &mut self,
        host: &mut Host,
        node: NodeId,
        pointer: &str,
        value: Option<Json>,
    ) -> bool {
        let found = self
            .graph()
            .and_then(|graph| graph.nodes.iter().find(|found| found.id == node));
        let watch = found.is_some_and(|found| found.definition == intrinsic::ON_WATCH);
        let mut settings = found
            .map(|found| found.settings.clone())
            .filter(Json::is_object)
            .unwrap_or_else(|| json!({}));
        let value = if watch {
            value.filter(|value| !value.is_null())
        } else {
            value
        };
        set_at(&mut settings, pointer, value);
        if watch
            && pointer == "/mode"
            && let Some(settings) = settings.as_object_mut()
        {
            let fields = watch_fields(settings.get("mode").and_then(Json::as_str));
            settings.retain(|field, _| fields.contains(&field.as_str()));
            for (field, default) in [("threshold", 0), ("low", 0), ("high", 1)] {
                if fields.contains(&field) {
                    settings.entry(field).or_insert_with(|| json!(default));
                }
            }
        }
        let Some(draft) = self.editing() else {
            return false;
        };
        let result = host.set_config(draft, node, &settings);
        self.edited(host, result)
    }

    /// Chooses the type of a generic node's parameter; `None` lets the wires decide.
    pub fn choose_type(
        &mut self,
        host: &mut Host,
        node: NodeId,
        param: &str,
        ty: Option<&str>,
    ) -> bool {
        let Some(draft) = self.editing() else {
            return false;
        };
        let result = host.choose_type(draft, node, param, ty);
        self.edited(host, result)
    }

    /// The declared variables for canvas and Properties pickers.
    pub(crate) fn variable_choices(&self) -> Vec<declarations::VariableChoice> {
        self.declarations()
            .into_iter()
            .flat_map(|declared| &declared.variables)
            .filter_map(|variable| {
                let label = format!("{} ({})", variable.name, type_name(&variable.ty));
                Some(declarations::VariableChoice {
                    id: variable.id.clone()?,
                    label,
                    scope: variable.scope,
                    ty: variable.ty.clone(),
                })
            })
            .collect()
    }

    /// The declared connection `node` names, if it names one.
    fn named_connection(&self, node: &NodeView) -> Option<&Connection> {
        connection_named(node, &self.declarations()?.connections)
    }

    pub(crate) fn node_config_window(&mut self, ctx: &egui::Context, host: &mut Host) {
        let Some((node, position)) = self.node_config_menu else {
            return;
        };
        if self.canvas.selected() != Some(node) {
            self.node_config_menu = None;
            return;
        }
        let Some(view) = self
            .graph()
            .and_then(|graph| graph.nodes.iter().find(|view| view.id == node))
        else {
            self.node_config_menu = None;
            return;
        };
        let label = host
            .connection_label(&view.definition)
            .unwrap_or_else(|| t("configure-node").to_owned());
        let mut open = true;
        orange_ui_controls::dialog_window(ctx, label)
            .id(egui::Id::unique("node-config").with(node))
            .default_pos(position)
            .default_width(330.0)
            .collapsible(false)
            .resizable(true)
            .open(&mut open)
            .show(ctx, |ui| {
                self.properties_panel(ui, host);
            });
        if !open
            || self.canvas.selected() != Some(node)
            || ctx.input(|input| input.key_pressed(egui::Key::Escape))
        {
            self.node_config_menu = None;
        }
    }

    /// Reads the fields of `node` again when another node is selected.
    fn inspect(&mut self, node: &NodeView, host: &Host) {
        if self.inspected == Some(node.id) {
            return;
        }
        self.inspected = Some(node.id);
        self.typed = node
            .ports
            .iter()
            .filter(|port| port.input && !port.exec)
            .map(|port| {
                let shown = port.value.clone().unwrap_or_default();
                // Text is shown in quotes on a card and typed without them.
                let typed = serde_json::from_str::<String>(&shown).unwrap_or(shown);
                (port.id.clone(), typed)
            })
            .collect();
        if let Some(document) = &self.document {
            for port in node.ports.iter().filter(|port| {
                port.input
                    && TextInput::for_port(node, port).is_some_and(|kind| kind != TextInput::Text)
            }) {
                let full =
                    stored_input_text(host, &document.draft, node.id, &port.id).unwrap_or_default();
                self.typed.insert(port.id.clone(), full);
            }
        }
        if let Some(properties) = node
            .schema
            .as_ref()
            .and_then(|schema| schema["properties"].as_object())
        {
            for (name, schema) in properties {
                let pointer = format!("/{name}");
                let typed =
                    Field::for_node(node, name, schema).typed(&node.settings[name.as_str()]);
                let _ = self.typed.insert(setting_key(&pointer), typed);
            }
        }
    }

    /// The value on an input as its field shows it before typing: text without quotes.
    pub(crate) fn shown_input(port: &PortView) -> String {
        let shown = port.value.clone().unwrap_or_default();
        serde_json::from_str::<String>(&shown).unwrap_or(shown)
    }

    fn properties_title(&mut self, ui: &mut egui::Ui, host: &mut Host, node: &NodeView) {
        let font = egui::TextStyle::Heading;
        let height = ui
            .text_style_height(&font)
            .max(ui.spacing().interact_size.y);
        let (row, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), height),
            egui::Sense::hover(),
        );
        let mut title_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("properties-title")
                .max_rect(row)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let ui = &mut title_ui;
        let ink = canvas::node_title_color(ui.visuals());
        if let Some(rename) = &mut self.renaming_node
            && rename.node == node.id
        {
            if let Some(save) = rename.field(ui, font) {
                let name = std::mem::take(&mut rename.name);
                self.renaming_node = None;
                if save {
                    self.rename_node(host, node.id, &name);
                }
            }
            return;
        }
        self.renaming_node = None;
        ui.add_space(3.0);
        let title = ui
            .add(
                egui::Label::new(
                    egui::RichText::new(self.canvas.display_name(node))
                        .heading()
                        .color(ink),
                )
                .truncate()
                .sense(egui::Sense::click()),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(format!("{}\n{}", node.name, t("rename-node-double-click")));
        if title.double_clicked() {
            self.begin_node_rename(node.id);
        }
    }

    pub(crate) fn properties_panel(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        // What the panel holds never widens it; only dragging its edge does. Text is cut
        // short with an ellipsis and shown whole on hover.
        ui.set_max_width(ui.available_width());
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 5.0);
        let visuals = ui.visuals_mut();
        visuals.widgets.inactive.weak_bg_fill = visuals.extreme_bg_color;
        let selected = self.canvas.selected().and_then(|id| {
            self.graph()?
                .nodes
                .iter()
                .find(|node| node.id == id)
                .cloned()
        });
        let Some(node) = selected else {
            self.inspected = None;
            self.renaming_node = None;
            ui.add_space(4.0);
            let many = self.canvas.selection();
            if many.len() > 1 {
                ui.strong(tf("nodes-selected", &[("count", many.len().into())]));
                ui.add(egui::Label::new(t("selection-copy-hint")).wrap());
                if ui
                    .button((icons::text(icons::FUNCTION), t("make-function-ellipsis")))
                    .clicked()
                {
                    self.preview_function(host);
                }
                if ui.button(t("delete-them")).clicked() {
                    let _ = self.remove_nodes(host, &many);
                }
            } else {
                ui.weak(t("select-node-hint"));
                ui.weak(t("multi-select-hint"));
            }
            return;
        };
        self.inspect(&node, host);
        ui.add_space(2.0);
        self.properties_title(ui, host, &node);
        let kind = self.canvas.kind_name(&node);
        let title = self.canvas.display_name(&node);
        let subtitle = match (title == kind, node.detail.as_deref()) {
            (true, detail) => detail.map(str::to_owned),
            (false, Some(detail)) => Some(format!("{kind} · {detail}")),
            (false, None) => Some(kind.to_owned()),
        };
        if let Some(subtitle) = subtitle {
            ui.add(egui::Label::new(egui::RichText::new(subtitle).weak()).truncate())
                .on_hover_text(node.definition.as_str());
        }
        if let Some(missing) = &node.missing {
            ui.add_space(4.0);
            let warning = ui.visuals().warn_fg_color;
            let _ = ui.add(
                egui::Label::new(egui::RichText::new(missing_package_text(missing)).color(warning))
                    .wrap(),
            );
            let _ = ui.add(egui::Label::new(t("missing-package-hint")).wrap());
            if self.file_dialogs
                && controls::button(
                    ui,
                    Surface::Panel,
                    controls::Action::Primary,
                    true,
                    egui::Button::new((icons::text(icons::ADD), t("install-missing-package"))),
                )
                .clicked()
            {
                self.ask(FileRequest::ImportPackage);
            }
        }
        if let Some(description) = self.canvas.description(&node.definition) {
            ui.add_space(4.0);
            let _ = ui.add(egui::Label::new(description).wrap());
        }
        // A variable write says what it does, as its card does.
        if let Some(caption) = node_ergonomics::operation_caption(&node) {
            ui.add_space(2.0);
            let _ = ui.add(egui::Label::new(egui::RichText::new(caption).monospace()).wrap());
        }
        if matches!(
            node.definition.as_str(),
            "org.orange.audio/noise-level" | "org.orange.audio/multiband-spectrum"
        ) {
            let has_reading = self.canvas.noise_readings.contains_key(&node.id)
                || self
                    .canvas
                    .spectra
                    .get(&node.id)
                    .is_some_and(|bins| bins.len() > 1);
            ui.weak(canvas::spectrum::audio_status(
                &node,
                self.canvas.audio_running,
                has_reading,
            ));
            if let Some(level) = self.canvas.noise_readings.get(&node.id) {
                ui.label(tf(
                    "audio-display-level",
                    &[("level", format!("{level:.1}").into())],
                ));
            }
            if let Some(graph) = self.graph()
                && let Some(wire) = graph
                    .wires
                    .iter()
                    .find(|wire| wire.to.0 == node.id && wire.to.1 == "stream")
                && let Some(source) = graph.nodes.iter().find(|source| source.id == wire.from.0)
            {
                let port = source
                    .ports
                    .iter()
                    .find(|port| !port.input && port.id == wire.from.1);
                let label = port
                    .and_then(|port| port.label.as_deref())
                    .map_or_else(|| port_label(&wire.from.1), str::to_owned);
                let from = tf(
                    "audio-display-source",
                    &[
                        ("node", self.canvas.display_name(source).into()),
                        ("port", label.into()),
                    ],
                );
                ui.add(egui::Label::new(&from).truncate())
                    .on_hover_text(&from);
            }
        }
        ui.add_space(6.0);
        // What to do once the fields are drawn: one change a frame.
        let mut change = None;
        // Delete stays in the panel's bottom right corner while the fields scroll.
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Max), |ui| {
            ui.add_space(4.0);
            if controls::button(
                ui,
                Surface::Panel,
                controls::Action::Destructive,
                true,
                egui::Button::new(t("delete-node")),
            )
            .clicked()
            {
                change = Some(Change::Delete);
            }
            ui.separator();
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let http = http_request_variant(&node);
                        if http {
                            self.values_section(ui, &node);
                        }
                        if http
                            && self.named_connection(&node).is_none_or(|connection| {
                                connection.scope == orange_host::ConnectionScope::Graph
                                    && connection.settings["base_url"]
                                        .as_str()
                                        .is_none_or(|url| url.trim().is_empty())
                            })
                        {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(t("http-profile-needs-setup")).weak(),
                                )
                                .wrap(),
                            );
                        }
                        if let Some(connection) = self.named_connection(&node)
                            && ui
                                .button(t("edit-connection"))
                                .on_hover_text(&connection.name)
                                .clicked()
                        {
                            let connection = connection.clone();
                            self.forms.edit_connection(&connection);
                            self.probing = None;
                        }
                        if let Some(adapter) = self
                            .named_connection(&node)
                            .map(|connection| connection.adapter.clone())
                            .or_else(|| http.then(|| declarations::network::HTTP.to_owned()))
                            && ui.button(t("add-connection")).clicked()
                        {
                            self.forms.new_connection_for(node.id, &adapter);
                            self.probing = None;
                        }
                        if self.canvas.has_failure_routes(node.id) {
                            let label = if self.canvas.failure_routes_expanded(node.id) {
                                t("hide-failure-routes")
                            } else {
                                t("show-failure-routes")
                            };
                            if ui
                                .button(label)
                                .on_hover_text(t("failure-routes-hint"))
                                .clicked()
                            {
                                self.canvas.toggle_failure_routes(node.id);
                            }
                        }
                        let has_settings = node
                            .schema
                            .as_ref()
                            .and_then(|schema| schema["properties"].as_object())
                            .is_some_and(|fields| !fields.is_empty());
                        let references = node
                            .references
                            .iter()
                            .filter(|(_, kind)| *kind != ReferenceKind::Connection);
                        let setup = references.clone().next().is_some()
                            || has_settings
                            || !node.types.is_empty();
                        if setup {
                            section(ui, t("set-up"));
                            for (pointer, kind) in references {
                                if let Some(asked) = self.reference_field(ui, &node, pointer, *kind)
                                {
                                    change = Some(asked);
                                }
                            }
                            if let Some(asked) = self.settings_fields(ui, &node, false) {
                                change = Some(asked);
                            }
                        }
                        if !node.variadic.is_empty()
                            && node.definition != "org.orange.audio/multiband-spectrum"
                        {
                            section(ui, t("ports"));
                            if node.definition == intrinsic::SWITCH
                                && node
                                    .ports
                                    .iter()
                                    .find(|port| port.input && port.id == "key")
                                    .and_then(|port| port.ty.as_deref())
                                    .is_some_and(|ty| {
                                        self.document.as_ref().is_some_and(|doc| {
                                            host.enum_choices(&doc.draft, ty).is_some()
                                        })
                                    })
                                && ui.button(t("switch-enum-cases")).clicked()
                            {
                                change = Some(Change::AddEnumCases);
                            }
                            if let Some(asked) = Self::variadic_fields(ui, &node) {
                                change = Some(asked);
                            }
                        }
                        if input_ports(&node).any(|port| !advanced_input(&node, port)) {
                            section(ui, t("inputs"));
                            if !self.build_map_pair_control(ui, &node)
                                && let Some(asked) = self.input_fields(ui, &node, host, false)
                            {
                                change = Some(asked);
                            }
                        }
                        // Tuning settings and type pins: rarely changed, never on the card.
                        let tuning = Self::has_tuning(&node);
                        let advanced_inputs =
                            input_ports(&node).any(|port| advanced_input(&node, port));
                        if tuning || advanced_inputs || !node.types.is_empty() {
                            egui::CollapsingHeader::new(
                                egui::RichText::new(t("advanced")).strong(),
                            )
                            .id_salt(("orange-advanced", node.id))
                            .show(ui, |ui| {
                                if tuning && let Some(asked) = self.settings_fields(ui, &node, true)
                                {
                                    change = Some(asked);
                                }
                                if advanced_inputs
                                    && let Some(asked) = self.input_fields(ui, &node, host, true)
                                {
                                    change = Some(asked);
                                }
                                if let Some(asked) = Self::type_fields(ui, &node) {
                                    change = Some(asked);
                                }
                            });
                        }
                        if !http {
                            self.values_section(ui, &node);
                        }
                        Self::outputs_section(ui, &node);
                        if let Some(current) = self
                            .document
                            .as_ref()
                            .and_then(|doc| host.entry_timing(&doc.draft, node.id))
                            && let Some(timing) = event_timing_fields(ui, node.id, current)
                        {
                            change = Some(Change::EventTiming(timing));
                        }
                    });
            });
        });
        let Some(change) = change else {
            return;
        };
        match change {
            Change::Setting(pointer, value) => {
                let _ = self.setting_with_mode_preview(host, node.id, &pointer, value);
            }
            Change::Type(param, ty) => {
                let _ = self.choose_type(host, node.id, &param, ty.as_deref());
            }
            Change::Input(port, typed) => {
                let _ = self.set_input(host, (node.id, &port), typed.as_deref());
            }
            Change::OmitRecordField(port, ty) => self.omit_record_field(host, node.id, &port, ty),
            Change::AddEnumCases => {
                if let Some(draft) = self.editing() {
                    let result = host.add_enum_cases(draft, node.id);
                    let _ = self.edited(host, result);
                }
            }
            Change::AddPort(port) => {
                let _ = self.add_port(host, node.id, &port);
            }
            Change::RemovePort(port) => {
                let _ = self.remove_port(host, (node.id, &port));
            }
            Change::Delete => {
                let _ = self.remove_node(host, node.id);
            }
            Change::EventTiming(timing) => {
                if let Some(draft) = self.editing() {
                    let result = host.set_entry_timing(draft, node.id, timing);
                    let _ = self.edited(host, result);
                }
            }
        }
    }

    /// A chooser of the declaration a setting names, with a way to declare a new one.
    fn reference_field(
        &mut self,
        ui: &mut egui::Ui,
        node: &NodeView,
        pointer: &str,
        kind: ReferenceKind,
    ) -> Option<Change> {
        let named = node
            .settings
            .pointer(pointer)
            .and_then(Json::as_str)
            .map(str::to_owned);
        let declared: Vec<_> = match kind {
            ReferenceKind::Variable => self
                .variable_choices()
                .into_iter()
                .filter(|choice| choice.fits(&node.definition))
                .map(|choice| (choice.id, choice.label))
                .collect(),
            ReferenceKind::Event => self
                .declarations()
                .into_iter()
                .flat_map(|declared| &declared.events)
                .map(|event| {
                    (
                        event.id.to_string(),
                        format!(
                            "{} ({})",
                            event.name,
                            declarations::payload_name(
                                event.payload.as_ref().map_or("", |ty| ty.as_str())
                            )
                        ),
                    )
                })
                .collect(),
            _ => Vec::new(),
        };
        let what = match kind {
            ReferenceKind::Variable => "variable",
            ReferenceKind::Connection => "connection",
            ReferenceKind::Event => "event",
            ReferenceKind::Function => "function",
        };
        let shown = named
            .as_ref()
            .and_then(|id| declared.iter().find(|(known, _)| known == id))
            .map_or_else(
                || tf("choose-reference", &[("kind", what.into())]),
                |(_, name)| name.clone(),
            );
        let label = if intrinsic::counter_operation(&node.definition).is_some()
            && kind == ReferenceKind::Variable
        {
            t("counter-variable").to_owned()
        } else {
            words(pointer.trim_start_matches('/'))
        };
        let mut chosen = None;
        let mut quick = false;
        row(ui, &label, None, |ui| {
            let _ = egui::ComboBox::from_id_salt(("orange-reference", pointer))
                .selected_text(shown)
                .wrap_mode(egui::TextWrapMode::Truncate)
                .width(ui.available_width())
                .show_select(ui, Surface::Panel, |ui| {
                    for (id, name) in &declared {
                        let current = named.as_deref() == Some(id.as_str());
                        if ui.selectable_label(current, name.as_str()).clicked() && !current {
                            chosen = Some(Change::Setting(pointer.to_owned(), Some(json!(id))));
                        }
                    }
                    if matches!(kind, ReferenceKind::Variable | ReferenceKind::Event) {
                        if !declared.is_empty() {
                            ui.separator();
                        }
                        let new = if kind == ReferenceKind::Event {
                            t("event-new").to_owned()
                        } else {
                            tf("new-reference", &[("kind", what.into())])
                        };
                        quick = ui.selectable_label(false, new).clicked();
                    } else if declared.is_empty() {
                        ui.weak(tf("no-references", &[("kind", what.into())]));
                    }
                });
        });
        if kind == ReferenceKind::Event
            && named
                .as_deref()
                .is_some_and(|id| declared.iter().any(|(key, _)| key == id))
            && ui.button(t("event-rename")).clicked()
        {
            self.rename_event_for_node(node.id);
        }
        if quick {
            match kind {
                ReferenceKind::Event => self.new_event_for_node(node.id),
                ReferenceKind::Variable => self.new_variable_for_node(node.id, pointer),
                _ => {}
            }
        }
        chosen
    }

    /// Fields for the settings the node's schema describes, but not those naming a
    /// declaration.
    fn settings_fields(
        &mut self,
        ui: &mut egui::Ui,
        node: &NodeView,
        tuning: bool,
    ) -> Option<Change> {
        let properties = node.schema.as_ref()?["properties"].as_object()?;
        let required: Vec<&str> = node.schema.as_ref()?["required"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Json::as_str)
            .collect();
        let mut changed = None;
        let watch = intrinsic::is_watch(&node.definition);
        let mut fields: Vec<_> = watch_field_entries(node)
            .chain(properties.iter().filter(|_| !watch))
            .collect();
        if !node.controls.regions.is_empty() {
            fields.sort_by_key(|(name, _)| field_order(node, name));
        }
        for (name, schema) in fields {
            if schema["readOnly"] == true {
                continue;
            }
            if name == "bands" && node.definition == "org.orange.audio/multiband-spectrum" {
                continue;
            }
            let pointer = format!("/{name}");
            if node.references.iter().any(|(at, _)| *at == pointer)
                || canvas::advanced_setting(node, name) != tuning
            {
                continue;
            }
            let field = Field::for_node(node, name, schema);
            let current = &node.settings[name.as_str()];
            let needed = required.contains(&name.as_str())
                || (watch && matches!(name.as_str(), "threshold" | "low" | "high"));
            let measurement_time =
                name == "window_ms" && node.definition.starts_with("org.orange.audio/");
            let stream = match (node.definition.as_str(), name.as_str()) {
                (intrinsic::COLLECT, "max_items") => {
                    Some(("stream-max-items", "stream-max-items-help"))
                }
                (intrinsic::TEE, "capacity") => {
                    Some(("stream-tee-capacity", "stream-tee-capacity-help"))
                }
                (intrinsic::TEE, "max_bytes") => {
                    Some(("stream-tee-max-bytes", "stream-tee-max-bytes-help"))
                }
                (intrinsic::TEE, "overflow") => {
                    Some(("stream-tee-overflow", "stream-tee-overflow-help"))
                }
                _ => None,
            };
            let label = if watch {
                watch_field_label(node, name).to_owned()
            } else if let Some((label, _)) = stream {
                t(label).to_owned()
            } else if name == "max_iterations" {
                t("max-iterations").to_owned()
            } else if let Some(label) = audio_setting_label(node, name) {
                if matches!(field, Field::Time) || name == "hold_ms" {
                    tf("in-milliseconds", &[("name", label.into())])
                } else {
                    label.to_owned()
                }
            } else if measurement_time {
                t("audio-setting-window-ms").to_owned()
            } else if matches!(field, Field::Time) {
                tf("in-milliseconds", &[("name", setting_name(name).into())])
            } else {
                setting_name(name)
            };
            let hover = Some(
                if let Some(help) = audio_trigger_help(node, name) {
                    help
                } else if let Some((_, help)) = stream {
                    t(help)
                } else if measurement_time {
                    t("audio-setting-window-help")
                } else if name == "max_iterations" {
                    t("max-iterations-help")
                } else if needed {
                    t("required")
                } else {
                    t("optional")
                }
                .to_owned(),
            );
            let mut said = None;
            row(ui, &label, hover, |ui| match &field {
                Field::Choice(options) if setting_hint(node, name, Widget::Segmented).is_some() => {
                    // The same row of choices as on the card (ADR-33).
                    let selected = current.as_str().or_else(|| schema["default"].as_str());
                    let labels: Vec<String> = options
                        .iter()
                        .map(|option| setting_choice_label(node, option).into_owned())
                        .collect();
                    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
                        egui::Sense::hover(),
                    );
                    let row = orange_ui_controls::ChoiceRow {
                        labels: &labels,
                        selected: options.iter().position(|option| selected == Some(*option)),
                        defaulted: current.is_null(),
                        surface: Surface::Panel,
                        zoom: 1.0,
                    };
                    let id = ui.make_persistent_id(("orange-setting-choices", name.as_str()));
                    if let Some(picked) = row.show_named(ui, rect, id, &label) {
                        changed = Some(Change::Setting(
                            pointer.clone(),
                            Some(json!(options[picked])),
                        ));
                    }
                }
                Field::Choice(options) => {
                    let selected = current.as_str().or_else(|| schema["default"].as_str());
                    let shown = selected.map_or_else(
                        || t("not-set").into(),
                        |choice| setting_choice_label(node, choice),
                    );
                    let _ = egui::ComboBox::from_id_salt(("orange-setting", name.as_str()))
                        .selected_text(shown)
                        .width(ui.available_width())
                        .show_select(ui, Surface::Panel, |ui| {
                            if node.definition != intrinsic::FOR_EACH
                                && !needed
                                && ui
                                    .selectable_label(current.is_null(), t("not-set"))
                                    .clicked()
                                && !current.is_null()
                            {
                                changed = Some(Change::Setting(pointer.clone(), None));
                            }
                            for option in options {
                                let on = selected == Some(*option);
                                let label = setting_choice_label(node, option);
                                if ui.selectable_label(on, label).clicked() && !on {
                                    changed =
                                        Some(Change::Setting(pointer.clone(), Some(json!(option))));
                                }
                            }
                        });
                }
                Field::Toggle => {
                    let on = current.as_bool().or_else(|| schema["default"].as_bool());
                    if let Some(on) = yes_no(ui, on) {
                        changed = Some(Change::Setting(pointer.clone(), Some(json!(on))));
                    }
                }
                Field::Whole
                | Field::Number
                | Field::WatchNumber { .. }
                | Field::Time
                | Field::Json
                | Field::Text => {
                    if name == ThresholdEdit::SETTING
                        && let Some(threshold) = ThresholdEdit::for_node(node)
                    {
                        let width = (ui.available_width() - 102.0).max(28.0);
                        let slider = ui
                            .allocate_ui(egui::vec2(width, ui.spacing().interact_size.y), |ui| {
                                ui.set_width(width);
                                threshold.slider(ui, Surface::Panel)
                            })
                            .inner;
                        if let Some(value) = threshold.number(ui).or(slider) {
                            changed = Some(Change::Setting(pointer.clone(), Some(json!(value))));
                        }
                        return;
                    }
                    if let Some(range) = NumberRange::of(node, name, schema)
                        && let Some(value) = current.as_f64().or_else(|| schema["default"].as_f64())
                    {
                        let id = ui.make_persistent_id((&node.id, &pointer));
                        let adjusted = if let Some(dial) = dial_of(node, name) {
                            if let Some(value) =
                                range.dial(ui, Surface::Panel, id, value, dial, &label, "", None)
                            {
                                changed =
                                    Some(Change::Setting(pointer.clone(), Some(json!(value))));
                            }
                            return;
                        } else {
                            let width = (ui.available_width() - 96.0).max(32.0);
                            ui.allocate_ui(egui::vec2(width, ui.spacing().interact_size.y), |ui| {
                                ui.set_width(width);
                                range.slider(
                                    ui,
                                    Surface::Panel,
                                    id,
                                    value,
                                    &label,
                                    node.controls.settings.get(name),
                                )
                            })
                            .inner
                        };
                        if let Some(value) = adjusted {
                            #[expect(
                                clippy::cast_possible_truncation,
                                reason = "an integer setting rounds the bounded slider position"
                            )]
                            let literal = if schema["type"] == "integer" {
                                json!(value.round() as i64)
                            } else {
                                json!(value)
                            };
                            changed = Some(Change::Setting(pointer.clone(), Some(literal)));
                        }
                    }
                    if let Some(stepper) = setting_hint(node, name, Widget::Stepper)
                        && let Some(value) = current.as_f64().or_else(|| schema["default"].as_f64())
                    {
                        // Minus and plus around the number, as on the card (ADR-33).
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
                            egui::Sense::hover(),
                        );
                        let id = ui.make_persistent_id(("orange-setting-steps", name.as_str()));
                        if let Some(sign) =
                            orange_ui_controls::step_buttons(ui, rect, id, Surface::Panel, 1.0)
                            && let Some(next) = stepper_value(stepper, schema, value, sign)
                            && let Ok(next) = Field::Whole.value(&next.to_string())
                        {
                            changed = Some(Change::Setting(pointer.clone(), Some(next)));
                        }
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            hinted_number(Some(stepper), value),
                            egui::TextStyle::Monospace.resolve(ui.style()),
                            ui.visuals().text_color(),
                        );
                        return;
                    }
                    let before = field.typed(current);
                    let typed = self.typed.entry(setting_key(&pointer)).or_default();
                    let hint = if needed { "required" } else { "optional" };
                    let clear = !needed && !current.is_null();
                    match text_field(ui, typed, hint, clear, None) {
                        Edit::Done if *typed != before => {
                            if typed.trim().is_empty() && !needed {
                                changed = Some(Change::Setting(pointer.clone(), None));
                            } else {
                                match field.value(typed) {
                                    Ok(value) => {
                                        changed =
                                            Some(Change::Setting(pointer.clone(), Some(value)));
                                    }
                                    Err(why) => said = Some(why),
                                }
                            }
                        }
                        Edit::Clear => changed = Some(Change::Setting(pointer.clone(), None)),
                        Edit::Done | Edit::None | Edit::Browse => {}
                    }
                }
            });
            if let Some(why) = said {
                self.say(tf("not-done", &[("why", why.to_string().into())]));
            }
        }
        changed
    }

    /// Whether the node has tuning settings, shown under Advanced.
    fn has_tuning(node: &NodeView) -> bool {
        node.schema
            .as_ref()
            .and_then(|schema| schema["properties"].as_object())
            .is_some_and(|fields| {
                fields
                    .keys()
                    .any(|name| canvas::advanced_setting(node, name))
            })
    }

    /// Choosers of the types of a generic node's parameters.
    fn type_fields(ui: &mut egui::Ui, node: &NodeView) -> Option<Change> {
        let mut typing = None;
        for param in &node.types {
            let shown = param
                .chosen
                .as_deref()
                .map_or_else(|| t("from-the-wires").to_owned(), type_name);
            let hover = param.found.as_deref().map_or_else(
                || t("no-type-yet").to_owned(),
                |found| tf("type-now", &[("type", type_name(found).into())]),
            );
            let label = tf("type-parameter", &[("name", param.name.to_string().into())]);
            row(ui, &label, Some(hover), |ui| {
                let _ = egui::ComboBox::from_id_salt(("orange-type", param.name.as_str()))
                    .selected_text(shown)
                    .width(ui.available_width())
                    .show_select(ui, Surface::Panel, |ui| {
                        if ui
                            .selectable_label(param.chosen.is_none(), t("from-the-wires"))
                            .clicked()
                            && param.chosen.is_some()
                        {
                            typing = Some(Change::Type(param.name.clone(), None));
                        }
                        for ty in COMMON_TYPES {
                            let current = param.chosen.as_deref() == Some(ty);
                            if ui.selectable_label(current, type_name(ty)).clicked() && !current {
                                typing =
                                    Some(Change::Type(param.name.clone(), Some(ty.to_owned())));
                            }
                        }
                    });
            });
        }
        typing
    }

    /// How many instances each variadic port has, with buttons for one more or one
    /// fewer (the last one, with its wires).
    fn variadic_fields(ui: &mut egui::Ui, node: &NodeView) -> Option<Change> {
        let mut change = None;
        for variadic in &node.variadic {
            let last = node
                .ports
                .iter()
                .rfind(|port| {
                    port.input == variadic.input
                        && port
                            .id
                            .split_once('#')
                            .is_some_and(|(name, _)| name == variadic.port)
                })
                .map(|port| port.id.clone());
            let hover = match variadic.max {
                Some(max) => tf(
                    "inputs-range",
                    &[("min", variadic.min.into()), ("max", max.into())],
                ),
                None => tf("inputs-at-least", &[("min", variadic.min.into())]),
            };
            row(ui, &words(&variadic.port), Some(hover), |ui| {
                ui.label(variadic.count.to_string());
                let fewer = variadic.count > variadic.min && last.is_some();
                if ui
                    .add_enabled(fewer, egui::Button::new("−"))
                    .on_hover_text(t("remove-last-input-hint"))
                    .clicked()
                    && let Some(last) = &last
                {
                    change = Some(Change::RemovePort(last.clone()));
                }
                let more = variadic.max.is_none_or(|max| variadic.count < max);
                if ui
                    .add_enabled(more, egui::Button::new(icons::text(icons::ADD)))
                    .on_hover_text(t("add-one-more"))
                    .clicked()
                {
                    change = Some(Change::AddPort(variadic.port.clone()));
                }
            });
        }
        change
    }

    /// Fields for the values on the node's inputs.
    fn input_fields(
        &mut self,
        ui: &mut egui::Ui,
        node: &NodeView,
        host: &Host,
        advanced: bool,
    ) -> Option<Change> {
        let record_omissions = self
            .document
            .as_ref()
            .filter(|_| node.missing.is_none())
            .map_or_else(Default::default, |document| {
                host.record_omissions(&document.draft, node.id)
            });
        let mut omit = None;
        let mut write = None;
        let mut ports: Vec<_> = input_ports(node)
            .filter(|port| advanced_input(node, port) == advanced)
            .collect();
        if !node.controls.regions.is_empty() {
            ports.sort_by_key(|port| field_order(node, &port.id));
        }
        for port in ports {
            let ty = port.ty.as_deref().map_or("value".to_owned(), type_name);
            let details = self.canvas.port_details(node.id, &port.id, true);
            let editor = port.literal_editor(details);
            let state = port_values::state_text(port.input_state(details));
            let mut help = format!("{ty}\n{state}");
            if let Some(description) = details.and_then(|detail| detail.description.as_deref()) {
                help.push('\n');
                help.push_str(description);
            }
            let multiline = TextInput::for_port(node, port).filter(|kind| *kind != TextInput::Text);
            let label = input_caption(node, port);
            if node.definition == "org.orange.logging/log-message"
                && port.id == "message"
                && node.variadic.iter().any(|port| port.port == "argument")
            {
                ui.add(egui::Label::new(egui::RichText::new(t("log-format-hint")).weak()).wrap());
            }
            let label = if is_folder_path(node, &port.id) {
                folder_path_label(
                    node,
                    label,
                    self.named_connection(node)
                        .map(|folder| folder.name.as_str()),
                )
            } else {
                label
            };
            row(ui, &label, Some(help), |ui| {
                if port.connected {
                    ui.weak(state);
                    return;
                }
                match record_omissions.get(&port.id) {
                    Some(true) => {
                        ui.weak(t("record-field-omitted"));
                    }
                    Some(false) if ui.small_button(t("record-omit-field")).clicked() => {
                        if let Some(ty) = port.ty.as_deref().and_then(|ty| ty.parse().ok()) {
                            omit = Some(Change::OmitRecordField(port.id.clone(), ty));
                            return;
                        }
                    }
                    _ => {}
                }
                let enum_picker = port
                    .ty
                    .as_deref()
                    .and_then(|ty| self.canvas.enums.get(ty))
                    .is_some_and(|choices| choices.iter().any(|choice| !choice.payload));
                if multiline.is_none()
                    && !enum_picker
                    && matches!(
                        editor,
                        orange_host::LiteralEditor::Option { .. }
                            | orange_host::LiteralEditor::Structured
                    )
                {
                    let shown = port
                        .value
                        .as_deref()
                        .or(port.default.as_deref())
                        .unwrap_or(state);
                    let response = ui.button(shown).on_hover_text(t("edit-literal"));
                    if response.clicked() {
                        self.begin_value_input(node.id, &port.id);
                    }
                    return;
                }
                if editor == orange_host::LiteralEditor::Unavailable {
                    ui.weak(t("literal-unavailable"));
                    return;
                }
                if port.ty.as_deref() == Some(orange_host::RGBA_TYPE) {
                    let color = input_color(port);
                    ui.horizontal(|ui| {
                        let (chip, _) =
                            ui.allocate_exact_size(egui::Vec2::splat(17.0), egui::Sense::hover());
                        orange_ui_controls::color_picker::swatch(ui, chip, color);
                        let response = ui.button(orange_host::rgba_hex(color));
                        if let Some(text) = pick_color(ui, &response, color) {
                            write = Some(Change::Input(port.id.clone(), Some(text)));
                        }
                    });
                    return;
                }
                if http_request_variant(node) && port.id == "method" {
                    let shown = port
                        .value
                        .as_deref()
                        .or(port.default.as_deref())
                        .unwrap_or_default();
                    let shown =
                        serde_json::from_str::<String>(shown).unwrap_or_else(|_| shown.to_owned());
                    egui::ComboBox::from_id_salt(("http-method", node.id))
                        .width(ui.available_width())
                        .selected_text(
                            egui::RichText::new(&shown)
                                .strong()
                                .color(ui.visuals().strong_text_color()),
                        )
                        .show_select(ui, Surface::Panel, |ui| {
                            if let Some(method) = choose_http_method(ui, &shown) {
                                write =
                                    Some(Change::Input(port.id.clone(), Some(method.to_owned())));
                            }
                        });
                    return;
                }
                if let Some(multiline) = multiline {
                    let typed = self.typed.entry(port.id.clone()).or_default();
                    let mut syntax_error = None;
                    let (response, formatted) = ui
                        .vertical(|ui| {
                            let id = ui.make_persistent_id(("property-text", node.id, &port.id));
                            let automatic =
                                text_area_height(typed, ui.available_width(), multiline);
                            let height = ui
                                .data(|data| data.get_temp::<f32>(id))
                                .unwrap_or(automatic);
                            let response = egui::ScrollArea::vertical()
                                .id_salt(id)
                                .max_height(height)
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::TextEdit::multiline(typed)
                                            .id(id.with("input"))
                                            .font(egui::TextStyle::Monospace)
                                            .layouter(&mut wrap_property_text)
                                            .hint_text(multiline.hint())
                                            .min_size(egui::vec2(0.0, height))
                                            .desired_width(ui.available_width()),
                                    )
                                })
                                .inner;
                            let (grip, _) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 8.0),
                                egui::Sense::hover(),
                            );
                            let delta = text_resize_grip(ui, grip, id.with("resize"));
                            if delta != 0.0 {
                                ui.data_mut(|data| {
                                    data.insert_temp(id, (height + delta).clamp(60.0, 280.0))
                                });
                                ui.ctx().request_repaint();
                            }
                            let mut formatted = false;
                            if multiline == TextInput::Body
                                && !typed.trim().is_empty()
                                && ui.small_button(t("literal-format-json")).clicked()
                            {
                                match serde_json::from_str::<Json>(typed) {
                                    Ok(value) => {
                                        if let Ok(pretty) = serde_json::to_string_pretty(&value) {
                                            *typed = pretty;
                                            formatted = true;
                                        }
                                    }
                                    Err(error) => {
                                        syntax_error = Some(port_values::json_syntax_message(
                                            error.line(),
                                            error.column(),
                                        ));
                                    }
                                }
                            }
                            (response, formatted)
                        })
                        .inner;
                    if response.lost_focus() || formatted {
                        let typed = typed.clone();
                        let original = self
                            .document
                            .as_ref()
                            .and_then(|document| {
                                stored_input_text(host, &document.draft, node.id, &port.id)
                            })
                            .unwrap_or_default();
                        if typed != original {
                            let literal = (!typed.is_empty()).then(|| multiline.literal(&typed));
                            write = Some(Change::Input(port.id.clone(), literal));
                        }
                    }
                    if let Some(error) = syntax_error {
                        self.say(error);
                    }
                    return;
                }
                if let Some(choices) = port.ty.as_deref().and_then(|ty| self.canvas.enums.get(ty)) {
                    let _ = egui::ComboBox::from_id_salt(("orange-choice", port.id.as_str()))
                        .width(ui.available_width())
                        .selected_text(choice_caption(
                            port,
                            port.value
                                .as_deref()
                                .or(port.default.as_deref())
                                .unwrap_or(t("choose")),
                        ))
                        .show_select(ui, Surface::Panel, |ui| {
                            for choice in choices.iter().filter(|choice| !choice.payload) {
                                let current = port.value.as_deref().or(port.default.as_deref())
                                    == Some(choice.name.as_str());
                                if ui
                                    .selectable_label(current, choice_caption(port, &choice.name))
                                    .clicked()
                                    && !current
                                {
                                    write = Some(Change::Input(
                                        port.id.clone(),
                                        Some(choice.literal.clone()),
                                    ));
                                }
                            }
                        });
                    return;
                }
                if port.ty.as_deref() == Some("core/bool@1") {
                    let on = port
                        .value
                        .as_deref()
                        .or(port.default.as_deref())
                        .map(|value| value == "true");
                    if let Some(on) = yes_no(ui, on) {
                        write = Some(Change::Input(port.id.clone(), Some(on.to_string())));
                    }
                    return;
                }
                if port.ty.as_deref() == Some("core/duration@1") {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if port.value.is_some()
                            && ui
                                .small_button(icons::text(icons::CLOSE))
                                .on_hover_text(t("clear-value-hint"))
                                .clicked()
                        {
                            write = Some(Change::Input(port.id.clone(), None));
                        }
                        let (slot, response) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 30.0),
                            egui::Sense::hover(),
                        );
                        let key = format!("properties-duration:{}", port.id);
                        if let Some(nanos) = self.canvas.duration_input(
                            ui,
                            (node.id, &key),
                            slot,
                            response.id.with("value"),
                            port,
                        ) {
                            write = Some(Change::Input(port.id.clone(), Some(nanos)));
                        }
                    });
                    return;
                }
                let before = Self::shown_input(port);
                if let Some(control) = input_hint(node, port)
                    && let Some(range) = NumberRange::of_input(control)
                {
                    let value = port
                        .value
                        .as_deref()
                        .or(port.default.as_deref())
                        .and_then(|text| {
                            text.parse::<f64>()
                                .ok()
                                .or_else(|| serde_json::from_str::<String>(text).ok()?.parse().ok())
                        })
                        .filter(|value| value.is_finite())
                        .unwrap_or_else(|| 0.0_f64.clamp(range.min, range.max));
                    let id = ui.make_persistent_id((node.id, &port.id, "hinted-input"));
                    let adjusted = if control.widget == Widget::Dial {
                        range.dial(ui, Surface::Panel, id, value, control, &label, "", None)
                    } else {
                        let width = (ui.available_width() * 0.6).max(1.0);
                        ui.allocate_ui(egui::vec2(width, ui.spacing().interact_size.y), |ui| {
                            ui.set_width(width);
                            range.slider(ui, Surface::Panel, id, value, &label, Some(control))
                        })
                        .inner
                    };
                    if let Some(value) = adjusted {
                        write = Some(Change::Input(
                            port.id.clone(),
                            Some(input_number_text(port, value)),
                        ));
                    }
                }
                // A path is chosen with the system's chooser, where the application
                // has one.
                let path = node
                    .paths
                    .iter()
                    .find(|path| path.port == port.id)
                    .filter(|_| self.file_dialogs);
                let browse = path.map(|path| if path.folder { "folder" } else { "file" });
                let typed = self.typed.entry(port.id.clone()).or_default();
                let hint = if http_request_variant(node) && port.id == "path" {
                    "/anything"
                } else if http_request_variant(node) && port.id == "content_type" {
                    "application/json"
                } else {
                    port.default.as_deref().unwrap_or(&ty)
                };
                match text_field(ui, typed, hint, port.value.is_some(), browse) {
                    Edit::Done if *typed != before => {
                        let value = (!typed.is_empty()).then(|| typed.clone());
                        write = Some(Change::Input(port.id.clone(), value));
                    }
                    Edit::Clear => write = Some(Change::Input(port.id.clone(), None)),
                    Edit::Browse => {
                        if let Some(path) = path {
                            self.request = Some(FileRequest::ChoosePath {
                                node: node.id,
                                port: port.id.clone(),
                                folder: path.folder,
                                save: path.save,
                                extensions: path.extensions.clone(),
                                current: typed.clone(),
                            });
                        }
                    }
                    Edit::Done | Edit::None => {}
                }
            });
        }
        omit.or(write)
    }
}

fn event_timing_fields(
    ui: &mut egui::Ui,
    node: NodeId,
    current: Option<EventTiming>,
) -> Option<Option<EventTiming>> {
    use std::num::NonZeroU32;
    section(ui, t("event-timing"));
    let mut mode = match current {
        None => 0,
        Some(EventTiming::Debounce { .. }) => 1,
        Some(EventTiming::Throttle { .. }) => 2,
    };
    let labels = [
        t("event-immediate"),
        t("event-debounce"),
        t("event-throttle"),
    ];
    egui::ComboBox::from_id_salt(("event-timing", node))
        .selected_text(labels[mode])
        .show_select(ui, Surface::Panel, |ui| {
            for (index, label) in labels.into_iter().enumerate() {
                ui.selectable_value(&mut mode, index, label);
            }
        });
    let mut interval = current.map_or(250, |timing| timing.interval_ms().get());
    if mode != 0 {
        ui.horizontal(|ui| {
            ui.label(t("event-interval"));
            ui.add(
                egui::DragValue::new(&mut interval)
                    .range(1..=u32::MAX)
                    .suffix(" ms")
                    .update_while_editing(false),
            );
        });
        ui.add(
            egui::Label::new(t(if mode == 1 {
                "event-debounce-help"
            } else {
                "event-throttle-help"
            }))
            .wrap(),
        );
    }
    let interval_ms = NonZeroU32::new(interval).unwrap_or(NonZeroU32::MIN);
    let selected = match mode {
        1 => Some(EventTiming::Debounce { interval_ms }),
        2 => Some(EventTiming::Throttle { interval_ms }),
        _ => None,
    };
    (current != selected).then_some(selected)
}

/// What a person asked for in the properties panel.
enum Change {
    Setting(String, Option<Json>),
    Type(String, Option<String>),
    Input(String, Option<String>),
    OmitRecordField(String, orange_host::TypeExpr),
    AddPort(String),
    AddEnumCases,
    RemovePort(String),
    EventTiming(Option<EventTiming>),
    /// Declare a variable and name it at the setting.
    Delete,
}

/// What happened to a text field this frame.
enum Edit {
    None,
    /// Typing ended: Enter, or the field lost the keys.
    Done,
    /// Its value was taken off.
    Clear,
    /// The chooser of a path was asked for.
    Browse,
}

/// A text field that fills its row, read when typing ends, with a button that takes
/// the value off when `clear` is set and one that asks for the system's chooser when
/// `browse` names what it chooses ("file" or "folder").
fn text_field(
    ui: &mut egui::Ui,
    typed: &mut String,
    hint: &str,
    clear: bool,
    browse: Option<&str>,
) -> Edit {
    // Laid out from the right: the button takes the room it really needs and the field
    // exactly what is left, so the row never asks for more than the panel has (a
    // resizable panel would otherwise widen itself on every frame of typing).
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let cleared = clear
            && ui
                .small_button(icons::text(icons::CLOSE))
                .on_hover_text(t("clear-value-hint"))
                .clicked();
        let browsed = browse.is_some_and(|what| {
            ui.small_button("…")
                .on_hover_text(tf("choose-path-kind", &[("kind", what.into())]))
                .clicked()
        });
        let field = ui.add(
            egui::TextEdit::singleline(typed)
                .hint_text(hint)
                .desired_width(ui.available_width().max(40.0)),
        );
        if cleared {
            Edit::Clear
        } else if browsed {
            Edit::Browse
        } else if field.lost_focus() {
            Edit::Done
        } else {
            Edit::None
        }
    })
    .inner
}

/// A small heading over a group of rows.
fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(title).strong());
}

/// One row of the panel, as a card's field is laid out (22 §3.1): its name on one
/// line, cut short and shown whole on hover with `hover` under it, and its field
/// across the panel's whole width on the next.
fn row(ui: &mut egui::Ui, label: &str, hover: Option<String>, field: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(2.0);
    let _ = ui
        .add(egui::Label::new(egui::RichText::new(label).small().weak()).truncate())
        .on_hover_text(match hover {
            Some(more) => format!("{label}\n{more}"),
            None => label.to_owned(),
        });
    let _ = ui.horizontal(|ui| {
        ui.set_width(ui.available_width());
        field(ui);
    });
}

/// A No | Yes switch across the row; `None` chooses neither. Returns the half clicked
/// when it is not the one chosen.
fn yes_no(ui: &mut egui::Ui, on: Option<bool>) -> Option<bool> {
    let (id, rect) = ui.allocate_space(egui::vec2(
        ui.available_width(),
        ui.spacing().interact_size.y,
    ));
    orange_ui_controls::BinaryChoice {
        labels: [t("no"), t("yes")],
        selected: on,
        defaulted: false,
        surface: Surface::Panel,
        zoom: 1.0,
    }
    .show(ui, rect, id)
}

/// A setting's name as a person reads it: `max_iterations` is "Max iterations".
pub(crate) fn words(name: &str) -> String {
    let mut text = name.replace(['_', '-'], " ");
    if let Some(first) = text.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    text
}

fn setting_name(name: &str) -> String {
    let key = format!("setting-{}", name.replace('_', "-"));
    orange_host::Catalogs::built_in()
        .message(&i18n::language(), &key)
        .map_or_else(|| words(name), std::borrow::Cow::into_owned)
}

/// A port's name as a card or the panel shows it: an instance of a variadic port
/// (`branch#0`) is counted from one ("branch 1"), and words are spaced.
/// A port the host's catalogues name in the current language shows that name.
pub fn port_label(id: &str) -> String {
    let name = |id: &str| i18n::port_name(id).unwrap_or_else(|| id.replace(['_', '-'], " "));
    match id.split_once('#') {
        Some((base, instance)) => match instance.parse::<u32>() {
            Ok(number) => format!("{} {}", name(base), number + 1),
            Err(_) => id.to_owned(),
        },
        None => name(id),
    }
}

pub(crate) fn setting_choice_label<'a>(
    node: &NodeView,
    choice: &'a str,
) -> std::borrow::Cow<'a, str> {
    if choice.starts_with("local/") {
        return type_name(choice).into();
    }
    if let Some(label) = setting_choice_caption(node, choice) {
        return label.into();
    }
    let key = format!("setting-choice-{}", choice.replace('_', "-"));
    orange_host::Catalogs::built_in()
        .message(&i18n::language(), &key)
        .unwrap_or_else(|| choice.into())
}

fn setting_choice_caption(node: &NodeView, choice: &str) -> Option<&'static str> {
    Some(match (node.definition.as_str(), choice) {
        (intrinsic::ON_WATCH, _) => watch_label(choice),
        ("org.orange.audio/noise-level", _) => canvas::spectrum::direction(choice),
        (intrinsic::FOR_EACH, "list") => t("for-each-list"),
        (intrinsic::FOR_EACH, "map") => t("for-each-map"),
        ("org.orange.audio/filter", "low-pass") => t("audio-filter-low"),
        ("org.orange.audio/filter", "high-pass") => t("audio-filter-high"),
        ("org.orange.audio/filter", "band-pass") => t("audio-filter-band"),
        ("org.orange.audio/filter", "notch") => t("audio-filter-notch"),
        ("org.orange.time/parse-timestamp" | "org.orange.time/format-timestamp", "rfc3339") => {
            t("timestamp-rfc3339")
        }
        (
            "org.orange.time/parse-timestamp" | "org.orange.time/format-timestamp",
            "unix-seconds",
        ) => t("timestamp-unix-seconds"),
        (
            "org.orange.time/parse-timestamp" | "org.orange.time/format-timestamp",
            "unix-milliseconds",
        ) => t("timestamp-unix-milliseconds"),
        (
            "org.orange.time/parse-timestamp" | "org.orange.time/format-timestamp",
            "unix-microseconds",
        ) => t("timestamp-unix-microseconds"),
        (
            "org.orange.time/parse-timestamp" | "org.orange.time/format-timestamp",
            "unix-nanoseconds",
        ) => t("timestamp-unix-nanoseconds"),
        _ => return None,
    })
}

/// What a node of a package this host lacks says it needs: the package and, when the
/// document records it, the version.
pub(crate) fn missing_package_text(missing: &orange_host::MissingPackage) -> String {
    if missing.version.is_empty() {
        tf(
            "node-needs-package-any",
            &[("package", missing.package.clone().into())],
        )
    } else {
        tf(
            "node-needs-package",
            &[
                ("package", missing.package.clone().into()),
                ("version", missing.version.clone().into()),
            ],
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_are_set_and_taken_off_at_a_pointer() {
        let mut settings = json!({"keep": 1});
        set_at(&mut settings, "/variable", Some(json!("x")));
        set_at(&mut settings, "/deep/er", Some(json!(2)));
        assert_eq!(
            settings,
            json!({"keep": 1, "variable": "x", "deep": {"er": 2}})
        );
        set_at(&mut settings, "/deep/er", None);
        set_at(&mut settings, "/missing/one", None);
        set_at(&mut settings, "/variable", None);
        assert_eq!(settings, json!({"keep": 1, "deep": {}}));
    }

    #[test]
    fn structured_settings_are_edited_as_json() {
        for (schema, value) in [
            (json!({"type": "object"}), json!({"nested": [true, 2]})),
            (json!({"type": "array"}), json!([1, "text"])),
        ] {
            let field = Field::of(&schema);
            assert_eq!(field.value(&field.typed(&value)).unwrap(), value);
            assert!(field.value("{ incomplete").is_err());
        }
    }

    #[test]
    fn lengths_of_time_are_typed_in_milliseconds() {
        let time = json!({"type": "string", "format": "duration-ns"});
        let field = Field::of(&time);
        assert_eq!(field.value("1500").unwrap(), json!("1500000000"));
        assert_eq!(field.typed(&json!("250000000")), "250");
        assert!(field.value("soon").is_err());
        assert!(field.value("-1").is_err());
        let whole = json!({"type": "integer"});
        assert_eq!(Field::of(&whole).value("12").unwrap(), json!(12));
        assert!(Field::of(&whole).value("1.5").is_err());
    }

    #[test]
    fn when_fields_keep_their_order_and_exact_integer_values() {
        let (host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        let watch = host
            .add_node(&mut draft, intrinsic::ON_WATCH, [0.0, 0.0])
            .unwrap();
        let threshold = watch_number_value("18446744073709551615", false).unwrap();
        assert_eq!(threshold.as_u64(), Some(u64::MAX));
        assert!(watch_number_value("-1", true).is_err());
        assert!(watch_number_value("1.5", true).is_err());
        host.set_config(
            &mut draft,
            watch,
            &json!({"mode": "above", "threshold": threshold}),
        )
        .unwrap();
        let view = host.draft_view(&draft).unwrap();
        let card = view.nodes.iter().find(|card| card.id == watch).unwrap();
        assert_eq!(
            watch_field_entries(card)
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["mode", "threshold", "release", "hold", "cooldown"]
        );
    }
}

#[cfg(test)]
mod numeric_hint_tests {
    use super::*;

    #[test]
    fn extreme_slider_ranges_produce_finite_positions_and_values() {
        for (scale, low, high, midpoint) in [
            (Scale::Linear, -f64::MAX, f64::MAX, 0.0),
            (Scale::Log, 1.0e-300, 1.0e300, 1.0),
        ] {
            let control = Control {
                widget: Widget::Slider,
                scale,
                range: Some([low, high]),
                ..Control::default()
            };
            let range = NumberRange::of_input(&control).unwrap();
            assert_eq!(range.fraction(f64::NAN), 0.0);
            assert_eq!(range.fraction(midpoint), 0.5);
            assert_eq!(range.value(0.5), midpoint);
            assert_eq!(range.value(0.0), low);
            assert_eq!(range.value(1.0), high);
            assert!(range.value(0.99).is_finite());
        }
    }

    #[test]
    fn typed_settings_reject_non_finite_numbers_and_overflowing_times() {
        for input in ["NaN", "inf", "-inf", "1e309"] {
            assert!(Field::Number.value(input).is_err());
        }
        for input in ["-1", "-0.000000001", "18446744073709.552", "1e300"] {
            assert!(Field::Time.value(input).is_err());
        }
        assert_eq!(Field::Time.value("0.000001").unwrap(), json!("1"));
    }

    #[test]
    fn steppers_reject_reversed_or_empty_integer_bounds() {
        let hint = Control {
            widget: Widget::Stepper,
            ..Control::default()
        };
        for schema in [
            json!({"minimum": 5, "maximum": 2}),
            json!({"minimum": 1.2, "maximum": 1.8}),
        ] {
            assert_eq!(stepper_value(&hint, &schema, 2.0, 1), None);
        }
        let limits = json!({"minimum": 1.2, "maximum": 3.8});
        assert_eq!(stepper_value(&hint, &limits, 3.0, 1), Some(3.0));
        assert_eq!(stepper_value(&hint, &limits, 2.0, -1), Some(2.0));
    }

    #[test]
    fn unrepresentable_dial_bounds_fall_back_to_typed_fields() {
        for [low, high] in [
            [-f64::MAX, f64::MAX],
            [-3.0e38, 3.0e38],
            [1.0, 1.0 + f64::EPSILON],
        ] {
            let control = Control {
                widget: Widget::Dial,
                range: Some([low, high]),
                ..Control::default()
            };
            assert!(NumberRange::of_input(&control).is_none());
        }
    }
}

#[cfg(test)]
mod width_tests {
    use super::*;

    #[test]
    fn the_white_properties_title_edits_inline_on_double_click() {
        let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        let node = editor.add_node(&mut host, intrinsic::WHOLE_NUMBER).unwrap();
        assert!(editor.rename_node(&mut host, node, "Sample amount"));
        let context = egui::Context::default();
        context.set_theme(egui::Theme::Dark);
        let mut frame = |editor: &mut Editor, events| {
            context.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    editor.properties_panel(ui, &mut host);
                },
            )
        };
        let output = frame(&mut editor, vec![]);
        let title = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Sample amount" => Some(text),
                _ => None,
            })
            .unwrap();
        assert!(
            title
                .galley
                .job
                .sections
                .iter()
                .all(|section| section.format.color == egui::Color32::WHITE)
        );
        let at = title.pos + title.galley.rect.center().to_vec2();
        let baseline = title.pos.y;
        output.drop_without_applying_deltas();
        for expected in [None, Some((node, "Sample amount".to_owned()))] {
            orange_testkit::ui::click(at, egui::PointerButton::Primary, |events| {
                frame(&mut editor, events).drop_without_applying_deltas();
            });
            assert_eq!(
                editor
                    .renaming_node
                    .as_ref()
                    .map(|rename| (rename.node, rename.name.clone())),
                expected
            );
        }
        let editing = frame(&mut editor, vec![]);
        let editing_baseline = editing
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Sample amount" => {
                    Some(text.pos.y)
                }
                _ => None,
            })
            .unwrap();
        assert!((editing_baseline - baseline).abs() <= 1.0);
        editing.drop_without_applying_deltas();
        let key = |key| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        for (typed, finish, expected) in [
            ("Discard this", egui::Key::Escape, "Sample amount"),
            ("Updated amount", egui::Key::Enter, "Updated amount"),
        ] {
            if editor.renaming_node.is_none() {
                assert!(editor.begin_node_rename(node));
            }
            frame(&mut editor, vec![]).drop_without_applying_deltas();
            frame(&mut editor, vec![egui::Event::Text(typed.into())])
                .drop_without_applying_deltas();
            assert_eq!(editor.renaming_node.as_ref().unwrap().name, typed);
            frame(&mut editor, vec![key(finish)]).drop_without_applying_deltas();
            assert!(editor.renaming_node.is_none());
            assert_eq!(editor.graph().unwrap().nodes[0].name, expected);
        }
    }

    #[test]
    fn a_value_field_with_its_clear_button_does_not_widen_a_resizable_panel() {
        let context = egui::Context::default();
        let mut typed = "12345".to_owned();
        let mut widths = Vec::new();
        for _ in 0..8 {
            let output = context.run_ui(egui::RawInput::default(), |ui| {
                let shown = egui::Panel::right("width-test")
                    .resizable(true)
                    .default_size(200.0)
                    .show(ui, |ui| {
                        ui.set_max_width(ui.available_width());
                        row(ui, "Value", None, |ui| {
                            let _ = text_field(ui, &mut typed, "a number", true, None);
                        });
                    });
                widths.push(shown.response.rect.width());
            });
            output.drop_without_applying_deltas();
        }
        assert!(
            widths
                .windows(2)
                .all(|pair| (pair[0] - pair[1]).abs() < 0.5),
            "{widths:?}"
        );
    }

    /// A node whose input holds a very long path keeps the whole panel at the width it
    /// was given: every line is cut short, none pushes the edge out.
    #[test]
    fn a_long_value_never_widens_the_properties_panel() {
        let (mut host, _) = Host::with_services(
            orange_host::HostOptions::default(),
            vec![orange_audio::package()],
        )
        .unwrap();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        let play = host
            .add_node(&mut draft, orange_audio::PLAY, [0.0, 0.0])
            .unwrap();
        let long = format!(r"C:\{}\sound.wav", "a very long folder name\\".repeat(12));
        host.set_input(&mut draft, (play, "path"), Some(&long))
            .unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut host, draft.text().unwrap());
        editor.canvas().select(Some(play));
        // And a run showed the same long value on the input.
        editor.canvas().show_observed_value(
            play,
            "path".into(),
            canvas::Shown {
                short: long.clone(),
                full: long.clone(),
            },
            &orange_host::Value::Str(std::sync::Arc::new(long)),
            true,
        );
        let context = egui::Context::default();
        let mut widths = Vec::new();
        for _ in 0..6 {
            let output = context.run_ui(egui::RawInput::default(), |ui| {
                let shown = egui::Panel::right("long-value-test")
                    .resizable(true)
                    .default_size(260.0)
                    .show(ui, |ui| editor.properties_panel(ui, &mut host));
                widths.push(shown.response.rect.width());
            });
            output.drop_without_applying_deltas();
        }
        assert!(widths.iter().all(|width| *width <= 262.0), "{widths:?}");
    }

    #[test]
    fn long_http_inputs_and_outputs_wrap_inside_the_open_properties_panel() {
        let (mut host, _) = Host::with_services(
            orange_host::HostOptions::default(),
            vec![orange_net::package()],
        )
        .unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        let request = editor
            .add_node(&mut host, orange_net::HTTP_REQUEST)
            .unwrap();
        editor.canvas().select(Some(request));
        let body = json!({"payload": "x".repeat(8192)}).to_string();
        let input = TextInput::Body.literal(&body);
        assert!(editor.set_input(&mut host, (request, "content"), Some(&input)));
        editor.canvas().show_observed_value(
            request,
            "response".into(),
            canvas::Shown {
                short: body.clone(),
                full: body.clone(),
            },
            &orange_host::Value::Str(std::sync::Arc::new(body)),
            false,
        );
        let ctx = egui::Context::default();
        for _ in 0..3 {
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let dock = egui::Panel::right("long-response-test")
                    .resizable(true)
                    .default_size(260.0)
                    .show(ui, |ui| editor.properties_panel(ui, &mut host));
                assert!(
                    dock.response.rect.width() <= 262.0,
                    "{:?}",
                    dock.response.rect
                );
            });
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn editing_a_request_reads_the_full_literal_instead_of_the_canvas_preview() {
        let (mut host, _) = Host::with_services(
            orange_host::HostOptions::default(),
            vec![orange_net::package()],
        )
        .unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        let request = editor
            .add_node(&mut host, orange_net::HTTP_REQUEST)
            .unwrap();
        let body = format!("{{\n  \"message\": \"{}\"\n}}\n", "a".repeat(200));
        let headers = "Accept: application/json\nX-Client: Orange";
        for (port, value) in [("content", body.as_str()), ("headers", headers)] {
            let literal = json!({"type": "core/string@1", "value": value}).to_string();
            assert!(editor.set_input(&mut host, (request, port), Some(&literal)));
        }
        let drawn = editor
            .graph()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.id == request)
            .unwrap()
            .clone();
        assert_ne!(
            Editor::shown_input(
                drawn
                    .ports
                    .iter()
                    .find(|port| port.id == "content")
                    .unwrap()
            ),
            body
        );
        editor.inspect(&drawn, &host);
        assert_eq!(editor.typed["content"], body);
        assert_eq!(editor.typed["headers"], headers);
    }
}

impl Editor {
    pub(crate) fn variable_inspector_window(&mut self, ctx: &egui::Context) {
        let Some(id) = &self.inspected_variable else {
            return;
        };
        let Some(document) = &self.document else {
            return;
        };
        let Some(variable) = document
            .declared
            .variables
            .iter()
            .find(|var| var.id.as_ref() == Some(id))
        else {
            self.inspected_variable = None;
            return;
        };
        let mut open = true;
        orange_ui_controls::dialog_window(ctx, &variable.name)
            .id(egui::Id::unique("orange-variable-inspector"))
            .default_width(480.0)
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.weak(type_name(&variable.ty));
                if let Some(value) = document.values.get(id) {
                    ui.weak(t(if document.flow.is_some() {
                        "value-now-hint"
                    } else {
                        "value-last-run-hint"
                    }));
                    let full = match &value.exact_text {
                        Some(orange_host::Value::Str(text)) => text.as_str(),
                        Some(orange_host::Value::Json(text)) => text.as_str(),
                        _ => &value.shown,
                    };
                    display_port_text(ui, &value.shown, full, 300.0);
                } else {
                    ui.weak(t("no-port-value"));
                }
            });
        if !open {
            self.inspected_variable = None;
        }
    }

    pub(crate) fn port_inspector_window(&mut self, ctx: &egui::Context) {
        let Some((node, port, input)) = &self.inspected_port else {
            return;
        };
        let Some(graph) = self.graph() else { return };
        let Some(view) = graph.nodes.iter().find(|view| view.id == *node) else {
            self.inspected_port = None;
            return;
        };
        let sample = self.canvas.port_sample(graph, *node, port, *input);
        let copyable = self.canvas.copyable_port_text(graph, *node, port, *input);
        let selected_port = view
            .ports
            .iter()
            .find(|candidate| candidate.id == *port && candidate.input == *input);
        let label = selected_port.map_or_else(
            || port_label(port),
            |candidate| {
                if *input {
                    input_caption(view, candidate)
                } else {
                    candidate.label.clone().unwrap_or_else(|| port_label(port))
                }
            },
        );
        let title = format!("{} · {label}", self.canvas.display_name(view));
        let mut open = true;
        orange_ui_controls::dialog_window(ctx, title)
            .id(egui::Id::unique("orange-port-inspector"))
            .default_width(480.0)
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                if let Some(ty) = selected_port.and_then(|port| port.ty.as_deref()) {
                    ui.weak(type_name(ty));
                }
                if let (Some((shown, stale)), Some(copyable)) = (sample, copyable) {
                    if stale {
                        ui.weak(t("previous-port-value"));
                    }
                    display_port_text(ui, &shown.full, copyable, 300.0);
                } else {
                    ui.weak(t("no-port-value"));
                }
            });
        if !open {
            self.inspected_port = None;
        }
    }

    /// The last observed values and any failure, with copyable result text.
    /// What the node gives, with the type of each, so a person can tell what to wire
    /// where before anything has run.
    fn outputs_section(ui: &mut egui::Ui, node: &NodeView) {
        let outputs: Vec<_> = node
            .ports
            .iter()
            .filter(|port| !port.input && !port.exec && !palette::bridge_link_port(node, port))
            .collect();
        if outputs.is_empty() {
            return;
        }
        section(ui, t("outputs"));
        for port in outputs {
            let caption = node_ergonomics::loop_result(node, port)
                .or_else(|| specialized_port_caption(&node.definition, port))
                .map(str::to_owned)
                .or_else(|| port.label.clone())
                .unwrap_or_else(|| port_label(&port.id));
            let ty = port.ty.as_deref().unwrap_or_default();
            ui.horizontal(|ui| {
                let _ = ui.add(egui::Label::new(&caption).truncate());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let _ = ui
                        .label(
                            egui::RichText::new(canvas::short_type(ty))
                                .small()
                                .color(canvas::type_color(Some(ty))),
                        )
                        .on_hover_text(exact_type(ty));
                });
            });
        }
    }

    fn values_section(&self, ui: &mut egui::Ui, node: &NodeView) {
        let Some(graph) = self.graph() else {
            return;
        };
        let http = http_request_variant(node);
        let failure = self.canvas.failure(node.id);
        let ports: Vec<_> = node
            .ports
            .iter()
            .filter(|port| {
                !port.exec && !canvas::stream_type(port.ty.as_deref()) && (!http || !port.input)
            })
            .filter_map(|port| {
                let (shown, stale) = self
                    .canvas
                    .port_sample(graph, node.id, &port.id, port.input)?;
                Some((port, shown, stale))
            })
            .collect();
        if failure.is_none() && ports.is_empty() {
            return;
        }
        section(ui, if http { t("http-results") } else { t("values") });
        if let Some(why) = failure {
            orange_ui_controls::notice(
                ui,
                controls::ButtonTone::Danger,
                tf("failed", &[("why", why.to_string().into())]),
            );
        }
        for (port, shown, stale) in ports {
            let caption = node_ergonomics::loop_result(node, port)
                .or_else(|| specialized_port_caption(&node.definition, port))
                .map(str::to_owned)
                .or_else(|| port.label.clone())
                .unwrap_or_else(|| port_label(&port.id));
            let side = if port.input {
                t("value-side-input")
            } else {
                t("value-side-output")
            };
            let freshness = if stale { t("previous-suffix") } else { "" };
            let response = http && !port.input && port.id == "response";
            let title = if response {
                format!("{caption}{freshness}")
            } else if shown.short == shown.full
                && shown.short.chars().take(17).count() <= 16
                && !shown.short.contains(['\n', '\r'])
            {
                format!("{caption} ({side}) = {}{freshness}", shown.short)
            } else {
                format!("{caption} ({side}){freshness}")
            };
            let id = ui.make_persistent_id(("orange-value", node.id, port.id.as_str(), port.input));
            let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                id,
                response,
            );
            let row = orange_ui_controls::navigation::sidebar_row(
                ui,
                &title,
                false,
                Some(state.is_open()),
                |ui, rect, ink| {
                    icons::paint_centered(ui.painter(), rect.center(), icons::WATCH, 15.0, ink);
                },
            )
            .on_hover_text(
                self.canvas
                    .port_details(node.id, &port.id, port.input)
                    .and_then(|detail| detail.description.as_deref())
                    .map_or(title.clone(), |help| format!("{title}\n{help}")),
            );
            if row.clicked() {
                state.toggle(ui);
                state.store(ui.ctx());
            }
            if state.is_open() {
                let copyable = self
                    .canvas
                    .copyable_port_text(graph, node.id, &port.id, port.input)
                    .unwrap_or(&shown.full);
                display_port_text(ui, &shown.full, copyable, 126.0);
            }
        }
    }
}

fn wrap_property_text(
    ui: &egui::Ui,
    text: &dyn egui::TextBuffer,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let mut job = egui::text::LayoutJob::simple(
        text.as_str().to_owned(),
        font,
        ui.visuals().text_color(),
        width,
    );
    job.wrap.break_anywhere = true;
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}

fn display_port_text(ui: &mut egui::Ui, mut preview: &str, copyable: &str, height: f32) {
    if ui.small_button(t("copy")).clicked() {
        ui.ctx().copy_text(copyable.to_owned());
    }
    if preview != copyable {
        ui.weak(t("port-preview-limited"));
    }
    egui::ScrollArea::vertical()
        .max_height(height)
        .show(ui, |ui| {
            let _ = ui.add(
                egui::TextEdit::multiline(&mut preview)
                    .font(egui::TextStyle::Monospace)
                    .layouter(&mut wrap_property_text)
                    .desired_rows(1)
                    .desired_width(ui.available_width()),
            );
        });
}
