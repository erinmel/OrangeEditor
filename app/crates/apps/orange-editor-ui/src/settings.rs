//! Searchable preferences with sidebar navigation and independently scrollable pages.
use super::*;

#[derive(Debug)]
pub(super) struct Bars {
    pub(super) status: bool,
    pub(super) zoom: bool,
    pub(super) graph_state: bool,
    pub(super) cpu: bool,
    pub(super) network: bool,
}

impl Default for Bars {
    fn default() -> Self {
        Self {
            status: true,
            zoom: true,
            graph_state: true,
            cpu: false,
            network: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Corners {
    pub(super) docks: u8,
    pub(super) windows: Option<u8>,
    pub(super) menus: Option<u8>,
    pub(super) controls: Option<u8>,
}

impl Default for Corners {
    fn default() -> Self {
        Self {
            docks: 8,
            windows: None,
            menus: None,
            controls: None,
        }
    }
}

impl Corners {
    pub(super) fn apply(self, visuals: &mut egui::Visuals) {
        if let Some(radius) = self.windows {
            visuals.window_corner_radius = radius.into();
        }
        if let Some(radius) = self.menus {
            visuals.menu_corner_radius = radius.into();
        }
        let Some(radius) = self.controls else {
            return;
        };
        let widgets = &mut visuals.widgets;
        for state in [
            &mut widgets.noninteractive,
            &mut widgets.inactive,
            &mut widgets.hovered,
            &mut widgets.active,
            &mut widgets.open,
        ] {
            state.corner_radius = radius.into();
        }
    }
}

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
    Panels,
    Toolbar,
    Canvas,
    Shortcuts,
    Extensions,
    Plugins,
}

impl Section {
    const ALL: [Self; 8] = [
        Self::Search,
        Self::Display,
        Self::Panels,
        Self::Toolbar,
        Self::Canvas,
        Self::Shortcuts,
        Self::Extensions,
        Self::Plugins,
    ];

    fn title_key(self) -> &'static str {
        match self {
            Self::Search => "search-heading",
            Self::Display => "general-display",
            Self::Panels => "settings-panels",
            Self::Toolbar => "toolbar-visibility",
            Self::Canvas => "settings-canvas",
            Self::Shortcuts => "shortcuts-heading",
            Self::Extensions => "packages",
            Self::Plugins => "native-plugins",
        }
    }

    fn title(self) -> &'static str {
        t(self.title_key())
    }

    fn relevance(self, query: &orange_host::SearchQuery) -> Option<orange_host::SearchRank> {
        if query.is_empty() {
            return Some(orange_host::SearchRank::BEST);
        }
        let mut labels = vec![self.title_key()];
        labels.extend_from_slice(match self {
            Self::Search => &[
                "search-section-words",
                "settings-node-search",
                "preview-node-cards",
                "preview-names-only",
                "settings-language",
                "settings-language-help",
            ],
            Self::Display => &[
                "display-heading",
                "display-section-words",
                "interface-scale",
                "appearance",
                "appearance-system",
                "appearance-light",
                "appearance-dark",
                "corner-radius",
                "radius-docks",
                "radius-windows",
                "radius-menus",
                "radius-controls",
            ],
            Self::Panels => &[
                "dock-show-names",
                "show-left-panel",
                "show-right-panel",
                "status-visibility",
                "status-cpu",
                "status-network",
                "tree-appearance",
                "tree-guides",
                "tree-arrows",
                "tree-background",
            ],
            Self::Toolbar => &[
                "customize-toolbar",
                "toolbar-reset",
                "toolbar-flex-spacer",
                "toolbar-fixed-spacer",
                "toolbar-divider",
                "toolbar-add-spacing",
            ],
            Self::Canvas => &[
                "wheel-pans",
                "snap-grid",
                "grid-type",
                "grid-dots",
                "grid-lines",
                "grid-hidden",
                "grid-spacing",
                "wireless-opacity",
            ],
            Self::Plugins => &[
                "settings-extensions",
                "native-plugins-hint",
                "manage-native-plugins",
            ],
            Self::Shortcuts => &["shortcuts-section-words", "restore-default-shortcuts"],
            Self::Extensions => &[
                "packages",
                "import-package",
                "settings-extensions",
                "graph-bundles",
                "open-shared-graph",
                "share-with-packages",
            ],
        });
        i18n::message_relevance(query, &labels, &[])
    }
}

