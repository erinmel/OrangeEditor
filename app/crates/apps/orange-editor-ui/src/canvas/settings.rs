//! Common card settings and the declaration a node names.
use super::*;
use crate::properties::{watch_field_entries, watch_label, watch_number_value};

/// Height of a row that chooses a declaration, such as a node's variable.
const REFERENCE_ROW: f32 = 30.0;

/// A change made with a card's settings.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum CardEdit {
    /// The setting at a JSON pointer takes a value.
    Set(String, serde_json::Value),
    /// A new variable is to be declared for the setting at a JSON pointer.
    NewVariable(String),
}

/// The settings of `node` that name one of the document's variables, as JSON pointers.
fn variable_pointers(node: &NodeView) -> impl Iterator<Item = &str> {
    node.references
        .iter()
        .filter(|(_, kind)| *kind == orange_host::ReferenceKind::Variable)
        .map(|(pointer, _)| pointer.as_str())
}

/// Whether `node` chooses a declaration on its card.
pub(super) fn has_references(node: &NodeView) -> bool {
    variable_pointers(node).next().is_some()
}

fn all_fields(node: &NodeView) -> impl Iterator<Item = (&String, &serde_json::Value)> {
    node.schema
        .as_ref()
        .and_then(|schema| schema["properties"].as_object())
        .into_iter()
        .flat_map(|properties| properties.iter())
        .filter(|(name, schema)| {
            !schema["readOnly"].as_bool().unwrap_or(false)
                && !node
                    .references
                    .iter()
                    .any(|(pointer, _)| pointer == &format!("/{name}"))
        })
        .filter(|(_, schema)| {
            schema["enum"].is_array()
                || matches!(
                    schema["type"].as_str(),
                    Some("string" | "boolean" | "number" | "integer")
                )
        })
}

/// Tuning settings belong in Properties.
fn advanced(name: &str) -> bool {
    matches!(name, "int_overflow" | "float_policy" | "window_ms")
}

pub(super) fn fields(node: &NodeView) -> impl Iterator<Item = (&String, &serde_json::Value)> {
    watch_field_entries(node).chain(
        all_fields(node).filter(|(name, _)| node.definition != "core/on-watch" && !advanced(name)),
    )
}

/// Every setting is one line: its name and its field (22 §3.1).
fn row_height(_schema: &serde_json::Value) -> f32 {
    30.0
}

/// The height of the settings rows under a card's ports.
pub(super) fn height(node: &NodeView) -> f32 {
    fields(node).map(|(_, schema)| row_height(schema)).sum()
}

/// The height of the pickers over a card's ports: what a node names comes first, so
/// Set variable reads "this variable, this value".
pub(super) fn reference_height(node: &NodeView) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a node names a few declarations"
    )]
    let rows = variable_pointers(node).count() as f32;
    rows * REFERENCE_ROW
}

/// The chooser of the variable at `pointer` among `variables` (ID, name), with a way
/// to declare a new one.
fn variable_field(
    ui: &mut egui::Ui,
    node: &NodeView,
    pointer: &str,
    variables: &[(String, String)],
    width: f32,
) -> Option<CardEdit> {
    let named = node
        .settings
        .pointer(pointer)
        .and_then(serde_json::Value::as_str);
    let shown = named
        .and_then(|id| variables.iter().find(|(known, _)| known == id))
        .map_or_else(
            || tf("choose-reference", &[("kind", "variable".into())]),
            |(_, name)| name.clone(),
        );
    let mut edit = None;
    let strong = ui.visuals().strong_text_color();
    ui.horizontal(|ui| {
        let _ = name_cell(
            ui,
            width * 0.32,
            egui::RichText::new(t("variable")).color(strong),
        );
        let _ = egui::ComboBox::from_id_salt("variable")
            .width(ui.available_width())
            .wrap_mode(egui::TextWrapMode::Truncate)
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for (id, name) in variables {
                    let current = named == Some(id.as_str());
                    if ui.selectable_label(current, name.as_str()).clicked() && !current {
                        edit = Some(CardEdit::Set(pointer.to_owned(), id.as_str().into()));
                    }
                }
                if !variables.is_empty() {
                    ui.separator();
                }
                if ui
                    .selectable_label(false, tf("new-reference", &[("kind", "variable".into())]))
                    .clicked()
                {
                    edit = Some(CardEdit::NewVariable(pointer.to_owned()));
                }
            });
    });
    edit
}

