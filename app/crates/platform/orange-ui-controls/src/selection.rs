use super::{CONTROL_RADIUS, ControlSize, Dropdown, SPACE_SMALL, Surface, Theme, motion};
use egui::{Button, Key, Rect, Response, Slider, Stroke, StrokeKind, Ui, pos2, vec2};
use std::num::NonZeroU16;
use std::ops::RangeInclusive;

/// Draws a single-choice group. Narrow panels stack the segments inside the
/// same container. Arrows move selection within the focused group.
pub fn segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    surface: Surface,
    label: &Response,
    value: &mut T,
    options: &[(T, &str)],
) -> Response {
    let theme = Theme::of(ui);
    let spacing = 2.0;
    let width = ui.available_width();
    let height = ui
        .spacing()
        .interact_size
        .y
        .max(ControlSize::Compact.height());
    let font = egui::TextStyle::Button.resolve(ui.style());
    let natural = options.iter().fold(0.0_f32, |width, (_, text)| {
        width.max(
            ui.painter()
                .layout_no_wrap((*text).to_owned(), font.clone(), theme.text)
                .size()
                .x
                + 20.0,
        )
    });
    let columns = options.len().max(1);
    #[expect(
        clippy::cast_precision_loss,
        reason = "the group contains a small number of options"
    )]
    let columns = columns as f32;
    let body_width = (width - 2.0 * (spacing + 1.0)).max(0.0);
    let segment_width = ((body_width + spacing) / columns - spacing).max(0.0);
    let single_row = segment_width >= natural;
    let mut next = None;
    let mut responses = Vec::with_capacity(options.len());
    let mut highlight = None;
    let mut group = ui
        .push_id(label.id, |ui| {
            egui::Frame::new()
                .fill(theme.background(surface).lerp_to_gamma(theme.inset, 0.5))
                .stroke(Stroke::new(1.0, theme.border))
                .corner_radius(CONTROL_RADIUS)
                .inner_margin(2)
                .show(ui, |ui| {
                    highlight = Some(ui.painter().add(egui::Shape::Noop));
                    let mut show = |ui: &mut Ui| {
                        ui.spacing_mut().item_spacing = vec2(spacing, spacing);
                        for (index, (_, text)) in options.iter().enumerate() {
                            let response = ui
                                .add(
                                    Button::new(egui::RichText::new(*text).color(theme.text))
                                        .frame(false)
                                        .truncate()
                                        .min_size(vec2(
                                            if single_row {
                                                segment_width
                                            } else {
                                                body_width
                                            },
                                            height,
                                        )),
                                )
                                .labelled_by(label.id);
                            if response.clicked() {
                                next = Some(index);
                            }
                            if response.has_focus() && options.len() > 1 {
                                let previous = ui.input_mut(|input| {
                                    input.consume_key(egui::Modifiers::NONE, Key::ArrowLeft)
                                        || input.consume_key(egui::Modifiers::NONE, Key::ArrowUp)
                                });
                                let forward = ui.input_mut(|input| {
                                    input.consume_key(egui::Modifiers::NONE, Key::ArrowRight)
                                        || input.consume_key(egui::Modifiers::NONE, Key::ArrowDown)
                                });
                                if previous || forward {
                                    let offset = if previous { options.len() - 1 } else { 1 };
                                    next = Some((index + offset) % options.len());
                                }
                            }
                            responses.push(response);
                        }
                    };
                    if single_row {
                        ui.horizontal(&mut show);
                    } else {
                        ui.vertical(show);
                    }
                })
                .response
        })
        .inner;
    if let Some(index) = next {
        if *value != options[index].0 {
            *value = options[index].0;
            group.mark_changed();
        }
        responses[index].request_focus();
    }
    if let Some(shape) = highlight
        && let Some(index) = options.iter().position(|(option, _)| *value == *option)
    {
        paint_selection(ui, surface, label, shape, &responses, index);
    }
    for ((option, text), response) in options.iter().zip(responses) {
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::Role::RadioButton,
                response.enabled(),
                *value == *option,
                *text,
            )
        });
        let (hover, press) = motion(ui, &response);
        ui.painter().rect(
            response.rect,
            CONTROL_RADIUS - 2,
            theme.text.gamma_multiply(hover * 0.07 + press * 0.09),
            Stroke::NONE,
            StrokeKind::Inside,
        );
        if response.has_focus() {
            let center = response.rect.center_bottom() - vec2(0.0, 3.0);
            ui.painter().hline(
                center.x - 5.0..=center.x + 5.0,
                center.y,
                Stroke::new(1.5, theme.focus),
            );
        }
    }
    group
}

