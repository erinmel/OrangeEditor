use super::{CONTROL_RADIUS, ControlSize, SPACE_SMALL, Surface, Theme};
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
    let tokens = crate::UiTokens::for_surface(ui, surface);
    tokens.scope(ui, None, |ui, tokens| {
        pill_segmented(ui, tokens, label, value, options)
    })
}

/// Two fixed-width choices that work in zoomed cards and ordinary forms.
#[derive(Debug, Clone, Copy)]
pub struct BinaryChoice<'a> {
    pub labels: [&'a str; 2],
    pub selected: Option<bool>,
    pub defaulted: bool,
    pub surface: Surface,
    pub zoom: f32,
}

impl BinaryChoice<'_> {
    pub fn show(self, ui: &Ui, rect: Rect, id: egui::Id) -> Option<bool> {
        self.show_named(ui, rect, id, self.labels)
    }

    /// Draws the same choices with names that identify their field to assistive tools.
    pub fn show_named(self, ui: &Ui, rect: Rect, id: egui::Id, names: [&str; 2]) -> Option<bool> {
        let (no, yes) = rect.split_left_right_at_fraction(0.5);
        let mut picked = None;
        let responses = [(0, no), (1, yes)].map(|(index, half)| {
            let value = index == 1;
            let response = ui.interact(half, id.with(index), egui::Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::Role::RadioButton,
                    response.enabled(),
                    self.selected == Some(value),
                    names[index],
                )
            });
            if response.clicked() && (self.defaulted || self.selected != Some(value)) {
                picked = Some(value);
            }
            response
        });
        self.paint(ui, rect, Some([&responses[0], &responses[1]]));
        picked
    }

    pub fn paint_preview(self, ui: &Ui, rect: Rect) {
        self.paint(ui, rect, None);
    }

    fn paint(self, ui: &Ui, rect: Rect, responses: Option<[&Response; 2]>) {
        if !ui.is_rect_visible(rect) {
            return;
        }
        let tokens = crate::UiTokens::for_surface(ui, self.surface);
        let theme = tokens.theme;
        let painter = ui.painter();
        let zoom = self.zoom;
        let fill = theme.bg_input();
        painter.rect(
            rect,
            5.0 * zoom,
            fill,
            theme.control_border(),
            StrokeKind::Inside,
        );
        let (no, yes) = rect.split_left_right_at_fraction(0.5);
        for (index, half) in [no, yes].into_iter().enumerate() {
            let chosen = self.selected == Some(index == 1);
            let bounds = half.shrink(2.0 * zoom);
            let response = responses.map(|items| items[index]);
            let mut ink = theme.text;
            if chosen {
                let highlight = if self.defaulted {
                    fill.lerp_to_gamma(theme.primary, 0.55)
                } else {
                    theme.primary
                };
                ink = crate::theme::ensure_contrast(theme.on_primary, highlight, 4.5);
                painter.rect_filled(bounds, 4.0 * zoom, highlight);
            } else if response.is_some_and(Response::hovered) {
                painter.rect_filled(bounds, 4.0 * zoom, theme.bg_elevated());
            }
            painter.text(
                bounds.center(),
                egui::Align2::CENTER_CENTER,
                self.labels[index],
                egui::FontId::proportional(12.5 * zoom),
                ink,
            );
            if response.is_some_and(Response::has_focus) {
                painter.rect_stroke(bounds, 4.0 * zoom, theme.focus_ring, StrokeKind::Inside);
            }
        }
    }
}

