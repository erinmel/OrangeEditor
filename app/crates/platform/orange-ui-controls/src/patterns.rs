use egui::{Frame, Label, Response, RichText, Sense, Stroke, StrokeKind, Ui, WidgetInfo, vec2};

use crate::{UiTokens, components};

/// An intrinsic-height card with independent header, body and footer slots.
pub fn structured_card<R>(
    ui: &mut Ui,
    tokens: UiTokens,
    header: impl FnOnce(&mut Ui),
    body: impl FnOnce(&mut Ui),
    footer: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    components::card(ui, tokens, |ui| {
        ui.set_min_width(ui.available_width());
        header(ui);
        crate::Divider::Section.show(ui, tokens);
        body(ui);
        crate::Divider::Section.show(ui, tokens);
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), tokens.density.row_height()),
            egui::Layout::right_to_left(egui::Align::Center),
            footer,
        )
        .inner
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckState {
    Off,
    Mixed,
    On,
}

/// The host computes parent state and applies toggles to its descendants.
pub fn tri_checkbox(
    ui: &mut Ui,
    tokens: UiTokens,
    label: &str,
    state: &mut CheckState,
) -> Response {
    let active = *state == CheckState::On;
    let mixed = *state == CheckState::Mixed;
    let theme = tokens.theme;
    let mut response = ui
        .horizontal(|ui| {
            let size = tokens.density.row_height() * 0.64;
            let (rect, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
            let fill = if active || mixed {
                theme.accent_primary()
            } else {
                theme.bg_elevated()
            };
            ui.painter()
                .rect(rect, 4, fill, theme.border_subtle(), StrokeKind::Inside);
            if mixed {
                ui.painter().hline(
                    rect.center().x - size * 0.24..=rect.center().x + size * 0.24,
                    rect.center().y,
                    Stroke::new(2.0, theme.on_primary),
                );
            } else if active {
                check_mark(ui, rect, theme.on_primary);
            }
            ui.add(Label::new(label).wrap().selectable(false));
        })
        .response
        .interact(Sense::click());
    if response.clicked() {
        *state = if active {
            CheckState::Off
        } else {
            CheckState::On
        };
        response.mark_changed();
    }
    response.widget_info(|| {
        WidgetInfo::selected(
            egui::Role::CheckBox,
            response.enabled(),
            *state == CheckState::On,
            label,
        )
    });
    if *state == CheckState::Mixed {
        ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_toggled(egui::accesskit::Toggled::Mixed)
        });
    }
    theme.focus(ui, &response);
    response
}

fn check_mark(ui: &Ui, rect: egui::Rect, color: egui::Color32) {
    let center = rect.center();
    let size = rect.width().min(rect.height());
    ui.painter().add(egui::Shape::line(
        vec![
            center + vec2(-0.25, 0.0) * size,
            center + vec2(-0.07, 0.18) * size,
            center + vec2(0.27, -0.21) * size,
        ],
        Stroke::new(1.8, color),
    ));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionStyle {
    Inline,
    Description,
    Card,
    Chip,
}

/// An option row. The host chooses radio or checkbox semantics and owns the
/// selection. The leading slot accepts any widget, including a bundled icon.
#[derive(Debug)]
pub struct OptionRow<'a> {
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub shortcut: Option<&'a str>,
    pub style: OptionStyle,
    pub exclusive: bool,
}

impl OptionRow<'_> {
    pub fn show(
        &self,
        ui: &mut Ui,
        tokens: UiTokens,
        selected: bool,
        leading: impl FnOnce(&mut Ui),
    ) -> Response {
        let theme = tokens.theme;
        let framed = matches!(self.style, OptionStyle::Card | OptionStyle::Chip);
        let mut frame =
            Frame::new().inner_margin(egui::Margin::same(tokens.density.spacing().sm as i8));
        if framed {
            frame = frame
                .corner_radius(if self.style == OptionStyle::Chip {
                    255
                } else {
                    theme.radius
                })
                .fill(if selected {
                    theme
                        .bg_elevated()
                        .lerp_to_gamma(theme.accent_primary(), 0.15)
                } else {
                    theme.bg_surface()
                })
                .stroke(if selected {
                    theme.focus_ring
                } else {
                    theme.border_subtle()
                });
        }
        let marker = tokens.density.row_height() * 0.6;
        let mut mark = egui::Rect::NOTHING;
        let response = frame
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    if self.style == OptionStyle::Card && !self.exclusive {
                        leading(ui);
                    }
                    let (rect, _) = ui.allocate_exact_size(vec2(marker, marker), Sense::hover());
                    mark = rect;
                    ui.vertical(|ui| {
                        ui.add(
                            Label::new(RichText::new(self.title).strong())
                                .wrap()
                                .selectable(false),
                        );
                        if let Some(description) = self.description {
                            ui.add(
                                Label::new(RichText::new(description).color(theme.text_muted))
                                    .wrap()
                                    .selectable(false),
                            );
                        }
                        if let Some(shortcut) = self.shortcut {
                            ui.add(
                                Label::new(
                                    RichText::new(shortcut).monospace().color(theme.text_muted),
                                )
                                .wrap()
                                .selectable(false),
                            );
                        }
                    });
                });
            })
            .response
            .interact(Sense::click());
        let role = if self.exclusive {
            egui::Role::RadioButton
        } else {
            egui::Role::CheckBox
        };
        response
            .widget_info(|| WidgetInfo::selected(role, response.enabled(), selected, self.title));
        if self.exclusive {
            ui.painter()
                .circle_stroke(mark.center(), mark.width() * 0.42, theme.border_strong);
            let active = ui.ctx().animate_bool(response.id, selected);
            if active > 0.0 {
                ui.painter().circle_filled(
                    mark.center(),
                    mark.width() * 0.26 * active,
                    theme.accent_primary(),
                );
            }
        } else {
            ui.painter().rect(
                mark.shrink(1.0),
                4,
                if selected {
                    theme.accent_primary()
                } else {
                    theme.bg_elevated()
                },
                theme.border_subtle(),
                StrokeKind::Inside,
            );
            if selected {
                check_mark(ui, mark, theme.on_primary);
            }
        }
        theme.focus(ui, &response);
        response
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Info,
    Success,
    Warning,
    Danger,
}

