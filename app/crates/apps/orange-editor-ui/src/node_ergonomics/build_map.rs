//! The paired editor for the Build Map node's two list literals.
use crate::{Editor, Host, NodeId, controls, t, tf};
use orange_host::{Literal, LiteralPayload, NodeView, PortKey, TypeExpr};
use serde_json::Value as Json;
use std::collections::BTreeMap;

const DEFINITION: &str = "org.orange.collections/build-map";
const PORTS: [&str; 2] = ["keys", "values"];

pub(crate) fn paired_map_available(node: &NodeView) -> bool {
    node.definition == DEFINITION
        && node.missing.is_none()
        && PORTS.iter().all(|key| {
            node.ports.iter().any(|port| {
                port.input
                    && !port.connected
                    && port.id == *key
                    && port
                        .ty
                        .as_deref()
                        .is_some_and(|ty| ty.starts_with("core/list@1<") && !ty.contains('$'))
            })
        })
}

struct Column {
    list: TypeExpr,
    element: String,
}

impl Column {
    fn parse(ty: &str) -> Option<Self> {
        if ty.contains('$') {
            return None;
        }
        let element = ty.strip_prefix("core/list@1<")?.strip_suffix('>')?;
        Some(Self {
            list: ty.parse().ok()?,
            element: element.to_owned(),
        })
    }
}

pub(crate) struct BuildMapRows {
    node: NodeId,
    revision: u64,
    columns: [Column; 2],
    rows: Vec<[Option<String>; 2]>,
    invalid: Option<(usize, usize)>,
    error: Option<String>,
}

impl std::fmt::Debug for BuildMapRows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuildMapRows")
            .field("node", &self.node)
            .field("count", &self.rows.len())
            .finish_non_exhaustive()
    }
}

impl BuildMapRows {
    fn literals(&self) -> Result<[Literal; 2], (usize, usize)> {
        let mut entries = [
            Vec::with_capacity(self.rows.len()),
            Vec::with_capacity(self.rows.len()),
        ];
        for (index, row) in self.rows.iter().enumerate() {
            for (column, text) in row.iter().enumerate() {
                let value = text
                    .as_deref()
                    .and_then(|text| cell_payload(&self.columns[column].element, text))
                    .ok_or((index, column))?;
                entries[column].push(value);
            }
        }
        Ok(std::array::from_fn(|column| {
            Literal::value(
                self.columns[column].list.clone(),
                Json::Array(std::mem::take(&mut entries[column])),
            )
        }))
    }
}

fn cell_payload(ty: &str, text: &str) -> Option<Json> {
    let scalar = text.trim();
    let string = match ty {
        "core/string@1" | "core/json@1" => text.to_owned(),
        "core/bytes@1" | "core/secret-ref@1" => scalar.to_owned(),
        "core/i64@1" | "core/duration@1" | "core/timestamp@1" => {
            scalar.parse::<i64>().ok()?.to_string()
        }
        "core/u64@1" => scalar.parse::<u64>().ok()?.to_string(),
        "core/f32@1" => match scalar.strip_prefix("0x") {
            Some(bits) => bits.to_owned(),
            None => format!("{:08x}", scalar.parse::<f32>().ok()?.to_bits()),
        },
        "core/f64@1" => match scalar.strip_prefix("0x") {
            Some(bits) => bits.to_owned(),
            None => format!("{:016x}", scalar.parse::<f64>().ok()?.to_bits()),
        },
        _ => return serde_json::from_str(text).ok(),
    };
    Some(Json::String(string))
}

fn cell_text(ty: &str, value: &Json) -> String {
    let Some(text) = value.as_str() else {
        return value.to_string();
    };
    match ty {
        "core/f32@1" => u32::from_str_radix(text, 16)
            .ok()
            .map(f32::from_bits)
            .filter(|value| value.is_finite())
            .map_or_else(|| format!("0x{text}"), |value| value.to_string()),
        "core/f64@1" => u64::from_str_radix(text, 16)
            .ok()
            .map(f64::from_bits)
            .filter(|value| value.is_finite())
            .map_or_else(|| format!("0x{text}"), |value| value.to_string()),
        "core/string@1" | "core/json@1" | "core/bytes@1" | "core/secret-ref@1" | "core/i64@1"
        | "core/u64@1" | "core/duration@1" | "core/timestamp@1" => text.to_owned(),
        _ => value.to_string(),
    }
}

