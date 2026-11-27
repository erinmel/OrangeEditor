use egui::{Label, RichText, Ui};

use crate::{Divider, UiTokens, theme::Theme};

#[derive(Clone, Copy, Debug)]
pub enum NoteTone {
    Info,
    Tip,
    Warning,
    Danger,
}

impl NoteTone {
    fn ink(self, theme: Theme) -> egui::Color32 {
        match self {
            Self::Info => theme.info,
            Self::Tip => theme.success(),
            Self::Warning => theme.warning,
            Self::Danger => theme.danger,
        }
    }

    fn paint(self, ui: &Ui, rect: egui::Rect, ink: egui::Color32) {
        let center = rect.center();
        let painter = ui.painter_at(rect);
        let stroke = egui::Stroke::new(1.5, ink);
        let offset = |x, y| center + egui::vec2(x, y);
        match self {
            Self::Warning => {
                painter.add(egui::Shape::closed_line(
                    vec![offset(0.0, -7.0), offset(7.0, 6.0), offset(-7.0, 6.0)],
                    stroke,
                ));
            }
            _ => {
                painter.circle_stroke(center, 7.0, stroke);
            }
        }
        match self {
            Self::Info => {
                painter.circle_filled(offset(0.0, -3.0), 1.0, ink);
                painter.line_segment([offset(0.0, 0.0), offset(0.0, 4.0)], stroke);
            }
            Self::Tip => {
                painter.add(egui::Shape::line(
                    vec![offset(-3.0, 0.0), offset(-1.0, 2.0), offset(3.0, -2.0)],
                    stroke,
                ));
            }
            Self::Warning => {
                painter.line_segment([offset(0.0, -2.0), offset(0.0, 1.0)], stroke);
                painter.circle_filled(offset(0.0, 3.0), 0.8, ink);
            }
            Self::Danger => {
                for sign in [-1.0, 1.0] {
                    painter
                        .line_segment([offset(-2.5, -2.5 * sign), offset(2.5, 2.5 * sign)], stroke);
                }
            }
        }
    }
}
#[derive(Debug, Default)]
pub struct NoteState {
    pub collapsed: bool,
    pub dismissed: bool,
}

#[derive(Debug)]
pub struct Note<'a> {
    pub tone: NoteTone,
    pub title: &'a str,
    pub body: &'a str,
    pub collapse_labels: Option<[&'a str; 2]>,
    pub dismiss_label: Option<&'a str>,
}

impl Note<'_> {
    pub fn show(
        &self,
        ui: &mut Ui,
        tokens: UiTokens,
        state: &mut NoteState,
    ) -> Option<egui::Response> {
        if state.dismissed {
            return None;
        }
        let theme = tokens.theme;
        let accent = self.tone.ink(theme);
        let frame = crate::components::surface_frame(tokens)
            .fill(theme.bg_surface().lerp_to_gamma(accent, 0.09))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width().max(0.0));
                ui.horizontal_top(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::Vec2::splat(18.0), egui::Sense::hover());
                    self.tone.paint(ui, rect, accent);
                    ui.add(
                        Label::new(RichText::new(self.title).heading().color(theme.text)).wrap(),
                    );
                });

                if !state.collapsed {
                    Divider::Section.show(ui, tokens);
                    ui.add(Label::new(RichText::new(self.body).color(theme.text)).wrap());
                }

                if self.collapse_labels.is_some() || self.dismiss_label.is_some() {
                    ui.horizontal_wrapped(|ui| {
                        if let Some([collapse, expand]) = self.collapse_labels {
                            let label = if state.collapsed { expand } else { collapse };
                            let response = crate::components::action(ui, tokens, label);
                            if response.clicked() {
                                state.collapsed = !state.collapsed;
                            }
                            ui.ctx().accesskit_node_builder(response.id, |node| {
                                node.set_expanded(!state.collapsed)
                            });
                        }
                        if let Some(label) = self.dismiss_label
                            && crate::components::action(ui, tokens, label).clicked()
                        {
                            state.dismissed = true;
                        }
                    });
                }
            });
        let rect = frame.response.rect;
        if ui.is_rect_visible(rect) {
            let mut edge = rect;
            edge.max.x = edge.min.x + f32::from(theme.radius).max(3.0).min(rect.width() * 0.5);
            ui.painter_at(edge).rect_stroke(
                rect,
                theme.radius,
                egui::Stroke::new(3.0, accent),
                egui::StrokeKind::Inside,
            );
        }
        Some(frame.response)
    }
}

#[cfg(test)]
#[test]
fn accent_uses_the_full_card_outline_in_both_densities() {
    let ctx = egui::Context::default();
    for density in [crate::Density::Compact, crate::Density::Comfy] {
        UiTokens::install(&ctx, crate::ThemeMode::Custom(Theme::BUBBLEGUM), density);
        let mut bounds = egui::Rect::NOTHING;
        let output = ctx.run_ui(Default::default(), |ui| {
            bounds = Note {
                tone: NoteTone::Warning,
                title: "Warning",
                body: "Check input gain.",
                collapse_labels: None,
                dismiss_label: None,
            }
            .show(ui, UiTokens::resolve(&ctx), &mut NoteState::default())
            .unwrap()
            .rect;
        });
        let matches = output.shapes.iter().any(|s| matches!(&s.shape,
            egui::Shape::Rect(r) if r.stroke.width == 3.0 && r.rect == bounds &&
            r.corner_radius.nw == Theme::BUBBLEGUM.radius && s.clip_rect.top() <= bounds.top() && s.clip_rect.bottom() >= bounds.bottom()));
        output.drop_without_applying_deltas();
        assert!(matches);
    }
}
