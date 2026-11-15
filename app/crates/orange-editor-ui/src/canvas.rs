//! The graph canvas: nodes as cards with their ports, wires between them, panning,
//! zooming, selecting and moving nodes, and controls on the cards for the values of
//! inputs.
//!
//! It is a widget of its own. An application that has its own lists and panels shows
//! only this: it asks its host for a [`GraphView`], calls [`Canvas::show`] each frame
//! and acts on the [`CanvasResponse`]. Nothing here knows about files, flows or the
//! other panels of the editor.
use egui::epaint::CubicBezierShape;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, UiBuilder, Vec2};
use orange_host::{GraphView, NodeId, NodeView, PortView};
use std::collections::{BTreeMap, BTreeSet};

/// Width of a node card, in graph units (one unit is one point at zoom 1).
pub const NODE_WIDTH: f32 = 236.0;
/// Height of a card's title.
pub const HEADER_HEIGHT: f32 = 34.0;
/// Where the first row of ports starts: under the title, with a little room.
pub const BODY_TOP: f32 = HEADER_HEIGHT + 6.0;
/// Height of one row of ports.
pub const ROW_HEIGHT: f32 = 26.0;
/// Space under the last row.
pub const FOOTER_HEIGHT: f32 = 8.0;
/// Distance between the columns and rows of an automatic layout.
pub const LAYOUT_GAP: Vec2 = Vec2::new(90.0, 36.0);
/// The zoom range.
pub const ZOOM_RANGE: std::ops::RangeInclusive<f32> = 0.2..=2.5;
const GRID: f32 = 24.0;
const PORT_RADIUS: f32 = 5.0;
/// Below this zoom, cards show their values without controls.
const CONTROLS_ZOOM: f32 = 0.75;
/// Space between a card's edge and what is inside it.
const CARD_PADDING: f32 = 12.0;
/// Height of a field or control in a row of a card.
const FIELD_HEIGHT: f32 = 20.0;
/// Width of the control for an input's value on a card.
const CONTROL_WIDTH: f32 = 96.0;

/// How a value on an input is changed on its card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Control {
    Toggle,
    Whole,
    Number,
    Text,
}

impl Control {
    fn of(port: &PortView) -> Option<Self> {
        if !port.input || port.exec || port.connected {
            return None;
        }
        match port.ty.as_deref()? {
            "core/bool@1" => Some(Self::Toggle),
            "core/i64@1" | "core/u64@1" | "core/i32@1" | "core/u32@1" => Some(Self::Whole),
            "core/f64@1" | "core/f32@1" => Some(Self::Number),
            "core/string@1" => Some(Self::Text),
            _ => None,
        }
    }
}

/// The size of a node's card in graph units: a title and one row for each input beside
/// an output. A node with no ports yet keeps one row, which says to set it up.
pub fn node_size(node: &NodeView) -> Vec2 {
    let inputs = node.ports.iter().filter(|port| port.input).count();
    let outputs = node.ports.len() - inputs;
    #[expect(clippy::cast_precision_loss, reason = "a node has few ports")]
    let rows = inputs.max(outputs).max(1) as f32;
    Vec2::new(NODE_WIDTH, BODY_TOP + rows * ROW_HEIGHT + FOOTER_HEIGHT)
}

/// Where a port's dot is, relative to the top left corner of its node's card.
pub fn port_offset(node: &NodeView, port: &str, input: bool) -> Option<Vec2> {
    let row = node
        .ports
        .iter()
        .filter(|candidate| candidate.input == input)
        .position(|candidate| candidate.id == port)?;
    #[expect(clippy::cast_precision_loss, reason = "a node has few ports")]
    let y = BODY_TOP + (row as f32 + 0.5) * ROW_HEIGHT;
    Some(Vec2::new(if input { 0.0 } else { NODE_WIDTH }, y))
}

mod layout;
pub use layout::auto_layout;
mod geometry;
use geometry::{Geometry, wire_points, wire_visible};

/// What the person did on the canvas this frame.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CanvasResponse {
    /// The trigger whose button was pressed.
    pub trigger: Option<NodeId>,
    /// Nodes were let go at new places: [`Canvas::places`] is worth saving.
    pub moved: bool,
    /// The node that was clicked.
    pub selected: Option<NodeId>,
    /// A value was written on an input with its control on the card, typed as a
    /// person types it.
    pub set_input: Option<(NodeId, String, String)>,
    /// The selected nodes are to be copied, or cut, to the clipboard.
    pub copy: bool,
    pub cut: bool,
    /// Text was pasted on the canvas.
    pub paste: Option<String>,
    /// A node kind was dropped on the canvas from a list: the kind and where, in graph
    /// units.
    pub dropped: Option<(String, [f32; 2])>,
    /// A wire was drawn from an output to an input: the two ends.
    pub connect: Option<((NodeId, String), (NodeId, String))>,
    /// The wires at a port were taken off: the node, the port and whether it is an input.
    pub disconnect: Option<(NodeId, String, bool)>,
    /// The selected nodes were deleted.
    pub remove: Vec<NodeId>,
}

/// A problem shown on the canvas, on the node or wire it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    /// An error stops the graph from running; anything else is a warning.
    pub error: bool,
    /// What the problem says, as a person reads it; shown when the pointer is on it.
    pub text: String,
}

/// A wire by its two ends, output first: a node and a port name each.
pub type WireEnds = ((NodeId, String), (NodeId, String));

/// The problems the canvas shows on nodes and wires.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Marks {
    pub nodes: std::collections::HashMap<NodeId, Mark>,
    /// By the wire's two ends, output first.
    pub wires: std::collections::HashMap<WireEnds, Mark>,
}

impl Marks {
    /// Adds a problem to what is already marked on a node or wire: the texts are
    /// kept one per line, and an error outweighs a warning.
    pub fn add<K: std::hash::Hash + Eq>(
        marks: &mut std::collections::HashMap<K, Mark>,
        key: K,
        error: bool,
        text: String,
    ) {
        match marks.entry(key) {
            std::collections::hash_map::Entry::Occupied(mut held) => {
                let held = held.get_mut();
                held.error |= error;
                if !held.text.lines().any(|line| line == text) {
                    held.text.push('\n');
                    held.text.push_str(&text);
                }
            }
            std::collections::hash_map::Entry::Vacant(free) => {
                let _ = free.insert(Mark { error, text });
            }
        }
    }
}

/// A wire being drawn from an output, not yet let go.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Wiring {
    from: (NodeId, String),
    exec: bool,
}

/// The view of one graph: where it is scrolled to, how far it is zoomed and where its
/// nodes are.
#[derive(Debug, Clone)]
pub struct Canvas {
    /// The graph point at the top left corner of the canvas.
    origin: Pos2,
    zoom: f32,
    places: BTreeMap<NodeId, Pos2>,
    /// The selected nodes, and the one whose properties are shown when it is the only one.
    selection: BTreeSet<NodeId>,
    selected: Option<NodeId>,
    /// Where a box selection started, in graph units.
    boxing: Option<Pos2>,
    /// Where the pointer is, in graph units, while it is over the canvas.
    pointer: Option<Pos2>,
    /// A number being dragged on a card, shown before it is let go and written.
    dragging: Option<(NodeId, String, f64)>,
    /// Text being typed on a card, and whether its field has had the keys yet.
    typing: Option<(NodeId, String, String, bool)>,
    /// A node was dragged and not yet let go.
    moving: bool,
    /// Show the whole graph at the next frame.
    fit: bool,
    wiring: Option<Wiring>,
    /// Problems drawn on nodes and wires.
    marks: Marks,
    /// The size of the canvas when it was last shown, in points.
    size: Vec2,
}
impl Default for Canvas {
    fn default() -> Self {
        Self {
            origin: Pos2::new(-40.0, -40.0),
            zoom: 1.0,
            places: BTreeMap::new(),
            selection: BTreeSet::new(),
            selected: None,
            boxing: None,
            pointer: None,
            dragging: None,
            typing: None,
            moving: false,
            fit: true,
            wiring: None,
            marks: Marks::default(),
            size: Vec2::new(800.0, 600.0),
        }
    }
}

