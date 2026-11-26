use super::*;

#[test]
fn settings_and_shortcuts_search_accept_localized_and_english_terms() {
    let mut editor = Editor::new();
    // Each search is given as typed correctly, then with a typo or other separators.
    for (saved, tag, display, shortcut) in [
        (
            "es_mx",
            "es-MX",
            ["cuadricula spacing", "cuadricual-spacing"],
            ["archivos save", "archvios save"],
        ),
        (
            "ja_jp",
            "ja-JP",
            ["グリッド spacing", "グリッド_spacing"],
            ["ファイル save", "ファイル-save"],
        ),
        (
            "en",
            "en-US",
            ["grid spacing", "grid_spacing"],
            ["files save", "fiels-save"],
        ),
    ] {
        editor.restore_preferences(&serde_json::json!({ "version": 1, "language": saved }));
        assert_eq!(editor.language(), tag);
        assert_eq!(editor.preferences()["language"], tag);
        for typed in display {
            let query = orange_host::SearchQuery::new(typed);
            assert!(
                Section::Display.relevance(&query).is_some(),
                "{tag}: {typed}"
            );
            assert!(
                Section::Search.relevance(&query).is_none(),
                "{tag}: {typed}"
            );
        }
        for typed in shortcut {
            assert!(
                editor.shortcut_search_rank(typed).is_some(),
                "{tag}: {typed}"
            );
        }
    }
}

#[test]
fn appearance_shows_both_theme_palettes_without_changing_the_active_theme() {
    let ctx = egui::Context::default();
    theme::apply(&ctx);
    ctx.set_theme(egui::Theme::Dark);
    let mut editor = Editor::new();
    let light_panel = ctx.style_of(egui::Theme::Light).visuals.panel_fill;
    let colors = [egui::Theme::Light, egui::Theme::Dark].map(|mode| {
        let style = ctx.style_of(mode);
        (style.visuals.window_fill, style.visuals.extreme_bg_color)
    });
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        editor.appearance_picker(ui, &ctx);
    });
    let shapes = std::mem::take(&mut output.shapes);
    output.drop_without_applying_deltas();
    for (window, field) in colors {
        let has_fill = |color| {
            shapes
                .iter()
                .any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == color))
        };
        assert!(has_fill(window) && has_fill(field));
    }
    assert!(shapes.iter().any(|shape| matches!(
        &shape.shape,
        egui::Shape::Path(path) if path.points.len() == 3 && path.fill == light_panel
    )));
    assert_eq!(ctx.theme(), egui::Theme::Dark);
    assert_eq!(editor.preferred_theme, None);
}

#[test]
fn clicking_a_theme_preview_changes_the_preference() {
    let ctx = egui::Context::default();
    ctx.set_theme(egui::ThemePreference::System);
    let mut editor = Editor::new();
    let frame = |editor: &mut Editor, events| {
        ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| {
                editor.appearance_picker(ui, &ctx);
            },
        )
    };
    let output = frame(&mut editor, Vec::new());
    let at = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == t("appearance-light") => {
                Some(text.pos + text.galley.size() / 2.0)
            }
            _ => None,
        })
        .unwrap();
    output.drop_without_applying_deltas();
    for pressed in [true, false] {
        frame(
            &mut editor,
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        )
        .drop_without_applying_deltas();
    }
    assert_eq!(editor.preferred_theme, Some(egui::ThemePreference::Light));
    assert_eq!(
        ctx.options(|options| options.theme_preference),
        egui::ThemePreference::Light
    );
}

#[test]
fn settings_rows_align_controls_and_stack_when_the_column_would_not_fit() {
    for (width, stacked) in [(340.0, true), (580.0, false)] {
        let ctx = egui::Context::default();
        let mut rows = Vec::new();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_max_width(width);
            for title in ["Size", "A longer preference"] {
                setting_row(ui, title, |ui, label| {
                    rows.push((label.rect, ui.available_rect_before_wrap().min));
                    ui.label("Control");
                });
            }
        });
        output.drop_without_applying_deltas();
        assert!((rows[0].1.x - rows[1].1.x).abs() < 1.0);
        for (label, control) in rows {
            if stacked {
                assert!(control.y >= label.bottom());
            } else {
                assert!(control.x > label.right());
                assert!(control.y < label.bottom());
            }
        }
    }
}

#[test]
fn done_fills_the_sidebar_footer_or_the_compact_footer() {
    for width in [380.0, 960.0] {
        let mut editor = Editor::new();
        editor.settings_open = true;
        editor.settings_tab = Section::Display;
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        icons::install(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 510.0));
        let mut seen = false;
        for _ in 0..3 {
            let mut window = egui::Rect::NOTHING;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ui| {
                    window = editor.settings_window(ui.ctx()).unwrap();
                },
            );
            let shapes = std::mem::take(&mut output.shapes);
            output.drop_without_applying_deltas();
            let done = shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == t("settings-done") => Some((
                    text.galley.mesh_bounds.translate(text.pos.to_vec2()),
                    shape.clip_rect,
                )),
                _ => None,
            });
            if let Some((ink, clip)) = done {
                let candidates = shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Rect(rect) if rect.rect.contains_rect(ink) => Some(rect.rect),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                let button = *candidates
                    .iter()
                    .find(|rect| rect.width() > 80.0 && rect.height() < 35.0)
                    .unwrap_or_else(|| {
                        panic!("width {width}, Done at {ink:?}, surrounding shapes {candidates:?}")
                    });
                assert!(clip.contains_rect(ink), "{width}: {clip:?}, {ink:?}");
                assert!(
                    (SPACE_MEDIUM..=SPACE_MEDIUM * 3.0).contains(&(button.left() - window.left()))
                );
                if width < 540.0 {
                    assert!(
                        (SPACE_MEDIUM..=SPACE_MEDIUM * 3.0)
                            .contains(&(window.right() - button.right()))
                    );
                } else {
                    let sidebar_right = shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Rect(rect)
                                if rect.rect.contains_rect(button)
                                    && rect.rect.height() > 200.0
                                    && rect.rect.width() < window.width() / 2.0 =>
                            {
                                Some(rect.rect.right())
                            }
                            _ => None,
                        })
                        .expect("left sidebar");
                    assert!((sidebar_right - button.right() - SPACE_MEDIUM).abs() < 2.0);
                }
                assert!(window.bottom() - button.bottom() >= SPACE_MEDIUM);
                assert!(
                    button.center().distance(ink.center()) < 3.0,
                    "{width}: {button:?}, {ink:?}"
                );
                seen = true;
                break;
            }
        }
        assert!(seen, "Done was missing at width {width}");
    }
}

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
        orange_testkit::ui::text_center(shapes, label).unwrap()
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
