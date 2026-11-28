use egui::{Button, Response, Ui};

use crate::{Action, Dropdown, Surface, UiTokens, button};

/// Full-width sidebar destination. A disclosure row also exposes its expanded
/// state to accessibility clients. The host supplies the icon and localized name.
pub fn sidebar_row(
    ui: &mut Ui,
    label: &str,
    selected: bool,
    expanded: Option<bool>,
    icon: impl FnOnce(&Ui, egui::Rect, egui::Color32),
) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ui.spacing().interact_size.y.max(28.0)),
        egui::Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, response.enabled(), selected, label)
    });
    if let Some(open) = expanded {
        ui.ctx()
            .accesskit_node_builder(response.id, |node| node.set_expanded(open));
    }
    if ui.is_rect_visible(rect) {
        let palette = crate::Theme::of(ui);
        let fill = if selected {
            palette.panel.lerp_to_gamma(palette.primary, 0.18)
        } else if response.hovered() {
            palette.panel.lerp_to_gamma(palette.text, 0.09)
        } else {
            palette.panel
        };
        let ink = crate::theme::ensure_contrast(palette.text, fill, crate::theme::TEXT_CONTRAST);
        let painter = ui.painter_at(rect);
        if selected || response.hovered() {
            painter.rect_filled(rect, super::control_radius(ui.ctx()), fill);
        }
        if selected {
            painter.rect_filled(
                egui::Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
                0,
                palette.primary,
            );
        }
        let center = egui::pos2(rect.left() + 15.0, rect.center().y);
        icon(
            ui,
            egui::Rect::from_center_size(center, egui::Vec2::splat(18.0)),
            ink,
        );
        let end = if let Some(open) = expanded {
            crate::paint_chevron(
                ui,
                egui::pos2(rect.right() - 10.0, center.y),
                if open {
                    0.0
                } else {
                    -std::f32::consts::FRAC_PI_2
                },
                ink,
            );
            rect.right() - 21.0
        } else {
            rect.right() - 4.0
        };
        let left = rect.left() + 30.0;
        if end > left {
            painter
                .with_clip_rect(egui::Rect::from_min_max(
                    egui::pos2(left, rect.top()),
                    egui::pos2(end, rect.bottom()),
                ))
                .text(
                    egui::pos2(left, center.y),
                    egui::Align2::LEFT_CENTER,
                    label,
                    egui::TextStyle::Body.resolve(ui.style()),
                    ink,
                );
        }
        if response.has_focus() {
            painter.rect_stroke(
                rect,
                0,
                egui::Stroke::new(2.0, palette.focus),
                egui::StrokeKind::Inside,
            );
        }
    }
    response
}
/// A full-width row with vertically centered content and trailing controls.
pub fn settings_row(
    ui: &mut Ui,
    height: f32,
    trailing_width: f32,
    leading: impl FnOnce(&mut Ui),
    trailing: impl FnOnce(&mut Ui),
) -> egui::Rect {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let split = (rect.right() - trailing_width).max(rect.left());
    let cell = |index, bounds, layout| {
        let mut cell = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(index)
                .max_rect(bounds)
                .layout(layout),
        );
        cell.set_clip_rect(ui.clip_rect().intersect(bounds));
        cell
    };
    let mut cell = cell;
    leading(&mut cell(
        0,
        egui::Rect::from_min_max(rect.min, egui::pos2(split, rect.bottom())),
        egui::Layout::left_to_right(egui::Align::Center),
    ));
    trailing(&mut cell(
        1,
        egui::Rect::from_min_max(egui::pos2(split, rect.top()), rect.max),
        egui::Layout::right_to_left(egui::Align::Center),
    ));
    rect
}

