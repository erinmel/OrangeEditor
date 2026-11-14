//! The properties panel: what the selected node settles (the declarations it names,
//! its settings, the types of a generic node) and the values on its inputs.
use super::*;
use orange_host::{NodeView, ReferenceKind};
use serde_json::{Value as Json, json};

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
    /// A length of time, typed in milliseconds and kept as nanoseconds in digits.
    Time,
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
            _ => Self::Text,
        }
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
                .map_err(|_| format!("`{typed}` is not a whole number")),
            Self::Number => typed
                .parse::<f64>()
                .map(Json::from)
                .map_err(|_| format!("`{typed}` is not a number")),
            Self::Time => typed
                .parse::<f64>()
                .ok()
                .filter(|millis| millis.is_finite() && *millis >= 0.0)
                .map(|millis| {
                    #[expect(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "a non-negative length of time well inside u64"
                    )]
                    let nanos = (millis * NANOS_PER_MILLI).round() as u64;
                    json!(nanos.to_string())
                })
                .ok_or_else(|| format!("`{typed}` is not a number of milliseconds")),
            Self::Choice(_) | Self::Toggle | Self::Text => Ok(json!(typed)),
        }
    }
}

/// The key of a setting among the typed fields, apart from the inputs'.
fn setting_key(pointer: &str) -> String {
    format!("@{pointer}")
}

impl Editor {
    /// Changes one setting of a node, keeping the others: `value` at the JSON
    /// `pointer`, or the setting taken off with `None`.
    pub fn set_setting(
        &mut self,
        host: &mut Host,
        node: NodeId,
        pointer: &str,
        value: Option<Json>,
    ) -> bool {
        let mut settings = self
            .graph()
            .and_then(|graph| graph.nodes.iter().find(|found| found.id == node))
            .map(|found| found.settings.clone())
            .filter(Json::is_object)
            .unwrap_or_else(|| json!({}));
        set_at(&mut settings, pointer, value);
        let Some(text) = self.editing() else {
            return false;
        };
        let result = host.set_config(&text, node, &settings);
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
        let Some(text) = self.editing() else {
            return false;
        };
        let result = host.choose_type(&text, node, param, ty);
        self.edited(host, result)
    }