/// Draws a notebook tab strip. It is the shared tab primitive used by docks and
/// settings sections; narrow widths stack choices inside the same frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotebookOrientation {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotebookPadding {
    Compact,
    Comfortable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotebookWidth {
    Fixed,
    Flex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliderMarks {
    Hidden,
    Notches,
    Values,
    NotchesAndValues,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SliderBehavior {
    Free,
    Step(f32),
}

#[derive(Debug, Clone, Copy)]
pub struct SliderGuide {
    pub fraction: f32,
    pub label: &'static str,
}

pub fn notebook<T: Copy + PartialEq>(
    ui: &mut Ui,
    surface: Surface,
    orientation: NotebookOrientation,
    value: &mut T,
    options: &[(T, &str)],
) -> Response {
    notebook_styled(
        ui,
        surface,
        orientation,
        NotebookPadding::Compact,
        value,
        options,
    )
}

pub fn notebook_styled<T: Copy + PartialEq>(
    ui: &mut Ui,
    surface: Surface,
    orientation: NotebookOrientation,
    padding: NotebookPadding,
    value: &mut T,
    options: &[(T, &str)],
) -> Response {
    notebook_sized(
        ui,
        surface,
        orientation,
        padding,
        NotebookWidth::Fixed,
        value,
        options,
    )
}

pub fn notebook_flex<T: Copy + PartialEq>(
    ui: &mut Ui,
    surface: Surface,
    orientation: NotebookOrientation,
    padding: NotebookPadding,
    value: &mut T,
    options: &[(T, &str)],
) -> Response {
    notebook_sized(
        ui,
        surface,
        orientation,
        padding,
        NotebookWidth::Flex,
        value,
        options,
    )
}

pub fn notebook_sized<T: Copy + PartialEq>(
    ui: &mut Ui,
    surface: Surface,
    orientation: NotebookOrientation,
    padding: NotebookPadding,
    width_mode: NotebookWidth,
    value: &mut T,
    options: &[(T, &str)],
) -> Response {
    let padded_width = ui.available_width();
    ui.scope(|ui| {
        ui.set_width(padded_width);
        match orientation {
            NotebookOrientation::Horizontal => {
                let height = ControlSize::Standard.height();
                let theme = Theme::of(ui);
                let font = egui::TextStyle::Button.resolve(ui.style());
                let widths: Vec<_> = options
                    .iter()
                    .map(|(_, text)| {
                        ui.painter()
                            .layout_no_wrap((*text).to_owned(), font.clone(), theme.text)
                            .size()
                            .x
                            + 16.0
                    })
                    .collect();
                let spacing = 2.0;
                let menu_width = 32.0;
                let frame_width = ui.available_width();
                let inner_width = (frame_width - 4.0).max(1.0);
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "the notebook contains a small number of options"
                )]
                let gaps = spacing * options.len().saturating_sub(1) as f32;
                let natural_width = widths.iter().sum::<f32>() + gaps;
                let mut visible_indices = Vec::new();
                let mut overflow_indices = Vec::new();
                if natural_width <= inner_width {
                    visible_indices.extend(0..options.len());
                } else {
                    let selected = options.iter().position(|(option, _)| *value == *option);
                    let mut used = 0.0;
                    for (index, width) in widths.iter().enumerate() {
                        if used + *width + spacing + menu_width <= inner_width {
                            visible_indices.push(index);
                            used += *width + spacing;
                        } else {
                            break;
                        }
                    }
                    if let Some(selected) = selected
                        && !visible_indices.contains(&selected)
                    {
                        while let Some(last) = visible_indices.last().copied() {
                            let candidate =
                                used - widths[last] - spacing + widths[selected] + spacing;
                            if candidate + menu_width <= inner_width {
                                break;
                            }
                            used -= widths[last] + spacing;
                            visible_indices.pop();
                        }
                        if used + widths[selected] + spacing + menu_width <= inner_width {
                            visible_indices.push(selected);
                        }
                    }
                    visible_indices.sort_unstable();
                    visible_indices.dedup();
                    for index in 0..options.len() {
                        if !visible_indices.contains(&index) {
                            overflow_indices.push(index);
                        }
                    }
                }

                let mut next = None;
                let mut responses = Vec::new();
                let mut overflow_response = None;
                egui::Frame::new()
                    .fill(theme.background(surface).lerp_to_gamma(theme.inset, 0.35))
                    .stroke(Stroke::new(1.0, theme.border))
                    .corner_radius(CONTROL_RADIUS)
                    .inner_margin(if padding == NotebookPadding::Comfortable {
                        4
                    } else {
                        2
                    })
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = spacing;
                        ui.horizontal(|ui| {
                            for index in &visible_indices {
                                let (option, text) = options[*index];
                                let selected = *value == option;
                                let tab_width = if width_mode == NotebookWidth::Flex {
                                    let count = visible_indices.len();
                                    if count == 0 {
                                        widths[*index]
                                    } else {
                                        #[expect(
                                            clippy::cast_precision_loss,
                                            reason = "the notebook has a small number of tabs"
                                        )]
                                        let count_float = count as f32;
                                        let free = (ui.available_width()
                                            - if overflow_indices.is_empty() {
                                                0.0
                                            } else {
                                                menu_width
                                            }
                                            - spacing * (count.saturating_sub(1) as f32))
                                            / count_float;
                                        free.max(widths[*index])
                                    }
                                } else {
                                    widths[*index]
                                };
                                let response = super::button(
                                    ui,
                                    surface,
                                    if selected {
                                        super::Action::Toggle(true)
                                    } else {
                                        super::Action::Toggle(false)
                                    },
                                    true,
                                    Button::new(text)
                                        .truncate()
                                        .min_size(vec2(tab_width, height)),
                                );
                                if response.clicked() {
                                    next = Some(*index);
                                }
                                responses.push(response);
                            }
                            if !overflow_indices.is_empty() {
                                let menu = egui::ComboBox::from_id_salt((
                                    "notebook-overflow",
                                    options.len(),
                                ))
                                .width(menu_width)
                                .selected_text("•••");
                                let shown = menu.show_select(ui, surface, |ui| {
                                    ui.set_min_width(menu_width.max(ui.available_width()));
                                    for index in &overflow_indices {
                                        let (option, text) = options[*index];
                                        let is_selected = *value == option;
                                        if ui.selectable_label(is_selected, text).clicked()
                                            && !is_selected
                                        {
                                            next = Some(*index);
                                            ui.close();
                                        }
                                    }
                                });
                                overflow_response = Some(shown.response);
                            }
                        });
                    });
                if let Some(index) = next
                    && *value != options[index].0
                {
                    *value = options[index].0;
                    let response = responses.last().cloned().or(overflow_response.clone());
                    if let Some(mut response) = response {
                        response.mark_changed();
                        return response;
                    }
                }
                responses.pop().or(overflow_response).unwrap_or_else(|| {
                    ui.allocate_response(vec2(frame_width, height), egui::Sense::hover())
                })
            }
            NotebookOrientation::Vertical => {
                let theme = Theme::of(ui);
                let width = ui.available_width();
                let mut response = None;
                let mut next = None;
                ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
                egui::Frame::new()
                    .fill(theme.background(surface).lerp_to_gamma(theme.inset, 0.35))
                    .stroke(Stroke::new(1.0, theme.border))
                    .corner_radius(CONTROL_RADIUS)
                    .inner_margin(2)
                    .show(ui, |ui| {
                        ui.set_min_width(width - 4.0);
                        for (index, (option, text)) in options.iter().enumerate() {
                            let selected = *value == *option;
                            let item = super::button(
                                ui,
                                surface,
                                if selected {
                                    super::Action::Toggle(true)
                                } else {
                                    super::Action::Toggle(false)
                                },
                                true,
                                Button::new(*text).truncate().min_size(vec2(
                                    ui.available_width(),
                                    ControlSize::Standard.height(),
                                )),
                            );
                            if item.clicked() {
                                next = Some(index);
                            }
                            response = Some(item);
                        }
                    });
                if let Some(index) = next
                    && *value != options[index].0
                {
                    *value = options[index].0;
                    if let Some(response) = response {
                        let mut response = response;
                        response.mark_changed();
                        return response;
                    }
                }
                response
                    .unwrap_or_else(|| ui.allocate_response(vec2(width, 0.0), egui::Sense::hover()))
            }
        }
    })
    .inner
}

