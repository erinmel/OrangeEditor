//! The properties panel: what the selected node settles (the declarations it names,
//! its settings, the types of a generic node) and the values on its inputs.
use super::*;
use orange_host::{Connection, NodeView, ReferenceKind, VarScope, Variable};
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

    /// The declarations of one kind a node can name, as (ID, name).
    /// Connections are those of kind `adapter` when the node says which it uses.
    fn choices(&self, kind: ReferenceKind, adapter: Option<&str>) -> Vec<(String, String)> {
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
                .filter(|connection| adapter.is_none_or(|kind| connection.adapter == kind))
                .filter_map(|connection| pairs(&connection.id, &connection.name))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The declared connection `node` names, if it names one.
    fn named_connection(&self, node: &NodeView) -> Option<Connection> {
        let id = node
            .references
            .iter()
            .filter(|(_, kind)| *kind == ReferenceKind::Connection)
            .find_map(|(pointer, _)| node.settings.pointer(pointer)?.as_str())?;
        self.declarations()?
            .connections
            .iter()
            .find(|connection| connection.id.as_deref() == Some(id))
            .cloned()
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

    /// The value on an input as its field shows it before typing: text without quotes.
    fn shown_input(port: &orange_host::PortView) -> String {
        let shown = port.value.clone().unwrap_or_default();
        serde_json::from_str::<String>(&shown).unwrap_or(shown)
    }

    pub(crate) fn properties_panel(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        // What the panel holds never widens it; only dragging its edge does.
        ui.set_max_width(ui.available_width());
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 5.0);
        let selected = self.canvas.selected().and_then(|id| {
            self.graph()?
                .nodes
                .iter()
                .find(|node| node.id == id)
                .cloned()
        });
        let Some(node) = selected else {
            self.inspected = None;
            self.quick_variable = None;
            ui.add_space(4.0);
            let many = self.canvas.selection();
            if many.len() > 1 {
                ui.strong(format!("{} nodes selected", many.len()));
                ui.label("Ctrl+C copies them; Ctrl+V pastes where the pointer is.");
                if ui.button("Delete them").clicked() {
                    let _ = self.remove_nodes(host, &many);
                }
            } else {
                ui.weak("Select a node to see and change its values.");
                ui.weak("Ctrl+click or Shift+drag selects several.");
            }
            return;
        };
        self.inspect(&node);
        if self
            .quick_variable
            .as_ref()
            .is_some_and(|quick| quick.node != node.id)
        {
            self.quick_variable = None;
        }
        ui.add_space(2.0);
        let _ = ui
            .add(egui::Label::new(egui::RichText::new(node.name.as_str()).heading()).truncate())
            .on_hover_text(node.name.as_str());
        let mut subtitle = node.kind.clone();
        if let Some(detail) = &node.detail {
            subtitle.push_str(" · ");
            subtitle.push_str(detail);
        }
        let _ = ui
            .add(egui::Label::new(egui::RichText::new(subtitle).weak()).truncate())
            .on_hover_text(node.definition.as_str());
        ui.separator();
        // What to do once the fields are drawn: one change a frame.
        let mut change = None;
        // The kind of connection the node uses, and the one it names with its choices.
        let adapter = host
            .nodes()
            .find(|info| info.definition.definition.id.as_str() == node.definition)
            .and_then(|info| info.presentation?.connection_adapter.clone());
        let connection = self.named_connection(&node);
        let choices = connection
            .as_ref()
            .map(|connection| self.adapter_choices(host, &connection.adapter, false))
            .unwrap_or_default();
        let mut refresh = false;
        let mut redeclare: Option<Connection> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // One label width keeps the fields in line; a narrow panel gives the
                // room to the fields.
                let labels = (ui.available_width() * 0.4).clamp(48.0, LABEL_WIDTH);
                let setup =
                    !node.references.is_empty() || node.schema.is_some() || !node.types.is_empty();
                if setup {
                    section(ui, "Set-up");
                    for (pointer, kind) in &node.references {
                        if let Some(asked) = self.reference_field(
                            ui,
                            labels,
                            &node,
                            pointer,
                            *kind,
                            adapter.as_deref(),
                        ) {
                            change = Some(asked);
                        }
                    }
                    if let Some(connection) = &connection
                        && !choices.is_empty()
                    {
                        if let Some((name, value)) = declarations::choice_fields(
                            ui,
                            "node",
                            &choices,
                            &connection.settings,
                            &mut refresh,
                        ) {
                            redeclare = Some(Connection {
                                settings: declarations::with_setting(
                                    &connection.settings,
                                    &name,
                                    value,
                                ),
                                ..connection.clone()
                            });
                        }
                        ui.weak(format!(
                            "Settings of the connection “{}”, for every node that uses it.",
                            connection.name
                        ));
                    }
                    if let Some(asked) = self.settings_fields(ui, labels, &node) {
                        change = Some(asked);
                    }
                    if let Some(asked) = Self::type_fields(ui, labels, &node) {
                        change = Some(asked);
                    }
                }
                if !node.variadic.is_empty() {
                    section(ui, "Ports");
                    if let Some(asked) = Self::variadic_fields(ui, labels, &node) {
                        change = Some(asked);
                    }
                }
                if node.ports.iter().any(|port| port.input && !port.exec) {
                    section(ui, "Inputs");
                    if let Some(asked) = self.input_fields(ui, labels, &node) {
                        change = Some(asked);
                    }
                }
                self.values_section(ui, &node);
                ui.add_space(10.0);
                if ui.button("Delete node").clicked() {
                    change = Some(Change::Delete);
                }
            });
        if let Some(connection) = redeclare {
            let _ = self.declare_connection(host, &connection);
        }
        if refresh && let Some(connection) = &connection {
            let _ = self.adapter_choices(host, &connection.adapter, true);
        }
        let Some(change) = change else {
            return;
        };
        match change {
            Change::Setting(pointer, value) => {
                let _ = self.set_setting(host, node.id, &pointer, value);
            }
            Change::Type(param, ty) => {
                let _ = self.choose_type(host, node.id, &param, ty.as_deref());
            }
            Change::Input(port, typed) => {
                let _ = self.set_input(host, (node.id, &port), typed.as_deref());
            }
            Change::AddPort(port) => {
                let _ = self.add_port(host, node.id, &port);
            }
            Change::RemovePort(port) => {
                let _ = self.remove_port(host, (node.id, &port));
            }
            Change::Declare(pointer, variable) => {
                if self
                    .declare_variable_for(host, node.id, &pointer, &variable)
                    .is_some()
                {
                    self.quick_variable = None;
                }
            }
            Change::CancelDeclare => self.quick_variable = None,
            Change::NewConnection => {
                self.panels.nodes = true;
                self.tab = crate::Tab::Connections;
                self.forms.reveal_connection();
            }
            Change::Delete => {
                let _ = self.remove_node(host, node.id);
            }
        }
    }

    /// A chooser of the declaration a setting names, with a way to declare a new one.
    fn reference_field(
        &mut self,
        ui: &mut egui::Ui,
        labels: f32,
        node: &NodeView,
        pointer: &str,
        kind: ReferenceKind,
        adapter: Option<&str>,
    ) -> Option<Change> {
        let named = node
            .settings
            .pointer(pointer)
            .and_then(Json::as_str)
            .map(str::to_owned);
        let declared = self.choices(kind, adapter);
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
        let mut chosen = None;
        let mut quick = false;
        row(
            ui,
            labels,
            &words(pointer.trim_start_matches('/')),
            None,
            |ui| {
                let _ = egui::ComboBox::from_id_salt(("orange-reference", pointer))
                    .selected_text(shown)
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        for (id, name) in &declared {
                            let current = named.as_deref() == Some(id.as_str());
                            if ui.selectable_label(current, name.as_str()).clicked() && !current {
                                chosen = Some(Change::Setting(pointer.to_owned(), Some(json!(id))));
                            }
                        }
                        match kind {
                            ReferenceKind::Variable | ReferenceKind::Connection => {
                                if !declared.is_empty() {
                                    ui.separator();
                                }
                                quick =
                                    ui.selectable_label(false, format!("New {what}…")).clicked();
                            }
                            _ if declared.is_empty() => {
                                ui.weak(format!("This graph declares no {what}s."));
                            }
                            _ => {}
                        }
                    });
            },
        );
        if quick {
            if kind != ReferenceKind::Variable {
                return Some(Change::NewConnection);
            }
            self.quick_variable = Some(QuickVariable {
                node: node.id,
                pointer: pointer.to_owned(),
                name: String::new(),
                ty: COMMON_TYPES[1].to_owned(),
            });
        }
        if kind == ReferenceKind::Variable
            && let Some(form) = self
                .quick_variable
                .as_mut()
                .filter(|form| form.pointer == pointer)
        {
            chosen = Self::quick_variable_form(ui, labels, form).or(chosen);
        }
        chosen
    }

    /// The short form for a variable declared from the node that will name it: a name
    /// and a type. The Document tab has the whole form.
    fn quick_variable_form(
        ui: &mut egui::Ui,
        labels: f32,
        form: &mut QuickVariable,
    ) -> Option<Change> {
        let mut change = None;
        let _ = egui::Frame::group(ui.style()).show(ui, |ui| {
            row(ui, labels, "Name", None, |ui| {
                let _ = ui.add(
                    egui::TextEdit::singleline(&mut form.name)
                        .hint_text("new variable")
                        .desired_width(ui.available_width()),
                );
            });
            row(ui, labels, "Type", None, |ui| {
                let _ = egui::ComboBox::from_id_salt("orange-quick-variable-type")
                    .selected_text(type_name(&form.ty))
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        for ty in COMMON_TYPES {
                            let _ = ui.selectable_value(&mut form.ty, ty.to_owned(), type_name(ty));
                        }
                    });
            });
            ui.horizontal(|ui| {
                let ready = !form.name.trim().is_empty();
                if ui
                    .add_enabled(ready, egui::Button::new("Declare"))
                    .clicked()
                {
                    change = Some(Change::Declare(form.pointer.clone(), form.variable()));
                }
                if ui.button("Cancel").clicked() {
                    change = Some(Change::CancelDeclare);
                }
            });
            ui.weak("Scope and starting value are under Document.");
        });
        change
    }

    /// Fields for the settings the node's schema describes, but not those naming a
    /// declaration.
    fn settings_fields(
        &mut self,
        ui: &mut egui::Ui,
        labels: f32,
        node: &NodeView,
    ) -> Option<Change> {
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
            let needed = required.contains(&name.as_str());
            let label = if matches!(field, Field::Time) {
                format!("{} (ms)", words(name))
            } else {
                words(name)
            };
            let hover = Some(if needed { "Required." } else { "Optional." }.to_owned());
            let mut said = None;
            row(ui, labels, &label, hover, |ui| match &field {
                Field::Choice(options) => {
                    let shown = current.as_str().unwrap_or("not set").to_owned();
                    let _ = egui::ComboBox::from_id_salt(("orange-setting", name.as_str()))
                        .selected_text(shown)
                        .width(ui.available_width())
                        .show_ui(ui, |ui| {
                            if !needed
                                && ui.selectable_label(current.is_null(), "not set").clicked()
                                && !current.is_null()
                            {
                                changed = Some(Change::Setting(pointer.clone(), None));
                            }
                            for option in options {
                                let on = current.as_str() == Some(*option);
                                if ui.selectable_label(on, *option).clicked() && !on {
                                    changed =
                                        Some(Change::Setting(pointer.clone(), Some(json!(option))));
                                }
                            }
                        });
                }
                Field::Toggle => {
                    let mut on = current.as_bool().unwrap_or(false);
                    if ui.checkbox(&mut on, "").changed() {
                        changed = Some(Change::Setting(pointer.clone(), Some(json!(on))));
                    }
                }
                Field::Whole | Field::Number | Field::Time | Field::Text => {
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
                self.say(format!("not done: {why}"));
            }
        }
        changed
    }

    /// Choosers of the types of a generic node's parameters.
    fn type_fields(ui: &mut egui::Ui, labels: f32, node: &NodeView) -> Option<Change> {
        let mut typing = None;
        for param in &node.types {
            let shown = param
                .chosen
                .as_deref()
                .map_or_else(|| "from the wires".to_owned(), type_name);
            let hover = param.found.as_deref().map_or_else(
                || "No type found yet.".to_owned(),
                |found| format!("Now {}.", type_name(found)),
            );
            let label = format!("Type {}", param.name);
            row(ui, labels, &label, Some(hover), |ui| {
                let _ = egui::ComboBox::from_id_salt(("orange-type", param.name.as_str()))
                    .selected_text(shown)
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(param.chosen.is_none(), "from the wires")
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
    fn variadic_fields(ui: &mut egui::Ui, labels: f32, node: &NodeView) -> Option<Change> {
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
                Some(max) => format!("From {} to {max}.", variadic.min),
                None => format!("At least {}.", variadic.min),
            };
            row(ui, labels, &words(&variadic.port), Some(hover), |ui| {
                ui.label(variadic.count.to_string());
                let fewer = variadic.count > variadic.min && last.is_some();
                if ui
                    .add_enabled(fewer, egui::Button::new("−"))
                    .on_hover_text("Take off the last one, with its wires.")
                    .clicked()
                    && let Some(last) = &last
                {
                    change = Some(Change::RemovePort(last.clone()));
                }
                let more = variadic.max.is_none_or(|max| variadic.count < max);
                if ui
                    .add_enabled(more, egui::Button::new("+"))
                    .on_hover_text("Add one more.")
                    .clicked()
                {
                    change = Some(Change::AddPort(variadic.port.clone()));
                }
            });
        }
        change
    }

    /// Fields for the values on the node's inputs.
    fn input_fields(&mut self, ui: &mut egui::Ui, labels: f32, node: &NodeView) -> Option<Change> {
        let mut write = None;
        for port in node.ports.iter().filter(|port| port.input && !port.exec) {
            let ty = port.ty.as_deref().map_or("value".to_owned(), type_name);
            row(ui, labels, &port_label(&port.id), Some(ty.clone()), |ui| {
                if port.connected {
                    ui.weak("from a wire");
                    return;
                }
                if port.ty.as_deref() == Some("core/bool@1") {
                    let mut on = port.value.as_deref() == Some("true");
                    if ui.checkbox(&mut on, "").changed() {
                        write = Some(Change::Input(port.id.clone(), Some(on.to_string())));
                    }
                    if port.value.is_some()
                        && ui
                            .small_button("×")
                            .on_hover_text("Take the value off.")
                            .clicked()
                    {
                        write = Some(Change::Input(port.id.clone(), None));
                    }
                    return;
                }
                let before = Self::shown_input(port);
                // A path is chosen with the system's chooser, where the application
                // has one.
                let path = node
                    .paths
                    .iter()
                    .find(|path| path.port == port.id)
                    .filter(|_| self.file_dialogs);
                let browse = path.map(|path| if path.folder { "folder" } else { "file" });
                let typed = self.typed.entry(port.id.clone()).or_default();
                match text_field(ui, typed, &ty, port.value.is_some(), browse) {
                    Edit::Done if *typed != before => {
                        let value = (!typed.is_empty()).then(|| typed.clone());
                        write = Some(Change::Input(port.id.clone(), value));
                    }
                    Edit::Clear => write = Some(Change::Input(port.id.clone(), None)),
                    Edit::Browse => {
                        if let Some(path) = path {
                            self.request = Some(crate::FileRequest::ChoosePath {
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
        write
    }
}

/// Widest the label column of the properties panel gets.
const LABEL_WIDTH: f32 = 120.0;

/// A variable being declared from a node's properties, for the node to name.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct QuickVariable {
    node: NodeId,
    pointer: String,
    name: String,
    ty: String,
}

impl QuickVariable {
    /// A form for `node`'s `/variable`, with a name typed.
    #[cfg(test)]
    pub(crate) fn for_test(node: NodeId) -> Self {
        Self {
            node,
            pointer: "/variable".to_owned(),
            name: "speed".to_owned(),
            ty: COMMON_TYPES[2].to_owned(),
        }
    }

    /// The variable to declare: one for each run, with no starting value.
    fn variable(&self) -> Variable {
        Variable {
            id: None,
            name: self.name.trim().to_owned(),
            ty: self.ty.clone(),
            scope: VarScope::Run,
            persistent: false,
            initial: None,
        }
    }
}

/// What a person asked for in the properties panel.
enum Change {
    Setting(String, Option<Json>),
    Type(String, Option<String>),
    Input(String, Option<String>),
    AddPort(String),
    RemovePort(String),
    /// Declare a variable and name it at the setting.
    Declare(String, Variable),
    CancelDeclare,
    NewConnection,
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
                .small_button("×")
                .on_hover_text("Take the value off.")
                .clicked();
        let browsed = browse.is_some_and(|what| {
            ui.small_button("…")
                .on_hover_text(format!("Choose a {what}."))
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

/// One row of the panel: a label in a column `labels` wide, then the field filling
/// the rest. A label too long for the column is shortened and shown whole on hover,
/// with `hover` under it.
fn row(
    ui: &mut egui::Ui,
    labels: f32,
    label: &str,
    hover: Option<String>,
    field: impl FnOnce(&mut egui::Ui),
) {
    let _ = ui.horizontal(|ui| {
        let height = ui.spacing().interact_size.y;
        let _ = ui
            .allocate_ui_with_layout(
                egui::vec2(labels, height),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(labels);
                    ui.add(egui::Label::new(label).truncate())
                },
            )
            .inner
            .on_hover_text(match hover {
                Some(more) => format!("{label}\n{more}"),
                None => label.to_owned(),
            });
        field(ui);
    });
}

/// A setting's name as a person reads it: `max_iterations` is "Max iterations".
pub(crate) fn words(name: &str) -> String {
    let mut text = name.replace(['_', '-'], " ");
    if let Some(first) = text.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    text
}

/// A port's name as a card or the panel shows it: an instance of a variadic port
/// (`branch#0`) is counted from one ("branch 1"), and words are spaced.
pub fn port_label(id: &str) -> String {
    match id.split_once('#') {
        Some((name, instance)) => match instance.parse::<u32>() {
            Ok(number) => format!("{} {}", name.replace(['_', '-'], " "), number + 1),
            Err(_) => id.to_owned(),
        },
        None => id.replace(['_', '-'], " "),
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

#[cfg(test)]
mod width_tests {
    use super::*;

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
                        row(ui, 80.0, "Value", None, |ui| {
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
}

impl Editor {
    /// What the last run did at the node: why its last step failed, and the value on
    /// each input it received and each output it gave, in full and copyable. Values from
    /// a run that has ended, or from before an edit, are said to be so.
    fn values_section(&self, ui: &mut egui::Ui, node: &NodeView) {
        let Some(graph) = self.graph() else {
            return;
        };
        let failure = self.canvas.failure(node.id);
        let ports: Vec<_> = node
            .ports
            .iter()
            .filter(|port| !port.exec)
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
        section(ui, "Values");
        if let Some(why) = failure {
            ui.colored_label(ui.visuals().error_fg_color, format!("Failed: {why}"));
        }
        for (port, shown, stale) in ports {
            let side = if port.input { "in" } else { "out" };
            let freshness = if stale { " (previous)" } else { "" };
            let title = format!("{} ({side}) = {}{freshness}", port_label(&port.id), shown.short);
            egui::CollapsingHeader::new(egui::RichText::new(title).monospace())
                .id_salt(("orange-value", node.id, port.id.as_str(), port.input))
                .show(ui, |ui| {
                    let mut full = shown.full.clone();
                    let _ = ui.add(
                        egui::TextEdit::multiline(&mut full)
                            .font(egui::TextStyle::Monospace)
                            .desired_rows(1)
                            .desired_width(f32::INFINITY)
                            .interactive(true),
                    );
                    if ui.small_button("Copy").clicked() {
                        ui.ctx().copy_text(shown.full.clone());
                    }
                });
        }
    }
}
