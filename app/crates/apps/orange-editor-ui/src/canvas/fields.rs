//! The controls of a card's fields (22 §3.1): one look per kind of value, the same for
//! an input and a setting. True or false is a No | Yes switch, a choice a dropdown, a
//! number a field dragged to step and clicked to type, text a field clicked to type, a
//! path a field that opens the system's chooser.
use super::card::FieldOf;
use super::settings;
use super::*;
use crate::controls::{Dropdown as _, Surface};
use crate::properties::{
    choose_http_method, http_request_variant, input_label, watch_label, watch_number_value,
};

/// What a field changed.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum FieldEdit {
    /// An input's value, typed as a person types it.
    Input(String, String),
    /// A setting at a JSON pointer.
    Setting(String, serde_json::Value),
    /// "New variable…" was chosen for the setting at a JSON pointer.
    NewVariable(String),
    /// A device was chosen; `None` is the system default.
    Device(Option<String>),
    /// The node's connection is to be set up, from here on the screen.
    Configure(Pos2),
    /// The system's chooser is to pick the input's path.
    Browse(String),
    /// One more instance of this variadic port.
    AddPort(String),
    /// The devices of this adapter are to be listed again.
    RefreshDevices(String),
}

/// The kind of control a value gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Switch,
    Choice,
    Whole,
    Number,
    Text,
}

impl Kind {
    pub(super) fn of_port(port: &PortView, enums: &Enums) -> Self {
        let ty = port.ty.as_deref().unwrap_or_default();
        if enums.contains_key(ty) {
            return Self::Choice;
        }
        match ty {
            "core/bool@1" => Self::Switch,
            "core/i64@1" | "core/u64@1" | "core/i32@1" | "core/u32@1" => Self::Whole,
            "core/f64@1" | "core/f32@1" => Self::Number,
            _ => Self::Text,
        }
    }

    fn of_setting(schema: &serde_json::Value) -> Self {
        if schema["enum"].is_array() {
            Self::Choice
        } else {
            match schema["type"].as_str() {
                Some("boolean") => Self::Switch,
                Some("integer") => Self::Whole,
                Some("number") => Self::Number,
                _ => Self::Text,
            }
        }
    }
}

/// The unit a setting is measured in, from its name.
fn unit(name: &str) -> &'static str {
    if name.ends_with("_hz") {
        "Hz"
    } else if name == "reset_db" {
        "dB"
    } else if name.ends_with("_db") {
        "dBFS"
    } else if name.ends_with("_ms") || matches!(name, "hold" | "cooldown") {
        "ms"
    } else {
        ""
    }
}

/// A setting's current value: what the node holds, else its schema's default.
fn setting_value(node: &NodeView, name: &str, schema: &serde_json::Value) -> serde_json::Value {
    node.settings
        .get(name)
        .cloned()
        .unwrap_or_else(|| schema["default"].clone())
}

fn pointer_of(name: &str) -> String {
    format!("/{}", name.replace('~', "~0").replace('/', "~1"))
}

/// A choice as a person reads it: When's modes in words, others as declared.
fn choice_label<'a>(node: &NodeView, name: &str, choice: &'a str) -> &'a str {
    if name == "direction" && spectrum::is_noise(node) {
        spectrum::direction(choice)
    } else {
        crate::properties::setting_choice_caption(node, choice)
    }
}

/// The text a field shows while nothing edits it, and whether it is a default.
pub(super) fn shown(node: &NodeView, field: FieldOf<'_>) -> (String, bool) {
    match field {
        FieldOf::TextArea(port, kind, _) => {
            let text = port
                .value
                .as_deref()
                .or(port.default.as_deref())
                .map(|text| crate::properties::text_area_preview(text, kind))
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| kind.hint().to_owned());
            (text, port.value.is_none())
        }
        FieldOf::Input(port) => match (&port.value, &port.default) {
            (Some(value), _) => (
                crate::properties::choice_caption(port, &field_text(value)).into_owned(),
                false,
            ),
            (None, Some(default)) => (
                crate::properties::choice_caption(port, &field_text(default)).into_owned(),
                true,
            ),
            (None, None) => ("-".to_owned(), true),
        },
        FieldOf::Path(port, path) => port
            .value
            .as_deref()
            .map(field_text)
            .filter(|value| !value.is_empty())
            .map_or_else(
                || {
                    let ask = if path.folder {
                        t("choose-folder")
                    } else {
                        t("choose-file")
                    };
                    (ask.to_owned(), true)
                },
                |value| {
                    if node.definition.starts_with("org.orange.files/") {
                        return (value, false);
                    }
                    let name = std::path::Path::new(&value)
                        .file_name()
                        .map_or(value.clone(), |name| name.to_string_lossy().into_owned());
                    (name, false)
                },
            ),
        FieldOf::Setting(name, schema) => {
            let value = setting_value(node, name, schema);
            let set = node.settings.get(name).is_some();
            let text = match &value {
                serde_json::Value::Null => "—".to_owned(),
                serde_json::Value::String(text) => choice_label(node, name, text).to_owned(),
                other => {
                    let unit = unit(name);
                    if unit.is_empty() {
                        other.to_string()
                    } else {
                        format!("{other} {unit}")
                    }
                }
            };
            (text, !set)
        }
        FieldOf::Variable(_) => node.detail.clone().map_or_else(
            || (tf("choose-reference", &[("kind", "variable".into())]), true),
            |name| (name, false),
        ),
        FieldOf::Connection => (t("choose-connection").to_owned(), true),
        FieldOf::AddPort(variadic) => (add_port_text(node, variadic), true),
    }
}

