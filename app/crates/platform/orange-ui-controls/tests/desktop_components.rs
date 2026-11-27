use egui::{Context, Event, Key, Modifiers, Pos2, RawInput, Rect, Sense, vec2};
use orange_ui_controls::{
    Density, Divider, ThemeMode, UiTokens,
    components::{PasswordLabels, PathPicker, password, switch},
    note::{Note, NoteState, NoteTone},
    pill_segmented,
    theme::Theme,
};

fn input(width: f32, events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 600.0))),
        events,
        ..Default::default()
    }
}

fn key(key: Key) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

fn luminance(color: egui::Color32) -> f32 {
    let [r, g, b, _] = egui::Rgba::from(color).to_array();
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn contrast(a: egui::Color32, b: egui::Color32) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn high_contrast_palettes_keep_text_and_status_inks_above_seven_to_one() {
    for theme in [
        Theme::high_contrast_dark(),
        Theme::high_contrast_light(),
        Theme::MIDNIGHT_CONTRAST,
    ] {
        for surface in [theme.surface, theme.surface_elevated] {
            for ink in [
                theme.text,
                theme.text_muted,
                theme.info,
                theme.tip,
                theme.warning,
                theme.danger,
            ] {
                assert!(contrast(surface, ink) >= 7.0, "{ink:?} on {surface:?}");
            }
            assert!(contrast(surface, theme.focus_ring.color) >= 3.0);
            assert!(contrast(surface, theme.border_strong.color) >= 3.0);
        }
        assert!(contrast(theme.primary, theme.on_primary) >= 7.0);
        assert_eq!(theme.focus_ring.width, 2.0);
        assert_eq!(theme.elevation, egui::Shadow::NONE);
    }
}

#[test]
fn system_palette_follows_host_contrast_mode_and_density_override_stays_local() {
    let ctx = Context::default();
    UiTokens::install(&ctx, ThemeMode::System, Density::Compact);
    for (system_theme, expected) in [
        (egui::Theme::Light, Theme::LIGHT),
        (egui::Theme::Dark, Theme::DARK),
    ] {
        let input = RawInput {
            system_theme: Some(system_theme),
            ..Default::default()
        };
        ctx.run_ui(input, |ui| {
            let tokens = UiTokens::resolve(ui.ctx());
            assert_eq!(tokens.theme.surface, expected.surface);
            tokens.scope(ui, Some(Density::Comfy), |ui, comfy| {
                assert_eq!(comfy.theme.surface, expected.surface);
                assert_eq!(ui.spacing().interact_size.y, Density::Comfy.row_height());
                assert_eq!(UiTokens::resolve(ui.ctx()).density, Density::Compact);
            });
        })
        .drop_without_applying_deltas();
    }
}

#[test]
fn composite_controls_fit_narrow_panels_and_keep_intrinsic_width_on_wide_panels() {
    for width in [140.0, 540.0] {
        for density in [Density::Compact, Density::Comfy] {
            Context::default()
                .run_ui(input(width, vec![]), |ui| {
                    let tokens = UiTokens::resolve(ui.ctx());
                    tokens.scope(ui, Some(density), |ui, tokens| {
                        let bounds = ui.available_rect_before_wrap();
                        let label = ui.label("Billing");
                        let pill = pill_segmented(
                            ui,
                            tokens,
                            &label,
                            &mut 0,
                            &[(0, "Annual"), (1, "Monthly"), (2, "Quarterly")],
                        );
                        let switch =
                            switch(ui, tokens, &mut false, "Enable background synchronization");
                        let text = password(
                            ui,
                            tokens,
                            &mut String::new(),
                            &mut false,
                            PasswordLabels {
                                field: "Password",
                                reveal: "Show password",
                                hide: "Hide password",
                            },
                            None,
                        );
                        let path = PathPicker {
                            name: "Choose folder",
                            display: "C:/Projects/Audio/Recording/Long folder path",
                            full_selection: "C:/Projects/Audio/Recording/Long folder path",
                            error: Some("Folder unavailable"),
                        }
                        .show(ui, tokens);
                        let note = Note {
                            tone: NoteTone::Warning,
                            title: "Background synchronization paused",
                            body: "Changes sync when your connection is available.",
                            collapse_labels: Some(["Collapse", "Expand"]),
                            dismiss_label: Some("Dismiss"),
                        }
                        .show(ui, tokens, &mut NoteState::default())
                        .unwrap();
                        for rect in [pill.rect, switch.rect, text.rect, path.rect, note.rect] {
                            assert!(
                                rect.right() <= bounds.right() + 0.5,
                                "{density:?} at {width}: {rect:?} exceeds {bounds:?}"
                            );
                        }
                        assert_eq!(switch.rect.height(), density.row_height());
                        assert_eq!(text.rect.height(), density.row_height());
                        assert_eq!(path.rect.height(), density.row_height());
                        if width > 140.0 {
                            assert!(pill.rect.width() < width * 0.7);
                            assert!(switch.rect.width() < width * 0.7);
                        } else {
                            assert!(pill.rect.height() >= 3.0 * density.row_height());
                        }
                    });
                })
                .drop_without_applying_deltas();
        }
    }
}

#[test]
fn controls_support_keyboard_activation_and_pill_arrows() {
    let ctx = Context::default();
    let mut enabled = false;
    let mut selected = 0;
    let mut switch_id = egui::Id::NULL;
    let mut first_segment = egui::Id::NULL;
    for (frame, events) in [vec![], vec![key(Key::Space)], vec![key(Key::ArrowRight)]]
        .into_iter()
        .enumerate()
    {
        let output = ctx.run_ui(input(540.0, events), |ui| {
            let tokens = UiTokens::resolve(ui.ctx());
            tokens.scope(ui, None, |ui, tokens| {
                let response = switch(ui, tokens, &mut enabled, "Enable sync");
                switch_id = response.id;
                let title = ui.label("Period");
                pill_segmented(
                    ui,
                    tokens,
                    &title,
                    &mut selected,
                    &[(0, "Monthly"), (1, "Yearly")],
                );
            });
        });
        if frame == 0 {
            ctx.memory_mut(|memory| memory.request_focus(switch_id));
            // The next Tab from the switch reaches the first radio in its group.
        } else if frame == 1 {
            assert!(enabled);
            // Establish ordinary keyboard focus via the next Tab in a separate pass.
            ctx.run_ui(input(540.0, vec![key(Key::Tab)]), |ui| {
                let tokens = UiTokens::resolve(ui.ctx());
                tokens.scope(ui, None, |ui, tokens| {
                    switch(ui, tokens, &mut enabled, "Enable sync");
                    let title = ui.label("Period");
                    pill_segmented(
                        ui,
                        tokens,
                        &title,
                        &mut selected,
                        &[(0, "Monthly"), (1, "Yearly")],
                    );
                });
            })
            .drop_without_applying_deltas();
            first_segment = ctx.memory(|memory| memory.focused()).unwrap();
        }
        output.drop_without_applying_deltas();
    }
    assert_ne!(first_segment, switch_id);
    assert_eq!(selected, 1);
}

#[test]
fn divider_keeps_section_gap_and_parent_padding() {
    Context::default()
        .run_ui(input(140.0, vec![]), |ui| {
            UiTokens::resolve(ui.ctx()).scope(ui, None, |ui, tokens| {
                egui::Frame::new()
                    .inner_margin(egui::Margin::same(12))
                    .show(ui, |ui| {
                        let bounds = ui.available_rect_before_wrap();
                        let (before, _) = ui.allocate_exact_size(vec2(30.0, 12.0), Sense::hover());
                        let divider = Divider::Section.show(ui, tokens);
                        let (after, _) = ui.allocate_exact_size(vec2(30.0, 12.0), Sense::hover());
                        assert_eq!(divider.x_range(), bounds.x_range());
                        let expected = tokens.density.spacing().md + 0.5;
                        assert_eq!(divider.center().y - before.bottom(), expected);
                        assert_eq!(after.top() - divider.center().y, expected);
                    });
            });
        })
        .drop_without_applying_deltas();
}

#[test]
fn accesskit_exposes_named_roles_checked_selection_and_note_expansion() {
    use egui::accesskit::Toggled;
    let ctx = Context::default();
    ctx.enable_accesskit();
    let mut checked = true;
    let mut password_value = String::from("sample");
    let mut selected = 0;
    let mut note_state = NoteState::default();
    let mut output = ctx.run_ui(input(540.0, vec![]), |ui| {
        UiTokens::resolve(ui.ctx()).scope(ui, None, |ui, tokens| {
            switch(ui, tokens, &mut checked, "Enable sync");
            let label = ui.label("Period");
            pill_segmented(
                ui,
                tokens,
                &label,
                &mut selected,
                &[(0, "Monthly"), (1, "Yearly")],
            );
            password(
                ui,
                tokens,
                &mut password_value,
                &mut false,
                PasswordLabels {
                    field: "Password",
                    reveal: "Show password",
                    hide: "Hide password",
                },
                Some("Enter a password"),
            );
            Note {
                tone: NoteTone::Info,
                title: "Sync",
                body: "Works offline.",
                collapse_labels: Some(["Collapse", "Expand"]),
                dismiss_label: Some("Dismiss"),
            }
            .show(ui, tokens, &mut note_state);
        });
    });
    let update = output.platform_output.accesskit_update.take();
    output.drop_without_applying_deltas();
    let update = update.expect("egui emitted the accessibility tree");
    let node = |label, role| {
        update
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label) && n.role() == role)
            .map(|(_, n)| n)
            .expect("the control appears with a localized name and expected role")
    };
    assert_eq!(
        node("Enable sync", egui::Role::CheckBox).toggled(),
        Some(Toggled::True)
    );
    assert_eq!(
        node("Monthly", egui::Role::RadioButton).toggled(),
        Some(Toggled::True)
    );
    assert_eq!(
        node("Show password", egui::Role::CheckBox).toggled(),
        Some(Toggled::False)
    );
    let field = node("Password", egui::Role::PasswordInput);
    assert_eq!(field.invalid(), Some(egui::accesskit::Invalid::True));
    assert_eq!(field.description(), Some("Enter a password"));
    assert_eq!(
        node("Collapse", egui::Role::Button).is_expanded(),
        Some(true)
    );
    node("Dismiss", egui::Role::Button);
}

