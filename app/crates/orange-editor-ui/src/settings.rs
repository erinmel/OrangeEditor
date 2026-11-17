//! Presentation preferences shared by the sidebar and the canvas picker.
use super::*;

/// Parameter-free `org.orange.host` messages use the host's existing locale catalogue.
/// The safe English fallback remains available when a host supplies no translation.
pub(crate) fn text<'a>(host: &'a Host, language: &str, key: &str, fallback: &'a str) -> &'a str {
    key.strip_prefix("org.orange.host/")
        .and_then(|id| host.search_words().message(language, id))
        .unwrap_or(fallback)
}

impl Editor {
    pub(crate) fn settings_window(&mut self, ctx: &egui::Context, host: &Host) {
        let mut open = self.settings_open;
        let language = self.language().to_owned();
        let tr = |key, fallback| text(host, &language, key, fallback);
        egui::Window::new(tr("org.orange.host/editor-settings-title", "Settings"))
            .id(egui::Id::unique("orange-settings"))
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(380.0)
            .resizable(true)
            .vscroll(true)
            .show(ctx, |ui| {
                ui.heading(tr("org.orange.host/editor-search-heading", "Node search"));
                ui.label(tr("org.orange.host/editor-result-layout", "Result layout"));
                ui.horizontal_wrapped(|ui| {
                    let _ = ui.selectable_value(
                        &mut self.palette_mode,
                        palette::PaletteMode::Names,
                        tr("org.orange.host/editor-result-names", "Names"),
                    );
                    let _ = ui.selectable_value(
                        &mut self.palette_mode,
                        palette::PaletteMode::Previews,
                        tr("org.orange.host/editor-result-previews", "Names + previews"),
                    );
                });
                ui.weak(tr("org.orange.host/editor-result-scope", "Applies to the sidebar and canvas picker."));
                ui.label(tr("org.orange.host/editor-search-language", "Search language"));
                self.language_choice(ui, host);
                ui.add_space(12.0);
                ui.heading(tr("org.orange.host/editor-display-heading", "Display and canvas"));
                let mut scale = ctx.zoom_factor();
                if ui
                    .add(egui::Slider::new(&mut scale, 0.75..=2.0).text(tr("org.orange.host/editor-interface-scale", "Interface scale")))
                    .changed()
                {
                    ctx.set_zoom_factor(scale);
                }
                let _ = ui.checkbox(&mut self.canvas.snap, tr("org.orange.host/editor-snap-grid", "Snap nodes to grid"));
                ui.weak(tr("org.orange.host/editor-snap-hint", "Hold Alt while dragging to move freely."));
                let _ = ui.checkbox(
                    &mut self.canvas.scroll_pans,
                    tr("org.orange.host/editor-wheel-pans", "Mouse wheel pans instead of zooming"),
                );
                ui.add_space(12.0);
                ui.heading(tr("org.orange.host/editor-shortcuts-heading", "Keyboard shortcuts"));
                egui::Grid::new("orange-shortcut-reference")
                    .num_columns(2)
                    .spacing(egui::vec2(24.0, 6.0))
                    .show(ui, |ui| {
                        for (keys, id, action) in [
                            ("Tab / Space", "org.orange.host/editor-shortcut-find", "Find a node at the pointer"),
                            ("Up / Down · Enter", "org.orange.host/editor-shortcut-choose", "Choose a search result"),
                            ("Escape", "org.orange.host/editor-shortcut-close", "Close search or menu"),
                            ("Ctrl+S", "org.orange.host/editor-shortcut-save", "Save"),
                            ("Ctrl+Z / Ctrl+Y", "org.orange.host/editor-shortcut-undo", "Undo / redo"),
                            ("Ctrl+C / X / V", "org.orange.host/editor-shortcut-copy", "Copy / cut / paste selection"),
                            ("Ctrl+D", "org.orange.host/editor-shortcut-duplicate", "Duplicate selection"),
                            ("Delete", "org.orange.host/editor-shortcut-delete", "Delete selected nodes or wire"),
                            ("Ctrl+G", "org.orange.host/editor-shortcut-group", "Group selection"),
                            ("Ctrl+E", "org.orange.host/editor-shortcut-function", "Make function from selection"),
                            ("Ctrl+L", "org.orange.host/editor-shortcut-arrange", "Arrange selection"),
                            ("Home", "org.orange.host/editor-shortcut-fit", "Fit graph to view"),
                            ("Shift / Ctrl + wheel", "org.orange.host/editor-shortcut-pan", "Pan horizontally / vertically"),
                        ] {
                            ui.label(keys);
                            ui.label(tr(id, action));
                            ui.end_row();
                        }
                    });
            });
        self.settings_open = open;
    }
}
