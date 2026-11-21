use orange_editor_ui::{Editor, MenuTarget, i18n, icons};
use orange_host::{Host, HostOptions};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
fn a_downstream_app_can_supply_layout_theme_command_icons_and_text() {
    assert!(i18n::add_application_catalogs(&[(
        "en",
        "properties = Custom inspector",
    )]));
    let (mut host, _) = Host::new(HostOptions::default()).unwrap();
    let mut editor = Editor::new();
    editor.open_untitled(&mut host, orange_host::new_document().unwrap());
    let node = editor.add_node(&mut host, "core/whole-number").unwrap();
    editor.open_menu(MenuTarget::Node(node), [0.0, 0.0], egui::pos2(90.0, 100.0));

    let ctx = egui::Context::default();
    ctx.set_theme(egui::Theme::Dark);
    let background = egui::Color32::from_rgb(15, 33, 51);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.visuals.panel_fill = background;
    ctx.set_style_of(egui::Theme::Dark, style);

    let paints = Arc::new(AtomicUsize::new(0));
    let paint_count = Arc::clone(&paints);
    let menu_paints = Arc::new(AtomicUsize::new(0));
    let menu_paint_count = Arc::clone(&menu_paints);
    icons::set_command_painter(
        &ctx,
        Some(Arc::new(move |painter, rect, icon, color| {
            match icon {
                icons::SAVE => paint_count.fetch_add(1, Ordering::Relaxed),
                icons::PROPERTIES => menu_paint_count.fetch_add(1, Ordering::Relaxed),
                _ => return false,
            };
            painter.circle_filled(rect.center(), 5.0, color);
            true
        })),
    );

    let mut saw_label = false;
    for frame in 0..4 {
        let output = ctx.run_ui(
            egui::RawInput {
                time: Some(f64::from(frame) * 0.1),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| {
                editor.ui_with_layout(ui, &mut host, |editor, ui, host| {
                    assert_eq!(ui.visuals().panel_fill, background);
                    editor.toolbar(ui, host);
                    egui::Panel::right("host-inspector")
                        .show(ui, |ui| editor.right_panel(ui, host));
                    egui::CentralPanel::default().show(ui, |ui| editor.canvas_panel(ui, host));
                });
            },
        );
        saw_label |= output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Custom inspector")
        });
        output.drop_without_applying_deltas();
    }
    assert!(paints.load(Ordering::Relaxed) > 0);
    assert!(menu_paints.load(Ordering::Relaxed) > 0);
    assert!(saw_label);
    icons::set_command_painter(&ctx, None);
}