fn paint_selection(
    ui: &Ui,
    surface: Surface,
    label: &Response,
    shape: egui::layers::ShapeIdx,
    segments: &[Response],
    index: usize,
) {
    #[expect(
        clippy::cast_precision_loss,
        reason = "the group contains a small number of options"
    )]
    let target = index as f32;
    let position = ui.ctx().animate_value_with_time(
        label.id.with("segment-selection"),
        target,
        ui.style().animation_time,
    );
    #[expect(
        clippy::cast_possible_truncation,
        reason = "positions interpolate between option indices"
    )]
    let from = (position.floor() as usize).min(segments.len() - 1);
    let to = (from + 1).min(segments.len() - 1);
    let rect = Rect::from_min_max(
        segments[from]
            .rect
            .min
            .lerp(segments[to].rect.min, position.fract()),
        segments[from]
            .rect
            .max
            .lerp(segments[to].rect.max, position.fract()),
    );
    let theme = Theme::of(ui);
    let fill = theme.selected_fill(surface);
    let fade = if ui.is_enabled() { 1.0 } else { 0.45 };
    ui.painter().set(
        shape,
        egui::epaint::RectShape::new(
            rect,
            CONTROL_RADIUS - 2,
            fill.gamma_multiply(fade),
            Stroke::new(1.0, theme.focus.gamma_multiply(fade)),
            StrokeKind::Inside,
        ),
    );
}

/// Draws a continuously adjustable slider with the shared rail and handle.
pub fn continuous_slider(
    ui: &mut Ui,
    surface: Surface,
    value: &mut f32,
    range: RangeInclusive<f32>,
) -> Response {
    let start = f64::from(*range.start());
    let end = f64::from(*range.end());
    let response = slider_input(ui, Slider::new(value, range), 22.0);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "normalized screen coordinate"
    )]
    let fraction = if start == end {
        0.0
    } else {
        ((f64::from(*value) - start) / (end - start)).clamp(0.0, 1.0) as f32
    };
    SliderTrack::new(&response, fraction).paint(ui, surface, &response, |_, _, _| {});
    response
}

/// Continuously adjusts a value with optional visual guide marks.
///
/// Marks are normalized positions on the rail and never snap or constrain the
/// value. This is for controls whose values remain continuous while reference
/// positions are useful to the person using the control.
pub fn continuous_marked_slider(
    ui: &mut Ui,
    surface: Surface,
    value: &mut f32,
    range: RangeInclusive<f32>,
    marks: &[f32],
) -> Response {
    let start = f64::from(*range.start());
    let end = f64::from(*range.end());
    let response = slider_input(ui, Slider::new(value, range), 22.0);
    let fraction = if start == end {
        0.0
    } else {
        ((f64::from(*value) - start) / (end - start)).clamp(0.0, 1.0) as f32
    };
    let track = SliderTrack::new(&response, fraction);
    track.paint(ui, surface, &response, |painter, active, idle| {
        for mark in marks {
            let x = egui::lerp(track.travel, mark.clamp(0.0, 1.0));
            painter.rect_filled(
                Rect::from_center_size(pos2(x, track.thumb.y), vec2(10.5, 7.0)),
                CONTROL_RADIUS - 2,
                if x <= track.thumb.x { active } else { idle },
            );
        }
    });
    response
}

