//! The Variables and Connections tabs: what the open document declares, with forms to
//! add, change and remove it. A connection's settings are drawn from what its adapter
//! says about them; a secret is typed masked, kept in the host's credential store and
//! written into the document only as its `credential://<name>` reference.
use super::*;
use crate::controls::{Action, Dropdown as _, Surface, button};
use crate::properties::COMMON_TYPES;
use orange_host::{
    AdapterInfo, Connection, ConnectionScope, Declarations, SECRET_PREFIX, SettingChoices,
    SettingField, SettingText, SettingToggle, VarScope, Variable,
};
use std::collections::BTreeMap;

pub(crate) mod network;
pub(crate) use network::HTTP;
const FOLDER_ADAPTER: &str = "org.orange.files/folder";

/// How an event's payload is named in lists: its type, or "No payload" for an
/// event-only channel (an empty type).
pub(crate) fn payload_name(ty: &str) -> String {
    if ty.is_empty() {
        t("event-no-payload").to_owned()
    } else {
        type_name(ty)
    }
}

pub(crate) fn variable_scope_allowed(definition: &str, scope: VarScope) -> bool {
    match definition {
        "core/set-shared" | "core/update-shared" => scope == VarScope::Workspace,
        "core/set-variable" | "core/update-variable" => scope != VarScope::Workspace,
        "core/on-variable-changed" => scope == VarScope::Run,
        _ => true,
    }
}

fn adapter_rank(id: &str) -> usize {
    [
        HTTP,
        "org.orange.net/websocket",
        "org.orange.net/amqp",
        "org.orange.net/mqtt",
        FOLDER_ADAPTER,
        "org.orange.audio/input",
        "org.orange.audio/output",
    ]
    .iter()
    .position(|known| *known == id)
    .unwrap_or(usize::MAX)
}

fn adapter_label(adapter: &AdapterInfo) -> &str {
    match adapter.id.as_str() {
        "org.orange.audio/input" => t("connection-audio-source"),
        "org.orange.audio/output" => t("connection-audio-output"),
        _ => &adapter.label,
    }
}

/// What a variable dragged from the Variables list carries, before its ID. Let go on
/// the canvas, it becomes a Get node, or a Set node with Alt held.
pub(crate) const VARIABLE_DRAG: &str = "orange-variable:";

/// Secrets typed into the connection form, by setting, until it is saved. Never shown
/// in debug output.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Typed {
    values: BTreeMap<String, String>,
    revealed: std::collections::BTreeSet<String>,
    local_status: BTreeMap<String, &'static str>,
}
impl std::fmt::Debug for Typed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(
                self.values
                    .keys()
                    .map(|setting| format!("{setting}: <redacted>")),
            )
            .finish()
    }
}
use serde_json::Value as Json;

/// What is being typed into the variable form.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VariableForm {
    /// The variable being changed; `None` for a new one.
    changing: Option<String>,
    name: String,
    ty: String,
    scope: VarScope,
    persistent: bool,
    initial: String,
}

impl Default for VariableForm {
    fn default() -> Self {
        Self {
            changing: None,
            name: String::new(),
            ty: "core/i64@1".to_owned(),
            scope: VarScope::Run,
            persistent: false,
            initial: String::new(),
        }
    }
}

/// What is being typed into the connection form.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ConnectionForm {
    changing: Option<String>,
    name: String,
    adapter: String,
    scope: ConnectionScope,
    /// The settings as JSON text.
    settings: String,
    /// Secrets typed for settings, stored when the form is saved.
    secrets: Typed,
    auth: Option<network::Auth>,
    saved: bool,
    /// What testing the connection found.
    tested: Option<Result<(), String>>,
}

impl Default for ConnectionForm {
    fn default() -> Self {
        Self {
            changing: None,
            name: String::new(),
            adapter: String::new(),
            scope: ConnectionScope::Graph,
            settings: "{}".to_owned(),
            secrets: Typed::default(),
            auth: None,
            saved: false,
            tested: None,
        }
    }
}

impl ConnectionForm {
    fn settings_for_save(&self) -> Result<Json, String> {
        let settings: Json = serde_json::from_str(&self.settings)
            .map_err(|_| t("settings-not-json-object").to_owned())?;
        if !settings.is_object() {
            return Err(t("settings-not-json-object").to_owned());
        }
        network::validate(&self.adapter, &settings, &self.secrets, self.auth)
            .map_err(|message| t(message).to_owned())?;
        Ok(settings)
    }

    fn check(&self, host: &Host, mut settings: Json) -> Result<(), String> {
        // Check the shape without looking up a credential from an imported profile.
        for field in host
            .adapter_texts(&self.adapter)
            .into_iter()
            .filter(|field| field.secret)
        {
            if text_at(&settings, &field.setting)
                .and_then(Json::as_str)
                .is_some_and(|text| text.starts_with(SECRET_PREFIX))
            {
                set_text(
                    &mut settings,
                    &field.setting,
                    Some(Json::from("credential-check")),
                );
            }
        }
        for (setting, secret) in &self.secrets.values {
            if !secret.is_empty() {
                set_text(&mut settings, setting, Some(Json::from(secret.as_str())));
            }
        }
        let object = settings
            .as_object()
            .ok_or_else(|| t("settings-not-json-object").to_owned())?;
        host.test_connection(&self.adapter, object)
    }
}

/// The forms of the Document tab.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Forms {
    variable: VariableForm,
    variable_target: Option<(NodeId, String)>,
    event: Option<EventForm>,
    connection: ConnectionForm,
    /// Open the new-variable form at the next frame.
    open_variable: bool,
    /// Whether the Connections panel is showing a form.
    pub(crate) open_connection: bool,
    connection_target: Option<NodeId>,
    /// The variable form is shown.
    variable_open: bool,
    /// What is being typed in place in the Variables list, by variable ID and field,
    /// while the field has the focus.
    inline: BTreeMap<(String, &'static str), String>,
    /// The field to put the keyboard in when it is next drawn, such as a new choice.
    focus: Option<(String, &'static str)>,
}

#[derive(Debug, Clone, PartialEq)]
struct EventForm {
    target: NodeId,
    name: String,
    payload: String,
    failed: bool,
}

impl Forms {
    /// Puts the keyboard in the in-place field `key` when it is next drawn.
    pub(crate) fn focus_inline(&mut self, key: (String, &'static str)) {
        self.focus = Some(key);
    }
    pub(crate) fn reveal_variable(&mut self, target: Option<(NodeId, String)>) {
        self.variable = VariableForm::default();
        self.variable_target = target;
        self.open_variable = true;
    }
    pub(crate) fn reveal_connection(&mut self) {
        self.open_connection = true;
    }
    pub(crate) fn new_connection_for(&mut self, node: NodeId, adapter: &str) {
        self.connection = ConnectionForm {
            adapter: adapter.to_owned(),
            ..ConnectionForm::default()
        };
        self.connection_target = Some(node);
        self.reveal_connection();
    }

    pub(crate) fn edit_connection(&mut self, connection: &Connection) {
        self.connection_target = None;
        self.connection = ConnectionForm {
            changing: connection.id.clone(),
            name: connection.name.clone(),
            adapter: connection.adapter.clone(),
            scope: connection.scope,
            settings: serde_json::to_string_pretty(&connection.settings).unwrap_or_default(),
            ..ConnectionForm::default()
        };
        self.reveal_connection();
    }
}

/// Lists to choose connection settings from, such as an audio connection's device,
/// over `settings`. A refresh button lists the values again. Returns the setting to
/// change and its new value, `None` to leave it out (the adapter's default).
pub(crate) fn choice_fields(
    ui: &mut egui::Ui,
    salt: &str,
    choices: &[SettingChoices],
    settings: &Json,
    refresh: &mut bool,
) -> Option<(String, Option<String>)> {
    let mut chosen = None;
    for choice in choices {
        let current = settings.get(&choice.setting).and_then(Json::as_str);
        // A value is shown by what it is called, such as a device's name for its ID.
        let label = |value: &str| {
            choice
                .values
                .iter()
                .position(|known| known == value)
                .and_then(|at| choice.labels.get(at))
                .map_or_else(
                    || {
                        if choice.setting == "device" {
                            t("device-unavailable")
                        } else {
                            value
                        }
                    },
                    String::as_str,
                )
                .to_owned()
        };
        // Its name over the list, which fills the row beside a Refresh button.
        let _ = ui.add(
            egui::Label::new(
                egui::RichText::new(properties::words(&choice.setting))
                    .small()
                    .weak(),
            )
            .truncate(),
        );
        ui.horizontal(|ui| {
            let shown = current.map_or_else(|| choice.unset.clone(), label);
            let refresh_width = 72.0;
            let _ = egui::ComboBox::from_id_salt(("orange-setting-choice", salt, &choice.setting))
                .width((ui.available_width() - refresh_width).max(60.0))
                .wrap_mode(egui::TextWrapMode::Truncate)
                .selected_text(shown)
                .show_select(ui, Surface::Panel, |ui| {
                    if ui
                        .selectable_label(current.is_none(), choice.unset.as_str())
                        .clicked()
                        && current.is_some()
                    {
                        chosen = Some((choice.setting.clone(), None));
                    }
                    for value in &choice.values {
                        let selected = current == Some(value.as_str());
                        if ui.selectable_label(selected, label(value)).clicked() && !selected {
                            chosen = Some((choice.setting.clone(), Some(value.clone())));
                        }
                    }
                });
            *refresh |= ui
                .small_button(t("refresh"))
                .on_hover_text(t("list-again"))
                .clicked();
        });
        if let Some(problem) = &choice.problem {
            ui.weak(tf(
                "devices-not-listed",
                &[("problem", problem.to_string().into())],
            ));
        }
        if let Some(value) = current
            && !choice.values.iter().any(|known| known == value)
        {
            ui.weak(tf("device-missing", &[("value", value.to_string().into())]));
        }
    }
    chosen
}

/// Fields for the number settings of a connection, over `settings`. A setting left out
/// shows what that means and a button that writes its default. Returns the setting to
/// change and its new value, `None` to leave it out.
pub(crate) fn number_fields(
    ui: &mut egui::Ui,
    fields: &[SettingField],
    settings: &Json,
) -> Option<(String, Option<f64>)> {
    let mut changed = None;
    for field in fields {
        let current = settings.get(&field.setting).and_then(Json::as_f64);
        ui.horizontal(|ui| {
            let _ = ui
                .label(field.label.as_str())
                .on_hover_text(field.help.as_str());
            match current {
                Some(mut value) => {
                    let speed = if field.whole { 1.0 } else { 0.5 };
                    let mut drag = egui::DragValue::new(&mut value)
                        .range(field.min..=field.max)
                        .speed(speed)
                        .suffix(format!(" {}", field.unit));
                    if field.whole {
                        drag = drag.fixed_decimals(0);
                    }
                    if ui.add(drag).changed() {
                        let value = if field.whole { value.round() } else { value };
                        changed = Some((field.setting.clone(), Some(value)));
                    }
                    if ui
                        .small_button(icons::text(icons::CLOSE))
                        .on_hover_text(t("leave-out-default"))
                        .clicked()
                    {
                        changed = Some((field.setting.clone(), None));
                    }
                }
                None => {
                    let start = field.default.unwrap_or((field.min + field.max) / 2.0);
                    match field.default {
                        Some(default) => ui.weak(tf(
                            "default-value",
                            &[
                                ("value", default.to_string().into()),
                                ("unit", field.unit.to_string().into()),
                            ],
                        )),
                        None => ui.weak(t("automatic-value")),
                    }
                    .on_hover_text(field.help.as_str());
                    if ui.small_button(t("set")).clicked() {
                        changed = Some((field.setting.clone(), Some(start)));
                    }
                }
            }
        });
    }
    changed
}

/// A switch that shows what the device of a connection being set up hears now, and
/// the meter while it is on. It listens apart from any run, only while switched on.
pub(crate) fn live_level(
    ui: &mut egui::Ui,
    host: &Host,
    adapter: &str,
    settings: &Json,
    choices: &[SettingChoices],
    fields: &[SettingField],
    probing: &mut Option<Probing>,
) {
    let mut on = probing.is_some();
    if ui
        .checkbox(&mut on, t("show-live-level"))
        .on_hover_text(t("live-level-hint"))
        .changed()
        && !on
    {
        *probing = None;
    }
    if !on {
        return;
    }
    let Some(settings) = settings.as_object() else {
        ui.weak(t("settings-not-json"));
        return;
    };
    // What chooses the device: changing it listens again; moving the trigger's levels
    // does not reopen the device.
    let key: String = choices
        .iter()
        .map(|choice| format!("{:?};", settings.get(&choice.setting)))
        .collect();
    let stale = probing
        .as_ref()
        .is_none_or(|probing| probing.adapter != adapter || probing.settings != key);
    if stale {
        *probing = host.probe(adapter, settings).map(|probe| Probing {
            adapter: adapter.to_owned(),
            settings: key,
            probe,
        });
    }
    let Some(probing) = probing.as_ref() else {
        return;
    };
    let level = match &probing.probe {
        Ok(probe) => probe.read(),
        Err(why) => Err(why.clone()),
    };
    let marks: Vec<(f64, &str)> = fields
        .iter()
        .filter(|field| field.unit == "dBFS")
        .filter_map(|field| {
            let db = settings
                .get(&field.setting)
                .and_then(Json::as_f64)
                .or(field.default)?;
            Some((db, field.label.as_str()))
        })
        .collect();
    level_meter(ui, level, &marks);
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(50));
}

/// Display floor in dBFS, matching the audio package's silence reading.
pub(crate) const AUDIO_SILENCE_DB: f64 = -120.0;

#[expect(
    clippy::cast_possible_truncation,
    reason = "fraction of a screen position"
)]
pub(crate) fn audio_level_fraction(db: f64) -> f32 {
    ((db.clamp(AUDIO_SILENCE_DB, 0.0) - AUDIO_SILENCE_DB) / -AUDIO_SILENCE_DB) as f32
}

