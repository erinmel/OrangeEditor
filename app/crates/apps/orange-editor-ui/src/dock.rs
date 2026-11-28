//! Workspace tool placement, independent of the graph document.
use super::*;
use crate::bar_spacing::Spacing;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DockItem {
    Tool(Tool),
    Spacing { kind: Spacing, id: u32 },
}

impl DockItem {
    fn preference(self) -> Json {
        match self {
            Self::Tool(tool) => json!(tool.key()),
            Self::Spacing { kind, id } => json!({"spacing": kind.key(), "id": id}),
        }
    }

    fn read(value: &Json) -> Option<Self> {
        if let Some(tool) = Tool::read(value) {
            return Some(Self::Tool(tool));
        }
        let kind = value["spacing"].as_str().and_then(Spacing::read)?;
        let id = u32::try_from(value["id"].as_u64()?).ok()?;
        Some(Self::Spacing { kind, id })
    }
}

const AREA_LABELS: [&str; 4] = [
    "dock-left",
    "dock-right",
    "dock-bottom-left",
    "dock-bottom-right",
];
const RAIL: f32 = 30.0;
pub(super) type DockSplitter = (usize, egui::Rect, f32, f32);

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
    items: [Vec<DockItem>; 4],
    active: [Option<Tool>; 4],
    sizes: [f32; 3],
    hidden: Vec<Tool>,
    pub(super) names: bool,
    tab: Tab,
    next_spacing_id: u32,
}

impl Default for Dock {
    fn default() -> Self {
        Self {
            items: [
                Tool::CATALOGUE.into_iter().map(DockItem::Tool).collect(),
                vec![DockItem::Tool(Tool::Properties)],
                vec![DockItem::Tool(Tool::GraphInfo)],
                vec![DockItem::Tool(Tool::Problems), DockItem::Tool(Tool::Log)],
            ],
            active: [Some(Tool::Nodes), Some(Tool::Properties), None, None],
            sizes: [280.0, 280.0, 170.0],
            hidden: Vec::new(),
            names: false,
            tab: Tab::Nodes,
            next_spacing_id: 0,
        }
    }
}

impl Dock {
    fn area(&self, tool: Tool) -> Option<usize> {
        self.item_area(DockItem::Tool(tool))
    }

    fn item_area(&self, item: DockItem) -> Option<usize> {
        self.items.iter().position(|items| items.contains(&item))
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
        self.move_item(
            DockItem::Tool(tool),
            destination,
            before.map(DockItem::Tool),
        );
    }

    fn move_item(&mut self, item: DockItem, destination: usize, before: Option<DockItem>) {
        if before == Some(item) || destination >= self.items.len() {
            return;
        }
        let Some(source) = self.item_area(item) else {
            return;
        };
        if let DockItem::Tool(tool) = item {
            self.show(tool, false);
        }
        self.items[source].retain(|current| *current != item);
        let Some(items) = self.items.get_mut(destination) else {
            return;
        };
        let index = before
            .and_then(|target| items.iter().position(|current| *current == target))
            .unwrap_or(items.len());
        items.insert(index, item);
        if let DockItem::Tool(tool) = item {
            self.hidden.retain(|hidden| *hidden != tool);
            self.active[destination] = Some(tool);
        }
    }

    fn add_spacing(&mut self, kind: Spacing, area: usize) -> bool {
        let Some(next_id) = self.next_spacing_id.checked_add(1) else {
            return false;
        };
        let Some(items) = self.items.get_mut(area) else {
            return false;
        };
        items.push(DockItem::Spacing {
            kind,
            id: self.next_spacing_id,
        });
        self.next_spacing_id = next_id;
        true
    }

    fn remove_spacing(&mut self, item: DockItem) {
        if matches!(item, DockItem::Spacing { .. }) {
            for items in &mut self.items {
                items.retain(|current| *current != item);
            }
        }
    }

    pub(super) fn preferences(&self) -> Json {
        let items = self.items.each_ref().map(|items| {
            items
                .iter()
                .copied()
                .map(DockItem::preference)
                .collect::<Vec<_>>()
        });
        json!({"tools": items, "active": self.active.map(|tool| tool.map(Tool::key)), "sizes": self.sizes, "names": self.names, "hidden": self.hidden.iter().map(|tool| tool.key()).collect::<Vec<_>>() })
    }