fn saved_columns<'a>(
    inputs: &'a BTreeMap<PortKey, Literal>,
    columns: &[Column; 2],
) -> [Option<&'a [Json]>; 2] {
    std::array::from_fn(|column| {
        let port = PORTS[column].parse().ok()?;
        let Some(literal) = inputs.get(&port) else {
            return Some(&[][..]);
        };
        match &literal.payload {
            LiteralPayload::Value(Json::Array(values)) if literal.ty == columns[column].list => {
                Some(values.as_slice())
            }
            _ => None,
        }
    })
}

impl Editor {
    pub(crate) fn build_map_pair_control(&mut self, ui: &mut egui::Ui, node: &NodeView) -> bool {
        if node.definition != DEFINITION || node.missing.is_some() {
            return false;
        }
        let [Some(keys), Some(values)] =
            PORTS.map(|id| node.ports.iter().find(|port| port.input && port.id == id))
        else {
            return false;
        };
        if keys.connected || values.connected {
            return false;
        }
        let [Some(keys), Some(values)] =
            [keys, values].map(|port| port.ty.as_deref().and_then(Column::parse))
        else {
            ui.weak(t("build-map-choose-types"));
            return false;
        };
        let Some(document) = &self.document else {
            return false;
        };
        let Some(instance) = document.draft.document().node(node.id) else {
            return false;
        };
        let columns = [keys, values];
        let [Some(keys), Some(values)] = saved_columns(&instance.inputs, &columns) else {
            return false;
        };
        let count = keys.len().max(values.len());
        if ui
            .button(tf("build-map-edit-pairs", &[("count", count.into())]))
            .clicked()
        {
            self.open_map_pairs(node.id);
        }
        true
    }

    pub(crate) fn open_map_pairs(&mut self, node_id: NodeId) {
        let Some(document) = &self.document else {
            return;
        };
        let Some(node) = document
            .node_view(node_id)
            .filter(|node| paired_map_available(node))
        else {
            return;
        };
        let Some(instance) = document.draft.document().node(node_id) else {
            return;
        };
        let [Some(keys), Some(values)] = PORTS.map(|id| {
            node.ports
                .iter()
                .find(|port| port.input && port.id == id)
                .and_then(|port| port.ty.as_deref())
                .and_then(Column::parse)
        }) else {
            return;
        };
        let columns = [keys, values];
        let [Some(keys), Some(values)] = saved_columns(&instance.inputs, &columns) else {
            return;
        };
        let count = keys.len().max(values.len());
        self.build_map_rows = Some(BuildMapRows {
            node: node.id,
            revision: document.draft.document().revision(),
            rows: (0..count)
                .map(|index| {
                    std::array::from_fn(|column| {
                        [keys, values][column]
                            .get(index)
                            .map(|value| cell_text(&columns[column].element, value))
                    })
                })
                .collect(),
            columns,
            invalid: None,
            error: None,
        });
    }