/// A bar showing `level` in dBFS, with a line at each of `marks` (a level and what it
/// is), or why there is no level.
pub(crate) fn level_meter(ui: &mut egui::Ui, level: Result<f64, String>, marks: &[(f64, &str)]) {
    let level = match level {
        Ok(level) => level,
        Err(why) => {
            ui.weak(tf("no-level", &[("why", why.to_string().into())]));
            return;
        }
    };
    let width = ui.available_width().max(60.0);
    let height = ui.spacing().interact_size.y * 0.7;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let at = |db| rect.left() + audio_level_fraction(db) * rect.width();
    let visuals = ui.visuals();
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, visuals.extreme_bg_color);
    let filled = egui::Rect::from_min_max(rect.min, egui::pos2(at(level), rect.max.y));
    let loud = marks
        .first()
        .is_some_and(|(threshold, _)| level >= *threshold);
    let color = if loud {
        visuals.warn_fg_color
    } else {
        visuals.selection.bg_fill
    };
    painter.rect_filled(filled, 2.0, color);
    for (db, _) in marks {
        let x = at(*db);
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.5, visuals.strong_text_color()),
        );
    }
    let hover: Vec<String> = marks
        .iter()
        .map(|(db, what)| format!("{what}: {db:.1} dBFS"))
        .collect();
    let _ = response.on_hover_text(hover.join("\n"));
    ui.label(tf("level-now", &[("level", format!("{level:.1}").into())]));
}

/// `settings` with `setting` set to the number `value`, or left out for `None`.
pub(crate) fn with_number(settings: &Json, setting: &str, value: Option<f64>) -> Json {
    let mut settings = settings.clone();
    if let Json::Object(fields) = &mut settings {
        match value {
            Some(value) => {
                // A whole number is written as one, as a person would type it.
                #[expect(clippy::cast_possible_truncation, reason = "checked to be whole")]
                let number = if value.fract() == 0.0 && value.abs() < 1e15 {
                    Json::from(value as i64)
                } else {
                    Json::from(value)
                };
                let _ = fields.insert(setting.to_owned(), number);
            }
            None => {
                let _ = fields.remove(setting);
            }
        }
    }
    settings
}

/// `settings` with `setting` set to `value`, or left out for `None`.
pub(crate) fn with_setting(settings: &Json, setting: &str, value: Option<String>) -> Json {
    let mut settings = settings.clone();
    if let Json::Object(fields) = &mut settings {
        match value {
            Some(value) => {
                let _ = fields.insert(setting.to_owned(), Json::String(value));
            }
            None => {
                let _ = fields.remove(setting);
            }
        }
    }
    settings
}

fn scope_name(scope: VarScope) -> &'static str {
    match scope {
        VarScope::Local => t("scope-local"),
        VarScope::Run => t("scope-run"),
        VarScope::Workspace => t("scope-workspace"),
    }
}

impl Editor {
    /// What the open document declares.
    pub fn declarations(&self) -> Option<&Declarations> {
        self.document.as_ref().map(|document| &document.declared)
    }

    /// Declares a variable, or changes the one with `variable.id`. Returns its ID.
    pub fn declare_variable(&mut self, host: &mut Host, variable: &Variable) -> Option<String> {
        let draft = self.editing()?;
        let result = host.declare_variable(draft, variable);
        self.edited_value(host, result)
    }

    /// Declares a variable and makes `node` name it at the setting `pointer` (such as
    /// `/variable`), keeping the node selected. Returns the variable's ID.
    pub fn declare_variable_for(
        &mut self,
        host: &mut Host,
        node: NodeId,
        pointer: &str,
        variable: &Variable,
    ) -> Option<String> {
        let definition = &self
            .graph()?
            .nodes
            .iter()
            .find(|view| view.id == node)?
            .definition;
        if !variable_scope_allowed(definition, variable.scope) {
            return None;
        }
        let id = self.declare_variable(host, variable)?;
        let _ = self.set_setting(
            host,
            node,
            pointer,
            Some(serde_json::Value::from(id.as_str())),
        );
        self.canvas.select(Some(node));
        Some(id)
    }

    /// Removes a variable; nodes that name it show as problems.
    pub fn remove_variable(&mut self, host: &mut Host, id: &str) -> bool {
        let Some(draft) = self.editing() else {
            return false;
        };
        let result = host.remove_variable(draft, id);
        self.edited(host, result)
    }

    /// Declares a connection, or changes the one with `connection.id`. A secret written
    /// in its settings instead of a `credential://` reference is refused. Returns its ID.
    pub fn declare_connection(
        &mut self,
        host: &mut Host,
        connection: &Connection,
    ) -> Option<String> {
        let draft = self.editing()?;
        let result = host.declare_connection(draft, connection);
        self.edited_value(host, result)
    }

    /// The setting choices of `adapter`, listed the first time they are asked for and
    /// again after `refresh`.
    pub(crate) fn adapter_choices(
        &mut self,
        host: &Host,
        adapter: &str,
        refresh: bool,
    ) -> Vec<SettingChoices> {
        if refresh {
            let _ = self.adapter_choices.remove(adapter);
        }
        self.adapter_choices
            .entry(adapter.to_owned())
            .or_insert_with(|| host.adapter_choices(adapter))
            .clone()
    }

    /// Removes a connection; nodes that use it show as problems.
    pub fn remove_connection(&mut self, host: &mut Host, id: &str) -> bool {
        let Some(draft) = self.editing() else {
            return false;
        };
        let result = host.remove_connection(draft, id);
        self.edited(host, result)
    }

    pub(crate) fn new_variable_for_node(&mut self, node: NodeId, pointer: &str) {
        let scope = self
            .graph()
            .and_then(|graph| graph.nodes.iter().find(|view| view.id == node))
            .filter(|view| !variable_scope_allowed(&view.definition, VarScope::Run))
            .map_or(VarScope::Run, |_| VarScope::Workspace);
        self.panels.nodes = true;
        self.tab = Tab::Variables;
        self.forms.reveal_variable(Some((node, pointer.to_owned())));
        self.forms.variable.scope = scope;
    }

    pub(crate) fn new_event_for_node(&mut self, target: NodeId) {
        self.forms.event = Some(EventForm {
            target,
            name: String::new(),
            payload: "core/bool@1".to_owned(),
            failed: false,
        });
    }