/// A large drag target with a centered grip, painted using the bundled icon font.
#[cfg(feature = "icons")]
pub fn drag_grip(ui: &mut Ui, glyph: char, label: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(30.0, 32.0), egui::Sense::drag());
    if response.hovered() || response.dragged() {
        ui.painter().rect_filled(
            rect.shrink(2.0),
            super::control_radius(ui.ctx()),
            ui.visuals().widgets.hovered.bg_fill,
        );
    }
    crate::icons::paint_centered(
        ui.painter(),
        rect.center(),
        glyph,
        18.0,
        ui.visuals().weak_text_color(),
    );
    response
        .on_hover_and_drag_cursor(egui::CursorIcon::Grab)
        .on_hover_text(label)
}

/// A splitter with a wide hit area across the edge and a slim grip inside it.
/// Pass a cardinal unit vector pointing from the edge into the panel.
pub fn splitter(
    ui: &Ui,
    id: egui::Id,
    edge: egui::Rect,
    inward: egui::Vec2,
    label: &str,
) -> Response {
    let vertical = inward.x != 0.0;
    let thickness = 3.0;
    let length = 40.0;
    let hit = edge.expand2(if vertical {
        egui::vec2(8.0, 0.0)
    } else {
        egui::vec2(0.0, 8.0)
    });
    let response = ui
        .interact(hit, id, egui::Sense::click_and_drag())
        .on_hover_and_drag_cursor(if vertical {
            egui::CursorIcon::ResizeHorizontal
        } else {
            egui::CursorIcon::ResizeVertical
        });
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Splitter, ui.is_enabled(), label));
    let size = if vertical {
        egui::vec2(thickness, edge.height().min(length))
    } else {
        egui::vec2(edge.width().min(length), thickness)
    };
    let color = if response.hovered() || response.dragged() || response.has_focus() {
        ui.visuals().selection.stroke.color
    } else {
        ui.visuals().weak_text_color().gamma_multiply(0.48)
    };
    let center = edge.center() + inward * (2.0 + thickness / 2.0);
    ui.painter()
        .rect_filled(egui::Rect::from_center_size(center, size), 2, color);
    response
}

/// A settings switch with its label on the left and the control aligned to the right.
pub fn settings_toggle(ui: &mut Ui, value: &mut bool, label: &str) {
    settings_row(
        ui,
        32.0,
        48.0,
        |ui| {
            ui.add(egui::Label::new(label).truncate());
        },
        |ui| {
            crate::checkbox(ui, Surface::Dialog, crate::CheckStyle::Toggle, value, "")
                .accessible_name(label);
        },
    );
}

pub fn breadcrumbs(ui: &mut Ui, labels: &[&str]) -> Option<usize> {
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        for (index, &label) in labels.iter().enumerate() {
            if index > 0 {
                ui.weak("›");
            }
            if index + 1 == labels.len() {
                ui.strong(label);
            } else if button(
                ui,
                Surface::Panel,
                Action::Tertiary,
                true,
                Button::new(label),
            )
            .clicked()
            {
                clicked = Some(index);
            }
        }
    });
    clicked
}

pub fn pagination(
    ui: &mut Ui,
    page: &mut usize,
    pages: usize,
    previous: &str,
    next: &str,
) -> Response {
    let mut changed = false;
    if pages > 0 {
        *page = (*page).min(pages - 1);
    }
    let mut response = ui
        .horizontal_wrapped(|ui| {
            if arrow_button(ui, false, *page > 0 && pages > 0, previous).clicked() {
                *page -= 1;
                changed = true;
            }
            let start = page.saturating_sub(2).min(pages.saturating_sub(5));
            for index in start..start.saturating_add(pages.min(5)) {
                if button(
                    ui,
                    Surface::Panel,
                    if *page == index {
                        Action::Primary
                    } else {
                        Action::Outline
                    },
                    true,
                    Button::new((index + 1).to_string())
                        .min_size(egui::vec2(ui.spacing().interact_size.y, 0.0)),
                )
                .clicked()
                {
                    changed |= *page != index;
                    *page = index;
                }
            }
            if arrow_button(ui, true, page.saturating_add(1) < pages, next).clicked() {
                *page += 1;
                changed = true;
            }
        })
        .response;
    if changed {
        response.mark_changed();
    }
    response
}

