//! Configurable shortcuts and presentation preferences.
#[cfg(test)]
mod tests;
use crate::controls::{self, Action as ButtonRole, Surface};
use crate::i18n::{t, tf};
use crate::icons;
use egui::{Key, KeyboardShortcut, Modifiers};
use orange_ui_controls::{CONTROL_HEIGHT, ControlSize, SPACE_SMALL, Theme};
use std::collections::BTreeMap;

type Action = (&'static str, &'static str, Key, Modifiers);
const CMD: Modifiers = Modifiers::COMMAND;
const CMD_SHIFT: Modifiers = CMD.plus(Modifiers::SHIFT);
const BARE: Modifiers = Modifiers::NONE;

const ACTION_GROUPS: &[(&str, &[Action])] = &[
    (
        "shortcut-files",
        &[
            ("save", "action-save", Key::S, CMD),
            ("save-as", "action-save-as", Key::S, CMD_SHIFT),
            ("new", "action-new", Key::N, CMD),
            ("open", "action-open", Key::O, CMD),
        ],
    ),
    (
        "shortcut-editing",
        &[
            ("undo", "action-undo", Key::Z, CMD),
            ("redo", "action-redo", Key::Y, CMD),
            ("copy", "action-copy", Key::C, CMD),
            ("cut", "action-cut", Key::X, CMD),
            ("paste", "action-paste", Key::V, CMD),
            ("duplicate", "action-duplicate", Key::D, CMD),
        ],
    ),
    (
        "shortcut-canvas",
        &[
            ("arrange", "action-arrange", Key::L, CMD),
            ("function", "action-function", Key::E, CMD),
            ("group", "action-group", Key::G, CMD),
            ("ungroup", "action-ungroup", Key::G, CMD_SHIFT),
            ("snap", "action-snap", Key::G, Modifiers::ALT),
            (
                "failure-routes",
                "action-failure-routes",
                Key::O,
                Modifiers::ALT,
            ),
            ("select-all", "action-select-all", Key::A, CMD),
            ("fit", "action-fit", Key::Home, BARE),
            ("delete", "action-delete", Key::Delete, BARE),
            ("add", "action-add", Key::Tab, BARE),
        ],
    ),
    (
        "shortcut-run",
        &[
            ("run", "action-run", Key::F5, BARE),
            ("stop", "action-stop", Key::F6, BARE),
        ],
    ),
    (
        "shortcut-application",
        &[
            ("settings", "action-settings", Key::Comma, CMD),
            ("left-panel", "action-left-panel", Key::Num1, CMD),
            ("right-panel", "action-right-panel", Key::Num2, CMD),
        ],
    ),
];

fn group_help(group: &str) -> &'static str {
    match group {
        "shortcut-files" => "shortcut-files-help",
        "shortcut-editing" => "shortcut-editing-help",
        "shortcut-canvas" => "shortcut-canvas-help",
        "shortcut-run" => "shortcut-run-help",
        _ => "shortcut-application-help",
    }
}

fn actions() -> impl Iterator<Item = &'static Action> {
    ACTION_GROUPS.iter().flat_map(|(_, group)| group.iter())
}

/// The name of `action` in the current language, or its ID when there is none.
pub(crate) fn action_label(action: &str) -> String {
    actions()
        .find(|(id, _, _, _)| *id == action)
        .map_or_else(|| action.to_owned(), |(_, label, _, _)| t(label).to_owned())
}

fn original(action: &str) -> Option<KeyboardShortcut> {
    actions()
        .find(|(id, _, _, _)| *id == action)
        .map(|(_, _, key, modifiers)| KeyboardShortcut::new(*modifiers, *key))
}

fn normalized_modifiers(mut modifiers: Modifiers) -> Modifiers {
    if cfg!(target_os = "macos") {
        modifiers.command |= modifiers.mac_cmd;
        modifiers.mac_cmd = false;
    } else {
        modifiers.command |= modifiers.ctrl;
        modifiers.ctrl = false;
    }
    modifiers
}

fn normalized(modifiers: Modifiers, key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(normalized_modifiers(modifiers), key)
}

