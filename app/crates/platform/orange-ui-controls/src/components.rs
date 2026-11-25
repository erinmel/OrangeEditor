//! Intrinsic desktop controls. Hosts supply localized labels and own application state.
use crate::UiTokens;
use egui::{
    Button, Color32, Frame, Label, Margin, Rect, Response, RichText, Role, Sense, Stroke,
    StrokeKind, Ui, WidgetInfo, vec2,
};

pub(crate) fn action(ui: &mut Ui, tokens: UiTokens, label: &str) -> Response {
    let theme = tokens.theme;
    let response = ui.add(
        Button::new(RichText::new(label).color(theme.text))
            .truncate()
            .corner_radius(theme.radius)
            .min_size(vec2(0.0, tokens.density.row_height())),
    );
    theme.focus(ui, &response);
    truncated_tooltip(ui, response, label, 2.0 * tokens.density.spacing().md)
}

fn truncated_tooltip(ui: &Ui, response: Response, text: &str, reserved: f32) -> Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let width = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, Color32::WHITE)
        .size()
        .x;
    if width + reserved > response.rect.width() {
        response.on_hover_text(text).on_disabled_hover_text(text)
    } else {
        response
    }
}

/// An animated checked control with an intrinsic, single-row hit target.
pub fn switch(ui: &mut Ui, tokens: UiTokens, value: &mut bool, label: &str) -> Response {
    switch_with_thumb(ui, tokens, value, label, |_, _, _, _| {})
}

/// Draws an animated switch with a host-provided icon inside its thumb.
pub fn switch_with_thumb(
    ui: &mut Ui,
    tokens: UiTokens,
    value: &mut bool,
    label: &str,
    draw: impl FnOnce(&Ui, Rect, Color32, bool),
) -> Response {
    let theme = tokens.theme;
    let s = tokens.density.spacing();
    let height = tokens.density.row_height();
    let track_width = height + s.sm;
    let font = egui::TextStyle::Button.resolve(ui.style());
    let text = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), theme.text);
    let width = (track_width + s.sm + text.size().x)
        .min(ui.available_width())
        .max(0.0);
    let (rect, mut response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response
        .widget_info(|| WidgetInfo::selected(Role::CheckBox, response.enabled(), *value, label));
    let painter = ui.painter_at(rect);
    let track = Rect::from_min_size(
        rect.left_center() - vec2(0.0, height * 0.30),
        vec2(track_width.min(width), height * 0.60),
    );
    let radius = track.height() * 0.5;
    let progress = ui
        .ctx()
        .animate_bool_with_time(response.id, *value, ui.style().animation_time);
    let fill = if *value {
        theme.primary
    } else {
        theme.surface_elevated
    };
    let thumb = if *value { theme.on_primary } else { theme.text };
    let border = if response.hovered() {
        theme.border_strong
    } else {
        theme.control_border()
    };
    painter.rect(track, 255, fill, border, StrokeKind::Inside);
    let travel = (track.width() - 2.0 * radius).max(0.0);
    let center = track.left_center() + vec2(radius + travel * progress, 0.0);
    let thumb_radius = (radius - 2.0).max(0.0);
    painter.circle_filled(center, thumb_radius, thumb);
    let thumb_rect = Rect::from_center_size(center, vec2(thumb_radius, thumb_radius) * 1.5);
    draw(ui, thumb_rect, fill, *value);
    let label_width = (width - track_width - s.sm).max(0.0);
    if label_width > 0.0 {
        let mut job =
            egui::text::LayoutJob::simple(label.to_owned(), font, theme.text, label_width);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        painter.galley(
            rect.left_center() + vec2(track_width + s.sm, -galley.size().y * 0.5),
            galley,
            theme.text,
        );
    }
    theme.focus(ui, &response);
    truncated_tooltip(ui, response, label, track_width + s.sm)
}

pub(crate) fn surface_frame(tokens: UiTokens) -> Frame {
    let theme = tokens.theme;
    Frame::new()
        .fill(theme.surface_elevated)
        .stroke(theme.border)
        .corner_radius(theme.radius)
        .shadow(theme.elevation)
        .inner_margin(Margin::same(tokens.density.spacing().md as i8))
}

pub fn card<R>(
    ui: &mut Ui,
    tokens: UiTokens,
    show: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    surface_frame(tokens).show(ui, show)
}