    pub(crate) fn event_dialog(&mut self, ctx: &egui::Context, host: &mut Host) {
        if let Some(form) = &self.forms.event
            && self
                .graph()
                .is_none_or(|graph| !graph.nodes.iter().any(|node| node.id == form.target))
        {
            self.forms.event = None;
        }
        let Some(form) = &mut self.forms.event else {
            return;
        };
        let shown =
            orange_ui_controls::modal(ctx, egui::Id::unique("declare-run-event")).show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 40.0).clamp(200.0, 380.0));
                ui.heading(t("event-new"));
                ui.label(t("event-run-hint"));
                let label = ui.label(t("name"));
                let id = ui.make_persistent_id("event-name");
                let _ = orange_ui_controls::text_field(
                    ui,
                    Surface::Dialog,
                    id,
                    &mut form.name,
                    t("event-name-hint"),
                )
                .labelled_by(label.id);
                let label = ui.label(t("event-payload-type"));
                let _ = egui::ComboBox::from_id_salt("event-payload-type")
                    .selected_text(payload_name(&form.payload))
                    .width(ui.available_width())
                    .show_select(ui, Surface::Dialog, |ui| {
                        // An event-only channel: its nodes have no payload port.
                        let _ = ui.selectable_value(
                            &mut form.payload,
                            String::new(),
                            t("event-no-payload"),
                        );
                        for ty in COMMON_TYPES.into_iter().chain([
                            "core/json@1",
                            "core/timestamp@1",
                            "core/i32@1",
                            "core/u32@1",
                            "core/u64@1",
                            "core/f32@1",
                        ]) {
                            let _ = ui.selectable_value(
                                &mut form.payload,
                                ty.to_owned(),
                                type_name(ty),
                            );
                        }
                        for (ty, name) in types_tab::choices() {
                            let _ = ui.selectable_value(&mut form.payload, ty, name);
                        }
                    })
                    .response
                    .labelled_by(label.id);
                if form.failed {
                    ui.colored_label(ui.visuals().warn_fg_color, t("event-create-failed"));
                }
                crate::controls::dialog_actions(
                    ui,
                    Action::Primary,
                    !form.name.trim().is_empty(),
                    egui::Button::new(t("event-create-bind")),
                )
            });
        if shown.inner.1.clicked() {
            let target = form.target;
            let name = form.name.clone();
            let payload = if form.payload.is_empty() {
                Ok(None)
            } else {
                form.payload.parse().map(Some)
            };
            if let Ok(payload) = payload
                && let Some(draft) = self.editing()
            {
                let result = host.declare_run_event_for(draft, target, &name, payload);
                if self.edited_value(host, result).is_some() {
                    self.forms.event = None;
                    return;
                }
            }
            if let Some(form) = &mut self.forms.event {
                form.failed = true;
            }
        } else if shown.inner.0.clicked() || shown.should_close() {
            self.forms.event = None;
        }
    }

    pub(crate) fn variables_tab(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        let Some(declared) = self.declarations().cloned() else {
            ui.weak(t("open-graph-variables"));
            return;
        };
        search_field(
            ui,
            "variables-search",
            &mut self.declaration_search[0],
            t("search-variables"),
        );
        egui::ScrollArea::vertical()
            .id_salt("variables-list")
            .show(ui, |ui| {
                self.variables_section(ui, host, &declared);
            });
    }

    pub(crate) fn connections_tab(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        let Some(declared) = self.declarations().cloned() else {
            ui.weak(t("open-graph-connections"));
            return;
        };
        search_field(
            ui,
            "connections-search",
            &mut self.declaration_search[1],
            t("search-connections"),
        );
        egui::ScrollArea::vertical()
            .id_salt("connections-list")
            .show(ui, |ui| {
                self.connections_section(ui, host, &declared);
            });
    }

    /// Adds a Get node for variable `id` where it was let go, or a Set node with `set`.
    /// A variable shared by graphs is written with Set shared, the only write the
    /// language accepts for it (11 §11.1). Returns the node.
    pub fn drop_variable(
        &mut self,
        host: &mut Host,
        id: &str,
        set: bool,
        at: [f32; 2],
    ) -> Option<NodeId> {
        let shared = self.declarations().is_some_and(|declared| {
            declared.variables.iter().any(|variable| {
                variable.id.as_deref() == Some(id) && variable.scope == VarScope::Workspace
            })
        });
        let definition = match (set, shared) {
            (false, _) => "core/get-variable",
            (true, false) => "core/set-variable",
            (true, true) => "core/set-shared",
        };
        let node = self.add_node_at(host, definition, at)?;
        let _ = self.set_setting(host, node, "/variable", Some(Json::from(id)));
        self.canvas.select(Some(node));
        Some(node)
    }

    /// A field typed in place in the Variables list, `width` wide: `shown` until it has
    /// the focus, then what is typed. Returns the text when the field is left with it
    /// changed.
    pub(crate) fn inline_field_sized(
        &mut self,
        ui: &mut egui::Ui,
        key: (String, &'static str),
        shown: &str,
        width: f32,
    ) -> Option<String> {
        let mut text = self
            .forms
            .inline
            .get(&key)
            .cloned()
            .unwrap_or_else(|| shown.to_owned());
        let tall = egui::vec2(0.0, ui.spacing().interact_size.y);
        let response = ui.add(
            edit_like(&mut text, "unset")
                .desired_width(width)
                .min_size(tall),
        );
        if self.forms.focus.as_ref() == Some(&key) {
            self.forms.focus = None;
            response.request_focus();
        }
        if response.has_focus() {
            let _ = self.forms.inline.insert(key, text);
            None
        } else {
            let _ = self.forms.inline.remove(&key);
            (response.lost_focus() && text.trim() != shown).then(|| text.trim().to_owned())
        }
    }

    fn variables_section(&mut self, ui: &mut egui::Ui, host: &mut Host, declared: &Declarations) {
        // A heading with its add button, as in a blueprint's variable list.
        if self.forms.open_variable {
            self.forms.open_variable = false;
            self.forms.variable_open = true;
        }
        let adding = section_heading(ui, t("variables"), t("add-variable"));
        if adding {
            self.forms.variable = VariableForm::default();
            self.forms.variable_target = None;
            self.forms.variable_open = true;
            self.declaration_search[0].clear();
        }
        if self.forms.variable_open {
            self.variable_form(ui, host);
        }
        let running = self
            .document
            .as_ref()
            .is_some_and(|document| document.flow.is_some());
        let mut remove = None;
        let mut change: Option<Variable> = None;
        let mut place: Option<(String, bool)> = None;
        let mut inspect_value = None;
        let mut find: Option<Vec<NodeId>> = None;
        // The nodes that name each variable: Get, Set, Update, On variable changed.
        let mut users_of: HashMap<String, Vec<NodeId>> = HashMap::new();
        for node in self.graph().map_or(&[][..], |graph| graph.nodes.as_slice()) {
            if let Some(variable) = node.settings.get("variable").and_then(Json::as_str) {
                users_of
                    .entry(variable.to_owned())
                    .or_default()
                    .push(node.id);
            }
        }
        let query = self.declaration_search[0].trim().to_lowercase();
        let mut visible: Vec<_> = declared
            .variables
            .iter()
            .filter(|variable| {
                contains_query(
                    &query,
                    &[
                        &variable.name,
                        &variable.ty,
                        &type_name(&variable.ty),
                        scope_name(variable.scope),
                    ],
                )
            })
            .collect();
        visible.sort_by_cached_key(|variable| (variable.scope as u8, variable.name.to_lowercase()));
        if visible.is_empty() && !declared.variables.is_empty() {
            ui.weak(t("no-declaration-matches"));
        }
        let mut previous_scope = None;
        for variable in visible {
            if previous_scope != Some(variable.scope) {
                ui.strong(scope_name(variable.scope));
                previous_scope = Some(variable.scope);
            }
            let Some(id) = variable.id.clone() else {
                continue;
            };
            let current_value = self
                .document
                .as_ref()
                .and_then(|doc| doc.values.get(&id))
                .map(|value| value.shown.clone());
            let _ = item_frame(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    // Dragged by its handle onto the graph, it becomes a Get, or a Set
                    // with Alt.
                    let drag = ui.scope_id().with(("orange-variable", id.as_str()));
                    let _ = ui
                        .dnd_drag_source(drag, format!("{VARIABLE_DRAG}{id}"), |ui| {
                            ui.add(
                                egui::Label::new(icons::text(icons::DRAG).weak())
                                    .selectable(false)
                                    .sense(egui::Sense::drag()),
                            )
                        })
                        .response
                        .on_hover_cursor(egui::CursorIcon::Grab)
                        .on_hover_text(t("variable-drag-hint"));
                    // The menu and the value now take their room first; the name fills
                    // what is left.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let _ = ui
                            .menu_button(icons::text(icons::MORE), |ui| {
                                let mut kept = variable.clone();
                                ui.label(t("lasts"));
                                for scope in [VarScope::Run, VarScope::Workspace, VarScope::Local] {
                                    let _ = orange_ui_controls::radio(
                                        ui,
                                        Surface::Dialog,
                                        orange_ui_controls::RadioStyle::Simple,
                                        &mut kept.scope,
                                        scope,
                                        scope_name(scope),
                                    );
                                }
                                let _ = orange_ui_controls::checkbox(
                                    ui,
                                    Surface::Dialog,
                                    orange_ui_controls::CheckStyle::Emphasis,
                                    &mut kept.persistent,
                                    t("keep-between-runs"),
                                );
                                if kept != *variable {
                                    change = Some(kept);
                                }
                                ui.separator();
                                if ui.button(t("delete")).clicked() {
                                    remove = Some(id.clone());
                                    ui.close();
                                }
                            })
                            .response
                            .on_hover_text(t("more-options"));
                        // Its value now while the graph runs; the last one, dimmed, after.
                        if current_value.is_some() && ui.small_button(t("inspect-value")).clicked()
                        {
                            inspect_value = Some(id.clone());
                        }
                        match &current_value {
                            Some(value) if running => {
                                let _ = ui
                                    .add(
                                        egui::Label::new(
                                            egui::RichText::new(value.as_str())
                                                .strong()
                                                .monospace(),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(t("value-now-hint"));
                            }
                            Some(value) => {
                                let _ = ui
                                    .add(
                                        egui::Label::new(
                                            egui::RichText::new(value.as_str()).weak().monospace(),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(t("value-last-run-hint"));
                            }
                            None => {}
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            let width = ui.available_width();
                            if let Some(name) = self.inline_field_sized(
                                ui,
                                (id.clone(), "name"),
                                &variable.name,
                                width,
                            ) && !name.is_empty()
                            {
                                change = Some(Variable {
                                    name,
                                    ..variable.clone()
                                });
                            }
                        });
                    });
                });
                // Its type and where it starts, on one line.
                ui.horizontal(|ui| {
                    let color = canvas::type_color(Some(&variable.ty));
                    let mut ty = variable.ty.clone();
                    let _ = egui::ComboBox::from_id_salt(("orange-variable-type", id.as_str()))
                        .width(84.0)
                        .wrap_mode(egui::TextWrapMode::Truncate)
                        .selected_text(egui::RichText::new(type_name(&variable.ty)).color(color))
                        .show_select(ui, Surface::Panel, |ui| {
                            for common in COMMON_TYPES {
                                let _ = ui.selectable_value(
                                    &mut ty,
                                    common.to_owned(),
                                    type_name(common),
                                );
                            }
                            for (local, name) in types_tab::choices() {
                                let _ = ui.selectable_value(&mut ty, local, name);
                            }
                        })
                        .response
                        .on_hover_text(exact_type(&variable.ty));
                    if ty != variable.ty {
                        // A start value of the old type is dropped with it.
                        change = Some(Variable {
                            ty,
                            initial: None,
                            ..variable.clone()
                        });
                    }
                    ui.weak(t("starts-at-lower"));
                    if let Some(choices) = self.canvas.enums.get(&variable.ty) {
                        let mut initial = variable.initial.clone();
                        choice_list(
                            ui,
                            egui::IdSalt::new(("orange-variable-initial", id.as_str())),
                            choices,
                            &mut initial,
                        );
                        if initial != variable.initial {
                            change = Some(Variable {
                                initial,
                                ..variable.clone()
                            });
                        }
                        return;
                    }
                    let shown = variable.initial.clone().unwrap_or_default();
                    let width = ui.available_width();
                    if let Some(initial) =
                        self.inline_field_sized(ui, (id.clone(), "initial"), &shown, width)
                    {
                        change = Some(Variable {
                            initial: Some(initial).filter(|text| !text.is_empty()),
                            ..variable.clone()
                        });
                    }
                })
                .response
                .on_hover_text(t("starts-at"));
                // Where it is used, which finds those nodes, and a Get or a Set
                // without dragging.
                ui.horizontal(|ui| {
                    let users = users_of.get(id.as_str()).map_or(&[][..], Vec::as_slice);
                    if users.is_empty() {
                        ui.weak(t("variable-unused"));
                    } else if ui
                        .link(
                            egui::RichText::new(tf(
                                "variable-use-count",
                                &[("count", users.len().into())],
                            ))
                            .weak(),
                        )
                        .on_hover_text(t("variable-find-uses-hint"))
                        .clicked()
                    {
                        find = Some(users.to_vec());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let named = [("name", variable.name.as_str().into())];
                        if ui
                            .small_button(t("variable-set"))
                            .on_hover_text(tf("variable-set-hint", &named))
                            .clicked()
                        {
                            place = Some((id.clone(), true));
                        }
                        if ui
                            .small_button(t("variable-get"))
                            .on_hover_text(tf("variable-get-hint", &named))
                            .clicked()
                        {
                            place = Some((id.clone(), false));
                        }
                    });
                });
            });
        }
        if let Some(id) = inspect_value {
            self.inspected_port = None;
            self.inspected_variable = Some(id);
        }
        if let Some((id, set)) = place {
            let at = self.canvas.paste_place();
            let _ = self.drop_variable(host, &id, set, at);
        }
        if let Some(users) = find
            && let Some(document) = &self.document
        {
            if let Some(first) = users.first() {
                self.canvas.focus(&document.graph, *first);
            }
            self.canvas.select_many(&users);
        }
        if declared.variables.is_empty() && !self.forms.variable_open {
            ui.weak(t("no-variables"));
        } else if !declared.variables.is_empty() {
            ui.weak(t("variable-drag-empty-hint"));
        }
        if let Some(variable) = change {
            let _ = self.declare_variable(host, &variable);
        }
        if let Some(id) = remove {
            let _ = self.remove_variable(host, &id);
        }
    }

    fn variable_form(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        let mut declare = false;
        let mut cancel = false;
        {
            let target = self
                .forms
                .variable_target
                .as_ref()
                .and_then(|(node, _)| self.graph()?.nodes.iter().find(|view| view.id == *node));
            let target_definition = target.map(|node| node.definition.clone());
            let target_name = target.map(|node| node.name.clone());
            let enums = &self.canvas.enums;
            let form = &mut self.forms.variable;
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.strong(t("new-variable"));
                if let Some(name) = &target_name {
                    ui.weak(tf(
                        "new-variable-for-node",
                        &[("name", name.as_str().into())],
                    ));
                }
                orange_ui_controls::form_grid("orange-variable-form").show(ui, |ui| {
                    ui.label(t("name"));
                    let _ = ui.add(
                        egui::TextEdit::singleline(&mut form.name)
                            .hint_text(t("variable-name-hint"))
                            .desired_width(ui.available_width()),
                    );
                    ui.end_row();
                    ui.label(t("type"));
                    let _ = egui::ComboBox::from_id_salt("orange-variable-type")
                        .selected_text(type_name(&form.ty))
                        .show_select(ui, Surface::Panel, |ui| {
                            for ty in COMMON_TYPES {
                                let _ =
                                    ui.selectable_value(&mut form.ty, ty.to_owned(), type_name(ty));
                            }
                            for (local, name) in types_tab::choices() {
                                let _ = ui.selectable_value(&mut form.ty, local, name);
                            }
                        });
                    ui.end_row();
                    ui.label(t("lasts")).on_hover_text(t("lasts-hint"));
                    let _ = egui::ComboBox::from_id_salt("orange-variable-scope")
                        .selected_text(scope_name(form.scope))
                        .show_select(ui, Surface::Panel, |ui| {
                            for scope in [VarScope::Run, VarScope::Workspace, VarScope::Local] {
                                if target_definition.as_deref().is_some_and(|definition| {
                                    !variable_scope_allowed(definition, scope)
                                }) {
                                    continue;
                                }
                                let _ =
                                    ui.selectable_value(&mut form.scope, scope, scope_name(scope));
                            }
                        });
                    ui.end_row();
                    ui.label(t("starts-at"));
                    if let Some(choices) = enums.get(&form.ty) {
                        let mut initial =
                            Some(form.initial.clone()).filter(|text| !text.is_empty());
                        choice_list(
                            ui,
                            egui::IdSalt::new("orange-new-variable-initial"),
                            choices,
                            &mut initial,
                        );
                        form.initial = initial.unwrap_or_default();
                    } else {
                        let _ = ui.add(
                            egui::TextEdit::singleline(&mut form.initial)
                                .hint_text(t("empty-until-written"))
                                .desired_width(ui.available_width()),
                        );
                    }
                    ui.end_row();
                    ui.label("");
                    let _ = orange_ui_controls::checkbox(
                        ui,
                        Surface::Dialog,
                        orange_ui_controls::CheckStyle::Emphasis,
                        &mut form.persistent,
                        t("keep-between-runs"),
                    );
                    ui.end_row();
                });
                ui.horizontal(|ui| {
                    let ready = !form.name.trim().is_empty();
                    declare = ui.add_enabled(ready, egui::Button::new(t("add"))).clicked();
                    cancel = ui.button(t("cancel")).clicked();
                });
            });
        }
        if declare {
            let form = self.forms.variable.clone();
            let variable = Variable {
                id: form.changing,
                name: form.name.trim().to_owned(),
                ty: form.ty,
                scope: form.scope,
                persistent: form.persistent,
                initial: Some(form.initial.trim().to_owned()).filter(|text| !text.is_empty()),
            };
            let target = self.forms.variable_target.clone().filter(|(node, _)| {
                self.graph()
                    .is_some_and(|graph| graph.nodes.iter().any(|view| view.id == *node))
            });
            let declared = if let Some((node, pointer)) = target {
                self.declare_variable_for(host, node, &pointer, &variable)
            } else {
                self.declare_variable(host, &variable)
            };
            if declared.is_some() {
                self.forms.variable = VariableForm::default();
                self.forms.variable_target = None;
                self.forms.variable_open = false;
            }
        }
        if cancel {
            self.forms.variable = VariableForm::default();
            self.forms.variable_target = None;
            self.forms.variable_open = false;
        }
    }

    /// Declares a connection, first storing each typed secret in the host's credential
    /// store; the document gets only their references. Returns its ID.
    pub fn declare_connection_with_secrets(
        &mut self,
        host: &mut Host,
        connection: &Connection,
        secrets: &[(String, String)],
    ) -> Option<String> {
        let draft = self.editing()?;
        let result = host
            .declare_connection_with_secrets(draft, connection, secrets)
            .and_then(|id| {
                if secrets.is_empty() {
                    host.approve_connection_credentials(draft, &id)?;
                }
                Ok(id)
            });
        self.edited_value(host, result)
    }

    /// The nodes that use connection `id`, by what they are called.
    /// The nodes that use connection `id`, with what each is called.
    fn connection_users(&self, host: &Host, id: &str) -> Vec<(NodeId, String)> {
        let Some(document) = &self.document else {
            return Vec::new();
        };
        host.connection_users(&document.draft, id)
            .into_iter()
            .map(|node| (node, document.names.called(&node)))
            .collect()
    }

    /// Saves the form and returns whether a newly added profile can close it.
    fn commit_connection_form(&mut self, host: &mut Host) -> bool {
        let form = self.forms.connection.clone();
        let settings = form.settings_for_save().and_then(|settings| {
            // HTTP only constructs a client here. Other adapters may start I/O.
            if form.adapter == HTTP {
                form.check(host, settings.clone())?;
            }
            Ok(settings)
        });
        let settings = match settings {
            Ok(settings) => settings,
            Err(error) => {
                self.forms.connection.tested = Some(Err(error));
                return false;
            }
        };
        let connection = Connection {
            id: form.changing,
            name: form.name.trim().to_owned(),
            adapter: form.adapter,
            scope: form.scope,
            settings,
        };
        let secrets: Vec<_> = form
            .secrets
            .values
            .into_iter()
            .filter(|(_, secret)| !secret.is_empty())
            .collect();
        let saved = if let Some(node) = self.forms.connection_target {
            let result = self.editing().map(|draft| {
                host.declare_connection_for(draft, node, &connection, &secrets)
                    .and_then(|id| {
                        if secrets.is_empty() {
                            host.approve_connection_credentials(draft, &id)?;
                        }
                        Ok(id)
                    })
            });
            result.and_then(|result| self.edited_value(host, result))
        } else {
            self.declare_connection_with_secrets(host, &connection, &secrets)
        };
        let Some(id) = saved else {
            self.forms.connection.tested = Some(Err(t("connection-save-failed").to_owned()));
            return false;
        };
        if connection.id.is_none() {
            return true;
        }
        let saved = self
            .declarations()
            .and_then(|declarations| {
                declarations
                    .connections
                    .iter()
                    .find(|saved| saved.id.as_deref() == Some(&id))
            })
            .cloned();
        if let Some(saved) = saved {
            self.forms.edit_connection(&saved);
            self.forms.connection.saved = true;
        }
        false
    }

    /// Applies a chosen root to the open connection draft. Apply saves the connection.
    pub fn choose_connection_folder(&mut self, path: &str) -> bool {
        let form = &mut self.forms.connection;
        if !self.forms.open_connection
            || form.adapter != FOLDER_ADAPTER
            || form.scope != ConnectionScope::Graph
            || path.is_empty()
        {
            return false;
        }
        let Ok(mut settings) = serde_json::from_str::<Json>(&form.settings) else {
            return false;
        };
        set_text(&mut settings, "path", Some(Json::String(path.to_owned())));
        form.settings = serde_json::to_string_pretty(&settings).unwrap_or_default();
        form.tested = None;
        form.saved = false;
        true
    }

    fn connections_section(&mut self, ui: &mut egui::Ui, host: &mut Host, declared: &Declarations) {
        let mut adapters = host.adapter_list();
        adapters.sort_by_key(|adapter| adapter_rank(&adapter.id));
        let label_of = |adapter: &str| {
            adapters
                .iter()
                .find(|known| known.id == adapter)
                .map_or_else(
                    || adapter.to_owned(),
                    |known| adapter_label(known).to_owned(),
                )
        };
        if section_heading(ui, t("connections"), t("add-connection")) {
            self.forms.connection = ConnectionForm::default();
            self.forms.connection_target = None;
            self.forms.reveal_connection();
            self.declaration_search[1].clear();
        }
        let mut remove = None;
        let query = self.declaration_search[1].trim().to_lowercase();
        let mut visible: Vec<_> = declared
            .connections
            .iter()
            .filter(|connection| {
                contains_query(
                    &query,
                    &[
                        &connection.name,
                        &connection.adapter,
                        &label_of(&connection.adapter),
                    ],
                )
            })
            .collect();
        visible.sort_by_cached_key(|connection| {
            (
                label_of(&connection.adapter),
                connection.name.to_lowercase(),
            )
        });
        if visible.is_empty() && !declared.connections.is_empty() {
            ui.weak(t("no-declaration-matches"));
        }
        let mut find: Option<Vec<NodeId>> = None;
        let mut previous_adapter = None;
        for connection in visible {
            if previous_adapter != Some(connection.adapter.as_str()) {
                ui.strong(label_of(&connection.adapter));
                previous_adapter = Some(connection.adapter.as_str());
            }
            let id = connection.id.clone().unwrap_or_default();
            let users = self.connection_users(host, &id);
            let detail = format!("{}\n{}", label_of(&connection.adapter), connection.adapter);
            let (edit, delete) = item_frame(ui)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let pressed = declaration_row(ui, &connection.name, &detail);
                    if users.is_empty() {
                        ui.weak(tf(
                            "connection-unused",
                            &[("adapter", label_of(&connection.adapter).into())],
                        ));
                    } else {
                        let names: Vec<&str> =
                            users.iter().map(|(_, name)| name.as_str()).collect();
                        // Clicking it selects and shows those nodes.
                        if ui
                            .link(
                                egui::RichText::new(tf(
                                    "connection-use-count",
                                    &[("count", users.len().into())],
                                ))
                                .weak(),
                            )
                            .on_hover_text(names.join("\n"))
                            .clicked()
                        {
                            find = Some(users.iter().map(|(node, _)| *node).collect());
                        }
                    }
                    pressed
                })
                .inner;
            if edit {
                self.forms.edit_connection(connection);
                self.probing = None;
            }
            if delete {
                remove = connection.id.clone();
            }
        }
        if let Some(users) = find
            && let Some(document) = &self.document
        {
            if let Some(first) = users.first() {
                self.canvas.focus(&document.graph, *first);
            }
            self.canvas.select_many(&users);
        }
        if declared.connections.is_empty() {
            ui.weak(t("no-connections"));
        }
        if let Some(id) = remove {
            let _ = self.remove_connection(host, &id);
        }
    }

    pub(crate) fn show_connection_dialog(&mut self, ctx: &egui::Context, host: &mut Host) {
        if !self.forms.open_connection {
            return;
        }
        let mut adapters = host.adapter_list();
        adapters.sort_by_key(|adapter| adapter_rank(&adapter.id));
        self.connection_dialog(ctx, host, &adapters);
    }

    fn connection_dialog(
        &mut self,
        ctx: &egui::Context,
        host: &mut Host,
        adapters: &[AdapterInfo],
    ) {
        if !self.forms.open_connection {
            self.probing = None;
            return;
        }
        if self.forms.connection.adapter.is_empty()
            && let Some(first) = adapters
                .iter()
                .find(|info| info.id == HTTP)
                .or_else(|| adapters.first())
        {
            self.forms.connection.adapter.clone_from(&first.id);
        }
        let adapter = self.forms.connection.adapter.clone();
        let choices = self.adapter_choices(host, &adapter, false);
        let fields = host.adapter_fields(&adapter);
        let mut texts = host.adapter_texts(&adapter);
        let folder_picker = self.file_dialogs && adapter == FOLDER_ADAPTER;
        if folder_picker {
            texts.retain(|field| field.setting != "path");
        }
        let mut pick_folder = None;
        let toggles = host.adapter_toggles(&adapter);
        let probes = host.adapter_probes(&adapter);
        let mut refresh = false;
        let mut declare = false;
        let mut cancel = false;
        let mut test = false;
        let probing = &mut self.probing;
        let form = &mut self.forms.connection;
        let title = if form.changing.is_some() {
            t("change-connection")
        } else {
            t("new-connection")
        };
        {
            let bounds = ctx.content_rect();
            let read_only = form.scope == ConnectionScope::Shared;
            let shown = orange_ui_controls::modal(ctx, egui::Id::unique("connection-form")).show(
                ctx,
                |ui| {
                    ui.set_width((bounds.width() - 40.0).clamp(240.0, 520.0));
                    ui.heading(title);
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .id_salt((&form.changing, "connection-fields"))
                        .auto_shrink([false, true])
                        .max_height((bounds.height() - 172.0).max(80.0))
                        .show(ui, |ui| {
                            let name_label = ui.label(t("name"));
                            if read_only {
                                ui.label(&form.name);
                            } else {
                                let id = ui.make_persistent_id("connection-name");
                                if orange_ui_controls::text_field(
                                    ui,
                                    Surface::Dialog,
                                    id,
                                    &mut form.name,
                                    t("connection-name-hint"),
                                )
                                .labelled_by(name_label.id)
                                .changed()
                                {
                                    form.saved = false;
                                }
                            }
                            adapter_picker(ui, adapters, form);
                            ui.separator();
                            if form.scope == ConnectionScope::Shared {
                                ui.weak(t("connection-host-hint"));
                                return;
                            }
                            if adapter != form.adapter {
                                return;
                            }
                            if let Ok(mut current) = serde_json::from_str::<Json>(&form.settings) {
                                let before = current.clone();
                                if folder_picker {
                                    let path = current["path"].as_str().unwrap_or_default();
                                    ui.label(t("connection-folder-root"));
                                    ui.horizontal_wrapped(|ui| {
                                        if button(
                                            ui,
                                            Surface::Dialog,
                                            Action::Secondary,
                                            true,
                                            egui::Button::new(t("choose-folder")),
                                        )
                                        .clicked()
                                        {
                                            pick_folder = Some(path.to_owned());
                                        }
                                        if !path.is_empty() {
                                            ui.add(egui::Label::new(path).truncate())
                                                .on_hover_text(path);
                                        }
                                    });
                                }
                                let fields_changed = network::fields(
                                    ui,
                                    &adapter,
                                    &texts,
                                    &mut current,
                                    &mut form.secrets,
                                    &mut form.auth,
                                    host,
                                );
                                let mut fields_changed = fields_changed;
                                let mut settings_controls =
                                    |ui: &mut egui::Ui, current: &mut Json| {
                                        toggle_fields(ui, &toggles, current);
                                        if let Some((setting, value)) = choice_fields(
                                            ui,
                                            "form",
                                            &choices,
                                            current,
                                            &mut refresh,
                                        ) {
                                            *current = with_setting(current, &setting, value);
                                        }
                                        if let Some((setting, value)) =
                                            number_fields(ui, &fields, current)
                                        {
                                            *current = with_number(current, &setting, value);
                                        }
                                    };
                                if network::uses_auth(&adapter) {
                                    let advanced = network::advanced_texts(&texts);
                                    if !advanced.is_empty()
                                        || !toggles.is_empty()
                                        || !choices.is_empty()
                                        || !fields.is_empty()
                                    {
                                        egui::CollapsingHeader::new(t("advanced"))
                                            .id_salt((&adapter, "connection-advanced"))
                                            .show(ui, |ui| {
                                                settings_controls(ui, &mut current);
                                                fields_changed |= text_fields(
                                                    ui,
                                                    &advanced,
                                                    &mut current,
                                                    &mut form.secrets,
                                                    host,
                                                );
                                            });
                                    }
                                } else {
                                    settings_controls(ui, &mut current);
                                }
                                if current != before {
                                    form.settings =
                                        serde_json::to_string_pretty(&current).unwrap_or_default();
                                }
                                if current != before || fields_changed {
                                    form.tested = None;
                                    form.saved = false;
                                }
                                if probes {
                                    live_level(
                                        ui,
                                        host,
                                        &form.adapter,
                                        &current,
                                        &choices,
                                        &fields,
                                        probing,
                                    );
                                }
                            } else {
                                *probing = None;
                                ui.colored_label(
                                    ui.visuals().warn_fg_color,
                                    t("settings-below-not-json"),
                                );
                            }
                            egui::CollapsingHeader::new(t("all-settings-json"))
                                .id_salt("orange-connection-json")
                                .default_open(
                                    !folder_picker
                                        && texts.is_empty()
                                        && choices.is_empty()
                                        && fields.is_empty(),
                                )
                                .show(ui, |ui| {
                                    let changed = ui
                                        .add(
                                            egui::TextEdit::multiline(&mut form.settings)
                                                .code_editor()
                                                .desired_rows(4)
                                                .desired_width(f32::INFINITY),
                                        )
                                        .changed();
                                    if changed {
                                        form.auth = None;
                                        form.secrets.revealed.clear();
                                        form.secrets.local_status.clear();
                                        form.tested = None;
                                        form.saved = false;
                                    }
                                    ui.weak(t("secret-hint"));
                                });
                        });
                    ui.separator();
                    if let Some(result) = &form.tested {
                        match result {
                            Ok(()) => {
                                ui.label(t("settings-accepted"));
                            }
                            Err(why) => {
                                ui.colored_label(ui.visuals().warn_fg_color, why);
                            }
                        }
                    }
                    ui.horizontal_wrapped(|ui| {
                        let ready = !form.name.trim().is_empty() && !form.adapter.is_empty();
                        let label = if form.settings.contains(SECRET_PREFIX)
                            || form
                                .secrets
                                .values
                                .values()
                                .any(|secret| !secret.is_empty())
                        {
                            t("save-and-approve-credentials")
                        } else if form.changing.is_some() {
                            t("save")
                        } else {
                            t("add")
                        };
                        if !read_only {
                            declare = button(
                                ui,
                                Surface::Dialog,
                                Action::Primary,
                                ready,
                                egui::Button::new(label),
                            )
                            .clicked();
                        }
                        let close = if form.saved || read_only {
                            t("connection-done")
                        } else {
                            t("cancel")
                        };
                        cancel = button(
                            ui,
                            Surface::Dialog,
                            Action::Secondary,
                            true,
                            egui::Button::new(close),
                        )
                        .clicked();
                        if !read_only {
                            test = button(
                                ui,
                                Surface::Dialog,
                                Action::Secondary,
                                adapter == form.adapter,
                                egui::Button::new(t("check-settings")),
                            )
                            .on_hover_text(t("check-settings-hint"))
                            .clicked();
                        }
                        if form.saved {
                            ui.label(t("connection-saved"));
                        }
                    });
                },
            );
            // Consume Escape without treating a backdrop click as Cancel.
            cancel |= shown.is_top_modal
                && !shown.any_popup_open
                && ctx
                    .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        }
        if let Some(current) = pick_folder {
            self.ask(FileRequest::ChooseConnectionFolder { current });
        }
        // A closed form shows no level, and the device it listened to is let go.
        if declare || cancel {
            self.probing = None;
        }
        if test {
            let form = &mut self.forms.connection;
            form.tested = Some(
                form.settings_for_save()
                    .and_then(|settings| form.check(host, settings)),
            );
        }
        if declare {
            cancel = self.commit_connection_form(host);
        }
        if cancel {
            self.forms.connection = ConnectionForm::default();
            self.forms.open_connection = false;
            self.forms.connection_target = None;
        }
        if refresh {
            let _ = self.adapter_choices(host, &adapter, true);
        }
    }
}