#[test]
fn context_menu_opens_from_keyboard_and_arrows_skip_disabled_choices() {
    use orange_ui_controls::components::{MenuItem, MenuLayout, MenuTrigger, menu};
    let ctx = Context::default();
    let mut triggered = None;
    let mut trigger_id = egui::Id::NULL;
    for (frame, events) in [
        vec![],
        vec![key(Key::Enter)],
        vec![],
        vec![key(Key::ArrowDown)],
        vec![key(Key::Space)],
    ]
    .into_iter()
    .enumerate()
    {
        ctx.run_ui(input(540.0, events), |ui| {
            let tokens = UiTokens::resolve(ui.ctx());
            let trigger = ui.button("Actions");
            trigger_id = trigger.id;
            let items = [
                MenuItem {
                    id: 0,
                    label: "Copy",
                    icon: None,
                    shortcut: None,
                    enabled: true,
                },
                MenuItem {
                    id: 1,
                    label: "Delete",
                    icon: None,
                    shortcut: None,
                    enabled: false,
                },
                MenuItem {
                    id: 2,
                    label: "Rename",
                    icon: None,
                    shortcut: None,
                    enabled: true,
                },
            ];
            triggered = menu(
                ui,
                tokens,
                &trigger,
                MenuTrigger::Context,
                MenuLayout::List,
                &items,
            );
        })
        .drop_without_applying_deltas();
        if frame == 0 {
            ctx.memory_mut(|memory| memory.request_focus(trigger_id));
        }
    }
    assert_eq!(triggered, Some(2));
    assert_eq!(ctx.memory(|memory| memory.focused()), Some(trigger_id));
}