/// A single slider presentation for continuous or strict values.
///
/// Guide labels are visual references: only `SliderBehavior::Step` changes
/// pointer and keyboard behavior to snap to a step.
pub fn guided_slider(
    ui: &mut Ui,
    surface: Surface,
    value: &mut f32,
    range: RangeInclusive<f32>,
    behavior: SliderBehavior,
    marks: SliderMarks,
    guides: &[SliderGuide],
) -> Response {
    let slider = match behavior {
        SliderBehavior::Free => Slider::new(value, range.clone()),
        SliderBehavior::Step(step) if step > 0.0 => Slider::new(value, range.clone())
            .step_by(f64::from(step))
            .smart_aim(false),
        SliderBehavior::Step(_) => Slider::new(value, range.clone()).smart_aim(false),
    };
    let response = slider_input(ui, slider, 22.0);
    let start = f64::from(*range.start());
    let end = f64::from(*range.end());
    let fraction = if start == end {
        0.0
    } else {
        ((f64::from(*value) - start) / (end - start)).clamp(0.0, 1.0) as f32
    };
    let track = SliderTrack::new(&response, fraction);
    track.paint(ui, surface, &response, |painter, active, idle| {
        if matches!(marks, SliderMarks::Notches | SliderMarks::NotchesAndValues) {
            for guide in guides {
                let x = egui::lerp(track.travel, guide.fraction.clamp(0.0, 1.0));
                painter.rect_filled(
                    Rect::from_center_size(pos2(x, track.thumb.y), vec2(10.5, 7.0)),
                    CONTROL_RADIUS - 2,
                    if x <= track.thumb.x { active } else { idle },
                );
            }
        }
    });
    if matches!(marks, SliderMarks::Values | SliderMarks::NotchesAndValues) && !guides.is_empty() {
        let mut font = egui::TextStyle::Monospace.resolve(ui.style());
        font.size *= 0.85;
        let theme = Theme::of(ui);
        let (_, row) = ui.allocate_space(vec2(response.rect.width(), font.size));
        let mut right = row.left() - SPACE_SMALL;
        for guide in guides {
            let galley =
                ui.painter()
                    .layout_no_wrap(guide.label.to_owned(), font.clone(), theme.muted_text);
            let width = galley.size().x;
            let x = (egui::lerp(track.travel, guide.fraction.clamp(0.0, 1.0)) - width * 0.5)
                .clamp(row.left(), (row.right() - width).max(row.left()));
            if x >= right + SPACE_SMALL && x + width <= row.right() {
                ui.painter()
                    .galley(pos2(x, row.top()), galley, theme.muted_text);
                right = x + width;
            }
        }
    }
    response
}

/// Adjusts a normalized value continuously, with visual marks that do not snap it.
pub fn marked_slider(ui: &mut Ui, surface: Surface, fraction: &mut f32, marks: &[f32]) -> Response {
    let height = ui.spacing().interact_size.y;
    let response = slider_input(
        ui,
        Slider::new(fraction, 0.0..=1.0).smart_aim(false),
        height,
    );
    let track = SliderTrack::new(&response, *fraction);
    track.paint(ui, surface, &response, |painter, active, idle| {
        for mark in marks {
            let x = egui::lerp(track.travel, *mark);
            painter.rect_filled(
                Rect::from_center_size(pos2(x, track.thumb.y), vec2(10.5, 7.0)),
                CONTROL_RADIUS - 2,
                if x <= track.thumb.x { active } else { idle },
            );
        }
    });
    response
}