/// A frameless field that reads like a label until it is clicked.
fn edit_like<'a>(text: &'a mut String, hint: &str) -> egui::TextEdit<'a> {
    egui::TextEdit::singleline(text)
        .hint_text(hint)
        .frame(egui::Frame::NONE)
        .desired_width(110.0)
        // As tall as the buttons and lists beside it, with its text centred among them.
        .align(egui::Align2::LEFT_CENTER)
}

/// A list of an enum's choices for a starting value: empty until written, or one
/// choice by name. `initial` holds the chosen name.
fn choice_list(
    ui: &mut egui::Ui,
    salt: egui::IdSalt,
    choices: &[orange_host::EnumChoice],
    initial: &mut Option<String>,
) {
    let _ = egui::ComboBox::from_id_salt(salt)
        .width(ui.available_width().min(160.0))
        .selected_text(
            initial
                .clone()
                .unwrap_or_else(|| t("empty-until-written").to_owned()),
        )
        .show_select(ui, Surface::Panel, |ui| {
            let _ = ui.selectable_value(initial, None, t("empty-until-written"));
            for choice in choices.iter().filter(|choice| !choice.payload) {
                let _ = ui.selectable_value(initial, Some(choice.name.clone()), &choice.name);
            }
        });
}

/// The list of adapters to make a connection of, by name, with each one's purpose.
fn adapter_picker(ui: &mut egui::Ui, adapters: &[AdapterInfo], form: &mut ConnectionForm) {
    let caption = ui.label(t("connection-protocol"));
    let current = adapters.iter().find(|known| known.id == form.adapter);
    let shown = current.map_or(form.adapter.as_str(), adapter_label);
    if form.changing.is_some() {
        ui.label(shown);
        if let Some(known) = current.filter(|known| !known.purpose.is_empty()) {
            ui.weak(known.purpose.as_str());
        }
        return;
    }
    let before = form.adapter.clone();
    let _ = egui::ComboBox::from_id_salt("orange-connection-adapter")
        .selected_text(shown)
        .width((ui.available_width() - 8.0).max(0.0))
        .show_select(ui, Surface::Dialog, |ui| {
            let mut in_audio = false;
            for adapter in adapters {
                let audio = matches!(
                    adapter.id.as_str(),
                    "org.orange.audio/input" | "org.orange.audio/output"
                );
                if audio != in_audio {
                    ui.separator();
                    if audio {
                        ui.weak(t("connection-audio-group"));
                    }
                    in_audio = audio;
                }
                let hover = if adapter.purpose.is_empty() {
                    adapter.id.clone()
                } else {
                    format!("{}\n{}", adapter.purpose, adapter.id)
                };
                let _ = ui
                    .selectable_value(
                        &mut form.adapter,
                        adapter.id.clone(),
                        adapter_label(adapter),
                    )
                    .on_hover_text(hover);
            }
        })
        .response
        .labelled_by(caption.id);
    if form.adapter != before {
        // Another kind has other settings: start from none.
        form.settings = "{}".to_owned();
        form.secrets = Typed::default();
        form.auth = None;
        form.tested = None;
    }
    if let Some(known) = current
        && !known.purpose.is_empty()
    {
        ui.weak(known.purpose.as_str());
    }
}

