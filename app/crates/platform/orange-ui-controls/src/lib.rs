//! Shared control roles. Colors follow the host theme and the surrounding surface.
use egui::{Button, ComboBox, InnerResponse, Response, Stroke, Ui};

pub mod audio_meter;
pub mod color_picker;
pub mod components;
mod desktop_style;
pub mod eq_plot;
pub mod frequency;
#[cfg(feature = "icons")]
pub mod icons;
pub use desktop_style::apply_desktop_style;
pub mod navigation;
pub mod note;
pub mod patterns;
pub mod theme;
pub use theme::{Density, Divider, SpacingScale, ThemeMode, UiTokens};
mod selection;
pub mod visualizations;
pub use selection::{
    BinaryChoice, ChoiceRow, DialFace, KnobStyle, NotchStyle, NotebookOrientation, NotebookPadding,
    NotebookWidth, SliderBehavior, SliderGuide, SliderMarks, SliderStyle, StepLabels, ThumbStyle,
    continuous_marked_slider, continuous_slider, dial, dial_preview, guided_slider, knob,
    logarithmic_slider, marked_slider, marked_slider_preview, notebook, notebook_flex,
    notebook_styled, pill_segmented, rotary, rotary_preview, segmented, step_buttons,
    stepped_slider,
};
pub use visualizations::{
    ChartOrientation, ChartRepresentation, LevelMeter, MeterColors, MeterOrientation,
    MeterRepresentation, MeterStop, chart, frequency_chart, line_overlay, paint_line,
    paint_line_iter, paint_vertical_meter,
};

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
        Self::resolve(ui.ctx(), ui.visuals())
    }

    fn resolve(ctx: &egui::Context, visuals: &egui::Visuals) -> Self {
        if let Some(theme) =
            ctx.data(|data| data.get_temp(egui::Id::unique("orange-ui-controls-theme")))
        {
            return theme;
        }
        if let Some(tokens) = UiTokens::installed(ctx) {
            let t = tokens.theme;
            return Self {
                panel: t.surface,
                dialog: t.surface_elevated,
                inset: t.surface_elevated,
                text: t.text,
                muted_text: t.text_muted,
                border: t.border.color,
                primary: t.primary,
                on_primary: t.on_primary,
                secondary: t.surface_elevated,
                on_secondary: t.text,
                focus: t.focus_ring.color,
                danger: t.danger,
                danger_fill: t.surface_elevated,
                on_danger: t.danger,
            };
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// A button's visual treatment independent of its semantic role.
pub enum ButtonStyle {
    /// A contrasting filled action.
    Solid,
    /// A bordered action with the surrounding surface fill.
    Outline,
    /// A quiet action with no persistent border.
    Ghost,
    /// A subdued action for menus and toolbars.
    Subtle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Semantic emphasis for a button without coupling meaning to visual filling.
pub enum ButtonTone {
    Neutral,
    Info,
    Success,
    Warning,
    Danger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Interaction state used by button, toggle, radio and segmented renderers.
pub enum ControlState {
    Inactive,
    Hovered,
    Pressed,
    Focused,
    Selected,
    Disabled,
}

impl ControlState {
    pub fn of(response: &Response, selected: bool) -> Self {
        if !response.enabled() {
            Self::Disabled
        } else if selected {
            Self::Selected
        } else if response.is_pointer_button_down_on() {
            Self::Pressed
        } else if response.has_focus() {
            Self::Focused
        } else if response.hovered() {
            Self::Hovered
        } else {
            Self::Inactive
        }
    }
}

#[derive(Debug, Clone, Copy)]
/// Geometry shared by ordinary and icon buttons.
pub struct ButtonGeometry {
    pub radius: u8,
    pub border_width: f32,
    pub border: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct ButtonSpec {
    pub style: ButtonStyle,
    pub tone: ButtonTone,
    pub geometry: ButtonGeometry,
    pub enabled: bool,
    pub active: bool,
}

impl ButtonSpec {
    pub const fn new(style: ButtonStyle, tone: ButtonTone) -> Self {
        Self {
            style,
            tone,
            geometry: ButtonGeometry::ROUNDED,
            enabled: true,
            active: false,
        }
    }

    pub const fn geometry(mut self, geometry: ButtonGeometry) -> Self {
        self.geometry = geometry;
        self
    }

    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub const fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }
}

impl ButtonGeometry {
    pub const ROUNDED: Self = Self {
        radius: CONTROL_RADIUS,
        border_width: 1.0,
        border: true,
    };
    pub const COMPACT: Self = Self {
        radius: 4,
        border_width: 1.0,
        border: true,
    };
    pub const PILL: Self = Self {
        radius: u8::MAX,
        border_width: 1.0,
        border: true,
    };
    pub const NO_BORDER: Self = Self {
        radius: CONTROL_RADIUS,
        border_width: 0.0,
        border: false,
    };
    pub const SQUARE: Self = Self {
        radius: 0,
        border_width: 1.0,
        border: true,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The visual density of a checkbox or toggle.
pub enum CheckStyle {
    /// A native-style square checkbox.
    Simple,
    /// A compact switch with a moving indicator.
    Toggle,
    /// A filled, high-contrast switch with a stronger selected state.
    Emphasis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The visual density of a radio or single-choice list item.
pub enum RadioStyle {
    /// A native-style circular radio.
    Simple,
    /// A bordered choice that reads as a persistent selection.
    Emphasis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionStyle {
    Compact,
    Segmented,
    Card,
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
            let theme = Theme::of(ui);
            let field = input_frame(ui, surface, CONTROL_RADIUS);
            let focus = Stroke::new(2.0, theme.focus);
            let widgets = &mut ui.visuals_mut().widgets;
            for state in [
                &mut widgets.inactive,
                &mut widgets.hovered,
                &mut widgets.active,
                &mut widgets.open,
            ] {
                state.bg_stroke = field.stroke;
                state.bg_fill = field.fill;
                state.weak_bg_fill = field.fill;
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
        spacing.interact_size.y = 26.0;
        spacing.button_padding = egui::vec2(SPACE_MEDIUM, 3.0);
        spacing.indent = 20.0;
        spacing.item_spacing.x = SPACE_SMALL;
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
        let fill = button_feedback_fill(fill, ink, hover * 0.07 + press * 0.09);
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

/// Draws a button with explicit visual style, geometry and semantic tone.
pub fn button_with(
    ui: &mut Ui,
    surface: Surface,
    spec: ButtonSpec,
    egui_button: Button<'_>,
) -> Response {
    let style = spec.style;
    let tone = spec.tone;
    let geometry = spec.geometry;
    let enabled = spec.enabled;
    let active = spec.active;
    let mut role = match tone {
        ButtonTone::Neutral => match style {
            ButtonStyle::Solid => Action::Secondary,
            ButtonStyle::Outline => Action::Outline,
            ButtonStyle::Ghost => Action::Tertiary,
            ButtonStyle::Subtle => Action::Menu,
        },
        ButtonTone::Info => Action::Primary,
        ButtonTone::Success => Action::Primary,
        ButtonTone::Warning => Action::Secondary,
        ButtonTone::Danger => Action::Destructive,
    };
    if active {
        role = Action::Toggle(true);
    }
    let theme = Theme::of(ui);
    let (fill, border, ink) = role_colors(theme, surface, role);
    let fill = match tone {
        ButtonTone::Info => theme.primary,
        ButtonTone::Success => theme.primary,
        ButtonTone::Warning => theme.secondary.lerp_to_gamma(theme.text, 0.12),
        ButtonTone::Danger => theme.danger_fill,
        ButtonTone::Neutral => fill,
    };
    let border = if geometry.border {
        border
    } else {
        Stroke::NONE.color
    };
    let background = ui.painter().add(egui::Shape::Noop);
    let response = with_style(ui, surface, role, |ui| {
        for state in widget_states(&mut ui.visuals_mut().widgets) {
            state.bg_stroke = Stroke::NONE;
            state.bg_fill = egui::Color32::TRANSPARENT;
            state.weak_bg_fill = egui::Color32::TRANSPARENT;
            state.fg_stroke.color = ink;
        }
        ui.visuals_mut().override_text_color = Some(ink);
        ui.add_enabled(enabled, egui_button.frame(true))
    });
    let (hover, press) = motion(ui, &response);
    if ui.is_rect_visible(response.rect) {
        let base = if active {
            theme.selected_fill(surface)
        } else {
            fill
        };
        let muted = button_feedback_fill(base, ink, hover * 0.07 + press * 0.09);
        let stroke = if response.has_focus() {
            Stroke::new(2.0, theme.focus)
        } else if geometry.border {
            Stroke::new(geometry.border_width, border)
        } else {
            Stroke::NONE
        };
        let fade = if enabled { 1.0 } else { 0.45 };
        ui.painter().set(
            background,
            egui::epaint::RectShape::new(
                response.rect,
                geometry.radius,
                muted.gamma_multiply(fade),
                Stroke::new(stroke.width, stroke.color.gamma_multiply(fade)),
                egui::StrokeKind::Inside,
            ),
        );
    }
    response
}

/// Standard confirmation and cancel pair for dialogs.
pub fn confirm_cancel(
    ui: &mut Ui,
    surface: Surface,
    confirm: &str,
    cancel: &str,
    confirm_tone: ButtonTone,
    confirm_enabled: bool,
) -> (Response, Response) {
    ui.horizontal_wrapped(|ui| {
        let cancel = button_with(
            ui,
            surface,
            ButtonSpec::new(ButtonStyle::Outline, ButtonTone::Neutral),
            Button::new(cancel),
        );
        let confirm = button_with(
            ui,
            surface,
            ButtonSpec::new(ButtonStyle::Solid, confirm_tone).enabled(confirm_enabled),
            Button::new(confirm),
        );
        (cancel, confirm)
    })
    .inner
}

/// Draws a button using an explicit visual treatment.
pub fn button_styled(
    ui: &mut Ui,
    surface: Surface,
    role: Action,
    style: ButtonStyle,
    enabled: bool,
    egui_button: Button<'_>,
) -> Response {
    let role = match style {
        ButtonStyle::Solid => role,
        ButtonStyle::Outline => Action::Outline,
        ButtonStyle::Ghost => Action::Tertiary,
        ButtonStyle::Subtle => Action::Menu,
    };
    button(ui, surface, role, enabled, egui_button)
}

/// A persistent on/off button that updates its boolean value and exposes hover,
/// pressed, focus and disabled states through the shared button renderer.
pub fn toggle_button(
    ui: &mut Ui,
    surface: Surface,
    value: &mut bool,
    enabled: bool,
    egui_button: Button<'_>,
) -> Response {
    let mut response = button(ui, surface, Action::Toggle(*value), enabled, egui_button);
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response
}

/// Draws a multi-select option list where several choices may be selected.
pub fn multi_select<T: Copy + PartialEq>(
    ui: &mut Ui,
    surface: Surface,
    selected: &mut Vec<T>,
    choices: &[(T, &str)],
) -> Response {
    let mut changed = false;
    let mut last = None;
    ui.vertical(|ui| {
        for (choice, label) in choices {
            let mut on = selected.contains(choice);
            let response = checkbox(ui, surface, CheckStyle::Simple, &mut on, label);
            if response.changed() {
                if on {
                    if !selected.contains(choice) {
                        selected.push(*choice);
                    }
                } else {
                    selected.retain(|value| value != choice);
                }
                changed = true;
            }
            last = Some(response);
        }
    });
    let mut response =
        last.unwrap_or_else(|| ui.allocate_response(egui::vec2(0.0, 0.0), egui::Sense::hover()));
    if changed {
        response.mark_changed();
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
        Action::Toggle(true) => {
            let fill = theme.selected_fill(surface);
            (
                fill,
                theme.focus,
                theme::ensure_contrast(theme.text, fill, 5.3),
            )
        }
        Action::Secondary => {
            let fill = if matches!(surface, Surface::Panel) {
                theme.secondary
            } else {
                neutral_fill.lerp_to_gamma(theme.text, 0.14)
            };
            (
                fill,
                theme.border,
                theme::ensure_contrast(theme.text, fill, 4.5),
            )
        }
        Action::Outline => (background, theme.border, theme.text),
        Action::Destructive => (theme.danger_fill, theme.danger, theme.on_danger),
        Action::Tertiary => (background, theme.border, theme.muted_text),
        Action::Toggle(false) | Action::Menu => (background, theme.border, theme.text),
    }
}

fn button_feedback_fill(fill: egui::Color32, ink: egui::Color32, amount: f32) -> egui::Color32 {
    let feedback = fill.lerp_to_gamma(ink, amount);
    if theme::contrast(ink, feedback) >= 4.5 {
        feedback
    } else {
        fill
    }
}

/// A Boolean option with a compact checkbox and quiet label.
pub fn toggle(ui: &mut Ui, surface: Surface, value: &mut bool, label: &str) -> Response {
    with_style(ui, surface, Action::Outline, |ui| {
        ControlSize::Compact.apply(ui.spacing_mut());
        ui.checkbox(value, label)
    })
}

/// Draws a checkbox or switch with one shared selected, disabled and focus contract.
pub fn checkbox(
    ui: &mut Ui,
    surface: Surface,
    style: CheckStyle,
    value: &mut bool,
    label: &str,
) -> Response {
    match style {
        CheckStyle::Simple => toggle(ui, surface, value, label),
        CheckStyle::Toggle | CheckStyle::Emphasis => {
            let tokens = UiTokens::for_surface(ui, surface);
            tokens.scope(ui, None, |ui, tokens| {
                components::switch(ui, tokens, value, label)
            })
        }
    }
}

/// Draws one radio choice. Segmented radio groups should use `segmented`; this
/// primitive is for independent choices inside forms or popup menus.
pub fn radio<T: Copy + PartialEq>(
    ui: &mut Ui,
    surface: Surface,
    style: RadioStyle,
    value: &mut T,
    choice: T,
    label: &str,
) -> Response {
    let selected = *value == choice;
    match style {
        RadioStyle::Simple => ui.radio_value(value, choice, label),
        RadioStyle::Emphasis => {
            let mut response = button(
                ui,
                surface,
                if selected {
                    Action::Toggle(true)
                } else {
                    Action::Toggle(false)
                },
                true,
                Button::new(label).truncate(),
            );
            if response.clicked() {
                *value = choice;
                response.mark_changed();
            }
            response
        }
    }
}

/// Draws a themed single-choice dropdown from labels and values.
pub fn select<T: Copy + PartialEq>(
    ui: &mut Ui,
    surface: Surface,
    id: egui::Id,
    selected: &mut T,
    selected_label: &str,
    options: &[(T, &str)],
) -> Response {
    let mut changed = false;
    let mut response = ComboBox::from_id_salt(id)
        .selected_text(selected_label)
        .show_select(ui, surface, |ui| {
            for (value, label) in options {
                let is_selected = *selected == *value;
                if ui.selectable_label(is_selected, *label).clicked() && !is_selected {
                    *selected = *value;
                    changed = true;
                    ui.close();
                }
            }
        })
        .response;
    if changed {
        response.mark_changed();
    }
    response
}

/// Draws a themed checklist for independent boolean choices.
pub fn check_list(
    ui: &mut Ui,
    surface: Surface,
    style: CheckStyle,
    choices: &mut [(&mut bool, &str)],
) -> Response {
    let mut changed = false;
    let mut response = None;
    ui.vertical(|ui| {
        for (value, label) in choices.iter_mut() {
            let item = checkbox(ui, surface, style, value, label);
            changed |= item.changed();
            response = Some(item);
        }
    });
    let mut response = response
        .unwrap_or_else(|| ui.allocate_response(egui::vec2(0.0, 0.0), egui::Sense::hover()));
    if changed {
        response.mark_changed();
    }
    response
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
    search_sized(
        ui,
        salt,
        query,
        hint,
        search_icon,
        clear_icon,
        clear_label,
        f32::INFINITY,
    )
}

/// Search field with a caller-set width, for expanding search affordances.
#[allow(
    clippy::too_many_arguments,
    reason = "shares the search field's host labels and icon slots"
)]
pub fn search_sized(
    ui: &mut Ui,
    salt: &str,
    query: &mut String,
    hint: &str,
    search_icon: egui::Atom<'static>,
    clear_icon: egui::Atom<'_>,
    clear_label: &str,
    width: f32,
) -> Response {
    let id = ui.make_persistent_id(salt);
    let clear_id = egui::IdSalt::new("clear-search");
    let theme = Theme::of(ui);
    let show_clear = !query.is_empty();
    let width = width.min((ui.clip_rect().right() - ui.next_widget_position().x - 2.0).max(0.0));
    let mut field = with_style(ui, Surface::Panel, Action::Outline, |ui| {
        let mut edit = egui::TextEdit::singleline(query)
            .id(id)
            .frame(input_frame(ui, Surface::Panel, 255))
            .align(egui::Align2::LEFT_CENTER)
            .min_size(egui::vec2(
                0.0,
                ui.spacing().interact_size.y.max(CONTROL_HEIGHT),
            ))
            .hint_text(egui::RichText::new(hint).color(theme.muted_text))
            .text_color(theme.text)
            .prefix(search_icon)
            .desired_width(width);
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
    paint_input_focus(ui, &field.response, 255);
    field.response
}

/// Consumes unmodified navigation keys reserved by a focused control.
pub fn consume_plain_keys<const N: usize>(ui: &Ui, keys: [egui::Key; N]) -> [bool; N] {
    ui.input_mut(|input| keys.map(|key| input.consume_key(egui::Modifiers::NONE, key)))
}

/// Draws wrapping toolbar actions followed by a field that fills the remaining
/// row. When the field would fall below its minimum width, it wraps below.
pub fn action_row<R>(
    ui: &mut Ui,
    field_min: f32,
    actions: impl FnOnce(&mut Ui),
    field: impl FnOnce(&mut Ui) -> R,
) -> R {
    let full_width = ui.available_width().max(0.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(SPACE_MEDIUM, SPACE_SMALL);
        ControlSize::Standard.apply(ui.spacing_mut());
        actions(ui);
        let remaining = ui.available_rect_before_wrap().width().max(0.0);
        let width = if remaining >= field_min.min(full_width) {
            remaining
        } else {
            ui.end_row();
            full_width
        };
        ui.allocate_ui_with_layout(
            egui::vec2(width, CONTROL_HEIGHT),
            egui::Layout::top_down(egui::Align::Min),
            field,
        )
        .inner
    })
    .inner
}

/// Two-column form grid with the same spacing as the shared fields.
pub fn form_grid(id: impl egui::AsIdSalt) -> egui::Grid {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing(egui::vec2(SPACE_LARGE, SPACE_MEDIUM))
}

/// A wrapping inline message. The caller supplies the text, including the
/// meaning of the state, so its color is only a visual cue.
pub fn notice(ui: &mut Ui, tone: ButtonTone, text: impl Into<egui::WidgetText>) -> Response {
    let theme = Theme::of(ui);
    let accent = match tone {
        ButtonTone::Danger => theme.danger,
        ButtonTone::Warning => ui.visuals().warn_fg_color,
        ButtonTone::Success => egui::Color32::from_rgb(64, 154, 108),
        ButtonTone::Info => theme.focus,
        ButtonTone::Neutral => theme.border,
    };
    egui::Frame::new()
        .fill(theme.dialog.lerp_to_gamma(accent, 0.11))
        .stroke(Stroke::new(1.0, theme.border.lerp_to_gamma(accent, 0.52)))
        .corner_radius(CONTROL_RADIUS)
        .inner_margin(egui::vec2(SPACE_MEDIUM, SPACE_SMALL))
        .show(ui, |ui| {
            ui.visuals_mut().override_text_color = Some(theme.text);
            ui.add(egui::Label::new(text).wrap())
        })
        .inner
}

/// A themed single-line text field for forms and in-place editors.
pub fn text_field(
    ui: &mut Ui,
    surface: Surface,
    id: egui::Id,
    text: &mut String,
    hint: &str,
) -> Response {
    with_style(ui, surface, Action::Outline, |ui| {
        let theme = Theme::of(ui);
        let response = egui::TextEdit::singleline(text)
            .id(id)
            .frame(input_frame(ui, surface, CONTROL_RADIUS))
            .align(egui::Align2::LEFT_CENTER)
            .min_size(egui::vec2(0.0, ui.spacing().interact_size.y))
            .hint_text(egui::RichText::new(hint).color(theme.muted_text))
            .text_color(theme.text)
            .desired_width(f32::INFINITY)
            .show(ui)
            .response;
        paint_input_focus(ui, &response, CONTROL_RADIUS);
        response.response
    })
}

/// A themed multi-line text area for JSON, scripts and descriptions.
pub fn text_area(
    ui: &mut Ui,
    surface: Surface,
    id: egui::Id,
    text: &mut String,
    hint: &str,
) -> Response {
    with_style(ui, surface, Action::Outline, |ui| {
        let theme = Theme::of(ui);
        let response = egui::TextEdit::multiline(text)
            .id(id)
            .frame(input_frame(ui, surface, CONTROL_RADIUS))
            .desired_rows(4)
            .hint_text(egui::RichText::new(hint).color(theme.muted_text))
            .text_color(theme.text)
            .desired_width(f32::INFINITY)
            .show(ui)
            .response;
        paint_input_focus(ui, &response, CONTROL_RADIUS);
        response.response
    })
}

pub(crate) fn input_frame(ui: &Ui, surface: Surface, radius: u8) -> egui::Frame {
    let theme = Theme::of(ui);
    let (fill, ink) = if UiTokens::configured(ui.ctx()) {
        let palette = UiTokens::resolve(ui.ctx()).theme;
        (palette.bg_input(), palette.control_border().color)
    } else {
        (theme.inset, theme.border)
    };
    let border = theme::ensure_contrast(ink, theme.background(surface), 3.0);
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(radius)
        .inner_margin(egui::Margin::symmetric(9, 4))
}

pub(crate) fn paint_input_focus(ui: &Ui, response: &Response, radius: u8) {
    if response.has_focus() {
        let theme = Theme::of(ui);
        ui.painter().rect_stroke(
            response.rect,
            radius,
            Stroke::new(2.0, theme.focus),
            egui::StrokeKind::Inside,
        );
    }
}

/// Multiline field with a visible resize handle and a caller-chosen resize axis.
pub fn resizable_text_area(
    ui: &mut Ui,
    surface: Surface,
    id: egui::Id,
    text: &mut String,
    hint: &str,
    axes: egui::Vec2b,
) -> Response {
    let width = ui.available_width().max(1.0);
    let key = id.with("area-size");
    let stored = ui
        .data(|data| data.get_temp::<egui::Vec2>(key))
        .unwrap_or(egui::vec2(width * 0.87, 94.0));
    let size = egui::vec2(
        if axes.x {
            stored.x.clamp(width.min(100.0), width)
        } else {
            width
        },
        if axes.y {
            stored.y.clamp(74.0, 280.0)
        } else {
            94.0
        },
    );
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let theme = Theme::of(ui);
    let mut child = ui.new_child(egui::UiBuilder::new().id_salt(id).max_rect(rect));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    let frame = input_frame(ui, surface, CONTROL_RADIUS);
    let response = frame
        .show(&mut child, |ui| {
            let inner = ui.available_size();
            egui::ScrollArea::vertical()
                .id_salt(id.with("scroll"))
                .auto_shrink([false, false])
                .max_height(inner.y)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(text)
                            .id(id)
                            .frame(egui::Frame::NONE)
                            .desired_width(ui.available_width())
                            .min_size(egui::vec2(ui.available_width(), inner.y))
                            .hint_text(egui::RichText::new(hint).color(theme.muted_text))
                            .text_color(theme.text),
                    )
                })
                .inner
        })
        .inner;
    if response.has_focus() {
        ui.painter_at(rect).rect_stroke(
            rect,
            CONTROL_RADIUS,
            Stroke::new(2.0, theme.focus),
            egui::StrokeKind::Inside,
        );
    }
    let (center, cursor) = match (axes.x, axes.y) {
        (false, true) => (
            rect.center_bottom() - egui::vec2(0.0, 4.0),
            egui::CursorIcon::ResizeVertical,
        ),
        (true, false) => (
            rect.right_center() - egui::vec2(4.0, 0.0),
            egui::CursorIcon::ResizeHorizontal,
        ),
        (true, true) => (
            rect.right_bottom() - egui::Vec2::splat(6.0),
            egui::CursorIcon::ResizeNwSe,
        ),
        _ => return response,
    };
    let target = match (axes.x, axes.y) {
        (true, false) => egui::vec2(20.0, 48.0),
        (false, true) => egui::vec2(48.0, 20.0),
        _ => egui::Vec2::splat(22.0),
    };
    let hit = egui::Rect::from_center_size(center, target).intersect(rect);
    let grip = ui
        .interact(hit, id.with("grip"), egui::Sense::drag())
        .on_hover_cursor(cursor);
    if grip.dragged() {
        let delta = grip.drag_delta();
        let next = egui::vec2(
            if axes.x {
                (size.x + delta.x).clamp(width.min(100.0), width)
            } else {
                size.x
            },
            if axes.y {
                (size.y + delta.y).clamp(74.0, 280.0)
            } else {
                size.y
            },
        );
        ui.data_mut(|data| data.insert_temp(key, next));
    }
    let direction = match (axes.x, axes.y) {
        (true, true) => egui::vec2(3.5, -3.5),
        (true, false) => egui::vec2(0.0, 16.0),
        _ => egui::vec2(16.0, 0.0),
    };
    ui.painter().line_segment(
        [center - direction, center + direction],
        Stroke::new(2.0, theme.muted_text),
    );
    response
}

fn surface_frame(frame: egui::Frame, theme: Theme) -> egui::Frame {
    frame
        .fill(theme.dialog)
        .stroke(Stroke::new(1.0, theme.border))
        .corner_radius(CONTROL_RADIUS)
        .inner_margin(SPACE_MEDIUM)
}

/// The shared frame for settings, confirmation and form windows.
pub fn dialog_frame(ui: &Ui) -> egui::Frame {
    surface_frame(egui::Frame::window(ui.style()), Theme::of(ui))
}

/// Builds a window using the same surface and insets as modal dialogs.
pub fn dialog_window(
    ctx: &egui::Context,
    title: impl Into<egui::WidgetText>,
) -> egui::Window<'static> {
    let style = ctx.style_of(ctx.theme());
    let theme = Theme::resolve(ctx, &style.visuals);
    egui::Window::new(title).frame(surface_frame(egui::Frame::window(&style), theme))
}

/// Builds a modal with the shared dialog surface.
pub fn modal(ctx: &egui::Context, id: egui::Id) -> egui::Modal {
    let style = ctx.style_of(ctx.theme());
    let theme = Theme::resolve(ctx, &style.visuals);
    egui::Modal::new(id).frame(surface_frame(egui::Frame::window(&style), theme))
}

/// The shared frame for menus, popups and anchored editors.
pub fn popup_frame(ui: &Ui) -> egui::Frame {
    surface_frame(egui::Frame::popup(ui.style()), Theme::of(ui))
}

/// Draws a foreground popup with the shared screen inset and preferred content width.
pub fn anchored_popup<R>(
    ctx: &egui::Context,
    id: egui::Id,
    anchor: egui::Pos2,
    preferred_width: f32,
    contents: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let bounds = ctx.content_rect().shrink(SPACE_SMALL);
    let width = preferred_width.min((bounds.width() - 2.0 * (SPACE_MEDIUM + 1.0)).max(1.0));
    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .fixed_pos(bounds.clamp(anchor))
        .constrain_to(bounds)
        .show(ctx, |ui| {
            popup_frame(ui)
                .show(ui, |ui| {
                    ui.set_width(width);
                    contents(ui)
                })
                .inner
        })
}

/// A leading label and trailing shortcut, with shared row height and center line.
pub fn menu_item(
    ui: &mut Ui,
    icon: egui::Atom<'_>,
    label: &str,
    shortcut: &str,
    enabled: bool,
) -> Response {
    let width = ui.available_width();
    let item = Button::new((icon, label))
        .shortcut_text(shortcut)
        .min_size(egui::vec2(width, CONTROL_HEIGHT))
        .truncate();
    button(ui, Surface::Dialog, Action::Menu, enabled, item)
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
        let icon_size = rect.width().min(rect.height()).min(16.0);
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
    fn action_row_preserves_order_and_wraps_the_field_at_narrow_widths() {
        for (width, wraps) in [(200.0, true), (420.0, false)] {
            let ctx = egui::Context::default();
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 220.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let bounds = ui.available_rect_before_wrap();
                    let mut action = egui::Rect::NOTHING;
                    let field = action_row(
                        ui,
                        120.0,
                        |ui| {
                            action = ui
                                .allocate_exact_size(
                                    egui::vec2(124.0, CONTROL_HEIGHT),
                                    egui::Sense::hover(),
                                )
                                .0;
                        },
                        |ui| {
                            ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), CONTROL_HEIGHT),
                                egui::Sense::hover(),
                            )
                            .0
                        },
                    );
                    assert!(
                        bounds.expand(1.0).contains_rect(field),
                        "{width}: {bounds:?} does not contain {field:?}"
                    );
                    if wraps {
                        assert!(
                            field.top() >= action.bottom(),
                            "{width}: {action:?}, {field:?}"
                        );
                        assert!((field.left() - bounds.left()).abs() < 1.0);
                    } else {
                        assert!(field.left() >= action.right());
                        assert!((field.center().y - action.center().y).abs() < 1.0);
                    }
                },
            )
            .drop_without_applying_deltas();
        }
    }

    #[test]
    fn anchored_popup_stays_inside_the_screen_when_opened_at_an_edge() {
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(240.0, 180.0));
        let mut shown = egui::Rect::NOTHING;
        for _ in 0..3 {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                },
                |ui| {
                    shown = anchored_popup(
                        ui.ctx(),
                        egui::Id::unique("edge-popup"),
                        screen.right_bottom(),
                        312.0,
                        |ui| {
                            ui.label("Popup near the bottom right");
                        },
                    )
                    .response
                    .rect;
                },
            )
            .drop_without_applying_deltas();
        }
        assert!(screen.contains_rect(shown), "{shown:?} outside {screen:?}");
    }

    #[test]
    fn notices_wrap_long_text_in_a_narrow_surface() {
        let ctx = egui::Context::default();
        let message = "Could not complete the requested operation. Review the details.";
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(148.0);
            let row = notice(ui, ButtonTone::Danger, message);
            assert!(row.rect.width() < 148.0);
        });
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == message && text.galley.rows.len() > 1
        )));
        output.drop_without_applying_deltas();
    }

    #[test]
    fn menu_labels_and_shortcut_share_a_center_line_in_both_themes() {
        for visuals in [egui::Visuals::light(), egui::Visuals::dark()] {
            let ctx = egui::Context::default();
            ctx.set_visuals(visuals);
            let mut row = egui::Rect::NOTHING;
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(200.0);
                row = menu_item(
                    ui,
                    egui::Atom::custom(
                        egui::IdSalt::new("sample-menu-icon"),
                        egui::Vec2::splat(12.0),
                    ),
                    "Copy item",
                    "Ctrl+C",
                    true,
                )
                .rect;
            });
            let find = |caption| {
                output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == caption => {
                            Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                        }
                        _ => None,
                    })
                    .unwrap()
            };
            let label = find("Copy item");
            let shortcut = find("Ctrl+C");
            assert!(row.contains_rect(label) && row.contains_rect(shortcut));
            assert!(label.right() < shortcut.left());
            assert!((label.center().y - shortcut.center().y).abs() < 2.5);
            assert!((label.center().y - row.center().y).abs() < 2.5);
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
    fn button_geometry_tones_active_and_disabled_states_share_the_contract() {
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            for geometry in [
                ButtonGeometry::ROUNDED,
                ButtonGeometry::COMPACT,
                ButtonGeometry::PILL,
                ButtonGeometry::NO_BORDER,
                ButtonGeometry::SQUARE,
            ] {
                let enabled = button_with(
                    ui,
                    Surface::Panel,
                    ButtonSpec::new(ButtonStyle::Solid, ButtonTone::Info).geometry(geometry),
                    Button::new("Enabled"),
                );
                assert!(enabled.enabled());
                let disabled = button_with(
                    ui,
                    Surface::Panel,
                    ButtonSpec::new(ButtonStyle::Outline, ButtonTone::Neutral)
                        .geometry(geometry)
                        .enabled(false),
                    Button::new("Disabled"),
                );
                assert!(!disabled.enabled());
            }
        })
        .drop_without_applying_deltas();

        ctx.run_ui(egui::RawInput::default(), |ui| {
            let mut active = false;
            let response = toggle_button(ui, Surface::Panel, &mut active, true, Button::new("Run"));
            assert!(!active);
            assert!(response.rect.height() > 0.0);
        })
        .drop_without_applying_deltas();
    }

    #[test]
    fn confirm_cancel_keeps_cancel_available_and_confirm_disabled_when_needed() {
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            let (cancel, confirm) = confirm_cancel(
                ui,
                Surface::Dialog,
                "Confirm",
                "Cancel",
                ButtonTone::Danger,
                false,
            );
            assert!(cancel.enabled());
            assert!(!confirm.enabled());
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

#[cfg(test)]
#[test]
fn gallery_palettes_have_readable_input_and_toggle_boundaries() {
    for palette in [
        theme::Theme::DARK,
        theme::Theme::LIGHT,
        theme::Theme::MONOKAI,
        theme::Theme::BUBBLEGUM,
        theme::Theme::high_contrast_dark(),
        theme::Theme::high_contrast_light(),
    ] {
        let context = egui::Context::default();
        UiTokens::install(&context, ThemeMode::Custom(palette), Density::Comfy);
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            let input = input_frame(ui, Surface::Card, CONTROL_RADIUS);
            assert!(theme::contrast(input.stroke.color, palette.bg_surface()) >= 3.0);
            assert!(theme::contrast(input.stroke.color, input.fill) >= 3.0);
            assert!(theme::contrast(palette.text_muted, input.fill) >= 4.5);
            let (fill, _, ink) = role_colors(Theme::of(ui), Surface::Card, Action::Toggle(true));
            for amount in [0.0, 0.07, 0.16] {
                assert!(theme::contrast(ink, button_feedback_fill(fill, ink, amount)) >= 4.5);
            }
        });
        output.drop_without_applying_deltas();
    }
}

