//! The graph canvas: nodes as cards with their ports, wires between them, panning,
//! zooming and moving nodes.
//!
//! It is a widget of its own. An application that has its own lists and panels shows
//! only this: it asks its host for a [`GraphView`], calls [`Canvas::show`] each frame
//! and acts on the [`CanvasResponse`]. Nothing here knows about files, flows or the
//! other panels of the editor.
use egui::epaint::CubicBezierShape;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use orange_host::{GraphView, NodeId, NodeView, PortView};
use std::collections::BTreeMap;

/// Width of a node card, in graph units (one unit is one point at zoom 1).
pub const NODE_WIDTH: f32 = 236.0;
/// Height of a card's title.
pub const HEADER_HEIGHT: f32 = 34.0;
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

/// The size of a node's card in graph units: a title and one row for each input beside
/// an output.
pub fn node_size(node: &NodeView) -> Vec2 {
    let inputs = node.ports.iter().filter(|port| port.input).count();
    let outputs = node.ports.len() - inputs;
    #[expect(clippy::cast_precision_loss, reason = "a node has few ports")]
    let rows = inputs.max(outputs) as f32;
    Vec2::new(
        NODE_WIDTH,
        HEADER_HEIGHT + rows * ROW_HEIGHT + FOOTER_HEIGHT,
    )
}

/// Where a port's dot is, relative to the top left corner of its node's card.
pub fn port_offset(node: &NodeView, port: &str, input: bool) -> Option<Vec2> {
    let row = node
        .ports
        .iter()
        .filter(|candidate| candidate.input == input)
        .position(|candidate| candidate.id == port)?;
    #[expect(clippy::cast_precision_loss, reason = "a node has few ports")]
    let y = HEADER_HEIGHT + (row as f32 + 0.5) * ROW_HEIGHT;
    Some(Vec2::new(if input { 0.0 } else { NODE_WIDTH }, y))
}

