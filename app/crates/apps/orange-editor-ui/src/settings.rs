//! Searchable preferences with sidebar navigation and independently scrollable pages.
use super::*;
use crate::canvas::{GRID_STEPS, GridStyle};
use crate::controls::{Action, Dropdown as _, Surface};
use orange_ui_controls::{CONTROL_HEIGHT, ControlSize, SPACE_LARGE, SPACE_MEDIUM, SPACE_SMALL};

const GRID_STYLES: [GridStyle; 3] = [GridStyle::Dots, GridStyle::Lines, GridStyle::Hidden];
const CHOICE_WIDTH: f32 = 280.0;

fn grid_style_label(style: GridStyle) -> &'static str {
    t(match style {
        GridStyle::Dots => "grid-dots",
        GridStyle::Lines => "grid-lines",
        GridStyle::Hidden => "grid-hidden",
    })
}

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Section {
    #[default]
    Search,
    Display,
    Shortcuts,
}

impl Section {
    const ALL: [Self; 3] = [Self::Search, Self::Display, Self::Shortcuts];

    fn title(self) -> &'static str {
        t(match self {
            Self::Search => "search-heading",
            Self::Display => "display-heading",
            Self::Shortcuts => "shortcuts-heading",
        })
    }

    fn matches(self, query: &str) -> bool {
        let labels: &[&str] = match self {
            Self::Search => &[
                "search-section-words",
                "settings-node-search",
                "preview-node-cards",
                "preview-names-only",
                "settings-language",
            ],
            Self::Display => &[
                "display-section-words",
                "interface-scale",
                "appearance",
                "appearance-system",
                "appearance-light",
                "appearance-dark",
                "show-left-panel",
                "show-right-panel",
                "wheel-pans",
                "snap-grid",
                "grid-type",
                "grid-spacing",
            ],
            Self::Shortcuts => &["shortcuts-section-words", "restore-default-shortcuts"],
        };
        self.title().to_lowercase().contains(query)
            || labels
                .iter()
                .any(|label| t(label).to_lowercase().contains(query))
    }
}