fn add_port_text(node: &NodeView, variadic: &orange_host::VariadicView) -> String {
    if node.definition == "org.orange.logging/log-message" && variadic.port == "argument" {
        return t("log-add-part").to_owned();
    }
    tf(
        "add-port",
        &[("name", crate::port_label(&variadic.port).into())],
    )
}

/// The name line's text of a field.
pub(super) fn name(node: &NodeView, field: FieldOf<'_>, connection: Option<&str>) -> String {
    match field {
        FieldOf::Input(port) | FieldOf::TextArea(port, ..) | FieldOf::Path(port, _) => {
            crate::properties::log_part_label(node, &port.id).unwrap_or_else(|| {
                input_label(node, &port.id).map_or_else(|| port_title(port), str::to_owned)
            })
        }
        FieldOf::Setting(name, schema) => {
            if node.definition == "core/on-watch" {
                watch_label(name).to_owned()
            } else if let Some(label) = crate::properties::noise_gate_label(node, name) {
                label.to_owned()
            } else {
                settings::label(name, schema)
            }
        }
        FieldOf::Variable(_) => t("variable").to_owned(),
        FieldOf::Connection => connection.unwrap_or(t("connection")).to_owned(),
        FieldOf::AddPort(variadic) => add_port_text(node, variadic),
    }
}

/// Paints a field's frame: the canvas's field background and a thin outline, the
/// selection color while the pointer is on it.
pub(super) fn frame(painter: &egui::Painter, rect: Rect, zoom: f32, colors: &Palette, hot: bool) {
    painter.rect_filled(rect, 5.0 * zoom, colors.background);
    painter.rect_stroke(
        rect,
        5.0 * zoom,
        Stroke::new(1.0, if hot { colors.selected } else { colors.outline }),
        StrokeKind::Inside,
    );
}