fn arrow_button(ui: &mut Ui, forward: bool, enabled: bool, name: &str) -> Response {
    let atom = egui::Atom::paint(egui::Vec2::splat(14.0), move |ui, args| {
        let center = args.rect.center();
        let x = if forward { -2.0 } else { 2.0 };
        ui.painter().add(egui::Shape::line(
            vec![
                center + egui::vec2(x, -4.0),
                center + egui::vec2(-x, 0.0),
                center + egui::vec2(x, 4.0),
            ],
            egui::Stroke::new(1.6, args.fallback_text_color),
        ));
    });
    button(
        ui,
        Surface::Panel,
        Action::Outline,
        enabled,
        Button::new(atom),
    )
    .accessible_name(name)
    .on_hover_text(name)
}

pub fn stepper(ui: &mut Ui, tokens: UiTokens, labels: &[&str], active: &mut usize) -> Response {
    let mut changed = false;
    let mut response = ui
        .horizontal_wrapped(|ui| {
            for (i, &label) in labels.iter().enumerate() {
                if i > 0 {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(12.0, 2.0), egui::Sense::hover());
                    let color = if i <= *active {
                        tokens.theme.accent_primary()
                    } else {
                        tokens.theme.border_subtle().color
                    };
                    ui.painter().hline(
                        rect.x_range(),
                        rect.center().y,
                        egui::Stroke::new(1.0, color),
                    );
                }
                let marked = i < *active;
                let display = if marked {
                    label.to_owned()
                } else {
                    format!("{}  {label}", i + 1)
                };
                let response = button(
                    ui,
                    Surface::Panel,
                    if i == *active {
                        Action::Primary
                    } else if marked {
                        Action::Toggle(true)
                    } else {
                        Action::Outline
                    },
                    true,
                    Button::new(display),
                );
                if response.clicked() {
                    changed |= *active != i;
                    *active = i;
                }
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::Role::RadioButton,
                        response.enabled(),
                        *active == i,
                        label,
                    )
                });
            }
        })
        .response;
    if changed {
        response.mark_changed();
    }
    response
}
#[derive(Debug, Default)]
pub struct MultiSelect {
    pub query: String,
    pub selected: Vec<usize>,
}

impl MultiSelect {
    pub fn show(&mut self, ui: &mut Ui, label: &str, hint: &str, options: &[&str]) -> Response {
        let mut changed = false;
        let mut response = ui
            .vertical(|ui| {
                egui::ComboBox::from_id_salt("multi-select")
                    .width(ui.available_width())
                    .selected_text(label)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show_select(ui, Surface::Card, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.query).hint_text(hint));
                        let query = self.query.to_lowercase();
                        egui::ScrollArea::vertical()
                            .max_height(180.0)
                            .show(ui, |ui| {
                                for (i, &option) in options
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, option)| option.to_lowercase().contains(&query))
                                {
                                    let mut selected = self.selected.contains(&i);
                                    if ui.checkbox(&mut selected, option).changed() {
                                        changed = true;
                                        if selected {
                                            self.selected.push(i);
                                        } else {
                                            self.selected.retain(|index| *index != i);
                                        }
                                    }
                                }
                            });
                    });
                ui.horizontal_wrapped(|ui| {
                    self.selected.retain(|&i| {
                        let Some(label) = options.get(i) else {
                            return false;
                        };
                        let remove = ui.button(format!("{label} ×")).clicked();
                        changed |= remove;
                        !remove
                    });
                });
            })
            .response;
        if changed {
            response.mark_changed();
        }
        response
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalendarDate {
    year: i32,
    month: u8,
    day: u8,
}

