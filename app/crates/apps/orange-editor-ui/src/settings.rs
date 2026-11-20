//! Searchable preferences with sidebar navigation and independently scrollable pages.
use super::*;
use crate::canvas::{GRID_STEPS, GridStyle};
use crate::controls::{Action, Dropdown as _, Surface};
use orange_ui_controls::{CONTROL_HEIGHT, SPACE_LARGE, SPACE_MEDIUM, SPACE_SMALL};

const GRID_STYLES: [GridStyle; 3] = [GridStyle::Dots, GridStyle::Lines, GridStyle::Hidden];

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
        let footer_height = CONTROL_HEIGHT + SPACE_SMALL * 2.0;
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
                egui::pos2(content.left() + nav_width, footer_y)
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
                nav_rect.max - egui::vec2(SPACE_MEDIUM, SPACE_SMALL),
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
                egui::pos2(content.right() - SPACE_MEDIUM, footer_y - SPACE_MEDIUM),
            ),
        );
        if matches.iter().any(|&found| found) {
            if self.settings_tab != Section::Shortcuts {
                page.strong(self.settings_tab.title());
                page.separator();
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
                                ui.add_space(12.0);
                                let label = ui.strong(t("settings-node-search"));
                                ui.separator();
                                ui.indent(label.id, |ui| {
                                    ui.set_max_width(ui.available_width().min(252.0));
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
        let footer = egui::Rect::from_min_max(egui::pos2(content.left(), footer_y), content.max);
        ui.painter().rect_filled(footer, 6, palette.panel);
        let done = egui::Rect::from_min_size(
            egui::pos2(content.right() - 94.0, footer_y + SPACE_SMALL),
            egui::vec2(86.0, CONTROL_HEIGHT),
        );
        controls::button(
            &mut settings_pane(ui, "settings-footer", done),
            Surface::Dialog,
            Action::Primary,
            true,
            egui::Button::new(egui::RichText::new(t("settings-done")).strong())
                .min_size(done.size()),
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
        settings_group(ui, t("settings-interface"), |ui| {
            self.appearance_picker(ui, ctx);
            ui.add_space(SPACE_MEDIUM);
            self.interface_scale_picker(ui, ctx);
        });
        ui.add_space(8.0);
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
        ui.add_space(8.0);
        settings_group(ui, t("settings-canvas"), |ui| {
            orange_ui_controls::toggle(
                ui,
                Surface::Dialog,
                &mut self.canvas.scroll_pans,
                t("wheel-pans"),
            );
            orange_ui_controls::toggle(ui, Surface::Dialog, &mut self.canvas.snap, t("snap-grid"));
            self.grid_menu(ui);
        });
    }

    fn appearance_picker(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        use egui::ThemePreference::{Dark, Light, System};
        ui.scope(|ui| {
            ui.set_max_width(ui.available_width().min(300.0));
            let label = ui.label(t("appearance"));
            let mut preference = ctx.options(|options| options.theme_preference);
            if orange_ui_controls::segmented(
                ui,
                Surface::Dialog,
                &label,
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
        });
    }

    fn interface_scale_picker(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let label = ui.label(t("interface-scale"));
        let mut scale = ctx.zoom_factor();
        egui::ComboBox::from_id_salt("interface-scale")
            .width(90.0)
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
    }

    pub(crate) fn grid_menu(&mut self, ui: &mut egui::Ui) {
        let kind = ui.strong(t("grid-type"));
        let options = GRID_STYLES.map(|style| (style, grid_style_label(style)));
        let _ = orange_ui_controls::segmented(
            ui,
            Surface::Dialog,
            &kind,
            &mut self.canvas.grid_style,
            &options,
        );
        ui.add_space(4.0);
        self.grid_spacing_slider(ui);
    }

    fn grid_spacing_slider(&mut self, ui: &mut egui::Ui) -> egui::Response {
        let [small, next, .., large] = GRID_STEPS;
        let label = ui
            .horizontal(|ui| {
                let label = ui.strong(t("grid-spacing"));
                ui.monospace(self.canvas.grid_step.to_string());
                label
            })
            .inner;
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

fn settings_group(ui: &mut egui::Ui, title: &str, contents: impl FnOnce(&mut egui::Ui)) {
    ui.strong(title);
    ui.separator();
    ui.indent(title, contents);
}
