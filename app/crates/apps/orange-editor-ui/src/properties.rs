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
    WatchNumber {
        whole: bool,
    },
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

    fn for_node(node: &NodeView, name: &str, schema: &'a Json) -> Self {
        if node.definition == "core/on-watch" && name != "mode" {
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
                .map(Json::from)
                .map_err(|_| tf("not-a-number", &[("typed", typed.into())])),
            Self::WatchNumber { whole } => watch_number_value(typed, *whole),
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
                .ok_or_else(|| tf("not-milliseconds", &[("typed", typed.into())])),
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
    let fields: &[&str] = if node.definition == "core/on-watch" {
        watch_fields(node.settings["mode"].as_str())
    } else {
        &[]
    };
    fields
        .iter()
        .filter_map(move |name| properties?.get_key_value(*name))
}

pub(crate) fn watch_label(name: &str) -> &'static str {
    match name {
        "changed" => t("when-changed"),
        "rising" => t("when-rising"),
        "above" => t("when-above"),
        "below" => t("when-below"),
        "inside" => t("when-inside"),
        "outside" => t("when-outside"),
        "threshold" => t("when-threshold"),
        "release" => t("when-release"),
        "low" => t("when-low"),
        "high" => t("when-high"),
        "hold" => t("when-hold"),
        "cooldown" => t("when-cooldown"),
        _ => t("when-mode"),
    }
}

