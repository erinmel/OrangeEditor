//! Workspace tool placement, independent of the graph document.
use super::*;
use serde_json::{Value as Json, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tool {
    Nodes,
    Properties,
    Variables,
    Types,
    Connections,
    GraphInfo,
    Problems,
    Log,
}

impl Tool {
    const ALL: [Self; 8] = [
        Self::Nodes,
        Self::Properties,
        Self::Variables,
        Self::Types,
        Self::Connections,
        Self::GraphInfo,
        Self::Problems,
        Self::Log,
    ];

    fn key(self) -> &'static str {
        match self {
            Self::Nodes => "nodes",
            Self::Properties => "properties",
            Self::Variables => "variables",
            Self::Types => "types",
            Self::Connections => "connections",
            Self::GraphInfo => "graph-info",
            Self::Problems => "problems",
            Self::Log => "log",
        }
    }

    fn icon(self) -> char {
        match self {
            Self::Nodes => icons::ARRANGE,
            Self::Properties => icons::PROPERTIES,
            Self::Variables => icons::VARIABLE,
            Self::Types => icons::TYPES,
            Self::Connections => icons::CONNECTIONS,
            Self::GraphInfo => icons::INFO,
            Self::Problems => icons::ERROR,
            Self::Log => icons::RUN_LOG,
        }
    }

    const CATALOGUE: [Self; 4] = [Self::Nodes, Self::Variables, Self::Types, Self::Connections];

    fn tab(self) -> Option<Tab> {
        match self {
            Self::Nodes => Some(Tab::Nodes),
            Self::Variables => Some(Tab::Variables),
            Self::Types => Some(Tab::Types),
            Self::Connections => Some(Tab::Connections),
            _ => None,
        }
    }

    fn read(value: &Json) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|tool| value.as_str() == Some(tool.key()))
    }
}

const AREA_LABELS: [&str; 4] = [
    "dock-left",
    "dock-right",
    "dock-bottom-left",
    "dock-bottom-right",
];
const RAIL: f32 = 30.0;

pub(super) fn workspace_scroll() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle {
        bar_width: 8.0,
        floating_width: 4.0,
        floating_allocated_width: 13.0,
        bar_inner_margin: 3.0,
        bar_outer_margin: 2.0,
        dormant_handle_opacity: 0.42,
        active_handle_opacity: 0.65,
        interact_handle_opacity: 0.9,
        dormant_background_opacity: 0.0,
        active_background_opacity: 0.0,
        interact_background_opacity: 0.0,
        ..egui::style::ScrollStyle::floating()
    }
}

#[derive(Clone, Debug)]
pub(super) struct Dock {
    tools: [Vec<Tool>; 4],
    active: [Option<Tool>; 4],
    sizes: [f32; 3],
    hidden: Vec<Tool>,
    pub(super) names: bool,
    tab: Tab,
}

impl Default for Dock {
    fn default() -> Self {
        Self {
            tools: [
                Tool::CATALOGUE.to_vec(),
                vec![Tool::Properties],
                vec![Tool::GraphInfo],
                vec![Tool::Problems, Tool::Log],
            ],
            active: [Some(Tool::Nodes), Some(Tool::Properties), None, None],
            sizes: [280.0, 280.0, 170.0],
            hidden: Vec::new(),
            names: false,
            tab: Tab::Nodes,
        }
    }
}

impl Dock {
    fn area(&self, tool: Tool) -> Option<usize> {
        self.tools.iter().position(|tools| tools.contains(&tool))
    }

    fn visible(&self, tool: Tool) -> bool {
        self.active.contains(&Some(tool))
    }

    fn show(&mut self, tool: Tool, visible: bool) {
        if visible && self.hidden.contains(&tool) {
            return;
        }
        let Some(area) = self.area(tool) else {
            return;
        };
        if visible || self.active[area] == Some(tool) {
            self.active[area] = visible.then_some(tool);
        }
    }

    fn hide(&mut self, tool: Tool) {
        self.show(tool, false);
        if !self.hidden.contains(&tool) {
            self.hidden.push(tool);
        }
    }