#[derive(Clone, Copy, Debug)]
pub struct PasswordLabels<'a> {
    pub field: &'a str,
    pub reveal: &'a str,
    pub hide: &'a str,
}

/// Single framed field with an independently focusable reveal control.
pub fn password(
    ui: &mut Ui,
    tokens: UiTokens,
    value: &mut String,
    revealed: &mut bool,
    labels: PasswordLabels<'_>,
    error: Option<&str>,
) -> Response {
    let theme = tokens.theme;
    let spacing = tokens.density.spacing();
    let row = tokens.density.row_height();
    let outline = ui.painter().add(egui::Shape::Noop);
    let content = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let width = ui.available_width();
        let eye = row.min(width);
        let inset = (2.0 * spacing.sm).min((width - eye).max(0.0));
        ui.add_space(inset);
        let field = ui
            .add(
                egui::TextEdit::singleline(value)
                    .password(!*revealed)
                    .align(egui::Align2::LEFT_CENTER)
                    .frame(Frame::NONE)
                    .margin(Margin::ZERO)
                    .desired_width((width - eye - inset).max(0.0))
                    .min_size(vec2(0.0, row)),
            )
            .accessible_name(labels.field);
        let (rect, reveal) = ui.allocate_exact_size(vec2(eye, row), Sense::click());
        if reveal.clicked() {
            *revealed = !*revealed;
        }
        let label = if *revealed {
            labels.hide
        } else {
            labels.reveal
        };
        reveal.widget_info(|| {
            WidgetInfo::selected(Role::CheckBox, reveal.enabled(), *revealed, label)
        });
        let painter = ui.painter_at(rect);
        if reveal.hovered() {
            painter.rect_stroke(rect, theme.radius, theme.border_strong, StrokeKind::Inside);
        }
        #[cfg(feature = "icons")]
        crate::icons::draw_centered_icon(
            ui,
            rect,
            crate::icons::UiIcon::Lucide(if *revealed {
                crate::icons::Lucide::Eye
            } else {
                crate::icons::Lucide::EyeOff
            }),
            theme.text_primary(),
        );
        #[cfg(not(feature = "icons"))]
        {
            let center = rect.center();
            let stroke = Stroke::new(1.5, theme.text);
            let left = center - vec2(7.0, 0.0);
            let right = center + vec2(7.0, 0.0);
            painter.add(egui::Shape::closed_line(
                vec![
                    left,
                    center - vec2(0.0, 4.0),
                    right,
                    center + vec2(0.0, 4.0),
                ],
                stroke,
            ));
            painter.circle_filled(center, 2.0, theme.text);
            if !*revealed {
                painter.line_segment([center - vec2(6.0, 6.0), center + vec2(6.0, 6.0)], stroke);
            }
        }
        theme.focus(ui, &reveal);
        reveal.on_hover_text(label);
        field
    });
    ui.painter().set(
        outline,
        egui::epaint::RectShape::new(
            content.response.rect,
            theme.radius,
            theme.bg_input(),
            if content.inner.has_focus() {
                theme.focus_ring
            } else if error.is_some() {
                Stroke::new(1.0, theme.danger)
            } else {
                theme.control_border()
            },
            StrokeKind::Inside,
        ),
    );
    if let Some(error) = error {
        validation_error(ui, tokens, &content.inner, error);
    }
    content.inner
}

fn validation_error(ui: &mut Ui, tokens: UiTokens, response: &Response, error: &str) {
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_invalid(egui::accesskit::Invalid::True);
        node.set_description(error);
    });
    ui.add(Label::new(RichText::new(format!("! {error}")).color(tokens.theme.danger)).wrap());
}

/// A framed trigger for single-file, multi-file or folder requests. The host
/// supplies the display string and the unabbreviated path or list.
#[derive(Debug)]
pub struct PathPicker<'a> {
    pub name: &'a str,
    pub display: &'a str,
    pub full_selection: &'a str,
    pub error: Option<&'a str>,
}