    /// The declarations of one kind a node can name, as (ID, name).
    fn choices(&self, kind: ReferenceKind) -> Vec<(String, String)> {
        let Some(declared) = self.declarations() else {
            return Vec::new();
        };
        let pairs = |id: &Option<String>, name: &str| Some((id.clone()?, name.to_owned()));
        match kind {
            ReferenceKind::Variable => declared
                .variables
                .iter()
                .filter_map(|variable| pairs(&variable.id, &variable.name))
                .collect(),
            ReferenceKind::Connection => declared
                .connections
                .iter()
                .filter_map(|connection| pairs(&connection.id, &connection.name))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Reads the fields of `node` again when another node is selected.
    fn inspect(&mut self, node: &NodeView) {
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
        if let Some(properties) = node
            .schema
            .as_ref()
            .and_then(|schema| schema["properties"].as_object())
        {
            for (name, schema) in properties {
                let pointer = format!("/{name}");
                let typed = Field::of(schema).typed(&node.settings[name.as_str()]);
                let _ = self.typed.insert(setting_key(&pointer), typed);
            }
        }
    }

    pub(crate) fn properties_panel(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        ui.heading("Properties");
        let selected = self.canvas.selected().and_then(|id| {
            self.graph()?
                .nodes
                .iter()
                .find(|node| node.id == id)
                .cloned()
        });
        let Some(node) = selected else {
            self.inspected = None;
            ui.weak("Select a node to see and change its values.");
            return;
        };
        self.inspect(&node);
        ui.strong(node.name.as_str());
        ui.weak(node.kind.as_str());
        ui.weak(node.definition.as_str());
        ui.separator();
        // What to do once the fields are drawn.
        let mut setting: Option<(String, Option<Json>)> = None;
        let mut typing: Option<(String, Option<String>)> = None;
        let mut write: Option<(String, Option<String>)> = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (pointer, kind) in &node.references {
                setting = self
                    .reference_field(ui, &node, pointer, *kind)
                    .or(setting.take());
            }
            if let Some(changed) = self.settings_fields(ui, &node) {
                setting = Some(changed);
            }
            for param in &node.types {
                let shown = param
                    .chosen
                    .as_deref()
                    .map_or_else(|| "from the wires".to_owned(), type_name);
                ui.label(format!("type {}", param.name)).on_hover_text(
                    param.found.as_deref().map_or_else(
                        || "No type found yet.".to_owned(),
                        |found| format!("Now {}.", type_name(found)),
                    ),
                );
                let _ = egui::ComboBox::from_id_salt(("orange-type", param.name.as_str()))
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(param.chosen.is_none(), "from the wires")
                            .clicked()
                            && param.chosen.is_some()
                        {
                            typing = Some((param.name.clone(), None));
                        }
                        for ty in COMMON_TYPES {
                            let current = param.chosen.as_deref() == Some(ty);
                            if ui.selectable_label(current, type_name(ty)).clicked() && !current {
                                typing = Some((param.name.clone(), Some(ty.to_owned())));
                            }
                        }
                    });
            }
            if !node.references.is_empty() || node.schema.is_some() || !node.types.is_empty() {
                ui.separator();
            }
            write = self.input_fields(ui, &node);
            ui.separator();
            if ui.button("Delete node").clicked() {
                let _ = self.remove_node(host, node.id);
            }
        });
        if let Some((pointer, value)) = setting {
            let _ = self.set_setting(host, node.id, &pointer, value);
        }
        if let Some((param, ty)) = typing {
            let _ = self.choose_type(host, node.id, &param, ty.as_deref());
        }
        if let Some((port, typed)) = write {
            let _ = self.set_input(host, (node.id, &port), typed.as_deref());
        }
    }

    /// A chooser of the declaration a setting names. Returns the setting to write.
    fn reference_field(
        &self,
        ui: &mut egui::Ui,
        node: &NodeView,
        pointer: &str,
        kind: ReferenceKind,
    ) -> Option<(String, Option<Json>)> {
        let label = pointer.trim_start_matches('/');
        let named = node
            .settings
            .pointer(pointer)
            .and_then(Json::as_str)
            .map(str::to_owned);
        let declared = self.choices(kind);
        let what = match kind {
            ReferenceKind::Variable => "variable",
            ReferenceKind::Connection => "connection",
            ReferenceKind::Event => "event",
            ReferenceKind::Function => "function",
        };
        let shown = named
            .as_ref()
            .and_then(|id| declared.iter().find(|(known, _)| known == id))
            .map_or_else(|| format!("choose a {what}"), |(_, name)| name.clone());
        ui.label(label);
        let mut chosen = None;
        let _ = egui::ComboBox::from_id_salt(("orange-reference", pointer))
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for (id, name) in &declared {
                    let current = named.as_deref() == Some(id.as_str());
                    if ui.selectable_label(current, name.as_str()).clicked() && !current {
                        chosen = Some((pointer.to_owned(), Some(json!(id))));
                    }
                }
                if declared.is_empty() {
                    ui.weak(format!(
                        "The document declares no {what}s. Declare one under Document."
                    ));
                }
            });
        chosen
    }

    /// Fields for the settings the node's schema describes, but not those naming a
    /// declaration. Returns a setting to write.
    fn settings_fields(
        &mut self,
        ui: &mut egui::Ui,
        node: &NodeView,
    ) -> Option<(String, Option<Json>)> {
        let properties = node.schema.as_ref()?["properties"].as_object()?;
        let required: Vec<&str> = node.schema.as_ref()?["required"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Json::as_str)
            .collect();
        let mut changed = None;
        for (name, schema) in properties {
            let pointer = format!("/{name}");
            if node.references.iter().any(|(at, _)| *at == pointer) {
                continue;
            }
            let field = Field::of(schema);
            let current = &node.settings[name.as_str()];
            let unit = if matches!(field, Field::Time) {
                " (milliseconds)"
            } else {
                ""
            };
            let needed = if required.contains(&name.as_str()) {
                ""
            } else {
                ", optional"
            };
            ui.label(format!("{name}{unit}{needed}"));
            match &field {
                Field::Choice(options) => {
                    let shown = current.as_str().unwrap_or("not set").to_owned();
                    let _ = egui::ComboBox::from_id_salt(("orange-setting", name.as_str()))
                        .selected_text(shown)
                        .show_ui(ui, |ui| {
                            for option in options {
                                let on = current.as_str() == Some(*option);
                                if ui.selectable_label(on, *option).clicked() && !on {
                                    changed = Some((pointer.clone(), Some(json!(option))));
                                }
                            }
                        });
                }
                Field::Toggle => {
                    let mut on = current.as_bool().unwrap_or(false);
                    if ui.checkbox(&mut on, "").changed() {
                        changed = Some((pointer.clone(), Some(json!(on))));
                    }
                }
                Field::Whole | Field::Number | Field::Time | Field::Text => {
                    let typed = self.typed.entry(setting_key(&pointer)).or_default();
                    let mut said = None;
                    ui.horizontal(|ui| {
                        let entry = ui.add(
                            egui::TextEdit::singleline(typed)
                                .hint_text("not set")
                                .desired_width(150.0),
                        );
                        let entered = entry.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter));
                        if entered || ui.small_button("Set").clicked() {
                            match field.value(typed) {
                                Ok(value) => changed = Some((pointer.clone(), Some(value))),
                                Err(why) => said = Some(why),
                            }
                        }
                        if !current.is_null() && ui.small_button("Clear").clicked() {
                            changed = Some((pointer.clone(), None));
                        }
                    });
                    if let Some(why) = said {
                        self.say(format!("not done: {why}"));
                    }
                }
            }
        }
        changed
    }

    /// Fields for the values on the node's inputs. Returns a value to write or take off.
    fn input_fields(
        &mut self,
        ui: &mut egui::Ui,
        node: &NodeView,
    ) -> Option<(String, Option<String>)> {
        let mut write = None;
        for port in node.ports.iter().filter(|port| port.input && !port.exec) {
            let ty = port.ty.as_deref().map_or("value".to_owned(), type_name);
            ui.label(format!("{} ({ty})", port.id));
            if port.connected {
                ui.weak("comes from a wire");
                continue;
            }
            let typed = self.typed.entry(port.id.clone()).or_default();
            ui.horizontal(|ui| {
                let field = ui.add(
                    egui::TextEdit::singleline(typed)
                        .hint_text("no value")
                        .desired_width(150.0),
                );
                let entered =
                    field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                if entered || ui.small_button("Set").clicked() {
                    write = Some((port.id.clone(), Some(typed.clone())));
                }
                if port.value.is_some() && ui.small_button("Clear").clicked() {
                    write = Some((port.id.clone(), None));
                }
            });
        }
        write
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
}