/// Colors of the canvas, taken from the surrounding egui style so it follows a light
/// or dark application.
struct Palette {
    error: Color32,
    warning: Color32,
    background: Color32,
    grid: Color32,
    card: Color32,
    header: Color32,
    entry_header: Color32,
    outline: Color32,
    selected: Color32,
    text: Color32,
    weak: Color32,
    exec: Color32,
}
impl Palette {
    fn problem(&self, error: bool) -> Color32 {
        if error { self.error } else { self.warning }
    }

    fn of(ui: &egui::Ui) -> Self {
        let visuals = ui.visuals();
        let dark = visuals.dark_mode;
        Self {
            error: visuals.error_fg_color,
            warning: visuals.warn_fg_color,
            background: visuals.extreme_bg_color,
            // Faint enough to sit behind the cards, clear enough to judge distances by.
            grid: visuals
                .widgets
                .noninteractive
                .bg_stroke
                .color
                .gamma_multiply(0.45),
            card: visuals.window_fill,
            header: visuals.faint_bg_color,
            entry_header: if dark {
                Color32::from_rgb(92, 58, 20)
            } else {
                Color32::from_rgb(255, 224, 178)
            },
            outline: visuals.widgets.noninteractive.bg_stroke.color,
            selected: visuals.selection.stroke.color,
            text: visuals.strong_text_color(),
            weak: visuals.weak_text_color(),
            exec: if dark {
                Color32::from_rgb(140, 214, 120)
            } else {
                Color32::from_rgb(56, 142, 60)
            },
        }
    }
}

/// The color of a value port, by the family of its type. The shape of the dot, not
/// the color, tells a value from a step.
fn type_color(ty: Option<&str>) -> Color32 {
    let name = ty.unwrap_or_default();
    let family = |words: &[&str]| words.iter().any(|word| name.starts_with(word));
    if family(&["core/string"]) {
        Color32::from_rgb(214, 120, 170)
    } else if family(&["core/bool"]) {
        Color32::from_rgb(214, 96, 96)
    } else if family(&[
        "core/i",
        "core/u",
        "core/f",
        "core/duration",
        "core/timestamp",
    ]) {
        Color32::from_rgb(96, 160, 222)
    } else if family(&["core/list", "core/map", "core/stream", "core/bytes"]) {
        Color32::from_rgb(170, 132, 214)
    } else {
        Color32::from_rgb(150, 160, 170)
    }
}

impl Canvas {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes the places of a graph's nodes: saved ones, and automatic ones for the
    /// rest. Call it when a document is opened or replaced.
    pub fn load(&mut self, graph: &GraphView) {
        self.places = auto_layout(graph);
        self.forget_missing();
        self.fit = true;
    }

    /// Follows an edit of the graph: nodes that are new take the place the document
    /// gives them or an automatic one, nodes that are gone are forgotten, and every
    /// other node stays where it is. The view is not moved.
    pub fn update(&mut self, graph: &GraphView) {
        // Most edits only change values or wires. Layout is needed only for a new
        // node without a saved position; existing nodes keep their dragged places.
        let needs_layout = graph
            .nodes
            .iter()
            .any(|node| !self.places.contains_key(&node.id) && node.position.is_none());
        let automatic = needs_layout.then(|| auto_layout(graph));
        let retained: std::collections::HashSet<_> =
            graph.nodes.iter().map(|node| node.id).collect();
        self.places.retain(|id, _| retained.contains(id));
        for node in &graph.nodes {
            if let std::collections::btree_map::Entry::Vacant(entry) = self.places.entry(node.id)
                && let Some(place) = node
                    .position
                    .map(|[x, y]| Pos2::new(x, y))
                    .or_else(|| automatic.as_ref()?.get(&node.id).copied())
            {
                let _ = entry.insert(place);
            }
        }
        self.forget_missing();
    }

    /// Follows an undo or redo: like [`Canvas::update`], but every node the document
    /// places goes back to that place. The view is not moved.
    pub fn restore(&mut self, graph: &GraphView) {
        self.update(graph);
        for node in &graph.nodes {
            if let Some([x, y]) = node.position {
                let _ = self.places.insert(node.id, Pos2::new(x, y));
            }
        }
    }

    /// Drops from the selection the nodes that are gone.
    fn forget_missing(&mut self) {
        let places = &self.places;
        self.selection.retain(|id| places.contains_key(id));
        if !self.selected.is_some_and(|id| self.selection.contains(&id)) {
            self.selected = None;
        }
    }

    /// The size of the canvas when it was last shown, in points.
    pub fn size(&self) -> [f32; 2] {
        [self.size.x, self.size.y]
    }

    /// The graph point in the middle of the canvas: where a new node goes.
    pub fn center(&self) -> [f32; 2] {
        let center = self.origin + self.size / self.zoom / 2.0;
        [center.x, center.y]
    }

    /// Selects a node alone, as clicking it does; `None` selects nothing.
    pub fn select(&mut self, node: Option<NodeId>) {
        self.select_many(node.as_slice());
    }

    /// Selects these nodes. With exactly one, it is also the one whose properties show.
    pub fn select_many(&mut self, nodes: &[NodeId]) {
        self.selection = nodes
            .iter()
            .copied()
            .filter(|id| self.places.contains_key(id))
            .collect();
        self.selected = match self.selection.len() {
            1 => self.selection.first().copied(),
            _ => None,
        };
    }

    /// Adds a node to the selection or takes it out, as clicking it with Ctrl does.
    pub fn toggle(&mut self, node: NodeId) {
        if !self.selection.remove(&node) && self.places.contains_key(&node) {
            let _ = self.selection.insert(node);
        }
        let nodes: Vec<NodeId> = self.selection.iter().copied().collect();
        self.select_many(&nodes);
    }

    /// The selected nodes.
    pub fn selection(&self) -> Vec<NodeId> {
        self.selection.iter().copied().collect()
    }

    /// Where pasted nodes go: under the pointer when it is over the canvas, otherwise
    /// the middle of what is shown.
    pub fn paste_place(&self) -> [f32; 2] {
        self.pointer
            .map_or_else(|| self.center(), |at| [at.x, at.y])
    }

    /// The nodes whose cards meet `area` (graph units).
    pub fn nodes_in(&self, graph: &GraphView, area: Rect) -> Vec<NodeId> {
        graph
            .nodes
            .iter()
            .filter(|node| {
                self.places.get(&node.id).is_some_and(|place| {
                    Rect::from_min_size(*place, node_size(node)).intersects(area)
                })
            })
            .map(|node| node.id)
            .collect()
    }

    /// The input a wire let go at `at` (graph units) lands on: the input of the same
    /// sort (step or value) whose row, on the left of its card, holds the point. A
    /// wire does not land on the node it started from.
    pub fn input_at(
        &self,
        graph: &GraphView,
        at: Pos2,
        exec: bool,
        from: NodeId,
    ) -> Option<(NodeId, String)> {
        for node in graph.nodes.iter().rev() {
            let Some(place) = self.places.get(&node.id).copied() else {
                continue;
            };
            if node.id == from {
                continue;
            }
            for port in node
                .ports
                .iter()
                .filter(|port| port.input && port.exec == exec)
            {
                let Some(offset) = port_offset(node, &port.id, true) else {
                    continue;
                };
                // From a little left of the dot to past the middle of the card.
                let row = Rect::from_min_max(
                    place + Vec2::new(-14.0, offset.y - ROW_HEIGHT / 2.0),
                    place + Vec2::new(NODE_WIDTH * 0.6, offset.y + ROW_HEIGHT / 2.0),
                );
                if row.contains(at) {
                    return Some((node.id, port.id.clone()));
                }
            }
        }
        None
    }

    /// Where each node is, for saving in the document's layout.
    pub fn places(&self) -> Vec<(NodeId, [f32; 2])> {
        self.places
            .iter()
            .map(|(id, place)| (*id, [place.x, place.y]))
            .collect()
    }

