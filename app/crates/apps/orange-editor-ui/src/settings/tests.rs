use super::*;

#[test]
fn appearance_switches_and_persists_including_system() {
    use egui::{Theme, ThemePreference};
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let mut editor = Editor::new();
    let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
    for (setting, os, expected) in [
        ("light", Theme::Dark, Theme::Light),
        ("dark", Theme::Light, Theme::Dark),
        ("system", Theme::Light, Theme::Light),
        ("system", Theme::Dark, Theme::Dark),
    ] {
        editor.restore_preferences(&serde_json::json!({"version": 1, "appearance": setting}));
        assert_eq!(editor.preferences()["appearance"], setting);
        for _ in 0..2 {
            ctx.run_ui(
                egui::RawInput {
                    system_theme: Some(os),
                    ..Default::default()
                },
                |ui| {
                    editor.ui_with_layout(ui, &mut host, |_, _, _| {});
                },
            )
            .drop_without_applying_deltas();
        }
        assert_eq!(ctx.theme(), expected);
    }
    assert_eq!(editor.preferred_theme, Some(ThemePreference::System));
    editor.restore_preferences(&serde_json::json!({"version": 1}));
    assert_eq!(editor.preferred_theme, None);
}

#[test]
fn one_search_filters_keyboard_shortcuts_on_a_compact_window() {
    let mut editor = Editor::new();
    editor.settings_open = true;
    editor.settings_tab = Section::Shortcuts;
    editor.settings_search = t("grid-spacing").to_owned();
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(425.0, 410.0));
    ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        },
        |ui| {
            editor.settings_window(ui.ctx()).unwrap();
        },
    )
    .drop_without_applying_deltas();
    assert_eq!(editor.settings_tab, Section::Display);

    editor.settings_search = editor.canvas.keys.label("save-as");
    let mut found = false;
    for _ in 0..3 {
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ui| {
                let rect = editor.settings_window(ui.ctx()).unwrap();
                assert!(rect.width() < screen.width());
            },
        );
        assert_eq!(editor.settings_tab, Section::Shortcuts);
        found |= output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == t("action-save-as") && shape.clip_rect.contains(text.pos)
        ));
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == t("action-open")
        )));
        output.drop_without_applying_deltas();
    }
    assert!(found, "the matching action must remain visible");
}

#[test]
fn grid_spacing_slider_snaps_to_the_four_supported_values() {
    let mut editor = Editor::new();
    let ctx = egui::Context::default();
    let mut frame = |events| {
        let mut rect = egui::Rect::NOTHING;
        ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                rect = editor.grid_spacing_slider(ui).rect;
            },
        )
        .drop_without_applying_deltas();
        (rect, editor.canvas.grid_step)
    };
    let (slider, _) = frame(vec![]);
    for (fraction, expected) in [(0.01, 12), (0.32, 24), (0.68, 36), (0.99, 48)] {
        let pos = egui::pos2(slider.left() + fraction * slider.width(), slider.center().y);
        frame(vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            frame(vec![egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        assert_eq!(frame(vec![]).1, expected);
    }
}

#[test]
fn interface_scale_changes_only_after_selecting_a_dropdown_option() {
    let mut editor = Editor::new();
    let ctx = egui::Context::default();
    let mut frame = |events| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                editor.interface_scale_picker(ui, &ctx);
            },
        );
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        shapes
    };
    let at = |shapes: &[egui::epaint::ClippedShape], label: &str| {
        shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.mesh_bounds.center().to_vec2())
                }
                _ => None,
            })
            .unwrap()
    };
    let mouse = |pos, pressed| egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: egui::Modifiers::NONE,
    };
    let picker = at(&frame(vec![]), "100%");
    frame(vec![egui::Event::PointerMoved(picker)]);
    frame(vec![mouse(picker, true)]);
    frame(vec![mouse(picker, false)]);
    let option = at(&frame(vec![]), "125%");
    frame(vec![egui::Event::PointerMoved(option)]);
    assert!((ctx.zoom_factor() - 1.0).abs() < f32::EPSILON);
    frame(vec![mouse(option, true)]);
    frame(vec![mouse(option, false)]);
    frame(vec![]);
    assert!((ctx.zoom_factor() - 1.25).abs() < f32::EPSILON);
    assert_eq!(editor.preferences()["scale"].as_f64(), Some(1.25));
}