/// Draws a circular value control with optional snapping and visual marks.
///
/// The marks are normalized positions from 0.0 to 1.0 and do not themselves
/// constrain the value. This is suitable for continuous, stepped and logarithmic
/// controls; mapping the displayed value to and from the normalized position
/// remains with the application.
pub fn knob(
    ui: &mut Ui,
    surface: Surface,
    value: &mut f32,
    range: RangeInclusive<f32>,
    step: Option<f32>,
    marks: &[f32],
) -> Response {
    let diameter = ui.spacing().interact_size.y.max(44.0);
    let (rect, mut response) =
        ui.allocate_exact_size(vec2(diameter, diameter), egui::Sense::click_and_drag());
    if response.clicked() || response.drag_started() {
        response.request_focus();
    }
    let start = *range.start();
    let end = *range.end();
    let span = (end - start).abs();
    let fraction = if span <= f32::EPSILON {
        0.0
    } else {
        ((*value - start) / (end - start)).clamp(0.0, 1.0)
    };

    let start_angle = -135.0_f32.to_radians();
    let sweep = 270.0_f32.to_radians();
    let pointer_fraction = ui.input(|input| {
        let pointer = input.pointer.interact_pos()?;
        let delta = pointer - rect.center();
        let angle = delta.y.atan2(delta.x);
        let mut offset = angle - start_angle;
        while offset < 0.0 {
            offset += std::f32::consts::TAU;
        }
        (offset <= sweep).then(|| (offset / sweep).clamp(0.0, 1.0))
    });
    let mut next_fraction = fraction;
    if response.dragged() || response.is_pointer_button_down_on() {
        if let Some(pointer) = pointer_fraction {
            next_fraction = pointer;
        } else {
            let delta = response.drag_delta();
            next_fraction += (delta.x - delta.y) / (diameter * 2.5);
            next_fraction = next_fraction.clamp(0.0, 1.0);
        }
    }
    let mut keyboard_changed = false;
    if response.has_focus() || response.hovered() {
        let keyboard_step = step
            .filter(|step| *step > 0.0)
            .unwrap_or_else(|| span / 100.0);
        let backward = ui.input_mut(|input| {
            input.consume_key(egui::Modifiers::NONE, Key::ArrowLeft)
                || input.consume_key(egui::Modifiers::NONE, Key::ArrowDown)
        });
        let forward = ui.input_mut(|input| {
            input.consume_key(egui::Modifiers::NONE, Key::ArrowRight)
                || input.consume_key(egui::Modifiers::NONE, Key::ArrowUp)
        });
        if backward || forward {
            keyboard_changed = true;
            let delta = if forward {
                keyboard_step
            } else {
                -keyboard_step
            };
            next_fraction =
                ((*value + delta - start) / (end - start).max(f32::EPSILON)).clamp(0.0, 1.0);
        }
    }

    let mut next = start + (end - start) * next_fraction;
    if let Some(step) = step.filter(|step| *step > 0.0) {
        next = (start + ((next - start) / step).round() * step).clamp(start, end);
        next_fraction = ((next - start) / (end - start).max(f32::EPSILON)).clamp(0.0, 1.0);
    }
    if (response.dragged() || keyboard_changed || response.is_pointer_button_down_on())
        && (*value - next).abs() > f32::EPSILON
    {
        *value = next;
        response.mark_changed();
    }

    if ui.is_rect_visible(rect) {
        let theme = Theme::of(ui);
        let painter = ui.painter();
        let center = rect.center();
        let radius = rect.width().min(rect.height()) * 0.36;
        let start_angle = -135.0_f32.to_radians();
        let sweep = 270.0_f32.to_radians();
        let point = |normalized: f32, ring: f32| {
            let angle = start_angle + sweep * normalized.clamp(0.0, 1.0);
            center + vec2(angle.cos() * ring, angle.sin() * ring)
        };
        let idle = theme.background(surface).lerp_to_gamma(theme.text, 0.23);
        painter.circle_filled(center, radius, idle);
        painter.circle_stroke(
            center,
            radius,
            Stroke::new(
                1.0,
                if response.has_focus() {
                    theme.focus
                } else {
                    theme.border
                },
            ),
        );

        if next_fraction > 0.0 {
            let segments = (next_fraction * 36.0).ceil().max(1.0) as usize;
            let arc_points = (0..=segments)
                .map(|index| {
                    #[expect(
                        clippy::cast_precision_loss,
                        reason = "the knob arc has at most 36 segments"
                    )]
                    let fraction = next_fraction * index as f32 / segments as f32;
                    point(fraction, radius)
                })
                .collect::<Vec<_>>();
            painter.add(egui::Shape::line(
                arc_points,
                Stroke::new(
                    3.0,
                    if response.enabled() {
                        theme.primary
                    } else {
                        theme.muted_text
                    },
                ),
            ));
        }

        for mark in marks {
            let mark = (*mark).clamp(0.0, 1.0);
            let outer = point(mark, radius + 5.0);
            let inner = point(mark, radius + 1.0);
            painter.line_segment(
                [inner, outer],
                Stroke::new(
                    1.0,
                    if mark <= next_fraction {
                        theme.primary
                    } else {
                        idle
                    },
                ),
            );
        }

        let thumb = point(next_fraction, radius);
        if response.has_focus() {
            painter.circle_stroke(center, radius + 4.0, Stroke::new(1.0, theme.focus));
        }
        painter.circle_filled(
            thumb,
            5.0,
            if response.enabled() {
                if response.dragged() {
                    theme.focus
                } else {
                    theme.text
                }
            } else {
                theme.muted_text
            },
        );
    }
    response
}

/// Controls whether numeric values appear beneath a stepped slider.
#[derive(Debug, Clone, Copy)]
pub enum StepLabels {
    Hidden,
    Values,
}