impl Editor {
    pub(crate) fn settings_window(&mut self, ctx: &egui::Context) -> Option<egui::Rect> {
        let mut open = self.settings_open;
        if !open {
            return None;
        }
        icons::install(ctx);
        let viewport =
            (ctx.content_rect().size() - egui::vec2(32.0, 56.0)).max(egui::vec2(240.0, 200.0));
        let min_size = egui::vec2(680.0, 380.0).min(viewport);
        let max_size = egui::vec2(1050.0, 720.0).min(viewport);
        let theme = ctx.style_of(ctx.theme());
        let title = egui::RichText::new(t("settings-title"))
            .strong()
            .color(theme.visuals.strong_text_color());
        let shown = egui::Window::new(title)
            .id(egui::Id::unique("orange-settings"))
            .frame(egui::Frame::window(&theme).inner_margin(egui::Margin::same(8)))
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_size(egui::vec2(840.0, 640.0).min(viewport))
            .min_size(min_size)
            .max_size(max_size)
            .collapsible(false)
            .resizable(true)
            .show(ctx, |ui| {
                let size = ui.available_size().clamp(min_size, max_size);
                let (content, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                self.settings_content(ui, ctx, content)
            });
        self.settings_open = open
            && !shown
                .as_ref()
                .is_some_and(|window| window.inner == Some(true));
        if !self.settings_open {
            self.shortcut_edit = None;
        }
        shown.map(|window| window.response.rect)
    }

    fn settings_content(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        content: egui::Rect,
    ) -> bool {
        let palette = orange_ui_controls::Theme::of(ui);
        let footer_height = CONTROL_HEIGHT + SPACE_MEDIUM * 2.0;
        let footer_y = content.bottom() - footer_height;
        let divider = egui::Stroke::new(1.0, palette.border);
        let compact = content.width() < 540.0;
        let nav_width = (content.width() * 0.23).clamp(152.0, 195.0);
        let strip_height = CONTROL_HEIGHT + SPACE_MEDIUM * 2.0;
        let nav_rect = egui::Rect::from_min_max(
            content.min,
            if compact {
                egui::pos2(content.right(), content.top() + strip_height)
            } else {
                egui::pos2(content.left() + nav_width, content.bottom())
            },
        );
        ui.painter().rect_filled(nav_rect, 6, palette.panel);
        let mut header = settings_pane(
            ui,
            "settings-search-sidebar",
            egui::Rect::from_min_max(
                nav_rect.min + egui::Vec2::splat(SPACE_MEDIUM),
                egui::pos2(
                    if compact {
                        nav_rect.center().x + 18.0
                    } else {
                        nav_rect.right() - SPACE_MEDIUM
                    },
                    nav_rect.top() + SPACE_MEDIUM + CONTROL_HEIGHT,
                ),
            ),
        );
        self.settings_search_ui(&mut header);

        let previous = self.settings_tab;
        let query = self.settings_search.trim().to_lowercase();
        let matches = Section::ALL.map(|section| {
            section.matches(&query)
                || section == Section::Shortcuts && self.shortcut_matches_any(&query)
        });
        if !Section::ALL
            .iter()
            .zip(matches)
            .any(|(&section, found)| section == self.settings_tab && found)
            && let Some(first) = matches.iter().position(|&found| found)
        {
            self.settings_tab = Section::ALL[first];
        }

        let navigation = if compact {
            ui.painter()
                .hline(content.x_range(), nav_rect.bottom(), divider);
            egui::Rect::from_min_max(
                egui::pos2(nav_rect.center().x + 24.0, nav_rect.top() + SPACE_MEDIUM),
                egui::pos2(
                    nav_rect.right() - SPACE_MEDIUM,
                    nav_rect.bottom() - SPACE_SMALL,
                ),
            )
        } else {
            ui.painter()
                .vline(nav_rect.right(), nav_rect.y_range(), divider);
            egui::Rect::from_min_max(
                nav_rect.min + egui::vec2(SPACE_MEDIUM, strip_height),
                egui::pos2(nav_rect.right() - SPACE_MEDIUM, footer_y - SPACE_SMALL),
            )
        };
        let mut nav = settings_pane(ui, "settings-navigation", navigation);
        self.settings_navigation(&mut nav, matches, compact);
        if previous != self.settings_tab || !matches.iter().any(|&found| found) {
            self.shortcut_edit = None;
        }

        let mut page = settings_pane(
            ui,
            "settings-page",
            egui::Rect::from_min_max(
                if compact {
                    egui::pos2(
                        content.left() + SPACE_MEDIUM,
                        nav_rect.bottom() + SPACE_MEDIUM,
                    )
                } else {
                    egui::pos2(nav_rect.right() + SPACE_LARGE, content.top() + SPACE_MEDIUM)
                },
                egui::pos2(
                    content.right() - SPACE_MEDIUM,
                    if compact { footer_y } else { content.bottom() } - SPACE_MEDIUM,
                ),
            ),
        );
        if matches.iter().any(|&found| found) {
            if self.settings_tab != Section::Shortcuts {
                page.label(
                    egui::RichText::new(self.settings_tab.title())
                        .size(16.0)
                        .strong(),
                );
                page.add_space(SPACE_MEDIUM);
            }
            match self.settings_tab {
                Section::Shortcuts => self.keybindings_ui(
                    &mut page,
                    if self.settings_tab.matches(&query) {
                        ""
                    } else {
                        &query
                    },
                ),
                Section::Search | Section::Display => {
                    egui::ScrollArea::vertical()
                        .id_salt("settings-fields")
                        .show(&mut page, |ui| {
                            if self.settings_tab == Section::Search {
                                settings_group(ui, t("settings-language"), |ui| {
                                    self.language_choice(ui);
                                    ui.weak(t("settings-language-help"));
                                });
                                ui.add_space(SPACE_LARGE);
                                let label = ui.strong(t("settings-node-search"));
                                ui.scope(|ui| {
                                    ui.set_max_width(ui.available_width().min(CHOICE_WIDTH));
                                    let _ = orange_ui_controls::segmented(
                                        ui,
                                        Surface::Dialog,
                                        &label,
                                        &mut self.node_search_previews,
                                        &[
                                            (false, t("preview-names-only")),
                                            (true, t("preview-node-cards")),
                                        ],
                                    )
                                    .on_hover_text(t("preview-node-cards-hint"));
                                });
                            } else {
                                self.display_settings(ui, ctx);
                            }
                        });
                }
            }
        } else {
            page.label(egui::RichText::new(t("settings-empty")).strong());
        }
        let left = content.left() + SPACE_MEDIUM;
        let right = if compact {
            content.right()
        } else {
            nav_rect.right()
        } - SPACE_MEDIUM;
        if compact {
            ui.painter().hline(left..=right, footer_y, divider);
        }
        let area = egui::Rect::from_min_max(
            egui::pos2(left, footer_y + SPACE_MEDIUM),
            egui::pos2(right, content.bottom() - SPACE_MEDIUM),
        );
        let mut actions = settings_pane(ui, "settings-footer", area);
        ControlSize::Standard.apply(actions.spacing_mut());
        controls::button(
            &mut actions,
            Surface::Panel,
            Action::Primary,
            true,
            egui::Button::new((egui::Atom::grow(), t("settings-done"), egui::Atom::grow()))
                .gap(0.0)
                .min_size(egui::vec2(88.0, CONTROL_HEIGHT)),
        )
        .clicked()
    }

    fn settings_search_ui(&mut self, header: &mut egui::Ui) {
        let search_id = header.make_persistent_id("field");
        if self.shortcut_edit.is_none()
            && header.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::F))
        {
            header.memory_mut(|memory| memory.request_focus(search_id));
        }
        let _ = controls::search(
            header,
            "field",
            &mut self.settings_search,
            t("settings-search-hint"),
        )
        .on_hover_text(t("settings-search-all"));
    }

    fn settings_navigation(&mut self, nav: &mut egui::Ui, matches: [bool; 3], compact: bool) {
        if !matches.iter().any(|&found| found) {
            return;
        }
        if compact {
            egui::ComboBox::from_id_salt("section")
                .width(nav.available_width())
                .selected_text(self.settings_tab.title())
                .show_select(nav, Surface::Dialog, |ui| {
                    for (section, found) in Section::ALL.into_iter().zip(matches) {
                        if found {
                            ui.selectable_value(&mut self.settings_tab, section, section.title());
                        }
                    }
                });
        } else {
            for (section, found) in Section::ALL.into_iter().zip(matches) {
                if found {
                    let selected = self.settings_tab == section;
                    let width = nav.available_width();
                    if controls::button(
                        nav,
                        Surface::Panel,
                        Action::Toggle(selected),
                        true,
                        egui::Button::new(())
                            .left_text(section.title())
                            .truncate()
                            .min_size(egui::vec2(width, CONTROL_HEIGHT)),
                    )
                    .clicked()
                    {
                        self.settings_tab = section;
                    }
                }
            }
        }
    }

    fn display_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.appearance_picker(ui, ctx);
        self.interface_scale_picker(ui, ctx);
        ui.add_space(SPACE_LARGE);
        settings_group(ui, t("settings-panels"), |ui| {
            orange_ui_controls::toggle(
                ui,
                Surface::Dialog,
                &mut self.panels.nodes,
                t("show-left-panel"),
            );
            orange_ui_controls::toggle(
                ui,
                Surface::Dialog,
                &mut self.panels.properties,
                t("show-right-panel"),
            );
        });
        ui.add_space(SPACE_LARGE);
        settings_group(ui, t("settings-canvas"), |ui| {
            setting_row(ui, t("grid-type"), |ui, label| {
                self.grid_type_choice(ui, label)
            });
            setting_row(ui, t("grid-spacing"), |ui, label| {
                let _ = self.grid_spacing_input(ui, label);
            });
            ui.add_space(SPACE_SMALL);
            orange_ui_controls::toggle(ui, Surface::Dialog, &mut self.canvas.snap, t("snap-grid"));
            orange_ui_controls::toggle(
                ui,
                Surface::Dialog,
                &mut self.canvas.scroll_pans,
                t("wheel-pans"),
            );
        });
    }

    fn appearance_picker(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        use egui::ThemePreference::{Dark, Light, System};
        setting_row(ui, t("appearance"), |ui, label| {
            let mut preference = ctx.options(|options| options.theme_preference);
            if orange_ui_controls::segmented(
                ui,
                Surface::Dialog,
                label,
                &mut preference,
                &[
                    (System, t("appearance-system")),
                    (Light, t("appearance-light")),
                    (Dark, t("appearance-dark")),
                ],
            )
            .changed()
            {
                self.preferred_theme = Some(preference);
                ctx.set_theme(preference);
            }
            ui.add_space(SPACE_SMALL);
            ui.horizontal_top(|ui| {
                let width = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
                for (theme, name) in [
                    (egui::Theme::Light, t("appearance-light")),
                    (egui::Theme::Dark, t("appearance-dark")),
                ] {
                    ui.vertical(|ui| {
                        paint_theme_sample(ui, &ctx.style_of(theme).visuals, width);
                        ui.small(name);
                    });
                }
            });
        });
    }

    fn interface_scale_picker(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        setting_row(ui, t("interface-scale"), |ui, label| {
            let mut scale = ctx.zoom_factor();
            egui::ComboBox::from_id_salt("interface-scale")
                .width(96.0)
                .selected_text(format!("{:.0}%", scale * 100.0))
                .show_select(ui, Surface::Dialog, |ui| {
                    for percent in [75_u16, 90, 100, 110, 125, 150, 175, 200] {
                        if ui
                            .selectable_value(
                                &mut scale,
                                f32::from(percent) / 100.0,
                                format!("{percent}%"),
                            )
                            .changed()
                        {
                            self.preferred_scale = Some(scale);
                            ctx.set_zoom_factor(scale);
                        }
                    }
                })
                .response
                .labelled_by(label.id);
        });
    }

    pub(crate) fn grid_menu(&mut self, ui: &mut egui::Ui) {
        ui.scope(|ui| {
            ui.set_max_width(ui.available_width().min(CHOICE_WIDTH));
            let kind = ui.label(t("grid-type"));
            self.grid_type_choice(ui, &kind);
            ui.add_space(SPACE_SMALL);
            self.grid_spacing_slider(ui);
        });
    }

    fn grid_type_choice(&mut self, ui: &mut egui::Ui, label: &egui::Response) {
        let options = GRID_STYLES.map(|style| (style, grid_style_label(style)));
        let _ = orange_ui_controls::segmented(
            ui,
            Surface::Dialog,
            label,
            &mut self.canvas.grid_style,
            &options,
        );
    }

    fn grid_spacing_slider(&mut self, ui: &mut egui::Ui) -> egui::Response {
        let label = ui
            .horizontal(|ui| {
                let label = ui.strong(t("grid-spacing"));
                ui.monospace(self.canvas.grid_step.to_string());
                label
            })
            .inner;
        self.grid_spacing_input(ui, &label)
    }

    fn grid_spacing_input(&mut self, ui: &mut egui::Ui, label: &egui::Response) -> egui::Response {
        let [small, next, .., large] = GRID_STEPS;
        orange_ui_controls::stepped_slider(
            ui,
            Surface::Dialog,
            &mut self.canvas.grid_step,
            small..=large,
            std::num::NonZeroU16::new(next - small).unwrap_or(std::num::NonZeroU16::MIN),
            orange_ui_controls::StepLabels::Values,
        )
        .labelled_by(label.id)
    }
}

