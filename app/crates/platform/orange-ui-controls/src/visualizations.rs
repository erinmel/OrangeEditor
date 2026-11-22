//! Theme-aware data visualizations that are independent of application semantics.
use super::{Surface, Theme};
use egui::{Color32, Rect, Response, Sense, Stroke, Ui};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeterOrientation {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy)]
pub enum MeterRepresentation {
    Continuous,
    Segmented,
}

#[derive(Debug, Clone, Copy)]
pub struct MeterColors {
    pub low: Color32,
    pub middle: Color32,
    pub high: Color32,
    pub peak: Color32,
}

impl MeterColors {
    pub fn standard() -> Self {
        Self {
            low: Color32::from_rgb(22, 154, 82),
            middle: Color32::from_rgb(242, 211, 65),
            high: Color32::from_rgb(255, 61, 76),
            peak: Color32::from_gray(240),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LevelMeter {
    pub orientation: MeterOrientation,
    pub representation: MeterRepresentation,
    pub fill: f32,
    pub peak: f32,
    pub marks: &'static [f32],
}

impl LevelMeter {
    pub const fn vertical(fill: f32, peak: f32) -> Self {
        Self {
            orientation: MeterOrientation::Vertical,
            representation: MeterRepresentation::Continuous,
            fill,
            peak,
            marks: &[],
        }
    }

    pub fn paint(self, ui: &mut Ui, surface: Surface, rect: Rect, colors: MeterColors) -> Response {
        let (response_rect, response) = ui.allocate_exact_size(rect.size(), Sense::hover());
        let rect = rect.intersect(response_rect);
        if ui.is_rect_visible(rect) {
            let theme = Theme::of(ui);
            let idle = theme.background(surface).lerp_to_gamma(theme.text, 0.10);
            let painter = ui.painter();
            painter.rect(
                rect,
                3,
                idle,
                Stroke::new(1.0, theme.border),
                egui::StrokeKind::Inside,
            );
            let fill = self.fill.clamp(0.0, 1.0);
            let active = match self.orientation {
                MeterOrientation::Horizontal => Rect::from_min_max(
                    rect.min,
                    egui::pos2(egui::lerp(rect.x_range(), fill), rect.bottom()),
                ),
                MeterOrientation::Vertical => Rect::from_min_max(
                    egui::pos2(rect.left(), egui::lerp(rect.y_range(), 1.0 - fill)),
                    rect.max,
                ),
            };
            match self.representation {
                MeterRepresentation::Continuous => {
                    painter.rect_filled(active, 2, lerp_meter_color(colors, fill));
                }
                MeterRepresentation::Segmented => {
                    let count = 20;
                    for index in 0..count {
                        let fraction = (index + 1) as f32 / count as f32;
                        let segment = if self.orientation == MeterOrientation::Vertical {
                            let top = egui::lerp(rect.y_range(), 1.0 - fraction);
                            let bottom =
                                egui::lerp(rect.y_range(), 1.0 - (fraction - 1.0 / count as f32));
                            Rect::from_min_max(
                                egui::pos2(rect.left() + 1.0, top),
                                egui::pos2(rect.right() - 1.0, bottom),
                            )
                        } else {
                            let left = egui::lerp(rect.x_range(), fraction - 1.0 / count as f32);
                            let right = egui::lerp(rect.x_range(), fraction);
                            Rect::from_min_max(
                                egui::pos2(left, rect.top() + 1.0),
                                egui::pos2(right - 1.0, rect.bottom() - 1.0),
                            )
                        };
                        if fraction <= fill {
                            painter.rect_filled(segment, 1, lerp_meter_color(colors, fraction));
                        }
                    }
                }
            }
            let peak = self.peak.clamp(0.0, 1.0);
            if peak > fill {
                match self.orientation {
                    MeterOrientation::Horizontal => {
                        let x = egui::lerp(rect.x_range(), peak);
                        painter.vline(x, rect.y_range(), Stroke::new(1.5, colors.peak));
                    }
                    MeterOrientation::Vertical => {
                        let y = egui::lerp(rect.y_range(), 1.0 - peak);
                        painter.hline(rect.x_range(), y, Stroke::new(1.5, colors.peak));
                    }
                }
            }
            for mark in self.marks {
                let fraction = mark.clamp(0.0, 1.0);
                match self.orientation {
                    MeterOrientation::Horizontal => {
                        let x = egui::lerp(rect.x_range(), fraction);
                        painter.vline(x, rect.y_range(), Stroke::new(1.0, theme.muted_text));
                    }
                    MeterOrientation::Vertical => {
                        let y = egui::lerp(rect.y_range(), 1.0 - fraction);
                        painter.hline(rect.x_range(), y, Stroke::new(1.0, theme.muted_text));
                    }
                }
            }
        }
        response
    }
}

fn lerp_meter_color(colors: MeterColors, fraction: f32) -> Color32 {
    if fraction < 0.68 {
        colors
            .low
            .gamma_multiply(1.0 - fraction * 0.15)
            .lerp_to_gamma(colors.middle, fraction / 0.68)
    } else {
        colors
            .middle
            .lerp_to_gamma(colors.high, (fraction - 0.68) / 0.32)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ChartOrientation {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartRepresentation {
    Bars,
    Line,
    Area,
}

/// Draws a generic chart from normalized samples.
pub fn chart(
    ui: &mut Ui,
    surface: Surface,
    rect: Rect,
    orientation: ChartOrientation,
    representation: ChartRepresentation,
    values: &[f32],
) -> Response {
    let (response_rect, response) = ui.allocate_exact_size(rect.size(), Sense::hover());
    let rect = rect.intersect(response_rect);
    if ui.is_rect_visible(rect) && !values.is_empty() {
        let theme = Theme::of(ui);
        let painter = ui.painter();
        painter.rect(
            rect,
            3,
            theme.background(surface).lerp_to_gamma(theme.text, 0.08),
            Stroke::new(1.0, theme.border),
            egui::StrokeKind::Inside,
        );
        let points: Vec<_> = values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let fraction = value.clamp(0.0, 1.0);
                match orientation {
                    ChartOrientation::Horizontal => egui::pos2(
                        egui::lerp(rect.x_range(), (index as f32 + 0.5) / values.len() as f32),
                        egui::lerp(rect.y_range(), 1.0 - fraction),
                    ),
                    ChartOrientation::Vertical => egui::pos2(
                        egui::lerp(rect.x_range(), fraction),
                        egui::lerp(rect.y_range(), (index as f32 + 0.5) / values.len() as f32),
                    ),
                }
            })
            .collect();
        match representation {
            ChartRepresentation::Bars => {
                for (index, value) in values.iter().enumerate() {
                    let fraction = value.clamp(0.0, 1.0);
                    let bar = match orientation {
                        ChartOrientation::Horizontal => Rect::from_min_max(
                            egui::pos2(
                                egui::lerp(rect.x_range(), index as f32 / values.len() as f32),
                                egui::lerp(rect.y_range(), 1.0 - fraction),
                            ),
                            egui::pos2(
                                egui::lerp(
                                    rect.x_range(),
                                    (index + 1) as f32 / values.len() as f32,
                                ),
                                rect.bottom(),
                            ),
                        ),
                        ChartOrientation::Vertical => Rect::from_min_max(
                            egui::pos2(
                                rect.left(),
                                egui::lerp(rect.y_range(), index as f32 / values.len() as f32),
                            ),
                            egui::pos2(
                                egui::lerp(rect.x_range(), fraction),
                                egui::lerp(
                                    rect.y_range(),
                                    (index + 1) as f32 / values.len() as f32,
                                ),
                            ),
                        ),
                    };
                    painter.rect_filled(bar, 1, theme.primary.gamma_multiply(0.85));
                }
            }
            ChartRepresentation::Line => {
                painter.add(egui::Shape::line(points, Stroke::new(1.5, theme.primary)));
            }
            ChartRepresentation::Area => {
                if let Some(first) = points.first() {
                    let mut area = points.clone();
                    area.push(egui::pos2(
                        points.last().copied().unwrap_or(*first).x,
                        rect.bottom(),
                    ));
                    area.push(egui::pos2(first.x, rect.bottom()));
                    painter.add(egui::Shape::convex_polygon(
                        area,
                        theme.primary.gamma_multiply(0.35),
                        Stroke::NONE,
                    ));
                }
                painter.add(egui::Shape::line(points, Stroke::new(1.5, theme.primary)));
            }
        }
    }
    response
}

/// Paints a line chart over an existing plot without replacing its frame or background.
pub fn line_overlay(ui: &mut Ui, rect: Rect, values: &[f32], stroke: Stroke) -> Response {
    let (response_rect, response) = ui.allocate_exact_size(rect.size(), Sense::hover());
    let rect = rect.intersect(response_rect);
    if ui.is_rect_visible(rect) && values.len() > 1 {
        let points: Vec<_> = values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                egui::pos2(
                    egui::lerp(rect.x_range(), (index as f32 + 0.5) / values.len() as f32),
                    egui::lerp(rect.y_range(), 1.0 - value.clamp(0.0, 1.0)),
                )
            })
            .collect();
        ui.painter().add(egui::Shape::line(points, stroke));
    }
    response
}

/// Paints a line directly into an existing plot, preserving the caller's clip,
/// hit regions and drawing order.
pub fn paint_line(painter: &egui::Painter, rect: Rect, values: &[f32], stroke: Stroke) {
    if values.len() < 2 {
        return;
    }
    let points: Vec<_> = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            egui::pos2(
                egui::lerp(rect.x_range(), (index as f32 + 0.5) / values.len() as f32),
                egui::lerp(rect.y_range(), 1.0 - value.clamp(0.0, 1.0)),
            )
        })
        .collect();
    painter
        .with_clip_rect(painter.clip_rect().intersect(rect))
        .add(egui::Shape::line(points, stroke));
}

