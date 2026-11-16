//! The Document tab: the variables and connections the open document declares, with
//! forms to add, change and remove them. A connection's secrets are never typed here:
//! a setting that holds one is a `credential://<name>` reference to the system's store.
use super::*;
use crate::properties::COMMON_TYPES;
use orange_host::{
    Connection, ConnectionScope, Declarations, SettingChoices, SettingField, VarScope, Variable,
};
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
}

impl Default for ConnectionForm {
    fn default() -> Self {
        Self {
            changing: None,
            name: String::new(),
            adapter: String::new(),
            scope: ConnectionScope::Graph,
            settings: "{}".to_owned(),
        }
    }
}

/// The forms of the Document tab.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Forms {
    variable: VariableForm,
    connection: ConnectionForm,
    /// Open the new-variable or new-connection form at the next frame, as asked from
    /// the toolbar or the properties panel.
    open_variable: bool,
    open_connection: bool,
    /// The variable form is shown.
    variable_open: bool,
}

impl Forms {
    pub(crate) fn reveal_variable(&mut self) {
        self.open_variable = true;
    }
    pub(crate) fn reveal_connection(&mut self) {
        self.open_connection = true;
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
                .map_or(value, String::as_str)
                .to_owned()
        };
        ui.horizontal(|ui| {
            ui.label(choice.setting.as_str());
            let shown = current.map_or_else(|| choice.unset.clone(), label);
            let _ = egui::ComboBox::from_id_salt(("orange-setting-choice", salt, &choice.setting))
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(current.is_none(), choice.unset.as_str())
                        .clicked()
                        && current.is_some()
                    {
                        chosen = Some((choice.setting.clone(), None));
                    }
                    for value in &choice.values {
                        let selected = current == Some(value.as_str());
                        if ui
                            .selectable_label(selected, label(value))
                            .on_hover_text(value.as_str())
                            .clicked()
                            && !selected
                        {
                            chosen = Some((choice.setting.clone(), Some(value.clone())));
                        }
                    }
                });
            *refresh |= ui
                .small_button("Refresh")
                .on_hover_text("List them again")
                .clicked();
        });
        if let Some(problem) = &choice.problem {
            ui.weak(format!("Could not list them: {problem}"));
        }
        if let Some(value) = current
            && !choice.values.iter().any(|known| known == value)
        {
            ui.weak(format!("“{value}” is not on this machine now."));
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
                        .small_button("×")
                        .on_hover_text("Leave it out: use the default.")
                        .clicked()
                    {
                        changed = Some((field.setting.clone(), None));
                    }
                }
                None => {
                    let start = field.default.unwrap_or((field.min + field.max) / 2.0);
                    match field.default {
                        Some(default) => ui.weak(format!("default, {default} {}", field.unit)),
                        None => ui.weak("automatic"),
                    }
                    .on_hover_text(field.help.as_str());
                    if ui.small_button("Set").clicked() {
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
fn live_level(
    ui: &mut egui::Ui,
    host: &Host,
    form: &ConnectionForm,
    choices: &[SettingChoices],
    fields: &[SettingField],
    probing: &mut Option<crate::Probing>,
) {
    let mut on = probing.is_some();
    if ui
        .checkbox(&mut on, "Show the live level")
        .on_hover_text("Listens to the device now, apart from any run.")
        .changed()
        && !on
    {
        *probing = None;
    }
    if !on {
        return;
    }
    let Ok(Json::Object(settings)) = serde_json::from_str::<Json>(&form.settings) else {
        ui.weak("The settings are not JSON.");
        return;
    };
    // What chooses the device: changing it listens again; moving the trigger's levels
    // does not reopen the device.
    let key: String = choices
        .iter()
        .map(|choice| format!("{:?};", settings.get(&choice.setting)))
        .collect();
    let stale = probing.as_ref().is_none_or(|probing| {
        probing.adapter != form.adapter || probing.settings != key || probing.probe.is_err()
    });
    if stale {
        *probing = host
            .probe(&form.adapter, &settings)
            .map(|probe| crate::Probing {
                adapter: form.adapter.clone(),
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

/// The loudest and quietest levels a level meter shows, in dBFS.
const METER_DB: std::ops::RangeInclusive<f64> = -90.0..=0.0;

/// A bar showing `level` in dBFS, with a line at each of `marks` (a level and what it
/// is), or why there is no level.
pub(crate) fn level_meter(ui: &mut egui::Ui, level: Result<f64, String>, marks: &[(f64, &str)]) {
    let level = match level {
        Ok(level) => level,
        Err(why) => {
            ui.weak(format!("No level: {why}"));
            return;
        }
    };
    let width = ui.available_width().max(60.0);
    let height = ui.spacing().interact_size.y * 0.7;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let span = METER_DB.end() - METER_DB.start();
    #[expect(clippy::cast_possible_truncation, reason = "a fraction of a width")]
    let at = |db: f64| {
        let fraction =
            ((db.clamp(*METER_DB.start(), *METER_DB.end()) - METER_DB.start()) / span) as f32;
        rect.left() + fraction * rect.width()
    };
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
    ui.label(format!("{level:.1} dBFS now"));
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
        VarScope::Local => "local",
        VarScope::Run => "each run",
        VarScope::Workspace => "workspace",
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
        match host.declare_variable(draft, variable) {
            Ok(id) => {
                let _ = self.edited(host, Ok(true));
                Some(id)
            }
            Err(error) => {
                let _ = self.edited(host, Err(error));
                None
            }
        }
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
        match host.declare_connection(draft, connection) {
            Ok(id) => {
                let _ = self.edited(host, Ok(true));
                Some(id)
            }
            Err(error) => {
                let _ = self.edited(host, Err(error));
                None
            }
        }
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

    pub(crate) fn variables_tab(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        let Some(declared) = self.declarations().cloned() else {
            ui.weak("Open a graph to see its variables.");
            return;
        };
        egui::ScrollArea::vertical().show(ui, |ui| {
            self.variables_section(ui, host, &declared);
        });
    }

    pub(crate) fn connections_tab(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        let Some(declared) = self.declarations().cloned() else {
            ui.weak("Open a graph to see its connections.");
            return;
        };
        egui::ScrollArea::vertical().show(ui, |ui| {
            self.connections_section(ui, host, &declared);
        });
    }

    fn variables_section(&mut self, ui: &mut egui::Ui, host: &mut Host, declared: &Declarations) {
        // A heading with its add button, as in a blueprint's variable list.
        if self.forms.open_variable {
            self.forms.open_variable = false;
            self.forms.variable_open = true;
        }
        let adding = section_heading(ui, "Variables", "Add a variable");
        if adding {
            self.forms.variable = VariableForm::default();
            self.forms.variable_open = true;
        }
        let mut remove = None;
        for variable in &declared.variables {
            let detail = format!(
                "{}, {}{}{}",
                exact_type(&variable.ty),
                scope_name(variable.scope),
                if variable.persistent {
                    ", kept between runs"
                } else {
                    ""
                },
                variable
                    .initial
                    .as_ref()
                    .map_or_else(String::new, |initial| format!(", starts at {initial}"))
            );
            let (edit, delete) =
                declaration_row(ui, &variable.name, &type_name(&variable.ty), &detail);
            if edit {
                self.forms.variable = VariableForm {
                    changing: variable.id.clone(),
                    name: variable.name.clone(),
                    ty: variable.ty.clone(),
                    scope: variable.scope,
                    persistent: variable.persistent,
                    initial: variable.initial.clone().unwrap_or_default(),
                };
                self.forms.variable_open = true;
            }
            if delete {
                remove = variable.id.clone();
            }
        }
        if declared.variables.is_empty() && !self.forms.variable_open {
            ui.weak("No variables yet. + adds one.");
        }
        let mut declare = false;
        let mut cancel = false;
        if self.forms.variable_open {
            let form = &mut self.forms.variable;
            let changing = form.changing.is_some();
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.strong(if changing {
                    "Change variable"
                } else {
                    "New variable"
                });
                egui::Grid::new("orange-variable-form")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Name");
                        let _ = ui.add(
                            egui::TextEdit::singleline(&mut form.name)
                                .hint_text("such as count")
                                .desired_width(ui.available_width()),
                        );
                        ui.end_row();
                        ui.label("Type");
                        let _ = egui::ComboBox::from_id_salt("orange-variable-type")
                            .selected_text(type_name(&form.ty))
                            .show_ui(ui, |ui| {
                                for ty in COMMON_TYPES {
                                    let _ = ui.selectable_value(
                                        &mut form.ty,
                                        ty.to_owned(),
                                        type_name(ty),
                                    );
                                }
                            });
                        ui.end_row();
                        ui.label("Lasts")
                            .on_hover_text("How long the value is kept while the graph runs.");
                        let _ = egui::ComboBox::from_id_salt("orange-variable-scope")
                            .selected_text(scope_name(form.scope))
                            .show_ui(ui, |ui| {
                                for scope in [VarScope::Run, VarScope::Workspace, VarScope::Local] {
                                    let _ = ui.selectable_value(
                                        &mut form.scope,
                                        scope,
                                        scope_name(scope),
                                    );
                                }
                            });
                        ui.end_row();
                        ui.label("Starts at");
                        let _ = ui.add(
                            egui::TextEdit::singleline(&mut form.initial)
                                .hint_text("empty until written")
                                .desired_width(ui.available_width()),
                        );
                        ui.end_row();
                        ui.label("");
                        let _ = ui.checkbox(&mut form.persistent, "Keep between runs");
                        ui.end_row();
                    });
                ui.horizontal(|ui| {
                    let ready = !form.name.trim().is_empty();
                    declare = ui
                        .add_enabled(
                            ready,
                            egui::Button::new(if changing { "Save" } else { "Add" }),
                        )
                        .clicked();
                    cancel = ui.button("Cancel").clicked();
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
            if self.declare_variable(host, &variable).is_some() {
                self.forms.variable = VariableForm::default();
                self.forms.variable_open = false;
            }
        }
        if cancel {
            self.forms.variable = VariableForm::default();
            self.forms.variable_open = false;
        }
        if let Some(id) = remove {
            let _ = self.remove_variable(host, &id);
        }
    }

    fn connections_section(&mut self, ui: &mut egui::Ui, host: &mut Host, declared: &Declarations) {
        if section_heading(ui, "Connections", "Add a connection") {
            self.forms.connection = ConnectionForm::default();
            self.forms.open_connection = true;
        }
        let mut remove = None;
        for connection in &declared.connections {
            let kind = connection
                .adapter
                .rsplit('/')
                .next()
                .unwrap_or(&connection.adapter);
            let (edit, delete) = declaration_row(ui, &connection.name, kind, &connection.adapter);
            {
                if edit {
                    self.forms.connection = ConnectionForm {
                        changing: connection.id.clone(),
                        name: connection.name.clone(),
                        adapter: connection.adapter.clone(),
                        scope: connection.scope,
                        settings: serde_json::to_string_pretty(&connection.settings)
                            .unwrap_or_default(),
                    };
                }
                if delete {
                    remove = connection.id.clone();
                }
            }
        }
        if declared.connections.is_empty() {
            ui.weak("No connections yet. + adds one.");
        }
        let adapters = host.adapters();
        if self.forms.connection.adapter.is_empty()
            && let Some(first) = adapters.first()
        {
            self.forms.connection.adapter.clone_from(first);
        }
        let adapter = self.forms.connection.adapter.clone();
        let choices = self.adapter_choices(host, &adapter, false);
        let fields = host.adapter_fields(&adapter);
        let probes = host.adapter_probes(&adapter);
        let probing = &mut self.probing;
        let mut refresh = false;
        let form = &mut self.forms.connection;
        let title = if form.changing.is_some() {
            "Change connection"
        } else {
            "New connection"
        };
        let mut declare = false;
        let mut cancel = false;
        let reveal = std::mem::take(&mut self.forms.open_connection);
        let form = &mut self.forms.connection;
        let shown = egui::CollapsingHeader::new(title)
            .id_salt("orange-connection-form")
            .default_open(false)
            .open((form.changing.is_some() || reveal).then_some(true))
            .show(ui, |ui| {
                let _ = ui.add(egui::TextEdit::singleline(&mut form.name).hint_text("name"));
                let _ = egui::ComboBox::from_id_salt("orange-connection-adapter")
                    .selected_text(form.adapter.as_str())
                    .show_ui(ui, |ui| {
                        for adapter in &adapters {
                            let _ = ui.selectable_value(
                                &mut form.adapter,
                                adapter.clone(),
                                adapter.as_str(),
                            );
                        }
                    });
                ui.horizontal(|ui| {
                    let _ = ui.radio_value(&mut form.scope, ConnectionScope::Graph, "this graph");
                    let _ = ui
                        .radio_value(&mut form.scope, ConnectionScope::Shared, "shared")
                        .on_hover_text("The host holds the settings; graphs share one session.");
                });
                if !choices.is_empty()
                    && let Ok(current) = serde_json::from_str::<Json>(&form.settings)
                    && let Some((setting, value)) =
                        choice_fields(ui, "form", &choices, &current, &mut refresh)
                {
                    form.settings =
                        serde_json::to_string_pretty(&with_setting(&current, &setting, value))
                            .unwrap_or_default();
                }
                if !fields.is_empty()
                    && let Ok(current) = serde_json::from_str::<Json>(&form.settings)
                    && let Some((setting, value)) = number_fields(ui, &fields, &current)
                {
                    form.settings =
                        serde_json::to_string_pretty(&with_number(&current, &setting, value))
                            .unwrap_or_default();
                }
                if probes {
                    live_level(ui, host, form, &choices, &fields, probing);
                }
                ui.label("settings (JSON)");
                let _ = ui.add(
                    egui::TextEdit::multiline(&mut form.settings)
                        .code_editor()
                        .desired_rows(4)
                        .desired_width(f32::INFINITY),
                );
                ui.weak("Write a secret as \"credential://name\": the secret stays in the system's store, never in the graph.");
                ui.horizontal(|ui| {
                    let ready = !form.name.trim().is_empty() && !form.adapter.is_empty();
                    let label = if form.changing.is_some() { "Change" } else { "Declare" };
                    declare = ui.add_enabled(ready, egui::Button::new(label)).clicked();
                    cancel = form.changing.is_some() && ui.button("Cancel").clicked();
                });
            });
        // A closed form shows no level, and the device it listened to is let go.
        if shown.body_returned.is_none() || declare || cancel {
            self.probing = None;
        }
        if declare {
            let form = self.forms.connection.clone();
            match serde_json::from_str(&form.settings) {
                Ok(settings) => {
                    let connection = Connection {
                        id: form.changing,
                        name: form.name.trim().to_owned(),
                        adapter: form.adapter,
                        scope: form.scope,
                        settings,
                    };
                    if self.declare_connection(host, &connection).is_some() {
                        self.forms.connection = ConnectionForm::default();
                    }
                }
                Err(error) => self.say(format!("not done: the settings are not JSON: {error}")),
            }
        }
        if cancel {
            self.forms.connection = ConnectionForm::default();
        }
        if refresh {
            let _ = self.adapter_choices(host, &adapter, true);
        }
        if let Some(id) = remove {
            let _ = self.remove_connection(host, &id);
        }
    }
}

/// A section's heading with an add button at its right end. Returns whether it was
/// pressed.
fn section_heading(ui: &mut egui::Ui, title: &str, add: &str) -> bool {
    ui.horizontal(|ui| {
        ui.heading(title);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.button("+").on_hover_text(add).clicked()
        })
        .inner
    })
    .inner
}

/// One declaration in a list: its name, what it is in weak text, and edit and delete
/// buttons at the right end. `detail` is shown on hover. Returns whether each was pressed.
fn declaration_row(ui: &mut egui::Ui, name: &str, kind: &str, detail: &str) -> (bool, bool) {
    ui.horizontal(|ui| {
        let _ = ui.label(name).on_hover_text(detail);
        ui.weak(kind);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let delete = ui.small_button("Delete").clicked();
            let edit = ui.small_button("Edit").clicked();
            (edit, delete)
        })
        .inner
    })
    .inner
}
