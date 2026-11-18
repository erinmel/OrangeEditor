//! Searchable preferences with independent navigation and content panes.
use super::*;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Section {
    #[default]
    Search,
    Display,
    Shortcuts,
}

impl Editor {
    pub(crate) fn settings_window(&mut self, ctx: &egui::Context) -> Option<egui::Rect> {
        let mut open = self.settings_open;
        if !open {
            return None;
        }
        let sections = [
            (
                Section::Search,
                t("search-heading"),
                t("search-section-words"),
            ),
            (
                Section::Display,
                t("display-heading"),
                t("display-section-words"),
            ),
            (
                Section::Shortcuts,
                t("shortcuts-heading"),
                t("shortcuts-section-words"),
            ),
        ];
        let viewport =
            (ctx.content_rect().size() - egui::vec2(24.0, 36.0)).max(egui::vec2(250.0, 220.0));
        let min_size = egui::vec2(680.0, 380.0).min(viewport);
        let max_size = egui::vec2(1050.0, 700.0).min(viewport);
        let shown = egui::Window::new(t("settings-title"))
            .id(egui::Id::unique("orange-settings"))
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_size(egui::vec2(840.0, 510.0).min(viewport))
            .min_size(min_size)
            .max_size(max_size)
            .collapsible(false)
            .resizable(true)
            .show(ctx, |ui| {
                let size = ui.available_size().clamp(min_size, max_size);
                let (content, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                let footer_y = content.bottom() - 41.0;
                let split = content.left() + (content.width() * 0.26).clamp(116.0, 210.0);
                let nav = egui::Rect::from_min_max(content.min, egui::pos2(split, footer_y));
                let page = egui::Rect::from_min_max(
                    egui::pos2(split + 16.0, content.top() + 9.0),
                    egui::pos2(content.right() - 10.0, footer_y - 8.0),
                );
                ui.painter()
                    .rect_filled(nav, 6.0, ui.visuals().faint_bg_color);
                let mut nav_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt("settings-navigation")
                        .max_rect(nav.shrink(8.0)),
                );
                nav_ui.set_clip_rect(nav_ui.clip_rect().intersect(nav));
                nav_ui.add(
                    egui::TextEdit::singleline(&mut self.settings_search)
                        .hint_text(t("settings-search"))
                        .desired_width(nav_ui.available_width()),
                );
                let query = self.settings_search.trim().to_lowercase();
                nav_ui.add_space(10.0);
                let mut matching = [false; 3];
                let mut section_matched = [false; 3];
                for (index, (section, title, words)) in sections.iter().enumerate() {
                    section_matched[index] = query.is_empty()
                        || title.to_lowercase().contains(&query)
                        || words.to_lowercase().contains(&query);
                    matching[index] = section_matched[index]
                        || *section == Section::Shortcuts
                            && keymap::ACTIONS.iter().any(|(id, label, _, _)| {
                                t(label).to_lowercase().contains(&query)
                                    || self.canvas.keys.label(id).to_lowercase().contains(&query)
                            });
                }
                if !sections
                    .iter()
                    .zip(matching)
                    .any(|((section, _, _), matched)| *section == self.settings_tab && matched)
                    && let Some(index) = matching.iter().position(|&matched| matched)
                {
                    self.settings_tab = sections[index].0;
                }
                for ((section, title, _), matched) in sections.iter().zip(matching) {
                    if matched {
                        let button =
                            egui::Button::selectable(self.settings_tab == *section, *title)
                                .truncate();
                        if nav_ui
                            .add_sized([nav_ui.available_width(), 31.0], button)
                            .clicked()
                        {
                            self.settings_tab = *section;
                        }
                    }
                }
                let mut page_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt("settings-page")
                        .max_rect(page),
                );
                page_ui.set_clip_rect(page_ui.clip_rect().intersect(page));
                if let Some(index) = sections
                    .iter()
                    .zip(matching)
                    .position(|((section, _, _), matched)| *section == self.settings_tab && matched)
                {
                    page_ui.heading(sections[index].1);
                    page_ui.add_space(12.0);
                    match self.settings_tab {
                        Section::Shortcuts => self.keybindings_ui(
                            &mut page_ui,
                            if section_matched[index] { "" } else { &query },
                        ),
                        Section::Search | Section::Display => {
                            egui::ScrollArea::vertical()
                                .id_salt("settings-fields")
                                .show(&mut page_ui, |ui| {
                                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                                    if self.settings_tab == Section::Search {
                                        ui.label(t("settings-language-help"));
                                        self.language_choice(ui);
                                    } else {
                                        self.display_settings(ui, ctx);
                                    }
                                });
                        }
                    }
                } else {
                    page_ui.weak(t("settings-empty"));
                }
                ui.painter().hline(
                    content.x_range(),
                    footer_y,
                    ui.visuals().widgets.noninteractive.bg_stroke,
                );
                let done = egui::Rect::from_min_size(
                    egui::pos2(content.right() - 103.0, footer_y + 8.0),
                    egui::vec2(95.0, 28.0),
                );
                ui.put(done, egui::Button::new(t("settings-done")))
                    .clicked()
            });
        self.settings_open = open
            && !shown
                .as_ref()
                .is_some_and(|window| window.inner == Some(true));
        if !self.settings_open {
            self.shortcut_edit = None;
        }
        shown.map(|window| window.response.rect)
    }

    fn display_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.strong(t("settings-interface"));
        let mut scale = ctx.zoom_factor();
        let slider = egui::Slider::new(&mut scale, 0.75..=2.0).text(t("interface-scale"));
        if ui.add(slider).changed() {
            ctx.set_zoom_factor(scale);
        }
        ui.add_space(14.0);
        ui.checkbox(&mut self.panels.nodes, t("show-left-panel"));
        ui.checkbox(&mut self.panels.properties, t("show-right-panel"));
        ui.add_space(14.0);
        ui.separator();
        ui.add_space(10.0);
        ui.strong(t("settings-canvas"));
        ui.checkbox(&mut self.canvas.snap, t("snap-grid"));
        ui.checkbox(&mut self.canvas.scroll_pans, t("wheel-pans"));
    }
}
