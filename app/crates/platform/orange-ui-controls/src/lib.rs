//! Shared control roles. Colors follow the host theme and the surrounding surface.
use egui::{Button, ComboBox, InnerResponse, Response, Stroke, Ui};

pub mod color_picker;
pub mod icons;
mod selection;
pub use selection::{StepLabels, continuous_slider, marked_slider, segmented, stepped_slider};

pub const SPACE_SMALL: f32 = 4.0;
pub const SPACE_MEDIUM: f32 = 8.0;
pub const SPACE_LARGE: f32 = 12.0;
pub const CONTROL_HEIGHT: f32 = 28.0;
pub const CONTROL_RADIUS: u8 = 6;

/// Density presets shared by text, icon, and segmented controls.
#[derive(Debug, Clone, Copy, Default)]
pub enum ControlSize {
    Compact,
    #[default]
    Standard,
    Comfortable,
}

impl ControlSize {
    pub const fn height(self) -> f32 {
        match self {
            Self::Compact => 24.0,
            Self::Standard => CONTROL_HEIGHT,
            Self::Comfortable => 36.0,
        }
    }

    pub fn apply(self, spacing: &mut egui::Spacing) {
        spacing.interact_size.y = self.height();
        spacing.button_padding = match self {
            Self::Compact => egui::vec2(SPACE_SMALL, 2.0),
            Self::Standard => egui::vec2(SPACE_MEDIUM, SPACE_SMALL),
            Self::Comfortable => egui::vec2(SPACE_LARGE, 6.0),
        };
    }
}

/// Semantic colors supplied by the host. Setting a theme also affects shared
/// controls drawn inside popups using the same egui context.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub panel: egui::Color32,
    pub dialog: egui::Color32,
    pub inset: egui::Color32,
    pub text: egui::Color32,
    pub muted_text: egui::Color32,
    pub border: egui::Color32,
    pub primary: egui::Color32,
    pub on_primary: egui::Color32,
    pub secondary: egui::Color32,
    pub on_secondary: egui::Color32,
    pub focus: egui::Color32,
    pub danger: egui::Color32,
    pub danger_fill: egui::Color32,
    pub on_danger: egui::Color32,
}

impl Theme {
    /// Applies an override to this context. Pass `None` to follow egui visuals.
    pub fn set(ctx: &egui::Context, theme: Option<Self>) {
        let id = egui::Id::unique("orange-ui-controls-theme");
        ctx.data_mut(|data| match theme {
            Some(theme) => {
                data.insert_temp(id, theme);
            }
            None => data.remove::<Self>(id),
        });
        ctx.request_repaint();
    }

    pub fn of(ui: &Ui) -> Self {
        if let Some(theme) = ui
            .ctx()
            .data(|data| data.get_temp(egui::Id::unique("orange-ui-controls-theme")))
        {
            return theme;
        }
        let visuals = ui.visuals();
        let text = visuals.strong_text_color();
        let primary = visuals.selection.bg_fill;
        let [r, g, b, _] = egui::Rgba::from(primary).to_array();
        let luminance = r * 0.2126 + g * 0.7152 + b * 0.0722;
        Self {
            panel: visuals.panel_fill,
            dialog: visuals.window_fill,
            inset: visuals.extreme_bg_color,
            text,
            muted_text: visuals.weak_text_color(),
            border: visuals.widgets.noninteractive.bg_stroke.color,
            primary,
            on_primary: if luminance > 0.179 {
                egui::Color32::BLACK
            } else {
                egui::Color32::WHITE
            },
            focus: visuals.selection.stroke.color,
            danger: visuals.error_fg_color,
            secondary: visuals.panel_fill.lerp_to_gamma(text, 0.055),
            on_secondary: text,
            danger_fill: visuals
                .panel_fill
                .lerp_to_gamma(visuals.error_fg_color, 0.12),
            on_danger: text,
        }
    }