    fn move_tool(&mut self, tool: Tool, destination: usize, before: Option<Tool>) {
        if before == Some(tool) {
            return;
        }
        let Some(source) = self.area(tool) else {
            return;
        };
        self.show(tool, false);
        self.tools[source].retain(|item| *item != tool);
        let tools = &mut self.tools[destination];
        let index = before
            .and_then(|target| tools.iter().position(|item| *item == target))
            .unwrap_or(tools.len());
        tools.insert(index, tool);
        self.hidden.retain(|hidden| *hidden != tool);
        self.active[destination] = Some(tool);
    }

    pub(super) fn preferences(&self) -> Json {
        let tools = self
            .tools
            .each_ref()
            .map(|tools| tools.iter().map(|tool| tool.key()).collect::<Vec<_>>());
        json!({"tools": tools, "active": self.active.map(|tool| tool.map(Tool::key)), "sizes": self.sizes, "names": self.names, "hidden": self.hidden.iter().map(|tool| tool.key()).collect::<Vec<_>>() })
    }

    pub(super) fn restore(&mut self, value: &Json) {
        let Some(groups) = value["tools"].as_array().filter(|groups| groups.len() == 4) else {
            return;
        };
        let defaults = Self::default();
        let mut seen = Vec::new();
        self.tools = std::array::from_fn(|area| {
            groups[area]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Tool::read)
                .filter(|tool| {
                    if seen.contains(tool) {
                        return false;
                    }
                    seen.push(*tool);
                    true
                })
                .collect()
        });
        for (area, tools) in defaults.tools.iter().enumerate() {
            self.tools[area].extend(tools.iter().copied().filter(|tool| !seen.contains(tool)));
        }
        self.names = value["names"].as_bool().unwrap_or(false);
        self.hidden = value["hidden"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Tool::read)
            .collect();
        self.active = std::array::from_fn(|area| {
            Tool::read(&value["active"][area])
                .filter(|tool| self.tools[area].contains(tool) && !self.hidden.contains(tool))
        });
        for (index, size) in self.sizes.iter_mut().enumerate() {
            *size = value["sizes"][index]
                .as_f64()
                .filter(|size| (70.0..=1600.0).contains(size))
                .map_or(defaults.sizes[index], |size| size as f32);
        }
    }
}

impl Editor {
    fn sync_visibility(dock: &mut Dock, panels: Panels, log_events: bool, tab: Tab) {
        if !panels.nodes {
            for tool in Tool::CATALOGUE {
                dock.show(tool, false);
            }
        } else if dock.tab != tab || !Tool::CATALOGUE.into_iter().any(|tool| dock.visible(tool)) {
            let visible_icons = Tool::CATALOGUE
                .into_iter()
                .filter(|tool| !dock.hidden.contains(tool));
            let tool = visible_icons
                .clone()
                .find(|tool| tool.tab() == Some(tab))
                .or_else(|| visible_icons.into_iter().next());
            if let Some(tool) = tool {
                dock.show(tool, true);
            }
        }
        dock.tab = tab;
        dock.show(Tool::Properties, panels.properties);
        if panels.log {
            let requested = if log_events {
                Tool::Log
            } else {
                Tool::Problems
            };
            dock.show(requested, true);
        } else {
            dock.show(Tool::Log, false);
            dock.show(Tool::Problems, false);
        }
    }

    pub(super) fn dock_preferences(&self) -> Json {
        let mut dock = self.dock.clone();
        Self::sync_visibility(&mut dock, self.panels, self.log_events, self.tab);
        dock.preferences()
    }

    fn remember_catalogue_tool(&mut self, tool: Tool) {
        if let Some(tab) = tool.tab() {
            self.tab = tab;
            self.preferred_tab = tab;
            self.dock.tab = tab;
            self.palette_highlighted = 0;
        }
    }

    fn move_dock_tool(&mut self, tool: Tool, area: usize, before: Option<Tool>) {
        self.dock.move_tool(tool, area, before);
        self.remember_catalogue_tool(tool);
        self.sync_panels_from_dock();
    }