    pub(super) fn restore(&mut self, value: &Json) {
        let Some(groups) = value["tools"].as_array().filter(|groups| groups.len() == 4) else {
            return;
        };
        let defaults = Self::default();
        let mut seen = Vec::new();
        let mut next_spacing_id = 0;
        self.items = std::array::from_fn(|area| {
            groups[area]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(DockItem::read)
                .filter(|item| {
                    if seen.contains(item) {
                        return false;
                    }
                    if let DockItem::Spacing { id, .. } = item {
                        next_spacing_id = next_spacing_id.max(id.saturating_add(1));
                    }
                    seen.push(*item);
                    true
                })
                .collect()
        });
        self.next_spacing_id = next_spacing_id;
        for (area, items) in defaults.items.iter().enumerate() {
            self.items[area].extend(items.iter().copied().filter(|item| !seen.contains(item)));
        }
        self.names = value["names"].as_bool().unwrap_or(false);
        self.hidden = value["hidden"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Tool::read)
            .collect();
        self.active = std::array::from_fn(|area| {
            Tool::read(&value["active"][area]).filter(|tool| {
                self.items[area].contains(&DockItem::Tool(*tool)) && !self.hidden.contains(tool)
            })
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

    pub(super) fn dock_layout(&mut self, ui: &mut egui::Ui, host: &mut Host) -> Vec<DockSplitter> {
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
            rail.resizable(false)
                .show_separator_line(false)
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
        let mut splitters = Vec::new();
        let bottom = [self.dock.active[2], self.dock.active[3]];
        if bottom.iter().any(Option::is_some) {
            let max = (ui.available_height() * 0.55).max(70.0);
            let panel = egui::Panel::bottom("dock-bottom-content")
                .resizable(false)
                .show_separator_line(false)
                .frame(
                    egui::Frame::NONE
                        .fill(ui.visuals().panel_fill)
                        .inner_margin(egui::Margin::symmetric(3, 4)),
                )
                .exact_size(self.dock.sizes[2].clamp(70.0, max))
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
            let rect = panel.response.rect;
            splitters.push((
                2,
                egui::Rect::from_min_max(rect.left_top(), rect.right_top()),
                70.0,
                max,
            ));
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
                    .resizable(false)
                    .show_separator_line(false)
                    .frame(
                        egui::Frame::NONE
                            .fill(ui.visuals().panel_fill)
                            .inner_margin(egui::Margin::symmetric(3, 4)),
                    )
                    .exact_size(self.dock.sizes[area].clamp(100.0, side_max))
                    .show(ui, |ui| self.dock_surface(ui, host, tool));
                *target = panel.response.rect;
                let rect = panel.response.rect;
                let x = if area == 0 { rect.right() } else { rect.left() };
                splitters.push((
                    area,
                    egui::Rect::from_min_max(
                        egui::pos2(x, rect.top()),
                        egui::pos2(x, rect.bottom()),
                    ),
                    100.0,
                    side_max,
                ));
            }
        }
        self.dock_drop(ui, targets, rails, &slots);
        splitters
    }

    pub(super) fn resize_docks(&mut self, ui: &egui::Ui, splitters: Vec<DockSplitter>) {
        for (area, edge, min, max) in splitters {
            let id = ui.scope_id().with(("dock-resize", area));
            let inward = match area {
                0 => -egui::Vec2::X,
                1 => egui::Vec2::X,
                _ => egui::Vec2::Y,
            };
            let response =
                orange_ui_controls::navigation::splitter(ui, id, edge, inward, t("dock-resize"));
            let origin_id = id.with("origin");
            if response.has_focus() {
                let (back, forward) = if area == 2 {
                    (egui::Key::ArrowUp, egui::Key::ArrowDown)
                } else {
                    (egui::Key::ArrowLeft, egui::Key::ArrowRight)
                };
                let steps = ui.input_mut(|input| {
                    i8::from(input.consume_key(egui::Modifiers::NONE, forward))
                        - i8::from(input.consume_key(egui::Modifiers::NONE, back))
                });
                if steps != 0 {
                    let direction = if area == 0 { 1.0 } else { -1.0 };
                    self.dock.sizes[area] = (self.dock.sizes[area].clamp(min, max)
                        + 12.0 * f32::from(steps) * direction)
                        .clamp(min, max);
                    ui.request_repaint();
                }
            }
            if response.drag_started() {
                let shown = self.dock.sizes[area];
                let axis = usize::from(area == 2);
                if let Some(press) = ui.input(|input| input.pointer.press_origin()) {
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(origin_id, (press[axis], shown.clamp(min, max)))
                    });
                }
            }
            if (response.dragged() || response.drag_stopped())
                && let Some(pointer) = response.interact_pointer_pos()
                && let Some((press, initial)) =
                    ui.ctx().data(|data| data.get_temp::<(f32, f32)>(origin_id))
            {
                let delta = pointer[usize::from(area == 2)] - press;
                let size = (initial + if area == 0 { delta } else { -delta }).clamp(min, max);
                if self.dock.sizes[area] != size {
                    self.dock.sizes[area] = size;
                    ui.request_repaint();
                }
            }
        }
    }

