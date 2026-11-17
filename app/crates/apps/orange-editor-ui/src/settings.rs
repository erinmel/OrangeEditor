//! Searchable preferences with stable navigation and content panes.
use super::*;

#[cfg(test)]
mod tests;

pub(crate) fn text(host: &Host, language: &str, key: &str, fallback: &str) -> String {
    key.strip_prefix("org.orange.host/")
        .and_then(|id| host.catalogs().message(language, id))
        .map_or_else(|| fallback.to_owned(), std::borrow::Cow::into_owned)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Section {
    #[default]
    Search,
    Display,
    Shortcuts,
}

impl Editor {
    pub(crate) fn result_style(&mut self, ui: &mut egui::Ui, host: &Host) {
        let language = self.language.clone();
        ui.horizontal(|ui| {
            for (mode, key, fallback) in [
                (
                    palette::PaletteMode::Names,
                    "org.orange.host/editor-result-names",
                    "Names",
                ),
                (
                    palette::PaletteMode::Previews,
                    "org.orange.host/editor-result-previews",
                    "Preview",
                ),
            ] {
                ui.selectable_value(
                    &mut self.palette_mode,
                    mode,
                    text(host, &language, key, fallback),
                );
            }
        });
    }

    pub(crate) fn settings_window(
        &mut self,
        ctx: &egui::Context,
        host: &Host,
    ) -> Option<egui::Rect> {
        let mut open = self.settings_open;
        if !open {
            return None;
        }
        let language = self.language.clone();
        let tr = |key, fallback| text(host, &language, key, fallback);
        let sections = [
            (
                Section::Search,
                tr("org.orange.host/editor-search-heading", "Node search"),
                "names preview language search",
            ),
            (
                Section::Display,
                tr(
                    "org.orange.host/editor-display-heading",
                    "Display and canvas",
                ),
                "scale zoom snap grid wheel panels",
            ),
            (
                Section::Shortcuts,
                tr(
                    "org.orange.host/editor-shortcuts-heading",
                    "Keyboard shortcuts",
                ),
                "keymap keyboard shortcuts",
            ),
        ];
        let shown = egui::Window::new(tr("org.orange.host/editor-settings-title", "Settings"))
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
                        .hint_text(tr(
                            "org.orange.host/editor-settings-search",
                            "Search settings",
                        ))
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
                                    label.to_lowercase().contains(&query)
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
                                    ui.selectable_value(
                                        &mut self.settings_tab,
                                        *section,
                                        title.as_str(),
                                    );
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
                                        ui.label("No matching settings");
                                        return;
                                    }
                                    let heading = sections
                                        .iter()
                                        .find(|(section, _, _)| *section == self.settings_tab)
                                        .map_or("", |(_, title, _)| title.as_str());
                                    ui.heading(heading);
                                    ui.add_space(16.0);
                                    match self.settings_tab {
                                        Section::Search => {
                                            ui.label(tr(
                                                "org.orange.host/editor-result-layout",
                                                "Result layout",
                                            ));
                                            self.result_style(ui, host);
                                            ui.add_space(16.0);
                                            ui.label(tr(
                                                "org.orange.host/editor-search-language",
                                                "Node language",
                                            ));
                                            self.language_choice(ui, host);
                                        }
                                        Section::Display => {
                                            let mut scale = ctx.zoom_factor();
                                            let slider = egui::Slider::new(&mut scale, 0.75..=2.0)
                                                .text(tr(
                                                    "org.orange.host/editor-interface-scale",
                                                    "Interface scale",
                                                ));
                                            if ui.add(slider).changed() {
                                                ctx.set_zoom_factor(scale);
                                            }
                                            ui.add_space(12.0);
                                            ui.checkbox(
                                                &mut self.canvas.snap,
                                                tr(
                                                    "org.orange.host/editor-snap-grid",
                                                    "Snap nodes to grid",
                                                ),
                                            );
                                            ui.checkbox(
                                                &mut self.canvas.scroll_pans,
                                                tr(
                                                    "org.orange.host/editor-wheel-pans",
                                                    "Mouse wheel pans instead of zooming",
                                                ),
                                            );
                                            ui.add_space(12.0);
                                            ui.checkbox(&mut self.panels.nodes, "Show left panel");
                                            ui.checkbox(
                                                &mut self.panels.properties,
                                                "Show right panel",
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