/// A row's name, left-aligned in a column `width` wide, cut short when longer.
fn name_cell(ui: &mut egui::Ui, width: f32, text: egui::RichText) -> egui::Response {
    ui.allocate_ui_with_layout(
        egui::vec2(width, ui.spacing().interact_size.y),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_width(width);
            ui.add(egui::Label::new(text).truncate())
        },
    )
    .inner
}

fn label(name: &str, schema: &serde_json::Value) -> String {
    let audio_label = match name {
        "window_ms" => Some("audio-setting-window-ms"),
        "threshold_db" => Some("audio-setting-threshold-db"),
        "release_db" => Some("audio-setting-release-db"),
        "hold_ms" => Some("audio-setting-hold-ms"),
        "reset_db" => Some("audio-reset-margin"),
        _ => None,
    };
    if let Some(key) = audio_label {
        return t(key).to_owned();
    }
    schema["title"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let name = name
                .strip_suffix("_hz")
                .or_else(|| name.strip_suffix("_db"))
                .or_else(|| name.strip_suffix("_ms"))
                .unwrap_or(name);
            let mut label = name.replace('_', " ");
            if let Some(first) = label.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            label
        })
}

fn watch_number(
    ui: &mut egui::Ui,
    name: &str,
    current: &serde_json::Value,
    width: f32,
) -> Option<serde_json::Value> {
    use serde_json::Value;
    let id = ui.make_persistent_id("watch-number");
    let (text, ended) = {
        let mut text = ui
            .data_mut(|data| data.get_temp::<String>(id))
            .unwrap_or_else(|| {
                current
                    .as_number()
                    .map_or_else(String::new, ToString::to_string)
            });
        let hint = if name == "release" {
            t("when-optional-release")
        } else {
            "0"
        };
        let response = ui.add(
            egui::TextEdit::singleline(&mut text)
                .id(id)
                .desired_width(width)
                .hint_text(hint),
        );
        if response.has_focus() {
            ui.data_mut(|data| data.insert_temp(id, text.clone()));
        } else {
            ui.data_mut(|data| data.remove::<String>(id));
        }
        (text, response.lost_focus())
    };
    if !ended {
        return None;
    }
    if text.trim().is_empty() && matches!(name, "release" | "hold" | "cooldown") {
        return (!current.is_null()).then_some(Value::Null);
    }
    watch_number_value(&text, matches!(name, "hold" | "cooldown"))
        .ok()
        .filter(|value| value != current)
}