    fn dock_tabs(
        &mut self,
        ui: &mut egui::Ui,
        area: usize,
        slots: &mut Vec<(DockItem, egui::Rect)>,
    ) {
        let mut items = self.dock.items[area]
            .iter()
            .copied()
            .filter(|item| match item {
                DockItem::Tool(tool) => !self.dock.hidden.contains(tool),
                DockItem::Spacing { .. } => true,
            })
            .collect::<Vec<_>>();
        if area >= 2 {
            items.reverse();
        }
        let flexible_count = items
            .iter()
            .filter(|item| {
                matches!(
                    item,
                    DockItem::Spacing {
                        kind: Spacing::Flexible,
                        ..
                    }
                )
            })
            .count();
        let fixed_height = items
            .iter()
            .map(|item| match item {
                DockItem::Tool(_) => 26.0,
                DockItem::Spacing { kind, .. } => kind.extent(0.0),
            })
            .sum::<f32>()
            + ui.spacing().item_spacing.y * items.len().saturating_sub(1) as f32;
        let flexible = ((ui.available_height() - fixed_height).max(0.0)
            / flexible_count.max(1) as f32)
            .max(0.0);
        for item in items {
            if let DockItem::Spacing { kind, .. } = item {
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), kind.extent(flexible)),
                    egui::Sense::click_and_drag(),
                );
                kind.paint(ui, rect, false);
                slots.push((item, rect));
                response.dnd_set_drag_payload(item);
                response.context_menu(|ui| {
                    if ui.button(t("bar-remove-spacing")).clicked() {
                        self.dock.remove_spacing(item);
                        ui.close();
                    }
                });
                continue;
            }
            let DockItem::Tool(tool) = item else {
                continue;
            };
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
            slots.push((item, response.rect));
            response.dnd_set_drag_payload(item);
            if response.clicked() {
                self.dock.show(tool, !active);
                if !active {
                    self.remember_catalogue_tool(tool);
                }
                self.sync_panels_from_dock();
            }
            response.context_menu(|ui| {
                if ui
                    .button((icons::centered(icons::CLOSE), t("dock-hide-icon")))
                    .clicked()
                {
                    self.dock.hide(tool);
                    self.sync_panels_from_dock();
                    ui.close();
                }
                ui.checkbox(&mut self.dock.names, t("dock-show-names"));
                self.dock_add_menu(ui, area);
                self.dock_add_spacing_menu(ui, area);
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
                    self.dock_add_menu(ui, area);
                    self.dock_add_spacing_menu(ui, area);
                });
        }
    }

    fn dock_add_menu(&mut self, ui: &mut egui::Ui, area: usize) {
        ui.add_enabled_ui(!self.dock.hidden.is_empty(), |ui| {
            ui.menu_button(t("dock-add-icon"), |ui| {
                for tool in Tool::ALL {
                    if self.dock.hidden.contains(&tool)
                        && ui
                            .button((icons::centered(tool.icon()), t(tool.key())))
                            .clicked()
                    {
                        self.move_dock_tool(tool, area, None);
                        ui.close();
                    }
                }
            });
        });
    }

    fn dock_add_spacing_menu(&mut self, ui: &mut egui::Ui, area: usize) {
        ui.menu_button(t("toolbar-add-spacing"), |ui| {
            for kind in Spacing::ALL {
                if ui.button(t(kind.add_key())).clicked() {
                    self.dock.add_spacing(kind, area);
                    ui.close();
                }
            }
        });
    }

    fn dock_drop(
        &mut self,
        ui: &egui::Ui,
        targets: [egui::Rect; 4],
        rails: [egui::Rect; 4],
        slots: &[(DockItem, egui::Rect)],
    ) {
        let Some(item) = egui::DragAndDrop::payload::<DockItem>(ui.ctx()) else {
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
            .find(|(slot, rect)| self.dock.item_area(*slot) == Some(area) && rect.contains(pointer))
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
            let _ = egui::DragAndDrop::take_payload::<DockItem>(ui.ctx());
            self.dock.move_item(*item, area, before);
            if let DockItem::Tool(tool) = *item {
                self.remember_catalogue_tool(tool);
                self.sync_panels_from_dock();
            }
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
                |ui| {
                    editor.dock_layout(ui, &mut host);
                },
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
        let spacing_menu = t("toolbar-add-spacing");
        let mut menu_shown = false;
        let mut spacing_menu_shown = false;
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
                |ui| {
                    editor.dock_layout(ui, &mut host);
                },
            );
            for shape in &output.shapes {
                if let egui::Shape::Text(text) = &shape.shape {
                    menu_shown |= text.galley.text() == menu;
                    spacing_menu_shown |= text.galley.text() == spacing_menu;
                }
            }
            output.drop_without_applying_deltas();
        }
        assert!(menu_shown);
        assert!(spacing_menu_shown);
    }

    #[test]
    fn icon_rail_edges_do_not_offer_resize_cursors() {
        let ctx = egui::Context::default();
        let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
        for names in [false, true] {
            editor.dock.names = names;
            let width = if names { 112.0 } else { RAIL };
            for x in [width, screen.width() - width] {
                for _ in 0..2 {
                    let output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(screen),
                            events: vec![egui::Event::PointerMoved(egui::pos2(x, 270.0))],
                            ..Default::default()
                        },
                        |ui| editor.ui(ui, &mut host),
                    );
                    let cursor = output.platform_output.cursor_icon;
                    output.drop_without_applying_deltas();
                    assert_ne!(cursor, egui::CursorIcon::ResizeHorizontal);
                }
            }
        }
    }

    #[test]
    fn splitter_grips_stay_inside_docks_and_drag_without_jumping() {
        let ctx = egui::Context::default();
        let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.dock.show(Tool::GraphInfo, true);
        let mut handles = [egui::Rect::NOTHING; 3];
        let mut frame = |editor: &mut Editor, events| {
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 800.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let root = ui.scope_id();
                    editor.ui(ui, &mut host);
                    for (area, handle) in handles.iter_mut().enumerate() {
                        if let Some(response) = ui.read_response(root.with(("dock-resize", area))) {
                            *handle = response.rect;
                        }
                    }
                },
            );
            let grips = std::array::from_fn::<_, 3, _>(|area| {
                let inward = [-egui::Vec2::X, egui::Vec2::X, egui::Vec2::Y][area];
                let size = if area == 2 {
                    egui::vec2(40.0, 3.0)
                } else {
                    egui::vec2(3.0, 40.0)
                };
                let expected =
                    egui::Rect::from_center_size(handles[area].center() + 3.5 * inward, size);
                output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.rect == expected))
            });
            output.drop_without_applying_deltas();
            (handles, grips)
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: egui::Modifiers::NONE,
        };
        for (area, motion) in [
            egui::vec2(25.0, 0.0),
            egui::vec2(-25.0, 0.0),
            egui::vec2(0.0, -25.0),
        ]
        .into_iter()
        .enumerate()
        {
            let before = editor.dock.sizes;
            let (handles, grips) = frame(&mut editor, vec![]);
            assert!(
                grips[area],
                "dock {area} grip must be inset with matching dimensions"
            );
            let handle = handles[area];
            assert!(handle.is_positive());
            let start = handle.center() + motion.normalized() * 6.0;
            let end = start + motion;
            for events in [
                vec![egui::Event::PointerMoved(start)],
                vec![button(start, true)],
                vec![egui::Event::PointerMoved(end)],
                vec![button(end, false)],
            ] {
                frame(&mut editor, events);
            }
            let mut expected = before;
            expected[area] += 25.0;
            assert_eq!(editor.dock.sizes, expected);
        }
    }

    #[test]
    fn hidden_tools_stay_hidden_until_added_to_a_rail() {
        let mut dock = Dock::default();
        assert_eq!(dock.items[0], Tool::CATALOGUE.map(DockItem::Tool));
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
        assert_eq!(
            dock.items[1],
            [
                DockItem::Tool(Tool::Connections),
                DockItem::Tool(Tool::Properties)
            ]
        );
        for area in [0, 2, 3, 1] {
            dock.move_tool(Tool::Connections, area, None);
            assert_eq!(dock.area(Tool::Connections), Some(area));
            assert_eq!(dock.active[area], Some(Tool::Connections));
        }
        let mut restored = Dock::default();
        restored.restore(&dock.preferences());
        assert_eq!(restored.items, dock.items);
        assert_eq!(restored.active, dock.active);
    }

    #[test]
    fn spacing_items_can_be_reordered_removed_and_restored() {
        let mut dock = Dock::default();
        assert!(dock.add_spacing(Spacing::Flexible, 0));
        assert!(dock.add_spacing(Spacing::Fixed, 0));
        assert!(dock.add_spacing(Spacing::Divider, 0));
        let flexible = dock.items[0][Tool::CATALOGUE.len()];
        dock.move_item(flexible, 2, Some(DockItem::Tool(Tool::GraphInfo)));
        assert_eq!(dock.item_area(flexible), Some(2));

        let mut restored = Dock::default();
        restored.restore(&dock.preferences());
        assert_eq!(restored.items, dock.items);
        assert!(restored.add_spacing(Spacing::Flexible, 3));
        let next = *restored.items[3].last().unwrap();
        assert_ne!(next, flexible);

        restored.remove_spacing(flexible);
        assert_eq!(restored.item_area(flexible), None);
        assert_eq!(restored.item_area(DockItem::Tool(Tool::GraphInfo)), Some(2));
    }
}