#[cfg(test)]
#[test]
fn diagonal_drags_only_move_enabled_text_area_edges() {
    for axes in [
        egui::Vec2b::new(false, true),
        egui::Vec2b::new(true, false),
        egui::Vec2b::TRUE,
    ] {
        let ctx = egui::Context::default();
        let id = egui::Id::unique("resize-axis-test");
        let mut draft = String::from("Keep this draft");
        let mut frame = |events| {
            let mut start = egui::Pos2::ZERO;
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(300.0, 240.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.set_width(200.0);
                    start = ui.next_widget_position();
                    resizable_text_area(ui, Surface::Card, id, &mut draft, "", axes);
                },
            )
            .drop_without_applying_deltas();
            start
        };
        let origin = frame(vec![]);
        let offset = match (axes.x, axes.y) {
            (false, true) => egui::vec2(100.0, 90.0),
            (true, false) => egui::vec2(170.0, 47.0),
            _ => egui::vec2(168.0, 88.0),
        };
        let at = origin + offset;
        frame(vec![egui::Event::PointerMoved(at)]);
        frame(vec![egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        }]);
        frame(vec![egui::Event::PointerMoved(at + egui::vec2(16.0, 18.0))]);
        let size = ctx
            .data(|d| d.get_temp::<egui::Vec2>(id.with("area-size")))
            .expect("grip handles drag");
        assert_eq!(size.x, if axes.x { 190.0 } else { 200.0 });
        assert_eq!(size.y, if axes.y { 112.0 } else { 94.0 });
        assert_eq!(draft, "Keep this draft");
    }
}