    pub fn selected(&self) -> Option<NodeId> {
        self.selected
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    /// Shows the whole graph at the next frame.
    pub fn fit(&mut self) {
        self.fit = true;
    }

    /// Returns to actual size, keeping the top left corner where it is.
    pub fn reset_zoom(&mut self) {
        self.zoom = 1.0;
    }

    /// Moves a node, as dragging it does. Unknown nodes are ignored.
    pub fn move_node(&mut self, node: NodeId, by: Vec2) {
        if let Some(place) = self.places.get_mut(&node) {
            *place += by;
        }
    }

    /// The zoom and origin that show `bounds` (graph units) inside a canvas of `size`
    /// points, with a margin.
    fn frame(bounds: Rect, size: Vec2) -> (f32, Pos2) {
        let margin = 40.0;
        let room = (size - Vec2::splat(2.0 * margin)).max(Vec2::splat(1.0));
        let wanted = bounds.size().max(Vec2::splat(1.0));
        let zoom = (room.x / wanted.x)
            .min(room.y / wanted.y)
            .clamp(*ZOOM_RANGE.start(), 1.0);
        // Centered: what is left over is split on both sides.
        let spare = size / zoom - wanted;
        (zoom, bounds.min - spare / 2.0)
    }

    fn bounds(&self, graph: &GraphView) -> Option<Rect> {
        graph
            .nodes
            .iter()
            .filter_map(|node| {
                Some(Rect::from_min_size(
                    *self.places.get(&node.id)?,
                    node_size(node),
                ))
            })
            .reduce(|all, rect| all.union(rect))
    }

    /// The problems drawn on nodes and wires.
    pub fn marks(&self) -> &Marks {
        &self.marks
    }

    /// The problems to draw on nodes and wires from now on.
    pub fn set_marks(&mut self, marks: Marks) {
        self.marks = marks;
    }

    /// Selects a node and moves the view so it is in the middle, at the same zoom.
    pub fn focus(&mut self, graph: &GraphView, node: NodeId) {
        let (Some(place), Some(shown)) = (
            self.places.get(&node).copied(),
            graph.nodes.iter().find(|found| found.id == node),
        ) else {
            return;
        };
        let middle = place + node_size(shown) / 2.0;
        self.origin = middle - self.size / (2.0 * self.zoom);
        self.select(Some(node));
    }

    /// Draws the graph into the space left in `ui` and applies panning, zooming,
    /// selecting and moving. `triggers` are the nodes that get a button to fire them.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        graph: &GraphView,
        triggers: &[NodeId],
    ) -> CanvasResponse {
        let mut done = CanvasResponse::default();
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let area = response.rect;
        self.size = area.size();
        let colors = Palette::of(ui);
        if self.fit {
            self.fit = false;
            if let Some(bounds) = self.bounds(graph) {
                (self.zoom, self.origin) = Self::frame(bounds, area.size());
            }
        }

        // Dragging the background scrolls; the wheel scrolls, and zooms with the
        // zoom gesture or key, about the pointer.
        // Dragging the background with Shift or Ctrl held draws a box that selects.
        let (shift, command) = ui.input(|input| (input.modifiers.shift, input.modifiers.command));
        let graph_point = |at: Pos2, origin: Pos2, zoom: f32| origin + (at - area.min) / zoom;
        if response.drag_started()
            && (shift || command)
            && let Some(at) = response.interact_pointer_pos()
        {
            self.boxing = Some(graph_point(at, self.origin, self.zoom));
        }
        if self.boxing.is_none() && response.dragged() {
            self.origin -= response.drag_delta() / self.zoom;
        }
        if response.drag_stopped()
            && let (Some(start), Some(at)) = (self.boxing.take(), response.interact_pointer_pos())
        {
            let end = graph_point(at, self.origin, self.zoom);
            let mut chosen = self.nodes_in(graph, Rect::from_two_pos(start, end));
            if shift || command {
                chosen.extend(self.selection.iter().copied());
            }
            self.select_many(&chosen);
        }
        if response.clicked() {
            self.select(None);
        }
        self.pointer = response
            .hover_pos()
            .map(|at| graph_point(at, self.origin, self.zoom));
        // A node kind dragged from a list and let go here.
        if let (Some(kind), Some(at)) = (
            response.dnd_release_payload::<String>(),
            ui.input(|input| input.pointer.interact_pos()),
        ) {
            let at = graph_point(at, self.origin, self.zoom);
            done.dropped = Some(((*kind).clone(), [at.x, at.y]));
        }
        if let Some(pointer) = response.hover_pos() {
            let (scroll, factor) =
                ui.input(|input| (input.smooth_scroll_delta, input.zoom_delta()));
            if (factor - 1.0).abs() > f32::EPSILON {
                let before = self.origin + (pointer - area.min) / self.zoom;
                self.zoom = (self.zoom * factor).clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
                self.origin = before - (pointer - area.min) / self.zoom;
            } else if scroll != Vec2::ZERO {
                self.origin -= scroll / self.zoom;
            }
        }

        let zoom = self.zoom;
        let origin = self.origin;
        let to_screen = |point: Pos2| area.min + (point - origin) * zoom;
        let geometry = Geometry::new(graph);

        painter.rect_filled(area, 0.0, colors.background);
        let step = GRID * zoom;
        if step >= 8.0 {
            let stroke = Stroke::new(1.0, colors.grid);
            let mut x = area.min.x - (origin.x * zoom).rem_euclid(step);
            while x < area.max.x {
                painter.line_segment([Pos2::new(x, area.min.y), Pos2::new(x, area.max.y)], stroke);
                x += step;
            }
            let mut y = area.min.y - (origin.y * zoom).rem_euclid(step);
            while y < area.max.y {
                painter.line_segment([Pos2::new(area.min.x, y), Pos2::new(area.max.x, y)], stroke);
                y += step;
            }
        }

        // Nodes are handled before anything is drawn, so a wire follows the node
        // being moved in the same frame.
        for node in &graph.nodes {
            let Some(place) = self.places.get(&node.id).copied() else {
                continue;
            };
            let rect = Rect::from_min_size(to_screen(place), geometry.size(node.id) * zoom);
            if !area.intersects(rect) {
                continue;
            }
            let touched = ui.interact(rect, response.id.with(node.id), Sense::click_and_drag());
            if touched.drag_started() && !self.selection.contains(&node.id) {
                self.select(Some(node.id));
            }
            if touched.dragged() {
                // Every selected node moves together.
                let by = touched.drag_delta() / zoom;
                for id in &self.selection {
                    if let Some(place) = self.places.get_mut(id) {
                        *place += by;
                    }
                }
                self.moving = true;
            }
            if touched.drag_stopped() && self.moving {
                self.moving = false;
                done.moved = true;
            }
            if touched.clicked() {
                if shift || command {
                    self.toggle(node.id);
                } else {
                    self.select(Some(node.id));
                }
                done.selected = self.selected;
            }
        }

        // Ports: dragging from an output draws a wire; the second mouse button on a
        // wired port takes its wires off.
        let grab = (PORT_RADIUS + 5.0) * zoom.max(0.6);
        for node in &graph.nodes {
            let Some(place) = self.places.get(&node.id).copied() else {
                continue;
            };
            if !area.expand(grab).intersects(Rect::from_min_size(
                to_screen(place),
                geometry.size(node.id) * zoom,
            )) {
                continue;
            }
            for port in &node.ports {
                let Some(port_geometry) = geometry.port(node.id, &port.id, port.input) else {
                    continue;
                };
                let at = to_screen(place + port_geometry.offset);
                if !area.contains(at) {
                    continue;
                }
                let touched = ui.interact(
                    Rect::from_center_size(at, Vec2::splat(2.0 * grab)),
                    response.id.with((node.id, port.id.as_str(), port.input)),
                    Sense::click_and_drag(),
                );
                if !port.input && touched.drag_started() {
                    self.wiring = Some(Wiring {
                        from: (node.id, port.id.clone()),
                        exec: port.exec,
                    });
                }
                if touched.secondary_clicked() && port.connected {
                    done.disconnect = Some((node.id, port.id.clone(), port.input));
                }
            }
        }
        let pointer = ui.input(|input| input.pointer.interact_pos());
        if let Some(wiring) = self.wiring.clone()
            && ui.input(|input| input.pointer.any_released())
        {
            if let Some(pointer) = pointer {
                let at = origin + (pointer - area.min) / zoom;
                done.connect = self
                    .input_at(graph, at, wiring.exec, wiring.from.0)
                    .map(|to| (wiring.from, to));
            }
            self.wiring = None;
        }
        // Keys, unless a text field has them: Delete removes the selection, Ctrl+A
        // selects everything, and copy, cut and paste go through the clipboard.
        if !ui.ctx().egui_wants_keyboard_input() {
            let (delete, all, events) = ui.input(|input| {
                (
                    input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace),
                    input.modifiers.command && input.key_pressed(egui::Key::A),
                    input.events.clone(),
                )
            });
            if delete && !self.selection.is_empty() {
                done.remove = self.selection();
                self.select(None);
            }
            if all {
                let every: Vec<NodeId> = graph.nodes.iter().map(|node| node.id).collect();
                self.select_many(&every);
            }
            for event in events {
                match event {
                    egui::Event::Copy => done.copy = !self.selection.is_empty(),
                    egui::Event::Cut => done.cut = !self.selection.is_empty(),
                    egui::Event::Paste(text) => done.paste = Some(text),
                    _ => {}
                }
            }
        }

