use super::*;
#[test]
fn changing_settings_sections_preserves_window_width() {
    let mut editor = Editor::new();
    editor.settings_open = true;
    let context = egui::Context::default();
    let mut widths = Vec::new();
    for section in [
        Section::Search,
        Section::Shortcuts,
        Section::Display,
        Section::Search,
    ] {
        editor.settings_tab = section;
        for _ in 0..3 {
            context
                .run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1200.0, 800.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        let rect = editor.settings_window(ui.ctx()).unwrap();
                        widths.push(rect.width());
                    },
                )
                .drop_without_applying_deltas();
        }
    }
    for width in &widths[3..] {
        assert!((width - widths[3]).abs() < 2.0, "{widths:?}");
    }
}
