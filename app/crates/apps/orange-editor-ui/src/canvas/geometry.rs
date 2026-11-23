//! Borrowed, frame-local geometry: one port walk instead of node scans per wire.
//! Rebuilding from the supplied view keeps standalone Canvas callers free of cache
//! invalidation obligations. Positions are read separately so dragging is immediate.
use super::card::Card;
use egui::{Pos2, Rect, Vec2};
use orange_host::{GraphView, NodeId, PortDirection};
use std::collections::HashMap;

pub(super) struct PortGeometry<'a> {
    pub offset: Vec2,
    pub ty: Option<&'a str>,
    pub exec: bool,
}

pub(super) struct Geometry<'a> {
    cards: HashMap<NodeId, Card<'a>>,
    ports: HashMap<(NodeId, bool, &'a str), PortGeometry<'a>>,
    incoming: HashMap<(NodeId, &'a str), (NodeId, &'a str)>,
    sole_exec_outputs: HashMap<NodeId, &'a str>,
}

impl<'a> Geometry<'a> {
    #[cfg(test)]
    pub fn new(graph: &'a GraphView) -> Self {
        Self::with_cards(graph, Card::new)
    }

    pub fn with_cards(
        graph: &'a GraphView,
        card: impl Fn(&'a orange_host::NodeView) -> Card<'a>,
    ) -> Self {
        let mut cards = HashMap::with_capacity(graph.nodes.len());
        let mut ports = HashMap::with_capacity(graph.nodes.iter().map(|n| n.ports.len()).sum());
        let mut sole_exec_outputs = HashMap::new();
        for node in &graph.nodes {
            let layout = card(node);
            for row in &layout.rows {
                for (port, x, y) in [
                    (row.input(), 0.0, row.input_anchor()),
                    (row.right, layout.size.x, row.anchor()),
                ] {
                    if let Some(port) = port {
                        ports
                            .entry((node.id, port.input, port.id.as_str()))
                            .or_insert(PortGeometry {
                                offset: Vec2::new(x, y),
                                ty: port.ty.as_deref(),
                                exec: port.exec,
                            });
                    }
                }
            }
            let _ = cards.insert(node.id, layout);
            let mut exec = node.ports.iter().filter(|port| port.exec && !port.input);
            if let (Some(port), None) = (exec.next(), exec.next()) {
                let _ = sole_exec_outputs.insert(node.id, port.id.as_str());
            }
        }
        let mut incoming = HashMap::with_capacity(graph.wires.len());
        for wire in &graph.wires {
            incoming
                .entry((wire.to.0, wire.to.1.as_str()))
                .or_insert((wire.from.0, wire.from.1.as_str()));
        }
        Self {
            cards,
            ports,
            incoming,
            sole_exec_outputs,
        }
    }

    pub fn size(&self, node: NodeId) -> Vec2 {
        self.card(node).size
    }

    pub fn card(&self, node: NodeId) -> &Card<'a> {
        &self.cards[&node]
    }

    pub fn source(&self, node: NodeId, port: &str) -> Option<(NodeId, &'a str)> {
        self.incoming.get(&(node, port)).copied()
    }

    pub fn sole_exec_output(&self, node: NodeId) -> Option<&'a str> {
        self.sole_exec_outputs.get(&node).copied()
    }

    pub fn port<'b>(
        &'b self,
        node: NodeId,
        port: &'b str,
        input: bool,
    ) -> Option<&'b PortGeometry<'b>> {
        self.ports.get(&(node, input, port))
    }

    pub fn diagnostic_port<'b>(
        &'b self,
        node: NodeId,
        port: &'b str,
        direction: Option<PortDirection>,
    ) -> Option<&'b PortGeometry<'b>> {
        match direction {
            Some(PortDirection::Input) => self.port(node, port, true),
            Some(PortDirection::Output) => self.port(node, port, false),
            None => match (self.port(node, port, true), self.port(node, port, false)) {
                (Some(port), None) | (None, Some(port)) => Some(port),
                _ => None,
            },
        }
    }
}

pub(super) fn wire_points(from: Pos2, to: Pos2, zoom: f32) -> [Pos2; 4] {
    let reach = ((to.x - from.x).abs() * 0.5).max(40.0 * zoom);
    [
        from,
        from + Vec2::new(reach, 0.0),
        to - Vec2::new(reach, 0.0),
        to,
    ]
}

/// A Bezier stays inside the convex hull of all four control points. Testing only
/// its endpoints would incorrectly hide wires bending into the viewport.
pub(super) fn wire_visible(points: &[Pos2; 4], clip: Rect, width: f32) -> bool {
    clip.intersects(Rect::from_points(points).expand(width))
}

/// Whether `at` is within `reach` of the wire drawn through `points`, sampled along
/// its curve.
pub(super) fn near_wire(points: [Pos2; 4], at: Pos2, reach: f32) -> bool {
    if !Rect::from_points(&points).expand(reach).contains(at) {
        return false;
    }
    const STEPS: usize = 24;
    let point = |t: f32| {
        let u = 1.0 - t;
        let weights = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
        points
            .iter()
            .zip(weights)
            .fold(Vec2::ZERO, |sum, (p, w)| sum + p.to_vec2() * w)
            .to_pos2()
    };
    (0..STEPS).any(|step| {
        let (a, b) = (
            point(step as f32 / STEPS as f32),
            point((step + 1) as f32 / STEPS as f32),
        );
        let along = b - a;
        let t = ((at - a).dot(along) / along.length_sq().max(f32::EPSILON)).clamp(0.0, 1.0);
        (a + along * t).distance(at) <= reach
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_point_on_the_drawn_curve_is_near_the_wire_and_one_off_it_is_not() {
        let points = wire_points(Pos2::new(0.0, 0.0), Pos2::new(200.0, 100.0), 1.0);
        // The curve passes through the middle of its ends and leaves level.
        assert!(near_wire(points, Pos2::new(100.0, 50.0), 3.0));
        assert!(near_wire(points, Pos2::new(5.0, 1.0), 3.0));
        assert!(!near_wire(points, Pos2::new(100.0, 0.0), 6.0));
        assert!(!near_wire(points, Pos2::new(400.0, 400.0), 6.0));
    }

    #[test]
    fn culling_keeps_crossing_and_bending_wires_and_stroke_margin() {
        let clip = Rect::from_min_max(Pos2::ZERO, Pos2::new(100.0, 100.0));
        assert!(wire_visible(
            &wire_points(Pos2::new(-50.0, 50.0), Pos2::new(150.0, 50.0), 1.0),
            clip,
            2.0
        ));
        // Both ends outside; the backward curve bends into the visible region.
        assert!(wire_visible(
            &wire_points(Pos2::new(-10.0, 10.0), Pos2::new(-20.0, 90.0), 1.0),
            clip,
            2.0
        ));
        assert!(wire_visible(
            &wire_points(Pos2::new(0.0, -1.0), Pos2::new(100.0, -1.0), 1.0),
            clip,
            2.0
        ));
        assert!(!wire_visible(
            &wire_points(Pos2::new(-100.0, -100.0), Pos2::new(-50.0, -100.0), 1.0),
            clip,
            2.0
        ));
    }
}