impl Tone {
    pub fn ink(self, tokens: UiTokens) -> egui::Color32 {
        match self {
            Self::Info => tokens.theme.info,
            Self::Success => tokens.theme.success(),
            Self::Warning => tokens.theme.warning,
            Self::Danger => tokens.theme.danger,
        }
    }
}

pub fn badge(ui: &mut Ui, tokens: UiTokens, tone: Tone, label: &str) -> Response {
    let ink = tone.ink(tokens);
    Frame::new()
        .fill(tokens.theme.bg_surface().lerp_to_gamma(ink, 0.12))
        .corner_radius(255)
        .inner_margin(egui::Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.add(Label::new(RichText::new(label).color(ink)).wrap())
        })
        .response
}

pub fn progress(ui: &mut Ui, tokens: UiTokens, value: Option<f32>, label: &str) -> Response {
    progress_track(ui, tokens, value, label, tokens.density.spacing().md)
}

fn progress_track(
    ui: &mut Ui,
    tokens: UiTokens,
    value: Option<f32>,
    label: &str,
    thickness: f32,
) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), thickness), Sense::hover());
    let theme = tokens.theme;
    response.widget_info(|| {
        WidgetInfo::labeled(egui::Role::ProgressIndicator, response.enabled(), label)
    });
    if ui.is_rect_visible(rect) {
        let range = if let Some(value) = value {
            0.0..=value.clamp(0.0, 1.0)
        } else {
            let t = ui.input(|input| input.time).rem_euclid(1.5) as f32 / 1.5;
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(32));
            let start = t * 1.3 - 0.3;
            start.max(0.0)..=(start + 0.3).min(1.0)
        };
        ui.painter().rect_filled(rect, 255, theme.bg_elevated());
        let start = egui::lerp(rect.x_range(), *range.start());
        let end = egui::lerp(rect.x_range(), *range.end());
        if end > start {
            ui.painter().rect_filled(
                egui::Rect::from_x_y_ranges(start..=end, rect.y_range()),
                255,
                theme.accent_primary(),
            );
        }
    }
    response
}

pub fn skeleton(ui: &mut Ui, tokens: UiTokens, rows: &[f32]) {
    for &fraction in rows {
        let size = vec2(
            ui.available_width() * fraction.clamp(0.0, 1.0),
            tokens.density.spacing().md,
        );
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        ui.painter().rect_filled(
            rect,
            4,
            tokens
                .theme
                .bg_elevated()
                .lerp_to_gamma(tokens.theme.text_muted, 0.13),
        );
    }
}

