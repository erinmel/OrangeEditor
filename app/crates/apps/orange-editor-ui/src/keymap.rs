//! Configurable shortcuts and presentation preferences.
#[cfg(test)]
mod tests;
use crate::i18n::{t, tf};
use crate::icons;
use egui::{Key, KeyboardShortcut, Modifiers};
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

fn shortcut_row(ui: &mut egui::Ui, width: f32, height: f32) -> (egui::Rect, [egui::Rect; 2]) {
    let (row, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let inner = row.shrink2(egui::vec2((width * 0.05).min(10.0), 0.0));
    let shortcut_width = (inner.width() * 0.42)
        .clamp(108.0, 180.0)
        .min(inner.width() * 0.52);
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
    pub(crate) fn keyboard(&mut self, ui: &egui::Ui, host: &mut orange_host::Host) {
        if self.settings_open || !ui.input(|i| i.focused) || ui.ctx().egui_wants_keyboard_input() {
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
              "scale":self.preferred_scale.unwrap_or(1.0),"tab":self.tab.key()})
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

    pub(crate) fn shortcut_matches_any(&self, query: &str) -> bool {
        let chord = parse_shortcut(query);
        actions().any(|(action, label, _, _)| self.shortcut_matches(action, t(label), query, chord))
    }

    fn shortcut_matches(
        &self,
        action: &str,
        label: &str,
        query: &str,
        chord: Option<KeyboardShortcut>,
    ) -> bool {
        label.to_lowercase().contains(query)
            || self
                .canvas
                .keys
                .label(action)
                .to_lowercase()
                .contains(query)
            || chord.is_some() && self.canvas.keys.0.get(action).copied().flatten() == chord
    }

    pub(crate) fn keybindings_ui(&mut self, ui: &mut egui::Ui, query: &str) {
        let reset = |ui: &mut egui::Ui| {
            ui.add(
                egui::Button::new((icons::text(icons::REFRESH), t("restore-default-shortcuts")))
                    .fill(ui.visuals().faint_bg_color),
            )
            .clicked()
        };
        let reset_requested = if ui.available_width() >= 510.0 {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let requested = reset(ui);
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.weak(t("settings-shortcuts-help"));
                    });
                    requested
                })
                .inner
            })
            .inner
        } else {
            ui.weak(t("settings-shortcuts-help"));
            reset(ui)
        };
        if reset_requested {
            self.canvas.keys = Keymap::default();
            self.shortcut_edit = None;
        }
        ui.add_space(10.0);
        if self.shortcut_edit.is_some() {
            self.edit_shortcut_ui(ui);
            ui.add_space(8.0);
        }
        let searched_chord = parse_shortcut(query);
        let table_width =
            (ui.available_width() - ui.spacing().scroll.allocated_width() - 6.0).max(0.0);
        let (header, [action, binding]) = shortcut_row(ui, table_width, 30.0);
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
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut any = false;
                for &(group, actions) in ACTION_GROUPS {
                    let mut headed = false;
                    for &(action, label, _, _) in actions {
                        let keys = self.canvas.keys.label(action);
                        let label = t(label);
                        if !self.shortcut_matches(action, label, query, searched_chord) {
                            continue;
                        }
                        any = true;
                        if !headed {
                            ui.add_space(8.0);
                            let (_, [heading, _]) = shortcut_row(ui, table_width, 25.0);
                            put_left(
                                ui,
                                heading,
                                egui::Label::new(egui::RichText::new(t(group)).strong()),
                            );
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
        let (row, [action_cell, binding_cell]) = shortcut_row(ui, width, 35.0);
        let current = self.canvas.keys.0.get(action).copied().flatten();
        let editing = self
            .shortcut_edit
            .as_ref()
            .is_some_and(|edit| edit.action == action);
        let visuals = ui.visuals();
        if editing {
            ui.painter().rect_filled(row, 4.0, visuals.faint_bg_color);
        }
        let divider = visuals.widgets.noninteractive.bg_stroke;
        ui.painter().hline(row.x_range(), row.bottom(), divider);
        let action_label = put_left(ui, action_cell, egui::Label::new(label).truncate());
        let shortcut = ui
            .put(
                binding_cell.shrink2(egui::vec2(0.0, 4.0)),
                egui::Button::new(egui::RichText::new(&keys).monospace())
                    .shortcut_text(icons::text(icons::EDIT))
                    .fill(if editing {
                        ui.visuals().selection.bg_fill
                    } else {
                        ui.visuals().extreme_bg_color
                    })
                    .stroke(divider)
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
                self.begin_shortcut_edit(action, original(action));
                ui.close();
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
            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
            .inner_margin(12)
            .corner_radius(6)
            .show(ui, |ui| {
                ui.strong(action_label(edit.action));
                let mut enter = false;
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
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
                            })
                            .fill(ui.visuals().selection.bg_fill),
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
