use super::*;

fn context() -> egui::Context {
    let context = egui::Context::default();
    #[cfg(feature = "icons")]
    orange_ui_controls::icons::install(&context);
    context
}

#[test]
fn theme_tile_and_density_apply_to_the_gallery() {
    let mut gallery = Gallery::default();
    let context = context();
    UiTokens::install(&context, ThemeMode::System, Density::Compact);

    let mut frame = |events| {
        context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(920.0, 760.0),
                )),
                events,
                ..Default::default()
            },
            |ui| gallery.canvas(ui),
        )
    };
    for label in ["Bubblegum", "Comfy"] {
        let output = frame(vec![]);
        let position = output.shapes.iter().find_map(|clipped| {
            if let egui::Shape::Text(text) = &clipped.shape
                && text.galley.text() == label
            {
                Some(text.pos + text.galley.size() * 0.5)
            } else {
                None
            }
        });
        output.drop_without_applying_deltas();
        let pos = position.expect("appearance control should be visible");
        frame(vec![egui::Event::PointerMoved(pos)]).drop_without_applying_deltas();
        for pressed in [true, false] {
            frame(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }])
            .drop_without_applying_deltas();
        }
    }
    let tokens = UiTokens::resolve(&context);
    assert_eq!(tokens.density, Density::Comfy);
    assert_eq!(tokens.theme.surface, Theme::BUBBLEGUM.surface);
}

#[test]
fn canvas_shows_every_component_and_palette_without_input() {
    for width in [240.0, 920.0] {
        let mut gallery = Gallery::default();
        let context = context();
        UiTokens::install(&context, ThemeMode::System, Density::Compact);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 12000.0),
            )),
            ..Default::default()
        };
        let output = context.run_ui(input, |ui| gallery.canvas(ui));
        let labels: std::collections::HashSet<_> = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.clone()),
                _ => None,
            })
            .collect();
        output.drop_without_applying_deltas();
        for label in ENTRIES
            .iter()
            .map(|entry| entry.title)
            .chain(["System"])
            .chain(PALETTES.map(|(label, _)| label))
            .chain([
                "Parametric EQ and trumpet spectrum",
                "Session track order",
                "Sortable track table",
                "Audio files and folders",
                "Navigation, menus and search",
                "Selection controls",
                "Feedback and loading",
            ])
        {
            assert!(
                labels.contains(label),
                "{label} missing at {width} points. Labels: {labels:?}"
            );
        }
    }
}

#[test]
fn comfy_card_covers_rotary_knobs_and_reserves_height_for_the_next_row() {
    let context = context();
    UiTokens::install(&context, ThemeMode::Dark, Density::Comfy);
    let output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(780.0, 1600.0),
            )),
            ..Default::default()
        },
        |ui| {
            let tokens = UiTokens::resolve(ui.ctx());
            tokens.scope(ui, None, |ui, tokens| {
                let mut gallery = Gallery::default();
                card_grid(ui, 2, 300.0, 2, |ui, index| {
                    gallery.entry(ui, tokens, index)
                });
                ui.label("Next row");
            });
        },
    );
    let text = |label| {
        output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) if text.galley.text() == label => Some((
                    text.galley.mesh_bounds.translate(text.pos.to_vec2()),
                    clipped.clip_rect,
                )),
                _ => None,
            })
            .expect("gallery fixture should paint its label")
    };
    let (row, _) = text("Next row");
    for label in [
        "Mix (%)",
        "Trim (dB)",
        "Pan",
        "Phase (°)",
        "C",
        "35.0",
        "-4",
    ] {
        let (bounds, clip) = text(label);
        assert!(clip.contains_rect(bounds), "{label} was clipped");
        assert!(bounds.bottom() < row.top(), "{label} exceeded the card row");
    }
    output.drop_without_applying_deltas();
}

#[test]
fn catalog_ids_are_unique_and_entries_fit_preview_widths() {
    let ids: std::collections::HashSet<_> = ENTRIES.iter().map(|entry| entry.id).collect();
    assert_eq!(ids.len(), ENTRIES.len());
    assert!(ids.contains("1.2.1"));

    let ctx = egui::Context::default();
    for &entry in ENTRIES {
        for width in [140.0, 360.0] {
            for density in [Density::Compact, Density::Comfy] {
                for theme in [
                    Theme::LIGHT,
                    Theme::DARK,
                    Theme::high_contrast_light(),
                    Theme::high_contrast_dark(),
                ] {
                    for state in std::iter::once(State::Normal)
                        .chain(entry.alternate().map(|(_, state)| state))
                    {
                        let input = egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, 900.0),
                            )),
                            ..Default::default()
                        };
                        ctx.run_ui(input, |ui| {
                            let bounds = ui.available_rect_before_wrap();
                            UiTokens { theme, density }.scope(ui, None, |ui, tokens| {
                                let response = ui
                                    .scope(|ui| {
                                        catalog::preview(
                                            ui,
                                            tokens,
                                            entry,
                                            state,
                                            bounds.width(),
                                            &mut Fixture::sample(),
                                        )
                                    })
                                    .response;
                                assert!(
                                    response.rect.right() <= bounds.right() + 0.5,
                                    "{} at {width} in {density:?}: {:?} exceeds {bounds:?}",
                                    entry.id,
                                    response.rect
                                );
                            });
                        })
                        .drop_without_applying_deltas();
                    }
                }
            }
        }
    }
}

#[test]
fn profile_theme_switch() {
    let ctx = context();
    let mut gallery = Gallery::default();
    let mut elapsed = Vec::with_capacity(120);
    for i in 0..140 {
        let start = std::time::Instant::now();
        gallery.palette = i % PALETTES.len() + 1;
        UiTokens::install(
            &ctx,
            ThemeMode::Custom(PALETTES[gallery.palette - 1].1),
            gallery.density,
        );
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(920.0, 760.0),
                )),
                ..Default::default()
            },
            |ui| gallery.canvas(ui),
        );
        std::hint::black_box(
            ctx.tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point),
        );
        output.drop_without_applying_deltas();
        if i >= 20 {
            elapsed.push(start.elapsed().as_micros());
        }
    }
    elapsed.sort_unstable();
    println!(
        "120 palette switches, CPU microseconds: median {}, p95 {}",
        elapsed[60], elapsed[114]
    );
}