impl PathPicker<'_> {
    pub fn show(&self, ui: &mut Ui, tokens: UiTokens) -> Response {
        let theme = tokens.theme;
        #[cfg(feature = "icons")]
        let trigger = Button::new((
            crate::icons::UiIcon::Lucide(crate::icons::Lucide::FolderOpen).atom(),
            self.name,
        ));
        #[cfg(not(feature = "icons"))]
        let trigger = Button::new(self.name);
        let response = ui.add(
            trigger
                .truncate()
                .fill(theme.surface_elevated)
                .stroke(if self.error.is_some() {
                    Stroke::new(1.0, theme.danger)
                } else {
                    theme.control_border()
                })
                .corner_radius(theme.radius)
                .min_size(vec2(0.0, tokens.density.row_height())),
        );
        if !self.display.is_empty() {
            ui.add(Label::new(RichText::new(self.display).color(theme.text_muted)).wrap());
        }
        theme.focus(ui, &response);
        let name = if self.display.is_empty() {
            self.name.to_owned()
        } else {
            format!("{}: {}", self.name, self.display)
        };
        let response = response.accessible_name(name);
        if let Some(error) = self.error {
            validation_error(ui, tokens, &response, error);
        }
        if self.full_selection.is_empty() {
            response
        } else {
            response
                .on_hover_text(self.full_selection)
                .on_disabled_hover_text(self.full_selection)
        }
    }
}

/// Run the native adapter only after the picker response activates.
#[cfg(all(
    feature = "native-dialogs",
    any(target_os = "windows", target_os = "linux", target_os = "macos")
))]
#[derive(Clone, Copy, Debug)]
pub enum NativeSelection {
    File,
    Files,
    Folder,
}