fn parse_shortcut(text: &str) -> Option<KeyboardShortcut> {
    let text = text.trim();
    let (prefix, key) = if text == "+" {
        ("", "Plus")
    } else if let Some(prefix) = text.strip_suffix("++") {
        (prefix, "Plus")
    } else {
        text.rsplit_once('+').unwrap_or(("", text))
    };
    let mut modifiers = Modifiers::NONE;
    for name in prefix.split('+').filter(|_| !prefix.is_empty()) {
        match name.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers.ctrl = true,
            "cmd" | "command" => modifiers.command = true,
            "alt" | "option" => modifiers.alt = true,
            "shift" | "mayús" | "mayus" => modifiers.shift = true,
            _ => return None,
        }
    }
    let key = key.trim();
    let key = Key::from_name(key).or_else(|| match key.to_ascii_lowercase().as_str() {
        "esc" => Some(Key::Escape),
        "return" | "intro" => Some(Key::Enter),
        "del" | "supr" => Some(Key::Delete),
        _ => Key::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.name().eq_ignore_ascii_case(key)),
    })?;
    Some(normalized(modifiers, key))
}

fn modifier_prefix(modifiers: Modifiers) -> String {
    format!(
        "{}{}{}{}",
        if modifiers.command || modifiers.mac_cmd {
            if cfg!(target_os = "macos") {
                "Cmd+"
            } else {
                "Ctrl+"
            }
        } else {
            ""
        },
        if modifiers.ctrl && cfg!(target_os = "macos") {
            "Ctrl+"
        } else {
            ""
        },
        if modifiers.alt { "Alt+" } else { "" },
        if modifiers.shift { "Shift+" } else { "" },
    )
}

fn format_shortcut(binding: KeyboardShortcut) -> String {
    modifier_prefix(binding.modifiers) + binding.logical_key.name()
}

fn is_modifier_key(key: Key) -> bool {
    matches!(
        key.name().to_ascii_lowercase().as_str(),
        "alt"
            | "altleft"
            | "altright"
            | "control"
            | "controlleft"
            | "controlright"
            | "shift"
            | "shiftleft"
            | "shiftright"
            | "meta"
            | "metaleft"
            | "metaright"
            | "super"
            | "superleft"
            | "superright"
    )
}

