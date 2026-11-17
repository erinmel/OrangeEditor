//! Common card settings, with numeric policies disclosed in the header menu.
use super::*;

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

fn advanced(name: &str) -> bool {
    matches!(name, "int_overflow" | "float_policy")
}

pub(super) fn fields(node: &NodeView) -> impl Iterator<Item = (&String, &serde_json::Value)> {
    all_fields(node).filter(|(name, _)| !advanced(name))
}

pub(super) fn has_advanced(node: &NodeView) -> bool {
    all_fields(node).any(|(name, _)| advanced(name))
}

fn row_height(schema: &serde_json::Value) -> f32 {
    if schema["enum"].is_array() || schema["type"] == "string" {
        52.0
    } else {
        32.0
    }
}

pub(super) fn height(node: &NodeView) -> f32 {
    fields(node).map(|(_, schema)| row_height(schema)).sum()
}

fn label(name: &str, schema: &serde_json::Value) -> String {
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
    let caption = label(name, schema);
    let stacked = row_height(schema) > 32.0;
    let edit = |ui: &mut egui::Ui, value: &mut serde_json::Value| {
        if let Some(options) = schema["enum"].as_array() {
            egui::ComboBox::from_id_salt("choice")
                .width(width)
                .wrap_mode(egui::TextWrapMode::Wrap)
                .selected_text(value.as_str().unwrap_or("Choose…"))
                .show_ui(ui, |ui| {
                    for option in options {
                        ui.selectable_value(
                            value,
                            option.clone(),
                            option.as_str().unwrap_or("Choice"),
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
        } else if schema["type"] == "number" || schema["type"] == "integer" {
            let mut number = value.as_f64().unwrap_or(0.0);
            let suffix = if name.ends_with("_hz") {
                " Hz"
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
    if stacked {
        ui.label(egui::RichText::new(caption).small())
            .on_hover_text(schema["description"].as_str().unwrap_or(name));
        edit(ui, &mut value);
    } else {
        ui.horizontal(|ui| {
            ui.add_sized(
                [width * 0.55, ui.spacing().interact_size.y],
                egui::Label::new(caption),
            )
            .on_hover_text(schema["description"].as_str().unwrap_or(name));
            edit(ui, &mut value);
        });
    }
    (value != current).then(|| {
        (
            format!("/{}", name.replace('~', "~0").replace('/', "~1")),
            value,
        )
    })
}

pub(super) fn menu(ui: &mut egui::Ui, node: &NodeView) -> Option<(String, serde_json::Value)> {
    let mut changed = None;
    ui.set_width(240.0);
    ui.strong("Numeric policies");
    for (name, schema) in all_fields(node).filter(|(name, _)| advanced(name)) {
        ui.push_id(name, |ui| {
            if let Some(edit) = field(ui, node, name, schema, 220.0) {
                changed = Some(edit);
            }
        });
    }
    changed
}

pub(super) fn draw(
    ui: &mut egui::Ui,
    node: &NodeView,
    rect: Rect,
    zoom: f32,
) -> Option<(String, serde_json::Value)> {
    let mut y = rect.bottom() - (FOOTER_HEIGHT + height(node)) * zoom;
    let mut changed = None;
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
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 3.0) * zoom;
                ui.spacing_mut().interact_size.y = FIELD_HEIGHT * zoom;
                ui.style_mut().override_font_id = Some(FontId::proportional(12.0 * zoom));
                if let Some(edit) = field(ui, node, name, schema, slot.width()) {
                    changed = Some(edit);
                }
            },
        );
    }
    changed
}