impl Editor {
    /// The configured root for a folder-scoped file node's path picker.
    pub fn file_picker_root(&self, node: NodeId) -> Option<Option<String>> {
        let node = self.graph()?.nodes.iter().find(|view| view.id == node)?;
        if !node.definition.starts_with("org.orange.files/") {
            return None;
        }
        Some(
            self.named_connection(node)
                .and_then(|connection| connection.settings["path"].as_str().map(str::to_owned)),
        )
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
        let watch = found.is_some_and(|found| found.definition == "core/on-watch");
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

    /// The declarations of one kind a node can name, as (ID, name).
    /// Connections are those of kind `adapter` when the node says which it uses.
    pub(crate) fn choices(
        &self,
        kind: ReferenceKind,
        adapter: Option<&str>,
    ) -> Vec<(String, String)> {
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
        egui::Window::new(label)
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
                let typed =
                    Field::for_node(node, name, schema).typed(&node.settings[name.as_str()]);
                let _ = self.typed.insert(setting_key(&pointer), typed);
            }
        }
    }

    /// The value on an input as its field shows it before typing: text without quotes.
    pub(crate) fn shown_input(port: &orange_host::PortView) -> String {
        let shown = port.value.clone().unwrap_or_default();
        serde_json::from_str::<String>(&shown).unwrap_or(shown)
    }

    pub(crate) fn properties_panel(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        // What the panel holds never widens it; only dragging its edge does. Text is cut
        // short with an ellipsis and shown whole on hover.
        ui.set_max_width(ui.available_width());
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
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
                ui.strong(tf("nodes-selected", &[("count", many.len().into())]));
                ui.label(t("selection-copy-hint"));
                if ui.button(t("delete-them")).clicked() {
                    let _ = self.remove_nodes(host, &many);
                }
            } else {
                ui.weak(t("select-node-hint"));
                ui.weak(t("multi-select-hint"));
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
            .add(
                egui::Label::new(egui::RichText::new(self.canvas.display_name(&node)).heading())
                    .truncate(),
            )
            .on_hover_text(node.name.as_str());
        let mut subtitle = self.canvas.kind_name(&node).to_owned();
        if let Some(detail) = &node.detail {
            subtitle.push_str(" · ");
            subtitle.push_str(detail);
        }
        let _ = ui
            .add(egui::Label::new(egui::RichText::new(subtitle).weak()).truncate())
            .on_hover_text(node.definition.as_str());
        if let Some(description) = self.canvas.description(&node.definition) {
            ui.add_space(4.0);
            let _ = ui.add(egui::Label::new(description).wrap());
        }
        ui.separator();
        // What to do once the fields are drawn: one change a frame.
        let mut change = None;
        // The kind of connection the node uses, and the one it names with its choices.
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
                let has_settings = node
                    .schema
                    .as_ref()
                    .and_then(|schema| schema["properties"].as_object())
                    .is_some_and(|fields| !fields.is_empty());
                let setup = !node.references.is_empty() || has_settings || !node.types.is_empty();
                if setup {
                    section(ui, t("set-up"));
                    for (pointer, kind) in &node.references {
                        if let Some(asked) = self.reference_field(ui, &node, host, pointer, *kind) {
                            change = Some(asked);
                        }
                    }
                    if let Some(connection) = &connection {
                        let mut current = connection.settings.clone();
                        if let Some((name, value)) = declarations::choice_fields(
                            ui,
                            "node",
                            &choices,
                            &current,
                            &mut refresh,
                        ) {
                            current = declarations::with_setting(&current, &name, value);
                        }
                        let fields = host.adapter_fields(&connection.adapter);
                        let texts: Vec<_> = host
                            .adapter_texts(&connection.adapter)
                            .into_iter()
                            .filter(|field| !field.secret)
                            .collect();
                        let toggles = host.adapter_toggles(&connection.adapter);
                        if !fields.is_empty() || !texts.is_empty() || !toggles.is_empty() {
                            egui::CollapsingHeader::new(t("source-settings"))
                                .id_salt(("source-settings", node.id))
                                .show(ui, |ui| {
                                    if let Some((name, value)) =
                                        declarations::number_fields(ui, &fields, &current)
                                    {
                                        current = declarations::with_number(&current, &name, value);
                                    }
                                    declarations::text_fields(
                                        ui,
                                        &texts,
                                        &mut current,
                                        &mut declarations::Typed::default(),
                                    );
                                    declarations::toggle_fields(ui, &toggles, &mut current);
                                });
                        }
                        if current != connection.settings {
                            redeclare = Some(Connection {
                                settings: current,
                                ..connection.clone()
                            });
                        }
                        ui.small(tf(
                            "shared-settings",
                            &[("name", connection.name.as_str().into())],
                        ));
                    }
                    if let Some(asked) = self.settings_fields(ui, &node, false) {
                        change = Some(asked);
                    }
                }
                if !node.variadic.is_empty()
                    && node.definition != "org.orange.audio/multiband-spectrum"
                {
                    section(ui, t("ports"));
                    if let Some(asked) = Self::variadic_fields(ui, &node) {
                        change = Some(asked);
                    }
                }
                if node.ports.iter().any(|port| port.input && !port.exec) {
                    section(ui, t("inputs"));
                    if let Some(asked) = self.input_fields(ui, &node) {
                        change = Some(asked);
                    }
                }
                // Tuning settings and type pins: rarely changed, never on the card.
                let tuning = Self::has_tuning(&node);
                if tuning || !node.types.is_empty() {
                    egui::CollapsingHeader::new(egui::RichText::new(t("advanced")).strong())
                        .id_salt(("orange-advanced", node.id))
                        .show(ui, |ui| {
                            if tuning && let Some(asked) = self.settings_fields(ui, &node, true) {
                                change = Some(asked);
                            }
                            if let Some(asked) = Self::type_fields(ui, &node) {
                                change = Some(asked);
                            }
                        });
                }
                self.values_section(ui, &node);
                ui.add_space(10.0);
                if ui.button(t("delete-node")).clicked() {
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
        node: &NodeView,
        host: &Host,
        pointer: &str,
        kind: ReferenceKind,
    ) -> Option<Change> {
        let named = node
            .settings
            .pointer(pointer)
            .and_then(Json::as_str)
            .map(str::to_owned);
        let adapter = host
            .nodes()
            .find(|info| info.definition.definition.id.as_str() == node.definition)
            .and_then(|info| info.presentation?.connection_adapter.as_deref());
        let declared = self.choices(kind, adapter);
        let connection_label = (kind == ReferenceKind::Connection)
            .then(|| host.connection_label(&node.definition))
            .flatten();
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
                || {
                    connection_label.as_ref().map_or_else(
                        || tf("choose-reference", &[("kind", what.into())]),
                        |label| tf("choose-named", &[("name", label.as_str().into())]),
                    )
                },
                |(_, name)| name.clone(),
            );
        let label = words(pointer.trim_start_matches('/'));
        let mut chosen = None;
        let mut quick = false;
        row(
            ui,
            connection_label.as_deref().unwrap_or(&label),
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
                                quick = ui
                                    .selectable_label(
                                        false,
                                        tf("new-reference", &[("kind", what.into())]),
                                    )
                                    .clicked();
                            }
                            _ if declared.is_empty() => {
                                ui.weak(tf("no-references", &[("kind", what.into())]));
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
            chosen = Self::quick_variable_form(ui, form).or(chosen);
        }
        chosen
    }

    /// The short form for a variable declared from the node that will name it: a name
    /// and a type. The Document tab has the whole form.
    fn quick_variable_form(ui: &mut egui::Ui, form: &mut QuickVariable) -> Option<Change> {
        let mut change = None;
        let _ = egui::Frame::group(ui.style()).show(ui, |ui| {
            row(ui, t("name"), None, |ui| {
                let _ = ui.add(
                    egui::TextEdit::singleline(&mut form.name)
                        .hint_text(t("new-variable-placeholder"))
                        .desired_width(ui.available_width()),
                );
            });
            row(ui, t("type"), None, |ui| {
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
                    .add_enabled(ready, egui::Button::new(t("declare")))
                    .clicked()
                {
                    change = Some(Change::Declare(form.pointer.clone(), form.variable()));
                }
                if ui.button(t("cancel")).clicked() {
                    change = Some(Change::CancelDeclare);
                }
            });
            ui.weak(t("scope-under-document"));
        });
        change
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
        let watch = node.definition == "core/on-watch";
        let fields = watch_field_entries(node).chain(properties.iter().filter(|_| !watch));
        for (name, schema) in fields {
            if name == "bands" && node.definition == "org.orange.audio/multiband-spectrum" {
                continue;
            }
            let pointer = format!("/{name}");
            if node.references.iter().any(|(at, _)| *at == pointer)
                || crate::canvas::advanced_setting(name) != tuning
            {
                continue;
            }
            let field = Field::for_node(node, name, schema);
            let current = &node.settings[name.as_str()];
            let needed = required.contains(&name.as_str())
                || (watch && matches!(name.as_str(), "threshold" | "low" | "high"));
            let measurement_time =
                name == "window_ms" && node.definition.starts_with("org.orange.audio/");
            let label = if watch {
                watch_label(name).to_owned()
            } else if measurement_time {
                t("audio-setting-window-ms").to_owned()
            } else if matches!(field, Field::Time) {
                tf("in-milliseconds", &[("name", words(name).into())])
            } else {
                words(name)
            };
            let hover = Some(
                if measurement_time {
                    t("audio-setting-window-help")
                } else if needed {
                    t("required")
                } else {
                    t("optional")
                }
                .to_owned(),
            );
            let mut said = None;
            row(ui, &label, hover, |ui| match &field {
                Field::Choice(options) => {
                    let shown = current.as_str().map_or(t("not-set"), |mode| {
                        if watch { watch_label(mode) } else { mode }
                    });
                    let _ = egui::ComboBox::from_id_salt(("orange-setting", name.as_str()))
                        .selected_text(shown)
                        .width(ui.available_width())
                        .show_ui(ui, |ui| {
                            if !needed
                                && ui
                                    .selectable_label(current.is_null(), t("not-set"))
                                    .clicked()
                                && !current.is_null()
                            {
                                changed = Some(Change::Setting(pointer.clone(), None));
                            }
                            for option in options {
                                let on = current.as_str() == Some(*option);
                                let label = if watch { watch_label(option) } else { *option };
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
                | Field::Text => {
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
                    .any(|name| crate::canvas::advanced_setting(name))
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
                    .show_ui(ui, |ui| {
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
    fn input_fields(&mut self, ui: &mut egui::Ui, node: &NodeView) -> Option<Change> {
        let mut write = None;
        for port in node.ports.iter().filter(|port| port.input && !port.exec) {
            let ty = port.ty.as_deref().map_or("value".to_owned(), type_name);
            row(ui, &port_label(&port.id), Some(ty.clone()), |ui| {
                if port.connected {
                    ui.weak(t("from-a-wire"));
                    return;
                }
                if let Some(choices) = port.ty.as_deref().and_then(|ty| self.canvas.enums.get(ty)) {
                    let _ = egui::ComboBox::from_id_salt(("orange-choice", port.id.as_str()))
                        .width(ui.available_width())
                        .selected_text(port.value.as_deref().unwrap_or(t("choose")))
                        .show_ui(ui, |ui| {
                            for choice in choices.iter().filter(|choice| !choice.payload) {
                                let current = port.value.as_deref() == Some(choice.name.as_str());
                                if ui.selectable_label(current, &choice.name).clicked() && !current
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
                let hint = port.default.as_deref().unwrap_or(&ty);
                match text_field(ui, typed, hint, port.value.is_some(), browse) {
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

/// A variable being declared from a node's properties, for the node to name.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct QuickVariable {
    node: NodeId,
    pointer: String,
    name: String,
    ty: String,
}

impl QuickVariable {
    /// An empty form for the variable `node` names at `pointer`.
    pub(crate) fn new(node: NodeId, pointer: &str) -> Self {
        Self {
            node,
            pointer: pointer.to_owned(),
            name: String::new(),
            ty: COMMON_TYPES[1].to_owned(),
        }
    }

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
    let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
    let mut clicked = None;
    for (value, label) in [(false, t("no")), (true, t("yes"))] {
        let chosen = on == Some(value);
        if ui
            .add_sized(
                egui::vec2(width, ui.spacing().interact_size.y),
                egui::Button::selectable(chosen, label),
            )
            .clicked()
            && !chosen
        {
            clicked = Some(value);
        }
    }
    clicked
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
/// A port the host's catalogues name in the current language shows that name.
pub fn port_label(id: &str) -> String {
    let name = |id: &str| crate::i18n::port_name(id).unwrap_or_else(|| id.replace(['_', '-'], " "));
    match id.split_once('#') {
        Some((base, instance)) => match instance.parse::<u32>() {
            Ok(number) => format!("{} {}", name(base), number + 1),
            Err(_) => id.to_owned(),
        },
        None => name(id),
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

    #[test]
    fn when_fields_keep_their_order_and_exact_integer_values() {
        let (host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        let watch = host
            .add_node(&mut draft, "core/on-watch", [0.0, 0.0])
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
        editor
            .canvas()
            .show_preview(play, "path".into(), long.clone(), long.clone());
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
        section(ui, t("values"));
        if let Some(why) = failure {
            ui.colored_label(
                ui.visuals().error_fg_color,
                tf("failed", &[("why", why.to_string().into())]),
            );
        }
        for (port, shown, stale) in ports {
            let side = if port.input { "in" } else { "out" };
            let freshness = if stale { t("previous-suffix") } else { "" };
            let title = format!(
                "{} ({side}) = {}{freshness}",
                port_label(&port.id),
                shown.short
            );
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
                    if ui.small_button(t("copy")).clicked() {
                        ui.ctx().copy_text(shown.full.clone());
                    }
                });
        }
    }
}