    pub fn background(self, surface: Surface) -> egui::Color32 {
        match surface {
            Surface::Panel => self.panel,
            Surface::Dialog => self.dialog,
            Surface::Card => self.inset,
        }
    }

    fn selected_fill(self, surface: Surface) -> egui::Color32 {
        self.background(surface).lerp_to_gamma(self.primary, 0.65)
    }
}

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
    /// An ordinary command with a neutral fill.
    Secondary,
    /// A secondary action with a border and the surrounding surface fill.
    Outline,
    /// A quiet inline command.
    Tertiary,
    /// A command that removes data.
    Destructive,
    /// A persistent on/off control or filter.
    Toggle(bool),
    /// A compact menu item with hover and focus states.
    Menu,
}

/// A dropdown with a surface fill and a centered chevron.
pub trait Dropdown {
    /// Draws the dropdown using the host's theme for `surface`.
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
            let focus = Stroke::new(1.0, Theme::of(ui).focus);
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
                let angle = if open { std::f32::consts::PI } else { 0.0 };
                paint_chevron(ui, rect.center(), angle, visuals.fg_stroke.color);
            })
            .show_ui(ui, contents)
        })
    }
}

fn paint_chevron(ui: &Ui, center: egui::Pos2, angle: f32, color: egui::Color32) {
    let rotation = egui::emath::Rot2::from_angle(angle);
    let points =
        [(-3.0, -1.5), (0.0, 1.5), (3.0, -1.5)].map(|(x, y)| center + rotation * egui::vec2(x, y));
    ui.painter()
        .add(egui::Shape::line(points.to_vec(), Stroke::new(1.25, color)));
}

/// Draws a compact disclosure row with native keyboard and expanded-state semantics.
pub fn tree_section<R>(
    ui: &mut Ui,
    surface: Surface,
    header: egui::CollapsingHeader,
    contents: impl FnOnce(&mut Ui) -> R,
) -> egui::CollapsingResponse<R> {
    let theme = Theme::of(ui);
    let body_style = ui.style().clone();
    with_style(ui, surface, Action::Menu, |ui| {
        let spacing = ui.spacing_mut();
        spacing.interact_size.y = 22.0;
        spacing.button_padding.y = 2.0;
        spacing.indent = 17.0;
        let visuals = ui.visuals_mut();
        visuals.collapsing_header_frame = true;
        for state in widget_states(&mut visuals.widgets) {
            state.bg_stroke = Stroke::NONE;
            state.corner_radius = 4.into();
            state.expansion = 0.0;
        }
        visuals.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
        visuals.widgets.active.bg_stroke = Stroke::new(1.0, theme.focus);
        header
            .icon(move |ui, openness, response| {
                let angle = (openness - 1.0) * std::f32::consts::FRAC_PI_2;
                paint_chevron(ui, response.rect.center(), angle, theme.muted_text);
            })
            .show(ui, |ui| {
                ui.set_style(body_style);
                contents(ui)
            })
    })
}