impl CalendarDate {
    pub fn new(year: i32, month: u8, day: u8) -> Self {
        let year = year.clamp(1, 9999);
        let month = month.clamp(1, 12);
        Self {
            year,
            month,
            day: day.clamp(1, month_days(year, month)),
        }
    }

    pub fn parts(self) -> (i32, u8, u8) {
        (self.year, self.month, self.day)
    }

    pub fn picker(&mut self, ui: &mut Ui, days: [&str; 7], previous: &str, next: &str) -> Response {
        self.picker_with_months(ui, days, None, previous, next)
    }

    /// Calendar picker with host-localized weekday and month labels.
    pub fn picker_with_months(
        &mut self,
        ui: &mut Ui,
        days: [&str; 7],
        months: Option<[&str; 12]>,
        previous: &str,
        next: &str,
    ) -> Response {
        let tokens = UiTokens::resolve(ui.ctx());
        let theme = tokens.theme;
        let label = format!("{:04}-{:02}-{:02}", self.year, self.month, self.day);
        let id = ui.next_auto_id();
        let mut changed = false;
        let width = ui.available_width().clamp(140.0, 252.0);
        let mut response = ui.add_sized(
            [width, ui.spacing().interact_size.y],
            Button::new(label)
                .fill(theme.bg_elevated())
                .stroke(theme.border_strong)
                .corner_radius(theme.radius),
        );
        crate::paint_chevron(
            ui,
            response.rect.right_center() - egui::vec2(12.0, 0.0),
            0.0,
            theme.text_muted,
        );
        let popup_id = id.with("calendar-popup");
        let was_open = egui::Popup::is_id_open(ui.ctx(), popup_id);
        if !was_open && response.clicked() {
            ui.data_mut(|data| {
                data.remove::<(i32, u8)>(id);
                data.remove::<u8>(id.with("jump"));
            });
        }
        let escape = was_open && ui.input(|input| input.key_pressed(egui::Key::Escape));
        egui::Popup::menu(&response)
            .id(popup_id)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .frame(crate::components::surface_frame(tokens))
            .show(|ui| {
                ui.set_width(224.0);
                let (mut year, mut month) = ui
                    .ctx()
                    .data(|data| data.get_temp(id))
                    .unwrap_or((self.year, self.month));
                ui.visuals_mut().override_text_color = None;
                ui.visuals_mut().selection.bg_fill = theme.primary;
                ui.visuals_mut().selection.stroke = egui::Stroke::new(1.0, theme.on_primary);
                let side = (ui.available_width() - 6.0 * 4.0) / 7.0;
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                let jump_id = id.with("jump");
                // Days, months and years replace one another inside the same popup.
                let mut page = ui.data(|d| d.get_temp::<u8>(jump_id)).unwrap_or(0);
                let first_year = ((year - 1) / 12) * 12 + 1;
                let before = (year, month, page);
                let cell_h = side.max(ui.spacing().interact_size.y);
                let heading_height = ui.spacing().interact_size.y;
                ui.set_height(heading_height + 1.0 + 20.0 + 8.0 * 4.0 + 6.0 * cell_h);
                let heading = ui
                    .horizontal(|ui| {
                        let h = ui.spacing().interact_size.y;
                        let middle =
                            (ui.available_width() - 2.0 * h - 2.0 * ui.spacing().item_spacing.x)
                                .max(1.0);
                        let back = ui
                            .add_sized([h, h], Button::new("<").frame(false))
                            .accessible_name(previous)
                            .clicked();
                        let title = match page {
                            1 => year.to_string(),
                            2 => format!("{} – {}", first_year, (first_year + 11).min(9999)),
                            _ => months.map_or_else(
                                || format!("{year:04}-{month:02}"),
                                |names| format!("{} {year}", names[usize::from(month - 1)]),
                            ),
                        };
                        let heading = ui.add_sized(
                            [middle, h],
                            Button::new(egui::RichText::new(title).strong())
                                .frame(false)
                                .truncate(),
                        );
                        if !was_open {
                            heading.request_focus();
                        }
                        if heading.clicked() {
                            page = (page + 1) % 3;
                        }
                        let forward = ui
                            .add_sized([h, h], Button::new(">").frame(false))
                            .accessible_name(next)
                            .clicked();
                        if back || forward {
                            let delta = if forward { 1 } else { -1 };
                            if page == 0 {
                                (year, month) = adjacent_month(year, month, delta);
                            } else {
                                year =
                                    (year + delta * if page == 2 { 12 } else { 1 }).clamp(1, 9999);
                            }
                        }
                        heading
                    })
                    .inner;
                ui.add(egui::Separator::default().spacing(1.0));
                if page != 0 {
                    let year_page = page == 2;
                    let width = ((ui.available_width() - 8.0) / 3.0).max(1.0);
                    egui::Grid::new(id.with(("date-jump", page)))
                        .min_col_width(width)
                        .num_columns(3)
                        .show(ui, |ui| {
                            for n in 0..12_u8 {
                                let y = ((year - 1) / 12) * 12 + 1 + i32::from(n);
                                if year_page && y > 9999 {
                                    break;
                                }
                                let m = n + 1;
                                let active = if year_page { y == year } else { m == month };
                                let name = if year_page {
                                    y.to_string()
                                } else {
                                    months.map_or_else(
                                        || m.to_string(),
                                        |names| names[usize::from(n)].to_owned(),
                                    )
                                };
                                let ink = if active { theme.on_primary } else { theme.text };
                                let mut b = Button::new(egui::RichText::new(name).color(ink))
                                    .truncate()
                                    .frame(false);
                                if active {
                                    b = b.frame(true).selected(true).fill(theme.primary);
                                }
                                if ui.add_sized([width, cell_h], b).clicked() {
                                    heading.request_focus();
                                    if year_page {
                                        year = y;
                                        page = 1;
                                    } else {
                                        month = m;
                                        page = 0;
                                    }
                                }
                                if n % 3 == 2 {
                                    ui.end_row();
                                }
                            }
                        });
                    keep_page(ui, (id, jump_id), before, (year, month, page));
                    return;
                }
                keep_page(ui, (id, jump_id), before, (year, month, page));
                let offset = monday_offset(year, month);
                egui::Grid::new(id.with("calendar"))
                    .min_col_width(side)
                    .spacing(egui::vec2(4.0, 4.0))
                    .show(ui, |ui| {
                        for day in days {
                            ui.add_sized(
                                [side, 20.0],
                                egui::Label::new(egui::RichText::new(day).weak()),
                            );
                        }
                        ui.end_row();
                        for cell in 0u8..42 {
                            if let Some(day) = cell
                                .checked_sub(offset)
                                .map(|day| day + 1)
                                .filter(|day| *day <= month_days(year, month))
                            {
                                let date = Self { year, month, day };
                                if ui
                                    .add_sized(
                                        [side, cell_h],
                                        if *self == date {
                                            Button::new(
                                                egui::RichText::new(day.to_string())
                                                    .color(theme.on_primary),
                                            )
                                            .fill(theme.primary)
                                            .selected(true)
                                        } else {
                                            Button::new(day.to_string()).frame(false)
                                        },
                                    )
                                    .clicked()
                                {
                                    changed = *self != date;
                                    *self = date;
                                    response.request_focus();
                                    ui.close();
                                }
                            } else {
                                ui.allocate_space(egui::vec2(side, cell_h));
                            }
                            if cell % 7 == 6 {
                                ui.end_row();
                            }
                        }
                    });
            });
        if escape {
            response.request_focus();
        }
        if changed {
            response.mark_changed();
        }
        response
    }
}