/// Paints `text` inside a field, left-aligned and cut with an ellipsis to fit, with an
/// optional dim `suffix` (a unit or a ▾) at the right end.
pub(super) fn field_text_in(
    painter: &egui::Painter,
    rect: Rect,
    zoom: f32,
    text: &str,
    ink: Color32,
    suffix: Option<(&str, Color32, FontId)>,
) -> bool {
    let font = FontId::proportional(12.5 * zoom);
    let pad = 8.0 * zoom;
    let mut right = rect.right() - pad;
    if let Some((suffix, color, font)) = suffix {
        let drawn = painter.text(
            Pos2::new(right, rect.center().y),
            Align2::RIGHT_CENTER,
            suffix,
            font,
            color,
        );
        right = drawn.left() - 6.0 * zoom;
    }
    let (text, cut) = elide(painter, text, &font, (right - rect.left() - pad).max(0.0));
    let _ = painter.text(
        Pos2::new(rect.left() + pad, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        font,
        ink,
    );
    cut
}

/// Paints a No | Yes switch with `on` chosen; `None` chooses neither. `dim` draws the
/// chosen half paler, for a default. Returns each half's rectangle.
pub(super) fn switch(
    painter: &egui::Painter,
    rect: Rect,
    zoom: f32,
    colors: &Palette,
    (on, dim): (Option<bool>, bool),
    hovered: Option<bool>,
) -> [Rect; 2] {
    let (no, yes) = rect.split_left_right_at_fraction(0.5);
    frame(painter, rect, zoom, colors, false);
    let font = FontId::proportional(12.5 * zoom);
    for (half, value, label) in [(no, false, t("no")), (yes, true, t("yes"))] {
        let chosen = on == Some(value);
        let inner = half.shrink(2.0 * zoom);
        if chosen {
            let fill = if dim {
                colors.selected.gamma_multiply(0.45)
            } else {
                colors.selected
            };
            painter.rect_filled(inner, 4.0 * zoom, fill);
        } else if hovered == Some(value) {
            painter.rect_filled(inner, 4.0 * zoom, colors.outline.gamma_multiply(0.6));
        }
        let ink = if chosen { colors.card } else { colors.text };
        let _ = painter.text(
            inner.center(),
            Align2::CENTER_CENTER,
            label,
            font.clone(),
            ink,
        );
    }
    [no, yes]
}

/// Paints a field as it reads when nothing edits it: for a card too small for
/// controls, or a card being dragged from the palette.
pub(super) fn paint_still(
    painter: &egui::Painter,
    node: &NodeView,
    field: FieldOf<'_>,
    rect: Rect,
    zoom: f32,
    colors: &Palette,
    profile: Option<&str>,
) {
    let switch_value = match field {
        FieldOf::Input(port) if port.ty.as_deref() == Some("core/bool@1") => Some((
            port.value
                .as_deref()
                .or(port.default.as_deref())
                .map(|v| v == "true"),
            port.value.is_none(),
        )),
        FieldOf::Setting(name, schema) if schema["type"] == "boolean" => Some((
            setting_value(node, name, schema).as_bool(),
            node.settings.get(name).is_none(),
        )),
        _ => None,
    };
    if let Some(value) = switch_value {
        let _ = switch(painter, rect, zoom, colors, value, None);
        return;
    }
    frame(painter, rect, zoom, colors, false);
    let (text, dim) = match (field, profile) {
        (FieldOf::Connection, Some(name)) => (name.to_owned(), false),
        _ => shown(node, field),
    };
    let _ = field_text_in(
        painter,
        rect,
        zoom,
        &text,
        if dim { colors.weak } else { colors.text },
        None,
    );
}

impl Canvas {
    /// The control of one field of a card, in `slot`. Returns what it changed.
    #[expect(
        clippy::too_many_arguments,
        reason = "field geometry and host input access"
    )]
    pub(super) fn field(
        &mut self,
        ui: &mut egui::Ui,
        node: &NodeView,
        field: FieldOf<'_>,
        slot: Rect,
        id: egui::Id,
        file_dialogs: bool,
        stored_text: Option<&StoredText<'_>>,
    ) -> Option<FieldEdit> {
        let zoom = slot.height() / field.control_height();
        let colors = Palette::of(ui);
        match field {
            FieldOf::TextArea(port, kind, height) => {
                let (preview, dim) = shown(node, field);
                if stored_text.is_none() && port.ty.as_deref() == Some("core/string@1") {
                    return self
                        .text_control(
                            ui,
                            (node.id, &port.id),
                            slot,
                            id,
                            &preview,
                            dim,
                            zoom,
                            port.value.as_deref().map(field_text),
                        )
                        .map(|text| FieldEdit::Input(port.id.clone(), text));
                }
                let read = || stored_text.and_then(|read| read(node.id, &port.id));
                let grip = Rect::from_min_max(
                    Pos2::new(slot.left(), slot.bottom() - 9.0 * zoom),
                    slot.max,
                );
                let resized = crate::properties::text_resize_grip(ui, grip, id.with("resize"));
                if resized != 0.0 {
                    self.text_heights.entry(node.id).or_default().insert(
                        port.id.clone(),
                        (height + resized / zoom).clamp(60.0, 280.0),
                    );
                    ui.ctx().request_repaint();
                }
                let slot = Rect::from_min_max(slot.min, Pos2::new(slot.right(), grip.top()));
                if let Some(edit) = self
                    .expanded_text
                    .as_mut()
                    .filter(|edit| edit.node == node.id && edit.port == port.id)
                {
                    let editor_id = id.with("request-input");
                    // egui drops focus on Escape before the widget is drawn.
                    let cancel = ui.input(|input| input.key_pressed(egui::Key::Escape));
                    let save = ui.memory(|memory| memory.has_focus(editor_id))
                        && ui.input_mut(|input| {
                            input.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)
                        });
                    let edited = multiline_field(
                        ui,
                        slot,
                        editor_id,
                        &mut edit.value,
                        kind.hint(),
                        zoom,
                        &colors,
                    );
                    if !edit.focused {
                        edited.request_focus();
                        edit.focused = true;
                    }
                    if save || cancel || edited.lost_focus() {
                        edited.surrender_focus();
                        let text = self.expanded_text.take()?.value;
                        ui.ctx().request_repaint();
                        if !cancel && text != read().unwrap_or_default() {
                            return Some(FieldEdit::Input(port.id.clone(), kind.literal(&text)));
                        }
                    }
                } else {
                    let response = ui
                        .interact(slot, id, Sense::click())
                        .on_hover_cursor(egui::CursorIcon::Text)
                        .on_hover_text(
                            if kind == crate::properties::TextInput::LogMessage
                                && node.variadic.iter().any(|port| port.port == "argument")
                            {
                                t("log-format-hint")
                            } else {
                                t("edit-text-hint")
                            },
                        );
                    frame(ui.painter(), slot, zoom, &colors, response.hovered());
                    let content = slot.shrink2(Vec2::new(6.0, 4.0) * zoom);
                    let tint = if dim { colors.weak } else { colors.text };
                    let galley = ui.painter().layout(
                        preview,
                        FontId::monospace(12.5 * zoom),
                        tint,
                        content.width(),
                    );
                    ui.painter()
                        .with_clip_rect(content.intersect(ui.clip_rect()))
                        .galley(content.min, galley, tint);
                    if response.clicked() {
                        if stored_text.is_none() {
                            return Some(FieldEdit::Configure(slot.left_bottom()));
                        }
                        self.front = Some(node.id);
                        self.expanded_text = Some(ExpandedText {
                            node: node.id,
                            port: port.id.clone(),
                            value: read().unwrap_or_default(),
                            focused: false,
                        });
                        ui.ctx().request_repaint();
                    }
                }
                None
            }
            FieldOf::Input(port) => {
                let input = |text: String| Some(FieldEdit::Input(port.id.clone(), text));
                if port.ty.as_deref() == Some(orange_host::RGBA_TYPE) {
                    let color = crate::properties::input_color(port);
                    let response = ui
                        .interact(slot, id, Sense::click())
                        .on_hover_text(t("color-picker"));
                    frame(ui.painter(), slot, zoom, &colors, response.hovered());
                    let chip = slot.shrink(4.0 * zoom);
                    let side = chip.height().min(18.0 * zoom);
                    let chip = Rect::from_center_size(
                        Pos2::new(chip.left() + side * 0.5, chip.center().y),
                        Vec2::splat(side),
                    );
                    orange_ui_controls::color_picker::swatch(ui, chip, color);
                    let label = Rect::from_min_max(
                        Pos2::new(chip.right() + 2.0 * zoom, slot.top()),
                        slot.max,
                    );
                    let _ = field_text_in(
                        ui.painter(),
                        label,
                        zoom,
                        &orange_host::rgba_hex(color),
                        colors.text,
                        None,
                    );
                    return crate::properties::pick_color(ui, &response, color).and_then(input);
                }
                if http_request_variant(node) && port.id == "method" {
                    let (text, _) = shown(node, field);
                    let mut chosen = None;
                    dropdown(ui, slot, id, &text, false, |ui| {
                        chosen = choose_http_method(ui, &text);
                    });
                    return chosen.and_then(|method| input(method.to_owned()));
                }
                match Kind::of_port(port, &self.enums) {
                    Kind::Switch => {
                        let value = port.value.as_deref().or(port.default.as_deref());
                        let on = value.map(|value| value == "true");
                        self.switch_control(ui, slot, id, (on, port.value.is_none()))
                            .and_then(|on| input(on.to_string()))
                    }
                    Kind::Choice => {
                        let choices = self.enums.get(port.ty.as_deref()?)?.clone();
                        let (text, dim) = shown(node, field);
                        let mut chosen = None;
                        dropdown(ui, slot, id, &text, dim, |ui| {
                            for choice in choices.iter().filter(|choice| !choice.payload) {
                                let current = port.value.as_deref().or(port.default.as_deref())
                                    == Some(choice.name.as_str());
                                if ui
                                    .selectable_label(
                                        current,
                                        crate::properties::choice_caption(port, &choice.name),
                                    )
                                    .clicked()
                                    && !current
                                {
                                    chosen = Some(choice.literal.clone());
                                }
                            }
                        });
                        chosen.and_then(input)
                    }
                    kind @ (Kind::Whole | Kind::Number) => {
                        let whole = kind == Kind::Whole;
                        let ty = port.ty.clone().unwrap_or_default();
                        self.number_control(
                            ui,
                            (node.id, &port.id),
                            slot,
                            id,
                            (port.value.as_deref(), port.default.as_deref()),
                            (whole, &ty, ""),
                        )
                        .and_then(input)
                    }
                    Kind::Text => {
                        let current = port.value.as_deref().map(field_text);
                        let (text, dim) = shown(node, field);
                        self.text_control(
                            ui,
                            (node.id, &port.id),
                            slot,
                            id,
                            &text,
                            dim,
                            zoom,
                            current,
                        )
                        .and_then(input)
                    }
                }
            }
            FieldOf::Path(port, _) if file_dialogs => {
                let (text, dim) = shown(node, field);
                let touched = ui
                    .interact(slot, id, Sense::click())
                    .on_hover_text(port.value.as_deref().map_or(t("choose-path"), |v| v));
                let painter = ui.painter();
                frame(painter, slot, zoom, &colors, touched.hovered());
                let _ = field_text_in(
                    painter,
                    slot,
                    zoom,
                    &text,
                    if dim { colors.weak } else { colors.text },
                    Some((
                        crate::icons::OPEN.to_string().as_str(),
                        colors.weak,
                        FontId::new(15.0 * zoom, crate::icons::family()),
                    )),
                );
                touched
                    .clicked()
                    .then(|| FieldEdit::Browse(port.id.clone()))
            }
            FieldOf::Path(port, _) => {
                let current = port.value.as_deref().map(field_text);
                let (text, dim) = shown(node, field);
                self.text_control(ui, (node.id, &port.id), slot, id, &text, dim, zoom, current)
                    .map(|text| FieldEdit::Input(port.id.clone(), text))
            }
            FieldOf::Setting(name, schema) => {
                let value = setting_value(node, name, schema);
                let set =
                    |value: serde_json::Value| Some(FieldEdit::Setting(pointer_of(name), value));
                match Kind::of_setting(schema) {
                    Kind::Switch => self
                        .switch_control(
                            ui,
                            slot,
                            id,
                            (value.as_bool(), node.settings.get(name).is_none()),
                        )
                        .and_then(|on| set(on.into())),
                    Kind::Choice => {
                        let options = schema["enum"].as_array().cloned().unwrap_or_default();
                        let (text, dim) = shown(node, field);
                        let mut chosen = None;
                        dropdown(ui, slot, id, &text, dim, |ui| {
                            for option in &options {
                                let label = option.as_str().map_or_else(
                                    || option.to_string(),
                                    |o| choice_label(node, name, o).to_owned(),
                                );
                                if ui.selectable_label(*option == value, label).clicked()
                                    && *option != value
                                {
                                    chosen = Some(option.clone());
                                }
                            }
                        });
                        chosen.and_then(set)
                    }
                    kind @ (Kind::Whole | Kind::Number) => {
                        let whole = kind == Kind::Whole;
                        let current = (!value.is_null()).then(|| value.to_string());
                        let key = pointer_of(name);
                        let ty = if whole { "core/i64@1" } else { "core/f64@1" };
                        let typed = self.number_control(
                            ui,
                            (node.id, &key),
                            slot,
                            id,
                            (current.as_deref(), None),
                            (whole, ty, unit(name)),
                        )?;
                        setting_number(node, name, schema, &typed).and_then(set)
                    }
                    Kind::Text => {
                        let current = value.as_str().map(str::to_owned);
                        let (text, dim) = shown(node, field);
                        let key = pointer_of(name);
                        self.text_control(ui, (node.id, &key), slot, id, &text, dim, zoom, current)
                            .and_then(|text| set(text.into()))
                    }
                }
            }
            FieldOf::Variable(pointer) => {
                let named = node
                    .settings
                    .pointer(pointer)
                    .and_then(serde_json::Value::as_str);
                let variables = &self.variables;
                let (text, dim) = variables
                    .iter()
                    .find(|(id, _)| named == Some(id.as_str()))
                    .map_or_else(|| shown(node, field), |(_, name)| (name.clone(), false));
                let mut edit = None;
                dropdown(ui, slot, id, &text, dim, |ui| {
                    for (variable, name) in variables {
                        let current = named == Some(variable.as_str());
                        if ui.selectable_label(current, name.as_str()).clicked() && !current {
                            edit = Some(FieldEdit::Setting(
                                pointer.to_owned(),
                                variable.as_str().into(),
                            ));
                        }
                    }
                    if !variables.is_empty() {
                        ui.separator();
                    }
                    if ui
                        .selectable_label(
                            false,
                            tf("new-reference", &[("kind", "variable".into())]),
                        )
                        .clicked()
                    {
                        edit = Some(FieldEdit::NewVariable(pointer.to_owned()));
                    }
                });
                edit
            }
            FieldOf::Connection => self.connection_control(ui, node, slot, id),
            FieldOf::AddPort(variadic) => {
                // A quiet button across the card: "+ Add branch".
                let label = add_port_text(node, variadic);
                let touched = ui
                    .interact(slot, id, Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                touched.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &label)
                });
                let painter = ui.painter();
                if touched.hovered() {
                    painter.rect_filled(slot, 5.0 * zoom, colors.background);
                }
                painter.rect_stroke(
                    slot,
                    5.0 * zoom,
                    Stroke::new(1.0, colors.outline),
                    StrokeKind::Inside,
                );
                let label =
                    painter.layout_no_wrap(label, FontId::proportional(12.0 * zoom), colors.text);
                let center = slot.center() - Vec2::new(label.size().x / 2.0, 0.0);
                crate::icons::paint_centered(
                    painter,
                    center - Vec2::new(8.0 * zoom, 0.0),
                    crate::icons::ADD,
                    14.0 * zoom,
                    colors.text,
                );
                painter.galley(
                    center + Vec2::new(3.0 * zoom, -label.size().y / 2.0),
                    label,
                    colors.text,
                );
                touched
                    .clicked()
                    .then(|| FieldEdit::AddPort(variadic.port.clone()))
            }
        }
    }

    /// A device selector for a device node, or a selector of saved profiles.
    fn connection_control(
        &self,
        ui: &mut egui::Ui,
        node: &NodeView,
        slot: Rect,
        id: egui::Id,
    ) -> Option<FieldEdit> {
        let Some(picker) = self.device_pickers.get(&node.id) else {
            let pointer = &node
                .references
                .iter()
                .find(|(_, kind)| *kind == orange_host::ReferenceKind::Connection)?
                .0;
            let profiles = self.profile_choices(node);
            let selected = node
                .settings
                .pointer(pointer)
                .and_then(serde_json::Value::as_str);
            let label = self.connection_selection(node);
            let mut edit = None;
            dropdown(
                ui,
                slot,
                id,
                label.unwrap_or_else(|| t("choose-connection")),
                label.is_none(),
                |ui| {
                    if profiles.is_empty() {
                        ui.weak(t("no-compatible-connections"));
                    }
                    for (id, name) in profiles {
                        let current = Some(id.as_str()) == selected;
                        if ui.selectable_label(current, name).clicked() && !current {
                            edit = Some(FieldEdit::Setting(pointer.clone(), serde_json::json!(id)));
                        }
                    }
                },
            );
            return edit;
        };
        let mut chosen = picker.selected.clone();
        let mut picked = false;
        let mut refresh = false;
        dropdown(ui, slot, id, picker.label(), false, |ui| {
            refresh = ui.small_button(t("refresh-devices")).clicked();
            if let Some(problem) = &picker.problem {
                ui.weak(problem);
            }
            picked |= ui
                .selectable_value(&mut chosen, None, &picker.default_label)
                .clicked();
            ui.separator();
            // Devices, what an output plays, then applications.
            let group = |id: &str| {
                if id.starts_with("loopback:") {
                    1
                } else if id.starts_with("app:") {
                    2
                } else {
                    0
                }
            };
            for kind in 0..3 {
                let options: Vec<_> = picker
                    .options
                    .iter()
                    .filter(|(id, _)| group(id) == kind)
                    .collect();
                if options.is_empty() {
                    if kind == 2
                        && picker.group_label == t("recording-devices")
                        && picker.problem.is_none()
                    {
                        ui.weak(t("no-audio-applications"));
                    }
                    continue;
                }
                ui.small(match kind {
                    1 => t("playback-capture"),
                    2 => t("applications"),
                    _ => &picker.group_label,
                });
                for (value, label) in options {
                    picked |= ui
                        .selectable_value(&mut chosen, Some(value.clone()), label)
                        .clicked();
                }
                ui.separator();
            }
        });
        if refresh {
            return Some(FieldEdit::RefreshDevices(picker.adapter.clone()));
        }
        (picked && (chosen != picker.selected || picker.connection.is_none()))
            .then_some(FieldEdit::Device(chosen))
    }

    /// A No | Yes switch. Returns the value clicked when it differs.
    fn switch_control(
        &mut self,
        ui: &mut egui::Ui,
        slot: Rect,
        id: egui::Id,
        (on, dim): (Option<bool>, bool),
    ) -> Option<bool> {
        let zoom = slot.height() / card::CONTROL;
        let colors = Palette::of(ui);
        let touched = ui.interact(slot, id, Sense::click());
        let side = |at: Pos2| at.x >= slot.center().x;
        let hovered = touched.hover_pos().map(side);
        let _ = switch(ui.painter(), slot, zoom, &colors, (on, dim), hovered);
        touched.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::accesskit::Role::Switch,
                true,
                on == Some(true),
                t("yes"),
            )
        });
        let clicked = touched
            .clicked()
            .then(|| touched.interact_pointer_pos().map(side))
            .flatten()?;
        (on != Some(clicked) || dim).then_some(clicked)
    }

    /// A text field: the value painted in its frame; a click types a new one, kept on
    /// Enter or when the field loses focus.
    #[expect(clippy::too_many_arguments, reason = "one field's whole description")]
    fn text_control(
        &mut self,
        ui: &mut egui::Ui,
        (node, key): (NodeId, &str),
        slot: Rect,
        id: egui::Id,
        text: &str,
        dim: bool,
        zoom: f32,
        current: Option<String>,
    ) -> Option<String> {
        let typing = self
            .typing
            .as_mut()
            .filter(|(at, name, _, _)| *at == node && name == key);
        let Some((_, _, typed, focused)) = typing else {
            let colors = Palette::of(ui);
            let touched = ui.interact(slot, id, Sense::click());
            frame(ui.painter(), slot, zoom, &colors, touched.hovered());
            let cut = field_text_in(
                ui.painter(),
                slot,
                zoom,
                text,
                if dim { colors.weak } else { colors.text },
                None,
            );
            let touched = if cut {
                touched.on_hover_text(text)
            } else {
                touched
            };
            if touched.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
            }
            if touched.clicked() {
                self.typing = Some((node, key.to_owned(), current.unwrap_or_default(), false));
            }
            return None;
        };
        let edited = typing_field(ui, slot, id, typed, false, zoom);
        if !*focused {
            edited.request_focus();
            *focused = true;
        }
        if edited.lost_focus() {
            let text = typed.clone();
            self.typing = None;
            let escaped = ui.input(|input| input.key_pressed(egui::Key::Escape));
            return (!escaped && current.as_deref() != Some(text.as_str())).then_some(text);
        }
        None
    }

    /// A number field: dragged sideways to step it, clicked to type it. Returns the
    /// number as typed.
    fn number_control(
        &mut self,
        ui: &mut egui::Ui,
        (node, key): (NodeId, &str),
        slot: Rect,
        id: egui::Id,
        (value, default): (Option<&str>, Option<&str>),
        (whole, ty, unit): (bool, &str, &str),
    ) -> Option<String> {
        let typing = self
            .typing
            .as_mut()
            .filter(|(at, name, _, _)| *at == node && name == key);
        if let Some((_, _, typed, focused)) = typing {
            let edited = typing_field(ui, slot, id, typed, true, slot.height() / card::CONTROL);
            if !*focused {
                edited.request_focus();
                *focused = true;
            }
            if edited.lost_focus() {
                let text = typed.trim().to_owned();
                self.typing = None;
                let escaped = ui.input(|input| input.key_pressed(egui::Key::Escape));
                return (!escaped)
                    .then(|| number_text(&text, whole, ty))
                    .flatten()
                    .filter(|text| Some(text.as_str()) != value);
            }
            return None;
        }
        let pixels = self
            .number_drag
            .as_ref()
            .filter(|(at, name, _)| *at == node && name == key)
            .map(|(_, _, pixels)| *pixels);
        let shown = match pixels {
            Some(pixels) => Some(stepped(value.or(default), pixels, whole, ty)),
            None => value.map(str::to_owned),
        };
        let touched = number_field(ui, slot, id, shown.as_deref(), default, unit);
        if touched.clicked() {
            self.number_drag = None;
            let typed = value.unwrap_or_default().to_owned();
            self.typing = Some((node, key.to_owned(), typed, false));
            return None;
        }
        if touched.drag_started() {
            self.number_drag = Some((node, key.to_owned(), 0.0));
        }
        if touched.dragged()
            && let Some((_, _, pixels)) = self.number_drag.as_mut()
        {
            *pixels += touched.drag_delta().x;
            return None;
        }
        if touched.drag_stopped() {
            self.number_drag = None;
            return pixels
                .map(|pixels| stepped(value.or(default), pixels, whole, ty))
                .filter(|text| Some(text.as_str()) != value);
        }
        None
    }
}

