//! Workspace tool placement, independent of the graph document.
use super::*;
use serde_json::{Value as Json, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tool {
    Nodes,
    Properties,
    Packages,
    GraphInfo,
    Problems,
    Log,
}

impl Tool {
    const ALL: [Self; 6] = [
        Self::Nodes,
        Self::Properties,
        Self::Packages,
        Self::GraphInfo,
        Self::Problems,
        Self::Log,
    ];

    fn key(self) -> &'static str {
        match self {
            Self::Nodes => "nodes",
            Self::Properties => "properties",
            Self::Packages => "packages",
            Self::GraphInfo => "graph-info",
            Self::Problems => "problems",
            Self::Log => "log",
        }
    }

    fn icon(self) -> char {
        match self {
            Self::Nodes => icons::ARRANGE,
            Self::Properties => icons::PROPERTIES,
            Self::Packages => icons::PACKAGES,
            Self::GraphInfo => icons::INFO,
            Self::Problems => icons::WARNING,
            Self::Log => icons::TEXT,
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

#[derive(Clone, Debug)]
pub(super) struct Dock {
    tools: [Vec<Tool>; 4],
    active: [Option<Tool>; 4],
    sizes: [f32; 3],
}

impl Default for Dock {
    fn default() -> Self {
        Self {
            tools: [
                vec![Tool::Nodes],
                vec![Tool::Properties, Tool::Packages],
                vec![Tool::GraphInfo],
                vec![Tool::Problems, Tool::Log],
            ],
            active: [Some(Tool::Nodes), Some(Tool::Properties), None, None],
            sizes: [280.0, 280.0, 170.0],
        }
    }
}

impl Dock {
    fn area(&self, tool: Tool) -> usize {
        self.tools
            .iter()
            .position(|tools| tools.contains(&tool))
            .expect("each tool has an area")
    }

    fn visible(&self, tool: Tool) -> bool {
        self.active.contains(&Some(tool))
    }

    fn show(&mut self, tool: Tool, visible: bool) {
        let area = self.area(tool);
        if visible || self.active[area] == Some(tool) {
            self.active[area] = visible.then_some(tool);
        }
    }

    fn move_tool(&mut self, tool: Tool, destination: usize, before: Option<Tool>) {
        if before == Some(tool) {
            return;
        }
        let source = self.area(tool);
        self.show(tool, false);
        self.tools[source].retain(|item| *item != tool);
        let tools = &mut self.tools[destination];
        let index = before
            .and_then(|target| tools.iter().position(|item| *item == target))
            .unwrap_or(tools.len());
        tools.insert(index, tool);
        self.active[destination] = Some(tool);
    }

    pub(super) fn preferences(&self) -> Json {
        let tools = self
            .tools
            .each_ref()
            .map(|tools| tools.iter().map(|tool| tool.key()).collect::<Vec<_>>());
        json!({"tools": tools, "active": self.active.map(|tool| tool.map(Tool::key)), "sizes": self.sizes})
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
        self.active = std::array::from_fn(|area| {
            Tool::read(&value["active"][area]).filter(|tool| self.tools[area].contains(tool))
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
    fn sync_visibility(dock: &mut Dock, panels: Panels, log_events: bool) {
        dock.show(Tool::Nodes, panels.nodes);
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
        Self::sync_visibility(&mut dock, self.panels, self.log_events);
        dock.preferences()
    }

    fn sync_panels_from_dock(&mut self) {
        self.panels.nodes = self.dock.visible(Tool::Nodes);
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
            self.sync_panels_from_dock();
        }
    }

    pub(super) fn dock_layout(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        Self::sync_visibility(&mut self.dock, self.panels, self.log_events);
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
            rail.frame(egui::Frame::NONE.fill(ui.visuals().panel_fill))
                .exact_size(RAIL)
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
                .resizable(true)
                .size_range(70.0..=max)
                .default_size(self.dock.sizes[2].clamp(70.0, max))
                .show(ui, |ui| {
                    if let [Some(left), Some(right)] = bottom {
                        ui.columns(2, |columns| {
                            for (index, tool) in [left, right].into_iter().enumerate() {
                                let column = &mut columns[index];
                                let bounds = column.max_rect().intersect(column.clip_rect());
                                targets[index + 2] = targets[index + 2].union(bounds);
                                column.set_clip_rect(column.clip_rect().intersect(bounds));
                                self.dock_content(column, host, tool);
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
                        self.dock_content(ui, host, tool);
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
                    .resizable(true)
                    .size_range(100.0..=side_max)
                    .default_size(self.dock.sizes[area].clamp(100.0, side_max))
                    .show(ui, |ui| self.dock_content(ui, host, tool));
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
        for tool in tools {
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
                .push_id(tool.key(), |ui| icons::dock_tab(ui, tool.icon(), active))
                .inner;
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::Button, true, active, t(tool.key()))
            });
            let response = response.on_hover_text(label);
            slots.push((tool, response.rect));
            response.dnd_set_drag_payload(tool);
            if response.clicked() {
                self.dock.show(tool, !active);
                self.sync_panels_from_dock();
            }
            response.context_menu(|ui| {
                for (destination, label) in AREA_LABELS.into_iter().enumerate() {
                    if ui
                        .add_enabled(destination != area, egui::Button::new(t(label)))
                        .clicked()
                    {
                        self.dock.move_tool(tool, destination, None);
                        self.sync_panels_from_dock();
                        ui.close();
                    }
                }
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
            .find(|(item, rect)| self.dock.area(*item) == area && rect.contains(pointer))
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
            self.dock.move_tool(*tool, area, before);
            self.sync_panels_from_dock();
            ui.ctx().request_repaint();
        }
    }

    fn dock_content(&mut self, ui: &mut egui::Ui, host: &mut Host, tool: Tool) {
        ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();
        match tool {
            Tool::Nodes => self.nodes_panel(ui, host),
            Tool::Properties => self.right_panel(ui, host),
            Tool::Packages => {
                self.graph_bundle_menu(ui, host);
                self.package_manager(ui, host);
            }
            Tool::Log | Tool::Problems => {
                let previous = std::mem::replace(&mut self.log_events, tool == Tool::Log);
                self.log_panel(ui);
                self.log_events = previous;
            }
            Tool::GraphInfo => {
                ui.strong(t("graph-info"));
                if let Some(path) = self.path().filter(|path| !path.is_empty()) {
                    ui.label(path).on_hover_text(path);
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
        assert_eq!(editor.dock.area(Tool::Nodes), 3);
        assert_eq!(editor.dock.active[3], Some(Tool::Nodes));
        let mut restored = Editor::new();
        restored.restore_preferences(&editor.preferences());
        assert_eq!(restored.dock.area(Tool::Nodes), 3);
        assert_eq!(restored.dock.active[3], Some(Tool::Nodes));
    }

    #[test]
    fn tabs_can_be_reordered_and_fit_into_each_area() {
        let mut dock = Dock::default();
        dock.move_tool(Tool::Packages, 1, Some(Tool::Properties));
        assert_eq!(dock.tools[1], [Tool::Packages, Tool::Properties]);
        for area in [0, 2, 3, 1] {
            dock.move_tool(Tool::Packages, area, None);
            assert_eq!(dock.area(Tool::Packages), area);
            assert_eq!(dock.active[area], Some(Tool::Packages));
        }
        let mut restored = Dock::default();
        restored.restore(&dock.preferences());
        assert_eq!(restored.tools, dock.tools);
        assert_eq!(restored.active, dock.active);
    }
}