/// Draws an action with the shared theme for `surface`.
pub fn button(
    ui: &mut Ui,
    surface: Surface,
    action: Action,
    enabled: bool,
    button: Button<'_>,
) -> Response {
    let theme = Theme::of(ui);
    let (fill, border, ink) = role_colors(theme, surface, action);
    let background = ui.painter().add(egui::Shape::Noop);
    let response = with_style(ui, surface, action, |ui| {
        for state in widget_states(&mut ui.visuals_mut().widgets) {
            state.bg_stroke = Stroke::NONE;
            state.bg_fill = egui::Color32::TRANSPARENT;
            state.weak_bg_fill = egui::Color32::TRANSPARENT;
            state.fg_stroke.color = ink;
        }
        ui.visuals_mut().override_text_color = Some(ink);
        ui.add_enabled(enabled, button.frame(true))
    });
    let (hover, press) = motion(ui, &response);
    if ui.is_rect_visible(response.rect) {
        let fill = fill.lerp_to_gamma(ink, hover * 0.07 + press * 0.09);
        let quiet = matches!(
            action,
            Action::Menu | Action::Tertiary | Action::Toggle(false)
        );
        let fill = if quiet && !response.has_focus() {
            fill.gamma_multiply(hover.max(press))
        } else {
            fill
        };
        let stroke = if response.has_focus() {
            Stroke::new(1.0, theme.focus)
        } else if quiet {
            Stroke::NONE
        } else {
            Stroke::new(1.0, border)
        };
        let fade = if response.enabled() { 1.0 } else { 0.45 };
        ui.painter().set(
            background,
            egui::epaint::RectShape::new(
                response.rect,
                CONTROL_RADIUS,
                fill.gamma_multiply(fade),
                Stroke::new(stroke.width, stroke.color.gamma_multiply(fade)),
                egui::StrokeKind::Inside,
            ),
        );
    }
    response
}

fn motion(ui: &Ui, response: &Response) -> (f32, f32) {
    let duration = ui.style().animation_time;
    let animate = |key, target| {
        if duration > 0.0 {
            ui.ctx()
                .animate_bool_with_time(response.id.with(key), target, duration)
        } else {
            f32::from(target)
        }
    };
    (
        animate("hover", response.hovered()),
        animate("press", response.is_pointer_button_down_on()),
    )
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

fn role_colors(
    theme: Theme,
    surface: Surface,
    action: Action,
) -> (egui::Color32, egui::Color32, egui::Color32) {
    let background = theme.background(surface);
    let neutral_fill = match surface {
        Surface::Card => theme.inset,
        Surface::Panel => theme.secondary,
        Surface::Dialog => background.lerp_to_gamma(theme.text, 0.055),
    };
    match action {
        Action::Primary => (theme.primary, theme.focus, theme.on_primary),
        Action::Toggle(true) => (theme.selected_fill(surface), theme.focus, theme.text),
        Action::Secondary => (neutral_fill, theme.border, theme.on_secondary),
        Action::Outline => (background, theme.border, theme.text),
        Action::Destructive => (theme.danger_fill, theme.danger, theme.on_danger),
        Action::Tertiary => (background, theme.border, theme.muted_text),
        Action::Toggle(false) | Action::Menu => (background, theme.border, theme.text),
    }
}

/// A Boolean option with a compact checkbox and quiet label.
pub fn toggle(ui: &mut Ui, surface: Surface, value: &mut bool, label: &str) -> Response {
    with_style(ui, surface, Action::Outline, |ui| {
        ControlSize::Compact.apply(ui.spacing_mut());
        ui.checkbox(value, label)
    })
}

/// Draws dialog actions in a recessed footer. Returns Cancel and Confirm in that order.
pub fn dialog_actions(
    ui: &mut Ui,
    cancel: &str,
    confirm: Button<'_>,
    role: Action,
    enabled: bool,
) -> (Response, Response) {
    let theme = Theme::of(ui);
    ui.add_space(SPACE_SMALL);
    footer(ui, theme.dialog.lerp_to_gamma(theme.inset, 0.5), |ui| {
        ui.horizontal_wrapped(|ui| {
            let cancel = button(
                ui,
                Surface::Dialog,
                Action::Outline,
                true,
                Button::new(cancel),
            );
            let confirm = button(ui, Surface::Dialog, role, enabled, confirm);
            (cancel, confirm)
        })
        .inner
    })
}

/// A full-width action area with the same border and insets in windows and dialogs.
pub fn footer<R>(ui: &mut Ui, fill: egui::Color32, contents: impl FnOnce(&mut Ui) -> R) -> R {
    let theme = Theme::of(ui);
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, theme.border))
        .corner_radius(CONTROL_RADIUS)
        .inner_margin(SPACE_MEDIUM)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            contents(ui)
        })
        .inner
}