    fn sync_panels_from_dock(&mut self) {
        self.panels.nodes = Tool::CATALOGUE
            .into_iter()
            .any(|tool| self.dock.visible(tool));
        self.panels.properties = self.dock.visible(Tool::Properties);
        let log = self.dock.visible(Tool::Log);
        let problems = self.dock.visible(Tool::Problems);
        self.panels.log = log || problems;
        if log != problems {
            self.log_events = log;
        }
    }

    pub(super) fn restore_dock(&mut self, value: &Json) {
        if value["tools"]
            .as_array()
            .is_some_and(|groups| groups.len() == 4)
        {
            self.dock.restore(value);
            self.dock.tab = self.tab;
            self.sync_panels_from_dock();
        }
    }

    pub(super) fn dock_layout(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        Self::sync_visibility(&mut self.dock, self.panels, self.log_events, self.tab);
        self.sync_panels_from_dock();
        let mut targets = [egui::Rect::NOTHING; 4];
        let mut rails = [egui::Rect::NOTHING; 4];
        let mut slots = Vec::new();
        for side in 0..2 {
            let rail = if side == 0 {
                egui::Panel::left("dock-left-tabs")
            } else {
                egui::Panel::right("dock-right-tabs")
            };
            rail.show_separator_line(false)
                .frame(egui::Frame::NONE.fill(ui.visuals().panel_fill))
                .exact_size(if self.dock.names { 112.0 } else { RAIL })
                .show(ui, |ui| {
                    let bounds = ui.max_rect().intersect(ui.clip_rect());
                    for area in [side, side + 2] {
                        let at_bottom = area >= 2;
                        let mut rect = bounds.shrink2(egui::vec2(0.0, 3.0));
                        if at_bottom {
                            rect.min.y = bounds.center().y;
                        } else {
                            rect.max.y = bounds.center().y;
                        }
                        rails[area] = rect;
                        let layout = if at_bottom {
                            egui::Layout::bottom_up(egui::Align::Center)
                        } else {
                            egui::Layout::top_down(egui::Align::Center)
                        };
                        let mut rail = ui.new_child(
                            egui::UiBuilder::new()
                                .id_salt(area)
                                .max_rect(rect)
                                .layout(layout),
                        );
                        rail.set_clip_rect(rect);
                        rail.spacing_mut().item_spacing.y = 2.0;
                        self.dock_tabs(&mut rail, area, &mut slots);
                    }
                });
        }
        let bottom = [self.dock.active[2], self.dock.active[3]];
        if bottom.iter().any(Option::is_some) {
            let max = (ui.available_height() * 0.55).max(70.0);
            let panel = egui::Panel::bottom("dock-bottom-content")
                .show_separator_line(false)
                .frame(
                    egui::Frame::NONE
                        .fill(ui.visuals().panel_fill)
                        .inner_margin(egui::Margin::symmetric(3, 4)),
                )
                .resizable(true)
                .size_range(70.0..=max)
                .default_size(self.dock.sizes[2].clamp(70.0, max))
                .show(ui, |ui| {
                    if let [Some(left), Some(right)] = bottom {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.columns(2, |columns| {
                            for (index, tool) in [left, right].into_iter().enumerate() {
                                let column = &mut columns[index];
                                let bounds = column.max_rect().intersect(column.clip_rect());
                                targets[index + 2] = targets[index + 2].union(bounds);
                                column.set_clip_rect(column.clip_rect().intersect(bounds));
                                self.dock_surface(column, host, tool);
                            }
                        });
                    } else if let Some(tool) = bottom[0].or(bottom[1]) {
                        let bounds = ui.max_rect().intersect(ui.clip_rect());
                        let center = bounds.center().x;
                        targets[2] = targets[2].union(egui::Rect::from_min_max(
                            bounds.min,
                            egui::pos2(center, bounds.bottom()),
                        ));
                        targets[3] = targets[3].union(egui::Rect::from_min_max(
                            egui::pos2(center, bounds.top()),
                            bounds.max,
                        ));
                        self.dock_surface(ui, host, tool);
                    }
                });
            if ui.input(|input| input.pointer.is_decidedly_dragging()) {
                self.dock.sizes[2] = panel.response.rect.height();
            }
        }
        let side_max = (ui.available_width() * 0.38).max(100.0);
        for (area, target) in targets.iter_mut().enumerate().take(2) {
            if let Some(tool) = self.dock.active[area] {
                let content = if area == 0 {
                    egui::Panel::left("orange-nodes")
                } else {
                    egui::Panel::right("orange-inspector")
                };
                let panel = content
                    .show_separator_line(false)
                    .frame(
                        egui::Frame::NONE
                            .fill(ui.visuals().panel_fill)
                            .inner_margin(egui::Margin::symmetric(3, 4)),
                    )
                    .resizable(true)
                    .size_range(100.0..=side_max)
                    .default_size(self.dock.sizes[area].clamp(100.0, side_max))
                    .show(ui, |ui| self.dock_surface(ui, host, tool));
                *target = panel.response.rect;
                if ui.input(|input| input.pointer.is_decidedly_dragging()) {
                    self.dock.sizes[area] = panel.response.rect.width();
                }
            }
        }
        self.dock_drop(ui, targets, rails, &slots);
    }

