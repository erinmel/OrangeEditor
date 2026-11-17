//! Card controls derived from the package's configuration schema.
use super::*;
pub(super) fn fields(node: &NodeView) -> impl Iterator<Item = (&String, &serde_json::Value)> {
    node.schema
        .as_ref()
        .and_then(|schema| schema["properties"].as_object())
        .into_iter()
        .flat_map(|properties| properties.iter())
        .filter(|(name, _)| {
            !node
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
pub(super) fn height(node: &NodeView) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "schema fields are bounded by the loaded definition"
    )]
    {
        fields(node).count() as f32 * ROW_HEIGHT
    }
}
pub(super) fn draw(
    ui: &mut egui::Ui,
    node: &NodeView,
    rect: Rect,
    zoom: f32,
) -> Option<(String, serde_json::Value)> {
    let count = fields(node).count();
    let mut changed = None;
    for (row, (name, schema)) in fields(node).enumerate() {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a card has a finite number of fields"
        )]
        let y = rect.bottom() - (FOOTER_HEIGHT + (count - row) as f32 * ROW_HEIGHT) * zoom;
        let slot = Rect::from_min_size(
            egui::pos2(rect.left() + CARD_PADDING * zoom, y),
            egui::vec2(rect.width() - 2.0 * CARD_PADDING * zoom, ROW_HEIGHT * zoom),
        );
        if !ui.is_rect_visible(slot) {
            continue;
        }
        let current = node
            .settings
            .get(name)
            .cloned()
            .unwrap_or_else(|| schema["default"].clone());
        let label = schema["title"]
            .as_str()
            .map_or_else(|| name.replace('_', " "), str::to_owned);
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(slot)
                .id_salt(("node-setting", node.id, name)),
            |ui| {
                ui.spacing_mut().interact_size.y = FIELD_HEIGHT * zoom;
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [slot.width() * 0.5, FIELD_HEIGHT * zoom],
                        egui::Label::new(egui::RichText::new(label).size(11.5 * zoom)).truncate(),
                    );
                    let mut value = current.clone();
                    if let Some(options) = schema["enum"].as_array() {
                        egui::ComboBox::from_id_salt("choice")
                            .truncate()
                            .selected_text(value.as_str().unwrap_or("Choose"))
                            .width(slot.width() * 0.4)
                            .show_ui(ui, |ui| {
                                for option in options {
                                    ui.selectable_value(
                                        &mut value,
                                        option.clone(),
                                        option.as_str().unwrap_or("Choice"),
                                    );
                                }
                            });
                    } else if schema["type"] == "boolean" {
                        let mut on = value.as_bool().unwrap_or(false);
                        if ui.checkbox(&mut on, "").changed() {
                            value = on.into();
                        }
                    } else if schema["type"] == "number" || schema["type"] == "integer" {
                        let mut number = value.as_f64().unwrap_or(0.0);
                        let min = schema["minimum"].as_f64().unwrap_or(-f64::MAX);
                        let max = schema["maximum"].as_f64().unwrap_or(f64::MAX);
                        if ui
                            .add(egui::DragValue::new(&mut number).range(min..=max).speed(
                                if schema["type"] == "integer" {
                                    1.0
                                } else {
                                    0.25
                                },
                            ))
                            .changed()
                        {
                            value = if schema["type"] == "integer" {
                                serde_json::from_str(&format!("{number:.0}"))
                                    .unwrap_or(current.clone())
                            } else {
                                serde_json::Value::from(number)
                            };
                        }
                    } else {
                        let mut typed = value.as_str().unwrap_or_default().to_owned();
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut typed)
                                    .desired_width(slot.width() * 0.4),
                            )
                            .changed()
                        {
                            value = typed.into();
                        }
                    }
                    if value != current {
                        changed = Some((
                            format!("/{}", name.replace('~', "~0").replace('/', "~1")),
                            value,
                        ));
                    }
                });
            },
        );
    }
    changed
}