impl Editor {
    /// Opens the native plugin management page for hosts that supply it.
    pub fn open_native_plugin_settings(&mut self) {
        self.settings_open = true;
        self.settings_tab = Section::Plugins;
        self.settings_search.clear();
    }

    #[cfg(test)]
    pub(crate) fn settings_window(
        &mut self,
        ctx: &egui::Context,
        host: &mut Host,
    ) -> Option<egui::Rect> {
        self.settings_window_with_plugins(ctx, host, &mut |_| {})
    }

    pub(crate) fn settings_window_with_plugins(
        &mut self,
        ctx: &egui::Context,
        host: &mut Host,
        plugins: &mut dyn FnMut(&mut egui::Ui),
    ) -> Option<egui::Rect> {
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
        let shown = orange_ui_controls::dialog_window(ctx, title)
            .id(egui::Id::unique("orange-settings"))
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
                self.settings_content(ui, ctx, content, host, plugins)
            });
        self.settings_open = open
            && !shown
                .as_ref()
                .is_some_and(|window| window.inner == Some(true));
        if !self.settings_open {
            self.shortcut_edit = None;
            self.shortcut_reset_open = false;
        }
        shown.map(|window| window.response.rect)
    }

    fn settings_content(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        content: egui::Rect,
        host: &mut Host,
        plugins: &mut dyn FnMut(&mut egui::Ui),
    ) -> bool {
        let palette = orange_ui_controls::Theme::of(ui);
        let footer_height = CONTROL_HEIGHT + SPACE_MEDIUM * 2.0;
        let footer_y = content.bottom() - footer_height;
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
        ui.painter()
            .rect_filled(nav_rect, self.corners.docks, palette.panel);
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
        let changed = self.settings_search_ui(&mut header);

        let previous = self.settings_tab;
        let query = self.settings_search.trim().to_owned();
        let prepared = orange_host::SearchQuery::new(&query);
        let ranks = Section::ALL.map(|section| {
            if section == Section::Plugins && !self.native_plugins {
                return None;
            }
            let rank = section.relevance(&prepared);
            let page_rank = match section {
                Section::Shortcuts => self.shortcut_search_rank(&query),
                Section::Toolbar => self.toolbar_search_rank(&prepared),
                _ => None,
            };
            rank.into_iter().chain(page_rank).min()
        });
        let current = Section::ALL
            .iter()
            .zip(ranks)
            .find_map(|(&section, rank)| (section == self.settings_tab).then_some(rank).flatten());
        if let Some((best, rank)) = Section::ALL
            .iter()
            .copied()
            .zip(ranks)
            .filter_map(|(section, rank)| rank.map(|rank| (section, rank)))
            .min_by_key(|&(_, rank)| rank)
            && current.is_none_or(|current| changed && rank < current)
        {
            self.settings_tab = best;
        }
        let matches = ranks.map(|rank| rank.is_some());

        let navigation = if compact {
            egui::Rect::from_min_max(
                egui::pos2(nav_rect.center().x + 24.0, nav_rect.top() + SPACE_MEDIUM),
                egui::pos2(
                    nav_rect.right() - SPACE_MEDIUM,
                    nav_rect.bottom() - SPACE_SMALL,
                ),
            )
        } else {
            egui::Rect::from_min_max(
                nav_rect.min + egui::vec2(SPACE_MEDIUM, strip_height),
                egui::pos2(nav_rect.right() - SPACE_MEDIUM, footer_y - SPACE_SMALL),
            )
        };
        let mut nav = settings_pane(ui, "settings-navigation", navigation);
        self.settings_navigation(&mut nav, matches, compact, !query.is_empty());
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
        page.style_mut().spacing.scroll = super::dock::workspace_scroll();
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
                Section::Extensions => {
                    self.graph_bundle_menu(&mut page, host);
                    self.package_manager(&mut page, host);
                }
                Section::Plugins => {
                    egui::ScrollArea::vertical()
                        .id_salt("native-plugins")
                        .show(&mut page, plugins);
                }
                Section::Shortcuts => self.keybindings_ui(
                    &mut page,
                    if self.settings_tab.relevance(&prepared).is_some() {
                        ""
                    } else {
                        &query
                    },
                ),
                Section::Search
                | Section::Display
                | Section::Panels
                | Section::Toolbar
                | Section::Canvas => {
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
        let right = nav_rect.right() - SPACE_MEDIUM;
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
                .min_size(area.size()),
        )
        .clicked()
    }

    fn settings_search_ui(&mut self, header: &mut egui::Ui) -> bool {
        let search_id = header.make_persistent_id("field");
        if self.shortcut_edit.is_none()
            && header.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::F))
        {
            header.memory_mut(|memory| memory.request_focus(search_id));
        }
        controls::search(
            header,
            "field",
            &mut self.settings_search,
            t("settings-search-hint"),
        )
        .on_hover_text(t("settings-search-all"))
        .changed()
    }

    fn settings_navigation(
        &mut self,
        nav: &mut egui::Ui,
        matches: [bool; 8],
        compact: bool,
        searching: bool,
    ) {
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
            return;
        }
        let section_before_navigation = self.settings_tab;
        let found = |section| {
            Section::ALL
                .iter()
                .zip(matches)
                .any(|(&page, matched)| page == section && matched)
        };
        egui::ScrollArea::vertical()
            .id_salt("settings-tree")
            .show(nav, |ui| {
                if found(Section::Search) {
                    self.settings_tree_row(ui, Section::Search, icons::SEARCH);
                }
                let display = [
                    Section::Display,
                    Section::Panels,
                    Section::Toolbar,
                    Section::Canvas,
                ];
                for (title, pages) in [
                    ("display-heading", display.as_slice()),
                    (
                        "settings-extensions",
                        [Section::Extensions, Section::Plugins].as_slice(),
                    ),
                ] {
                    if !pages.iter().copied().any(found) {
                        continue;
                    }
                    let id = ui.make_persistent_id(title);
                    let stored = ui
                        .ctx()
                        .data(|data| data.get_temp::<bool>(id))
                        .unwrap_or_else(|| pages.contains(&self.settings_tab));
                    let mut expanded = stored
                        || searching
                        || (pages.contains(&self.settings_tab)
                            && self.settings_tab != self.last_settings_tab);
                    let glyph = if title == "display-heading" {
                        icons::DISPLAY
                    } else {
                        icons::PACKAGES
                    };
                    if orange_ui_controls::navigation::sidebar_row(
                        ui,
                        t(title),
                        pages.contains(&self.settings_tab),
                        Some(expanded),
                        |ui, rect, ink| {
                            icons::paint_centered(ui.painter(), rect.center(), glyph, 15.0, ink)
                        },
                    )
                    .clicked()
                        && !searching
                    {
                        expanded = !expanded;
                        ui.ctx().data_mut(|data| data.insert_temp(id, expanded));
                        if expanded && !pages.contains(&self.settings_tab) {
                            self.settings_tab = pages[0];
                        }
                    }
                    if expanded {
                        if !searching {
                            ui.ctx().data_mut(|data| data.insert_temp(id, true));
                        }
                        ui.horizontal(|ui| {
                            ui.add_space(9.0);
                            ui.vertical(|ui| {
                                for &section in pages {
                                    if found(section) {
                                        let icon = match section {
                                            Section::Display => icons::DISPLAY,
                                            Section::Panels => icons::PROPERTIES,
                                            Section::Toolbar => icons::SETTINGS,
                                            Section::Canvas => icons::GRID,
                                            Section::Extensions => icons::PACKAGES,
                                            _ => icons::FULL_ACCESS,
                                        };
                                        self.settings_tree_row(ui, section, icon);
                                    }
                                }
                            });
                        });
                    }
                    if title == "display-heading" && found(Section::Shortcuts) {
                        self.settings_tree_row(ui, Section::Shortcuts, icons::KEYBOARD);
                    }
                }
                if !display.into_iter().any(found) && found(Section::Shortcuts) {
                    self.settings_tree_row(ui, Section::Shortcuts, icons::KEYBOARD);
                }
            });
        if self.settings_tab == section_before_navigation {
            self.last_settings_tab = self.settings_tab;
        }
    }

    fn settings_tree_row(&mut self, ui: &mut egui::Ui, section: Section, glyph: char) {
        if orange_ui_controls::navigation::sidebar_row(
            ui,
            section.title(),
            self.settings_tab == section,
            None,
            |ui, rect, ink| icons::paint_centered(ui.painter(), rect.center(), glyph, 15.0, ink),
        )
        .clicked()
        {
            self.settings_tab = section;
        }
    }

    fn display_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.settings_tab == Section::Display {
            self.appearance_picker(ui, ctx);
            self.interface_scale_picker(ui, ctx);
            self.corner_settings(ui, ctx);
        }
        if self.settings_tab == Section::Panels {
            ui.add_space(SPACE_LARGE);
            settings_group(ui, t("settings-panels"), |ui| {
                orange_ui_controls::navigation::settings_toggle(
                    ui,
                    &mut self.dock.names,
                    t("dock-show-names"),
                );
                orange_ui_controls::navigation::settings_toggle(
                    ui,
                    &mut self.panels.nodes,
                    t("show-left-panel"),
                );
                orange_ui_controls::navigation::settings_toggle(
                    ui,
                    &mut self.panels.properties,
                    t("show-right-panel"),
                );
            });
            ui.add_space(SPACE_LARGE);
            settings_group(ui, t("tree-appearance"), |ui| {
                orange_ui_controls::navigation::settings_toggle(
                    ui,
                    &mut self.tree_style.guides,
                    t("tree-guides"),
                );
                orange_ui_controls::navigation::settings_toggle(
                    ui,
                    &mut self.tree_style.arrows,
                    t("tree-arrows"),
                );
                orange_ui_controls::navigation::settings_toggle(
                    ui,
                    &mut self.tree_style.branch_background,
                    t("tree-background"),
                );
            });
        }
        if self.settings_tab == Section::Toolbar {
            self.toolbar_options(ui, true);
        }
        if self.settings_tab == Section::Panels {
            settings_group(ui, t("status-visibility"), |ui| {
                orange_ui_controls::navigation::settings_toggle(
                    ui,
                    &mut self.bars.status,
                    t("status-show"),
                );
                orange_ui_controls::navigation::settings_toggle(
                    ui,
                    &mut self.bars.zoom,
                    t("zoom-controls"),
                );
                orange_ui_controls::navigation::settings_toggle(
                    ui,
                    &mut self.bars.graph_state,
                    t("graph-info"),
                );
                orange_ui_controls::navigation::settings_toggle(
                    ui,
                    &mut self.bars.cpu,
                    t("status-cpu"),
                );
                orange_ui_controls::navigation::settings_toggle(
                    ui,
                    &mut self.bars.network,
                    t("status-network"),
                );
            });
        }
        if self.settings_tab == Section::Canvas {
            ui.add_space(SPACE_LARGE);
            settings_group(ui, t("settings-canvas"), |ui| {
                setting_row(ui, t("grid-type"), |ui, label| {
                    self.grid_type_choice(ui, label)
                });
                setting_row(ui, t("grid-spacing"), |ui, label| {
                    let _ = self.grid_spacing_input(ui, label);
                });
                setting_row(ui, t("wireless-opacity"), |ui, label| {
                    ui.set_max_width(CHOICE_WIDTH);
                    let mut percent = f32::from(self.canvas.wireless_opacity);
                    let response = orange_ui_controls::guided_slider(
                        ui,
                        Surface::Dialog,
                        &mut percent,
                        0.0..=100.0,
                        orange_ui_controls::SliderBehavior::Step(1.0),
                        orange_ui_controls::SliderMarks::NotchesAndValues,
                        &[
                            orange_ui_controls::SliderGuide {
                                fraction: 0.0,
                                label: "0%",
                            },
                            orange_ui_controls::SliderGuide {
                                fraction: 0.1,
                                label: "10%",
                            },
                            orange_ui_controls::SliderGuide {
                                fraction: 0.5,
                                label: "50%",
                            },
                            orange_ui_controls::SliderGuide {
                                fraction: 1.0,
                                label: "100%",
                            },
                        ],
                    )
                    .labelled_by(label.id)
                    .on_hover_text(t("wireless-opacity-help"));
                    if response.changed() {
                        #[expect(
                            clippy::cast_possible_truncation,
                            clippy::cast_sign_loss,
                            reason = "a percentage"
                        )]
                        let percent = percent.round().clamp(0.0, 100.0) as u8;
                        self.canvas.wireless_opacity = percent;
                    }
                });
                ui.add_space(SPACE_SMALL);
                orange_ui_controls::navigation::settings_toggle(
                    ui,
                    &mut self.canvas.snap,
                    t("snap-grid"),
                );
                orange_ui_controls::navigation::settings_toggle(
                    ui,
                    &mut self.canvas.scroll_pans,
                    t("wheel-pans"),
                );
            });
        }
    }

    fn corner_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let visuals = &ctx.style_of(ctx.theme()).visuals;
        settings_group(ui, t("corner-radius"), |ui| {
            ui.add(egui::Slider::new(&mut self.corners.docks, 0..=16).text(t("radius-docks")));
            for (radius, default, label) in [
                (
                    &mut self.corners.windows,
                    visuals.window_corner_radius.nw.min(16),
                    "radius-windows",
                ),
                (
                    &mut self.corners.menus,
                    visuals.menu_corner_radius.nw.min(16),
                    "radius-menus",
                ),
                (
                    &mut self.corners.controls,
                    orange_ui_controls::CONTROL_RADIUS,
                    "radius-controls",
                ),
            ] {
                let mut value = radius.unwrap_or(default);
                if ui
                    .add(egui::Slider::new(&mut value, 0..=16).text(t(label)))
                    .changed()
                {
                    *radius = Some(value);
                }
            }
        });
    }

    fn appearance_picker(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        use egui::ThemePreference::{Dark, Light, System};
        setting_row(ui, t("appearance"), |ui, label| {
            let preference = ctx.options(|options| options.theme_preference);
            let light = ctx.style_of(egui::Theme::Light);
            let dark = ctx.style_of(egui::Theme::Dark);
            ui.horizontal_top(|ui| {
                let width =
                    ((ui.available_width() - 2.0 * ui.spacing().item_spacing.x) / 3.0).max(1.0);
                for (choice, name) in [
                    (System, t("appearance-system")),
                    (Light, t("appearance-light")),
                    (Dark, t("appearance-dark")),
                ] {
                    let response = controls::button(
                        ui,
                        Surface::Dialog,
                        Action::Toggle(preference == choice),
                        true,
                        egui::Button::new(()).min_size(egui::vec2(width, 96.0)),
                    )
                    .labelled_by(label.id);
                    if !ui.is_rect_visible(response.rect) {
                        continue;
                    }
                    let sample = egui::Rect::from_min_max(
                        response.rect.min + egui::vec2(5.0, 5.0),
                        response.rect.max - egui::vec2(5.0, 30.0),
                    );
                    let painter = ui.painter_at(response.rect);
                    let light = orange_ui_controls::Theme::from_visuals(&light.visuals);
                    let dark = orange_ui_controls::Theme::from_visuals(&dark.visuals);
                    let (first, split) = match choice {
                        Light => (light, None),
                        Dark => (dark, None),
                        System => (light, Some(dark)),
                    };
                    orange_ui_controls::paint_theme_preview(&painter, sample, first, split);
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::Role::RadioButton,
                            ui.is_enabled(),
                            preference == choice,
                            name,
                        )
                    });
                    painter.text(
                        response.rect.center_bottom() - egui::vec2(0.0, 11.0),
                        egui::Align2::CENTER_CENTER,
                        name,
                        egui::TextStyle::Body.resolve(ui.style()),
                        ui.visuals().text_color(),
                    );
                    if response.clicked() {
                        self.preferred_theme = Some(choice);
                        ctx.set_theme(choice);
                    }
                }
            });
        });
    }

    fn interface_scale_picker(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        setting_row(ui, t("interface-scale"), |ui, label| {
            let mut scale = ctx.zoom_factor();
            let options = [
                (0.75, "75%"),
                (0.90, "90%"),
                (1.00, "100%"),
                (1.10, "110%"),
                (1.25, "125%"),
                (1.50, "150%"),
                (1.75, "175%"),
                (2.00, "200%"),
            ];
            let selected_label = options
                .iter()
                .find(|(value, _)| (*value - scale).abs() < f32::EPSILON)
                .map_or_else(
                    || format!("{:.0}%", scale * 100.0),
                    |(_, label)| (*label).to_owned(),
                );
            let response = orange_ui_controls::select(
                ui,
                Surface::Dialog,
                ui.make_persistent_id("interface-scale"),
                &mut scale,
                &selected_label,
                &options,
            )
            .labelled_by(label.id);
            if response.changed() {
                self.preferred_scale = Some(scale);
                ctx.set_zoom_factor(scale);
            }
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