fn settings_pane(ui: &mut egui::Ui, id: &str, rect: egui::Rect) -> egui::Ui {
    let mut pane = ui.new_child(egui::UiBuilder::new().id_salt(id).max_rect(rect));
    pane.set_clip_rect(pane.clip_rect().intersect(rect));
    pane
}

fn paint_theme_sample(ui: &mut egui::Ui, visuals: &egui::Visuals, width: f32) {
    use egui::{Rect, Stroke, StrokeKind, pos2, vec2};
    let (rect, _) = ui.allocate_exact_size(vec2(width.max(1.0), 53.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let border = visuals.window_stroke;
    painter.rect(rect, 5, visuals.window_fill, border, StrokeKind::Inside);
    let inner = rect.shrink(5.0);
    let left = inner.left() + inner.width() * 0.29;
    let sidebar = Rect::from_min_max(inner.min, pos2(left, inner.bottom()));
    painter.rect_filled(sidebar, 3, visuals.panel_fill);
    let nav = sidebar.shrink2(vec2(3.0, 7.0));
    let nav = Rect::from_min_size(nav.min, vec2(nav.width().max(0.0), 7.0));
    painter.rect_filled(nav, 2, visuals.selection.bg_fill);
    let content = Rect::from_min_max(pos2(left + 4.0, inner.top()), inner.max);
    let label = content.min + vec2(3.0, 4.0);
    painter.hline(
        label.x..=label.x + content.width() * 0.39,
        label.y,
        Stroke::new(2.0, visuals.text_color()),
    );
    let input = Rect::from_min_size(
        label + vec2(0.0, 6.0),
        vec2((content.width() - 6.0).max(0.0), 10.0),
    );
    painter.rect(
        input,
        2,
        visuals.extreme_bg_color,
        border,
        StrokeKind::Inside,
    );
    let button = Rect::from_min_size(label + vec2(0.0, 20.0), vec2(content.width() * 0.54, 10.0));
    painter.rect(
        button,
        2,
        visuals.selection.bg_fill,
        visuals.selection.stroke,
        StrokeKind::Inside,
    );
}

fn settings_group(ui: &mut egui::Ui, title: &str, contents: impl FnOnce(&mut egui::Ui)) {
    ui.label(egui::RichText::new(title).size(14.0).strong());
    ui.scope(contents);
}

fn setting_row(
    ui: &mut egui::Ui,
    title: &str,
    contents: impl FnOnce(&mut egui::Ui, &egui::Response),
) {
    const LABEL_WIDTH: f32 = 124.0;
    if ui.available_width() >= LABEL_WIDTH + SPACE_MEDIUM + CHOICE_WIDTH {
        ui.horizontal_top(|ui| {
            let label = ui
                .allocate_ui_with_layout(
                    egui::vec2(LABEL_WIDTH, CONTROL_HEIGHT),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_min_width(LABEL_WIDTH);
                        ui.add(egui::Label::new(title).truncate())
                    },
                )
                .inner;
            let width = ui.available_width().min(CHOICE_WIDTH);
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.set_max_width(width);
                contents(ui, &label);
            });
        });
    } else {
        let label = ui.label(title);
        let width = ui.available_width().min(CHOICE_WIDTH);
        ui.scope(|ui| {
            ui.set_max_width(width);
            contents(ui, &label);
        });
    }
}
