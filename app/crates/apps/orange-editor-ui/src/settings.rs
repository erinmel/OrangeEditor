//! Searchable preferences with stable navigation and content panes.
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
    pub(crate) fn result_style(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (mode, label) in [
                (palette::PaletteMode::Names, "result-names"),
                (palette::PaletteMode::Previews, "result-previews"),
            ] {
                ui.selectable_value(&mut self.palette_mode, mode, t(label));
            }
        });
    }

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
        let shown = egui::Window::new(t("settings-title"))
            .id(egui::Id::unique("orange-settings"))
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_size([760.0, 480.0])
            .min_width(680.0)
            .min_height(360.0)
            .max_width(1000.0)
            .collapsible(false)
            .resizable(true)
            .show(ctx, |ui| {
                ui.set_min_width(680.0);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings_search)
                        .hint_text(t("settings-search"))
                        .desired_width(ui.available_width()),
                );
                ui.add_space(8.0);
                let query = self.settings_search.trim().to_lowercase();
                let visible: Vec<_> = sections
                    .iter()
                    .filter(|(section, title, terms)| {
                        query.is_empty()
                            || title.to_lowercase().contains(&query)
                            || terms.contains(&query)
                            || *section == Section::Shortcuts
                                && keymap::ACTIONS.iter().any(|(id, label, _, _)| {
                                    t(label).to_lowercase().contains(&query)
                                        || self
                                            .canvas
                                            .keys
                                            .label(id)
                                            .to_lowercase()
                                            .contains(&query)
                                })
                    })
                    .map(|(section, _, _)| *section)
                    .collect();
                if !visible.contains(&self.settings_tab)
                    && let Some(section) = visible.first()
                {
                    self.settings_tab = *section;
                }
                let height = ui.available_height().max(320.0);
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(160.0, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(160.0);
                            for (section, title, _) in &sections {
                                if visible.contains(section) {
                                    ui.selectable_value(&mut self.settings_tab, *section, *title);
                                }
                            }
                        },
                    );
                    ui.separator();
                    let width = ui.available_width();
                    ui.allocate_ui_with_layout(
                        egui::vec2(width, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_max_width(width);
                            ui.set_min_width(width);
                            egui::ScrollArea::vertical()
                                .id_salt("settings-content")
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.set_max_width(width - 16.0);
                                    if visible.is_empty() {
                                        ui.label(t("settings-empty"));
                                        return;
                                    }
                                    let heading = sections
                                        .iter()
                                        .find(|(section, _, _)| *section == self.settings_tab)
                                        .map_or("", |(_, title, _)| *title);
                                    ui.heading(heading);
                                    ui.add_space(16.0);
                                    match self.settings_tab {
                                        Section::Search => {
                                            ui.label(t("result-layout"));
                                            self.result_style(ui);
                                            ui.add_space(16.0);
                                            ui.label(t("language"));
                                            self.language_choice(ui);
                                        }
                                        Section::Display => {
                                            let mut scale = ctx.zoom_factor();
                                            let slider = egui::Slider::new(&mut scale, 0.75..=2.0)
                                                .text(t("interface-scale"));
                                            if ui.add(slider).changed() {
                                                ctx.set_zoom_factor(scale);
                                            }
                                            ui.add_space(12.0);
                                            ui.checkbox(&mut self.canvas.snap, t("snap-grid"));
                                            ui.checkbox(
                                                &mut self.canvas.scroll_pans,
                                                t("wheel-pans"),
                                            );
                                            ui.add_space(12.0);
                                            ui.checkbox(
                                                &mut self.panels.nodes,
                                                t("show-left-panel"),
                                            );
                                            ui.checkbox(
                                                &mut self.panels.properties,
                                                t("show-right-panel"),
                                            );
                                        }
                                        Section::Shortcuts => self.keybindings_ui(ui),
                                    }
                                });
                        },
                    );
                });
            });
        self.settings_open = open;
        if !open {
            self.key_capture = None;
            self.key_conflict = None;
        }
        shown.map(|window| window.response.rect)
    }
}