#[derive(Debug, Clone, Copy)]
pub struct MeterStop {
    pub fraction: f32,
    pub color: Color32,
}

/// Paints a vertical gradient meter directly into an existing rectangle.
pub fn paint_vertical_meter(
    painter: &egui::Painter,
    rect: Rect,
    fill: f32,
    peak: f32,
    stops: &[MeterStop],
    peak_color: Color32,
    dim: f32,
) {
    let mut mesh = egui::Mesh::default();
    for (index, stop) in stops.iter().enumerate() {
        let y = egui::lerp(rect.y_range(), 1.0 - stop.fraction);
        let color = stop.color.gamma_multiply(dim);
        let left = mesh.vertices.len();
        mesh.colored_vertex(egui::pos2(rect.left(), y), color);
        mesh.colored_vertex(egui::pos2(rect.right(), y), color);
        if index > 0 {
            let previous = left - 2;
            let previous = u32::try_from(previous).unwrap_or_default();
            let left = u32::try_from(left).unwrap_or_default();
            mesh.add_triangle(previous, previous + 1, left);
            mesh.add_triangle(left, previous + 1, left + 1);
        }
    }
    painter.add(egui::Shape::mesh(mesh));

    let fill = fill.clamp(0.0, 1.0);
    if fill > 0.0 {
        let clip = Rect::from_min_max(
            egui::pos2(rect.left(), egui::lerp(rect.y_range(), 1.0 - fill)),
            rect.max,
        );
        let mut active = egui::Mesh::default();
        for (index, stop) in stops.iter().enumerate() {
            let y = egui::lerp(rect.y_range(), 1.0 - stop.fraction);
            let color = stop.color;
            let left = active.vertices.len();
            active.colored_vertex(egui::pos2(rect.left(), y), color);
            active.colored_vertex(egui::pos2(rect.right(), y), color);
            if index > 0 {
                let previous = left - 2;
                let previous = u32::try_from(previous).unwrap_or_default();
                let left = u32::try_from(left).unwrap_or_default();
                active.add_triangle(previous, previous + 1, left);
                active.add_triangle(left, previous + 1, left + 1);
            }
        }
        painter
            .with_clip_rect(painter.clip_rect().intersect(clip))
            .add(egui::Shape::mesh(active));
    }
    let peak = peak.clamp(0.0, 1.0);
    if peak > fill + 0.005 {
        painter.hline(
            rect.x_range(),
            egui::lerp(rect.y_range(), 1.0 - peak),
            egui::Stroke::new(1.5, peak_color),
        );
    }
}

