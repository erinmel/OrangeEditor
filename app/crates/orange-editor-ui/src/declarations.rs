//! The Document tab: the variables and connections the open document declares, with
//! forms to add, change and remove them. A connection's secrets are never typed here:
//! a setting that holds one is a `credential://<name>` reference to the system's store.
use super::*;
use crate::properties::COMMON_TYPES;
use orange_host::{Connection, ConnectionScope, Declarations, VarScope, Variable};

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
        let text = self.editing()?;
        match host.declare_variable(&text, variable) {
            Ok((text, id)) => {
                let _ = self.edited(host, Ok(text));
                Some(id)
            }
            Err(error) => {
                let _ = self.edited(host, Err(error));
                None
            }
        }
    }

    /// Removes a variable; nodes that name it show as problems.
    pub fn remove_variable(&mut self, host: &mut Host, id: &str) -> bool {
        let Some(text) = self.editing() else {
            return false;
        };
        let result = host.remove_variable(&text, id);
        self.edited(host, result)
    }

    /// Declares a connection, or changes the one with `connection.id`. A secret written
    /// in its settings instead of a `credential://` reference is refused. Returns its ID.
    pub fn declare_connection(
        &mut self,
        host: &mut Host,
        connection: &Connection,
    ) -> Option<String> {
        let text = self.editing()?;
        match host.declare_connection(&text, connection) {
            Ok((text, id)) => {
                let _ = self.edited(host, Ok(text));
                Some(id)
            }
            Err(error) => {
                let _ = self.edited(host, Err(error));
                None
            }
        }
    }

    /// Removes a connection; nodes that use it show as problems.
    pub fn remove_connection(&mut self, host: &mut Host, id: &str) -> bool {
        let Some(text) = self.editing() else {
            return false;
        };
        let result = host.remove_connection(&text, id);
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
        egui::CollapsingHeader::new(title)
            .id_salt("orange-variable-form")
            .default_open(false)
            .open(form.changing.is_some().then_some(true))
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
        let form = &mut self.forms.connection;
        if form.adapter.is_empty()
            && let Some(first) = adapters.first()
        {
            form.adapter.clone_from(first);
        }
        let title = if form.changing.is_some() {
            "Change connection"
        } else {
            "New connection"
        };
        let mut declare = false;
        let mut cancel = false;
        egui::CollapsingHeader::new(title)
            .id_salt("orange-connection-form")
            .default_open(false)
            .open(form.changing.is_some().then_some(true))
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
        if let Some(id) = remove {
            let _ = self.remove_connection(host, &id);
        }
    }
}