/// Remembers the shown month and picker page when they changed this frame.
fn keep_page(
    ui: &Ui,
    (month_id, page_id): (egui::Id, egui::Id),
    before: (i32, u8, u8),
    (year, month, page): (i32, u8, u8),
) {
    if before != (year, month, page) {
        ui.data_mut(|data| {
            data.insert_temp(month_id, (year, month));
            data.insert_temp(page_id, page);
        });
    }
}

fn adjacent_month(year: i32, month: u8, delta: i32) -> (i32, u8) {
    let index = ((year - 1) * 12 + i32::from(month) - 1 + delta).clamp(0, 9999 * 12 - 1);
    (index / 12 + 1, u8::try_from(index % 12 + 1).unwrap_or(1))
}

fn month_days(year: i32, month: u8) -> u8 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn monday_offset(year: i32, month: u8) -> u8 {
    let preceding = year - 1;
    let days_in_preceding_years =
        preceding * 365 + preceding / 4 - preceding / 100 + preceding / 400;
    let days_in_year: i32 = (1..month)
        .map(|month| i32::from(month_days(year, month)))
        .sum();
    ((days_in_preceding_years + days_in_year) % 7) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_switches_pages_without_resizing_and_commits_only_a_day() {
        use orange_testkit::ui::{click, text_center};
        let ctx = egui::Context::default();
        let mut date = CalendarDate::new(2023, 12, 31);
        let mut frame = |events| {
            let mut launcher = None;
            let mut popup_id = None;
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    popup_id = Some(ui.next_auto_id().with("calendar-popup"));
                    let response = date.picker_with_months(
                        ui,
                        ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
                        Some([
                            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct",
                            "Nov", "Dec",
                        ]),
                        "Previous",
                        "Next",
                    );
                    launcher = Some(response.id);
                },
            );
            let shapes = output.shapes.clone();
            output.drop_without_applying_deltas();
            (
                shapes,
                date,
                ctx.memory(|memory| memory.has_focus(launcher.unwrap())),
                ctx.read_response(popup_id.unwrap())
                    .map(|response| response.rect),
            )
        };
        let press = |key| {
            [true, false]
                .into_iter()
                .map(|pressed| egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                })
                .collect()
        };
        type CalendarFrame = (
            Vec<egui::epaint::ClippedShape>,
            CalendarDate,
            bool,
            Option<egui::Rect>,
        );
        let click_label =
            |label: &str, frame: &mut dyn FnMut(Vec<egui::Event>) -> CalendarFrame| {
                let (shapes, _, _, _) = frame(Vec::new());
                let at = text_center(&shapes, label).expect(label);
                click(at, egui::PointerButton::Primary, |events| {
                    frame(events);
                });
            };
        click_label("2023-12-31", &mut frame);
        let (days, initial, _, bounds) = frame(Vec::new());
        let bounds = bounds.expect("popup bounds");
        assert!((text_center(&days, "Dec 2023").unwrap().x - bounds.center().x).abs() < 2.0);
        click_label(">", &mut frame);
        assert!(text_center(&frame(Vec::new()).0, "Jan 2024").is_some());
        click_label("<", &mut frame);
        click_label("Dec 2023", &mut frame);
        let (month_shapes, uncommitted, _, months) = frame(Vec::new());
        assert_eq!(initial, uncommitted);
        assert!((bounds.size() - months.unwrap().size()).length() < 0.1);
        let theme = UiTokens::resolve(&ctx).theme;
        assert!(month_shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "Dec"
                && text.galley.job.sections.iter().any(|section| section.format.color == theme.on_primary))
        }));
        click_label("2023", &mut frame);
        let (_, uncommitted, _, years) = frame(Vec::new());
        assert_eq!(initial, uncommitted);
        assert!((bounds.size() - years.unwrap().size()).length() < 0.1);
        click_label("2024", &mut frame);
        click_label("Feb", &mut frame);
        click_label("29", &mut frame);
        let (_, selected, focused, _) = frame(Vec::new());
        assert_eq!(selected.parts(), (2024, 2, 29));
        assert!(focused);
        frame(press(egui::Key::Enter));
        assert!(text_center(&frame(Vec::new()).0, "Feb 2024").is_some());
        frame(press(egui::Key::Escape));
        let (closed, _, focused, _) = frame(Vec::new());
        assert!(focused);
        assert!(text_center(&closed, "Feb 2024").is_none());
    }

    #[test]
    fn calendar_handles_leap_centuries_and_monday_based_grid() {
        assert_eq!(adjacent_month(2026, 12, 1), (2027, 1));
        assert_eq!(adjacent_month(2027, 1, -1), (2026, 12));
        assert_eq!(adjacent_month(1, 1, -1), (1, 1));
        assert_eq!(adjacent_month(9999, 12, 1), (9999, 12));
        assert_eq!(CalendarDate::new(2000, 2, 30).parts(), (2000, 2, 29));
        assert_eq!(CalendarDate::new(2100, 2, 30).parts(), (2100, 2, 28));
        assert_eq!(monday_offset(2026, 11), 6);
        assert_eq!(monday_offset(2026, 12), 1);
    }
}