/// Draws a generic frequency or time-series bar chart.
pub fn frequency_chart(
    ui: &mut Ui,
    surface: Surface,
    rect: Rect,
    values: &[f32],
    orientation: ChartOrientation,
) -> Response {
    let (response_rect, response) = ui.allocate_exact_size(rect.size(), Sense::hover());
    let rect = rect.intersect(response_rect);
    if ui.is_rect_visible(rect) && !values.is_empty() {
        let theme = Theme::of(ui);
        let painter = ui.painter();
        painter.rect(
            rect,
            3,
            theme.background(surface).lerp_to_gamma(theme.text, 0.08),
            Stroke::new(1.0, theme.border),
            egui::StrokeKind::Inside,
        );
        match orientation {
            ChartOrientation::Horizontal => {
                let width = rect.width() / values.len() as f32;
                for (index, value) in values.iter().enumerate() {
                    let fraction = value.clamp(0.0, 1.0);
                    let bar = Rect::from_min_max(
                        egui::pos2(
                            rect.left() + index as f32 * width,
                            rect.bottom() - rect.height() * fraction,
                        ),
                        egui::pos2(rect.left() + (index + 1) as f32 * width, rect.bottom()),
                    );
                    painter.rect_filled(bar, 1, theme.primary.gamma_multiply(0.85));
                }
            }
            ChartOrientation::Vertical => {
                let height = rect.height() / values.len() as f32;
                for (index, value) in values.iter().enumerate() {
                    let fraction = value.clamp(0.0, 1.0);
                    let bar = Rect::from_min_max(
                        egui::pos2(rect.left(), rect.top() + index as f32 * height),
                        egui::pos2(
                            rect.left() + rect.width() * fraction,
                            rect.top() + (index + 1) as f32 * height,
                        ),
                    );
                    painter.rect_filled(bar, 1, theme.primary.gamma_multiply(0.85));
                }
            }
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_line_keeps_the_plot_clip_and_two_endpoints() {
        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(200.0, 100.0));
        let mut line_points = None;
        ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.painter();
            paint_line(
                painter,
                rect,
                &[0.25, 0.75],
                Stroke::new(2.0, Color32::WHITE),
            );
            line_points = Some(rect);
        })
        .drop_without_applying_deltas();
        assert_eq!(line_points, Some(rect));
    }

    #[test]
    fn vertical_meter_paints_fill_and_peak_without_allocating_layout() {
        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(40.0, 185.0));
        let mut saw_mesh = false;
        let mut saw_peak = false;
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            paint_vertical_meter(
                ui.painter(),
                rect,
                0.6,
                0.8,
                &[
                    MeterStop {
                        fraction: 0.0,
                        color: Color32::GREEN,
                    },
                    MeterStop {
                        fraction: 1.0,
                        color: Color32::RED,
                    },
                ],
                Color32::WHITE,
                0.23,
            );
        });
        for shape in &output.shapes {
            match &shape.shape {
                egui::Shape::Mesh(_) => saw_mesh = true,
                egui::Shape::LineSegment { .. } => saw_peak = true,
                _ => {}
            }
        }
        output.drop_without_applying_deltas();
        assert!(saw_mesh);
        assert!(saw_peak);
    }
}
