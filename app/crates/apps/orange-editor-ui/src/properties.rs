//! The properties panel: what the selected node settles (the declarations it names,
//! its settings, the types of a generic node) and the values on its inputs.
use super::*;
use orange_host::{Connection, NodeView, ReferenceKind, VarScope, Variable};
use serde_json::{Value as Json, json};

const HTTP_METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

pub(crate) fn choose_http_method(ui: &mut egui::Ui, shown: &str) -> Option<&'static str> {
    let mut chosen = None;
    for method in HTTP_METHODS {
        if ui.selectable_label(shown == method, method).clicked() && shown != method {
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
            | "org.orange.net/http-stream"
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TextInput {
    Text,
    Message,
    Body,
    Headers,
    HeaderList,
    JsonPath,
}

impl TextInput {
    pub(crate) fn for_port(node: &NodeView, port: &orange_host::PortView) -> Option<Self> {
        if node.paths.iter().any(|path| path.port == port.id)
            || (http_request_variant(node) && port.id == "method")
        {
            return None;
        }
        match port.id.as_str() {
            "content" if http_request(node) => Some(Self::Body),
            "headers" if http_request(node) => Some(Self::Headers),
            "headers" if http_request_variant(node) => Some(Self::HeaderList),
            "path" if node.definition == "org.orange.json/get-path" => Some(Self::JsonPath),
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

pub(crate) fn http_request_label(port: &str) -> Option<&'static str> {
    Some(t(match port {
        "method" => "http-method",
        "content" => "http-body",
        "path" => "http-path",
        "content_type" => "http-content-type",
        "headers" => "http-headers",
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

pub(crate) fn noise_gate_label(node: &NodeView, setting: &str) -> Option<&'static str> {
    if node.definition != "org.orange.audio/noise-gate" {
        return None;
    }
    let key = match setting {
        "threshold_db" => "audio-setting-threshold-db",
        "reset_db" => "noise-gate-reset",
        "attack_ms" => "noise-gate-attack",
        "hold_ms" => "audio-setting-hold-ms",
        "release_ms" => "noise-gate-release",
        _ => return None,
    };
    Some(t(key))
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

    /// The declared variables as (ID, name).
    pub(crate) fn variable_choices(&self) -> Vec<(String, String)> {
        self.declarations()
            .into_iter()
            .flat_map(|declared| &declared.variables)
            .filter_map(|variable| Some((variable.id.clone()?, variable.name.clone())))
            .collect()
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
        self.inspect(&node, host);
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
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let http = http_request_variant(&node);
                if http {
                    self.values_section(ui, &node);
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
                let setup =
                    references.clone().next().is_some() || has_settings || !node.types.is_empty();
                if setup {
                    section(ui, t("set-up"));
                    for (pointer, kind) in references {
                        if let Some(asked) = self.reference_field(ui, &node, pointer, *kind) {
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
                    if let Some(asked) = Self::variadic_fields(ui, &node) {
                        change = Some(asked);
                    }
                }
                if node.ports.iter().any(|port| port.input && !port.exec) {
                    section(ui, t("inputs"));
                    if let Some(asked) = self.input_fields(ui, &node, host) {
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
                if !http {
                    self.values_section(ui, &node);
                }
                ui.add_space(10.0);
                if ui.button(t("delete-node")).clicked() {
                    change = Some(Change::Delete);
                }
            });
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
        pointer: &str,
        kind: ReferenceKind,
    ) -> Option<Change> {
        let named = node
            .settings
            .pointer(pointer)
            .and_then(Json::as_str)
            .map(str::to_owned);
        let declared = if kind == ReferenceKind::Variable {
            self.variable_choices()
        } else {
            Vec::new()
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
        let label = words(pointer.trim_start_matches('/'));
        let mut chosen = None;
        let mut quick = false;
        row(ui, &label, None, |ui| {
            let _ = egui::ComboBox::from_id_salt(("orange-reference", pointer))
                .selected_text(shown)
                .wrap_mode(egui::TextWrapMode::Truncate)
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for (id, name) in &declared {
                        let current = named.as_deref() == Some(id.as_str());
                        if ui.selectable_label(current, name.as_str()).clicked() && !current {
                            chosen = Some(Change::Setting(pointer.to_owned(), Some(json!(id))));
                        }
                    }
                    if kind == ReferenceKind::Variable {
                        if !declared.is_empty() {
                            ui.separator();
                        }
                        quick = ui
                            .selectable_label(false, tf("new-reference", &[("kind", what.into())]))
                            .clicked();
                    } else if declared.is_empty() {
                        ui.weak(tf("no-references", &[("kind", what.into())]));
                    }
                });
        });
        if quick {
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
                || canvas::advanced_setting(name) != tuning
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
            } else if let Some(label) = noise_gate_label(node, name) {
                if matches!(field, Field::Time) {
                    tf("in-milliseconds", &[("name", label.into())])
                } else {
                    label.to_owned()
                }
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
            .is_some_and(|fields| fields.keys().any(|name| canvas::advanced_setting(name)))
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
    fn input_fields(&mut self, ui: &mut egui::Ui, node: &NodeView, host: &Host) -> Option<Change> {
        let mut write = None;
        for port in node.ports.iter().filter(|port| port.input && !port.exec) {
            let ty = port.ty.as_deref().map_or("value".to_owned(), type_name);
            let multiline = TextInput::for_port(node, port).filter(|kind| *kind != TextInput::Text);
            let label = http_request_variant(node)
                .then(|| http_request_label(&port.id))
                .flatten()
                .map_or_else(|| port_label(&port.id), str::to_owned);
            row(ui, &label, Some(ty.clone()), |ui| {
                if port.connected {
                    ui.weak(t("from-a-wire"));
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
                        .selected_text(&shown)
                        .show_ui(ui, |ui| {
                            if let Some(method) = choose_http_method(ui, &shown) {
                                write =
                                    Some(Change::Input(port.id.clone(), Some(method.to_owned())));
                            }
                        });
                    return;
                }
                if let Some(multiline) = multiline {
                    let typed = self.typed.entry(port.id.clone()).or_default();
                    let response = egui::ScrollArea::vertical()
                        .id_salt(("property-text", node.id, &port.id))
                        .max_height(126.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(typed)
                                    .font(egui::TextStyle::Monospace)
                                    .layouter(&mut wrap_property_text)
                                    .hint_text(multiline.hint())
                                    .desired_rows(3)
                                    .desired_width(ui.available_width()),
                            )
                        })
                        .inner;
                    if response.lost_focus() {
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
    let name = |id: &str| i18n::port_name(id).unwrap_or_else(|| id.replace(['_', '-'], " "));
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
        let title = format!("{} · {}", self.canvas.display_name(view), port_label(port));
        let mut open = true;
        egui::Window::new(title)
            .id(egui::Id::unique("orange-port-inspector"))
            .default_width(480.0)
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
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
    fn values_section(&self, ui: &mut egui::Ui, node: &NodeView) {
        let Some(graph) = self.graph() else {
            return;
        };
        let http = http_request_variant(node);
        let failure = self.canvas.failure(node.id);
        let ports: Vec<_> = node
            .ports
            .iter()
            .filter(|port| !port.exec && (!http || !port.input))
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
            ui.colored_label(
                ui.visuals().error_fg_color,
                tf("failed", &[("why", why.to_string().into())]),
            );
        }
        for (port, shown, stale) in ports {
            let side = if port.input { "in" } else { "out" };
            let freshness = if stale { t("previous-suffix") } else { "" };
            let response = http && !port.input && port.id == "response";
            let title = if response {
                format!("{}{freshness}", port_label(&port.id))
            } else if shown.short == shown.full
                && shown.short.chars().take(17).count() <= 16
                && !shown.short.contains(['\n', '\r'])
            {
                format!(
                    "{} ({side}) = {}{freshness}",
                    port_label(&port.id),
                    shown.short
                )
            } else {
                format!("{} ({side}){freshness}", port_label(&port.id))
            };
            egui::CollapsingHeader::new(egui::RichText::new(title).monospace())
                .id_salt(("orange-value", node.id, port.id.as_str(), port.input))
                .default_open(response)
                .show(ui, |ui| {
                    let copyable = self
                        .canvas
                        .copyable_port_text(graph, node.id, &port.id, port.input)
                        .unwrap_or(&shown.full);
                    display_port_text(ui, &shown.full, copyable, 126.0);
                });
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
