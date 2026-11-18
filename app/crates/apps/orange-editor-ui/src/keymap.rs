//! Configurable shortcuts and presentation preferences.
#[cfg(test)]
mod tests;
use crate::i18n::{t, tf};
use egui::{Key, KeyboardShortcut, Modifiers};
use std::collections::BTreeMap;

/// Each action: its ID, the message naming it, its default key and whether that key
/// takes the command modifier.
pub(crate) const ACTIONS: &[(&str, &str, Key, bool)] = &[
    ("save", "action-save", Key::S, true),
    ("save-as", "action-save-as", Key::S, true),
    ("new", "action-new", Key::N, true),
    ("open", "action-open", Key::O, true),
    ("undo", "action-undo", Key::Z, true),
    ("redo", "action-redo", Key::Y, true),
    ("copy", "action-copy", Key::C, true),
    ("cut", "action-cut", Key::X, true),
    ("paste", "action-paste", Key::V, true),
    ("duplicate", "action-duplicate", Key::D, true),
    ("arrange", "action-arrange", Key::L, true),
    ("function", "action-function", Key::E, true),
    ("group", "action-group", Key::G, true),
    ("select-all", "action-select-all", Key::A, true),
    ("fit", "action-fit", Key::Home, false),
    ("delete", "action-delete", Key::Delete, false),
    ("add", "action-add", Key::Tab, false),
    ("run", "action-run", Key::F5, false),
    ("stop", "action-stop", Key::F6, false),
    ("settings", "action-settings", Key::Comma, true),
];

/// The name of `action` in the current language, or its ID when there is none.
pub(crate) fn action_label(action: &str) -> String {
    ACTIONS
        .iter()
        .find(|(id, _, _, _)| *id == action)
        .map_or_else(|| action.to_owned(), |(_, label, _, _)| t(label).to_owned())
}

fn original(action: &str) -> Option<KeyboardShortcut> {
    ACTIONS
        .iter()
        .find(|(id, _, _, _)| *id == action)
        .map(|(id, _, key, command)| {
            let mut modifiers = if *command {
                Modifiers::COMMAND
            } else {
                Modifiers::NONE
            };
            modifiers.shift = *id == "save-as";
            KeyboardShortcut::new(modifiers, *key)
        })
}

fn normalized(modifiers: Modifiers, key: Key) -> KeyboardShortcut {
    let mut modifiers = modifiers;
    if cfg!(target_os = "macos") {
        modifiers.command |= modifiers.mac_cmd;
        modifiers.mac_cmd = false;
    } else {
        modifiers.command |= modifiers.ctrl;
        modifiers.ctrl = false;
    }
    KeyboardShortcut::new(modifiers, key)
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
            "shift" => modifiers.shift = true,
            _ => return None,
        }
    }
    let key = key.trim();
    let key = Key::from_name(key).or_else(|| match key.to_ascii_lowercase().as_str() {
        "esc" => Some(Key::Escape),
        "return" => Some(Key::Enter),
        "del" => Some(Key::Delete),
        _ => Key::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.name().eq_ignore_ascii_case(key)),
    })?;
    Some(normalized(modifiers, key))
}