/// `settings` with the text setting at `path` (names joined by `/`) set to `value`, or
/// taken out with the objects it leaves empty for `None`.
pub(crate) fn set_text(settings: &mut Json, path: &str, value: Option<Json>) {
    fn go(at: &mut Json, names: &[&str], value: Option<Json>) {
        let Some((name, rest)) = names.split_first() else {
            return;
        };
        if !at.is_object() {
            if value.is_none() {
                return;
            }
            *at = Json::Object(serde_json::Map::new());
        }
        let Json::Object(fields) = at else {
            return;
        };
        if rest.is_empty() {
            match value {
                Some(value) => {
                    let _ = fields.insert((*name).to_owned(), value);
                }
                None => {
                    let _ = fields.remove(*name);
                }
            }
            return;
        }
        let inner = fields
            .entry((*name).to_owned())
            .or_insert_with(|| Json::Object(serde_json::Map::new()));
        go(inner, rest, value);
        if inner.as_object().is_some_and(serde_json::Map::is_empty) {
            let _ = fields.remove(*name);
        }
    }
    let names: Vec<&str> = path.split('/').filter(|name| !name.is_empty()).collect();
    go(settings, &names, value);
}

/// The value at `path` (names joined by `/`) of `settings`.
fn text_at<'a>(settings: &'a Json, path: &str) -> Option<&'a Json> {
    path.split('/')
        .filter(|name| !name.is_empty())
        .try_fold(settings, |at, name| at.get(name))
}

