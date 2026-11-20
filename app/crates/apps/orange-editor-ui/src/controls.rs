//! Shared control roles. Colors follow the host theme and the surrounding surface.
use crate::{i18n::t, icons};
use egui::{Button, ComboBox, InnerResponse, Response, Stroke, Ui};

#[derive(Debug, Clone, Copy)]
/// The surface that provides the theme colors for a control.
pub enum Surface {
    /// Editor panels and toolbars.
    Panel,
    /// Dialog and popup surfaces.
    Dialog,
    /// The inset surface of a canvas card.
    Card,
}

#[derive(Debug, Clone, Copy)]
/// A button's role, which determines its fill and outline.
pub enum Action {
    /// The main confirmation.
    Primary,
    /// An ordinary framed command.
    Secondary,
    /// A command that removes data.
    Destructive,
    /// A persistent on/off control or filter.
    Toggle(bool),
    /// A compact menu item with hover and focus states.
    Menu,
}

/// A dropdown with a surface fill and a centered chevron.
pub trait Dropdown {
    /// Draws the dropdown using this editor's theme for `surface`.
    fn show_select<R>(
        self,
        ui: &mut Ui,
        surface: Surface,
        contents: impl FnOnce(&mut Ui) -> R,
    ) -> InnerResponse<Option<R>>;
}

impl Dropdown for ComboBox<'_> {
    fn show_select<R>(
        self,
        ui: &mut Ui,
        surface: Surface,
        contents: impl FnOnce(&mut Ui) -> R,
    ) -> InnerResponse<Option<R>> {
        with_style(ui, surface, Action::Secondary, |ui| {
            let focus = ui.visuals().selection.stroke;
            let widgets = &mut ui.visuals_mut().widgets;
            for state in [
                &mut widgets.inactive,
                &mut widgets.hovered,
                &mut widgets.active,
                &mut widgets.open,
            ] {
                state.bg_stroke = Stroke::NONE;
                state.expansion = 0.0;
            }
            widgets.active.bg_stroke = focus;
            widgets.open.bg_stroke = focus;
            self.icon(|ui, rect, visuals, open| {
                let center = rect.center();
                let size = rect.width().min(rect.height());
                let x = size * 0.26;
                let y = size * if open { -0.12 } else { 0.12 };
                ui.painter().add(egui::Shape::line(
                    vec![
                        center + egui::vec2(-x, -y),
                        center + egui::vec2(0.0, y),
                        center + egui::vec2(x, -y),
                    ],
                    Stroke::new((size * 0.09).max(1.0), visuals.fg_stroke.color),
                ));
            })
            .show_ui(ui, contents)
        })
    }
}

/// Draws an action with the shared theme for `surface`.
pub fn button(
    ui: &mut Ui,
    surface: Surface,
    action: Action,
    enabled: bool,
    button: Button<'_>,
) -> Response {
    with_style(ui, surface, action, |ui| {
        ui.add_enabled(
            enabled,
            button
                .frame(true)
                .frame_when_inactive(!matches!(action, Action::Menu)),
        )
    })
}

fn with_style<R>(
    ui: &mut Ui,
    surface: Surface,
    action: Action,
    draw: impl FnOnce(&mut Ui) -> R,
) -> R {
    let previous = ui.style().clone();
    style(ui, surface, action);
    let response = draw(ui);
    ui.set_style(previous);
    response
}

fn style(ui: &mut Ui, surface: Surface, action: Action) {
    let visuals = ui.visuals_mut();
    let background = match surface {
        Surface::Panel => visuals.panel_fill,
        Surface::Dialog | Surface::Card => visuals.window_fill,
    };
    let text = visuals.strong_text_color();
    let accent = visuals.selection.stroke.color;
    let primary = visuals.selection.bg_fill;
    let danger = visuals.error_fg_color;
    let neutral_fill = match surface {
        Surface::Card => visuals.extreme_bg_color,
        Surface::Panel | Surface::Dialog => background.lerp_to_gamma(text, 0.055),
    };
    let (fill, outline) = match action {
        Action::Primary => (primary, accent),
        Action::Secondary => (neutral_fill, background.lerp_to_gamma(text, 0.23)),
        Action::Destructive => (neutral_fill, background.lerp_to_gamma(danger, 0.28)),
        Action::Toggle(false) | Action::Menu => (background, background),
        Action::Toggle(true) => (primary, primary.lerp_to_gamma(accent, 0.16)),
    };
    visuals.button_frame = true;
    let widgets = &mut visuals.widgets;
    for (state, tint, border) in [
        (&mut widgets.inactive, 0.0, outline),
        (
            &mut widgets.hovered,
            0.07,
            outline.lerp_to_gamma(text, 0.12),
        ),
        (&mut widgets.active, 0.10, accent),
        (&mut widgets.open, 0.10, accent),
    ] {
        state.bg_fill = fill.lerp_to_gamma(text, tint);
        state.weak_bg_fill = state.bg_fill;
        state.bg_stroke = Stroke::new(1.0, border);
    }
    widgets.hovered.fg_stroke.color = text;
    widgets.active.fg_stroke.color = text;
    widgets.open.fg_stroke.color = text;
    widgets.open.expansion = 0.0;
}

/// Search row with a label for assistive technology and a clear action that
/// returns focus to the field.
pub fn search(ui: &mut Ui, salt: &str, query: &mut String, hint: &str) {
    let id = ui.make_persistent_id(salt);
    ui.horizontal(|ui| {
        style(ui, Surface::Panel, Action::Secondary);
        let hint_color = ui
            .visuals()
            .strong_text_color()
            .lerp_to_gamma(ui.visuals().extreme_bg_color, 0.25);
        let show_clear = !query.is_empty();
        let clear_width = 30.0;
        let reserved = if show_clear {
            clear_width + ui.spacing().item_spacing.x
        } else {
            0.0
        };
        let response = ui.add(
            egui::TextEdit::singleline(query)
                .id(id)
                .hint_text(egui::RichText::new(hint).color(hint_color))
                .prefix(icons::text(icons::SEARCH).color(hint_color))
                .desired_width((ui.available_width() - reserved).max(0.0)),
        );
        if show_clear {
            let clear = ui
                .add(Button::new(icons::text(icons::CLOSE)).min_size(egui::vec2(clear_width, 0.0)))
                .on_hover_text(t("clear-search"));
            clear.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, true, t("clear-search"))
            });
            if clear.clicked() {
                query.clear();
                response.request_focus();
            }
        }
    });
}
