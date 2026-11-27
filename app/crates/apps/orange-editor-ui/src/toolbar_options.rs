//! Main toolbar actions and persistent customization.
use super::*;

const GROUP_GAP: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    New,
    Open,
    Save,
    SaveAs,
    Undo,
    Redo,
    Group,
    Ungroup,
    Arrange,
    Unlink,
    Fit,
    Snap,
    Grid,
    Check,
    Run,
}

impl Action {
    const ALL: [Self; 15] = [
        Self::New,
        Self::Open,
        Self::Save,
        Self::SaveAs,
        Self::Undo,
        Self::Redo,
        Self::Group,
        Self::Ungroup,
        Self::Arrange,
        Self::Unlink,
        Self::Fit,
        Self::Snap,
        Self::Grid,
        Self::Check,
        Self::Run,
    ];

    fn details(self) -> (&'static str, char, &'static str, &'static str) {
        match self {
            Self::New => ("new-graph", icons::NEW, "new", "file"),
            Self::Open => ("open-graph", icons::OPEN, "open", "file"),
            Self::Save => ("save", icons::SAVE, "save", "file"),
            Self::SaveAs => ("save-as", icons::SAVE_AS, "save-as", "file"),
            Self::Undo => ("undo", icons::UNDO, "undo", "edit"),
            Self::Redo => ("redo", icons::REDO, "redo", "edit"),
            Self::Group => ("group-selection", icons::GROUP, "group", "canvas"),
            Self::Ungroup => ("ungroup", icons::UNGROUP, "ungroup", "canvas"),
            Self::Arrange => ("arrange-selection", icons::ARRANGE, "arrange", "canvas"),
            Self::Unlink => ("unlink-wire", icons::DISCONNECT, "delete", "canvas"),
            Self::Fit => ("fit-to-view", icons::FIT, "fit", "canvas"),
            Self::Snap => ("snap-grid", icons::SNAP, "snap", "canvas"),
            Self::Grid => ("grid-settings", icons::GRID, "", "canvas"),
            Self::Check => ("toolbar-check-graph", icons::CHECK, "", "run"),
            Self::Run => ("toolbar-run-graph", icons::RUN, "run", "run"),
        }
    }

    fn supported(self, dialogs: bool) -> bool {
        dialogs || !matches!(self, Self::New | Self::Open | Self::SaveAs)
    }

    fn pinned(self) -> bool {
        matches!(self, Self::Check | Self::Run)
    }
}

#[derive(Debug)]
pub(super) struct Toolbar {
    order: Vec<Action>,
    hidden: Vec<Action>,
}

impl Default for Toolbar {
    fn default() -> Self {
        Self {
            order: Action::ALL.to_vec(),
            hidden: Vec::new(),
        }
    }
}

impl Toolbar {
    fn visible(&self, action: Action) -> bool {
        !self.hidden.contains(&action)
    }

    fn set_visible(&mut self, action: Action, visible: bool) {
        self.hidden.retain(|hidden| *hidden != action);
        if !visible {
            self.hidden.push(action);
        }
    }

    pub(super) fn check(&self) -> bool {
        self.visible(Action::Check)
    }
    pub(super) fn run(&self) -> bool {
        self.visible(Action::Run)
    }

    pub(super) fn leading_width(&self, dialogs: bool, button: f32) -> f32 {
        let mut previous = None;
        let mut width = 0.0;
        for &action in &self.order {
            if action.pinned() || !action.supported(dialogs) || !self.visible(action) {
                continue;
            }
            let category = action.details().3;
            if previous.is_some_and(|group| group != category) {
                width += GROUP_GAP;
            }
            previous = Some(category);
            width += button;
        }
        width
    }

    pub(super) fn preferences(&self) -> serde_json::Value {
        serde_json::json!({
            "order": self.order.iter().map(|action| action.details().0).collect::<Vec<_>>(),
            "hidden": self.hidden.iter().map(|action| action.details().0).collect::<Vec<_>>(),
        })
    }

    pub(super) fn restore(&mut self, value: &serde_json::Value, legacy: &serde_json::Value) {
        *self = Self::default();
        let read = |value: &serde_json::Value| {
            Action::ALL
                .into_iter()
                .find(|action| Some(action.details().0) == value.as_str())
        };
        if let Some(order) = value["order"].as_array() {
            self.order.clear();
            for action in order.iter().filter_map(read).chain(Action::ALL) {
                if !self.order.contains(&action) {
                    self.order.push(action);
                }
            }
        }
        if let Some(hidden) = value["hidden"].as_array() {
            for action in hidden.iter().filter_map(read) {
                self.set_visible(action, false);
            }
        } else {
            for action in Action::ALL {
                self.set_visible(action, legacy[action.details().3].as_bool().unwrap_or(true));
            }
        }
    }
}