/// A setting's number as typed, held to its schema; When's optional fields take an
/// empty text as "none".
fn setting_number(
    node: &NodeView,
    name: &str,
    schema: &serde_json::Value,
    typed: &str,
) -> Option<serde_json::Value> {
    if node.definition == "core/on-watch" {
        if typed.trim().is_empty() && matches!(name, "release" | "hold" | "cooldown") {
            return Some(serde_json::Value::Null);
        }
        return watch_number_value(typed, matches!(name, "hold" | "cooldown")).ok();
    }
    let number: f64 = typed.trim().parse().ok()?;
    let least = schema["minimum"].as_f64().unwrap_or(f64::MIN);
    let most = schema["maximum"].as_f64().unwrap_or(f64::MAX);
    let number = number.clamp(least, most);
    if schema["type"] == "integer" {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "held to the schema's range"
        )]
        let whole = number.round() as i64;
        Some(whole.into())
    } else {
        serde_json::Number::from_f64(number).map(serde_json::Value::Number)
    }
}

/// A text box over a field while it is typed in, framed like the field.
fn typing_field(
    ui: &mut egui::Ui,
    slot: Rect,
    id: egui::Id,
    typed: &mut String,
    monospace: bool,
    zoom: f32,
) -> egui::Response {
    ui.scope_builder(UiBuilder::new().max_rect(slot).id_salt(id), |ui| {
        styled(ui, zoom);
        let font = if monospace {
            FontId::monospace(12.5 * zoom)
        } else {
            FontId::proportional(12.5 * zoom)
        };
        ui.add_sized(
            slot.size(),
            egui::TextEdit::singleline(typed)
                .font(font)
                .margin(egui::vec2(8.0, 0.0) * zoom)
                .align(Align2::LEFT_CENTER),
        )
    })
    .inner
}