/// Three cyclic time columns. Drag a column vertically, scroll over it, or
/// use its arrow keys. Each unit wraps independently.
pub fn clock_wheels(
    ui: &mut Ui,
    tokens: UiTokens,
    time: &mut [u8; 3],
    units: [&str; 3],
) -> Response {
    let mut changed = false;
    let mut response = ui
        .horizontal_wrapped(|ui| {
            for (index, (value, unit)) in time.iter_mut().zip(units).enumerate() {
                let cycle = if index == 0 { 24 } else { 60 };
                *value %= cycle;
                ui.push_id(index, |ui| {
                    ui.vertical(|ui| {
                        let theme = tokens.theme;
                        let row = tokens.density.row_height();
                        let width = (row * 1.55).clamp(46.0, ui.available_width().max(46.0));
                        let (rect, wheel) = ui.allocate_exact_size(
                            egui::vec2(width, row * 2.65),
                            egui::Sense::click_and_drag(),
                        );
                        if wheel.clicked() || wheel.drag_started() {
                            wheel.request_focus();
                        }
                        let drag_id = wheel.id.with("clock-drag");
                        let drag = if wheel.dragged() {
                            let motion = wheel.drag_delta().y;
                            ui.ctx().data_mut(|data| {
                                let prior = if wheel.drag_started() {
                                    0.0
                                } else {
                                    data.get_temp(drag_id).unwrap_or(0.0)
                                };
                                let (steps, remaining) = clock_drag(prior, motion, row * 0.85);
                                data.insert_temp(drag_id, remaining);
                                steps
                            })
                        } else {
                            if wheel.drag_stopped() {
                                ui.ctx().data_mut(|data| data.remove::<f32>(drag_id));
                            }
                            0
                        };
                        let painter = ui.painter_at(rect);
                        painter.rect(
                            rect,
                            6,
                            theme.bg_input(),
                            theme.control_border(),
                            egui::StrokeKind::Inside,
                        );
                        let hover = wheel.hovered();
                        if wheel.dragged() || hover {
                            ui.ctx().set_cursor_icon(if wheel.dragged() {
                                egui::CursorIcon::Grabbing
                            } else {
                                egui::CursorIcon::Grab
                            });
                        }
                        let scroll = if hover { crate::take_wheel(ui) } else { 0.0 };
                        let [up, down] = if wheel.has_focus() {
                            crate::consume_plain_keys(
                                ui,
                                [egui::Key::ArrowUp, egui::Key::ArrowDown],
                            )
                        } else {
                            [false; 2]
                        };
                        let click = wheel
                            .interact_pointer_pos()
                            .filter(|_| wheel.clicked())
                            .map_or(0, |p| {
                                if p.y < rect.center().y - row * 0.42 {
                                    -1
                                } else if p.y > rect.center().y + row * 0.42 {
                                    1
                                } else {
                                    0
                                }
                            });
                        let delta = drag + click + i16::from(down || scroll < 0.0)
                            - i16::from(up || scroll > 0.0);
                        if delta != 0 {
                            *value = wrap_time(*value, delta, cycle);
                            changed = true;
                        }
                        let selection = egui::Rect::from_center_size(
                            rect.center(),
                            egui::vec2(width - 8.0, row * 0.84),
                        );
                        painter.rect_filled(
                            selection,
                            4,
                            theme
                                .bg_elevated()
                                .lerp_to_gamma(theme.accent_primary(), 0.2),
                        );
                        for offset in [-1i16, 0, 1] {
                            painter.text(
                                rect.center() + egui::vec2(0.0, f32::from(offset) * row * 0.85),
                                egui::Align2::CENTER_CENTER,
                                format!("{:02}", wrap_time(*value, offset, cycle)),
                                egui::FontId::monospace(if offset == 0 {
                                    row * 0.57
                                } else {
                                    row * 0.42
                                }),
                                if offset == 0 {
                                    theme.text_primary()
                                } else {
                                    theme.text_muted
                                },
                            );
                        }
                        wheel.widget_info(|| {
                            egui::WidgetInfo::slider(wheel.enabled(), f64::from(*value), unit)
                        });
                        if wheel.has_focus() {
                            painter.rect_stroke(
                                rect.shrink(1.0),
                                6,
                                theme.focus_ring,
                                egui::StrokeKind::Inside,
                            );
                        }
                        ui.weak(unit);
                    });
                });
            }
        })
        .response;
    if changed {
        response.mark_changed();
    }
    response
}