        let anchor = |node: NodeId, port: &str, input: bool| -> Option<Pos2> {
            let place = *self.places.get(&node)?;
            Some(to_screen(place + geometry.port(node, port, input)?.offset))
        };
        for wire in &graph.wires {
            let (Some(from), Some(to)) = (
                anchor(wire.from.0, &wire.from.1, false),
                anchor(wire.to.0, &wire.to.1, true),
            ) else {
                continue;
            };
            let mark = self.marks.wires.get(&(wire.from.clone(), wire.to.clone()));
            let color = if let Some(mark) = mark {
                colors.problem(mark.error)
            } else if wire.exec {
                colors.exec
            } else {
                let ty = geometry
                    .port(wire.from.0, &wire.from.1, false)
                    .and_then(|port| port.ty);
                type_color(ty)
            };
            // The wire leaves and arrives level, bending more the further it goes.
            let points = wire_points(from, to, zoom);
            let width = if wire.exec { 2.5 } else { 2.0 } * zoom.max(0.6)
                + if mark.is_some() {
                    1.5 * zoom.max(0.6)
                } else {
                    0.0
                };
            if !wire_visible(&points, painter.clip_rect(), width) {
                continue;
            }
            painter.add(CubicBezierShape::from_points_stroke(
                points,
                false,
                Color32::TRANSPARENT,
                Stroke::new(width, color),
            ));
        }

        // The wire being drawn follows the pointer until it is let go.
        if let (Some(wiring), Some(to)) = (&self.wiring, pointer)
            && let Some(from) = anchor(wiring.from.0, &wiring.from.1, false)
        {
            let points = wire_points(from, to, zoom);
            let color = if wiring.exec {
                colors.exec
            } else {
                colors.selected
            };
            painter.add(CubicBezierShape::from_points_stroke(
                points,
                false,
                Color32::TRANSPARENT,
                Stroke::new(2.0 * zoom.max(0.6), color),
            ));
        }

        for node in &graph.nodes {
            let Some(place) = self.places.get(&node.id).copied() else {
                continue;
            };
            let rect = Rect::from_min_size(to_screen(place), geometry.size(node.id) * zoom);
            if !area.intersects(rect) {
                continue;
            }
            let selected = self.selection.contains(&node.id);
            let fire = triggers.contains(&node.id);
            let controls = zoom >= CONTROLS_ZOOM;
            let (pressed, slots) = draw_node(
                ui,
                &painter,
                node,
                rect,
                zoom,
                &colors,
                (selected, fire, controls),
                response.id,
            );
            if pressed {
                done.trigger = Some(node.id);
            }
            if let Some(mark) = self.marks.nodes.get(&node.id) {
                draw_mark(
                    ui,
                    &painter,
                    rect,
                    zoom,
                    &colors,
                    mark,
                    response.id.with(("mark", node.id)),
                );
            }
            for (port, slot) in slots {
                if area.contains_rect(slot)
                    && let Some(written) = self.control(ui, node, port, slot, response.id)
                {
                    done.set_input = Some(written);
                }
            }
        }
        // The box being drawn to select.
        if let (Some(start), Some(at)) = (self.boxing, self.pointer) {
            let shown = Rect::from_two_pos(to_screen(start), to_screen(at));
            painter.rect_filled(shown, 2.0, colors.selected.gamma_multiply(0.12));
            painter.rect_stroke(
                shown,
                2.0,
                Stroke::new(1.0, colors.selected),
                StrokeKind::Inside,
            );
        }
        // Where a dragged node kind would land.
        if response.dnd_hover_payload::<String>().is_some() {
            painter.rect_stroke(
                area,
                0.0,
                Stroke::new(2.0, colors.selected),
                StrokeKind::Inside,
            );
        }
        done
    }

    /// The control for one input's value on a card, in `slot`. Returns the value
    /// written, as typed.
    fn control(
        &mut self,
        ui: &mut egui::Ui,
        node: &NodeView,
        port: &PortView,
        slot: Rect,
        canvas: egui::Id,
    ) -> Option<(NodeId, String, String)> {
        let control = Control::of(port)?;
        let id = canvas.with(("control", node.id, port.id.as_str()));
        let place = |ui: &mut egui::Ui, add: &mut dyn FnMut(&mut egui::Ui) -> egui::Response| {
            ui.scope_builder(
                UiBuilder::new().max_rect(slot).id_salt(id).layout(
                    egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                ),
                |ui| add(ui),
            )
            .inner
        };
        let written = |text: String| Some((node.id, port.id.clone(), text));
        match control {
            Control::Toggle => {
                let mut on = port.value.as_deref() == Some("true");
                let changed =
                    place(ui, &mut |ui| ui.add(egui::Checkbox::without_text(&mut on))).changed();
                changed.then(|| written(on.to_string())).flatten()
            }
            Control::Whole | Control::Number => {
                let whole = control == Control::Whole;
                let held = self
                    .dragging
                    .as_ref()
                    .filter(|(at, name, _)| *at == node.id && *name == port.id)
                    .map(|(_, _, value)| *value);
                let mut value = held.unwrap_or_else(|| {
                    port.value
                        .as_deref()
                        .and_then(|text| text.parse().ok())
                        .unwrap_or(0.0)
                });
                // An input with no value says so, rather than showing a 0 it does not have.
                let unset = held.is_none() && port.value.is_none();
                let touched = place(ui, &mut |ui| {
                    let drag =
                        egui::DragValue::new(&mut value).speed(if whole { 0.2 } else { 0.01 });
                    let drag = if whole { drag.fixed_decimals(0) } else { drag };
                    let drag = if unset {
                        drag.custom_formatter(|_, _| "—".to_owned())
                    } else {
                        drag
                    };
                    ui.add(drag)
                });
                if touched.dragged() {
                    self.dragging = Some((node.id, port.id.clone(), value));
                    return None;
                }
                if touched.drag_stopped() || touched.changed() {
                    self.dragging = None;
                    let text = if whole {
                        format!("{value:.0}")
                    } else {
                        value.to_string()
                    };
                    return written(text);
                }
                None
            }
            Control::Text => {
                let typing = self
                    .typing
                    .as_mut()
                    .filter(|(at, name, _, _)| *at == node.id && *name == port.id);
                let Some((_, _, typed, focused)) = typing else {
                    // The value shows as a pill; a double click types a new one.
                    let touched = ui.interact(slot, id, Sense::click());
                    if touched.double_clicked() {
                        let shown = port.value.clone().unwrap_or_default();
                        let typed = serde_json::from_str::<String>(&shown).unwrap_or(shown);
                        self.typing = Some((node.id, port.id.clone(), typed, false));
                    }
                    return None;
                };
                let field = place(ui, &mut |ui| ui.add(egui::TextEdit::singleline(typed)));
                if !*focused {
                    field.request_focus();
                    *focused = true;
                }
                let entered =
                    field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                if field.lost_focus() {
                    let text = typed.clone();
                    self.typing = None;
                    return entered.then(|| written(text)).flatten();
                }
                None
            }
        }
    }
}

