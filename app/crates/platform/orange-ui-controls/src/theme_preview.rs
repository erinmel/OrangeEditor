use crate::{Action, Surface, Theme, role_colors};

/// Paints the same miniature interface in each theme. The optional second
/// theme occupies the lower-right side of a diagonal split.
pub fn paint_theme_preview(
    painter: &egui::Painter,
    rect: egui::Rect,
    theme: Theme,
    other: Option<Theme>,
) {
    use egui::{Rect, Stroke, pos2};
    if rect.width() < 26.0 || rect.height() <= 0.0 || !rect.is_finite() {
        return;
    }
    let draw = |theme: Theme, half: Option<f32>| {
        let fill = |bounds: Rect, color| {
            let Some(sign) = half else {
                painter.rect_filled(bounds, 0, color);
                return;
            };
            let distance = |p: egui::Pos2| {
                sign * ((p.x - rect.left()) / rect.width() + (p.y - rect.top()) / rect.height()
                    - 1.0)
            };
            let corners = [
                bounds.left_top(),
                bounds.right_top(),
                bounds.right_bottom(),
                bounds.left_bottom(),
            ];
            let mut points = Vec::with_capacity(5);
            for (a, b) in corners.into_iter().zip(corners.into_iter().cycle().skip(1)) {
                let (da, db) = (distance(a), distance(b));
                if da >= 0.0 {
                    points.push(a);
                }
                if (da < 0.0 && db > 0.0) || (da > 0.0 && db < 0.0) {
                    points.push(a.lerp(b, da / (da - db)));
                }
            }
            if points.len() >= 3 {
                painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
            }
        };
        let at = |x, y| {
            pos2(
                rect.left() + x * rect.width(),
                rect.top() + y * rect.height() / 53.0,
            )
        };
        let box_at = |l, t, r, b| Rect::from_min_max(at(l, t), at(r, b));
        let (primary, _, on_primary) = role_colors(theme, Surface::Panel, Action::Primary);
        let (secondary, border, on_secondary) =
            role_colors(theme, Surface::Panel, Action::Secondary);
        fill(rect, theme.panel);
        fill(box_at(0.05, 4.0, 0.24, 49.0), theme.dialog);
        fill(box_at(0.08, 11.0, 0.19, 13.0), primary);
        fill(box_at(0.28, 4.0, 0.95, 49.0), theme.dialog);
        fill(box_at(0.34, 10.0, 0.64, 12.0), theme.text);
        fill(box_at(0.33, 15.0, 0.90, 27.0), theme.inset);
        fill(
            box_at(0.34, 19.0, 0.89, 22.0),
            theme.slider_idle(Surface::Card),
        );
        fill(box_at(0.34, 19.0, 0.63, 22.0), primary);
        fill(box_at(0.60, 16.5, 0.66, 24.5), theme.text);
        fill(box_at(0.34, 31.0, 0.59, 43.0), primary);
        fill(box_at(0.39, 36.0, 0.54, 38.0), on_primary);
        fill(box_at(0.63, 31.0, 0.88, 43.0), border);
        fill(box_at(0.64, 32.0, 0.87, 42.0), secondary);
        fill(box_at(0.68, 36.0, 0.83, 38.0), on_secondary);
    };
    if let Some(other) = other {
        draw(theme, Some(-1.0));
        draw(other, Some(1.0));
    } else {
        draw(theme, None);
    }
}