fn clock_drag(prior: f32, delta: f32, pitch: f32) -> (i16, f32) {
    let distance = (prior - delta).clamp(-pitch * 120.0, pitch * 120.0);
    let steps = (distance / pitch).trunc() as i16;
    (steps, distance - f32::from(steps) * pitch)
}

fn wrap_time(value: u8, delta: i16, cycle: u8) -> u8 {
    u8::try_from((i16::from(value) + delta).rem_euclid(i16::from(cycle))).unwrap_or_default()
}

#[cfg(test)]
#[test]
fn time_wheels_wrap_all_endpoints() {
    assert_eq!(wrap_time(0, -1, 24), 23);
    assert_eq!(wrap_time(23, 1, 24), 0);
    assert_eq!(wrap_time(0, -1, 60), 59);
    assert_eq!(wrap_time(59, 1, 60), 0);
}

#[cfg(test)]
#[test]
fn clock_drag_accumulates_motion_and_wraps_in_both_directions() {
    let (steps, remainder) = clock_drag(0.0, -14.0, 20.0);
    assert_eq!((steps, remainder), (0, 14.0));
    let (steps, remainder) = clock_drag(remainder, -10.0, 20.0);
    assert_eq!((steps, remainder), (1, 4.0));
    assert_eq!(wrap_time(59, steps, 60), 0);
    assert_eq!(wrap_time(0, clock_drag(0.0, 24.0, 20.0).0, 24), 23);
}

#[cfg(test)]
#[test]
fn sidebar_rows_share_width_and_keep_icons_in_one_column() {
    let ctx = egui::Context::default();
    ctx.run_ui(Default::default(), |ui| {
        ui.set_width(175.0);
        let mut slots = Vec::new();
        for (label, expanded) in [("Gallery", Some(true)), ("Appearance", None)] {
            let row = sidebar_row(ui, label, expanded.is_none(), expanded, |_, r, _| {
                slots.push(r)
            });
            assert_eq!(row.rect.width(), 175.0);
        }
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].left(), slots[1].left());
        assert!(slots[0].bottom() < slots[1].top());
    })
    .drop_without_applying_deltas();
}