#[cfg(all(
    feature = "native-dialogs",
    any(target_os = "windows", target_os = "linux", target_os = "macos")
))]
impl NativeSelection {
    pub fn pick(self, dialog: rfd::FileDialog) -> Option<Vec<std::path::PathBuf>> {
        match self {
            Self::File => dialog.pick_file().map(|path| vec![path]),
            Self::Files => dialog.pick_files(),
            Self::Folder => dialog.pick_folder().map(|path| vec![path]),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MenuItem<'a, T> {
    pub id: T,
    pub label: &'a str,
    pub icon: Option<&'a str>,
    pub shortcut: Option<&'a str>,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum MenuTrigger {
    Button,
    Context,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum MenuLayout {
    #[default]
    List,
    Wrap,
}

/// The caller can edit the item list between frames. Returns the selected ID.
pub fn menu<T: Copy + std::hash::Hash + std::fmt::Debug>(
    ui: &Ui,
    tokens: UiTokens,
    trigger: &Response,
    kind: MenuTrigger,
    layout: MenuLayout,
    items: &[MenuItem<'_, T>],
) -> Option<T> {
    menu_layout(
        ui,
        tokens,
        trigger,
        kind,
        layout,
        items,
        |ui, tokens, item| match layout {
            MenuLayout::List => menu_row(ui, tokens, item.label, item.icon, item.shortcut, false),
            MenuLayout::Wrap => {
                let text = [item.icon, Some(item.label), item.shortcut]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join("  ");
                ui.add(Button::new(text).frame(false))
            }
        },
    )
}

/// Vertical menu using a host painter for the fixed icon column.
pub fn menu_with_icons<T: Copy + std::hash::Hash + std::fmt::Debug>(
    ui: &Ui,
    tokens: UiTokens,
    trigger: &Response,
    kind: MenuTrigger,
    items: &[MenuItem<'_, T>],
    draw: impl Fn(T, &Ui, Rect, Color32),
) -> Option<T> {
    menu_layout(
        ui,
        tokens,
        trigger,
        kind,
        MenuLayout::List,
        items,
        |ui, tokens, item| {
            menu_row_with_icon(
                ui,
                tokens,
                item.label,
                item.shortcut,
                false,
                |ui, rect, ink| draw(item.id, ui, rect, ink),
            )
        },
    )
}

fn menu_layout<T: Copy + std::hash::Hash + std::fmt::Debug>(
    ui: &Ui,
    tokens: UiTokens,
    trigger: &Response,
    kind: MenuTrigger,
    layout: MenuLayout,
    items: &[MenuItem<'_, T>],
    render: impl Fn(&mut Ui, UiTokens, &MenuItem<'_, T>) -> Response,
) -> Option<T> {
    let keyboard_open = trigger.clicked() && !trigger.clicked_by(egui::PointerButton::Primary);
    let popup_id = egui::Popup::default_response_id(trigger);
    let anchor_id = popup_id.with("pointer-anchor");
    if trigger.secondary_clicked() || keyboard_open {
        ui.ctx()
            .data_mut(|data| data.insert_temp(anchor_id, !keyboard_open));
    }
    let pointer_anchor = ui
        .ctx()
        .data(|data| data.get_temp(anchor_id))
        .unwrap_or(true);
    let was_open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    let popup = match kind {
        MenuTrigger::Button => egui::Popup::menu(trigger),
        MenuTrigger::Context if !pointer_anchor => egui::Popup::menu(trigger),
        MenuTrigger::Context => egui::Popup::context_menu(trigger),
    };
    let open = popup
        .width(ui.max_rect().width().clamp(48.0, 230.0))
        .frame(
            Frame::popup(ui.style())
                .fill(tokens.theme.bg_elevated())
                .stroke(tokens.theme.border_subtle()),
        )
        .show(|popup_ui| {
            tokens.scope(popup_ui, None, |ui, tokens| {
                let mut choices = Vec::with_capacity(items.len());
                let mut selected = None;
                let rows = |ui: &mut Ui| {
                    for item in items {
                        ui.push_id(item.id, |ui| {
                            ui.add_enabled_ui(item.enabled, |ui| {
                                let response = render(ui, tokens, item);
                                response.widget_info(|| {
                                    WidgetInfo::labeled(
                                        Role::MenuItem,
                                        response.enabled(),
                                        item.label,
                                    )
                                });
                                if let Some(shortcut) = item.shortcut {
                                    ui.ctx().accesskit_node_builder(response.id, |node| {
                                        node.set_keyboard_shortcut(shortcut)
                                    });
                                }
                                if response.clicked() {
                                    selected = Some(item.id);
                                }
                                if response.enabled() {
                                    choices.push(response);
                                }
                            });
                        });
                    }
                };
                match layout {
                    MenuLayout::List => {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.vertical(rows);
                    }
                    MenuLayout::Wrap => {
                        ui.horizontal_wrapped(rows);
                    }
                }
                let [up, down, left, right] = crate::consume_plain_keys(
                    ui,
                    [
                        egui::Key::ArrowUp,
                        egui::Key::ArrowDown,
                        egui::Key::ArrowLeft,
                        egui::Key::ArrowRight,
                    ],
                );
                let (back, forward) = (up || left, down || right);
                if !choices.is_empty() && (back || forward) {
                    let current = choices.iter().position(Response::has_focus);
                    let next = match (current, back) {
                        (Some(i), false) => (i + 1) % choices.len(),
                        (Some(i), true) => (i + choices.len() - 1) % choices.len(),
                        (None, true) => choices.len() - 1,
                        (None, false) => 0,
                    };
                    choices[next].request_focus();
                } else if keyboard_open && let Some(first) = choices.first() {
                    first.request_focus();
                }
                if selected.is_some() {
                    ui.close();
                    trigger.request_focus();
                }
                selected
            })
        });
    let is_open = egui::Popup::is_id_open(ui.ctx(), popup_id);
    if was_open && !is_open && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        trigger.request_focus();
    }
    ui.ctx()
        .accesskit_node_builder(trigger.id, |node| node.set_expanded(is_open));
    open.and_then(|response| response.inner)
}

/// A borderless row for floating menus. The caller owns submenu and command state.
pub fn menu_row(
    ui: &mut Ui,
    tokens: UiTokens,
    label: &str,
    icon: Option<&str>,
    shortcut: Option<&str>,
    submenu: bool,
) -> Response {
    menu_row_with_icon(ui, tokens, label, shortcut, submenu, |ui, rect, ink| {
        if let Some(icon) = icon {
            ui.painter_at(rect).text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                icon,
                egui::TextStyle::Body.resolve(ui.style()),
                ink,
            );
        }
    })
}

/// Borderless menu row with a fixed icon slot for a centered vector or glyph.
pub fn menu_row_with_icon(
    ui: &mut Ui,
    tokens: UiTokens,
    label: &str,
    shortcut: Option<&str>,
    submenu: bool,
    draw: impl FnOnce(&Ui, Rect, Color32),
) -> Response {
    use egui::{Align2, pos2};
    let theme = tokens.theme;
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width().max(1.0), tokens.density.row_height()),
        Sense::click(),
    );
    let painter = ui.painter_at(rect);
    if response.hovered() || response.has_focus() {
        painter.rect_filled(
            rect.shrink(1.0),
            4,
            theme
                .bg_elevated()
                .lerp_to_gamma(theme.accent_primary(), 0.15),
        );
    }
    theme.focus(ui, &response);
    let pad = tokens.density.spacing().sm;
    let font = egui::TextStyle::Body.resolve(ui.style());
    let mut right = rect.right() - pad;
    if submenu {
        painter.text(
            pos2(right, rect.center().y),
            Align2::RIGHT_CENTER,
            "▸",
            font.clone(),
            theme.text_muted,
        );
        right -= 14.0;
    }
    if let Some(shortcut) = shortcut {
        let galley = painter.layout_no_wrap(shortcut.to_owned(), font.clone(), theme.text_muted);
        if galley.size().x < rect.width() * 0.44 {
            let size = galley.size();
            painter.galley(
                pos2(right - size.x, rect.center().y - size.y * 0.5),
                galley,
                theme.text_muted,
            );
            right -= size.x + pad;
        }
    }
    let left = rect.left() + pad + 22.0;
    let icon_rect = Rect::from_center_size(
        pos2(rect.left() + pad + 9.0, rect.center().y),
        vec2(18.0, rect.height()),
    );
    draw(ui, icon_rect.intersect(rect), theme.text_muted);
    let mut job =
        egui::text::LayoutJob::simple_singleline(label.to_owned(), font, theme.text_primary());
    job.wrap.max_width = (right - left).max(1.0);
    job.wrap.max_rows = 1;
    let galley = painter.layout_job(job);
    painter.galley(
        pos2(left, rect.center().y - galley.size().y * 0.5),
        galley,
        theme.text_primary(),
    );
    response.widget_info(|| WidgetInfo::labeled(Role::MenuItem, response.enabled(), label));
    if let Some(shortcut) = shortcut {
        ui.ctx()
            .accesskit_node_builder(response.id, |node| node.set_keyboard_shortcut(shortcut));
    }
    response
}

/// Inserts a single physical pixel separator between menu groups.
pub fn menu_separator(ui: &mut Ui, tokens: UiTokens) {
    let pixel = ui.ctx().pixels_per_point().recip();
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), pixel), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(pixel, tokens.theme.divider),
    );
}
/// A themed modal whose contents inherit the Comfy metrics.
pub fn modal<R>(
    ctx: &egui::Context,
    id: egui::Id,
    tokens: UiTokens,
    show: impl FnOnce(&mut Ui, UiTokens) -> R,
) -> egui::ModalResponse<R> {
    let effective = UiTokens {
        density: crate::Density::Comfy,
        ..tokens
    };
    egui::Modal::new(id)
        .backdrop_color(Color32::from_black_alpha(160))
        .frame(surface_frame(effective))
        .show(ctx, |ui| effective.scope(ui, None, show))
}

