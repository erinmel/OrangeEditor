//! The Variables and Connections tabs: what the open document declares, with forms to
//! add, change and remove it. A connection's settings are drawn from what its adapter
//! says about them; a secret is typed masked, kept in the host's credential store and
//! written into the document only as its `credential://<name>` reference.
use super::*;
use crate::properties::COMMON_TYPES;
use orange_host::{
    AdapterInfo, Connection, ConnectionScope, Declarations, SECRET_PREFIX, SettingChoices,
    SettingField, SettingText, SettingToggle, VarScope, Variable,
};
use std::collections::BTreeMap;

/// What a variable dragged from the Variables list carries, before its ID. Let go on
/// the canvas, it becomes a Get node, or a Set node with Alt held.
pub(crate) const VARIABLE_DRAG: &str = "orange-variable:";

/// Secrets typed into the connection form, by setting, until it is saved. Never shown
/// in debug output.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Typed(BTreeMap<String, String>);
impl std::fmt::Debug for Typed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(
                self.0
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
            tested: None,
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
    /// What is being typed in place in the Variables list, by variable ID and field,
    /// while the field has the focus.
    inline: BTreeMap<(String, &'static str), String>,
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

    /// Adds a Get node for variable `id` where it was let go, or a Set node with `set`.
    /// Returns the node.
    pub fn drop_variable(
        &mut self,
        host: &mut Host,
        id: &str,
        set: bool,
        at: [f32; 2],
    ) -> Option<NodeId> {
        let definition = if set {
            "core/set-variable"
        } else {
            "core/get-variable"
        };
        let node = self.add_node_at(host, definition, at)?;
        let _ = self.set_setting(host, node, "/variable", Some(Json::from(id)));
        self.canvas.select(Some(node));
        Some(node)
    }

    /// A field typed in place in the Variables list: `shown` until it has the focus,
    /// then what is typed. Returns the text when the field is left with it changed.
    fn inline_field(
        &mut self,
        ui: &mut egui::Ui,
        key: (String, &'static str),
        shown: &str,
    ) -> Option<String> {
        let mut text = self
            .forms
            .inline
            .get(&key)
            .cloned()
            .unwrap_or_else(|| shown.to_owned());
        let response = ui.add(edit_like(&mut text, "unset"));
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
        let adding = section_heading(ui, "Variables", "Add a variable");
        if adding {
            self.forms.variable = VariableForm::default();
            self.forms.variable_open = true;
        }
        let running = self
            .document
            .as_ref()
            .is_some_and(|document| document.flow.is_some());
        let values = self
            .document
            .as_ref()
            .map(|document| document.values.clone())
            .unwrap_or_default();
        let mut remove = None;
        let mut change: Option<Variable> = None;
        for variable in &declared.variables {
            let Some(id) = variable.id.clone() else {
                continue;
            };
            ui.horizontal(|ui| {
                // Dragged by its handle onto the graph, it becomes a Get, or a Set with Alt.
                let drag = ui.scope_id().with(("orange-variable", id.as_str()));
                let _ = ui
                    .dnd_drag_source(drag, format!("{VARIABLE_DRAG}{id}"), |ui| {
                        ui.add(egui::Label::new("::").selectable(false))
                    })
                    .response
                    .on_hover_text(
                        "Drag onto the graph to read it (Get); hold Alt to write it (Set).",
                    );
                // Its name, renamed in place.
                if let Some(name) = self.inline_field(ui, (id.clone(), "name"), &variable.name)
                    && !name.is_empty()
                {
                    change = Some(Variable {
                        name,
                        ..variable.clone()
                    });
                }
                // Its type, as a coloured chip that changes it.
                let color = crate::canvas::type_color(Some(&variable.ty));
                let mut ty = variable.ty.clone();
                let _ = egui::ComboBox::from_id_salt(("orange-variable-type", id.as_str()))
                    .width(70.0)
                    .selected_text(egui::RichText::new(type_name(&variable.ty)).color(color))
                    .show_ui(ui, |ui| {
                        for common in COMMON_TYPES {
                            let _ =
                                ui.selectable_value(&mut ty, common.to_owned(), type_name(common));
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
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let _ = ui.menu_button("…", |ui| {
                        let mut kept = variable.clone();
                        ui.label("Lasts");
                        for scope in [VarScope::Run, VarScope::Workspace, VarScope::Local] {
                            let _ = ui.radio_value(&mut kept.scope, scope, scope_name(scope));
                        }
                        let _ = ui.checkbox(&mut kept.persistent, "Keep between runs");
                        if kept != *variable {
                            change = Some(kept);
                        }
                        ui.separator();
                        if ui.button("Delete").clicked() {
                            remove = Some(id.clone());
                            ui.close();
                        }
                    });
                    // Its value now while the graph runs; the last one, dimmed, after.
                    match values.get(&id) {
                        Some(value) if running => {
                            let _ = ui
                                .strong(value.as_str())
                                .on_hover_text("Its value now, in this run.");
                        }
                        Some(value) => {
                            let _ = ui
                                .weak(value.as_str())
                                .on_hover_text("Its value when the last run stopped.");
                        }
                        None => {}
                    }
                });
            });
            // Where it starts, typed in place.
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                ui.weak("starts at");
                let shown = variable.initial.clone().unwrap_or_default();
                if let Some(initial) = self.inline_field(ui, (id.clone(), "initial"), &shown) {
                    change = Some(Variable {
                        initial: Some(initial).filter(|text| !text.is_empty()),
                        ..variable.clone()
                    });
                }
            });
        }
        if declared.variables.is_empty() && !self.forms.variable_open {
            ui.weak("No variables yet. + adds one.");
        } else if !declared.variables.is_empty() {
            ui.weak("Drag :: onto the graph for a Get node, with Alt for a Set node.");
        }
        let mut declare = false;
        let mut cancel = false;
        if self.forms.variable_open {
            let form = &mut self.forms.variable;
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.strong("New variable");
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
                    declare = ui.add_enabled(ready, egui::Button::new("Add")).clicked();
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
        if let Some(variable) = change {
            let _ = self.declare_variable(host, &variable);
        }
        if let Some(id) = remove {
            let _ = self.remove_variable(host, &id);
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
        match host.declare_connection_with_secrets(draft, connection, secrets) {
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

    /// The nodes that use connection `id`, by what they are called.
    fn connection_users(&self, host: &Host, id: &str) -> Vec<String> {
        let Some(document) = &self.document else {
            return Vec::new();
        };
        host.connection_users(&document.draft, id)
            .iter()
            .map(|node| document.names.called(node))
            .collect()
    }

    fn connections_section(&mut self, ui: &mut egui::Ui, host: &mut Host, declared: &Declarations) {
        let adapters = host.adapter_list();
        let label_of = |adapter: &str| {
            adapters
                .iter()
                .find(|known| known.id == adapter)
                .map_or_else(|| adapter.to_owned(), |known| known.label.clone())
        };
        if section_heading(ui, "Connections", "Add a connection") {
            self.forms.connection = ConnectionForm::default();
            self.forms.open_connection = true;
        }
        let mut remove = None;
        for connection in &declared.connections {
            let id = connection.id.clone().unwrap_or_default();
            let users = self.connection_users(host, &id);
            let detail = format!("{}\n{}", label_of(&connection.adapter), connection.adapter);
            let (edit, delete) = declaration_row(ui, &connection.name, &detail);
            ui.horizontal_wrapped(|ui| {
                ui.add_space(12.0);
                if users.is_empty() {
                    ui.weak(format!("{}, not used yet", label_of(&connection.adapter)));
                } else {
                    let _ = ui
                        .weak(format!(
                            "{}, used by {} node{}: {}",
                            label_of(&connection.adapter),
                            users.len(),
                            if users.len() == 1 { "" } else { "s" },
                            users.join(", ")
                        ))
                        .on_hover_text(users.join("\n"));
                }
            });
            if edit {
                self.forms.connection = ConnectionForm {
                    changing: connection.id.clone(),
                    name: connection.name.clone(),
                    adapter: connection.adapter.clone(),
                    scope: connection.scope,
                    settings: serde_json::to_string_pretty(&connection.settings)
                        .unwrap_or_default(),
                    ..ConnectionForm::default()
                };
                self.probing = None;
            }
            if delete {
                remove = connection.id.clone();
            }
        }
        if declared.connections.is_empty() {
            ui.weak("No connections yet. + adds one.");
        }
        if self.forms.connection.adapter.is_empty()
            && let Some(first) = adapters.first()
        {
            self.forms.connection.adapter.clone_from(&first.id);
        }
        let adapter = self.forms.connection.adapter.clone();
        let choices = self.adapter_choices(host, &adapter, false);
        let fields = host.adapter_fields(&adapter);
        let texts = host.adapter_texts(&adapter);
        let toggles = host.adapter_toggles(&adapter);
        let probes = host.adapter_probes(&adapter);
        let mut refresh = false;
        let mut declare = false;
        let mut cancel = false;
        let mut test = false;
        let reveal = std::mem::take(&mut self.forms.open_connection);
        let probing = &mut self.probing;
        let form = &mut self.forms.connection;
        let title = if form.changing.is_some() {
            "Change connection"
        } else {
            "New connection"
        };
        let shown = egui::CollapsingHeader::new(title)
            .id_salt("orange-connection-form")
            .default_open(false)
            .open((form.changing.is_some() || reveal).then_some(true))
            .show(ui, |ui| {
                let _ = ui.add(
                    egui::TextEdit::singleline(&mut form.name)
                        .hint_text("name, such as weather api")
                        .desired_width(f32::INFINITY),
                );
                adapter_picker(ui, &adapters, form);
                ui.horizontal(|ui| {
                    let _ = ui.radio_value(&mut form.scope, ConnectionScope::Graph, "this graph");
                    let _ = ui
                        .radio_value(&mut form.scope, ConnectionScope::Shared, "shared")
                        .on_hover_text("The host holds the settings; graphs share one session.");
                });
                ui.separator();
                if let Ok(mut current) = serde_json::from_str::<Json>(&form.settings) {
                    let before = current.clone();
                    text_fields(ui, &texts, &mut current, &mut form.secrets);
                    toggle_fields(ui, &toggles, &mut current);
                    if let Some((setting, value)) =
                        choice_fields(ui, "form", &choices, &current, &mut refresh)
                    {
                        current = with_setting(&current, &setting, value);
                    }
                    if let Some((setting, value)) = number_fields(ui, &fields, &current) {
                        current = with_number(&current, &setting, value);
                    }
                    if current != before {
                        form.settings = serde_json::to_string_pretty(&current).unwrap_or_default();
                        form.tested = None;
                    }
                } else {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        "The settings below are not JSON.",
                    );
                }
                if probes {
                    live_level(ui, host, form, &choices, &fields, probing);
                }
                ui.horizontal(|ui| {
                    test = ui
                        .button("Check settings")
                        .on_hover_text(
                            "Checks whether the adapter can open with these settings. Some services connect on first use.",
                        )
                        .clicked();
                    match &form.tested {
                        Some(Ok(())) => {
                            ui.label("Settings accepted.");
                        }
                        Some(Err(why)) => {
                            ui.colored_label(ui.visuals().warn_fg_color, why.as_str());
                        }
                        None => {}
                    }
                });
                egui::CollapsingHeader::new("All settings (JSON)")
                    .id_salt("orange-connection-json")
                    .default_open(texts.is_empty() && choices.is_empty() && fields.is_empty())
                    .show(ui, |ui| {
                        let _ = ui.add(
                            egui::TextEdit::multiline(&mut form.settings)
                                .code_editor()
                                .desired_rows(4)
                                .desired_width(f32::INFINITY),
                        );
                        ui.weak("A secret is written as \"credential://name\": it stays in the system's credential store, never in the graph.");
                    });
                ui.horizontal(|ui| {
                    let ready = !form.name.trim().is_empty() && !form.adapter.is_empty();
                    let label = if form.changing.is_some() { "Save" } else { "Add" };
                    declare = ui.add_enabled(ready, egui::Button::new(label)).clicked();
                    cancel = form.changing.is_some() && ui.button("Cancel").clicked();
                });
            });
        // A closed form shows no level, and the device it listened to is let go.
        if shown.body_returned.is_none() || declare || cancel {
            self.probing = None;
        }
        if test {
            let form = &mut self.forms.connection;
            form.tested = Some(match serde_json::from_str::<Json>(&form.settings) {
                Ok(Json::Object(mut settings)) => {
                    // Typed secrets are tried as they are, before they are stored.
                    let mut with = Json::Object(std::mem::take(&mut settings));
                    for (setting, secret) in &form.secrets.0 {
                        if !secret.is_empty() {
                            set_text(&mut with, setting, Some(Json::from(secret.as_str())));
                        }
                    }
                    match with {
                        Json::Object(settings) => host.test_connection(&form.adapter, &settings),
                        _ => Err("the settings are not an object".to_owned()),
                    }
                }
                _ => Err("the settings are not a JSON object".to_owned()),
            });
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
                    let secrets: Vec<(String, String)> = form
                        .secrets
                        .0
                        .into_iter()
                        .filter(|(_, secret)| !secret.is_empty())
                        .collect();
                    if self
                        .declare_connection_with_secrets(host, &connection, &secrets)
                        .is_some()
                    {
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

/// A frameless field that reads like a label until it is clicked.
fn edit_like<'a>(text: &'a mut String, hint: &str) -> egui::TextEdit<'a> {
    egui::TextEdit::singleline(text)
        .hint_text(hint)
        .frame(egui::Frame::NONE)
        .desired_width(110.0)
}

/// The list of adapters to make a connection of, by name, with each one's purpose.
fn adapter_picker(ui: &mut egui::Ui, adapters: &[AdapterInfo], form: &mut ConnectionForm) {
    let current = adapters.iter().find(|known| known.id == form.adapter);
    let shown = current.map_or(form.adapter.as_str(), |known| known.label.as_str());
    let before = form.adapter.clone();
    let _ = egui::ComboBox::from_id_salt("orange-connection-adapter")
        .selected_text(shown)
        .width(ui.available_width() - 8.0)
        .show_ui(ui, |ui| {
            for adapter in adapters {
                let hover = if adapter.purpose.is_empty() {
                    adapter.id.clone()
                } else {
                    format!("{}\n{}", adapter.purpose, adapter.id)
                };
                let _ = ui
                    .selectable_value(
                        &mut form.adapter,
                        adapter.id.clone(),
                        adapter.label.as_str(),
                    )
                    .on_hover_text(hover);
            }
        });
    if form.adapter != before {
        // Another kind has other settings: start from none.
        form.settings = "{}".to_owned();
        form.secrets = Typed::default();
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
fn text_fields(ui: &mut egui::Ui, texts: &[SettingText], settings: &mut Json, secrets: &mut Typed) {
    {
        {
            for text in texts {
                let label = if text.required {
                    format!("{} *", text.label)
                } else {
                    text.label.clone()
                };
                let _ = ui.small(label).on_hover_text(text.help.as_str());
                let current = text_at(settings, &text.setting).cloned();
                if text.secret {
                    let stored = current
                        .as_ref()
                        .and_then(Json::as_str)
                        .filter(|value| value.starts_with(SECRET_PREFIX))
                        .map(str::to_owned);
                    match (stored, secrets.0.get_mut(&text.setting)) {
                        (Some(reference), None) => {
                            ui.horizontal(|ui| {
                                let _ = ui
                                    .weak("Saved in the credential store")
                                    .on_hover_text(reference.as_str());
                                if ui.small_button("Replace").clicked() {
                                    let _ = secrets.0.insert(text.setting.clone(), String::new());
                                }
                                if ui
                                    .small_button("×")
                                    .on_hover_text("Leave it out.")
                                    .clicked()
                                {
                                    set_text(settings, &text.setting, None);
                                }
                            });
                        }
                        (_, typed) => {
                            let mut secret = typed.cloned().unwrap_or_default();
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut secret)
                                    .password(true)
                                    .hint_text("kept in the credential store")
                                    .desired_width(f32::INFINITY),
                            );
                            if response.changed() {
                                let _ = secrets.0.insert(text.setting.clone(), secret);
                            }
                        }
                    }
                } else if text.list {
                    let mut lines = current
                        .as_ref()
                        .and_then(Json::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(Json::as_str)
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default();
                    let response = ui.add(
                        egui::TextEdit::multiline(&mut lines)
                            .hint_text(text.hint.as_str())
                            .desired_rows(2)
                            .desired_width(f32::INFINITY),
                    );
                    if response.changed() {
                        let items: Vec<Json> = lines
                            .lines()
                            .map(str::trim)
                            .filter(|line| !line.is_empty())
                            .map(Json::from)
                            .collect();
                        let value = (!items.is_empty()).then_some(Json::Array(items));
                        set_text(settings, &text.setting, value);
                    }
                } else {
                    let mut value = current
                        .as_ref()
                        .and_then(Json::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut value)
                            .hint_text(text.hint.as_str())
                            .desired_width(f32::INFINITY),
                    );
                    if response.changed() {
                        let value = (!value.is_empty()).then(|| Json::from(value));
                        set_text(settings, &text.setting, value);
                    }
                }
                ui.add_space(2.0);
            }
        }
    }
}

/// Switches for an adapter's true-or-false settings. One switched back to its default
/// is left out.
fn toggle_fields(ui: &mut egui::Ui, toggles: &[SettingToggle], settings: &mut Json) {
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
            ui.button("+").on_hover_text(add).clicked()
        })
        .inner
    })
    .inner
}

/// One declaration in a list: its name, with edit and delete
/// buttons at the right end. `detail` is shown on hover. Returns whether each was pressed.
fn declaration_row(ui: &mut egui::Ui, name: &str, detail: &str) -> (bool, bool) {
    ui.horizontal(|ui| {
        let _ = ui.strong(name).on_hover_text(detail);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let delete = ui.small_button("Delete").clicked();
            let edit = ui.small_button("Edit").clicked();
            (edit, delete)
        })
        .inner
    })
    .inner
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
            .0
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
        assert_eq!(store.0.lock().unwrap().len(), 1);
        draw(&mut editor, &mut host);
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