fn multiline_field(
    ui: &mut egui::Ui,
    slot: Rect,
    id: egui::Id,
    value: &mut String,
    hint: &str,
    zoom: f32,
    colors: &Palette,
) -> egui::Response {
    frame(ui.painter(), slot, zoom, colors, true);
    let inner = slot.shrink2(Vec2::new(4.0, 3.0) * zoom);
    ui.scope_builder(UiBuilder::new().max_rect(inner).id_salt(id), |ui| {
        ui.set_clip_rect(ui.clip_rect().intersect(inner));
        egui::ScrollArea::vertical()
            .max_height(inner.height())
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(value)
                        .id(id)
                        .font(FontId::monospace(12.5 * zoom))
                        .frame(egui::Frame::NONE)
                        .desired_width(ui.available_width())
                        .min_size(Vec2::new(0.0, inner.height()))
                        .hint_text(hint),
                )
            })
            .inner
    })
    .inner
}

/// A dropdown filling `slot`: the chosen item's text, and its list on a click.
fn dropdown(
    ui: &mut egui::Ui,
    slot: Rect,
    id: egui::Id,
    text: &str,
    dim: bool,
    list: impl FnOnce(&mut egui::Ui),
) {
    let zoom = slot.height() / card::CONTROL;
    let colors = Palette::of(ui);
    let _ = ui.scope_builder(UiBuilder::new().max_rect(slot).id_salt(id), |ui| {
        styled(ui, zoom);
        ui.spacing_mut().icon_width = 12.0 * zoom;
        ui.spacing_mut().icon_spacing = 4.0 * zoom;
        let selection = if dim {
            egui::RichText::new(text).color(colors.weak)
        } else {
            egui::RichText::new(text).color(colors.text).strong()
        };
        let response = egui::ComboBox::from_id_salt("dropdown")
            .width(slot.width())
            .height(320.0)
            .wrap_mode(egui::TextWrapMode::Truncate)
            .selected_text(selection)
            .show_select(ui, Surface::Card, |ui| {
                ui.style_mut().override_font_id = None;
                ui.set_min_width(slot.width() / zoom.max(0.5));
                list(ui);
            })
            .response;
        response.on_hover_text(text);
    });
}

