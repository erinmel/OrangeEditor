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
        self.0.get(action).and_then(Option::as_ref).map_or_else(
            || t("unassigned").to_owned(),
            |b| {
                format!(
                    "{}{}{}{}{}",
                    if b.modifiers.command {
                        if cfg!(target_os = "macos") {
                            "Cmd+"
                        } else {
                            "Ctrl+"
                        }
                    } else {
                        ""
                    },
                    if b.modifiers.ctrl && !b.modifiers.command {
                        "Ctrl+"
                    } else {
                        ""
                    },
                    if b.modifiers.alt { "Alt+" } else { "" },
                    if b.modifiers.shift { "Shift+" } else { "" },
                    b.logical_key.name()
                )
            },
        )
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
                    Some(KeyboardShortcut::new(
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
              "previews":self.palette_mode == crate::palette::PaletteMode::Previews,
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
        self.palette_mode = if value["previews"].as_bool().unwrap_or(false) {
            crate::palette::PaletteMode::Previews
        } else {
            crate::palette::PaletteMode::Names
        };
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
    pub(crate) fn keybindings_ui(&mut self, ui: &mut egui::Ui) {
        if let Some(action) = self.key_capture
            && ui.input(|input| input.focused)
        {
            let pressed = ui.input(|i| {
                i.events.iter().find_map(|e| {
                    if let egui::Event::Key {
                        key,
                        modifiers,
                        pressed: true,
                        repeat: false,
                        ..
                    } = e
                    {
                        Some((*key, *modifiers))
                    } else {
                        None
                    }
                })
            });
            if let Some((key, mut modifiers)) = pressed {
                if key == Key::Escape {
                    self.key_capture = None;
                } else {
                    if modifiers.command && !modifiers.mac_cmd {
                        modifiers.ctrl = false;
                    }
                    let binding = KeyboardShortcut::new(modifiers, key);
                    if let Some((other, _)) = self
                        .canvas
                        .keys
                        .0
                        .iter()
                        .find(|(id, b)| id.as_str() != action && **b == Some(binding))
                    {
                        self.key_conflict = Some(tf(
                            "already-assigned",
                            &[("action", action_label(other).into())],
                        ));
                    } else {
                        self.canvas.keys.0.insert(action.into(), Some(binding));
                        self.key_capture = None;
                        self.key_conflict = None;
                    }
                }
            }
        }
        ui.label(t("shortcut-edit-hint"));
        if let Some(error) = &self.key_conflict {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        egui::Grid::new("editable-shortcuts")
            .num_columns(3)
            .spacing(egui::vec2(16.0, 8.0))
            .show(ui, |ui| {
                let query = self.settings_search.to_lowercase();
                for &(id, label, _, _) in ACTIONS {
                    let label = t(label);
                    let keys = self.canvas.keys.label(id);
                    if !query.is_empty()
                        && !label.to_lowercase().contains(&query)
                        && !keys.to_lowercase().contains(&query)
                        && !["keyboard shortcuts", "keymap"]
                            .iter()
                            .any(|term| term.contains(&query))
                    {
                        continue;
                    }
                    ui.label(label);
                    if ui
                        .button(if self.key_capture == Some(id) {
                            t("press-keys").into()
                        } else {
                            keys
                        })
                        .clicked()
                    {
                        self.key_capture = Some(id);
                        self.key_conflict = None;
                    }
                    if ui
                        .small_button(crate::icons::text(crate::icons::CLOSE))
                        .on_hover_text(t("remove-shortcut"))
                        .clicked()
                    {
                        self.canvas.keys.0.insert(id.into(), None);
                    }
                    ui.end_row();
                }
            });
        if ui.button(t("restore-default-shortcuts")).clicked() {
            self.canvas.keys = Keymap::default();
            self.key_capture = None;
            self.key_conflict = None;
        }
    }
}