/// Paints the surface behind an existing, interactive list row.
pub fn list_row(ui: &Ui, surface: Surface, row: &Response, selected: bool) {
    if !ui.is_rect_visible(row.rect) {
        return;
    }
    let theme = Theme::of(ui);
    let background = theme.background(surface);
    let base = if selected {
        background.lerp_to_gamma(theme.primary, 0.55)
    } else {
        theme.secondary
    };
    let (hover, press) = motion(ui, row);
    let fill = base.lerp_to_gamma(theme.text, hover * 0.06 + press * 0.06);
    let border = if selected || row.has_focus() {
        theme.focus
    } else {
        theme.border
    };
    ui.painter().rect(
        row.rect,
        5,
        fill,
        Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );
}

fn widget_states(widgets: &mut egui::style::Widgets) -> [&mut egui::style::WidgetVisuals; 4] {
    [
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ]
}

fn style(ui: &mut Ui, surface: Surface, action: Action) {
    let theme = Theme::of(ui);
    let (fill, outline, text) = role_colors(theme, surface, action);
    let accent = theme.focus;
    let visuals = ui.visuals_mut();
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
        state.fg_stroke.color = text;
    }
    widgets.open.expansion = 0.0;
}

/// Search row with a label for assistive technology and a clear action that
/// returns focus to the field.
pub fn search(
    ui: &mut Ui,
    salt: &str,
    query: &mut String,
    hint: &str,
    search_icon: egui::Atom<'static>,
    clear_icon: egui::Atom<'_>,
    clear_label: &str,
) -> Response {
    let id = ui.make_persistent_id(salt);
    let clear_id = egui::IdSalt::new("clear-search");
    let theme = Theme::of(ui);
    let show_clear = !query.is_empty();
    let mut field = with_style(ui, Surface::Panel, Action::Outline, |ui| {
        ui.visuals_mut().extreme_bg_color = theme.inset;
        let mut edit = egui::TextEdit::singleline(query)
            .id(id)
            .align(egui::Align2::LEFT_CENTER)
            .min_size(egui::vec2(0.0, CONTROL_HEIGHT))
            .hint_text(egui::RichText::new(hint).color(theme.muted_text))
            .text_color(theme.text)
            .prefix(search_icon)
            .margin(egui::Margin::symmetric(8, 3))
            .desired_width(f32::INFINITY);
        if show_clear {
            edit = edit.suffix(egui::Atom::custom(clear_id, egui::vec2(20.0, 20.0)));
        }
        edit.show(ui).response
    });
    if let Some(rect) = field.rect(clear_id) {
        let mut clear_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(id.with("clear"))
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        clear_ui.spacing_mut().button_padding = egui::Vec2::ZERO;
        clear_ui.spacing_mut().interact_size = rect.size();
        let clear = button(
            &mut clear_ui,
            Surface::Panel,
            Action::Tertiary,
            true,
            Button::new(clear_icon).min_size(rect.size()),
        )
        .on_hover_text(clear_label);
        clear.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, clear_label));
        if clear.clicked() {
            query.clear();
            field.request_focus();
            field.mark_changed();
        }
    }
    field.response
}