/// Fields for an adapter's text settings. A secret is typed masked into `secrets`,
/// never into the settings; one already stored shows as such, with a button to type
/// another.
pub(super) fn text_fields(
    ui: &mut egui::Ui,
    texts: &[SettingText],
    settings: &mut Json,
    secrets: &mut Typed,
    host: &Host,
) -> bool {
    let mut changed = false;
    for text in texts {
        let label = if text.required {
            format!("{} *", text.label)
        } else {
            text.label.clone()
        };
        let caption = ui.label(&label).on_hover_text(text.help.as_str());
        let current = text_at(settings, &text.setting).and_then(Json::as_str);
        if text.secret {
            let field = &text.setting;
            let reference = current.filter(|value| value.starts_with(SECRET_PREFIX));
            if let Some(reference) = reference.filter(|_| !secrets.values.contains_key(field)) {
                ui.label(t(secrets
                    .local_status
                    .get(reference)
                    .copied()
                    .unwrap_or("connection-credential-reference")));
                let remove = ui
                    .horizontal_wrapped(|ui| {
                        if button(
                            ui,
                            Surface::Dialog,
                            Action::Secondary,
                            true,
                            egui::Button::new(t("connection-credential-check")),
                        )
                        .clicked()
                        {
                            let status = match host.credential_available(reference) {
                                Ok(true) => "connection-credential-available",
                                Ok(false) => "connection-credential-missing",
                                Err(_) => "connection-credential-unavailable",
                            };
                            secrets.local_status.insert(reference.to_owned(), status);
                        }
                        if button(
                            ui,
                            Surface::Dialog,
                            Action::Secondary,
                            true,
                            egui::Button::new(t("replace")),
                        )
                        .clicked()
                        {
                            secrets.values.insert(field.clone(), String::new());
                        }
                        button(
                            ui,
                            Surface::Dialog,
                            Action::Outline,
                            true,
                            egui::Button::new(t("connection-credential-remove")),
                        )
                        .clicked()
                    })
                    .inner;
                if remove {
                    set_text(settings, field, None);
                }
            } else {
                let mut value = secrets.values.get(field).cloned().unwrap_or_default();
                let mut revealed = secrets.revealed.contains(field);
                let tokens = orange_ui_controls::UiTokens::for_surface(ui, Surface::Dialog);
                let response = ui
                    .push_id(field, |ui| {
                        orange_ui_controls::components::password(
                            ui,
                            tokens,
                            &mut value,
                            &mut revealed,
                            orange_ui_controls::components::PasswordLabels {
                                field: &label,
                                reveal: t("connection-secret-show"),
                                hide: t("connection-secret-hide"),
                            },
                            None,
                        )
                    })
                    .inner;
                if revealed {
                    secrets.revealed.insert(field.clone());
                } else {
                    secrets.revealed.remove(field);
                }
                if response.changed() {
                    changed = true;
                    secrets.values.insert(field.clone(), value);
                }
                if reference.is_some() {
                    ui.weak(t("connection-credential-replace-hint"));
                    if ui.button(t("connection-credential-keep")).clicked() {
                        secrets.values.remove(field);
                        secrets.revealed.remove(field);
                        changed = true;
                    }
                } else {
                    ui.weak(t("kept-in-credential-store"));
                }
            }
        } else if text.list {
            let mut lines = text_at(settings, &text.setting)
                .and_then(Json::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Json::as_str)
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            if ui
                .add(
                    egui::TextEdit::multiline(&mut lines)
                        .hint_text(text.hint.as_str())
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(caption.id)
                .changed()
            {
                let items: Vec<Json> = lines
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(Json::from)
                    .collect();
                set_text(
                    settings,
                    &text.setting,
                    (!items.is_empty()).then_some(Json::Array(items)),
                );
            }
        } else {
            let mut value = current.unwrap_or_default().to_owned();
            let id = ui.make_persistent_id(("connection-text", &text.setting));
            if orange_ui_controls::text_field(ui, Surface::Dialog, id, &mut value, &text.hint)
                .labelled_by(caption.id)
                .changed()
            {
                set_text(
                    settings,
                    &text.setting,
                    (!value.is_empty()).then(|| Json::from(value)),
                );
            }
        }
        ui.add_space(6.0);
    }
    changed
}

/// Switches for an adapter's true-or-false settings. One switched back to its default
/// is left out.
pub(super) fn toggle_fields(ui: &mut egui::Ui, toggles: &[SettingToggle], settings: &mut Json) {
    for toggle in toggles {
        let mut on = settings
            .get(&toggle.setting)
            .and_then(Json::as_bool)
            .unwrap_or(toggle.default);
        if ui
            .checkbox(&mut on, toggle.label.as_str())
            .on_hover_text(toggle.help.as_str())
            .changed()
        {
            let value = (on != toggle.default).then_some(Json::Bool(on));
            set_text(settings, &toggle.setting, value);
        }
    }
}

/// A section's heading with an add button at its right end. Returns whether it was
/// pressed.
fn section_heading(ui: &mut egui::Ui, title: &str, add: &str) -> bool {
    ui.horizontal(|ui| {
        ui.heading(title);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.button(icons::text(icons::ADD))
                .on_hover_text(add)
                .clicked()
        })
        .inner
    })
    .inner
}

