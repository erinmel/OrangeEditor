//! Optional desktop styling. Embedding hosts retain control of their own theme.
pub fn apply(ctx: &egui::Context) {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let mut style = (*ctx.style_of(theme)).clone();
        style.spacing.button_padding = egui::vec2(8.0, 5.0);
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.interact_size = egui::vec2(32.0, 28.0);
        if style.visuals.dark_mode {
            let rgb = egui::Color32::from_rgb;
            style.visuals.panel_fill = rgb(28, 32, 41);
            style.visuals.window_fill = rgb(32, 38, 54);
            style.visuals.extreme_bg_color = rgb(19, 23, 33);
            style.visuals.faint_bg_color = rgb(36, 41, 52);
            style.visuals.selection.bg_fill = rgb(52, 73, 97);
            style.visuals.selection.stroke = egui::Stroke::new(1.0, rgb(150, 190, 231));
            let widgets = &mut style.visuals.widgets;
            widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, rgb(61, 64, 73));
            widgets.inactive.bg_fill = rgb(43, 46, 52);
            widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
            widgets.inactive.bg_stroke = egui::Stroke::new(1.0, rgb(64, 68, 77));
            widgets.inactive.fg_stroke = egui::Stroke::new(1.0, rgb(201, 205, 214));
            widgets.hovered.bg_fill = rgb(57, 61, 70);
            widgets.hovered.weak_bg_fill = rgb(57, 61, 70);
            widgets.hovered.bg_stroke = egui::Stroke::new(1.0, rgb(108, 126, 150));
            widgets.active.bg_fill = rgb(50, 66, 86);
            widgets.active.weak_bg_fill = rgb(50, 66, 86);
        }
        ctx.set_style_of(theme, style);
    }
}