/// Outlines a card that has a problem and puts a badge on its corner; the pointer on
/// the badge shows what the problem says.
fn draw_mark(
    ui: &egui::Ui,
    painter: &egui::Painter,
    rect: Rect,
    zoom: f32,
    colors: &Palette,
    mark: &Mark,
    id: egui::Id,
) {
    let color = colors.problem(mark.error);
    painter.rect_stroke(
        rect.expand(2.0 * zoom),
        10.0 * zoom,
        Stroke::new(2.0 * zoom.max(0.6), color),
        StrokeKind::Outside,
    );
    let radius = 8.0 * zoom.max(0.75);
    let center = Pos2::new(rect.max.x - radius * 0.4, rect.min.y + radius * 0.4);
    painter.circle_filled(center, radius, color);
    let _ = painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        "!",
        egui::FontId::proportional(radius * 1.4),
        Color32::WHITE,
    );
    let badge = Rect::from_center_size(center, Vec2::splat(radius * 2.0));
    let _ = ui
        .interact(badge, id, Sense::hover())
        .on_hover_text(mark.text.as_str());
}

/// Draws one card. Returns whether its trigger button was pressed.
/// `flags` are whether the card is selected, has a trigger button and shows controls
/// for its values. Also returns where each input's control goes.
#[expect(clippy::too_many_arguments, reason = "one card's drawing state")]
fn draw_node<'a>(
    ui: &egui::Ui,
    painter: &egui::Painter,
    node: &'a NodeView,
    rect: Rect,
    zoom: f32,
    colors: &Palette,
    (selected, fire, controls): (bool, bool, bool),
    canvas: egui::Id,
) -> (bool, Vec<(&'a PortView, Rect)>) {
    let radius = 8.0 * zoom;
    let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), HEADER_HEIGHT * zoom));
    painter.rect_filled(rect, radius, colors.card);
    // The header carries the node family's color: a tint, and a line along its foot,
    // so nodes of one package are told apart at a glance, even zoomed out.
    let family = family_color(&node.definition);
    let header_color = if node.entry {
        colors.entry_header
    } else {
        mix(colors.header, family, 0.18)
    };
    painter.rect_filled(
        header,
        egui::CornerRadius {
            nw: radius as u8,
            ne: radius as u8,
            sw: 0,
            se: 0,
        },
        header_color,
    );
    painter.line_segment(
        [
            header.left_bottom() + Vec2::new(1.0, 0.0),
            header.right_bottom() - Vec2::new(1.0, 0.0),
        ],
        Stroke::new(2.0 * zoom, family),
    );
    let outline = if selected {
        Stroke::new(2.0, colors.selected)
    } else {
        Stroke::new(1.0, colors.outline)
    };
    painter.rect_stroke(rect, radius, outline, StrokeKind::Outside);

    // Text smaller than this is a smudge: far out, a card is only its shape.
    let readable = zoom >= 0.45;
    let mut pressed = false;
    if readable {
        let title = FontId::proportional(14.5 * zoom);
        let small = FontId::proportional(11.5 * zoom);
        // Long titles stay inside the header, clear of the trigger button.
        let reserved = if fire { 80.0 * zoom } else { 8.0 * zoom };
        let clip = Rect::from_min_max(header.min, header.max - Vec2::new(reserved, 0.0));
        let header_painter = painter.with_clip_rect(clip.intersect(painter.clip_rect()));
        let left = header.left_center() + Vec2::new(CARD_PADDING * zoom, 0.0);
        let named = header_painter.text(left, Align2::LEFT_CENTER, &node.name, title, colors.text);
        // What the node is set to (its variable), else what kind of node it is when its
        // name says something else.
        let aside = node
            .detail
            .as_deref()
            .or_else(|| (node.name != node.kind).then_some(node.kind.as_str()));
        if let Some(aside) = aside {
            // What it is set to reads as text; what kind it is, quieter.
            let color = if node.detail.is_some() {
                colors.text.gamma_multiply(0.8)
            } else {
                colors.weak
            };
            let _ = header_painter.text(
                Pos2::new(named.max.x + 8.0 * zoom, header.center().y),
                Align2::LEFT_CENTER,
                aside,
                small.clone(),
                color,
            );
        }
        if node.ports.is_empty() {
            let _ = painter.text(
                Pos2::new(
                    rect.center().x,
                    rect.min.y + (BODY_TOP + ROW_HEIGHT / 2.0) * zoom,
                ),
                Align2::CENTER_CENTER,
                "Set it up in Properties",
                small.clone(),
                colors.weak,
            );
        }
        if fire {
            let size = Vec2::new(64.0, 22.0) * zoom;
            let button = Rect::from_center_size(
                header.right_center() - Vec2::new(size.x / 2.0 + 8.0 * zoom, 0.0),
                size,
            );
            let touched = ui.interact(button, canvas.with(("trigger", node.id)), Sense::click());
            let fill = if touched.hovered() {
                colors.selected
            } else {
                colors.exec
            };
            painter.rect_filled(button, 5.0 * zoom, fill);
            let _ = painter.text(
                button.center(),
                Align2::CENTER_CENTER,
                "Trigger",
                small.clone(),
                colors.card,
            );
            pressed = touched.clicked();
        }
    }

    // Where an input's value may reach: the middle of the card when an output shares
    // its row, else the card's inner right edge.
    let shared_rows = node.ports.iter().filter(|port| !port.input).count();
    // The values of a card's inputs start in one column, after its longest input name,
    // but never past the middle of the card.
    let names = FontId::proportional(12.5 * zoom);
    let longest = node
        .ports
        .iter()
        .filter(|port| port.input && !port.exec && readable)
        .map(|port| {
            painter
                .layout_no_wrap(crate::port_label(&port.id), names.clone(), colors.text)
                .size()
                .x
        })
        .fold(0.0, f32::max);
    let value_start =
        (rect.min.x + (CARD_PADDING + 10.0) * zoom + longest).min(rect.center().x - 10.0 * zoom);
    let (mut inputs, mut outputs) = (0usize, 0usize);
    let mut slots = Vec::new();
    for port in &node.ports {
        let row = if port.input {
            &mut inputs
        } else {
            &mut outputs
        };
        let index = *row;
        *row += 1;
        #[expect(clippy::cast_precision_loss, reason = "a node has few ports")]
        let y = rect.min.y + (BODY_TOP + (index as f32 + 0.5) * ROW_HEIGHT) * zoom;
        let x = if port.input { rect.min.x } else { rect.max.x };
        let value_end = if index < shared_rows {
            rect.center().x + 6.0 * zoom
        } else {
            rect.max.x - CARD_PADDING * zoom
        };
        // Toggles and numbers are drawn by their control instead of a field.
        let control = Control::of(port).filter(|_| controls && readable);
        let style = PortStyle {
            readable,
            field: !matches!(
                control,
                Some(Control::Toggle | Control::Whole | Control::Number)
            ),
            hinted: control.is_none(),
            value_start,
            value_end,
        };
        if let Some(label) = draw_port(painter, port, Pos2::new(x, y), zoom, colors, style)
            && let Some(control) = control
        {
            let height = FIELD_HEIGHT * zoom;
            let left = (label.max.x + 8.0 * zoom).max(value_start);
            let width = match control {
                Control::Toggle => height,
                Control::Whole | Control::Number => {
                    (value_end - left).clamp(36.0 * zoom, CONTROL_WIDTH * zoom)
                }
                Control::Text => (value_end - left).max(36.0 * zoom),
            };
            let slot =
                Rect::from_min_size(Pos2::new(left, y - height / 2.0), Vec2::new(width, height));
            slots.push((port, slot));
        }
    }
    (pressed, slots)
}

/// How one port of a card is drawn.
#[derive(Debug, Clone, Copy)]
struct PortStyle {
    /// Big enough to read: names and values are drawn.
    readable: bool,
    /// An input's value is drawn as a field (no control draws it).
    field: bool,
    /// A short type is shown beside a value port nothing feeds.
    hinted: bool,
    /// Where an input's value starts, unless its name is longer.
    value_start: f32,
    /// The farthest right an input's value reaches.
    value_end: f32,
}