impl Editor {
    pub(super) fn toolbar_options(&mut self, ui: &mut egui::Ui, full: bool) {
        if !full {
            if ui.button(t("customize-toolbar")).clicked() {
                self.settings_open = true;
                self.settings_tab = settings::Section::Display;
                self.display_page = settings::DisplayPage::Toolbar;
                self.settings_search.clear();
                ui.close();
            }
            ui.add_space(4.0);
        }
        let actions: Vec<_> = self
            .toolbar_options
            .order
            .iter()
            .copied()
            .filter(|action| action.supported(self.file_dialogs))
            .collect();
        for (index, &action) in actions.iter().enumerate() {
            let (label, icon, _, _) = action.details();
            ui.push_id(label, |ui| {
                ui.horizontal(|ui| {
                    let mut visible = self.toolbar_options.visible(action);
                    ui.label(icons::text(icon));
                    if ui.checkbox(&mut visible, t(label)).changed() {
                        self.toolbar_options.set_visible(action, visible);
                    }
                    if full && !action.pinned() {
                        for (offset, icon, label) in [
                            (-1, icons::MOVE_UP, "toolbar-move-earlier"),
                            (1, icons::MOVE_DOWN, "toolbar-move-later"),
                        ] {
                            let other = index
                                .checked_add_signed(offset)
                                .and_then(|index| actions.get(index))
                                .filter(|action| !action.pinned());
                            if icons::button(ui, other.is_some(), icon, t(label), "").clicked()
                                && let Some(other) = other
                            {
                                let order = &mut self.toolbar_options.order;
                                if let (Some(a), Some(b)) = (
                                    order.iter().position(|item| *item == action),
                                    order.iter().position(|item| item == other),
                                ) {
                                    order.swap(a, b);
                                }
                            }
                        }
                    }
                });
            });
        }
        if full && ui.button(t("toolbar-reset")).clicked() {
            self.toolbar_options = Toolbar::default();
        }
    }

    pub(super) fn toolbar_edit_commands(&mut self, ui: &mut egui::Ui, host: &mut Host, open: bool) {
        let actions = self.toolbar_options.order.clone();
        let mut previous = None;
        let selection = self.canvas.selection();
        let group = self.canvas.selected_group().map(|group| group.id.clone());
        let wire = if selection.is_empty() && group.is_none() {
            self.canvas.selected_wire().cloned()
        } else {
            None
        };
        for action in actions {
            if action.pinned()
                || !action.supported(self.file_dialogs)
                || !self.toolbar_options.visible(action)
            {
                continue;
            }
            let (label, icon, key, category) = action.details();
            if previous.is_some_and(|group| group != category) {
                ui.add_space(GROUP_GAP);
            }
            previous = Some(category);
            let shortcut = if key.is_empty() {
                String::new()
            } else {
                self.canvas.keys.label(key)
            };
            if action == Action::Grid {
                let grid = icons::button(ui, true, icon, t(label), "");
                egui::Popup::menu(&grid)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                        ui.set_width(ui.available_width().min(250.0));
                        self.grid_menu(ui);
                    });
                continue;
            }
            let enabled = match action {
                Action::New | Action::Open => true,
                Action::Undo => self.can_undo().0,
                Action::Redo => self.can_undo().1,
                Action::Group => open && !selection.is_empty() && group.is_none(),
                Action::Ungroup => group.is_some(),
                Action::Arrange => open && selection.len() > 1,
                Action::Unlink => wire.is_some(),
                _ => open,
            };
            let response = if action == Action::Snap {
                icons::toggle_button(ui, icon, self.canvas.snap, t(label), &shortcut)
                    .on_hover_text(t("snap-grid-hint"))
            } else {
                icons::button(ui, enabled, icon, t(label), &shortcut)
            };
            if response.clicked() {
                match action {
                    Action::New => self.ask(FileRequest::New),
                    Action::Open => self.ask(FileRequest::Open),
                    Action::Save => {
                        if let Err(error) = self.save() {
                            self.say(error);
                        }
                    }
                    Action::SaveAs => self.ask(FileRequest::SaveAs),
                    Action::Undo => self.undo(host),
                    Action::Redo => self.redo(host),
                    Action::Group => {
                        let _ = self.group_selection(host, t("group"));
                    }
                    Action::Ungroup => {
                        if let Some(id) = &group {
                            let _ = self.ungroup(host, id);
                        }
                    }
                    Action::Arrange => {
                        let _ = self.arrange_selection();
                    }
                    Action::Unlink => {
                        if let Some(wire) = &wire {
                            let _ = self.remove_wire(host, wire);
                        }
                    }
                    Action::Fit => self.canvas.fit(),
                    Action::Snap => self.canvas.snap = !self.canvas.snap,
                    Action::Grid | Action::Check | Action::Run => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolbar_migrates_groups_and_round_trips_individual_settings() {
        let mut editor = Editor::new();
        editor.restore_preferences(
            &serde_json::json!({"version": 1, "bars": {"file": false}, "corner_radius": 0}),
        );
        assert!(!editor.toolbar_options.visible(Action::Save));
        assert!(editor.toolbar_options.visible(Action::Undo));
        assert_eq!(editor.corners.docks, 0);
        assert_eq!(editor.corners.windows, Some(0));
        editor.corners.menus = Some(9);
        editor.toolbar_options.set_visible(Action::Save, true);
        editor.toolbar_options.set_visible(Action::Run, false);
        editor.toolbar_options.order.swap(2, 0);

        let mut restored = Editor::new();
        restored.restore_preferences(&editor.preferences());
        assert_eq!(restored.toolbar_options.order[0], Action::Save);
        assert!(restored.toolbar_options.visible(Action::Save));
        assert!(!restored.toolbar_options.run());
        assert_eq!(restored.corners.menus, Some(9));
        assert_eq!(restored.corners.controls, Some(0));
    }
}