/// One declaration in a list: its name, with edit and delete
/// buttons at the right end. `detail` is shown on hover. Returns whether each was pressed.
fn declaration_row(ui: &mut egui::Ui, name: &str, detail: &str) -> (bool, bool) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let delete = ui
                .small_button(icons::text(icons::DELETE))
                .on_hover_text(t("delete"))
                .clicked();
            let edit = ui
                .small_button(icons::text(icons::SETTINGS))
                .on_hover_text(t("edit"))
                .clicked();
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let _ = ui
                    .add(egui::Label::new(egui::RichText::new(name).strong()).truncate())
                    .on_hover_text(detail);
            });
            (edit, delete)
        })
        .inner
    })
    .inner
}

/// The frame around one declaration in a list: the panel's faint fill, rounded, with
/// room inside and a gap below.
fn item_frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .outer_margin(egui::Margin {
            bottom: 6,
            ..egui::Margin::ZERO
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::graph;
    use orange_host::{HostOptions, SecretError, SecretStore};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    fn blank() -> String {
        let mut saved: Json = serde_json::from_str(&graph()).unwrap();
        saved["nodes"] = json!([]);
        saved["edges"] = json!([]);
        saved.to_string()
    }

    fn draw(editor: &mut Editor, host: &mut Host) {
        let context = egui::Context::default();
        let output = context.run_ui(egui::RawInput::default(), |ui| editor.ui(ui, host));
        output.drop_without_applying_deltas();
    }

    #[derive(Debug, Default)]
    struct Store(Mutex<BTreeMap<String, String>>);
    impl SecretStore for Store {
        fn get(&self, name: &str) -> Result<String, SecretError> {
            self.0
                .lock()
                .unwrap()
                .get(name)
                .cloned()
                .ok_or(SecretError::NotFound)
        }
        fn replace(&self, name: &str, secret: &str) -> Result<(), SecretError> {
            let _ = self
                .0
                .lock()
                .unwrap()
                .insert(name.to_owned(), secret.to_owned());
            Ok(())
        }
    }

    #[derive(Debug)]
    struct Thing;
    impl orange_host::Session for Thing {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }
    #[derive(Debug)]
    struct Service;
    impl orange_host::ConnectionAdapter for Service {
        fn open(
            &self,
            settings: &serde_json::Map<String, Json>,
        ) -> Result<Arc<dyn orange_host::Session>, String> {
            match settings.get("token") {
                Some(Json::String(token)) if token == "s3cret-value" => Ok(Arc::new(Thing)),
                _ => Err("`token` is wrong".into()),
            }
        }
        fn about(&self) -> orange_host::AdapterAbout {
            orange_host::AdapterAbout {
                label: "Some service".into(),
                purpose: "Talks to a service.".into(),
            }
        }
        fn texts(&self) -> Vec<SettingText> {
            vec![SettingText {
                setting: "token".into(),
                label: "Token".into(),
                secret: true,
                ..SettingText::default()
            }]
        }
    }

    #[test]
    fn a_variable_dragged_onto_the_canvas_becomes_a_get_or_with_alt_a_set() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open(&mut host, "new.json", blank());
        let count = editor.declarations().unwrap().variables[0]
            .id
            .clone()
            .unwrap();
        for (set, kind) in [(false, "core/get-variable"), (true, "core/set-variable")] {
            let node = editor
                .drop_variable(&mut host, &count, set, [40.0, 80.0])
                .unwrap();
            let shown = editor
                .graph()
                .unwrap()
                .nodes
                .iter()
                .find(|found| found.id == node)
                .cloned()
                .unwrap();
            assert_eq!(shown.definition, kind);
            assert_eq!(shown.settings["variable"], json!(count));
        }
        // The list draws each variable with what it carries when dragged.
        editor.panels.nodes = true;
        editor.tab = Tab::Variables;
        draw(&mut editor, &mut host);
    }

    #[test]
    fn a_shared_variable_is_written_with_set_shared_and_the_graph_runs() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open(&mut host, "new.json", blank());
        let shared = editor
            .declare_variable(
                &mut host,
                &Variable {
                    id: None,
                    name: "shared".into(),
                    ty: "core/i64@1".into(),
                    scope: VarScope::Workspace,
                    persistent: false,
                    initial: Some("0".into()),
                },
            )
            .unwrap();
        let node = editor
            .drop_variable(&mut host, &shared, true, [40.0, 80.0])
            .unwrap();
        let graph = editor.graph().unwrap();
        let set = graph.nodes.iter().find(|found| found.id == node).unwrap();
        assert_eq!(set.definition, "core/set-shared");
        assert_eq!(set.settings["variable"], json!(shared));

        // The host learns the shared variables on the first run, so it starts.
        editor.run(&mut host);
        assert!(editor.is_running());
        assert_eq!(host.shared_variables().len(), 1);
        editor.stop(&mut host);
        // A second run finds them already shared.
        editor.run(&mut host);
        assert!(editor.is_running());
    }

    #[test]
    fn every_adapter_is_offered_and_a_typed_secret_never_reaches_the_document() {
        let store = Arc::new(Store::default());
        let package = orange_host::ServicePackage::new("acme.kit", "1.0.0")
            .adapter("acme.kit/service", Service);
        let options = HostOptions {
            secrets: Some(Arc::clone(&store) as Arc<dyn SecretStore>),
            ..HostOptions::default()
        };
        let mut host = Host::with_services(options, vec![package]).unwrap().0;
        let mut editor = Editor::new();
        editor.open(&mut host, "new.json", blank());

        // The form starts on an adapter the host has; a typed secret stays in the form,
        // out of its debug text, until it is saved.
        editor.panels.nodes = true;
        editor.tab = Tab::Connections;
        editor.forms.reveal_connection();
        draw(&mut editor, &mut host);
        assert_eq!(editor.forms.connection.adapter, "acme.kit/service");
        let _ = editor
            .forms
            .connection
            .secrets
            .values
            .insert("token".into(), "s3cret-value".into());
        assert!(!format!("{:?}", editor.forms).contains("s3cret-value"));

        let connection = Connection {
            id: None,
            name: "service".into(),
            adapter: "acme.kit/service".into(),
            scope: ConnectionScope::Graph,
            settings: json!({}),
        };
        let secrets = [("token".to_owned(), "s3cret-value".to_owned())];
        let id = editor
            .declare_connection_with_secrets(&mut host, &connection, &secrets)
            .unwrap();
        let text = editor.text().unwrap();
        assert!(!text.contains("s3cret-value"));
        assert!(text.contains(&format!(
            "credential://{}",
            orange_host::secret_name(&id, "token")
        )));
        assert_eq!(
            store
                .0
                .lock()
                .unwrap()
                .get(&orange_host::secret_name(&id, "token"))
                .map(String::as_str),
            Some("s3cret-value")
        );
        draw(&mut editor, &mut host);
    }

    #[test]
    fn variable_inspector_copies_full_text_beyond_the_preview() {
        use orange_testkit::ui::{click, text_center};
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut host, blank());
        let id = editor
            .declare_variable(
                &mut host,
                &Variable {
                    id: None,
                    name: "Response body".into(),
                    ty: "core/string@1".into(),
                    scope: VarScope::Run,
                    persistent: false,
                    initial: None,
                },
            )
            .unwrap();
        let full = format!("{}end", "x".repeat(8192));
        editor.document.as_mut().unwrap().values.insert(
            id,
            crate::ObservedVariable {
                shown: "short preview".into(),
                exact_text: Some(orange_host::Value::Str(std::sync::Arc::new(full.clone()))),
            },
        );

        let context = egui::Context::default();
        let mut frame = |events| {
            context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    editor.variables_tab(ui, &mut host);
                    editor.variable_inspector_window(ui.ctx());
                },
            )
        };
        frame(Vec::new()).drop_without_applying_deltas();
        let layout = frame(Vec::new());
        let inspect = text_center(&layout.shapes, t("inspect-value")).unwrap();
        layout.drop_without_applying_deltas();
        click(inspect, egui::PointerButton::Primary, |events| {
            frame(events).drop_without_applying_deltas();
        });
        let open = frame(Vec::new());
        let copy = text_center(&open.shapes, t("copy")).unwrap();
        open.drop_without_applying_deltas();
        let mut copied = false;
        click(copy, egui::PointerButton::Primary, |events| {
            let output = frame(events);
            copied |= output.platform_output.commands.iter().any(
                |command| matches!(command, egui::OutputCommand::CopyText(text) if text == &full),
            );
            output.drop_without_applying_deltas();
        });
        assert!(copied);
    }

    #[test]
    fn a_connection_created_from_properties_binds_only_its_target() {
        let (mut host, _) =
            Host::with_services(HostOptions::default(), vec![orange_net::package()]).unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut host, blank());
        let target = editor
            .add_node(&mut host, orange_net::HTTP_REQUEST)
            .unwrap();
        let other = editor
            .add_node(&mut host, orange_net::HTTP_REQUEST)
            .unwrap();
        let original = editor
            .graph()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.id == other)
            .unwrap()
            .settings["connection"]
            .clone();
        editor.forms.new_connection_for(target, HTTP);
        editor.forms.connection.name = "Second service".into();
        editor.forms.connection.settings = json!({"base_url": "https://example.test"}).to_string();
        assert!(editor.commit_connection_form(&mut host));
        let graph = editor.graph().unwrap();
        assert_ne!(
            graph
                .nodes
                .iter()
                .find(|node| node.id == target)
                .unwrap()
                .settings["connection"],
            original
        );
        assert_eq!(
            graph
                .nodes
                .iter()
                .find(|node| node.id == other)
                .unwrap()
                .settings["connection"],
            original
        );
    }

    #[test]
    fn properties_opens_the_nodes_connection_without_switching_tabs() {
        use orange_testkit::ui::{click, text_center};
        let (mut host, _) =
            Host::with_services(HostOptions::default(), vec![orange_net::package()]).unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut host, blank());
        let node = editor
            .add_node(&mut host, orange_net::HTTP_REQUEST)
            .unwrap();
        editor.canvas.select(Some(node));
        let profile = editor.declarations().unwrap().connections[0].id.clone();
        let context = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(820.0, 700.0));
        let mut frame = |events| {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    ..Default::default()
                },
                |ui| {
                    editor.ui_with_layout(ui, &mut host, |editor, ui, host| {
                        editor.properties_panel(ui, host);
                    })
                },
            );
            output.textures_delta.clear();
            output.shapes
        };
        frame(Vec::new());
        let position = text_center(&frame(Vec::new()), t("edit-connection")).unwrap();
        click(position, egui::PointerButton::Primary, |events| {
            frame(events);
        });
        frame(Vec::new());
        assert!(editor.forms.open_connection);
        assert_eq!(editor.forms.connection.changing, profile);
        assert!(editor.tab == super::super::Tab::Nodes);
        assert_eq!(editor.canvas.selected(), Some(node));
    }

    #[test]
    fn the_connection_dialog_keeps_actions_visible_and_checks_references_on_request() {
        use orange_testkit::ui::{click, text_center};
        let (mut host, _) =
            Host::with_services(HostOptions::default(), vec![orange_net::package()]).unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut host, blank());
        editor.forms.edit_connection(&Connection {
            id: Some("example".to_owned()),
            name: "Example".to_owned(),
            adapter: HTTP.to_owned(),
            scope: ConnectionScope::Graph,
            settings: json!({
                "base_url": "https://example.test",
                "basic": {"user": "example", "password": "credential://example-token"}
            }),
        });

        let adapters = host.adapter_list();
        let context = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(820.0, 560.0));
        let mut frame = |screen_rect, events| {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen_rect),
                    events,
                    ..egui::RawInput::default()
                },
                |ui| editor.connection_dialog(ui.ctx(), &mut host, &adapters),
            );
            output.textures_delta.clear();
            output.shapes
        };
        frame(screen, Vec::new());
        let shapes = frame(screen, Vec::new());
        for action in ["save-and-approve-credentials", "cancel", "check-settings"] {
            let position = text_center(&shapes, t(action)).unwrap();
            assert!(screen.shrink(8.0).contains(position));
        }
        let tall = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(820.0, 800.0));
        frame(tall, Vec::new());
        let check =
            text_center(&frame(tall, Vec::new()), t("connection-credential-check")).unwrap();
        click(check, egui::PointerButton::Primary, |events| {
            frame(tall, events);
        });
        frame(tall, Vec::new());
        assert_eq!(
            editor.forms.connection.secrets.local_status["credential://example-token"],
            "connection-credential-unavailable"
        );
        assert!(editor.forms.connection.secrets.values.is_empty());
    }

    #[test]
    fn creating_a_typed_run_event_assigns_and_restores_its_references() {
        use orange_testkit::ui::{click, text_center};
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut host, blank());
        let awaiter = editor.add_node(&mut host, "core/await-event").unwrap();
        editor.new_event_for_node(awaiter);
        let form = editor.forms.event.as_mut().unwrap();
        form.name = "Reading received".into();
        form.payload = "core/i64@1".into();

        let context = egui::Context::default();
        let mut frame = |events| {
            let mut output = context.run_ui(
                egui::RawInput {
                    events,
                    ..egui::RawInput::default()
                },
                |ui| editor.event_dialog(ui.ctx(), &mut host),
            );
            output.textures_delta.clear();
            output.shapes
        };
        frame(Vec::new());
        let create = text_center(&frame(Vec::new()), t("event-create-bind")).unwrap();
        click(create, egui::PointerButton::Primary, |events| {
            frame(events);
        });
        let [event] = editor.declarations().unwrap().events.as_slice() else {
            panic!("expected one event");
        };
        let event = event.id.to_string();
        editor.undo(&mut host);
        assert!(editor.declarations().unwrap().events.is_empty());
        assert!(
            editor.graph().unwrap().nodes[0]
                .settings
                .get("event")
                .is_none()
        );
        editor.redo(&mut host);

        for kind in ["core/emit", "core/on-event"] {
            let node = editor.add_node(&mut host, kind).unwrap();
            assert!(editor.set_setting(&mut host, node, "/event", Some(json!(event))));
        }
        let text = editor.text().unwrap();
        editor.open_untitled(&mut host, text);
        let declared = &editor.declarations().unwrap().events[0];
        assert_eq!(declared.name, "Reading received");
        assert_eq!(declared.payload.as_ref().unwrap().as_str(), "core/i64@1");
        for node in &editor.graph().unwrap().nodes {
            assert_eq!(node.settings["event"], event);
            assert!(
                node.ports
                    .iter()
                    .any(|port| port.id == "payload" && port.ty.as_deref() == Some("core/i64@1"))
            );
        }
        let graph = editor.graph().unwrap();
        let emit = graph
            .nodes
            .iter()
            .find(|node| node.definition == "core/emit")
            .unwrap()
            .id;
        let receive = graph
            .nodes
            .iter()
            .find(|node| node.definition == "core/on-event")
            .unwrap()
            .id;
        let manual = editor.add_node(&mut host, "core/on-manual").unwrap();
        let store = editor.add_node(&mut host, "core/set-variable").unwrap();
        let value = editor
            .declare_variable_for(
                &mut host,
                store,
                "/variable",
                &Variable {
                    id: None,
                    name: "Received".into(),
                    ty: "core/i64@1".into(),
                    scope: VarScope::Run,
                    persistent: false,
                    initial: Some("0".into()),
                },
            )
            .unwrap()
            .parse()
            .unwrap();
        assert!(editor.connect(&mut host, (manual, "then"), (emit, "in")));
        assert!(editor.set_input(&mut host, (emit, "payload"), Some("42")));
        assert!(editor.connect(&mut host, (receive, "then"), (store, "in")));
        assert!(editor.connect(&mut host, (receive, "payload"), (store, "value")));
        editor.run(&mut host);
        let flow = editor.document.as_ref().unwrap().flow.as_ref().unwrap().0;
        editor.trigger(&host, manual);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while host.variable_value(flow, value).unwrap() != Some(orange_host::Value::I64(42)) {
            assert!(
                std::time::Instant::now() < deadline,
                "event did not deliver its typed value"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        editor.stop(&mut host);
    }

    #[test]
    fn quick_create_and_bind_respect_shared_variable_scopes() {
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut host, blank());
        for kind in ["core/set-shared", "core/update-shared"] {
            let node = editor.add_node(&mut host, kind).unwrap();
            editor.new_variable_for_node(node, "/variable");
            assert_eq!(editor.forms.variable.scope, VarScope::Workspace);
            let mut variable = Variable {
                id: None,
                name: kind.to_owned(),
                ty: "core/i64@1".into(),
                scope: VarScope::Run,
                persistent: false,
                initial: Some("0".into()),
            };
            let count = editor.declarations().unwrap().variables.len();
            assert!(
                editor
                    .declare_variable_for(&mut host, node, "/variable", &variable)
                    .is_none()
            );
            assert_eq!(editor.declarations().unwrap().variables.len(), count);
            variable.scope = VarScope::Workspace;
            assert!(
                editor
                    .declare_variable_for(&mut host, node, "/variable", &variable)
                    .is_some()
            );
        }
    }

    #[test]
    fn newly_placed_stream_nodes_have_explicit_bounded_settings() {
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut host, blank());
        let _ = editor.add_node(&mut host, "core/collect").unwrap();
        let _ = editor.add_node(&mut host, "core/tee").unwrap();
        let graph = editor.graph().unwrap();
        let collect = graph
            .nodes
            .iter()
            .find(|node| node.definition == "core/collect")
            .unwrap();
        let tee = graph
            .nodes
            .iter()
            .find(|node| node.definition == "core/tee")
            .unwrap();
        assert_eq!(collect.settings["max_items"], 1024);
        assert_eq!(tee.settings["capacity"], 32);
        assert_eq!(tee.settings["max_bytes"], 1_048_576);
        assert_eq!(tee.settings["overflow"], "backpressure");
    }

    #[test]
    fn saving_changes_keeps_the_profile_open_and_replaces_a_draft_secret_with_its_reference() {
        let store = Arc::new(Store::default());
        let options = HostOptions {
            secrets: Some(Arc::clone(&store) as Arc<dyn SecretStore>),
            ..HostOptions::default()
        };
        let (mut host, _) = Host::with_services(options, vec![orange_net::package()]).unwrap();
        let mut editor = Editor::new();
        editor.open(&mut host, "test.json", blank());
        let connection = Connection {
            id: None,
            name: "API".to_owned(),
            adapter: HTTP.to_owned(),
            scope: ConnectionScope::Graph,
            settings: json!({"base_url": "https://example.test"}),
        };
        let id = editor.declare_connection(&mut host, &connection).unwrap();
        let saved = editor.declarations().unwrap().connections[0].clone();
        editor.forms.edit_connection(&saved);
        let form = &mut editor.forms.connection;
        form.name = "Edited API".to_owned();
        form.auth = Some(network::Auth::Bearer);
        form.secrets
            .values
            .insert("bearer".to_owned(), "new-test-secret".to_owned());

        assert!(!editor.commit_connection_form(&mut host));
        assert!(editor.forms.open_connection);
        let form = &editor.forms.connection;
        assert_eq!(form.changing.as_deref(), Some(id.as_str()));
        assert_eq!(form.name, "Edited API");
        assert!(form.saved);
        assert!(form.secrets.values.is_empty());
        assert!(form.settings.contains("credential://"));
        assert!(!form.settings.contains("new-test-secret"));
        assert_eq!(editor.declarations().unwrap().connections.len(), 1);
    }

    #[test]
    fn choosing_a_folder_updates_only_the_open_folder_draft() {
        let mut editor = Editor::new();
        let path = r"C:\Users\editor\Media";
        assert!(!editor.choose_connection_folder(path));
        editor.forms.open_connection = true;
        editor.forms.connection.adapter = FOLDER_ADAPTER.to_owned();
        editor.forms.connection.scope = ConnectionScope::Graph;
        editor.forms.connection.settings = json!({"path": "old", "mode": "read-only"}).to_string();
        editor.forms.connection.saved = true;

        assert!(editor.choose_connection_folder(path));
        let settings: Json = serde_json::from_str(&editor.forms.connection.settings).unwrap();
        assert_eq!(settings, json!({"path": path, "mode": "read-only"}));
        assert!(!editor.forms.connection.saved);

        editor.forms.connection.adapter = HTTP.to_owned();
        assert!(!editor.choose_connection_folder("ignored"));
        assert_eq!(
            serde_json::from_str::<Json>(&editor.forms.connection.settings).unwrap(),
            settings
        );
    }

    #[test]
    fn nested_text_settings_are_set_and_taken_out_with_their_empty_objects() {
        let mut settings = json!({"url": "x"});
        set_text(&mut settings, "basic/user", Some(json!("me")));
        assert_eq!(settings, json!({"url": "x", "basic": {"user": "me"}}));
        set_text(&mut settings, "basic/user", None);
        assert_eq!(settings, json!({"url": "x"}));
    }
}