/// Draws a port's dot and name, an input's value as a field, and a short type
/// beside a value port nothing feeds. Returns where an input's name is.
fn draw_port(
    painter: &egui::Painter,
    port: &PortView,
    at: Pos2,
    zoom: f32,
    colors: &Palette,
    style: PortStyle,
) -> Option<Rect> {
    let radius = PORT_RADIUS * zoom;
    let color = if port.exec {
        colors.exec
    } else {
        type_color(port.ty.as_deref())
    };
    // Every port is a dot on the card's edge: filled once wired, a ring before. A step
    // is told from a value by its color and by the arrow beside its name.
    if port.connected {
        painter.circle_filled(at, radius, color);
    } else {
        painter.circle_filled(at, radius, colors.card);
        painter.circle_stroke(at, radius, Stroke::new(1.5, color));
    }
    if !style.readable {
        return None;
    }
    let font = FontId::proportional(12.5 * zoom);
    let hint = FontId::proportional(10.5 * zoom);
    let gap = CARD_PADDING * zoom;
    let name = step_label(port).unwrap_or_else(|| crate::port_label(&port.id));
    if port.exec {
        // Steps read as on a flow chart: the way in is a tinted row inside the card,
        // the ways on are named with an arrow toward their dot.
        let arrow = 11.0 * zoom;
        if port.input {
            let text = painter.layout_no_wrap(name, font, colors.text);
            let height = FIELD_HEIGHT * zoom;
            let row = Rect::from_min_size(
                Pos2::new(at.x + gap - 4.0 * zoom, at.y - height / 2.0),
                Vec2::new(text.size().x + arrow + 20.0 * zoom, height),
            );
            painter.rect_filled(row, 5.0 * zoom, color.gamma_multiply(0.16));
            draw_arrow(
                painter,
                Pos2::new(row.min.x + 7.0 * zoom, at.y),
                arrow,
                color,
                zoom,
            );
            let place = Pos2::new(row.min.x + 13.0 * zoom + arrow, at.y - text.size().y / 2.0);
            painter.galley(place, text, colors.text);
            return Some(row);
        }
        let start = at.x - gap - arrow;
        draw_arrow(painter, Pos2::new(start, at.y), arrow, color, zoom);
        let _ = painter.text(
            Pos2::new(start - 6.0 * zoom, at.y),
            Align2::RIGHT_CENTER,
            &name,
            font,
            colors.text,
        );
        return None;
    }
    // What a value port takes or gives, when nothing on the card says it already.
    let type_hint = (style.hinted && !port.connected && !(port.input && port.value.is_some()))
        .then(|| port.ty.as_deref().map(short_type))
        .flatten();
    if !port.input {
        let label = painter.text(
            at - Vec2::new(gap, 0.0),
            Align2::RIGHT_CENTER,
            &name,
            font,
            colors.text,
        );
        if let Some(ty) = type_hint {
            let _ = painter.text(
                Pos2::new(label.min.x - 6.0 * zoom, at.y),
                Align2::RIGHT_CENTER,
                ty,
                hint,
                colors.weak,
            );
        }
        return None;
    }
    let label = painter.text(
        at + Vec2::new(gap, 0.0),
        Align2::LEFT_CENTER,
        &name,
        font.clone(),
        colors.text,
    );
    if let Some(ty) = type_hint.filter(|_| style.field) {
        let _ = painter.text(
            Pos2::new(label.max.x + 6.0 * zoom, at.y),
            Align2::LEFT_CENTER,
            ty,
            hint,
            colors.weak,
        );
    }
    // The value written on an input that no wire feeds, in a field that ends where
    // the row's value may reach, shortened with an ellipsis when it is longer.
    // An input that can be typed into and has no value shows an empty field.
    let shown = match port.value.as_deref() {
        Some(value) => Some((field_text(value), colors.text)),
        None if !style.hinted => Some(("—".to_owned(), colors.weak)),
        None => None,
    };
    if let Some((value, ink)) = shown.filter(|_| !port.connected && style.field) {
        let left = (label.max.x + 8.0 * zoom).max(style.value_start);
        let room = style.value_end - left;
        let pad = 6.0 * zoom;
        if room > 3.0 * pad {
            let mut job = egui::text::LayoutJob::simple_singleline(value, font, ink);
            job.wrap = egui::text::TextWrapping {
                max_width: room - 2.0 * pad,
                max_rows: 1,
                break_anywhere: true,
                overflow_character: Some('…'),
            };
            let text = painter.layout_job(job);
            let height = FIELD_HEIGHT * zoom;
            let field = Rect::from_min_size(
                Pos2::new(left, at.y - height / 2.0),
                Vec2::new(text.size().x + 2.0 * pad, height),
            );
            painter.rect_filled(field, 4.0 * zoom, colors.background);
            painter.rect_stroke(
                field,
                4.0 * zoom,
                Stroke::new(1.0, colors.outline),
                StrokeKind::Inside,
            );
            let place = Pos2::new(field.min.x + pad, at.y - text.size().y / 2.0);
            painter.galley(place, text, ink);
        }
    }
    Some(label)
}

/// A value as its field on a card shows it: text without its quotes, others as
/// written.
fn field_text(value: &str) -> String {
    serde_json::from_str::<String>(value).unwrap_or_else(|_| value.to_owned())
}

/// What a card calls the way into a node and the way on from it: "Enter" and "Exit".
/// Only the words shown change; the ports keep their IDs `in` and `then`.
fn step_label(port: &PortView) -> Option<String> {
    match (port.exec, port.input, port.id.as_str()) {
        (true, true, "in") => Some("Enter".to_owned()),
        (true, false, "then") => Some("Exit".to_owned()),
        _ => None,
    }
}

/// An arrow pointing right, `width` long, from `from` (its tail, at mid height).
fn draw_arrow(painter: &egui::Painter, from: Pos2, width: f32, color: Color32, zoom: f32) {
    let stroke = Stroke::new(1.6 * zoom, color);
    let tip = from + Vec2::new(width, 0.0);
    let head = width * 0.4;
    painter.line_segment([from, tip], stroke);
    painter.line_segment([tip, tip + Vec2::new(-head, -head)], stroke);
    painter.line_segment([tip, tip + Vec2::new(-head, head)], stroke);
}

/// `base` moved toward `tint` by `amount` (0 keeps `base`, 1 is `tint`).
fn mix(base: Color32, tint: Color32, amount: f32) -> Color32 {
    let channel = |a: u8, b: u8| {
        let value = f32::from(a) + (f32::from(b) - f32::from(a)) * amount;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a blend of two bytes stays a byte"
        )]
        let byte = value.round().clamp(0.0, 255.0) as u8;
        byte
    };
    Color32::from_rgb(
        channel(base.r(), tint.r()),
        channel(base.g(), tint.g()),
        channel(base.b(), tint.b()),
    )
}

/// A type in a few letters for a card: "text", "number", "list<…>"; a type not found
/// yet (`$T`) is "any".
fn short_type(ty: &str) -> String {
    if ty.starts_with('$') {
        return "any".to_owned();
    }
    let name = crate::type_name(ty);
    let mut short: String = name.chars().take(14).collect();
    if short.len() < name.len() {
        short.push('…');
    }
    short
}

