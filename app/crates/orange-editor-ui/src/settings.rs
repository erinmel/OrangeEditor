//! Presentation preferences shared by the sidebar and the canvas picker.
use super::*;

/// Parameter-free `org.orange.host` messages use the host's existing locale catalogue.
/// The safe English fallback remains available when a host supplies no translation.
pub(crate) fn text<'a>(host: &'a Host, language: &str, key: &str, fallback: &'a str) -> &'a str {
    key.strip_prefix("org.orange.host/")
        .and_then(|id| host.search_words().message(language, id))
        .unwrap_or(fallback)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Section {
    #[default]
    Search,
    Display,
    Shortcuts,
}

const SHORTCUTS: &[(&str, &str, &str)] = &[
    (
        "Tab / Space",
        "org.orange.host/editor-shortcut-find",
        "Find a node at the pointer",
    ),
    (
        "Up / Down · Enter",
        "org.orange.host/editor-shortcut-choose",
        "Choose a search result",
    ),
    (
        "Escape",
        "org.orange.host/editor-shortcut-close",
        "Close search or menu",
    ),
    ("Ctrl+S", "org.orange.host/editor-shortcut-save", "Save"),
    (
        "Ctrl+Z / Ctrl+Y",
        "org.orange.host/editor-shortcut-undo",
        "Undo / redo",
    ),
    (
        "Ctrl+C / X / V",
        "org.orange.host/editor-shortcut-copy",
        "Copy / cut / paste selection",
    ),
    (
        "Ctrl+D",
        "org.orange.host/editor-shortcut-duplicate",
        "Duplicate selection",
    ),
    (
        "Delete",
        "org.orange.host/editor-shortcut-delete",
        "Delete selected nodes or wire",
    ),
    (
        "Ctrl+G",
        "org.orange.host/editor-shortcut-group",
        "Group selection",
    ),
    (
        "Ctrl+E",
        "org.orange.host/editor-shortcut-function",
        "Make function from selection",
    ),
    (
        "Ctrl+L",
        "org.orange.host/editor-shortcut-arrange",
        "Arrange selection",
    ),
    (
        "Home",
        "org.orange.host/editor-shortcut-fit",
        "Fit graph to view",
    ),
    (
        "Shift / Ctrl + wheel",
        "org.orange.host/editor-shortcut-pan",
        "Pan horizontally / vertically",
    ),
];

impl Editor {
    pub(crate) fn result_style(&mut self, ui: &mut egui::Ui, host: &Host) {
        let language = self.language().to_owned();
        ui.horizontal_wrapped(|ui| {
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
                let _ = ui.selectable_value(
                    &mut self.palette_mode,
                    mode,
                    text(host, &language, key, fallback),
                );
            }
        });
    }

    pub(crate) fn settings_window(&mut self, ctx: &egui::Context, host: &Host) {
        let mut open = self.settings_open;
        if !open {
            return;
        }
        let language = self.language().to_owned();
        let tr = |key, fallback| text(host, &language, key, fallback);
        egui::Window::new(tr("org.orange.host/editor-settings-title", "Settings"))
            .id(egui::Id::unique("orange-settings"))
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_size([650.0, 420.0])
            .min_width(460.0)
            .min_height(280.0)
            .resizable(true)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings_search)
                        .hint_text(tr(
                            "org.orange.host/editor-settings-search",
                            "Search settings",
                        ))
                        .desired_width(f32::INFINITY),
                );
                ui.separator();
                let query = self.settings_search.trim().to_lowercase();
                let sections = [
                    (
                        Section::Search,
                        tr("org.orange.host/editor-search-heading", "Node search"),
                        [
                            tr("org.orange.host/editor-result-layout", "Result layout"),
                            tr("org.orange.host/editor-search-language", "Search language"),
                            tr("org.orange.host/editor-result-previews", "Preview"),
                        ],
                    ),
                    (
                        Section::Display,
                        tr(
                            "org.orange.host/editor-display-heading",
                            "Display and canvas",
                        ),
                        [
                            tr("org.orange.host/editor-interface-scale", "Interface scale"),
                            tr("org.orange.host/editor-snap-grid", "Snap nodes to grid"),
                            tr(
                                "org.orange.host/editor-wheel-pans",
                                "Mouse wheel pans instead of zooming",
                            ),
                        ],
                    ),
                    (
                        Section::Shortcuts,
                        tr(
                            "org.orange.host/editor-shortcuts-heading",
                            "Keyboard shortcuts",
                        ),
                        ["Ctrl+S", "Escape", "Tab / Space"],
                    ),
                ];
                let visible = |section: Section, title: &str, keywords: [&str; 3]| {
                    (section == Section::Shortcuts
                        && SHORTCUTS.iter().any(|(keys, key, fallback)| {
                            keys.to_lowercase().contains(&query)
                                || tr(key, fallback).to_lowercase().contains(&query)
                        }))
                        || title.to_lowercase().contains(&query)
                        || keywords
                            .iter()
                            .any(|word| word.to_lowercase().contains(&query))
                };
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_min_width(142.0);
                        for (section, title, keywords) in sections {
                            if visible(section, title, keywords) {
                                let _ = ui.selectable_value(&mut self.settings_tab, section, title);
                            }
                        }
                    });
                    ui.separator();
                    let active = sections
                        .iter()
                        .find(|(section, title, terms)| {
                            *section == self.settings_tab && visible(*section, title, *terms)
                        })
                        .or_else(|| {
                            sections
                                .iter()
                                .find(|(section, title, terms)| visible(*section, title, *terms))
                        });
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let Some((section, title, _)) = active else {
                            ui.weak(tr(
                                "org.orange.host/editor-settings-empty",
                                "No matching sections",
                            ));
                            return;
                        };
                        ui.heading(*title);
                        ui.add_space(8.0);
                        match section {
                            Section::Search => {
                                ui.label(tr(
                                    "org.orange.host/editor-result-layout",
                                    "Result layout",
                                ));
                                self.result_style(ui, host);
                                ui.weak(tr(
                                    "org.orange.host/editor-result-scope",
                                    "Applies to the sidebar and canvas picker.",
                                ));
                                ui.label(tr(
                                    "org.orange.host/editor-search-language",
                                    "Search language",
                                ));
                                self.language_choice(ui, host);
                            }
                            Section::Display => {
                                let mut scale = ctx.zoom_factor();
                                if ui
                                    .add(egui::Slider::new(&mut scale, 0.75..=2.0).text(tr(
                                        "org.orange.host/editor-interface-scale",
                                        "Interface scale",
                                    )))
                                    .changed()
                                {
                                    ctx.set_zoom_factor(scale);
                                }
                                let _ = ui.checkbox(
                                    &mut self.canvas.snap,
                                    tr("org.orange.host/editor-snap-grid", "Snap nodes to grid"),
                                );
                                ui.weak(tr(
                                    "org.orange.host/editor-snap-hint",
                                    "Hold Alt while dragging to move freely.",
                                ));
                                let _ = ui.checkbox(
                                    &mut self.canvas.scroll_pans,
                                    tr(
                                        "org.orange.host/editor-wheel-pans",
                                        "Mouse wheel pans instead of zooming",
                                    ),
                                );
                            }
                            Section::Shortcuts => {
                                egui::Grid::new("orange-shortcut-reference")
                                    .num_columns(2)
                                    .spacing(egui::vec2(24.0, 6.0))
                                    .show(ui, |ui| {
                                        for &(keys, id, action) in SHORTCUTS {
                                            ui.label(keys);
                                            ui.label(tr(id, action));
                                            ui.end_row();
                                        }
                                    });
                            }
                        }
                    });
                });
            });
        self.settings_open = open;
    }
}