pub fn empty_state(ui: &mut Ui, tokens: UiTokens, title: &str, description: &str) {
    components::card(ui, tokens, |ui| {
        ui.strong(title);
        ui.weak(description);
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastAction {
    None,
    Dismiss,
    Undo,
}

#[derive(Debug)]
pub struct Toast<'a> {
    pub tone: Tone,
    pub message: &'a str,
    pub remaining: Option<(f32, &'a str)>,
    pub dismiss_label: &'a str,
    pub undo_label: Option<&'a str>,
}

/// A notification card. The host chooses stack placement, timing and actions.
pub fn toast(ui: &mut Ui, tokens: UiTokens, spec: Toast<'_>) -> ToastAction {
    toast_with_body(ui, tokens, spec, None)
}

/// Host-selected timing. Timed notices accept 2 through 120 seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToastLifetime(Option<std::time::Duration>);
impl ToastLifetime {
    pub const PERSISTENT: Self = Self(None);
    pub fn timed(duration: std::time::Duration) -> Self {
        Self(Some(duration.clamp(
            std::time::Duration::from_secs(2),
            std::time::Duration::from_secs(120),
        )))
    }
    pub fn remaining(self, elapsed: std::time::Duration) -> Option<f32> {
        self.0
            .map(|duration| (1.0 - elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0))
    }
}

pub fn toast_with_body(
    ui: &mut Ui,
    tokens: UiTokens,
    spec: Toast<'_>,
    body: Option<&str>,
) -> ToastAction {
    components::card(ui, tokens, |ui| {
        badge(ui, tokens, spec.tone, spec.message);
        if let Some(body) = body {
            ui.add(Label::new(body).wrap());
        }
        let action = ui
            .horizontal_wrapped(|ui| {
                let undo = spec
                    .undo_label
                    .is_some_and(|label| ui.button(label).clicked());
                let dismiss = ui.button(spec.dismiss_label).clicked();
                if undo {
                    ToastAction::Undo
                } else if dismiss {
                    ToastAction::Dismiss
                } else {
                    ToastAction::None
                }
            })
            .inner;
        if let Some((remaining, time_label)) = spec.remaining {
            progress_track(ui, tokens, Some(remaining), time_label, 2.0);
        }
        action
    })
    .inner
}

/// Structured dialog over a viewport-wide backdrop. The caller owns open state
/// and handles the actions returned by its footer.
pub fn dialog<R, A>(
    ctx: &egui::Context,
    id: egui::Id,
    tokens: UiTokens,
    title: &str,
    close_label: &str,
    body: impl FnOnce(&mut Ui) -> R,
    footer: impl FnOnce(&mut Ui, &R) -> A,
) -> egui::ModalResponse<(A, bool)> {
    dialog_with_close(ctx, id, tokens, title, Some(close_label), body, footer)
}

/// Structured dialog with an optional header close action.
pub fn dialog_with_close<R, A>(
    ctx: &egui::Context,
    id: egui::Id,
    tokens: UiTokens,
    title: &str,
    close_label: Option<&str>,
    body: impl FnOnce(&mut Ui) -> R,
    footer: impl FnOnce(&mut Ui, &R) -> A,
) -> egui::ModalResponse<(A, bool)> {
    egui::Modal::new(id)
        .backdrop_color(egui::Color32::from_black_alpha(160))
        .frame(
            components::surface_frame(tokens)
                .inner_margin(egui::Margin::same(tokens.density.spacing().xl as i8)),
        )
        .show(ctx, |ui| {
            tokens.scope(ui, None, |ui, tokens| {
                ui.set_width((ctx.content_rect().width() - 48.0).clamp(120.0, 480.0));
                let close = ui
                    .allocate_ui_with_layout(
                        vec2(ui.available_width(), tokens.density.row_height()),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            let close = close_label.is_some_and(|label| {
                                let side = tokens.density.row_height();
                                let response = ui
                                    .add_sized([side, side], egui::Button::new("×"))
                                    .accessible_name(label)
                                    .on_hover_text(label);
                                if response.clicked() {
                                    ui.close();
                                }
                                response.clicked()
                            });
                            ui.with_layout(
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.add(Label::new(RichText::new(title).heading()).wrap());
                                },
                            );
                            close
                        },
                    )
                    .inner;
                ui.add_space(tokens.density.spacing().lg);
                let result = egui::ScrollArea::vertical()
                    .max_height(ctx.content_rect().height() * 0.55)
                    .show(ui, body)
                    .inner;
                ui.add_space(tokens.density.spacing().lg);
                let action = ui
                    .allocate_ui_with_layout(
                        vec2(ui.available_width(), tokens.density.row_height()),
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| footer(ui, &result),
                    )
                    .inner;
                (action, close)
            })
        })
}

#[cfg(test)]
#[test]
fn toast_duration_clamps_and_persistent_notices_never_expire() {
    use std::time::Duration as D;
    assert_eq!(ToastLifetime::PERSISTENT.remaining(D::MAX), None);
    assert_eq!(
        ToastLifetime::timed(D::ZERO).remaining(D::from_secs(1)),
        Some(0.5)
    );
    assert_eq!(
        ToastLifetime::timed(D::MAX).remaining(D::from_secs(120)),
        Some(0.0)
    );
}

#[cfg(test)]
#[test]
fn clicking_modal_close_reports_cancellation() {
    let ctx = egui::Context::default();
    let mut closed = false;
    let mut frame = |events| {
        let output = ctx.run_ui(
            egui::RawInput {
                events,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    vec2(800.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                let modal = dialog(
                    ui.ctx(),
                    egui::Id::unique("close-test"),
                    UiTokens::resolve(ui.ctx()),
                    "Session",
                    "Close dialog",
                    |ui| ui.label("Body"),
                    |ui, _| ui.button("Done").clicked(),
                );
                closed |= modal.inner.1;
            },
        );
        let point = output.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == "×" => Some(t.pos + t.galley.size() * 0.5),
            _ => None,
        });
        output.drop_without_applying_deltas();
        point
    };
    frame(vec![]);
    let pos = frame(vec![]).expect("close button appears in modal header");
    frame(vec![egui::Event::PointerMoved(pos)]);
    for pressed in [true, false] {
        frame(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }]);
    }
    assert!(closed);
}