/// Places for the nodes of a graph that has none saved: columns from left to right
/// following the wires, each column stacked from the top. A node keeps the place the
/// document gives it.
pub fn auto_layout(graph: &GraphView) -> BTreeMap<NodeId, Pos2> {
    // A node's column is one past the furthest column that feeds it. The passes are
    // bounded by the node count, so a cycle cannot loop forever.
    let mut column: BTreeMap<NodeId, usize> = graph.nodes.iter().map(|node| (node.id, 0)).collect();
    for _ in 0..graph.nodes.len() {
        let mut changed = false;
        for wire in &graph.wires {
            let from = column.get(&wire.from.0).copied().unwrap_or(0);
            if let Some(to) = column.get_mut(&wire.to.0)
                && *to < from + 1
                && from + 1 < graph.nodes.len()
            {
                *to = from + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut next_y: BTreeMap<usize, f32> = BTreeMap::new();
    let mut places = BTreeMap::new();
    for node in &graph.nodes {
        let at = column.get(&node.id).copied().unwrap_or(0);
        let y = next_y.entry(at).or_insert(0.0);
        #[expect(clippy::cast_precision_loss, reason = "a graph has few columns")]
        let placed = Pos2::new(at as f32 * (NODE_WIDTH + LAYOUT_GAP.x), *y);
        *y += node_size(node).y + LAYOUT_GAP.y;
        let place = node.position.map_or(placed, |[x, y]| Pos2::new(x, y));
        let _ = places.insert(node.id, place);
    }
    places
}

/// What the person did on the canvas this frame.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CanvasResponse {
    /// The trigger whose button was pressed.
    pub trigger: Option<NodeId>,
    /// Nodes were let go at new places: [`Canvas::places`] is worth saving.
    pub moved: bool,
    /// The node that was clicked.
    pub selected: Option<NodeId>,
    /// A wire was drawn from an output to an input: the two ends.
    pub connect: Option<((NodeId, String), (NodeId, String))>,
    /// The wires at a port were taken off: the node, the port and whether it is an input.
    pub disconnect: Option<(NodeId, String, bool)>,
    /// The selected node was deleted.
    pub remove: Option<NodeId>,
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
    selected: Option<NodeId>,
    /// A node was dragged and not yet let go.
    moving: bool,
    /// Show the whole graph at the next frame.
    fit: bool,
    wiring: Option<Wiring>,
    /// The size of the canvas when it was last shown, in points.
    size: Vec2,
}
impl Default for Canvas {
    fn default() -> Self {
        Self {
            origin: Pos2::new(-40.0, -40.0),
            zoom: 1.0,
            places: BTreeMap::new(),
            selected: None,
            moving: false,
            fit: true,
            wiring: None,
            size: Vec2::new(800.0, 600.0),
        }
    }
}

/// Colors of the canvas, taken from the surrounding egui style so it follows a light
/// or dark application.
struct Palette {
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
    fn of(ui: &egui::Ui) -> Self {
        let visuals = ui.visuals();
        let dark = visuals.dark_mode;
        Self {
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
        if !self.places.keys().any(|id| Some(*id) == self.selected) {
            self.selected = None;
        }
        self.fit = true;
    }

    /// Follows an edit of the graph: nodes that are new take the place the document
    /// gives them or an automatic one, nodes that are gone are forgotten, and every
    /// other node stays where it is. The view is not moved.
    pub fn update(&mut self, graph: &GraphView) {
        let fresh = auto_layout(graph);
        self.places.retain(|id, _| fresh.contains_key(id));
        for (id, place) in fresh {
            let _ = self.places.entry(id).or_insert(place);
        }
        if !self.places.keys().any(|id| Some(*id) == self.selected) {
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

    /// Selects a node, as clicking it does.
    pub fn select(&mut self, node: Option<NodeId>) {
        self.selected = node.filter(|id| self.places.contains_key(id));
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
        if response.dragged() {
            self.origin -= response.drag_delta() / self.zoom;
        }
        if response.clicked() {
            self.selected = None;
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
            let rect = Rect::from_min_size(to_screen(place), node_size(node) * zoom);
            if !area.intersects(rect) {
                continue;
            }
            let touched = ui.interact(rect, response.id.with(node.id), Sense::click_and_drag());
            if touched.dragged() {
                self.move_node(node.id, touched.drag_delta() / zoom);
                self.moving = true;
                self.selected = Some(node.id);
            }
            if touched.drag_stopped() && self.moving {
                self.moving = false;
                done.moved = true;
            }
            if touched.clicked() {
                self.selected = Some(node.id);
                done.selected = Some(node.id);
            }
        }

        // Ports: dragging from an output draws a wire; the second mouse button on a
        // wired port takes its wires off.
        let grab = (PORT_RADIUS + 5.0) * zoom.max(0.6);
        for node in &graph.nodes {
            let Some(place) = self.places.get(&node.id).copied() else {
                continue;
            };
            for port in &node.ports {
                let Some(offset) = port_offset(node, &port.id, port.input) else {
                    continue;
                };
                let at = to_screen(place + offset);
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
        // The delete key removes the selected node, unless a text field has the keys.
        if let Some(node) = self.selected
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input(|input| {
                input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace)
            })
        {
            done.remove = Some(node);
            self.selected = None;
        }

        let anchor = |node: NodeId, port: &str, input: bool| -> Option<Pos2> {
            let view = graph.nodes.iter().find(|view| view.id == node)?;
            let place = *self.places.get(&node)?;
            Some(to_screen(place + port_offset(view, port, input)?))
        };
        for wire in &graph.wires {
            let (Some(from), Some(to)) = (
                anchor(wire.from.0, &wire.from.1, false),
                anchor(wire.to.0, &wire.to.1, true),
            ) else {
                continue;
            };
            let color = if wire.exec {
                colors.exec
            } else {
                let ty = graph
                    .nodes
                    .iter()
                    .find(|node| node.id == wire.from.0)
                    .and_then(|node| {
                        node.ports
                            .iter()
                            .find(|port| !port.input && port.id == wire.from.1)
                    })
                    .and_then(|port| port.ty.as_deref());
                type_color(ty)
            };
            // The wire leaves and arrives level, bending more the further it goes.
            let reach = ((to.x - from.x).abs() * 0.5).max(40.0 * zoom);
            let points = [
                from,
                from + Vec2::new(reach, 0.0),
                to - Vec2::new(reach, 0.0),
                to,
            ];
            let width = if wire.exec { 2.5 } else { 2.0 } * zoom.max(0.6);
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
            let reach = ((to.x - from.x).abs() * 0.5).max(40.0 * zoom);
            let points = [
                from,
                from + Vec2::new(reach, 0.0),
                to - Vec2::new(reach, 0.0),
                to,
            ];
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
            let rect = Rect::from_min_size(to_screen(place), node_size(node) * zoom);
            if !area.intersects(rect) {
                continue;
            }
            let selected = self.selected == Some(node.id);
            let fire = triggers.contains(&node.id);
            if draw_node(
                ui,
                &painter,
                node,
                rect,
                zoom,
                &colors,
                selected,
                fire,
                response.id,
            ) {
                done.trigger = Some(node.id);
            }
        }
        done
    }
}

/// Draws one card. Returns whether its trigger button was pressed.
#[expect(clippy::too_many_arguments, reason = "one card's drawing state")]
fn draw_node(
    ui: &egui::Ui,
    painter: &egui::Painter,
    node: &NodeView,
    rect: Rect,
    zoom: f32,
    colors: &Palette,
    selected: bool,
    fire: bool,
    canvas: egui::Id,
) -> bool {
    let radius = 8.0 * zoom;
    let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), HEADER_HEIGHT * zoom));
    painter.rect_filled(rect, radius, colors.card);
    let header_color = if node.entry {
        colors.entry_header
    } else {
        colors.header
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
        let title = FontId::proportional(14.0 * zoom);
        let small = FontId::proportional(11.5 * zoom);
        let left = header.left_center() + Vec2::new(12.0 * zoom, 0.0);
        let _ = painter.text(left, Align2::LEFT_CENTER, &node.name, title, colors.text);
        // What kind of node it is, when its name says something else.
        if node.name != node.kind && !fire {
            let right = header.right_center() - Vec2::new(10.0 * zoom, 0.0);
            let _ = painter.text(
                right,
                Align2::RIGHT_CENTER,
                &node.kind,
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

    let (mut inputs, mut outputs) = (0u16, 0u16);
    for port in &node.ports {
        let row = if port.input {
            &mut inputs
        } else {
            &mut outputs
        };
        let y = rect.min.y + (HEADER_HEIGHT + (f32::from(*row) + 0.5) * ROW_HEIGHT) * zoom;
        *row += 1;
        let x = if port.input { rect.min.x } else { rect.max.x };
        draw_port(painter, port, Pos2::new(x, y), zoom, colors, readable);
    }
    pressed
}

fn draw_port(
    painter: &egui::Painter,
    port: &PortView,
    at: Pos2,
    zoom: f32,
    colors: &Palette,
    readable: bool,
) {
    let radius = PORT_RADIUS * zoom;
    let color = if port.exec {
        colors.exec
    } else {
        type_color(port.ty.as_deref())
    };
    if port.exec {
        // A step is an arrowhead pointing along the flow.
        let tip = at + Vec2::new(radius * 1.2, 0.0);
        let back = at - Vec2::new(radius * 0.9, 0.0);
        let points = vec![
            back + Vec2::new(0.0, -radius),
            tip,
            back + Vec2::new(0.0, radius),
        ];
        let fill = if port.connected { color } else { colors.card };
        painter.add(egui::Shape::convex_polygon(
            points,
            fill,
            Stroke::new(1.5, color),
        ));
    } else if port.connected {
        painter.circle_filled(at, radius, color);
    } else {
        // A value port nothing is wired to is a ring.
        painter.circle_filled(at, radius, colors.card);
        painter.circle_stroke(at, radius, Stroke::new(1.5, color));
    }
    if !readable {
        return;
    }
    let font = FontId::proportional(12.5 * zoom);
    let gap = Vec2::new(12.0 * zoom, 0.0);
    if port.input {
        let label = painter.text(
            at + gap,
            Align2::LEFT_CENTER,
            &port.id,
            font.clone(),
            colors.text,
        );
        // The value written on an input that no wire feeds.
        if let Some(value) = port.value.as_deref().filter(|_| !port.connected) {
            let shown: String = value.chars().take(18).collect();
            let shown = if shown.len() < value.len() {
                format!("{shown}…")
            } else {
                shown
            };
            let from = Pos2::new(label.max.x + 8.0 * zoom, at.y);
            let text = painter.layout_no_wrap(shown, font, colors.text);
            let pill = Rect::from_min_size(
                from - Vec2::new(0.0, text.size().y / 2.0 + 2.0 * zoom),
                text.size() + Vec2::new(10.0, 4.0) * zoom,
            );
            painter.rect_filled(pill, 4.0 * zoom, colors.header);
            painter.galley(pill.min + Vec2::new(5.0, 2.0) * zoom, text, colors.text);
        }
    } else {
        let _ = painter.text(at - gap, Align2::RIGHT_CENTER, &port.id, font, colors.text);
    }
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
            entry: n == 1,
            ports,
            position: None,
            settings: serde_json::json!({}),
            schema: None,
            references: Vec::new(),
            types: Vec::new(),
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
            HEADER_HEIGHT + ROW_HEIGHT + FOOTER_HEIGHT
        );
        assert_eq!(
            node_size(&graph.nodes[2]).y,
            HEADER_HEIGHT + 3.0 * ROW_HEIGHT + FOOTER_HEIGHT
        );
        let third = &graph.nodes[2];
        assert_eq!(
            port_offset(third, "b", true),
            Some(Vec2::new(0.0, HEADER_HEIGHT + 2.5 * ROW_HEIGHT))
        );
        assert_eq!(
            port_offset(&graph.nodes[1], "then", false),
            Some(Vec2::new(NODE_WIDTH, HEADER_HEIGHT + 0.5 * ROW_HEIGHT))
        );
        // A port is looked for on its own side only.
        assert_eq!(port_offset(third, "b", false), None);
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
        let row = |n: f32| third + Vec2::new(20.0, HEADER_HEIGHT + (n + 0.5) * ROW_HEIGHT);
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
        let right = third + Vec2::new(NODE_WIDTH - 10.0, HEADER_HEIGHT + 0.5 * ROW_HEIGHT);
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
}