/// Shows a host-decoded video frame at its original aspect ratio, scaled to fit.
/// The host owns decoding, texture updates and repaint scheduling. Use a stable
/// texture ID across frames so pause and incidental UI paints reuse the image.
pub fn video_frame(
    ui: &mut Ui,
    texture: egui::load::SizedTexture,
    max_size: egui::Vec2,
    label: &str,
) -> Response {
    let bounds = max_size.min(ui.available_size()).max(egui::Vec2::ZERO);
    let scale = if texture.size.x > 0.0 && texture.size.y > 0.0 {
        (bounds.x / texture.size.x).min(bounds.y / texture.size.y)
    } else {
        0.0
    };
    ui.add(
        egui::Image::from_texture(texture)
            .fit_to_exact_size(texture.size * scale)
            .alt_text(label),
    )
}

#[cfg(test)]
#[test]
fn video_frame_fits_wide_and_portrait_sources_without_stretching() {
    let ctx = egui::Context::default();
    ctx.run_ui(Default::default(), |ui| {
        for (source, expected) in [
            (vec2(1920.0, 1080.0), vec2(160.0, 90.0)),
            (vec2(1080.0, 1920.0), vec2(90.0, 160.0)),
        ] {
            let texture = egui::load::SizedTexture::new(egui::TextureId::User(1), source);
            let response = video_frame(ui, texture, egui::Vec2::splat(160.0), "Video");
            assert_eq!(response.rect.size(), expected);
        }
    })
    .drop_without_applying_deltas();
}