    fn dock_tabs(&mut self, ui: &mut egui::Ui, area: usize, slots: &mut Vec<(Tool, egui::Rect)>) {
        let mut tools = self.dock.tools[area].clone();
        if area >= 2 {
            tools.reverse();
        }
        for tool in tools
            .into_iter()
            .filter(|tool| !self.dock.hidden.contains(tool))
            .collect::<Vec<_>>()
        {
            let active = self.dock.active[area] == Some(tool);
            let label = match tool {
                Tool::Nodes => format!(
                    "{} ({})",
                    t(tool.key()),
                    self.canvas.keys.label("left-panel")
                ),
                Tool::Properties => format!(
                    "{} ({})",
                    t(tool.key()),
                    self.canvas.keys.label("right-panel")
                ),
                _ => t(tool.key()).to_owned(),
            };
            let response = ui
                .push_id(tool.key(), |ui| {
                    icons::dock_tab(
                        ui,
                        tool.icon(),
                        active,
                        self.dock.names.then(|| t(tool.key())),
                    )
                })
                .inner;
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::Button, true, active, t(tool.key()))
            });
            let response = response.on_hover_text(label);
            slots.push((tool, response.rect));
            response.dnd_set_drag_payload(tool);
            if response.clicked() {
                self.dock.show(tool, !active);
                if !active {
                    self.remember_catalogue_tool(tool);
                }
                self.sync_panels_from_dock();
            }
            response.context_menu(|ui| {
                if ui.button(t("dock-hide-icon")).clicked() {
                    self.dock.hide(tool);
                    self.sync_panels_from_dock();
                    ui.close();
                }
                ui.checkbox(&mut self.dock.names, t("dock-show-names"));
                ui.add_space(4.0);
                for (destination, label) in AREA_LABELS.into_iter().enumerate() {
                    if ui
                        .add_enabled(destination != area, egui::Button::new(t(label)))
                        .clicked()
                    {
                        self.move_dock_tool(tool, destination, None);
                        ui.close();
                    }
                }
            });
        }
        let rest = ui.available_rect_before_wrap().intersect(ui.clip_rect());
        if rest.is_positive() {
            ui.interact(rest, ui.scope_id().with("empty-rail"), egui::Sense::click())
                .context_menu(|ui| {
                    ui.checkbox(&mut self.dock.names, t("dock-show-names"));
                    ui.add_enabled_ui(!self.dock.hidden.is_empty(), |ui| {
                        ui.menu_button(t("dock-add-icon"), |ui| {
                            for tool in self.dock.hidden.clone() {
                                if ui.button(t(tool.key())).clicked() {
                                    self.move_dock_tool(tool, area, None);
                                    ui.close();
                                }
                            }
                        })
                    });
                });
        }
    }

    fn dock_drop(
        &mut self,
        ui: &egui::Ui,
        targets: [egui::Rect; 4],
        rails: [egui::Rect; 4],
        slots: &[(Tool, egui::Rect)],
    ) {
        let Some(tool) = egui::DragAndDrop::payload::<Tool>(ui.ctx()) else {
            return;
        };
        let Some(pointer) = ui.input(|input| input.pointer.interact_pos()) else {
            return;
        };
        let Some((area, target)) = rails
            .iter()
            .enumerate()
            .chain(targets.iter().enumerate())
            .find(|(_, rect)| rect.contains(pointer))
        else {
            return;
        };
        let before = slots
            .iter()
            .find(|(item, rect)| self.dock.area(*item) == Some(area) && rect.contains(pointer))
            .map(|(item, _)| *item);
        ui.ctx()
            .layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::unique("dock-drop"),
            ))
            .rect_stroke(
                target.shrink(1.0),
                4,
                ui.visuals().selection.stroke,
                egui::StrokeKind::Inside,
            );
        if ui.input(|input| input.pointer.primary_released()) {
            let _ = egui::DragAndDrop::take_payload::<Tool>(ui.ctx());
            self.move_dock_tool(*tool, area, before);
            ui.ctx().request_repaint();
        }
    }

    fn dock_surface(&mut self, ui: &mut egui::Ui, host: &mut Host, tool: Tool) {
        egui::Frame::new()
            .fill(
                ui.visuals()
                    .panel_fill
                    .lerp_to_gamma(ui.visuals().faint_bg_color, 0.42),
            )
            .corner_radius(self.corners.docks)
            .inner_margin(egui::Margin::symmetric(9, 8))
            .show(ui, |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(ui.max_rect()));
                ui.set_min_size(ui.available_size());
                if tool != Tool::GraphInfo {
                    ui.strong(t(tool.key()));
                    ui.add_space(4.0);
                }
                self.dock_content(ui, host, tool);
            });
    }

    fn dock_content(&mut self, ui: &mut egui::Ui, host: &mut Host, tool: Tool) {
        ui.style_mut().spacing.scroll = workspace_scroll();
        match tool {
            Tool::Nodes => self.node_catalogue(ui, host),
            Tool::Variables => self.variables_tab(ui, host),
            Tool::Types => self.types_tab(ui, host),
            Tool::Connections => self.connections_tab(ui, host),
            Tool::Properties => self.right_panel(ui, host),
            Tool::Log | Tool::Problems => {
                let previous = std::mem::replace(&mut self.log_events, tool == Tool::Log);
                self.log_panel(ui);
                self.log_events = previous;
            }
            Tool::GraphInfo => {
                ui.strong(t("graph-info"));
                if let Some(path) = self.path().filter(|path| !path.is_empty()) {
                    let name = std::path::Path::new(path)
                        .file_name()
                        .map_or_else(|| path.into(), std::ffi::OsStr::to_string_lossy);
                    ui.add(egui::Label::new(name).truncate())
                        .on_hover_text(path);
                }
                ui.label(t(if self.is_running() {
                    "running"
                } else if self.document.is_some() {
                    "stopped"
                } else {
                    "no-graph"
                }));
                if self.is_unsaved() {
                    ui.weak(t("unsaved"));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dragging_an_icon_moves_it_to_an_empty_area_and_persists() {
        let mut editor = Editor::new();
        let ctx = egui::Context::default();
        icons::install(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0));
        let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
        editor.dock.move_tool(Tool::Problems, 2, None);
        editor.dock.move_tool(Tool::Log, 2, None);
        editor.dock.show(Tool::Log, false);

        let positions = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let painted = positions.clone();
        icons::set_command_painter(
            &ctx,
            Some(std::sync::Arc::new(move |_, rect, icon, _| {
                painted.lock().unwrap().push((icon, rect.center()));
                true
            })),
        );
        let mut frame = |editor: &mut Editor, events| {
            positions.lock().unwrap().clear();
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    ..Default::default()
                },
                |ui| editor.dock_layout(ui, &mut host),
            )
            .drop_without_applying_deltas();
        };
        frame(&mut editor, vec![]);
        let start = {
            let positions = positions.lock().unwrap();
            let at = |tool: Tool| {
                positions
                    .iter()
                    .find(|(icon, _)| *icon == tool.icon())
                    .unwrap()
                    .1
            };
            let nodes = at(Tool::Nodes);
            let info = at(Tool::GraphInfo);
            assert!(nodes.x < RAIL && info.x < RAIL);
            assert!(nodes.y < screen.center().y && info.y > screen.center().y);
            assert!(at(Tool::Properties).x > screen.right() - RAIL);
            nodes
        };
        let destination = egui::pos2(screen.right() - RAIL * 0.5, screen.bottom() - 28.0);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(start)],
            vec![button(start, true)],
            vec![egui::Event::PointerMoved(destination)],
            vec![button(destination, false)],
        ] {
            frame(&mut editor, events);
        }
        assert_eq!(editor.dock.area(Tool::Nodes), Some(3));
        assert_eq!(editor.dock.active[3], Some(Tool::Nodes));
        let mut restored = Editor::new();
        restored.restore_preferences(&editor.preferences());
        assert_eq!(restored.dock.area(Tool::Nodes), Some(3));
        assert_eq!(restored.dock.active[3], Some(Tool::Nodes));
    }

    #[test]
    fn right_clicking_empty_bottom_rail_offers_to_restore_a_hidden_icon() {
        let mut editor = Editor::new();
        let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
        editor.dock.hide(Tool::Variables);
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(700.0, 500.0));
        let at = egui::pos2(screen.right() - RAIL / 2.0, screen.bottom() - 108.0);
        let menu = t("dock-add-icon");
        let mut menu_shown = false;
        for events in [
            vec![],
            vec![egui::Event::PointerMoved(at)],
            vec![egui::Event::PointerButton {
                pos: at,
                pressed: true,
                button: egui::PointerButton::Secondary,
                modifiers: egui::Modifiers::NONE,
            }],
            vec![egui::Event::PointerButton {
                pos: at,
                pressed: false,
                button: egui::PointerButton::Secondary,
                modifiers: egui::Modifiers::NONE,
            }],
            vec![],
        ] {
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    ..Default::default()
                },
                |ui| editor.dock_layout(ui, &mut host),
            );
            menu_shown |= output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == menu));
            output.drop_without_applying_deltas();
        }
        assert!(menu_shown);
    }

    #[test]
    fn hidden_tools_stay_hidden_until_added_to_a_rail() {
        let mut dock = Dock::default();
        assert_eq!(dock.tools[0], Tool::CATALOGUE);
        dock.hide(Tool::Nodes);
        dock.names = true;
        let mut restored = Dock::default();
        restored.restore(&dock.preferences());
        assert_eq!(restored.hidden, [Tool::Nodes]);
        assert!(restored.names);
        restored.show(Tool::Nodes, true);
        assert!(!restored.visible(Tool::Nodes));
        restored.move_tool(Tool::Nodes, 3, None);
        assert!(restored.hidden.is_empty());
        assert!(restored.visible(Tool::Nodes));
        assert_eq!(restored.area(Tool::Nodes), Some(3));
    }

    #[test]
    fn tabs_can_be_reordered_and_fit_into_each_area() {
        let mut dock = Dock::default();
        dock.move_tool(Tool::Connections, 1, Some(Tool::Properties));
        assert_eq!(dock.tools[1], [Tool::Connections, Tool::Properties]);
        for area in [0, 2, 3, 1] {
            dock.move_tool(Tool::Connections, area, None);
            assert_eq!(dock.area(Tool::Connections), Some(area));
            assert_eq!(dock.active[area], Some(Tool::Connections));
        }
        let mut restored = Dock::default();
        restored.restore(&dock.preferences());
        assert_eq!(restored.tools, dock.tools);
        assert_eq!(restored.active, dock.active);
    }
}
