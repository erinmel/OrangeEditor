//! Searchable preferences with a fixed search header and independently scrollable pages.
use super::*;
use crate::canvas::{GRID_STEPS, GridStyle};

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

    fn icon(self) -> char {
        match self {
            Self::Search => icons::SETTINGS,
            Self::Display => icons::DISPLAY,
            Self::Shortcuts => icons::KEYBOARD,
        }
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
            (ctx.content_rect().size() - egui::vec2(24.0, 36.0)).max(egui::vec2(250.0, 220.0));
        let min_size = egui::vec2(680.0, 380.0).min(viewport);
        let max_size = egui::vec2(1050.0, 720.0).min(viewport);
        let theme = ctx.style_of(ctx.theme());
        let title = egui::RichText::new(t("settings-title"))
            .strong()
            .color(theme.visuals.strong_text_color());
        let shown = egui::Window::new(title)
            .id(egui::Id::unique("orange-settings"))
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_size(egui::vec2(840.0, 540.0).min(viewport))
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
        let header_bottom = content.top() + 46.0;
        let footer_y = content.bottom() - 44.0;
        let mut header = settings_pane(
            ui,
            "settings-search-header",
            egui::Rect::from_min_max(
                content.min + egui::vec2(8.0, 6.0),
                egui::pos2(content.right() - 8.0, header_bottom),
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

        let divider = ui.visuals().widgets.noninteractive.bg_stroke;
        ui.painter()
            .hline(content.x_range(), header_bottom, divider);
        let compact = content.width() < 580.0;
        let nav_width = (content.width() * 0.25).clamp(170.0, 224.0);
        let nav_rect = if compact {
            egui::Rect::from_min_max(
                egui::pos2(content.left() + 8.0, header_bottom + 8.0),
                egui::pos2(content.right() - 8.0, header_bottom + 42.0),
            )
        } else {
            egui::Rect::from_min_max(
                egui::pos2(content.left(), header_bottom),
                egui::pos2(content.left() + nav_width, footer_y),
            )
        };
        if !compact {
            ui.painter()
                .rect_filled(nav_rect, 0.0, ui.visuals().faint_bg_color);
            ui.painter()
                .vline(nav_rect.right(), nav_rect.y_range(), divider);
        }
        let mut nav = settings_pane(
            ui,
            "settings-navigation",
            if compact {
                nav_rect
            } else {
                nav_rect.shrink(8.0)
            },
        );
        self.settings_navigation(&mut nav, matches, compact);
        if previous != self.settings_tab || !matches.iter().any(|&found| found) {
            self.shortcut_edit = None;
        }

        let mut page = settings_pane(
            ui,
            "settings-page",
            egui::Rect::from_min_max(
                if compact {
                    egui::pos2(content.left() + 12.0, nav_rect.bottom() + 7.0)
                } else {
                    egui::pos2(nav_rect.right() + 18.0, header_bottom + 14.0)
                },
                egui::pos2(content.right() - 12.0, footer_y - 8.0),
            ),
        );
        if matches.iter().any(|&found| found) {
            page.label(
                egui::RichText::new(self.settings_tab.title())
                    .size(23.0)
                    .strong(),
            );
            page.separator();
            page.add_space(8.0);
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
                                settings_group(ui, t("settings-node-search"), |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.selectable_value(
                                            &mut self.node_search_previews,
                                            false,
                                            t("preview-names-only"),
                                        );
                                        ui.selectable_value(
                                            &mut self.node_search_previews,
                                            true,
                                            t("preview-node-cards"),
                                        )
                                        .on_hover_text(t("preview-node-cards-hint"));
                                    });
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
        ui.painter().hline(content.x_range(), footer_y, divider);
        let done = egui::Rect::from_min_size(
            egui::pos2(content.right() - 110.0, footer_y + 7.0),
            egui::vec2(100.0, 30.0),
        );
        ui.put(
            done,
            egui::Button::new(egui::RichText::new(t("settings-done")).strong())
                .fill(ui.visuals().selection.bg_fill),
        )
        .clicked()
    }

    fn settings_search_ui(&mut self, header: &mut egui::Ui) {
        let search_id = header.scope_id().with("field");
        if self.shortcut_edit.is_none()
            && header.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::F))
        {
            header.memory_mut(|memory| memory.request_focus(search_id));
        }
        header.horizontal(|ui| {
            let clear = t("clear-settings-search");
            let clear_width = ui.fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap(
                        clear.to_owned(),
                        egui::TextStyle::Button.resolve(ui.style()),
                        ui.visuals().text_color(),
                    )
                    .size()
                    .x
            }) + 2.0 * ui.spacing().button_padding.x
                + ui.spacing().item_spacing.x;
            let has_query = !self.settings_search.is_empty();
            let width = ui.available_width() - if has_query { clear_width } else { 0.0 };
            ui.add_sized(
                [width.max(60.0), 30.0],
                egui::TextEdit::singleline(&mut self.settings_search)
                    .id(search_id)
                    .hint_text(t("settings-search-all")),
            );
            if has_query && ui.button(clear).clicked() {
                self.settings_search.clear();
                ui.memory_mut(|memory| memory.request_focus(search_id));
            }
        });
    }

    fn settings_navigation(&mut self, nav: &mut egui::Ui, matches: [bool; 3], compact: bool) {
        if !matches.iter().any(|&found| found) {
            return;
        }
        if compact {
            egui::ComboBox::from_id_salt("section")
                .width(nav.available_width())
                .selected_text(self.settings_tab.title())
                .show_ui(nav, |ui| {
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
                    let button = egui::Button::selectable(
                        selected,
                        (icons::text(section.icon()), section.title()),
                    )
                    .truncate();
                    if nav
                        .add_sized([nav.available_width(), 31.0], button)
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
            self.interface_scale_picker(ui, ctx);
        });
        ui.add_space(8.0);
        settings_group(ui, t("settings-panels"), |ui| {
            ui.checkbox(&mut self.panels.nodes, t("show-left-panel"));
            ui.checkbox(&mut self.panels.properties, t("show-right-panel"));
        });
        ui.add_space(8.0);
        settings_group(ui, t("settings-canvas"), |ui| {
            ui.checkbox(&mut self.canvas.scroll_pans, t("wheel-pans"));
            ui.checkbox(&mut self.canvas.snap, t("snap-grid"));
            self.grid_menu(ui);
        });
    }

    fn interface_scale_picker(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let label = ui.label(t("interface-scale"));
        let mut scale = ctx.zoom_factor();
        egui::ComboBox::from_id_salt("interface-scale")
            .width(90.0)
            .selected_text(format!("{:.0}%", scale * 100.0))
            .show_ui(ui, |ui| {
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
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (index, style) in GRID_STYLES.into_iter().enumerate() {
                let label = grid_style_label(style);
                let selected = self.canvas.grid_style == style;
                let outside = if index == 0 { 5 } else { 0 };
                let far = if index == GRID_STYLES.len() - 1 { 5 } else { 0 };
                let segment = egui::Button::new(label)
                    .frame_when_inactive(true)
                    .selected(selected)
                    .min_size(egui::vec2(62.0, 26.0))
                    .corner_radius(egui::CornerRadius {
                        nw: outside,
                        sw: outside,
                        ne: far,
                        se: far,
                    });
                let response = ui.add(segment).labelled_by(kind.id);
                response.widget_info(|| {
                    egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, label)
                });
                if response.clicked() {
                    self.canvas.grid_style = style;
                }
            }
        });
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
        ui.spacing_mut().slider_width = 134.0;
        ui.add(
            egui::Slider::new(&mut self.canvas.grid_step, small..=large)
                .step_by(f64::from(next - small))
                .show_value(false),
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