/// Intrinsic pill group with a sliding selection and a stacked narrow layout.
pub fn pill_segmented<T: Copy + PartialEq>(
    ui: &mut Ui,
    tokens: crate::UiTokens,
    label: &Response,
    value: &mut T,
    options: &[(T, &str)],
) -> Response {
    let theme = tokens.theme;
    let spacing = tokens.density.spacing().xs;
    let width = ui.available_width();
    let height = tokens.density.row_height();
    let font = egui::TextStyle::Button.resolve(ui.style());
    let natural = options.iter().fold(0.0_f32, |width, (_, text)| {
        width.max(
            ui.painter()
                .layout_no_wrap((*text).to_owned(), font.clone(), theme.text)
                .size()
                .x
                + 2.0 * tokens.density.spacing().md
                + 8.0,
        )
    });
    let columns = options.len().max(1);
    #[expect(
        clippy::cast_precision_loss,
        reason = "the group contains a small number of options"
    )]
    let columns = columns as f32;
    let body_width = (width - 2.0 * (spacing + theme.border.width)).max(0.0);
    let single_row = (natural + spacing) * columns - spacing <= body_width;
    let segment_width = natural.min(body_width);
    let mut next = None;
    let mut responses = Vec::with_capacity(options.len());
    let mut highlight = None;
    let mut group = ui
        .push_id(label.id, |ui| {
            egui::Frame::new()
                .fill(theme.surface_elevated)
                .stroke(theme.border_strong)
                .corner_radius(if single_row { 255 } else { theme.radius })
                .inner_margin(egui::Margin::same(spacing as i8))
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
                                        .min_size(vec2(segment_width, height)),
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
                            responses.push(if !single_row && body_width < natural {
                                response.on_hover_text(*text).on_disabled_hover_text(*text)
                            } else {
                                response
                            });
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
        paint_selection(ui, theme, label, shape, &responses, index);
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
        if response.has_focus() && !response.hovered() {
            let center = response.rect.center_bottom() - vec2(0.0, 2.0);
            ui.painter()
                .hline(center.x - 8.0..=center.x + 8.0, center.y, theme.focus_ring);
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

fn notebook_tab(
    ui: &Ui,
    painter: &egui::Painter,
    response: &Response,
    label: &str,
    font: &egui::FontId,
    theme: Theme,
    selected: bool,
) -> bool {
    let rect = response.rect;
    if selected || response.hovered() || response.has_focus() {
        let fill = if selected { theme.primary } else { theme.inset };
        let stroke = if response.has_focus() || selected {
            Stroke::new(1.0, theme.focus)
        } else {
            Stroke::NONE
        };
        painter.rect(rect, 4, fill, stroke, StrokeKind::Inside);
    }
    let ink = if selected {
        theme.on_primary
    } else {
        theme.text
    };
    let job = egui::text::LayoutJob::simple_singleline(label.to_owned(), font.clone(), ink);
    let mut job = job;
    job.wrap.max_width = (rect.width() - 4.0).max(0.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = painter.layout_job(job);
    painter
        .with_clip_rect(rect)
        .galley(rect.center() - galley.size() * 0.5, galley, ink);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked()
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
    ui.scope(|ui| match orientation {
        NotebookOrientation::Horizontal => {
            let compact = padding == NotebookPadding::Compact;
            let height = if compact {
                23.0
            } else {
                ControlSize::Standard.height()
            };
            let inset = if compact { 3.0 } else { 5.0 };
            let width = (ui.clip_rect().right() - ui.next_widget_position().x - 1.0)
                .min(padded_width)
                .max(0.0);
            let (outer, mut group) =
                ui.allocate_exact_size(vec2(width, height + 2.0 * inset), egui::Sense::hover());
            if options.is_empty() || !ui.is_rect_visible(outer) {
                return group;
            }
            let inner = outer.shrink(inset);
            if inner.width() <= 0.0 {
                return group;
            }
            let theme = Theme::of(ui);
            let font = egui::TextStyle::Button.resolve(ui.style());
            let widths: Vec<_> = options
                .iter()
                .map(|(_, text)| {
                    ui.painter()
                        .layout_no_wrap((*text).to_owned(), font.clone(), theme.text)
                        .size()
                        .x
                        + if compact { 8.0 } else { 16.0 }
                })
                .collect();
            let gap = 2.0;
            #[expect(clippy::cast_precision_loss, reason = "small number of tabs")]
            let gaps = gap * options.len().saturating_sub(1) as f32;
            let natural = widths.iter().sum::<f32>();
            let compress = ((inner.width() - gaps) / natural).clamp(0.0, 1.0);
            let needs_menu = compress < if compact { 0.89 } else { 1.0 };
            let menu_width = if needs_menu {
                inner.width().min(27.0)
            } else {
                0.0
            };
            let selected = options.iter().position(|(option, _)| *value == *option);
            let mut visible = vec![!needs_menu; options.len()];
            let mut used = 0.0;
            if needs_menu {
                for index in selected.into_iter().chain(0..options.len()) {
                    if !visible[index] && used + widths[index] + gap + menu_width <= inner.width() {
                        visible[index] = true;
                        used += widths[index] + gap;
                    }
                }
            }
            let extra = if width_mode == NotebookWidth::Flex {
                #[expect(clippy::cast_precision_loss, reason = "small number of tabs")]
                let count = visible.iter().filter(|&&shown| shown).count().max(1) as f32;
                let spare = if needs_menu {
                    inner.width() - menu_width - used
                } else {
                    inner.width() - gaps - natural * compress
                };
                spare.max(0.0) / count
            } else {
                0.0
            };
            let painter = ui.painter_at(outer);
            painter.rect(
                outer,
                CONTROL_RADIUS,
                theme.background(surface).lerp_to_gamma(theme.inset, 0.35),
                Stroke::new(1.0, theme.border),
                StrokeKind::Inside,
            );
            let mut left = inner.left();
            for (index, (option, text)) in options
                .iter()
                .enumerate()
                .filter(|(index, _)| visible[*index])
            {
                let tab_width = widths[index] * if needs_menu { 1.0 } else { compress } + extra;
                let tab = Rect::from_min_size(pos2(left, inner.top()), vec2(tab_width, height))
                    .intersect(inner);
                let response = ui.interact(tab, group.id.with(index), egui::Sense::click());
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::Role::Tab,
                        response.enabled(),
                        *value == *option,
                        *text,
                    )
                });
                if notebook_tab(
                    ui,
                    &painter,
                    &response,
                    text,
                    &font,
                    theme,
                    *value == *option,
                ) && *value != *option
                {
                    *value = *option;
                    group.mark_changed();
                }
                left += tab_width + gap;
            }
            if needs_menu {
                let rect = Rect::from_min_max(
                    pos2(
                        (inner.right() - menu_width).max(left.min(inner.right())),
                        inner.top(),
                    ),
                    inner.max,
                );
                let overflow = ui.interact(rect, group.id.with("overflow"), egui::Sense::click());
                let label = selected
                    .filter(|&index| !visible[index])
                    .map_or("…", |index| options[index].1);
                notebook_tab(ui, &painter, &overflow, "…", &font, theme, false);
                overflow.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::Button, overflow.enabled(), label)
                });
                egui::Popup::menu(&overflow).show(|ui| {
                    for (index, (option, text)) in options
                        .iter()
                        .enumerate()
                        .filter(|(index, _)| !visible[*index])
                    {
                        if ui
                            .selectable_label(selected == Some(index), *text)
                            .clicked()
                        {
                            if *value != *option {
                                *value = *option;
                                group.mark_changed();
                            }
                            ui.close();
                        }
                    }
                });
            }
            group
        }
        NotebookOrientation::Vertical => {
            ui.set_width(padded_width);
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
                    ui.set_min_width(ui.available_width().max(0.0));
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
            response.unwrap_or_else(|| ui.allocate_response(vec2(width, 0.0), egui::Sense::hover()))
        }
    })
    .inner
}

