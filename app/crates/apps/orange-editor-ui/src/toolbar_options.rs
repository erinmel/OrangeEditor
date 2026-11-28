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
    Spacer,
    SpacerTwo,
    SpacerThree,
}

impl Action {
    const ALL: [Self; 18] = [
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
        Self::Spacer,
        Self::SpacerTwo,
        Self::SpacerThree,
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
            Self::Spacer => ("toolbar-flex-spacer", icons::DRAG, "", "spacer"),
            Self::SpacerTwo => ("flex-spacer-2", icons::DRAG, "", "spacer"),
            Self::SpacerThree => ("flex-spacer-3", icons::DRAG, "", "spacer"),
        }
    }

    fn supported(self, dialogs: bool) -> bool {
        dialogs || !matches!(self, Self::New | Self::Open | Self::SaveAs)
    }

    const SPACERS: [Self; 3] = [Self::Spacer, Self::SpacerTwo, Self::SpacerThree];

    fn spacer(self) -> bool {
        Self::SPACERS.contains(&self)
    }

    fn default_group(self) -> usize {
        usize::from(matches!(self, Self::Check | Self::Run)) * 2
    }
}

#[derive(Debug)]
pub(super) struct Toolbar {
    order: Vec<Action>,
    hidden: Vec<Action>,
    center: Vec<Action>,
    right: Vec<Action>,
}