/// The color of a node's family: the namespace of its kind (`core`, `org.orange.math`),
/// so every node of one package shares it.
fn family_color(definition: &str) -> Color32 {
    let family = definition.split('/').next().unwrap_or(definition);
    if family == "core" {
        return Color32::from_rgb(232, 140, 48);
    }
    // FNV-1a: stable across runs and platforms, unlike the standard hasher.
    let hash = family.bytes().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    });
    let [a, b, c, _] = hash.to_le_bytes();
    // Mid-tone channels read on both light and dark cards.
    let tone = |byte: u8| 70 + byte % 140;
    Color32::from_rgb(tone(a), tone(b), tone(c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use orange_host::WireView;

    fn id(n: u32) -> NodeId {
        format!("10000000-0000-4000-8000-{n:012x}").parse().unwrap()
    }
    fn port(name: &str, input: bool, exec: bool) -> PortView {
        PortView {
            id: name.into(),
            input,
            exec,
            ty: (!exec).then(|| "core/i64@1".to_owned()),
            value: None,
            connected: false,
        }
    }
    fn node(n: u32, ports: Vec<PortView>) -> NodeView {
        NodeView {
            id: id(n),
            name: format!("node {n}"),
            kind: "Kind".into(),
            definition: "org.example/kind".into(),
            detail: None,
            entry: n == 1,
            ports,
            position: None,
            settings: serde_json::json!({}),
            schema: None,
            references: Vec::new(),
            types: Vec::new(),
            variadic: Vec::new(),
        }
    }
    fn wire(from: u32, to: u32) -> WireView {
        WireView {
            from: (id(from), "then".into()),
            to: (id(to), "in".into()),
            exec: true,
        }
    }
    /// 1 → 2 → 3, and 1 → 3: three columns.
    fn graph() -> GraphView {
        let step = || vec![port("in", true, true), port("then", false, true)];
        GraphView {
            nodes: vec![
                node(1, vec![port("then", false, true)]),
                node(2, step()),
                node(
                    3,
                    vec![
                        port("in", true, true),
                        port("a", true, false),
                        port("b", true, false),
                    ],
                ),
            ],
            wires: vec![wire(1, 2), wire(2, 3), wire(1, 3)],
        }
    }

    #[test]
    fn a_card_is_as_tall_as_its_longer_side_and_ports_sit_on_its_edges() {
        let graph = graph();
        assert_eq!(
            node_size(&graph.nodes[0]).y,
            BODY_TOP + ROW_HEIGHT + FOOTER_HEIGHT
        );
        assert_eq!(
            node_size(&graph.nodes[2]).y,
            BODY_TOP + 3.0 * ROW_HEIGHT + FOOTER_HEIGHT
        );
        let third = &graph.nodes[2];
        assert_eq!(
            port_offset(third, "b", true),
            Some(Vec2::new(0.0, BODY_TOP + 2.5 * ROW_HEIGHT))
        );
        assert_eq!(
            port_offset(&graph.nodes[1], "then", false),
            Some(Vec2::new(NODE_WIDTH, BODY_TOP + 0.5 * ROW_HEIGHT))
        );
        // A port is looked for on its own side only.
        assert_eq!(port_offset(third, "b", false), None);
    }

    #[test]
    fn indexed_geometry_preserves_port_sides_and_refreshes_with_the_view() {
        let mut graph = graph();
        graph.nodes[1].ports.push(port("in", false, false));
        let geometry = Geometry::new(&graph);
        for node in &graph.nodes {
            assert_eq!(geometry.size(node.id), node_size(node));
            for port in &node.ports {
                let found = geometry.port(node.id, &port.id, port.input).unwrap();
                assert_eq!(Some(found.offset), port_offset(node, &port.id, port.input));
                assert_eq!(found.ty, port.ty.as_deref());
            }
        }
        assert!(geometry.port(id(99), "in", true).is_none());
        drop(geometry);
        graph.nodes[1].ports.reverse();
        graph.nodes[1].ports[0].ty = Some("core/string@1".into());
        let geometry = Geometry::new(&graph);
        assert_eq!(
            geometry.port(id(2), "in", false).unwrap().ty,
            Some("core/string@1")
        );
        assert_eq!(
            Some(geometry.port(id(2), "in", false).unwrap().offset),
            port_offset(&graph.nodes[1], "in", false)
        );
    }

    #[test]
    fn layout_is_order_independent_and_handles_cycles_missing_ends_and_saved_places() {
        let mut graph = graph();
        let expected = auto_layout(&graph);
        graph.wires.reverse();
        assert_eq!(auto_layout(&graph), expected);
        graph.wires.extend([wire(99, 1), wire(3, 98)]);
        assert_eq!(auto_layout(&graph), expected);
        graph.wires.extend([wire(2, 2), wire(3, 1)]);
        let cyclic = auto_layout(&graph);
        graph.wires.reverse();
        assert_eq!(auto_layout(&graph), cyclic);
        assert_eq!(cyclic.len(), graph.nodes.len());
        assert!(cyclic.values().all(|point| point.is_finite()));
        for (n, node) in graph.nodes.iter_mut().enumerate() {
            node.position = Some([n as f32 * -100.0, 50.0]);
        }
        let saved = auto_layout(&graph);
        for node in &graph.nodes {
            let place = saved[&node.id];
            assert_eq!([place.x, place.y], node.position.unwrap());
        }
    }

    #[test]
    fn updating_values_keeps_dragged_places_and_add_remove_updates_selection() {
        let mut graph = graph();
        let mut canvas = Canvas::new();
        canvas.load(&graph);
        canvas.move_node(id(2), Vec2::new(-500.0, 70.0));
        canvas.select(Some(id(2)));
        let places = canvas.places();
        graph.nodes[1].ports[0].value = Some("changed".into());
        canvas.update(&graph);
        assert_eq!(canvas.places(), places);
        graph.nodes.remove(1);
        let mut added = node(4, vec![]);
        added.position = Some([-100.0, -200.0]);
        graph.nodes.push(added);
        canvas.update(&graph);
        assert_eq!(canvas.selected(), None);
        assert!(!canvas.places.contains_key(&id(2)));
        assert_eq!(canvas.places[&id(4)], Pos2::new(-100.0, -200.0));
        graph.nodes.push(node(5, vec![]));
        canvas.update(&graph);
        assert!(canvas.places.contains_key(&id(5)));
    }

    #[test]
    fn nodes_without_a_place_are_laid_out_along_the_wires_and_saved_places_are_kept() {
        let mut graph = graph();
        let places = auto_layout(&graph);
        let column = NODE_WIDTH + LAYOUT_GAP.x;
        assert_eq!(places[&id(1)], Pos2::new(0.0, 0.0));
        assert_eq!(places[&id(2)], Pos2::new(column, 0.0));
        // Fed by 1 and by 2: one past the furthest.
        assert_eq!(places[&id(3)], Pos2::new(2.0 * column, 0.0));

        graph.nodes[1].position = Some([500.0, -20.0]);
        assert_eq!(auto_layout(&graph)[&id(2)], Pos2::new(500.0, -20.0));

        // A cycle still gets places, and two nodes of one column do not overlap.
        graph.wires.push(wire(3, 1));
        assert_eq!(auto_layout(&graph).len(), 3);
        let fan = GraphView {
            nodes: vec![node(1, vec![]), node(2, vec![]), node(3, vec![])],
            wires: vec![wire(1, 2), wire(1, 3)],
        };
        let places = auto_layout(&fan);
        assert_eq!(places[&id(2)].x, places[&id(3)].x);
        assert!(places[&id(3)].y >= places[&id(2)].y + node_size(&fan.nodes[1]).y);
    }

    #[test]
    fn a_graph_is_framed_whole_and_a_moved_node_reports_its_place() {
        let graph = graph();
        let mut canvas = Canvas::new();
        canvas.load(&graph);
        // A graph wider than the canvas is zoomed out until it fits, never in.
        let bounds = canvas.bounds(&graph).unwrap();
        let (zoom, origin) = Canvas::frame(bounds, Vec2::new(500.0, 400.0));
        assert!(zoom < 1.0 && zoom >= *ZOOM_RANGE.start());
        assert!(origin.x <= bounds.min.x && origin.y <= bounds.min.y);
        assert!((bounds.max.x - origin.x) * zoom <= 500.0);
        let (roomy, _) = Canvas::frame(bounds, Vec2::new(5_000.0, 4_000.0));
        assert_eq!(roomy, 1.0);

        canvas.move_node(id(2), Vec2::new(12.0, -7.0));
        canvas.move_node(id(99), Vec2::new(1.0, 1.0));
        let places = canvas.places();
        let second = places.iter().find(|(node, _)| *node == id(2)).unwrap().1;
        assert_eq!(second, [NODE_WIDTH + LAYOUT_GAP.x + 12.0, -7.0]);
        assert_eq!(places.len(), 3);
    }

    #[test]
    fn a_wire_lands_on_the_input_row_of_its_own_sort_and_an_edit_keeps_places() {
        let graph = graph();
        let mut canvas = Canvas::new();
        canvas.load(&graph);
        let third = canvas.places[&id(3)];
        let row = |n: f32| third + Vec2::new(20.0, BODY_TOP + (n + 0.5) * ROW_HEIGHT);
        // Row 0 is the step way in; rows 1 and 2 are the values a and b.
        assert_eq!(
            canvas.input_at(&graph, row(0.0), true, id(1)),
            Some((id(3), "in".into()))
        );
        assert_eq!(canvas.input_at(&graph, row(0.0), false, id(1)), None);
        assert_eq!(
            canvas.input_at(&graph, row(2.0), false, id(1)),
            Some((id(3), "b".into()))
        );
        // Not on the node the wire started from, not on the right of the card, not on
        // empty canvas.
        assert_eq!(canvas.input_at(&graph, row(0.0), true, id(3)), None);
        let right = third + Vec2::new(NODE_WIDTH - 10.0, BODY_TOP + 0.5 * ROW_HEIGHT);
        assert_eq!(canvas.input_at(&graph, right, true, id(1)), None);
        assert_eq!(
            canvas.input_at(&graph, Pos2::new(-500.0, -500.0), true, id(1)),
            None
        );

        // After an edit, kept nodes stay where they were put, a new one gets a place
        // and a removed one is forgotten along with the selection of it.
        canvas.move_node(id(2), Vec2::new(40.0, 40.0));
        let moved = canvas.places[&id(2)];
        canvas.select(Some(id(1)));
        let mut edited = graph.clone();
        edited.nodes.remove(0);
        edited.wires.retain(|wire| wire.from.0 != id(1));
        let mut added = node(4, vec![]);
        added.position = Some([900.0, 900.0]);
        edited.nodes.push(added);
        canvas.update(&edited);
        assert_eq!(canvas.places[&id(2)], moved);
        assert_eq!(canvas.places[&id(4)], Pos2::new(900.0, 900.0));
        assert!(!canvas.places.contains_key(&id(1)));
        assert_eq!(canvas.selected(), None);
        // A new node goes to the middle of what is shown.
        let [x, y] = Canvas::new().center();
        assert_eq!([x, y], [-40.0 + 400.0, -40.0 + 300.0]);
    }

    /// One frame of the canvas with these input events.
    fn frame(canvas: &mut Canvas, graph: &GraphView, events: Vec<egui::Event>) -> CanvasResponse {
        let context = egui::Context::default();
        // Held keys are part of the input, not only of each key event.
        let modifiers = events
            .iter()
            .find_map(|event| match event {
                egui::Event::Key { modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or_default();
        let mut held = vec![egui::Event::ModifiersChanged(modifiers)];
        held.extend(events);
        let input = egui::RawInput {
            events: held,
            ..egui::RawInput::default()
        };
        let mut response = CanvasResponse::default();
        let output = context.run_ui(input, |ui| response = canvas.show(ui, graph, &[]));
        output.drop_without_applying_deltas();
        response
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn several_nodes_are_selected_by_box_click_and_keys_and_go_together() {
        let graph = graph();
        let mut canvas = Canvas::new();
        canvas.load(&graph);
        // A box over the first two columns takes nodes 1 and 2, not 3.
        let first = canvas.places[&id(1)];
        let second = canvas.places[&id(2)];
        let area = Rect::from_two_pos(first, second + Vec2::new(10.0, 10.0));
        assert_eq!(canvas.nodes_in(&graph, area), vec![id(1), id(2)]);
        canvas.select_many(&[id(1), id(2)]);
        // Several selected: none is the one whose properties show.
        assert_eq!(canvas.selected(), None);
        canvas.toggle(id(2));
        assert_eq!(
            (canvas.selection(), canvas.selected()),
            (vec![id(1)], Some(id(1)))
        );
        canvas.toggle(id(3));
        assert_eq!(canvas.selection(), vec![id(1), id(3)]);

        // Ctrl+A selects all, Delete removes all of them, copy asks for the selection.
        let all = frame(
            &mut canvas,
            &graph,
            vec![key(egui::Key::A, egui::Modifiers::COMMAND)],
        );
        assert!(all.remove.is_empty());
        assert_eq!(canvas.selection().len(), 3);
        let copied = frame(&mut canvas, &graph, vec![egui::Event::Copy]);
        assert!(copied.copy && !copied.cut);
        let pasted = frame(&mut canvas, &graph, vec![egui::Event::Paste("clip".into())]);
        assert_eq!(pasted.paste.as_deref(), Some("clip"));
        let deleted = frame(
            &mut canvas,
            &graph,
            vec![key(egui::Key::Delete, egui::Modifiers::NONE)],
        );
        assert_eq!(deleted.remove, vec![id(1), id(2), id(3)]);
        assert!(canvas.selection().is_empty());
        // With nothing selected there is nothing to copy.
        assert!(!frame(&mut canvas, &graph, vec![egui::Event::Copy]).copy);
        // Paste goes to the middle of the view when the pointer is elsewhere.
        assert_eq!(canvas.paste_place(), canvas.center());
    }

    #[test]
    fn inputs_with_values_get_controls_by_type() {
        let port = |ty: &str, connected: bool| PortView {
            id: "v".into(),
            input: true,
            exec: false,
            ty: Some(ty.into()),
            value: None,
            connected,
        };
        assert_eq!(
            Control::of(&port("core/bool@1", false)),
            Some(Control::Toggle)
        );
        assert_eq!(
            Control::of(&port("core/i64@1", false)),
            Some(Control::Whole)
        );
        assert_eq!(
            Control::of(&port("core/f64@1", false)),
            Some(Control::Number)
        );
        assert_eq!(
            Control::of(&port("core/string@1", false)),
            Some(Control::Text)
        );
        // A wired input and a type without a control get none.
        assert_eq!(Control::of(&port("core/i64@1", true)), None);
        assert_eq!(Control::of(&port("core/list@1<core/i64@1>", false)), None);
    }

    #[test]
    fn cards_name_types_shortly_and_color_families_alike() {
        assert_eq!(short_type("core/string@1"), "text");
        assert_eq!(short_type("$T"), "any");
        // The way in and the way on read as Enter and Exit; other ports keep their names.
        assert_eq!(
            step_label(&port("in", true, true)).as_deref(),
            Some("Enter")
        );
        assert_eq!(
            step_label(&port("then", false, true)).as_deref(),
            Some("Exit")
        );
        assert_eq!(step_label(&port("in", true, false)), None);
        assert_eq!(step_label(&port("completed", false, true)), None);
        assert_eq!(short_type("core/list@1<core/string@1>"), "list<string>");
        assert_eq!(short_type("org.example.geo/point@1"), "org.example.ge…");
        // Every node of one package shares a color; core has its own.
        assert_eq!(
            family_color("org.orange.math/add"),
            family_color("org.orange.math/sub")
        );
        assert_ne!(
            family_color("org.orange.math/add"),
            family_color("core/fork")
        );
        // A node with no ports yet keeps a row for its hint.
        let empty = node(9, Vec::new());
        assert_eq!(node_size(&empty).y, BODY_TOP + ROW_HEIGHT + FOOTER_HEIGHT);
    }

    #[test]
    fn the_canvas_draws_cards_and_wires_without_a_window() {
        let graph = graph();
        let mut canvas = Canvas::new();
        canvas.load(&graph);
        let context = egui::Context::default();
        let mut response = CanvasResponse::default();
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            response = canvas.show(ui, &graph, &[id(1)]);
        });
        // Three cards, three wires and their ports are far more than a background.
        assert!(output.shapes.len() > 20, "{}", output.shapes.len());
        output.drop_without_applying_deltas();
        assert_eq!(response, CanvasResponse::default());
    }

    #[test]
    fn drawing_culls_offscreen_wires_but_keeps_crossings_and_tracks_moved_endpoints() {
        let mut graph = GraphView {
            nodes: (1..=4)
                .map(|n| node(n, vec![port("in", true, true), port("then", false, true)]))
                .collect(),
            wires: vec![wire(1, 2), wire(3, 4)],
        };
        for (node, position) in graph.nodes.iter_mut().zip([
            [-400.0, 30.0],
            [1000.0, 30.0],
            [10000.0, 10000.0],
            [11000.0, 10000.0],
        ]) {
            node.position = Some(position);
        }
        let mut canvas = Canvas::new();
        canvas.load(&graph);
        canvas.fit = false;
        canvas.origin = Pos2::ZERO;
        let context = egui::Context::default();
        let draw = |canvas: &mut Canvas| {
            let output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                    ..Default::default()
                },
                |ui| {
                    let _ = canvas.show(ui, &graph, &[]);
                },
            );
            let curves: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::epaint::Shape::CubicBezier(curve) => Some(curve.points),
                    _ => None,
                })
                .collect();
            output.drop_without_applying_deltas();
            curves
        };
        let before = draw(&mut canvas);
        assert_eq!(before.len(), 1);
        canvas.move_node(id(1), Vec2::new(50.0, 0.0));
        let after = draw(&mut canvas);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0][0], before[0][0] + Vec2::new(50.0, 0.0));
        assert_eq!(after[0][3], before[0][3]);
    }
}
