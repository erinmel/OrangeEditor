//! Optional desktop styling. Embedding hosts retain control of their own theme.
use crate::{CONTROL_RADIUS, Density};

pub fn apply_desktop_style(ctx: &egui::Context) {
    for theme in [egui::Theme::Dark, egui::Theme::Light] {
        let mut style = (*ctx.style_of(theme)).clone();
        Density::Compact.apply_style(&mut style);
        style.spacing.interact_size.x = 32.0;
        let rgb = egui::Color32::from_rgb;
        let [
            panel,
            dialog,
            inset,
            layer,
            text,
            muted,
            border,
            primary,
            focus,
        ] = match theme {
            egui::Theme::Dark => [
                rgb(26, 31, 40),
                rgb(32, 38, 50),
                rgb(18, 23, 32),
                rgb(39, 46, 59),
                rgb(227, 233, 242),
                rgb(164, 174, 191),
                rgb(67, 77, 94),
                rgb(56, 83, 118),
                rgb(143, 187, 234),
            ],
            egui::Theme::Light => [
                rgb(237, 241, 247),
                rgb(249, 251, 254),
                rgb(226, 233, 242),
                rgb(231, 237, 245),
                rgb(30, 42, 60),
                rgb(77, 92, 113),
                rgb(171, 185, 204),
                rgb(189, 216, 248),
                rgb(46, 96, 155),
            ],
        };
        let visuals = &mut style.visuals;
        visuals.panel_fill = panel;
        visuals.window_corner_radius = 12.into();
        visuals.menu_corner_radius = 8.into();
        visuals.window_fill = dialog;
        visuals.window_stroke = egui::Stroke::new(1.0, border);
        visuals.extreme_bg_color = inset;
        visuals.faint_bg_color = layer;
        visuals.weak_text_color = Some(muted);
        visuals.selection.bg_fill = primary;
        visuals.selection.stroke = egui::Stroke::new(1.0, focus);
        let widgets = &mut visuals.widgets;
        for state in [
            &mut widgets.noninteractive,
            &mut widgets.inactive,
            &mut widgets.hovered,
            &mut widgets.active,
            &mut widgets.open,
        ] {
            state.fg_stroke.color = text;
            state.corner_radius = CONTROL_RADIUS.into();
            state.expansion = 0.0;
        }
        widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, border);
        widgets.inactive.bg_fill = layer;
        widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
        widgets.inactive.bg_stroke = egui::Stroke::NONE;
        widgets.hovered.bg_fill = layer;
        widgets.hovered.weak_bg_fill = layer;
        widgets.hovered.bg_stroke = egui::Stroke::NONE;
        widgets.active.bg_fill = primary;
        widgets.active.weak_bg_fill = primary;
        widgets.active.bg_stroke = egui::Stroke::new(1.0, focus);
        widgets.open.bg_fill = dialog;
        widgets.open.weak_bg_fill = dialog;
        widgets.open.bg_stroke = egui::Stroke::new(1.0, border);
        ctx.set_style_of(theme, style);
    }
}