fn field(
    ui: &mut egui::Ui,
    node: &NodeView,
    name: &str,
    schema: &serde_json::Value,
    width: f32,
) -> Option<(String, serde_json::Value)> {
    let current = node
        .settings
        .get(name)
        .cloned()
        .unwrap_or_else(|| schema["default"].clone());
    let mut value = current.clone();
    let is_watch = node.definition == "core/on-watch";
    let option_title = |item: &str| {
        if is_watch {
            watch_label(item)
        } else if name == "direction" && super::spectrum::is_noise(node) {
            super::spectrum::direction(item)
        } else {
            item
        }
        .to_owned()
    };
    let caption = if is_watch {
        watch_label(name).to_owned()
    } else {
        label(name, schema)
    };
    // One line: the name, then the field (22 §3.1).
    let label_width = width * 0.42;
    let width = width * 0.55;
    let edit = |ui: &mut egui::Ui, value: &mut serde_json::Value| {
        if let Some(options) = schema["enum"].as_array() {
            egui::ComboBox::from_id_salt("choice")
                .width(width)
                .truncate()
                .selected_text(
                    value
                        .as_str()
                        .map_or_else(|| t("choose").to_owned(), option_title),
                )
                .show_ui(ui, |ui| {
                    for option in options {
                        ui.selectable_value(
                            value,
                            option.clone(),
                            option
                                .as_str()
                                .map_or_else(|| t("choice").to_owned(), option_title),
                        );
                    }
                })
                .response
                .on_hover_text(current.as_str().unwrap_or_default());
        } else if schema["type"] == "boolean" {
            let mut on = value.as_bool().unwrap_or(false);
            if ui.checkbox(&mut on, "").changed() {
                *value = on.into();
            }
        } else if is_watch && matches!(schema["type"].as_str(), Some("number" | "integer")) {
            if let Some(new) = watch_number(ui, name, &current, width * 0.4) {
                *value = new;
            }
        } else if schema["type"] == "number" || schema["type"] == "integer" {
            let mut number = value.as_f64().unwrap_or(0.0);
            let suffix = if name.ends_with("_hz") {
                " Hz"
            } else if name == "reset_db" {
                " dB"
            } else if name.ends_with("_db") {
                " dBFS"
            } else if name.ends_with("_ms") {
                " ms"
            } else {
                ""
            };
            let integer = schema["type"] == "integer";
            if ui
                .add(
                    egui::DragValue::new(&mut number)
                        .range(
                            schema["minimum"].as_f64().unwrap_or(-f64::MAX)
                                ..=schema["maximum"].as_f64().unwrap_or(f64::MAX),
                        )
                        .suffix(suffix)
                        .speed(if integer { 1.0 } else { 0.25 }),
                )
                .changed()
            {
                *value = if integer {
                    serde_json::from_str(&format!("{number:.0}")).unwrap_or(current.clone())
                } else {
                    number.into()
                };
            }
        } else {
            let mut text = value.as_str().unwrap_or_default().to_owned();
            if ui
                .add(egui::TextEdit::singleline(&mut text).desired_width(width))
                .changed()
            {
                *value = text.into();
            }
        }
    };
    ui.horizontal(|ui| {
        name_cell(ui, label_width, egui::RichText::new(caption)).on_hover_text(
            if name == "window_ms" {
                t("audio-setting-window-help")
            } else {
                schema["description"].as_str().unwrap_or(name)
            },
        );
        edit(ui, &mut value);
    });
    (value != current).then(|| {
        (
            format!("/{}", name.replace('~', "~0").replace('/', "~1")),
            value,
        )
    })
}

pub(super) fn draw(
    ui: &mut egui::Ui,
    node: &NodeView,
    rect: Rect,
    zoom: f32,
    variables: &[(String, String)],
) -> Option<CardEdit> {
    let mut y = rect.top() + super::card_layout::CardLayout::new(node).reference_top * zoom;
    let mut changed = None;
    let scoped = |ui: &mut egui::Ui| {
        super::card_layout::style_controls(ui, zoom);
        // Every field on a card has the same framed look (22 §3.1).
        let colors = Palette::of(ui);
        let widgets = &mut ui.visuals_mut().widgets;
        for state in [
            &mut widgets.inactive,
            &mut widgets.hovered,
            &mut widgets.active,
        ] {
            state.bg_fill = colors.background;
            state.weak_bg_fill = colors.background;
        }
        widgets.inactive.bg_stroke = Stroke::new(1.0, colors.outline);
        widgets.inactive.corner_radius = egui::CornerRadius::same(4);
    };
    for pointer in variable_pointers(node) {
        let slot = Rect::from_min_size(
            egui::pos2(rect.left() + CARD_PADDING * zoom, y),
            egui::vec2(
                rect.width() - 2.0 * CARD_PADDING * zoom,
                REFERENCE_ROW * zoom,
            ),
        );
        y += REFERENCE_ROW * zoom;
        if !ui.is_rect_visible(slot) {
            continue;
        }
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(slot)
                .id_salt(("node-reference", node.id, pointer)),
            |ui| {
                scoped(ui);
                if let Some(edit) = variable_field(ui, node, pointer, variables, slot.width()) {
                    changed = Some(edit);
                }
            },
        );
    }
    y = rect.bottom() - (FOOTER_HEIGHT + height(node)) * zoom;
    for (name, schema) in fields(node) {
        let slot = Rect::from_min_size(
            egui::pos2(rect.left() + CARD_PADDING * zoom, y),
            egui::vec2(
                rect.width() - 2.0 * CARD_PADDING * zoom,
                row_height(schema) * zoom,
            ),
        );
        y += row_height(schema) * zoom;
        if !ui.is_rect_visible(slot) {
            continue;
        }
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(slot)
                .id_salt(("node-setting", node.id, name)),
            |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(slot));
                scoped(ui);
                if let Some((pointer, value)) = field(ui, node, name, schema, slot.width()) {
                    changed = Some(CardEdit::Set(pointer, value));
                }
            },
        );
    }
    changed
}
