use egui::{Color32, Context, Id, Rect, Sense, Shadow, Stroke, ThemePreference, Ui, vec2};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Density {
    #[default]
    Compact,
    Comfy,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpacingScale {
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
}

impl Density {
    pub const fn row_height(self) -> f32 {
        match self {
            Self::Compact => 24.0,
            Self::Comfy => 36.0,
        }
    }
    pub const fn spacing(self) -> SpacingScale {
        let unit = match self {
            Self::Compact => 2.0,
            Self::Comfy => 3.0,
        };
        SpacingScale {
            xs: unit,
            sm: 2.0 * unit,
            md: 4.0 * unit,
            lg: 6.0 * unit,
            xl: 8.0 * unit,
        }
    }

    fn apply(self, ui: &mut Ui) {
        self.apply_style(ui.style_mut());
    }

    pub(super) fn apply_style(self, style: &mut egui::Style) {
        let s = self.spacing();
        let (body, heading) = match self {
            Self::Compact => (13.0, 15.0),
            Self::Comfy => (15.0, 18.0),
        };
        style.spacing.item_spacing = vec2(s.sm, s.sm);
        style.spacing.button_padding = vec2(s.md, s.xs);
        style.spacing.interact_size.y = self.row_height();
        for (key, size) in [
            (egui::TextStyle::Body, body),
            (egui::TextStyle::Button, body),
            (egui::TextStyle::Heading, heading),
        ] {
            style
                .text_styles
                .entry(key)
                .and_modify(|font| font.size = size);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub dark: bool,
    pub primary: Color32,
    pub on_primary: Color32,
    pub surface: Color32,
    pub surface_elevated: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub divider: Color32,
    pub border: Stroke,
    pub border_strong: Stroke,
    pub focus_ring: Stroke,
    pub info: Color32,
    pub tip: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub radius: u8,
    pub elevation: Shadow,
}

const fn rgb(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

const fn stroke(width: f32, color: u32) -> Stroke {
    Stroke {
        width,
        color: rgb(color),
    }
}

impl Theme {
    pub const fn bg_canvas(self) -> Color32 {
        self.surface
    }
    pub const fn bg_surface(self) -> Color32 {
        self.surface_elevated
    }
    pub fn bg_elevated(self) -> Color32 {
        self.surface_elevated.lerp_to_gamma(self.text, 0.05)
    }
    pub fn bg_input(self) -> Color32 {
        self.surface_elevated.lerp_to_gamma(self.surface, 0.88)
    }

    pub fn control_border(self) -> Stroke {
        let fill = if self.dark {
            self.surface_elevated
        } else {
            self.bg_input()
        };
        Stroke::new(1.0, ensure_contrast(self.border_strong.color, fill, 3.0))
    }
    pub const fn border_subtle(self) -> Stroke {
        self.border
    }
    pub const fn accent_primary(self) -> Color32 {
        self.primary
    }
    pub const fn text_primary(self) -> Color32 {
        self.text
    }
    pub const fn success(self) -> Color32 {
        self.tip
    }

    /// Broadcast meter inks preserve the green, amber and red reading order
    /// across themes, including palettes with a non-green success accent.
    pub const fn meter_zone_inks(self) -> [Color32; 3] {
        if self.dark {
            [rgb(0x33D36F), rgb(0xF4CF41), rgb(0xFF5465)]
        } else {
            [rgb(0x148543), rgb(0xA76E00), rgb(0xC42634)]
        }
    }

    pub const DARK: Self = Self {
        dark: true,
        primary: rgb(0xFFC085),
        on_primary: rgb(0x2B1708),
        surface: rgb(0x1C1F25),
        surface_elevated: rgb(0x282D36),
        text: rgb(0xF5F4F7),
        text_muted: rgb(0xB7BFCC),
        divider: rgb(0x525B6A),
        border: stroke(1.0, 0x525B6A),
        border_strong: stroke(1.0, 0xA6ADBA),
        focus_ring: stroke(2.0, 0xFFC085),
        info: rgb(0x9BCBFF),
        tip: rgb(0x9DE3B0),
        warning: rgb(0xFFD17D),
        danger: rgb(0xFFABAD),
        radius: 10,
        elevation: Shadow {
            offset: [0, 2],
            blur: 8,
            spread: 0,
            color: Color32::from_black_alpha(65),
        },
    };
    pub const LIGHT: Self = Self {
        dark: false,
        primary: rgb(0x8B3E13),
        on_primary: Color32::WHITE,
        surface: rgb(0xF4F1EC),
        surface_elevated: Color32::WHITE,
        text: rgb(0x211B17),
        text_muted: rgb(0x544943),
        divider: rgb(0xD0C5BC),
        border: stroke(1.0, 0xD0C5BC),
        border_strong: stroke(1.0, 0x786A62),
        focus_ring: stroke(2.0, 0x8B3E13),
        info: rgb(0x175286),
        tip: rgb(0x1F6640),
        warning: rgb(0x7B4B06),
        danger: rgb(0x9C2338),
        ..Self::DARK
    };
    pub const MONOKAI: Self = Self {
        primary: rgb(0xA6E22E),
        on_primary: rgb(0x272822),
        surface: rgb(0x272822),
        surface_elevated: rgb(0x34352D),
        text: rgb(0xF8F8F2),
        text_muted: rgb(0xC7C7BA),
        divider: rgb(0x656659),
        border: stroke(1.0, 0x656659),
        focus_ring: stroke(2.0, 0xA6E22E),
        info: rgb(0x66D9EF),
        tip: rgb(0xA6E22E),
        warning: rgb(0xE6DB74),
        danger: rgb(0xFF9CB9),
        ..Self::DARK
    };
    /// Original cool-green palette using the standard dark semantic inks.
    pub const SPRUCE: Self = Self {
        primary: rgb(0x8EE1BB),
        on_primary: rgb(0x123126),
        surface: rgb(0x182923),
        surface_elevated: rgb(0x22382E),
        border_strong: stroke(1.0, 0x86AA97),
        focus_ring: stroke(2.0, 0x8EE1BB),
        ..Self::DARK
    };

    /// Original warm-paper palette using the standard light semantic inks.
    pub const PARCHMENT: Self = Self {
        primary: rgb(0x754222),
        on_primary: Color32::WHITE,
        surface: rgb(0xF5EFE3),
        surface_elevated: rgb(0xFFF9EE),
        border_strong: stroke(1.0, 0x806E58),
        focus_ring: stroke(2.0, 0x754222),
        ..Self::LIGHT
    };

    pub const BUBBLEGUM: Self = Self {
        primary: rgb(0xA02D69),
        surface: rgb(0xFCE8F3),
        surface_elevated: rgb(0xFFF6FB),
        divider: rgb(0xD8A8C1),
        text: rgb(0x361D2D),
        text_muted: rgb(0x69475D),
        border: stroke(1.0, 0xD8A8C1),
        focus_ring: stroke(2.0, 0xA02D69),
        info: rgb(0x594190),
        radius: 14,
        ..Self::LIGHT
    };

    pub const MIDNIGHT_CONTRAST: Self = Self {
        surface: rgb(0x061124),
        surface_elevated: rgb(0x101B2C),
        primary: rgb(0x96E8FF),
        focus_ring: stroke(2.0, 0x96E8FF),
        ..Self::high_contrast_dark()
    };

    pub const fn high_contrast_dark() -> Self {
        Self {
            primary: rgb(0xFFDF62),
            on_primary: Color32::BLACK,
            surface: Color32::BLACK,
            surface_elevated: rgb(0x151515),
            text: Color32::WHITE,
            text_muted: rgb(0xCACACA),
            divider: Color32::WHITE,
            border: stroke(1.0, 0xFFFFFF),
            border_strong: stroke(1.0, 0xFFFFFF),
            focus_ring: stroke(2.0, 0xFFDF62),
            info: rgb(0x8FDCFF),
            tip: rgb(0xA0FFAA),
            warning: rgb(0xFFDF62),
            danger: rgb(0xFFB2BD),
            elevation: Shadow::NONE,
            ..Self::DARK
        }
    }

    pub const fn high_contrast_light() -> Self {
        Self {
            primary: rgb(0x182060),
            on_primary: Color32::WHITE,
            surface: Color32::WHITE,
            surface_elevated: rgb(0xF0F0F0),
            text: Color32::BLACK,
            text_muted: rgb(0x424242),
            divider: Color32::BLACK,
            border: stroke(1.0, 0x000000),
            border_strong: stroke(1.0, 0x000000),
            focus_ring: stroke(2.0, 0x242278),
            info: rgb(0x16405C),
            tip: rgb(0x16472C),
            warning: rgb(0x4D3500),
            danger: rgb(0x710D25),
            elevation: Shadow::NONE,
            ..Self::LIGHT
        }
    }

    pub(crate) fn focus(self, ui: &Ui, response: &egui::Response) {
        if response.has_focus() {
            ui.painter().rect_stroke(
                response.rect,
                self.radius,
                self.focus_ring,
                egui::StrokeKind::Inside,
            );
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
    HighContrastLight,
    HighContrastDark,
    Custom(Theme),
}

#[derive(Clone, Copy, Debug)]
pub struct UiTokens {
    pub theme: Theme,
    pub density: Density,
}

impl UiTokens {
    pub(crate) fn configured(ctx: &Context) -> bool {
        ctx.data(|data| data.get_temp::<(ThemeMode, Density)>(Self::id()))
            .is_some()
    }

    /// Uses the installed palette, or derives tokens from the current host surface.
    pub fn for_surface(ui: &Ui, surface: crate::Surface) -> Self {
        if let Some(tokens) = Self::installed(ui.ctx()) {
            return tokens;
        }
        let existing = crate::Theme::of(ui);
        let base = if ui.visuals().dark_mode {
            Theme::DARK
        } else {
            Theme::LIGHT
        };
        Self {
            theme: Theme {
                surface: existing.background(surface),
                surface_elevated: existing.inset,
                primary: existing.primary,
                on_primary: existing.on_primary,
                text: existing.text,
                text_muted: existing.muted_text,
                border: Stroke::new(1.0, existing.border),
                border_strong: Stroke::new(1.0, existing.border),
                focus_ring: Stroke::new(2.0, existing.focus),
                ..base
            },
            density: if ui.spacing().interact_size.y >= Density::Comfy.row_height() {
                Density::Comfy
            } else {
                Density::Compact
            },
        }
    }
    pub fn install(ctx: &Context, mode: ThemeMode, density: Density) {
        if ctx.data(|data| data.get_temp::<(ThemeMode, Density)>(Self::id()))
            == Some((mode, density))
        {
            return;
        }
        let preference = match mode {
            ThemeMode::System => ThemePreference::System,
            ThemeMode::Light | ThemeMode::HighContrastLight => ThemePreference::Light,
            ThemeMode::Dark | ThemeMode::HighContrastDark => ThemePreference::Dark,
            ThemeMode::Custom(theme) if theme.dark => ThemePreference::Dark,
            ThemeMode::Custom(_) => ThemePreference::Light,
        };
        ctx.set_theme(preference);
        ctx.data_mut(|data| data.insert_temp(Self::id(), (mode, density)));
        ctx.request_repaint();
    }

    pub fn resolve(ctx: &Context) -> Self {
        let (mode, density) = ctx
            .data(|data| data.get_temp(Self::id()))
            .unwrap_or_default();
        Self::from_mode(ctx, mode, density)
    }

    pub(crate) fn installed(ctx: &Context) -> Option<Self> {
        let (mode, density) = ctx.data(|data| data.get_temp::<(ThemeMode, Density)>(Self::id()))?;
        Some(Self::from_mode(ctx, mode, density))
    }

    fn from_mode(ctx: &Context, mode: ThemeMode, density: Density) -> Self {
        let theme = match mode {
            ThemeMode::System if ctx.system_theme() == Some(egui::Theme::Light) => Theme::LIGHT,
            ThemeMode::System | ThemeMode::Dark => Theme::DARK,
            ThemeMode::Light => Theme::LIGHT,
            ThemeMode::HighContrastLight => Theme::high_contrast_light(),
            ThemeMode::HighContrastDark => Theme::high_contrast_dark(),
            ThemeMode::Custom(theme) => theme,
        };
        Self { theme, density }
    }

    fn id() -> Id {
        Id::unique("orange-ui-controls/tokens")
    }

    pub fn scope<R>(
        self,
        ui: &mut Ui,
        density: Option<Density>,
        show: impl FnOnce(&mut Ui, Self) -> R,
    ) -> R {
        let tokens = Self {
            density: density.unwrap_or(self.density),
            ..self
        };
        ui.scope(|ui| {
            tokens.density.apply(ui);
            let theme = tokens.theme;
            let visuals = ui.visuals_mut();
            visuals.override_text_color = Some(theme.text);
            visuals.dark_mode = theme.dark;
            visuals.weak_text_color = Some(theme.text_muted);
            visuals.warn_fg_color = theme.warning;
            visuals.error_fg_color = theme.danger;
            visuals.faint_bg_color = theme.bg_elevated();
            visuals.panel_fill = theme.surface;
            visuals.window_fill = theme.surface_elevated;
            visuals.extreme_bg_color = theme.surface_elevated;
            visuals.text_edit_bg_color = Some(theme.bg_input());
            visuals.indent_has_left_vline = false;
            visuals.window_stroke = theme.border;
            visuals.selection.bg_fill = theme.primary;
            visuals.selection.stroke = Stroke::new(1.0, theme.on_primary);
            for state in [
                &mut visuals.widgets.noninteractive,
                &mut visuals.widgets.inactive,
                &mut visuals.widgets.hovered,
                &mut visuals.widgets.active,
                &mut visuals.widgets.open,
            ] {
                state.fg_stroke.color = theme.text;
                state.bg_fill = theme.surface_elevated;
                state.weak_bg_fill = theme.surface_elevated;
                state.bg_stroke = theme.border;
                state.expansion = 0.0;
                state.corner_radius = theme.radius.into();
            }
            visuals.widgets.hovered.bg_stroke = theme.border_strong;
            visuals.widgets.active.bg_stroke = theme.focus_ring;
            show(ui, tokens)
        })
        .inner
    }
}

pub(crate) fn contrast(a: Color32, b: Color32) -> f32 {
    let luma = |color| {
        let [r, g, b, _] = egui::Rgba::from(color).to_array();
        r.mul_add(0.2126, g.mul_add(0.7152, b * 0.0722))
    };
    let (a, b) = (luma(a), luma(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

pub(crate) fn ensure_contrast(ink: Color32, background: Color32, minimum: f32) -> Color32 {
    if contrast(ink, background) >= minimum {
        return ink;
    }
    let target = if contrast(Color32::BLACK, background) > contrast(Color32::WHITE, background) {
        Color32::BLACK
    } else {
        Color32::WHITE
    };
    (1..=16u8)
        .map(|step| ink.lerp_to_gamma(target, f32::from(step) / 16.0))
        .find(|&ink| contrast(ink, background) >= minimum)
        .unwrap_or(target)
}

#[derive(Clone, Copy, Debug)]
pub enum Divider {
    Section,
    Inline { height: f32 },
}

impl Divider {
    /// Use Section in vertical layouts and Inline in horizontal layouts.
    /// The surrounding UI owns the content bounds and ordinary item spacing.
    pub fn show(self, ui: &mut Ui, tokens: UiTokens) -> Rect {
        let section_gap = tokens.density.spacing().md;
        let gap = ui.spacing().item_spacing;
        let (size, horizontal) = match self {
            Self::Section => (
                vec2(
                    ui.available_width().max(0.0),
                    1.0 + 2.0 * (section_gap - gap.y).max(0.0),
                ),
                true,
            ),
            Self::Inline { height } => (
                vec2(1.0 + 2.0 * (section_gap - gap.x).max(0.0), height.max(0.0)),
                false,
            ),
        };
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let stroke = Stroke::new(1.0, tokens.theme.divider);
        if horizontal {
            ui.painter().hline(rect.x_range(), rect.center().y, stroke);
        } else {
            ui.painter().vline(rect.center().x, rect.y_range(), stroke);
        }
        rect
    }
}