    pub(crate) fn build_map_rows_dialog(&mut self, ctx: &egui::Context, host: &mut Host) {
        let Some(edit) = &mut self.build_map_rows else {
            return;
        };
        let modal = orange_ui_controls::modal(ctx, egui::Id::unique("orange-build-map-rows")).show(
            ctx,
            |ui| {
                ui.set_width(ui.available_width().min(460.0));
                ui.heading(t("build-map-pairs-title"));
                let row_height = ui.spacing().interact_size.y;
                let column_width = ((ui.available_width() - 56.0) / 2.0).max(55.0);
                let labels = [t("build-map-key"), t("build-map-value")];
                let mut remove = None;
                ui.horizontal(|ui| {
                    for (column, label) in labels.into_iter().enumerate() {
                        let ty = crate::type_name(&edit.columns[column].element);
                        ui.add_sized([column_width, row_height], egui::Label::new(label))
                            .on_hover_text(ty);
                    }
                });
                egui::ScrollArea::vertical().max_height(300.0).show_rows(
                    ui,
                    row_height,
                    edit.rows.len(),
                    |ui, range| {
                        for index in range {
                            ui.horizontal(|ui| {
                                for (column, cell) in edit.rows[index].iter_mut().enumerate() {
                                    let present = cell.is_some();
                                    let mut text = cell.take().unwrap_or_default();
                                    let response = ui.add(
                                        egui::TextEdit::singleline(&mut text)
                                            .id_salt(("map-cell", index, column))
                                            .desired_width(column_width)
                                            .hint_text(labels[column]),
                                    );
                                    if response.changed() || present {
                                        *cell = Some(text);
                                        if response.changed() {
                                            edit.invalid = None;
                                            edit.error = None;
                                        }
                                    }
                                }
                                if ui
                                    .small_button(crate::icons::text(crate::icons::DELETE))
                                    .on_hover_text(t("build-map-remove-pair"))
                                    .clicked()
                                {
                                    remove = Some(index);
                                }
                            });
                        }
                    },
                );
                if let Some(index) = remove {
                    let _ = edit.rows.remove(index);
                    edit.invalid = None;
                    edit.error = None;
                }
                if ui.button(t("build-map-add-pair")).clicked() {
                    edit.rows.push([Some(String::new()), Some(String::new())]);
                    edit.invalid = None;
                    edit.error = None;
                }
                if let Some((row, column)) = edit.invalid {
                    ui.label(tf(
                        "build-map-invalid-cell",
                        &[("row", (row + 1).into()), ("column", labels[column].into())],
                    ));
                }
                if let Some(error) = &edit.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                controls::dialog_actions(
                    ui,
                    controls::Action::Primary,
                    true,
                    egui::Button::new(t("save")),
                )
            },
        );
        if modal.inner.0.clicked() || modal.should_close() {
            self.build_map_rows = None;
            return;
        }
        if !modal.inner.1.clicked() {
            return;
        }
        self.save_build_map_rows(host);
    }

    fn save_build_map_rows(&mut self, host: &mut Host) {
        let Some(edit) = self.build_map_rows.as_mut() else {
            return;
        };
        let Ok(literals) = edit
            .literals()
            .inspect_err(|&site| edit.invalid = Some(site))
        else {
            return;
        };
        let node = edit.node;
        let columns = &edit.columns;
        let map_type = format!("core/map@1<{},{}>", columns[0].element, columns[1].element);
        if !self
            .document
            .as_ref()
            .is_some_and(|doc| doc.draft.document().revision() == edit.revision)
        {
            self.build_map_rows = None;
            self.say(t("node-edit-preview-stale").to_owned());
            return;
        }
        let Some(draft) = self.editing() else {
            return;
        };
        let [
            LiteralPayload::Value(Json::Array(keys)),
            LiteralPayload::Value(Json::Array(values)),
        ] = literals.each_ref().map(|literal| &literal.payload)
        else {
            return;
        };
        let check = map_type.parse().ok().map(|ty| {
            let pairs = keys
                .iter()
                .zip(values)
                .map(|(key, value)| Json::Array(vec![key.clone(), value.clone()]))
                .collect();
            host.value_in(draft.document(), &Literal::value(ty, Json::Array(pairs)))
        });
        if let Some(Err(error)) = check {
            if let Some(edit) = &mut self.build_map_rows {
                edit.error = Some(error.to_string());
            }
            return;
        }
        let result = host.set_input_literals(
            draft,
            PORTS
                .into_iter()
                .zip(literals)
                .map(|(port, literal)| ((node, port), literal)),
        );
        if result.is_ok() {
            self.build_map_rows = None;
        }
        let _ = self.edited(host, result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paired_cells_round_trip_whitespace_and_exact_float_bits() {
        for (ty, value) in [
            ("core/string@1", Json::from("  { key }  ")),
            ("core/f64@1", Json::from("8000000000000000")),
            ("core/f64@1", Json::from("7ff8000000000010")),
            ("core/f32@1", Json::from("40490fdb")),
            ("core/i64@1", Json::from("-9223372036854775808")),
            ("core/u64@1", Json::from("18446744073709551615")),
        ] {
            assert_eq!(cell_payload(ty, &cell_text(ty, &value)), Some(value));
        }
    }
}