/// Draws a compact icon button and gives the painter a centered square.
pub fn icon_button(
    ui: &mut Ui,
    width: f32,
    icon_color: Option<egui::Color32>,
    idle_fill: Option<egui::Color32>,
    paint: impl FnOnce(&Ui, egui::Rect, egui::Color32),
) -> Response {
    let size = egui::vec2(width, ui.spacing().interact_size.y);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let (hover, press) = motion(ui, &response);
        let fill = match idle_fill {
            Some(fill) => fill.gamma_multiply(1.0 + hover.max(press) * 0.18),
            None => ui
                .visuals()
                .widgets
                .hovered
                .bg_fill
                .lerp_to_gamma(ui.visuals().widgets.active.bg_fill, press)
                .gamma_multiply(hover.max(press)),
        };
        let stroke = if response.has_focus() {
            ui.visuals().selection.stroke
        } else {
            Stroke::NONE
        };
        let painter = ui.painter();
        painter.rect(rect, CONTROL_RADIUS, fill, stroke, egui::StrokeKind::Inside);

        let color = icon_color.unwrap_or(visuals.fg_stroke.color);
        let icon_size = rect.width().min(rect.height()).min(icons::SIZE);
        paint(
            ui,
            egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(icon_size)),
            color,
        );
    }
    if let Some(cursor) = ui.visuals().interact_cursor
        && response.hovered()
    {
        ui.ctx().set_cursor_icon(cursor);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_and_clear_fit_narrow_and_wide_panels() {
        for width in [88.0, 112.0, 320.0] {
            let context = egui::Context::default();
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 120.0),
                )),
                ..Default::default()
            };
            context
                .run_ui(input, |ui| {
                    let right = ui.available_rect_before_wrap().right();
                    let mut query = "a long search query".to_owned();
                    let row = search(
                        ui,
                        "search",
                        &mut query,
                        "Search",
                        egui::Atom::paint(egui::Vec2::splat(16.0), |_, _| {}),
                        egui::Atom::paint(egui::Vec2::splat(16.0), |_, _| {}),
                        "Clear search",
                    );
                    assert!(row.rect.right() <= right + 0.5, "{width}: {:?}", row.rect);
                    assert!(ui.min_rect().right() <= right + 0.5);
                })
                .drop_without_applying_deltas();
        }
    }

    #[test]
    fn search_centers_content_and_clears_inside_the_field() {
        let ctx = egui::Context::default();
        let mut query = "Active search".to_owned();
        let mut frame = |events| {
            let mut rect = egui::Rect::NOTHING;
            let output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.set_width(160.0);
                    rect = search(
                        ui,
                        "centered",
                        &mut query,
                        "Find",
                        egui::Atom::paint(egui::Vec2::splat(16.0), |_, _| {}),
                        egui::Atom::paint(egui::vec2(16.0, 16.0), |ui, args| {
                            ui.painter().circle_filled(
                                args.rect.center(),
                                2.0,
                                egui::Color32::MAGENTA,
                            );
                        }),
                        "Clear search",
                    )
                    .rect;
                },
            );
            (rect, output, query.clone())
        };
        let (field, output, _) = frame(vec![]);
        let clear = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Circle(icon) if icon.fill == egui::Color32::MAGENTA => {
                    Some(icon.center)
                }
                _ => None,
            })
            .unwrap();
        assert!(field.contains(clear));
        assert!((clear.y - field.center().y).abs() <= 1.0);
        let text_center = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Active search" => {
                    Some(text.pos.y + text.galley.size().y * 0.5)
                }
                _ => None,
            })
            .unwrap();
        assert!((text_center - field.center().y).abs() <= 1.0);
        output.drop_without_applying_deltas();
        frame(vec![egui::Event::PointerMoved(clear)])
            .1
            .drop_without_applying_deltas();
        for pressed in [true, false] {
            let (_, output, text) = frame(vec![egui::Event::PointerButton {
                pos: clear,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: egui::Modifiers::NONE,
            }]);
            assert_eq!(text.is_empty(), !pressed);
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn icon_buttons_center_the_painter_and_fit_compact_sizes() {
        let context = egui::Context::default();
        context
            .run_ui(egui::RawInput::default(), |ui| {
                for width in [12.0, 28.0, 48.0] {
                    let mut painted = None;
                    let button = icon_button(ui, width, None, None, |_, rect, _| {
                        painted = Some(rect);
                    });
                    let painted = painted.unwrap();
                    assert_eq!(painted.center(), button.rect.center());
                    assert!(button.rect.contains_rect(painted));
                    assert_eq!(painted.width(), painted.height());
                }
            })
            .drop_without_applying_deltas();
    }

    #[test]
    fn controls_restore_the_hosts_styles() {
        let context = egui::Context::default();
        context
            .run_ui(egui::RawInput::default(), |ui| {
                for visuals in [egui::Visuals::light(), egui::Visuals::dark()] {
                    *ui.visuals_mut() = visuals;
                    let theme = ui.style().clone();
                    let _ = button(
                        ui,
                        Surface::Dialog,
                        Action::Primary,
                        true,
                        Button::new("Save"),
                    );
                    let _ = ComboBox::from_id_salt("kind")
                        .selected_text("Folder")
                        .show_select(ui, Surface::Panel, |ui| ui.label("Folder"));
                    assert_eq!(ui.style(), &theme);
                }
            })
            .drop_without_applying_deltas();
    }

    #[test]
    fn host_colors_apply_to_button_roles_and_can_be_reset() {
        let context = egui::Context::default();
        context
            .run_ui(egui::RawInput::default(), |ui| {
                let mut theme = Theme::of(ui);
                theme.primary = egui::Color32::DARK_BLUE;
                theme.secondary = egui::Color32::LIGHT_GRAY;
                theme.danger_fill = egui::Color32::DARK_RED;
                Theme::set(ui.ctx(), Some(theme));
                let supplied = Theme::of(ui);
                for (role, expected) in [
                    (Action::Primary, theme.primary),
                    (Action::Secondary, theme.secondary),
                    (Action::Outline, theme.panel),
                    (Action::Destructive, theme.danger_fill),
                ] {
                    assert_eq!(role_colors(supplied, Surface::Panel, role).0, expected);
                }
                Theme::set(ui.ctx(), None);
                assert_eq!(Theme::of(ui).primary, ui.visuals().selection.bg_fill);
            })
            .drop_without_applying_deltas();
    }

    #[test]
    fn dialog_actions_keep_cancel_available_and_wrap_in_a_narrow_dialog() {
        let ctx = egui::Context::default();
        for (width, enabled) in [(116.0, false), (320.0, true)] {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 160.0),
                )),
                ..Default::default()
            };
            ctx.run_ui(input, |ui| {
                let bounds = ui.available_rect_before_wrap();
                let (cancel, confirm) = dialog_actions(
                    ui,
                    "Cancel",
                    Button::new("Remove package"),
                    Action::Destructive,
                    enabled,
                );
                assert!(cancel.enabled());
                assert_eq!(confirm.enabled(), enabled);
                assert!(bounds.contains_rect(cancel.rect));
                assert!(bounds.contains_rect(confirm.rect));
                assert!(!cancel.rect.intersects(confirm.rect));
                if enabled {
                    assert!(cancel.rect.right() < confirm.rect.left());
                } else {
                    assert!(cancel.rect.bottom() < confirm.rect.top());
                }
            })
            .drop_without_applying_deltas();
        }
    }

    #[test]
    fn hover_color_transitions_between_idle_and_active() {
        let ctx = egui::Context::default();
        ctx.style_mut_of(ctx.theme(), |style| style.animation_time = 0.2);
        let frame = |time, events| {
            let mut rect = egui::Rect::NOTHING;
            let output = ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    rect = button(
                        ui,
                        Surface::Panel,
                        Action::Secondary,
                        true,
                        Button::new("Save"),
                    )
                    .rect;
                },
            );
            let fill = orange_testkit::ui::rect_fill(&output.shapes, rect).unwrap();
            output.drop_without_applying_deltas();
            (rect, fill)
        };
        let (rect, idle) = frame(0.0, vec![]);
        frame(0.02, vec![egui::Event::PointerMoved(rect.center())]);
        let (_, transition) = frame(0.12, vec![]);
        let (_, hovered) = frame(0.30, vec![]);
        assert_ne!(transition, idle);
        assert_ne!(transition, hovered);
    }
}