/// The spacing and type size of egui widgets drawn in a card's chart body at `zoom`.
pub(super) fn style_controls(ui: &mut egui::Ui, zoom: f32) {
    /// The height of a control in a chart body, in graph units.
    const CHART_CONTROL: f32 = 20.0;
    let style = ui.style_mut();
    style.override_font_id = Some(FontId::proportional(12.0 * zoom));
    style.wrap_mode = Some(egui::TextWrapMode::Truncate);
    style.spacing.item_spacing = Vec2::new(6.0, 4.0) * zoom;
    style.spacing.button_padding = Vec2::new(6.0, 2.0) * zoom;
    style.spacing.interact_size = Vec2::new(24.0, CHART_CONTROL) * zoom;
    style.spacing.icon_width = 12.0 * zoom;
    style.spacing.icon_width_inner = 8.0 * zoom;
    style.spacing.icon_spacing = 4.0 * zoom;
}

/// Card widgets drawn by egui take the card's look and scale (22 §3.1).
fn styled(ui: &mut egui::Ui, zoom: f32) {
    let colors = Palette::of(ui);
    ui.spacing_mut().item_spacing = egui::vec2(4.0, 3.0) * zoom;
    ui.spacing_mut().interact_size.y = card::CONTROL * zoom;
    ui.spacing_mut().button_padding = egui::vec2(8.0, 2.0) * zoom;
    ui.style_mut().override_font_id = Some(FontId::proportional(12.5 * zoom));
    let widgets = &mut ui.visuals_mut().widgets;
    for state in [
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ] {
        state.bg_fill = colors.background;
        state.weak_bg_fill = colors.background;
        state.corner_radius = egui::CornerRadius::same((5.0 * zoom) as u8);
        state.expansion = 0.0;
    }
    widgets.inactive.bg_stroke = Stroke::new(1.0, colors.outline);
    widgets.hovered.bg_stroke = Stroke::new(1.0, colors.selected);
    widgets.active.bg_stroke = Stroke::new(1.0, colors.selected);
    widgets.open.bg_stroke = Stroke::new(1.0, colors.selected);
}