/// Draws evenly spaced stops starting at the lower bound. The last stop is the
/// highest aligned value within the range. Dense tracks omit overlapping labels.
pub fn stepped_slider(
    ui: &mut Ui,
    surface: Surface,
    value: &mut u16,
    range: RangeInclusive<u16>,
    step: NonZeroU16,
    labels: StepLabels,
) -> Response {
    let start = *range.start();
    let step = step.get();
    let stops = range.end().saturating_sub(start) / step;
    let end = start + stops * step;
    let response = slider_input(
        ui,
        Slider::new(value, start..=end)
            .step_by(f64::from(step))
            .smart_aim(false),
        22.0,
    );
    let track = SliderTrack::new(
        &response,
        f32::from(value.saturating_sub(start)) / f32::from((end - start).max(1)),
    );
    let theme = Theme::of(ui);
    let stride = tick_stride(stops, track.travel.span(), 12.0);
    let x_at = |tick| egui::lerp(track.travel, f32::from(tick) / f32::from(stops.max(1)));
    track.paint(ui, surface, &response, |painter, active, idle| {
        for tick in mark_indices(stops, stride) {
            let x = x_at(tick);
            let color = if response.enabled() && x <= track.thumb.x {
                active
            } else {
                idle
            };
            painter.rect_filled(
                Rect::from_center_size(pos2(x, track.thumb.y), vec2(10.5, 7.0)),
                CONTROL_RADIUS - 2,
                color,
            );
        }
    });
    if matches!(labels, StepLabels::Values) {
        let mut font = egui::TextStyle::Monospace.resolve(ui.style());
        font.size *= 0.85;
        let size = ui
            .painter()
            .layout_no_wrap(end.to_string(), font.clone(), theme.muted_text)
            .size();
        let stride = tick_stride(stops, track.travel.span(), size.x + 10.0);
        let (_, row) = ui.allocate_space(vec2(response.rect.width(), size.y));
        let mut right = row.left() - SPACE_SMALL;
        for tick in mark_indices(stops, stride) {
            let number = start + tick * step;
            let color = if number == *value && response.enabled() {
                theme.text
            } else {
                theme.muted_text
            };
            let galley = ui
                .painter()
                .layout_no_wrap(number.to_string(), font.clone(), color);
            let width = galley.size().x;
            let x =
                (x_at(tick) - width * 0.5).clamp(row.left(), (row.right() - width).max(row.left()));
            if x >= right + SPACE_SMALL && x + width <= row.right() {
                ui.painter().galley(pos2(x, row.top()), galley, color);
                right = x + width;
            }
        }
    }
    response
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the stride is derived from a u16 count"
)]
fn tick_stride(stops: u16, span: f32, gap: f32) -> usize {
    (gap * f32::from(stops) / span.max(1.0)).ceil().max(1.0) as usize
}

fn mark_indices(stops: u16, stride: usize) -> impl Iterator<Item = u16> {
    (0..stops).step_by(stride).chain(std::iter::once(stops))
}

const HANDLE_ASPECT: f32 = 0.72;

struct SliderTrack {
    travel: egui::Rangef,
    thumb: egui::Pos2,
}

impl SliderTrack {
    fn new(response: &Response, fraction: f32) -> Self {
        let rect = response.rect;
        // egui 0.37 uses this inset for the rectangular handle's input travel.
        let inset = (rect.height() / 2.5 * HANDLE_ASPECT).min(rect.width() * 0.5);
        let travel = rect.x_range().shrink(inset);
        let thumb = pos2(egui::lerp(travel, fraction), rect.center().y);
        Self { travel, thumb }
    }

    fn paint(
        &self,
        ui: &Ui,
        surface: Surface,
        response: &Response,
        paint_stops: impl FnOnce(&egui::Painter, egui::Color32, egui::Color32),
    ) {
        if !ui.is_rect_visible(response.rect) {
            return;
        }
        let theme = Theme::of(ui);
        let painter = ui.painter();
        let y = self.thumb.y;
        let rail = Rect::from_min_max(
            pos2(self.travel.min - 2.5, y - 2.5),
            pos2(self.travel.max + 2.5, y + 2.5),
        )
        .intersect(response.rect);
        let idle = theme.background(surface).lerp_to_gamma(theme.text, 0.23);
        painter.rect_filled(rail, 2, idle);
        if response.enabled() && self.thumb.x > self.travel.min {
            painter.rect_filled(
                Rect::from_min_max(rail.min, pos2(self.thumb.x, rail.bottom())),
                2,
                theme.primary,
            );
        }
        paint_stops(painter, theme.primary, idle);
        let engaged = response.hovered() || response.dragged() || response.has_focus();
        let fill = if !response.enabled() {
            theme.muted_text
        } else if engaged {
            theme.focus
        } else {
            theme.text
        };
        let thumb = Rect::from_center_size(self.thumb, vec2(response.rect.width().min(12.0), 8.0));
        if response.has_focus() {
            painter.rect_stroke(
                thumb.expand(2.0),
                6,
                Stroke::new(1.0, theme.focus),
                StrokeKind::Outside,
            );
        }
        painter.rect_filled(thumb, 4, fill);
    }
}