fn paint_selection(
    ui: &Ui,
    theme: crate::theme::Theme,
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
    let fill = theme.surface_elevated.lerp_to_gamma(theme.primary, 0.21);
    let fade = if ui.is_enabled() { 1.0 } else { 0.45 };
    ui.painter().set(
        shape,
        egui::epaint::RectShape::new(
            rect,
            255,
            fill.gamma_multiply(fade),
            Stroke::new(1.0, theme.focus_ring.color.gamma_multiply(fade)),
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
    SliderTrack::new(response.rect, fraction).paint(ui, surface, Some(&response), |_, _, _| {});
    response
}

/// Draws the shared slider on a logarithmic scale for a positive, finite range.
pub fn logarithmic_slider(
    ui: &mut Ui,
    surface: Surface,
    value: &mut f32,
    range: RangeInclusive<f32>,
) -> Response {
    let start = f64::from(*range.start());
    let end = f64::from(*range.end());
    if start <= 0.0 || !start.is_finite() || !end.is_finite() || end <= start {
        return continuous_slider(ui, surface, value, range);
    }
    let response = slider_input(
        ui,
        Slider::new(value, range).logarithmic(true).smart_aim(false),
        22.0,
    );
    #[expect(
        clippy::cast_possible_truncation,
        reason = "normalized screen coordinate"
    )]
    let fraction = ((f64::from(*value).clamp(start, end) / start).ln() / (end / start).ln()) as f32;
    SliderTrack::new(response.rect, fraction).paint(ui, surface, Some(&response), |_, _, _| {});
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
    let track = SliderTrack::new(response.rect, fraction);
    track.paint(ui, surface, Some(&response), |painter, active, idle| {
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
    let track = SliderTrack::new(response.rect, fraction);
    track.paint(ui, surface, Some(&response), |painter, active, idle| {
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
                    .galley(pos2(x, track.thumb.y + 7.0), galley, theme.muted_text);
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
    SliderTrack::new(response.rect, *fraction).marked(ui, surface, Some(&response), marks);
    response
}

/// Paints a marked track in a read-only preview, with no input registration.
pub fn marked_slider_preview(ui: &Ui, rect: Rect, surface: Surface, fraction: f32, marks: &[f32]) {
    SliderTrack::new(rect, fraction).marked(ui, surface, None, marks);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KnobStyle {
    #[default]
    Arc,
    Bipolar,
    Notched,
    /// A full-turn dial whose upper bound wraps to its lower bound.
    Loop,
}
/// Draws a rotary value control with optional snapping and visual marks.
/// Drag up or right to increase it. Hold Shift while dragging for finer control.
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
    rotary(ui, surface, value, range, step, marks, KnobStyle::Arc)
}

/// Relative rotary control with an optional center detent or outer tick marks.
pub fn rotary(
    ui: &mut Ui,
    surface: Surface,
    value: &mut f32,
    range: RangeInclusive<f32>,
    step: Option<f32>,
    marks: &[f32],
    style: KnobStyle,
) -> Response {
    let start = *range.start();
    let end = *range.end();
    let span = end - start;
    let step = step.filter(|step| step.is_finite() && *step > 0.0);
    let looping = style == KnobStyle::Loop;
    let normalize = |raw| rotary_value(raw, &range, None, looping);
    let commit = |raw| rotary_value(raw, &range, step, looping);
    let diameter =
        (ui.spacing().interact_size.y.max(24.0) + 45.0).min(ui.available_width().max(1.0));
    let (rect, mut response) =
        ui.allocate_exact_size(vec2(diameter, diameter), egui::Sense::click_and_drag());
    if response.clicked() || response.drag_started() {
        response.request_focus();
    }
    let edit_id = response.id.with("knob-edit");
    if response.double_clicked() {
        ui.ctx()
            .data_mut(|data| data.insert_temp(edit_id, value.to_string()));
    }
    let edit = ui.ctx().data(|data| data.get_temp::<String>(edit_id));
    if let Some(mut buffer) = edit {
        let edit_rect = Rect::from_center_size(
            rect.center(),
            vec2(rect.width() - 2.0, ui.spacing().interact_size.y),
        );
        let field = ui.put(
            edit_rect,
            egui::TextEdit::singleline(&mut buffer).id(edit_id.with("value")),
        );
        if response.double_clicked() {
            field.request_focus();
        }
        let [enter, escape] = super::consume_plain_keys(ui, [Key::Enter, Key::Escape]);
        if enter || field.lost_focus() || escape {
            if !escape
                && let Ok(parsed) = buffer.parse::<f32>()
                && parsed.is_finite()
                && end > start
            {
                let parsed = commit(parsed);
                if parsed != *value {
                    *value = parsed;
                    response.mark_changed();
                }
            }
            ui.ctx().data_mut(|data| data.remove::<String>(edit_id));
        } else {
            ui.ctx().data_mut(|data| data.insert_temp(edit_id, buffer));
        }
        return response;
    }
    let drag_id = response.id.with("raw-knob-value");
    let mut next = *value;
    if span > 0.0 && response.dragged() {
        let delta = response.drag_delta();
        let sensitivity = ui.input(|input| if input.modifiers.shift { 0.2 } else { 1.0 });
        next = ui.ctx().data_mut(|data| {
            let previous = if response.drag_started() {
                *value
            } else {
                data.get_temp(drag_id).unwrap_or(*value)
            };
            let raw = normalize(previous + span * (delta.x - delta.y) * sensitivity / 160.0);
            data.insert_temp(drag_id, raw);
            raw
        });
    } else if response.drag_stopped() {
        ui.ctx().data_mut(|data| data.remove::<f32>(drag_id));
    }
    if span > 0.0 && response.has_focus() {
        let keyboard_step = step.unwrap_or(span / 100.0);
        let [left, down, right, up] = super::consume_plain_keys(
            ui,
            [
                Key::ArrowLeft,
                Key::ArrowDown,
                Key::ArrowRight,
                Key::ArrowUp,
            ],
        );
        if left || down {
            next -= keyboard_step;
        }
        if right || up {
            next += keyboard_step;
        }
    }
    if span > 0.0 && response.hovered() && response.has_focus() {
        let wheel = ui.input_mut(|input| {
            let wheel = input
                .raw
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                    _ => None,
                })
                .sum::<f32>();
            input.smooth_scroll_delta.y = 0.0;
            wheel
        });
        if wheel != 0.0 {
            next += wheel.signum() * step.unwrap_or(span / 100.0);
        }
    }
    if style == KnobStyle::Bipolar && response.dragged() && next.abs() < span * 0.018 {
        next = 0.0;
    }
    if span > 0.0 && next != *value {
        next = commit(next);
        if next != *value {
            *value = next;
            response.mark_changed();
        }
    }
    response.widget_info(|| egui::WidgetInfo::slider(response.enabled(), f64::from(*value), ""));

    paint_rotary(
        ui,
        rect,
        surface,
        *value,
        &range,
        step,
        marks,
        style,
        Some(&response),
    );
    response
}

/// Paints the same rotary face in a read-only card preview.
#[allow(clippy::too_many_arguments, reason = "one rotary face's display state")]
pub fn rotary_preview(
    ui: &Ui,
    rect: Rect,
    surface: Surface,
    value: f32,
    range: RangeInclusive<f32>,
    step: Option<f32>,
    marks: &[f32],
    style: KnobStyle,
) {
    paint_rotary(ui, rect, surface, value, &range, step, marks, style, None);
}

#[allow(clippy::too_many_arguments, reason = "one rotary face's display state")]
fn paint_rotary(
    ui: &Ui,
    rect: Rect,
    surface: Surface,
    value: f32,
    range: &RangeInclusive<f32>,
    step: Option<f32>,
    marks: &[f32],
    style: KnobStyle,
    response: Option<&Response>,
) {
    if ui.is_rect_visible(rect) {
        let theme = Theme::of(ui);
        let start = *range.start();
        let end = *range.end();
        let span = end - start;
        let looping = style == KnobStyle::Loop;
        let diameter = rect.width();
        let scale = (diameter / 60.0).clamp(0.35, 1.2);
        let enabled = response.is_none_or(Response::enabled);
        let painter = ui.painter();
        let fraction = if span > 0.0 {
            ((value - start) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let center = rect.center();
        let radius = rect.width() * 0.35;
        let point = |normalized: f32, ring: f32| {
            let angle = if looping {
                -90.0 + 360.0 * normalized
            } else {
                135.0 + 270.0 * normalized
            }
            .to_radians();
            center + vec2(angle.cos() * ring, angle.sin() * ring)
        };
        let arc = |from: f32, through: f32, stroke| {
            let count = (radius * (through - from).abs()).ceil().clamp(8.0, 48.0) as u8;
            painter.add(egui::Shape::line(
                (0..=count)
                    .map(|i| {
                        point(
                            egui::lerp(from..=through, f32::from(i) / f32::from(count)),
                            radius,
                        )
                    })
                    .collect(),
                stroke,
            ));
        };
        let focus = if response.is_some_and(Response::has_focus) {
            theme.focus
        } else {
            theme.border
        };
        let ink = if enabled {
            theme.primary
        } else {
            theme.muted_text
        };
        painter.circle_filled(
            center,
            radius * 0.76,
            theme.background(surface).lerp_to_gamma(theme.text, 0.08),
        );
        painter.circle_stroke(center, radius * 0.76, Stroke::new(1.0, focus));
        arc(0.0, 1.0, Stroke::new(2.5, theme.border));
        let zero = if style == KnobStyle::Bipolar && start < 0.0 && end > 0.0 {
            -start / span
        } else {
            0.0
        };
        if fraction != zero && !looping {
            arc(zero, fraction, Stroke::new(2.5, ink));
        }
        let marks = if style == KnobStyle::Notched && marks.is_empty() {
            &[0.0, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875, 1.0]
        } else {
            marks
        };
        for (index, mark) in marks.iter().enumerate() {
            let mark = mark.clamp(0.0, 1.0);
            let major = index == 0
                || index + 1 == marks.len()
                || (mark - 0.5).abs() < 0.001
                || index % 5 == 0;
            let active = !looping && (zero.min(fraction)..=zero.max(fraction)).contains(&mark);
            painter.line_segment(
                [
                    point(mark, radius + 3.0 * scale),
                    point(mark, radius + if major { 7.0 * scale } else { 5.0 * scale }),
                ],
                Stroke::new(
                    if major { 1.6 } else { 1.0 },
                    if active { ink } else { theme.muted_text },
                ),
            );
        }
        painter.line_segment(
            [
                point(fraction, radius * 0.82),
                point(fraction, radius * 1.02),
            ],
            Stroke::new(
                2.0,
                if enabled {
                    theme.text
                } else {
                    theme.muted_text
                },
            ),
        );
        let label = if style == KnobStyle::Bipolar {
            if value.abs() < 0.5 {
                "C".to_owned()
            } else if value < 0.0 {
                format!("L{:.0}", value.abs())
            } else {
                format!("R{value:.0}")
            }
        } else if step.is_some() {
            format!("{value:.0}")
        } else {
            format!("{value:.1}")
        };
        let font = |size| egui::FontId::monospace(size);
        let size = (diameter * 0.2).clamp(6.5, 15.0);
        let galley = painter.layout_no_wrap(label.clone(), font(size), theme.text);
        let max_width = radius * 1.4;
        let galley = if galley.size().x > max_width {
            painter.layout_no_wrap(label, font(size * max_width / galley.size().x), theme.text)
        } else {
            galley
        };
        painter.galley(center - galley.size() * 0.5, galley, theme.text);
    }
}

fn rotary_value(raw: f32, range: &RangeInclusive<f32>, step: Option<f32>, looping: bool) -> f32 {
    let start = *range.start();
    let end = *range.end();
    let span = end - start;
    if !raw.is_finite() || !span.is_finite() || span <= 0.0 {
        return start;
    }
    let value = step.map_or(raw, |step| start + ((raw - start) / step).round() * step);
    if looping {
        start + (value - start).rem_euclid(span)
    } else {
        value.clamp(start, end)
    }
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
        response.rect,
        f32::from(value.saturating_sub(start)) / f32::from((end - start).max(1)),
    );
    let theme = Theme::of(ui);
    let stride = tick_stride(stops, track.travel.span(), 12.0);
    let x_at = |tick| egui::lerp(track.travel, f32::from(tick) / f32::from(stops.max(1)));
    track.paint(ui, surface, Some(&response), |painter, active, idle| {
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
                ui.painter()
                    .galley(pos2(x, track.thumb.y + 7.0), galley, color);
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
    rect: Rect,
    travel: egui::Rangef,
    thumb: egui::Pos2,
}

impl SliderTrack {
    fn new(rect: Rect, fraction: f32) -> Self {
        // egui 0.37 uses this inset for the rectangular handle's input travel.
        let inset = (rect.height() / 2.5 * HANDLE_ASPECT).min(rect.width() * 0.5);
        let travel = rect.x_range().shrink(inset);
        let thumb = pos2(egui::lerp(travel, fraction), rect.center().y);
        Self {
            rect,
            travel,
            thumb,
        }
    }

    fn marked(&self, ui: &Ui, surface: Surface, response: Option<&Response>, marks: &[f32]) {
        self.paint(ui, surface, response, |painter, active, idle| {
            for mark in marks {
                let x = egui::lerp(self.travel, *mark);
                painter.rect_filled(
                    Rect::from_center_size(pos2(x, self.thumb.y), vec2(10.5, 7.0)),
                    CONTROL_RADIUS - 2,
                    if x <= self.thumb.x && response.is_none_or(Response::enabled) {
                        active
                    } else {
                        idle
                    },
                );
            }
        });
    }

    fn paint(
        &self,
        ui: &Ui,
        surface: Surface,
        response: Option<&Response>,
        paint_stops: impl FnOnce(&egui::Painter, egui::Color32, egui::Color32),
    ) {
        if !ui.is_rect_visible(self.rect) {
            return;
        }
        let enabled = response.is_none_or(Response::enabled);
        let theme = Theme::of(ui);
        let painter = ui.painter();
        let y = self.thumb.y;
        let rail = Rect::from_min_max(
            pos2(self.travel.min - 2.5, y - 2.5),
            pos2(self.travel.max + 2.5, y + 2.5),
        )
        .intersect(self.rect);
        let background = theme.background(surface);
        let idle = crate::theme::ensure_contrast(
            background.lerp_to_gamma(theme.text, 0.23),
            background,
            3.0,
        );
        painter.rect_filled(rail, 2, idle);
        if enabled && self.thumb.x > self.travel.min {
            painter.rect_filled(
                Rect::from_min_max(rail.min, pos2(self.thumb.x, rail.bottom())),
                2,
                theme.primary,
            );
        }
        paint_stops(painter, theme.primary, idle);
        let engaged = response.is_some_and(|r| r.hovered() || r.dragged() || r.has_focus());
        let fill = if !enabled {
            theme.muted_text
        } else if engaged {
            theme.focus
        } else {
            theme.text
        };
        let size = (self.rect.height() * 0.38)
            .clamp(9.0, 14.0)
            .min(self.rect.width());
        let thumb =
            Rect::from_center_size(self.thumb, vec2((size * 1.5).min(self.rect.width()), size));
        if response.is_some_and(Response::has_focus) {
            painter.rect_stroke(
                thumb.expand(2.0),
                6,
                Stroke::new(1.0, theme.focus),
                StrokeKind::Outside,
            );
        }
        painter.rect_filled(thumb, 255, fill);
    }
}

fn slider_input(ui: &mut Ui, slider: Slider<'_>, height: f32) -> Response {
    let themed = crate::UiTokens::configured(ui.ctx());
    let width = if themed {
        ui.available_width()
    } else {
        ui.available_width().min(320.0)
    };
    let height = if themed {
        ui.spacing().interact_size.y.max(height)
    } else {
        height
    };
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

    #[test]
    fn stepped_encoder_wraps_in_both_directions_and_pan_clamps() {
        assert_eq!(rotary_value(360.0, &(0.0..=360.0), Some(5.0), true), 0.0);
        assert_eq!(rotary_value(-5.0, &(0.0..=360.0), Some(5.0), true), 355.0);
        assert_eq!(
            rotary_value(-53.0, &(-50.0..=50.0), Some(1.0), false),
            -50.0
        );
    }

    fn input(width: f32, events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 240.0))),
            events,
            ..Default::default()
        }
    }

    #[test]
    fn groups_slide_one_highlight_and_respect_zero_motion() {
        for options in [
            &[(0, "Names only"), (1, "Node cards")][..],
            &[(0, "Dots"), (1, "Lines"), (2, "Hidden")][..],
            &[(0, "A"), (1, "B"), (2, "C"), (3, "D")][..],
            &[(0, "1"), (1, "2"), (2, "3"), (3, "4"), (4, "5")][..],
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
                        highlight_fill = theme.inset.lerp_to_gamma(theme.primary, 0.21);
                        let label = ui.label("Options");
                        let _ = segmented(ui, Surface::Panel, &label, &mut selected, options);
                    },
                );
                let centers = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Rect(rect) if rect.fill == highlight_fill => {
                            Some(rect.rect.center().x)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                output.drop_without_applying_deltas();
                assert_eq!(centers.len(), 1);
                centers[0]
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
                matches!(&shape.shape, egui::Shape::Rect(rect) if rect.stroke.color == focus && rect.stroke.width == 1.0)
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
        assert_eq!(zoom, 1.37);
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
    fn logarithmic_slider_paints_a_full_rail_and_uses_a_geometric_midpoint() {
        let ctx = egui::Context::default();
        let mut value = 20_000.0;
        let mut frame = |events| {
            let mut rect = Rect::NOTHING;
            let out = ctx.run_ui(input(240.0, events), |ui| {
                rect = logarithmic_slider(ui, Surface::Card, &mut value, 20.0..=20_000.0).rect;
            });
            let has_rail = out.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Rect(rail)
                    if rail.rect.width() > rect.width() * 0.9
                        && rail.rect.height() == 5.0
                        && rail.fill != egui::Color32::TRANSPARENT)
            });
            out.drop_without_applying_deltas();
            (rect, value, has_rail)
        };
        let (rect, start, visible) = frame(vec![]);
        assert!(visible && start == 20_000.0);
        let pos = rect.center();
        frame(vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            frame(vec![egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        let (_, value, visible) = frame(vec![]);
        assert!(visible && (value - 632.46).abs() < 1.0, "{value}");
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
                colors = Some((
                    theme.primary,
                    crate::theme::ensure_contrast(
                        theme.panel.lerp_to_gamma(theme.text, 0.23),
                        theme.panel,
                        3.0,
                    ),
                ));
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
                    egui::Shape::Rect(thumb) if thumb.rect.size() == vec2(13.5, 9.0) => Some(thumb),
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
                        assert!((text.pos.y - rect.center().y - 7.0).abs() < 0.01);
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
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "…")
            });
            ellipsis_width =
                ellipsis_width.or(output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == "…" => {
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
    fn notebook_and_search_fit_the_visible_dock_during_resize() {
        let ctx = egui::Context::default();
        let mut selected = 2;
        let mut query = "frequency".to_owned();
        for visible_width in [243.0, 180.0, 110.0, 265.0] {
            let mut bounds = None;
            let output = ctx.run_ui(input(280.0, vec![]), |ui| {
                let mut clip = ui.clip_rect();
                clip.max.x = visible_width;
                ui.set_clip_rect(clip);
                let notebook = notebook_flex(
                    ui,
                    Surface::Panel,
                    NotebookOrientation::Horizontal,
                    NotebookPadding::Compact,
                    &mut selected,
                    &[
                        (0, "Nodos"),
                        (1, "Variables"),
                        (2, "Conexiones"),
                        (3, "Tipos"),
                    ],
                );
                let search = crate::search(
                    ui,
                    "resize-test",
                    &mut query,
                    "Search",
                    egui::Atom::from("+"),
                    egui::Atom::from("x"),
                    "Clear",
                );
                bounds = Some((notebook.rect, search.rect));
            });
            output.drop_without_applying_deltas();
            let (notebook, search) = bounds.unwrap();
            for rect in [notebook, search] {
                assert!(
                    rect.right() <= visible_width,
                    "{rect:?} exceeds {visible_width}"
                );
            }
            assert!(search.top() >= notebook.bottom());
            assert_eq!(selected, 2);
        }
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
    fn stepped_knob_accumulates_small_relative_drags_without_jumping() {
        let ctx = egui::Context::default();
        let mut value = 0.0;
        let mut frame = |events| {
            let mut rect = Rect::NOTHING;
            ctx.run_ui(input(240.0, events), |ui| {
                rect = knob(ui, Surface::Panel, &mut value, -12.0..=12.0, Some(1.0), &[]).rect;
            })
            .drop_without_applying_deltas();
            (rect, value)
        };
        let pos = frame(vec![]).0.center();
        frame(vec![egui::Event::PointerMoved(pos)]);
        let (_, value) = frame(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert_eq!(value, 0.0);
        for delta in 1u8..=30 {
            frame(vec![egui::Event::PointerMoved(
                pos + vec2(f32::from(delta), 0.0),
            )]);
        }
        assert!((3.0..=5.0).contains(&frame(vec![]).1));
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