impl Default for Toolbar {
    fn default() -> Self {
        Self {
            order: Action::ALL.to_vec(),
            hidden: Action::SPACERS.to_vec(),
            center: Vec::new(),
            right: vec![Action::Check, Action::Run],
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

    fn group(&self, action: Action) -> usize {
        if self.right.contains(&action) {
            2
        } else if self.center.contains(&action) {
            1
        } else {
            0
        }
    }

    fn place(&mut self, action: Action, group: usize, before: Option<Action>) {
        if before == Some(action) {
            return;
        }
        self.center.retain(|item| *item != action);
        self.right.retain(|item| *item != action);
        match group {
            1 => self.center.push(action),
            2 => self.right.push(action),
            _ => {}
        }
        self.order.retain(|item| *item != action);
        let index = before
            .and_then(|before| self.order.iter().position(|item| *item == before))
            .unwrap_or(self.order.len());
        self.order.insert(index, action);
    }

    fn actions(&self, group: usize, dialogs: bool, include_hidden: bool) -> Vec<Action> {
        self.order
            .iter()
            .copied()
            .filter(|&action| {
                self.group(action) == group
                    && action.supported(dialogs)
                    && (include_hidden || self.visible(action))
            })
            .collect()
    }

    fn group_width(actions: &[Action], button: f32) -> f32 {
        let mut previous = None;
        let mut width = 0.0;
        for &action in actions {
            let category = action.details().3;
            if previous.is_some_and(|group| group != category) {
                width += GROUP_GAP;
            }
            previous = Some(category);
            if !action.spacer() {
                width += button;
            }
        }
        width
    }

    pub(super) fn preferences(&self) -> serde_json::Value {
        serde_json::json!({
            "order": self.order.iter().map(|action| action.details().0).collect::<Vec<_>>(),
            "groups": self.order.iter().map(|action| (action.details().0, self.group(*action))).collect::<std::collections::BTreeMap<_, _>>(),
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
        for action in Action::ALL {
            let group = value["groups"][action.details().0]
                .as_u64()
                .unwrap_or(action.default_group() as u64);
            self.center.retain(|item| *item != action);
            self.right.retain(|item| *item != action);
            if group == 1 {
                self.center.push(action);
            } else if group == 2 {
                self.right.push(action);
            }
        }
        if let Some(hidden) = value["hidden"].as_array() {
            self.hidden.clear();
            for action in hidden.iter().filter_map(read) {
                self.set_visible(action, false);
            }
            for spacer in Action::SPACERS {
                if value["groups"][spacer.details().0].is_null() {
                    self.set_visible(spacer, false);
                }
            }
        } else {
            for action in Action::ALL {
                self.set_visible(
                    action,
                    !action.spacer() && legacy[action.details().3].as_bool().unwrap_or(true),
                );
            }
        }
    }
}

impl Editor {
    pub(super) fn toolbar_search_rank(
        &self,
        query: &orange_host::SearchQuery,
    ) -> Option<orange_host::SearchRank> {
        Action::ALL
            .into_iter()
            .filter(|action| action.supported(self.file_dialogs) && !action.spacer())
            .filter_map(|action| i18n::message_relevance(query, &[action.details().0], &[]))
            .min()
    }

    pub(super) fn toolbar_options(&mut self, ui: &mut egui::Ui, full: bool) {
        if !full {
            if ui.button(t("customize-toolbar")).clicked() {
                self.settings_open = true;
                self.settings_tab = settings::Section::Toolbar;
                self.settings_search.clear();
                ui.close();
            }
            ui.add_space(4.0);
        }
        if full {
            let mut zones = Vec::new();
            let mut rows = Vec::new();
            for (group, label) in ["toolbar-left", "toolbar-center", "toolbar-right"]
                .into_iter()
                .enumerate()
            {
                let mut actions = self.toolbar_options.actions(group, self.file_dialogs, true);
                actions.retain(|action| !action.spacer() || self.toolbar_options.visible(*action));
                let frame = egui::Frame::new()
                    .fill(ui.visuals().faint_bg_color)
                    .corner_radius(self.corners.docks)
                    .inner_margin(8)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.strong(t(label));
                            let unused = Action::SPACERS
                                .into_iter()
                                .find(|action| !self.toolbar_options.visible(*action));
                            if ui
                                .add_enabled(
                                    unused.is_some(),
                                    egui::Button::new(t("toolbar-add-spacer")),
                                )
                                .clicked()
                                && let Some(action) = unused
                            {
                                self.toolbar_options.place(action, group, None);
                                self.toolbar_options.set_visible(action, true);
                            }
                        });
                        for &action in &actions {
                            let (label, icon, _, _) = action.details();
                            let title = t(if action.spacer() {
                                "toolbar-flex-spacer"
                            } else {
                                label
                            });
                            let mut visible = self.toolbar_options.visible(action);
                            let rect = ui
                                .push_id(label, |ui| {
                                    orange_ui_controls::navigation::settings_row(
                                        ui,
                                        34.0,
                                        84.0,
                                        |ui| {
                                            let grip = orange_ui_controls::navigation::drag_grip(
                                                ui,
                                                icons::DRAG,
                                                t("toolbar-drag-action"),
                                            );
                                            grip.dnd_set_drag_payload(action);
                                            let (rect, _) = ui.allocate_exact_size(
                                                egui::Vec2::splat(16.0),
                                                egui::Sense::hover(),
                                            );
                                            icons::paint_command(
                                                ui,
                                                rect,
                                                icon,
                                                ui.visuals().text_color(),
                                            );
                                            ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(title).size(14.0),
                                                )
                                                .truncate(),
                                            );
                                        },
                                        |ui| {
                                            let options = icons::button(
                                                ui,
                                                true,
                                                icons::MORE,
                                                t("toolbar-action-options"),
                                                "",
                                            );
                                            self.toolbar_action_menu(
                                                &options, action, group, &actions,
                                            );
                                            orange_ui_controls::checkbox(
                                                ui,
                                                Surface::Dialog,
                                                orange_ui_controls::CheckStyle::Toggle,
                                                &mut visible,
                                                "",
                                            )
                                            .accessible_name(title);
                                        },
                                    )
                                })
                                .inner;
                            self.toolbar_options.set_visible(action, visible);
                            rows.push((group, action, rect));
                        }
                    });
                zones.push((group, frame.response.rect));
                ui.add_space(6.0);
            }
            self.toolbar_drop(ui, &zones, &rows);
            if ui.button(t("toolbar-reset")).clicked() {
                self.toolbar_options = Toolbar::default();
            }
        } else {
            for action in self
                .toolbar_options
                .order
                .clone()
                .into_iter()
                .filter(|action| action.supported(self.file_dialogs) && !action.spacer())
            {
                let (label, _, _, _) = action.details();
                let mut visible = self.toolbar_options.visible(action);
                if ui.checkbox(&mut visible, t(label)).changed() {
                    self.toolbar_options.set_visible(action, visible);
                }
            }
        }
    }

    fn toolbar_action_menu(
        &mut self,
        options: &egui::Response,
        action: Action,
        group: usize,
        actions: &[Action],
    ) {
        egui::Popup::menu(options).show(|ui| {
            let index = actions.iter().position(|item| *item == action);
            if let Some(index) = index {
                for (offset, label) in [(-1, "toolbar-move-earlier"), (1, "toolbar-move-later")] {
                    let other = index
                        .checked_add_signed(offset)
                        .and_then(|target| actions.get(target));
                    if ui
                        .add_enabled(other.is_some(), egui::Button::new(t(label)))
                        .clicked()
                        && let Some(other) = other
                    {
                        let before = if offset < 0 {
                            Some(*other)
                        } else {
                            actions.get(index + 2).copied()
                        };
                        self.toolbar_options.place(action, group, before);
                        ui.close();
                    }
                }
            }
            ui.add_space(4.0);
            for (target, label) in ["toolbar-left", "toolbar-center", "toolbar-right"]
                .into_iter()
                .enumerate()
            {
                if ui
                    .add_enabled(group != target, egui::Button::new(t(label)))
                    .clicked()
                {
                    self.toolbar_options.place(action, target, None);
                    ui.close();
                }
            }
        });
    }

    fn toolbar_drop(
        &mut self,
        ui: &mut egui::Ui,
        zones: &[(usize, egui::Rect)],
        rows: &[(usize, Action, egui::Rect)],
    ) {
        let payload = egui::DragAndDrop::payload::<Action>(ui.ctx());
        let pickup = ui.ctx().animate_bool_with_time(
            ui.scope_id().with("toolbar-drag-pickup"),
            payload.is_some(),
            ui.style().animation_time,
        );
        let Some(action) = payload else {
            return;
        };
        let Some(pointer) = ui.input(|input| input.pointer.interact_pos()) else {
            return;
        };
        self.toolbar_drag_preview(ui, *action, pointer, pickup);
        let clip = ui.clip_rect();
        if !clip.contains(pointer) {
            return;
        }
        if ui.input(|input| input.pointer.primary_down()) {
            let edge = 28.0;
            let top = (clip.top() + edge - pointer.y).clamp(0.0, edge);
            let bottom = (pointer.y - clip.bottom() + edge).clamp(0.0, edge);
            if top != bottom {
                ui.scroll_with_delta_animation(
                    egui::vec2(0.0, (top - bottom) * 0.48),
                    egui::style::ScrollAnimation::none(),
                );
                ui.request_repaint();
            }
        }
        let Some(&(group, zone)) = zones
            .iter()
            .find(|(_, rect)| rect.intersect(clip).contains(pointer))
        else {
            return;
        };
        let before = rows
            .iter()
            .find(|(index, _, rect)| *index == group && pointer.y < rect.center().y);
        let target = before.map_or(zone.bottom() - 3.0, |(_, _, rect)| rect.top());
        let line = ui.ctx().animate_value_with_time(
            ui.scope_id().with(("toolbar-drop-line", group)),
            target,
            ui.style().animation_time,
        );
        ui.painter().hline(
            zone.shrink(7.0).x_range(),
            line,
            ui.visuals().selection.stroke,
        );
        if ui.input(|input| input.pointer.primary_released()) {
            let _ = egui::DragAndDrop::take_payload::<Action>(ui.ctx());
            self.toolbar_options
                .place(*action, group, before.map(|(_, action, _)| *action));
            ui.request_repaint();
        }
    }

    fn toolbar_drag_preview(
        &self,
        ui: &mut egui::Ui,
        action: Action,
        pointer: egui::Pos2,
        pickup: f32,
    ) {
        let (label, icon, _, _) = action.details();
        let label = t(if action.spacer() {
            "toolbar-flex-spacer"
        } else {
            label
        });
        let bounds = ui.ctx().content_rect().shrink(4.0);
        let painter = ui
            .ctx()
            .layer_painter(egui::LayerId::new(
                egui::Order::Tooltip,
                ui.scope_id().with("toolbar-drag-preview"),
            ))
            .with_clip_rect(bounds);
        let theme = orange_ui_controls::Theme::of(ui);
        let font = egui::TextStyle::Body.resolve(ui.style());
        let galley = painter.layout(
            label.to_owned(),
            font,
            theme.text,
            (bounds.width() - 42.0).max(1.0),
        );
        let size = galley.size() + egui::vec2(42.0, 14.0);
        let offset = egui::vec2(15.0, 9.0 + 6.0 * pickup);
        let prefer = if pointer.y + offset.y + size.y > bounds.bottom() {
            pointer + egui::vec2(offset.x, -offset.y - size.y)
        } else {
            pointer + offset
        };
        let anchor = prefer
            .min((bounds.max - size).max(bounds.min))
            .max(bounds.min);
        let rect = egui::Rect::from_min_size(anchor, size);
        painter.rect_filled(
            rect.translate(egui::vec2(0.0, 3.0)),
            self.corners.docks,
            egui::Color32::from_black_alpha(75),
        );
        painter.rect(
            rect,
            self.corners.docks,
            theme.dialog,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
        let mut preview = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("toolbar-drag-icon")
                .layer_id(painter.layer_id())
                .max_rect(rect),
        );
        preview.set_clip_rect(bounds);
        let icon_rect = egui::Rect::from_center_size(
            anchor + egui::vec2(16.0, size.y / 2.0),
            egui::Vec2::splat(16.0),
        );
        icons::paint_command(&preview, icon_rect, icon, theme.text);
        painter.galley(anchor + egui::vec2(30.0, 7.0), galley, theme.text);
    }

    pub(super) fn toolbar_groups(
        &mut self,
        ui: &mut egui::Ui,
        host: &mut Host,
        open: bool,
        trailing: f32,
    ) {
        let gap = ui.spacing().item_spacing.x;
        let button = icons::BUTTON_WIDTH + gap;
        let groups = [0, 1, 2].map(|group| {
            self.toolbar_options
                .actions(group, self.file_dialogs, false)
        });
        let base = groups
            .each_ref()
            .map(|actions| Toolbar::group_width(actions, button));
        let spacers = groups.each_ref().map(|actions| {
            actions
                .iter()
                .filter(|action| action.spacer())
                .map(|_| 1.0)
                .sum::<f32>()
        });
        let available = (ui.available_width() - trailing).max(1.0);
        let measured = base.iter().sum::<f32>();
        let spacer = (available - measured).max(0.0) / spacers.iter().sum::<f32>().max(1.0);
        let widths: [f32; 3] = std::array::from_fn(|index| base[index] + spacer * spacers[index]);
        let render = |editor: &mut Self, ui: &mut egui::Ui, host: &mut Host| {
            let origin = ui.cursor().left();
            let right = (available - widths[2]).max(widths[0] + widths[1]);
            let center = ((available - widths[1]) / 2.0)
                .clamp(widths[0], (right - widths[1]).max(widths[0]));
            let starts = [0.0, center, right];
            if measured <= available {
                editor.toolbar_open_space(ui, available, starts, widths, &groups);
            }
            for (index, actions) in groups.iter().enumerate() {
                if actions.is_empty() {
                    continue;
                }
                let distance = origin + starts[index] - ui.cursor().left();
                if distance > gap {
                    ui.add_space(distance - gap);
                }
                editor.toolbar_edit_commands(ui, host, open, actions, (spacer - gap).max(0.0));
            }
        };
        if measured > available {
            egui::ScrollArea::horizontal()
                .id_salt("toolbar-edit-commands")
                .max_width(available)
                .min_scrolled_width(1.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| render(self, ui, host));
                });
        } else {
            render(self, ui, host);
        }
    }

    fn toolbar_open_space(
        &mut self,
        ui: &mut egui::Ui,
        available: f32,
        starts: [f32; 3],
        widths: [f32; 3],
        groups: &[Vec<Action>; 3],
    ) {
        let mut from = 0.0;
        let mut largest: (f32, f32) = (0.0, 0.0);
        for (index, actions) in groups.iter().enumerate() {
            if actions.is_empty() {
                continue;
            }
            if (starts[index] - from) > (largest.1 - largest.0) {
                largest = (from, starts[index]);
            }
            from = starts[index] + widths[index];
        }
        if available - from > largest.1 - largest.0 {
            largest = (from, available);
        }
        if largest.1 - largest.0 < 12.0 {
            return;
        }
        let top = ui.cursor().min;
        let rect = egui::Rect::from_min_size(
            top + egui::vec2(largest.0, 0.0),
            egui::vec2(largest.1 - largest.0, ui.spacing().interact_size.y),
        );
        let response = ui.interact(
            rect,
            ui.scope_id().with("toolbar-open-space"),
            egui::Sense::click_and_drag(),
        );
        response.context_menu(|ui| self.toolbar_options(ui, false));
        if self.window_toolbar {
            if rect.width() > 100.0 {
                let name = self
                    .path()
                    .and_then(|path| std::path::Path::new(path).file_name())
                    .map_or_else(|| "OrangeEditor".into(), std::ffi::OsStr::to_string_lossy);
                ui.painter()
                    .with_clip_rect(rect.shrink2(egui::vec2(8.0, 0.0)))
                    .text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        name,
                        egui::FontId::proportional(12.0),
                        ui.visuals().weak_text_color(),
                    );
            }
            if response.double_clicked() {
                let maximized = ui.input(|input| input.viewport().maximized.unwrap_or(false));
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
            } else if response.drag_started() && ui.input(|input| input.pointer.primary_down()) {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
        }
    }

    fn toolbar_edit_commands(
        &mut self,
        ui: &mut egui::Ui,
        host: &mut Host,
        open: bool,
        actions: &[Action],
        spacer: f32,
    ) {
        let mut previous = None;
        let selection = self.canvas.selection();
        let group = self.canvas.selected_group().map(|group| group.id.clone());
        let wire = if selection.is_empty() && group.is_none() {
            self.canvas.selected_wire().cloned()
        } else {
            None
        };
        for &action in actions {
            let (label, icon, key, category) = action.details();
            if previous.is_some_and(|group| group != category) {
                ui.add_space(GROUP_GAP);
            }
            previous = Some(category);
            match action {
                Action::Spacer | Action::SpacerTwo | Action::SpacerThree => {
                    ui.add_space(spacer);
                    continue;
                }
                Action::Run => {
                    self.toolbar_run(ui, host, open);
                    continue;
                }
                Action::Check => {
                    if icons::button(ui, open, icons::CHECK, t("check-graph-hint"), "").clicked() {
                        self.check(host);
                    }
                    continue;
                }
                _ => {}
            }
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
                    Action::Grid
                    | Action::Check
                    | Action::Run
                    | Action::Spacer
                    | Action::SpacerTwo
                    | Action::SpacerThree => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolbar_settings_and_preview_use_the_toolbar_command_painter() {
        let mut editor = Editor::new();
        let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
        let ctx = egui::Context::default();
        icons::install(&ctx);
        let painted = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let capture = painted.clone();
        icons::set_command_painter(
            &ctx,
            Some(std::sync::Arc::new(move |painter, rect, icon, _| {
                capture
                    .lock()
                    .unwrap()
                    .push((icon, painter.layer_id().order, rect.size()));
                true
            })),
        );

        let render = |draw: &mut dyn FnMut(&mut egui::Ui)| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 850.0),
                    )),
                    ..Default::default()
                },
                draw,
            )
            .drop_without_applying_deltas();
            std::mem::take(&mut *painted.lock().unwrap())
        };
        let toolbar = render(&mut |ui| editor.toolbar(ui, &mut host));
        let settings = render(&mut |ui| editor.toolbar_options(ui, true));
        for icon in [icons::GROUP, icons::UNGROUP, icons::ARRANGE, icons::SNAP] {
            assert!(
                toolbar
                    .iter()
                    .any(|entry| entry.0 == icon && entry.2 == egui::Vec2::splat(16.0))
            );
            assert!(
                settings
                    .iter()
                    .any(|entry| entry.0 == icon && entry.2 == egui::Vec2::splat(16.0))
            );
        }
        let preview = render(&mut |ui| {
            editor.toolbar_drag_preview(ui, Action::Snap, egui::pos2(150.0, 150.0), 1.0);
        });
        assert_eq!(
            preview,
            [(icons::SNAP, egui::Order::Tooltip, egui::Vec2::splat(16.0))]
        );
    }

    #[test]
    fn dragging_a_command_grip_moves_it_into_the_center_group() {
        let mut editor = Editor::new();
        editor.file_dialogs = true;
        let ctx = egui::Context::default();
        icons::install(&ctx);
        let frame = |editor: &mut Editor, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(680.0, 900.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(760.0)
                        .show(ui, |ui| editor.toolbar_options(ui, true));
                },
            );
            let shapes = std::mem::take(&mut output.shapes);
            output.drop_without_applying_deltas();
            shapes
        };
        frame(&mut editor, vec![]);
        let shapes = frame(&mut editor, vec![]);
        let at = |label: &str| {
            shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(text.pos + text.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let grip = at(&icons::DRAG.to_string())[0];
        let center = at(t("toolbar-center"))[0];
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: egui::Modifiers::NONE,
        };
        let target = center + egui::vec2(180.0, 3.0);
        let mut saw_preview = false;
        for (step, events) in [
            vec![egui::Event::PointerMoved(grip)],
            vec![button(grip, true)],
            vec![egui::Event::PointerMoved(grip + egui::vec2(12.0, 0.0))],
            vec![egui::Event::PointerMoved(target)],
            vec![button(target, false)],
        ]
        .into_iter()
        .enumerate()
        {
            let shapes = frame(&mut editor, events);
            if step == 3 {
                saw_preview = shapes
                    .iter()
                    .filter(|shape| {
                        matches!(&shape.shape,
                            egui::Shape::Text(text) if text.galley.text() == t("new-graph")
                        )
                    })
                    .count()
                    >= 2;
            }
        }
        assert!(saw_preview, "the command label follows the pointer");
        assert_eq!(editor.toolbar_options.group(Action::New), 1);
    }

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
        assert!(!restored.toolbar_options.visible(Action::Run));
        assert_eq!(restored.corners.menus, Some(9));
        assert_eq!(restored.corners.controls, Some(0));
        restored.toolbar_options.place(Action::Run, 1, None);
        restored
            .toolbar_options
            .place(Action::Spacer, 2, Some(Action::Check));
        restored.toolbar_options.set_visible(Action::Spacer, true);
        editor.restore_preferences(&restored.preferences());
        assert_eq!(editor.toolbar_options.group(Action::Run), 1);
        assert_eq!(
            editor.toolbar_options.actions(2, true, false),
            [Action::Spacer, Action::Check]
        );
    }
}