fn slider_input(ui: &mut Ui, slider: Slider<'_>, height: f32) -> Response {
    let width = ui.available_width().min(320.0);
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = width;
        ui.spacing_mut().interact_size.y = height;
        let widgets = &mut ui.visuals_mut().widgets;
        widgets.noninteractive.bg_fill = egui::Color32::TRANSPARENT;
        widgets.noninteractive.fg_stroke = Stroke::NONE;
        for state in super::widget_states(widgets) {
            state.bg_fill = egui::Color32::TRANSPARENT;
            state.fg_stroke = Stroke::NONE;
        }
        ui.add(slider.show_value(false).trailing_fill(false).handle_shape(
            egui::style::HandleShape::Rect {
                aspect_ratio: HANDLE_ASPECT,
            },
        ))
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(width: f32, events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 240.0))),
            events,
            ..Default::default()
        }
    }

    #[test]
    fn two_and_three_option_groups_slide_the_highlight_and_respect_zero_motion() {
        for options in [
            &[(0, "Names only"), (1, "Node cards")][..],
            &[(0, "Dots"), (1, "Lines"), (2, "Hidden")][..],
        ] {
            let ctx = egui::Context::default();
            let last = options.last().unwrap().0;
            let frame = |time, mut selected, duration| {
                let mut highlight_fill = egui::Color32::TRANSPARENT;
                let output = ctx.run_ui(
                    egui::RawInput {
                        time: Some(time),
                        ..input(280.0, vec![])
                    },
                    |ui| {
                        ui.style_mut().animation_time = duration;
                        let theme = Theme::of(ui);
                        highlight_fill = theme.selected_fill(Surface::Panel);
                        let label = ui.label("Options");
                        let _ = segmented(ui, Surface::Panel, &label, &mut selected, options);
                    },
                );
                let center = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Rect(rect) if rect.fill == highlight_fill => {
                            Some(rect.rect.center().x)
                        }
                        _ => None,
                    })
                    .unwrap();
                output.drop_without_applying_deltas();
                center
            };
            let left = frame(0.0, 0, 0.2);
            frame(0.02, last, 0.2);
            let middle = frame(0.10, last, 0.2);
            let right = frame(0.30, last, 0.2);
            assert!(left < middle && middle < right);
            assert_eq!(frame(0.33, 0, 0.0), left);
        }
    }

    #[test]
    fn segmented_wraps_and_arrow_keys_move_the_single_selection() {
        let ctx = egui::Context::default();
        let mut selected = 0;
        let mut frame = |width, events| {
            let mut rect = Rect::NOTHING;
            let mut focus = egui::Color32::TRANSPARENT;
            let output = ctx.run_ui(input(width, events), |ui| {
                focus = Theme::of(ui).focus;
                let label = ui.label("Grid type");
                rect = segmented(
                    ui,
                    Surface::Panel,
                    &label,
                    &mut selected,
                    &[(0, "Dots"), (1, "Lines"), (2, "Hidden")],
                )
                .rect;
            });
            let outlines = output.shapes.iter().filter(|shape| {
                matches!(&shape.shape, egui::Shape::Rect(rect) if rect.stroke.color == focus && rect.stroke.width > 0.0)
            }).count();
            assert_eq!(outlines, 1);
            output.drop_without_applying_deltas();
            (rect, selected)
        };
        let (narrow, _) = frame(90.0, vec![]);
        let (wide, _) = frame(240.0, vec![]);
        assert!(narrow.height() > wide.height() * 2.0);
        assert!(narrow.width() <= 90.5);
        let pos = wide.center();
        frame(240.0, vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            frame(
                240.0,
                vec![egui::Event::PointerButton {
                    pos,
                    pressed,
                    button: egui::PointerButton::Primary,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        assert_eq!(frame(240.0, vec![]).1, 1);
        assert_eq!(
            frame(
                240.0,
                vec![egui::Event::Key {
                    key: Key::ArrowRight,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }]
            )
            .1,
            2
        );
    }

    #[test]
    fn continuous_marked_slider_keeps_free_values_between_guides() {
        let ctx = egui::Context::default();
        let mut zoom = 1.37;
        let mut rect = Rect::NOTHING;
        ctx.run_ui(input(240.0, vec![]), |ui| {
            rect = continuous_marked_slider(
                ui,
                Surface::Dialog,
                &mut zoom,
                0.20..=1.75,
                &[0.193548, 0.516129, 1.0],
            )
            .rect;
        })
        .drop_without_applying_deltas();
        assert!(zoom == 1.37);
        assert!(rect.width() > 0.0);
    }

    #[test]
    fn guided_slider_supports_free_strict_notches_and_values() {
        let ctx = egui::Context::default();
        let guides = [
            SliderGuide {
                fraction: 0.0,
                label: "0",
            },
            SliderGuide {
                fraction: 0.5,
                label: "50",
            },
            SliderGuide {
                fraction: 1.0,
                label: "100",
            },
        ];
        let mut free = 37.0;
        let mut strict = 50.0;
        ctx.run_ui(input(240.0, vec![]), |ui| {
            let _ = guided_slider(
                ui,
                Surface::Panel,
                &mut free,
                0.0..=100.0,
                SliderBehavior::Free,
                SliderMarks::NotchesAndValues,
                &guides,
            );
            let _ = guided_slider(
                ui,
                Surface::Panel,
                &mut strict,
                0.0..=100.0,
                SliderBehavior::Step(25.0),
                SliderMarks::Notches,
                &guides,
            );
        })
        .drop_without_applying_deltas();
        assert_eq!(free, 37.0);
        assert_eq!(strict, 50.0);
    }

    #[test]
    fn continuous_drag_uses_values_between_the_stepped_stops() {
        let ctx = egui::Context::default();
        let mut continuous = 4.0;
        let mut stepped = 4;
        let mut frame = |events| {
            let mut rect = Rect::NOTHING;
            ctx.run_ui(input(240.0, events), |ui| {
                rect = continuous_slider(ui, Surface::Panel, &mut continuous, 4.0..=16.0).rect;
                let _ = stepped_slider(
                    ui,
                    Surface::Panel,
                    &mut stepped,
                    4..=17,
                    NonZeroU16::new(4).unwrap(),
                    StepLabels::Hidden,
                );
            })
            .drop_without_applying_deltas();
            (rect, continuous, stepped)
        };
        let rect = frame(vec![]).0;
        let pos = pos2(rect.left() + rect.width() * 0.44, rect.center().y);
        frame(vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            frame(vec![egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        let (_, value, tick) = frame(vec![]);
        assert!(value > 8.2 && value < 10.0, "{value}");
        assert_eq!(tick, 4);
    }

    #[test]
    fn stepped_slider_centers_pill_stops_on_the_rail_and_keeps_labels_optional() {
        for labels in [StepLabels::Hidden, StepLabels::Values] {
            let ctx = egui::Context::default();
            let mut value = 24;
            let mut rect = Rect::NOTHING;
            let mut colors = None;
            let output = ctx.run_ui(input(200.0, vec![]), |ui| {
                let theme = Theme::of(ui);
                colors = Some((theme.primary, theme.panel.lerp_to_gamma(theme.text, 0.23)));
                rect = stepped_slider(
                    ui,
                    Surface::Panel,
                    &mut value,
                    12..=48,
                    NonZeroU16::new(12).unwrap(),
                    labels,
                )
                .rect;
            });
            let track = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Rect(stop) if stop.rect.size() == vec2(10.5, 7.0) => {
                        assert_eq!(stop.stroke, Stroke::NONE);
                        Some((stop.rect.center(), stop.fill))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(track.len(), 4);
            assert!(
                track
                    .iter()
                    .all(|(p, _)| (p.y - rect.center().y).abs() < 0.01)
            );
            let (active, idle) = colors.unwrap();
            assert_eq!(
                track.iter().map(|(_, fill)| *fill).collect::<Vec<_>>(),
                [active, active, idle, idle]
            );
            let thumb = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(thumb) if thumb.rect.size() == vec2(12.0, 8.0) => Some(thumb),
                    _ => None,
                })
                .unwrap();
            assert_eq!(thumb.rect.center().y, rect.center().y);
            assert!((thumb.rect.center().x - track[1].0.x).abs() < 0.01);
            let values = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => {
                        assert!(text.pos.y >= rect.bottom());
                        Some(text.galley.text())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            match labels {
                StepLabels::Hidden => assert!(values.is_empty()),
                StepLabels::Values => assert_eq!(values, ["12", "24", "36", "48"]),
            }
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn horizontal_notebook_overflows_to_a_fixed_height_menu() {
        let ctx = egui::Context::default();
        let mut selected = 0;
        let mut rects = Vec::new();
        let mut has_overflow = false;
        let mut ellipsis_width = None;
        let mut frame = |width| {
            let output = ctx.run_ui(input(width, vec![]), |ui| {
                let response = notebook_styled(
                    ui,
                    Surface::Panel,
                    NotebookOrientation::Horizontal,
                    NotebookPadding::Comfortable,
                    &mut selected,
                    &[
                        (0, "Nodes"),
                        (1, "Variables"),
                        (2, "Connections"),
                        (3, "Types"),
                    ],
                );
                rects.push(response.rect);
            });
            has_overflow |= output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "•••")
            });
            ellipsis_width =
                ellipsis_width.or(output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "•••" => {
                        Some(text.galley.size().x)
                    }
                    _ => None,
                }));
            output.drop_without_applying_deltas();
        };
        frame(180.0);
        frame(420.0);
        assert!(has_overflow);
        assert!(ellipsis_width.is_some_and(|width| width > 0.0 && width < 30.0));
        assert!(rects.iter().all(|rect| rect.height() <= 38.0));

        selected = 3;
        let output = ctx.run_ui(input(180.0, vec![]), |ui| {
            let _ = notebook_styled(
                ui,
                Surface::Panel,
                NotebookOrientation::Horizontal,
                NotebookPadding::Comfortable,
                &mut selected,
                &[
                    (0, "Nodes"),
                    (1, "Variables"),
                    (2, "Connections"),
                    (3, "Types"),
                ],
            );
        });
        let types_drawn = output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Types")
        });
        output.drop_without_applying_deltas();
        assert!(types_drawn || has_overflow);
    }

    #[test]
    fn knob_visits_steps_with_arrow_keys_and_paints_marks() {
        let ctx = egui::Context::default();
        let mut value = 0.0;
        let mut frame = |events| {
            let mut rect = Rect::NOTHING;
            ctx.run_ui(input(120.0, events), |ui| {
                rect = knob(
                    ui,
                    Surface::Panel,
                    &mut value,
                    -12.0..=12.0,
                    Some(1.0),
                    &[-12.0_f32 / 24.0 + 0.5, 0.5, 12.0_f32 / 24.0 + 0.5],
                )
                .rect;
            })
            .drop_without_applying_deltas();
            rect
        };
        let rect = frame(vec![]);
        let pos = rect.center();
        frame(vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            frame(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        frame(vec![egui::Event::Key {
            key: Key::ArrowRight,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert!(rect.width() >= 44.0);
        assert_eq!(value, 1.0);
    }

    #[test]
    fn stepped_slider_keyboard_visits_each_stop_and_keeps_the_range_aligned() {
        let ctx = egui::Context::default();
        let mut selected = 4;
        let mut frame = |events, focus| {
            ctx.run_ui(input(240.0, events), |ui| {
                let slider = stepped_slider(
                    ui,
                    Surface::Panel,
                    &mut selected,
                    4..=17,
                    NonZeroU16::new(4).unwrap(),
                    StepLabels::Hidden,
                );
                if focus {
                    slider.request_focus();
                }
            })
            .drop_without_applying_deltas();
            selected
        };
        assert_eq!(frame(vec![], true), 4);
        for expected in [8, 12, 16, 16] {
            assert_eq!(
                frame(
                    vec![egui::Event::Key {
                        key: Key::ArrowRight,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    false
                ),
                expected
            );
            frame(
                vec![egui::Event::Key {
                    key: Key::ArrowRight,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                false,
            );
        }
    }
}