fn shortcut_row(ui: &mut egui::Ui, width: f32, height: f32) -> (egui::Rect, [egui::Rect; 2]) {
    let (row, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let inner = row.shrink2(egui::vec2((width * 0.05).min(10.0), 0.0));
    let shortcut_width = (inner.width() * 0.48)
        .clamp(142.0, 252.0)
        .min(inner.width() * 0.64);
    let binding = egui::Rect::from_min_max(
        egui::pos2(inner.right() - shortcut_width, inner.top()),
        inner.max,
    );
    let action = egui::Rect::from_min_max(
        inner.min,
        egui::pos2((binding.left() - 10.0).max(inner.left()), inner.bottom()),
    );
    (row, [action, binding])
}

#[derive(Debug)]
pub(crate) struct ShortcutEdit {
    action: &'static str,
    text: String,
    recording: bool,
    focus: bool,
}
#[derive(Debug, Clone)]
pub(crate) struct Keymap(pub BTreeMap<String, Option<KeyboardShortcut>>);
impl Default for Keymap {
    fn default() -> Self {
        Self(
            actions()
                .map(|(id, _, _, _)| ((*id).to_owned(), original(id)))
                .collect(),
        )
    }
}
impl Keymap {
    pub fn pressed(&self, input: &egui::InputState, action: &str) -> bool {
        let Some(Some(binding)) = self.0.get(action) else {
            return false;
        };
        // Extra Shift or Alt can be part of typing a logical key. When that key
        // also has an exact chord, only the exact command should run.
        let modifiers = input.modifiers;
        let direct = input.key_pressed(binding.logical_key)
            && (modifiers.matches_exact(binding.modifiers)
                || modifiers.matches_logically(binding.modifiers)
                    && !self.0.values().flatten().any(|other| {
                        other.logical_key == binding.logical_key
                            && modifiers.matches_exact(other.modifiers)
                    }));
        if direct {
            return true;
        }
        if self.0.values().flatten().any(|other| {
            input.modifiers.matches_logically(other.modifiers)
                && input.key_pressed(other.logical_key)
        }) {
            return false;
        }
        if Some(*binding) != original(action) {
            return false;
        }
        let unclaimed_key = |key| {
            input.key_pressed(key)
                && !self.0.values().flatten().any(|other| {
                    other.logical_key == key && modifiers.matches_exact(other.modifiers)
                })
        };
        match action {
            "redo" => {
                modifiers.matches_logically(Modifiers::COMMAND | Modifiers::SHIFT)
                    && unclaimed_key(Key::Z)
            }
            "copy" => {
                unclaimed_key(Key::Copy)
                    || cfg!(target_os = "windows") && modifiers.ctrl && unclaimed_key(Key::Insert)
            }
            "cut" => {
                unclaimed_key(Key::Cut)
                    || cfg!(target_os = "windows") && modifiers.shift && unclaimed_key(Key::Delete)
            }
            "paste" => {
                unclaimed_key(Key::Paste)
                    || cfg!(target_os = "windows") && modifiers.shift && unclaimed_key(Key::Insert)
            }
            "delete" => modifiers.is_none() && unclaimed_key(Key::Backspace),
            "add" => modifiers.is_none() && unclaimed_key(Key::Space),
            _ => false,
        }
    }
    pub fn label(&self, action: &str) -> String {
        self.0
            .get(action)
            .copied()
            .flatten()
            .map_or_else(|| t("unassigned").to_owned(), format_shortcut)
    }

    fn conflict(&self, action: &str, binding: KeyboardShortcut) -> Option<&str> {
        self.0.iter().find_map(|(id, candidate)| {
            (id != action
                && candidate
                    .is_some_and(|other| normalized(other.modifiers, other.logical_key) == binding))
            .then_some(id.as_str())
        })
    }
    fn json(&self) -> serde_json::Value {
        serde_json::Value::Object(
            self.0
                .iter()
                .map(|(id, b)| {
                    (
                        id.clone(),
                        b.map_or(serde_json::Value::Null, |b| {
                            serde_json::json!({
                "key":b.logical_key.name(),"command":b.modifiers.command,"shift":b.modifiers.shift,
                "alt":b.modifiers.alt,"ctrl":b.modifiers.ctrl,"mac_cmd":b.modifiers.mac_cmd})
                        }),
                    )
                })
                .collect(),
        )
    }
    fn restore(&mut self, saved: &serde_json::Value) {
        let mut explicit = BTreeMap::new();
        for &(action, _, _, _) in actions() {
            let Some(value) = saved.get(action) else {
                continue;
            };
            if value.is_null() {
                explicit.insert(action.to_owned(), None);
            } else if let Some(key) = value["key"].as_str().and_then(Key::from_name) {
                let on = |id: &str| value[id].as_bool().unwrap_or(false);
                explicit.insert(
                    action.to_owned(),
                    Some(normalized(
                        Modifiers {
                            command: on("command"),
                            shift: on("shift"),
                            alt: on("alt"),
                            ctrl: on("ctrl"),
                            mac_cmd: on("mac_cmd"),
                        },
                        key,
                    )),
                );
            }
        }
        // Preserve explicit bindings from older preferences when a new default takes
        // the same chord. Reject a map that explicitly assigns one chord twice.
        let bindings: Vec<_> = explicit.values().flatten().copied().collect();
        if bindings
            .iter()
            .enumerate()
            .any(|(at, b)| bindings[..at].contains(b))
        {
            return;
        }
        for (action, binding) in &mut self.0 {
            if !explicit.contains_key(action) && binding.is_some_and(|key| bindings.contains(&key))
            {
                *binding = None;
            }
        }
        self.0.extend(explicit);
    }
}
impl crate::Editor {
    /// Whether the native host should deliver raw keys to the shortcut recorder.
    pub fn is_recording_shortcut(&self) -> bool {
        self.settings_open
            && self
                .shortcut_edit
                .as_ref()
                .is_some_and(|edit| edit.recording)
    }

    pub(crate) fn keyboard(&mut self, ui: &egui::Ui, host: &mut orange_host::Host) {
        if self.settings_open
            || self.has_node_ergonomics_modal()
            || self.value_form.is_some()
            || !ui.input(|i| i.focused)
            || ui.ctx().egui_wants_keyboard_input()
        {
            return;
        }
        let actions: Vec<_> = ui.input(|i| {
            actions()
                .filter(|(id, _, _, _)| self.canvas.keys.pressed(i, id))
                .map(|(id, _, _, _)| *id)
                .collect()
        });
        for action in actions {
            match action {
                "save" => {
                    if let Err(error) = self.save() {
                        self.say(error);
                    }
                }
                "save-as" if self.file_dialogs => self.ask(crate::FileRequest::SaveAs),
                "new" if self.file_dialogs => self.ask(crate::FileRequest::New),
                "open" if self.file_dialogs => self.ask(crate::FileRequest::Open),
                "undo" => self.undo(host),
                "redo" => self.redo(host),
                "duplicate" => {
                    let _ = self.duplicate(host);
                }
                "arrange" => {
                    let _ = self.arrange_selection();
                }
                "function" if !self.canvas.selection().is_empty() => self.preview_function(host),
                "fit" => self.canvas.fit(),
                "snap" => self.canvas.snap = !self.canvas.snap,
                "failure-routes"
                    if self.quick.is_none()
                        && self.menu.is_none()
                        && self.node_config_menu.is_none() =>
                {
                    if let Some(node) = self.canvas.selected() {
                        self.canvas.toggle_failure_routes(node);
                    }
                }
                "left-panel" => self.toggle_left_panel(),
                "right-panel" => self.toggle_right_panel(),
                "run" => self.run_asked = true,
                "stop" => self.stop(host),
                "settings" => self.settings_open = true,
                _ => {}
            }
        }
    }
    /// Presentation settings, independent of graph semantics and undo.
    pub fn preferences(&self) -> serde_json::Value {
        serde_json::json!({"version":1,"keybindings":self.canvas.keys.json(),"snap":self.canvas.snap,
              "grid_step":self.canvas.grid_step,"grid_style":self.canvas.grid_style.key(),
              "scroll_pans":self.canvas.scroll_pans,"language":self.language,
              "scale":self.preferred_scale.unwrap_or(1.0),"tab":self.preferred_tab.key(),
              "panels":{"nodes":self.panels.nodes,"properties":self.panels.properties,
                        "packages":self.panels.packages,"log":self.panels.log},
              "appearance":self.preferred_theme.map(egui::ThemePreference::name),
              "node_search_previews":self.node_search_previews,
              "wireless_opacity":self.canvas.wireless_opacity})
    }
    /// Restores supported preferences while preserving defaults for absent settings.
    pub fn restore_preferences(&mut self, value: &serde_json::Value) {
        if value["version"].as_u64() != Some(1) {
            return;
        }
        self.canvas.keys.restore(&value["keybindings"]);
        self.canvas.snap = value["snap"].as_bool().unwrap_or(true);
        self.canvas.grid_step = value["grid_step"]
            .as_u64()
            .and_then(|size| u16::try_from(size).ok())
            .filter(|size| crate::canvas::GRID_STEPS.contains(size))
            .unwrap_or(crate::canvas::DEFAULT_GRID_STEP);
        self.canvas.grid_style = crate::canvas::GridStyle::from_key(value["grid_style"].as_str());
        self.canvas.scroll_pans = value["scroll_pans"].as_bool().unwrap_or(false);
        self.preferred_scale = value["scale"]
            .as_f64()
            .filter(|scale| scale.is_finite() && (0.75..=2.0).contains(scale))
            .map(|scale| scale as f32);
        self.scale_applied = false;
        self.preferred_theme = match value["appearance"].as_str() {
            Some("light") => Some(egui::ThemePreference::Light),
            Some("dark") => Some(egui::ThemePreference::Dark),
            Some("system") => Some(egui::ThemePreference::System),
            _ => None,
        };
        self.node_search_previews = value["node_search_previews"].as_bool().unwrap_or(false);
        self.canvas.wireless_opacity = value["wireless_opacity"]
            .as_u64()
            .and_then(|percent| u8::try_from(percent.min(100)).ok())
            .unwrap_or(crate::canvas::DEFAULT_WIRELESS_OPACITY);
        if let Some(tab) = value["tab"].as_str().and_then(crate::Tab::from_key) {
            self.tab = tab;
            self.preferred_tab = tab;
        }
        if let Some(panels) = value["panels"].as_object() {
            self.panels.nodes = panels
                .get("nodes")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(self.panels.nodes);
            self.panels.properties = panels
                .get("properties")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(self.panels.properties);
            self.panels.packages = panels
                .get("packages")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(self.panels.packages);
            self.panels.log = panels
                .get("log")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(self.panels.log);
        }
        if let Some(language) = value["language"].as_str() {
            self.language = language.into();
        }
    }
    fn begin_shortcut_edit(&mut self, action: &'static str, binding: Option<KeyboardShortcut>) {
        self.shortcut_edit = Some(ShortcutEdit {
            action,
            text: binding.map_or_else(String::new, format_shortcut),
            recording: true,
            focus: true,
        });
    }

    pub(crate) fn shortcut_matches_any(&self, query: &str) -> bool {
        let chord = parse_shortcut(query);
        ACTION_GROUPS.iter().any(|(group, actions)| {
            actions.iter().any(|(action, label, _, _)| {
                self.shortcut_matches(action, t(label), group, query, chord)
            })
        })
    }

    fn shortcut_matches(
        &self,
        action: &str,
        label: &str,
        group: &'static str,
        query: &str,
        chord: Option<KeyboardShortcut>,
    ) -> bool {
        if query.is_empty() {
            return true;
        }
        let terms = orange_host::fold_search(&format!(
            "{label} {} {} {}",
            t(group),
            t(group_help(group)),
            self.canvas.keys.label(action)
        ));
        query.split_whitespace().all(|word| terms.contains(word))
            || chord.is_some() && self.canvas.keys.0.get(action).copied().flatten() == chord
    }

    pub(crate) fn keybindings_ui(&mut self, ui: &mut egui::Ui, query: &str) {
        let reset_requested = ui
            .horizontal(|ui| {
                ui.label(
                    egui::RichText::new(t("shortcuts-heading"))
                        .size(16.0)
                        .strong(),
                )
                .on_hover_text(t("settings-shortcuts-help"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    controls::button(
                        ui,
                        Surface::Dialog,
                        ButtonRole::Outline,
                        true,
                        egui::Button::new(t("restore-default-shortcuts")).truncate(),
                    )
                    .clicked()
                })
                .inner
            })
            .inner;
        if reset_requested {
            self.canvas.keys = Keymap::default();
            self.shortcut_edit = None;
        }
        let searched_chord = parse_shortcut(query);
        let table_width =
            (ui.available_width() - ui.spacing().scroll.allocated_width() - 6.0).max(0.0);
        let (header, [action, binding]) = shortcut_row(ui, table_width, CONTROL_HEIGHT);
        ui.painter()
            .rect_filled(header, 5.0, ui.visuals().faint_bg_color);
        for (cell, label) in [(action, "shortcut-action"), (binding, "shortcut-binding")] {
            put_left(
                ui,
                cell,
                egui::Label::new(egui::RichText::new(t(label)).strong()),
            );
        }
        egui::ScrollArea::vertical()
            .id_salt("keymap-actions")
            .animated(false)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut any = false;
                for &(group, actions) in ACTION_GROUPS {
                    let mut headed = false;
                    for &(action, label, _, _) in actions {
                        let keys = self.canvas.keys.label(action);
                        let label = t(label);
                        if !self
                            .shortcut_edit
                            .as_ref()
                            .is_some_and(|edit| edit.action == action)
                            && !self.shortcut_matches(action, label, group, query, searched_chord)
                        {
                            continue;
                        }
                        any = true;
                        if !headed {
                            ui.add_space(SPACE_SMALL);
                            let (_, [heading, _]) = shortcut_row(ui, table_width, 19.0);
                            put_left(
                                ui,
                                heading,
                                egui::Label::new(egui::RichText::new(t(group)).strong()),
                            )
                            .on_hover_text(t(group_help(group)));
                            headed = true;
                        }
                        self.shortcut_binding_row(ui, table_width, action, label, keys);
                    }
                }
                if !any {
                    ui.weak(t("shortcut-no-results"));
                }
            });
    }

    fn shortcut_binding_row(
        &mut self,
        ui: &mut egui::Ui,
        width: f32,
        action: &'static str,
        label: &str,
        keys: String,
    ) {
        let (row, [action_cell, binding_cell]) = shortcut_row(ui, width, CONTROL_HEIGHT + 2.0);
        let theme = Theme::of(ui);
        let action_label = put_left(
            ui,
            action_cell,
            egui::Label::new(egui::RichText::new(label).color(theme.text)).truncate(),
        );
        if self
            .shortcut_edit
            .as_ref()
            .is_some_and(|edit| edit.action == action)
        {
            self.edit_shortcut_ui(ui, row, binding_cell, action_label.id);
            return;
        }
        let current = self.canvas.keys.0.get(action).copied().flatten();
        let cell = binding_cell.shrink2(egui::vec2(0.0, 3.0));
        let mut binding_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("shortcut-binding", action))
                .max_rect(cell)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        binding_ui.spacing_mut().interact_size.y = cell.height();
        binding_ui.spacing_mut().button_padding.y = 2.0;
        let shortcut = controls::button(
            &mut binding_ui,
            Surface::Card,
            ButtonRole::Outline,
            true,
            egui::Button::new(())
                .left_text(egui::RichText::new(&keys).monospace())
                .shortcut_text(icons::text(icons::EDIT))
                .min_size(cell.size())
                .truncate(),
        )
        .labelled_by(action_label.id)
        .on_hover_text(format!("{}\n{keys}", t("shortcut-edit")));
        if shortcut.clicked() {
            self.begin_shortcut_edit(action, current);
        }
        shortcut.context_menu(|ui| {
            if ui.button(t("shortcut-edit")).clicked() {
                self.begin_shortcut_edit(action, current);
                ui.close();
            }
            if ui
                .add_enabled(current.is_some(), egui::Button::new(t("remove-shortcut")))
                .clicked()
            {
                self.canvas.keys.0.insert(action.to_owned(), None);
                self.shortcut_edit = None;
                ui.close();
            }
            if ui
                .add_enabled(
                    current != original(action),
                    egui::Button::new(t("shortcut-reset")),
                )
                .clicked()
            {
                if let Some(original) = original(action)
                    && self.canvas.keys.conflict(action, original).is_none()
                {
                    self.canvas.keys.0.insert(action.to_owned(), Some(original));
                    self.shortcut_edit = None;
                } else {
                    self.begin_shortcut_edit(action, original(action));
                    if let Some(edit) = &mut self.shortcut_edit {
                        edit.recording = false;
                    }
                }
                ui.close();
            }
        });
    }

    fn edit_shortcut_ui(
        &mut self,
        ui: &mut egui::Ui,
        row: egui::Rect,
        cell: egui::Rect,
        label: egui::Id,
    ) {
        let Some(mut edit) = self.shortcut_edit.take() else {
            return;
        };
        if edit.recording {
            let captured = ui.input_mut(|input| {
                if !input.focused {
                    return None;
                }
                let captured = input.events.iter().find_map(|event| match *event {
                    egui::Event::Key {
                        key,
                        modifiers,
                        pressed: true,
                        repeat: false,
                        ..
                    } if !is_modifier_key(key) => Some((key, modifiers)),
                    _ => None,
                });
                if let Some((key, modifiers)) = captured {
                    input.consume_key(modifiers, key);
                }
                input.events.retain(|event| {
                    !matches!(
                        event,
                        egui::Event::Text(_)
                            | egui::Event::Copy
                            | egui::Event::Cut
                            | egui::Event::Paste(_)
                            | egui::Event::PasteImage(_)
                    )
                });
                captured
            });
            if let Some((key, modifiers)) = captured {
                edit.recording = false;
                if key == Key::Escape && modifiers.is_none() {
                    return;
                }
                edit.text = format_shortcut(normalized(modifiers, key));
                edit.focus = true;
            }
        } else if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
            return;
        }
        let scroll_to_editor = edit.focus;
        if edit.recording {
            edit.focus = false;
        }
        let mut recording_label = if edit.recording {
            let modifiers = ui.input(|input| input.modifiers);
            modifier_prefix(normalized_modifiers(modifiers))
        } else {
            String::new()
        };
        let (enter, apply, cancel, remove) = {
            let mut editor_ui = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(("shortcut-binding", edit.action))
                    .max_rect(cell)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            let ui = &mut editor_ui;
            ui.spacing_mut().item_spacing.x = 2.0;
            ControlSize::Compact.apply(ui.spacing_mut());
            ui.spacing_mut().button_padding.x = 2.0;
            let shown = if edit.recording {
                &mut recording_label
            } else {
                &mut edit.text
            };
            let mut field = egui::TextEdit::singleline(shown)
                .id(ui.make_persistent_id(("shortcut-entry", edit.action)))
                .font(egui::TextStyle::Monospace)
                .align(egui::Align2::LEFT_CENTER)
                .min_size(egui::vec2(0.0, ControlSize::Compact.height()))
                .desired_width((cell.width() - 94.0).max(12.0))
                .margin(egui::Margin::symmetric(3, 1))
                .interactive(!edit.recording)
                .hint_text(if edit.recording {
                    t("press-keys")
                } else {
                    t("shortcut-example")
                })
                .show(ui);
            let response = field.response.response.labelled_by(label);
            if !edit.recording && std::mem::take(&mut edit.focus) {
                use egui::text::{CCursor, CCursorRange};
                response.request_focus();
                field.state.cursor.set_char_range(Some(CCursorRange::two(
                    CCursor::new(0),
                    CCursor::new(edit.text.chars().count()),
                )));
                field.state.store(ui.ctx(), response.id);
            }
            let enter = response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
            let binding = parse_shortcut(&edit.text);
            let can_apply = !edit.recording
                && binding.is_some_and(|binding| {
                    self.canvas.keys.conflict(edit.action, binding).is_none()
                });
            if shortcut_edit_button(
                ui,
                icons::KEYBOARD,
                t("shortcut-record"),
                !edit.recording,
                ButtonRole::Tertiary,
            ) {
                edit.recording = true;
            }
            let apply = shortcut_edit_button(
                ui,
                icons::CHECK,
                t("shortcut-save"),
                can_apply,
                ButtonRole::Primary,
            );
            let cancel =
                shortcut_edit_button(ui, icons::CLOSE, t("cancel"), true, ButtonRole::Tertiary);
            let remove = shortcut_edit_button(
                ui,
                icons::DELETE,
                t("remove-shortcut"),
                self.canvas
                    .keys
                    .0
                    .get(edit.action)
                    .is_some_and(Option::is_some),
                ButtonRole::Tertiary,
            );
            (enter, apply, cancel, remove)
        };
        let binding = parse_shortcut(&edit.text);
        let conflict = binding.and_then(|binding| self.canvas.keys.conflict(edit.action, binding));
        let mut reassign = false;
        if !edit.recording {
            if let Some(other) = conflict {
                ui.horizontal_wrapped(|ui| {
                    orange_ui_controls::notice(
                        ui,
                        controls::ButtonTone::Warning,
                        tf(
                            "already-assigned",
                            &[("action", action_label(other).into())],
                        ),
                    );
                    reassign = controls::button(
                        ui,
                        Surface::Dialog,
                        ButtonRole::Primary,
                        true,
                        egui::Button::new(t("shortcut-reassign")),
                    )
                    .clicked();
                });
            } else if binding.is_none() && !edit.text.trim().is_empty() {
                orange_ui_controls::notice(ui, controls::ButtonTone::Danger, t("shortcut-invalid"));
            }
        }
        if remove {
            self.canvas.keys.0.insert(edit.action.to_owned(), None);
        } else if !cancel {
            if (reassign || !edit.recording && conflict.is_none() && (apply || enter))
                && let Some(binding) = binding
            {
                if let Some(other) = conflict {
                    self.canvas.keys.0.insert(other.to_owned(), None);
                }
                self.canvas
                    .keys
                    .0
                    .insert(edit.action.to_owned(), Some(binding));
            } else {
                self.shortcut_edit = Some(edit);
            }
        }
        if scroll_to_editor {
            ui.scroll_to_rect(row, None);
        }
    }
}

fn shortcut_edit_button(
    ui: &mut egui::Ui,
    icon: char,
    label: &str,
    enabled: bool,
    role: ButtonRole,
) -> bool {
    let response = controls::button(
        ui,
        Surface::Dialog,
        role,
        enabled,
        egui::Button::new(icons::centered(icon)).min_size(egui::Vec2::splat(20.0)),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    response
        .on_hover_text(label)
        .on_disabled_hover_text(label)
        .clicked()
}

/// Adds `widget` at the left of `cell`, centred vertically: `Ui::put` would centre it
/// across the cell too.
fn put_left(ui: &mut egui::Ui, cell: egui::Rect, widget: impl egui::Widget) -> egui::Response {
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(cell)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| ui.add(widget),
    )
    .inner
}