#[test]
fn changing_settings_sections_preserves_window_width() {
    let mut editor = Editor::new();
    editor.settings_open = true;
    let context = egui::Context::default();
    let mut widths = Vec::new();
    let chord = editor.canvas.keys.label("save-as");
    let mut shortcut_is_readable = false;
    for section in [
        Section::Search,
        Section::Shortcuts,
        Section::Display,
        Section::Search,
    ] {
        editor.settings_tab = section;
        for _ in 0..3 {
            let output = context.run_ui(
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
            );
            if section == Section::Shortcuts {
                shortcut_is_readable |= output.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text() == chord && text.galley.rows.len() == 1
                ));
            }
            output.drop_without_applying_deltas();
        }
    }
    for width in &widths[3..] {
        assert!((width - widths[3]).abs() < 2.0, "{widths:?}");
    }
    assert!(shortcut_is_readable);
}

#[test]
fn shortcut_columns_stay_aligned_and_the_binding_opens_the_editor() {
    for [width, height] in [[1200.0, 800.0], [490.0, 470.0]] {
        let mut editor = Editor::new();
        editor.settings_open = true;
        editor.settings_tab = Section::Shortcuts;
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let save = editor.canvas.keys.label("save");
        let save_as = editor.canvas.keys.label("save-as");
        let mut frame = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, height),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let _ = editor.settings_window(ui.ctx());
                },
            );
            let shapes = std::mem::take(&mut output.shapes);
            output.drop_without_applying_deltas();
            shapes
        };
        for _ in 0..3 {
            frame(vec![]);
        }
        let shapes = frame(vec![]);
        let text_rect = |label: &str| {
            shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text)
                        if text.galley.text() == label && shape.clip_rect.contains(text.pos) =>
                    {
                        Some(text.galley.mesh_bounds.translate(text.pos.to_vec2()))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing {label} at width {width}"))
        };
        assert!(
            (text_rect(t("shortcut-action")).left() - text_rect(t("action-save")).left()).abs()
                < 2.0
        );
        assert!((text_rect(&save).left() - text_rect(&save_as).left()).abs() < 2.0);
        assert!(text_rect(t("shortcut-binding")).left() < text_rect(&save).left());
        assert!(text_rect(&save).left() - text_rect(t("shortcut-binding")).left() < 16.0);
        let pencil = icons::EDIT.to_string();
        text_rect(&pencil);
        let pos = text_rect(&save).center();
        frame(vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            frame(vec![egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        assert!(editor.shortcut_edit.is_some());
    }
}

#[test]
fn settings_title_uses_strong_text_and_general_exposes_the_preview_preference() {
    let mut editor = Editor::new();
    editor.settings_open = true;
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    let expected = ctx.style_of(ctx.theme()).visuals.strong_text_color();
    let mut seen_title = false;
    let mut seen_names = false;
    let mut seen_preview = false;
    for _ in 0..3 {
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            editor.settings_window(ui.ctx()).unwrap();
        });
        for shape in &output.shapes {
            if let egui::Shape::Text(text) = &shape.shape {
                if text.galley.text() == t("settings-title") {
                    seen_title = true;
                    assert!(
                        text.galley
                            .job
                            .sections
                            .iter()
                            .any(|section| section.format.color == expected)
                    );
                }
                seen_names |= text.galley.text() == t("preview-names-only");
                seen_preview |= text.galley.text() == t("preview-node-cards");
            }
        }
        output.drop_without_applying_deltas();
    }
    assert!(seen_title && seen_names && seen_preview);
}