fn format_shortcut(binding: KeyboardShortcut) -> String {
    let modifiers = binding.modifiers;
    format!(
        "{}{}{}{}{}",
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
        binding.logical_key.name(),
    )
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
            ACTIONS
                .iter()
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
        let direct = input.modifiers.matches_logically(binding.modifiers)
            && input.key_pressed(binding.logical_key);
        if direct {
            return true;
        }
        if self.0.values().flatten().any(|other| {
            input.modifiers.matches_logically(other.modifiers)
                && input.key_pressed(other.logical_key)
        }) {
            return false;
        }
        Some(*binding) == original(action)
            && match action {
                "redo" => {
                    input
                        .modifiers
                        .matches_logically(Modifiers::COMMAND | Modifiers::SHIFT)
                        && input.key_pressed(Key::Z)
                }
                "delete" => input.modifiers.is_none() && input.key_pressed(Key::Backspace),
                "add" => input.modifiers.is_none() && input.key_pressed(Key::Space),
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
        let mut restored = self.0.clone();
        for &(action, _, _, _) in ACTIONS {
            let Some(value) = saved.get(action) else {
                continue;
            };
            if value.is_null() {
                restored.insert(action.into(), None);
            } else if let Some(key) = value["key"].as_str().and_then(Key::from_name) {
                let on = |id: &str| value[id].as_bool().unwrap_or(false);
                restored.insert(
                    action.into(),
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
        // Reject an ambiguous saved map as a whole.
        let bindings: Vec<_> = restored.values().flatten().collect();
        if !bindings
            .iter()
            .enumerate()
            .any(|(at, b)| bindings[..at].contains(b))
        {
            self.0 = restored;
        }
    }
}
impl crate::Editor {
    pub(crate) fn keyboard(&mut self, ui: &egui::Ui, host: &mut orange_host::Host) {
        if self.settings_open || !ui.input(|i| i.focused) || ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let actions: Vec<_> = ui.input(|i| {
            ACTIONS
                .iter()
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
              "scroll_pans":self.canvas.scroll_pans,"language":self.language,
              "scale":self.preferred_scale.unwrap_or(1.0),"tab":self.tab.key()})
    }
    /// Restores supported preferences while preserving defaults for absent settings.
    pub fn restore_preferences(&mut self, value: &serde_json::Value) {
        if value["version"].as_u64() != Some(1) {
            return;
        }
        self.canvas.keys.restore(&value["keybindings"]);
        self.canvas.snap = value["snap"].as_bool().unwrap_or(true);
        self.canvas.scroll_pans = value["scroll_pans"].as_bool().unwrap_or(false);
        self.preferred_scale = value["scale"]
            .as_f64()
            .filter(|scale| scale.is_finite() && (0.75..=2.0).contains(scale))
            .map(|scale| scale as f32);
        self.scale_applied = false;
        if let Some(tab) = value["tab"].as_str().and_then(crate::Tab::from_key) {
            self.tab = tab;
        }
        if let Some(language) = value["language"].as_str() {
            self.language = language.into();
        }
    }
    fn begin_shortcut_edit(&mut self, action: &'static str, binding: Option<KeyboardShortcut>) {
        self.shortcut_edit = Some(ShortcutEdit {
            action,
            text: binding.map_or_else(String::new, format_shortcut),
            recording: false,
            focus: true,
        });
    }

    pub(crate) fn keybindings_ui(&mut self, ui: &mut egui::Ui, settings_query: &str) {
        let (toolbar, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::hover());
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("keymap-toolbar")
                .max_rect(toolbar)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                if ui.button(t("restore-default-shortcuts")).clicked() {
                    self.canvas.keys = Keymap::default();
                    self.shortcut_edit = None;
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.shortcut_search)
                        .hint_text(t("shortcut-search"))
                        .desired_width(ui.available_width()),
                );
            },
        );
        if self.shortcut_edit.is_some() {
            self.edit_shortcut_ui(ui);
            ui.add_space(8.0);
        }
        let query = if self.shortcut_search.trim().is_empty() {
            settings_query
        } else {
            &self.shortcut_search
        };
        let query = query.to_lowercase();
        let searched_chord = parse_shortcut(&query);
        let shortcut_width = (ui.available_width() * 0.38).clamp(118.0, 220.0);
        let command_width = (ui.available_width() - shortcut_width - 28.0).max(84.0);
        ui.horizontal(|ui| {
            ui.add_sized(
                [command_width, 24.0],
                egui::Label::new(egui::RichText::new(t("shortcut-action")).strong()).truncate(),
            );
            ui.add_sized(
                [shortcut_width, 24.0],
                egui::Label::new(egui::RichText::new(t("shortcut-binding")).strong()).truncate(),
            );
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("keymap-actions")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let groups = [
                    ("shortcut-files", 0..4),
                    ("shortcut-editing", 4..10),
                    ("shortcut-canvas", 10..17),
                    ("shortcut-run", 17..19),
                    ("shortcut-application", 19..ACTIONS.len()),
                ];
                let mut any = false;
                for (group, actions) in groups {
                    let mut headed = false;
                    for &(action, label, _, _) in &ACTIONS[actions] {
                        let current = self.canvas.keys.0.get(action).copied().flatten();
                        let keys = self.canvas.keys.label(action);
                        let label = t(label);
                        let matches_chord = searched_chord.is_some() && current == searched_chord;
                        if !label.to_lowercase().contains(&query)
                            && !keys.to_lowercase().contains(&query)
                            && !matches_chord
                        {
                            continue;
                        }
                        any = true;
                        if !headed {
                            ui.add_space(8.0);
                            ui.strong(t(group));
                            headed = true;
                        }
                        ui.horizontal(|ui| {
                            ui.add_sized([command_width, 28.0], egui::Label::new(label).truncate());
                            let selected = self
                                .shortcut_edit
                                .as_ref()
                                .is_some_and(|edit| edit.action == action);
                            let shortcut = ui
                                .add_sized(
                                    [shortcut_width, 28.0],
                                    egui::Button::new(
                                        egui::RichText::new(keys.clone()).monospace(),
                                    )
                                    .selected(selected)
                                    .truncate(),
                                )
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
                                    .add_enabled(
                                        current.is_some(),
                                        egui::Button::new(t("remove-shortcut")),
                                    )
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
                                    self.begin_shortcut_edit(action, original(action));
                                    ui.close();
                                }
                            });
                        });
                    }
                }
                if !any {
                    ui.weak(t("shortcut-no-results"));
                }
            });
    }

    fn edit_shortcut_ui(&mut self, ui: &mut egui::Ui) {
        let Some(mut edit) = self.shortcut_edit.take() else {
            return;
        };
        if edit.recording && ui.input(|input| input.focused) {
            let pressed = ui.input(|input| {
                input.events.iter().find_map(|event| match event {
                    egui::Event::Key {
                        key,
                        modifiers,
                        pressed: true,
                        repeat: false,
                        ..
                    } => Some((*key, *modifiers)),
                    _ => None,
                })
            });
            if let Some((key, modifiers)) = pressed {
                ui.input_mut(|input| input.consume_key(modifiers, key));
                edit.recording = false;
                if key != Key::Escape {
                    edit.text = format_shortcut(normalized(modifiers, key));
                }
                edit.focus = true;
            }
        } else if ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
            return;
        }
        let mut keep_editing = true;
        egui::Frame::new()
            .fill(ui.visuals().faint_bg_color)
            .inner_margin(10)
            .corner_radius(6)
            .show(ui, |ui| {
                ui.strong(action_label(edit.action));
                let mut enter = false;
                ui.horizontal(|ui| {
                    let mut input = egui::TextEdit::singleline(&mut edit.text)
                        .font(egui::TextStyle::Monospace)
                        .desired_width((ui.available_width() - 135.0).max(90.0))
                        .interactive(!edit.recording)
                        .hint_text(t("shortcut-example"))
                        .show(ui);
                    if std::mem::take(&mut edit.focus) {
                        use egui::text::{CCursor, CCursorRange};
                        input.response.request_focus();
                        input.state.cursor.set_char_range(Some(CCursorRange::two(
                            CCursor::new(0),
                            CCursor::new(edit.text.chars().count()),
                        )));
                        input.state.store(ui.ctx(), input.response.id);
                    }
                    enter = input.response.lost_focus()
                        && ui.input(|state| state.key_pressed(Key::Enter));
                    if ui
                        .add_enabled(
                            !edit.recording,
                            egui::Button::new(if edit.recording {
                                t("press-keys")
                            } else {
                                t("shortcut-record")
                            }),
                        )
                        .clicked()
                    {
                        edit.recording = true;
                    }
                });
                let binding = parse_shortcut(&edit.text);
                let conflict = binding
                    .and_then(|binding| self.canvas.keys.conflict(edit.action, binding))
                    .map(str::to_owned);
                if let Some(other) = &conflict {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        tf(
                            "already-assigned",
                            &[("action", action_label(other).into())],
                        ),
                    );
                } else if binding.is_none() && !edit.text.trim().is_empty() {
                    ui.colored_label(ui.visuals().error_fg_color, t("shortcut-invalid"));
                }
                ui.horizontal_wrapped(|ui| {
                    let apply = ui
                        .add_enabled(
                            binding.is_some() && !edit.recording,
                            egui::Button::new(if conflict.is_some() {
                                t("shortcut-reassign")
                            } else {
                                t("shortcut-save")
                            }),
                        )
                        .clicked();
                    if (apply || enter && conflict.is_none())
                        && let Some(binding) = binding
                    {
                        if let Some(other) = conflict {
                            self.canvas.keys.0.insert(other, None);
                        }
                        self.canvas
                            .keys
                            .0
                            .insert(edit.action.to_owned(), Some(binding));
                        keep_editing = false;
                    }
                    if ui.button(t("cancel")).clicked() {
                        keep_editing = false;
                    }
                    if ui.button(t("remove-shortcut")).clicked() {
                        self.canvas.keys.0.insert(edit.action.to_owned(), None);
                        keep_editing = false;
                    }
                });
            });
        if keep_editing {
            self.shortcut_edit = Some(edit);
        }
    }
}
