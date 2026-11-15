//! The Document tab: the variables and connections the open document declares, with
//! forms to add, change and remove them. A connection's secrets are never typed here:
//! a setting that holds one is a `credential://<name>` reference to the system's store.
use super::*;
use crate::properties::COMMON_TYPES;
use orange_host::{Connection, ConnectionScope, Declarations, SettingChoices, VarScope, Variable};
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
        ui.horizontal(|ui| {
            ui.label(choice.setting.as_str());
            let _ = egui::ComboBox::from_id_salt(("orange-setting-choice", salt, &choice.setting))
                .selected_text(current.unwrap_or(choice.unset.as_str()))
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
                        if ui.selectable_label(selected, value.as_str()).clicked() && !selected {
                            chosen = Some((choice.setting.clone(), Some(value.clone())));
                        }
                    }
                });
            *refresh |= ui
                .small_button("↻")
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

    pub(crate) fn document_tab(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        let Some(declared) = self.declarations().cloned() else {
            ui.weak("Open a graph to see what it declares.");
            return;
        };
        egui::ScrollArea::vertical().show(ui, |ui| {
            self.variables_section(ui, host, &declared);
            ui.separator();
            self.connections_section(ui, host, &declared);
        });
    }

    fn variables_section(&mut self, ui: &mut egui::Ui, host: &mut Host, declared: &Declarations) {
        ui.heading("Variables");
        let mut remove = None;
        for variable in &declared.variables {
            ui.horizontal(|ui| {
                let _ = ui
                    .label(format!("{} ({})", variable.name, type_name(&variable.ty)))
                    .on_hover_text(format!(
                        "{}{}{}",
                        scope_name(variable.scope),
                        if variable.persistent { ", kept" } else { "" },
                        variable
                            .initial
                            .as_ref()
                            .map_or_else(String::new, |initial| format!(", starts at {initial}"))
                    ));
                if ui.small_button("Change").clicked() {
                    self.forms.variable = VariableForm {
                        changing: variable.id.clone(),
                        name: variable.name.clone(),
                        ty: variable.ty.clone(),
                        scope: variable.scope,
                        persistent: variable.persistent,
                        initial: variable.initial.clone().unwrap_or_default(),
                    };
                }
                if ui.small_button("Remove").clicked() {
                    remove = variable.id.clone();
                }
            });
        }
        if declared.variables.is_empty() {
            ui.weak("None yet.");
        }
        let form = &mut self.forms.variable;
        let title = if form.changing.is_some() {
            "Change variable"
        } else {
            "New variable"
        };
        let mut declare = false;
        let mut cancel = false;
        let reveal = std::mem::take(&mut self.forms.open_variable);
        let form = &mut self.forms.variable;
        egui::CollapsingHeader::new(title)
            .id_salt("orange-variable-form")
            .default_open(false)
            .open((form.changing.is_some() || reveal).then_some(true))
            .show(ui, |ui| {
                let _ = ui.add(egui::TextEdit::singleline(&mut form.name).hint_text("name"));
                let _ = egui::ComboBox::from_id_salt("orange-variable-type")
                    .selected_text(type_name(&form.ty))
                    .show_ui(ui, |ui| {
                        for ty in COMMON_TYPES {
                            let _ = ui.selectable_value(&mut form.ty, ty.to_owned(), type_name(ty));
                        }
                    });
                let _ = egui::ComboBox::from_id_salt("orange-variable-scope")
                    .selected_text(scope_name(form.scope))
                    .show_ui(ui, |ui| {
                        for scope in [VarScope::Run, VarScope::Workspace, VarScope::Local] {
                            let _ = ui.selectable_value(&mut form.scope, scope, scope_name(scope));
                        }
                    });
                let _ = ui.checkbox(&mut form.persistent, "kept between runs");
                let _ = ui.add(
                    egui::TextEdit::singleline(&mut form.initial).hint_text("starts at (optional)"),
                );
                ui.horizontal(|ui| {
                    let ready = !form.name.trim().is_empty();
                    let label = if form.changing.is_some() {
                        "Change"
                    } else {
                        "Declare"
                    };
                    declare = ui.add_enabled(ready, egui::Button::new(label)).clicked();
                    cancel = form.changing.is_some() && ui.button("Cancel").clicked();
                });
            });
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
            }
        }
        if cancel {
            self.forms.variable = VariableForm::default();
        }
        if let Some(id) = remove {
            let _ = self.remove_variable(host, &id);
        }
    }

    fn connections_section(&mut self, ui: &mut egui::Ui, host: &mut Host, declared: &Declarations) {
        ui.heading("Connections");
        let mut remove = None;
        for connection in &declared.connections {
            ui.horizontal(|ui| {
                let _ = ui
                    .label(connection.name.as_str())
                    .on_hover_text(connection.adapter.as_str());
                if ui.small_button("Change").clicked() {
                    self.forms.connection = ConnectionForm {
                        changing: connection.id.clone(),
                        name: connection.name.clone(),
                        adapter: connection.adapter.clone(),
                        scope: connection.scope,
                        settings: serde_json::to_string_pretty(&connection.settings)
                            .unwrap_or_default(),
                    };
                }
                if ui.small_button("Remove").clicked() {
                    remove = connection.id.clone();
                }
            });
        }
        if declared.connections.is_empty() {
            ui.weak("None yet.");
        }
        let adapters = host.adapters();
        if self.forms.connection.adapter.is_empty()
            && let Some(first) = adapters.first()
        {
            self.forms.connection.adapter.clone_from(first);
        }
        let adapter = self.forms.connection.adapter.clone();
        let choices = self.adapter_choices(host, &adapter, false);
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
        egui::CollapsingHeader::new(title)
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
