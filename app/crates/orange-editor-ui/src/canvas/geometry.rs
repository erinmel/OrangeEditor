//! Borrowed, frame-local geometry: one port walk instead of node scans per wire.
//! Rebuilding from the supplied view keeps standalone Canvas callers free of cache
//! invalidation obligations. Positions are read separately so dragging is immediate.
use super::{BODY_TOP, FOOTER_HEIGHT, NODE_WIDTH, ROW_HEIGHT};
use egui::{Pos2, Rect, Vec2};
use orange_host::{GraphView, NodeId};
use std::collections::HashMap;

pub(super) struct PortGeometry<'a> {
    pub offset: Vec2,
    pub ty: Option<&'a str>,
}

pub(super) struct Geometry<'a> {
    sizes: HashMap<NodeId, Vec2>,
    ports: HashMap<(NodeId, bool, &'a str), PortGeometry<'a>>,
}

impl<'a> Geometry<'a> {
    pub fn new(graph: &'a GraphView) -> Self {
        let mut sizes = HashMap::with_capacity(graph.nodes.len());
        let mut ports = HashMap::with_capacity(graph.nodes.iter().map(|n| n.ports.len()).sum());
        for node in &graph.nodes {
            let mut rows = [0usize; 2];
            for port in &node.ports {
                let row = &mut rows[usize::from(port.input)];
                #[expect(clippy::cast_precision_loss, reason = "screen coordinates")]
                let y = BODY_TOP + (*row as f32 + 0.5) * ROW_HEIGHT;
                // Match port_offset: first occurrence on this side wins, even for
                // an incomplete or malformed view supplied by an embedding host.
                let _ = ports
                    .entry((node.id, port.input, port.id.as_str()))
                    .or_insert(PortGeometry {
                        offset: Vec2::new(if port.input { 0.0 } else { NODE_WIDTH }, y),
                        ty: port.ty.as_deref(),
                    });
                *row += 1;
            }
            #[expect(clippy::cast_precision_loss, reason = "screen coordinates")]
            let height = BODY_TOP + rows[0].max(rows[1]) as f32 * ROW_HEIGHT + FOOTER_HEIGHT;
            let _ = sizes.insert(node.id, Vec2::new(NODE_WIDTH, height));
        }
        Self { sizes, ports }
    }

    pub fn size(&self, node: NodeId) -> Vec2 {
        self.sizes[&node]
    }

    pub fn port<'b>(
        &'b self,
        node: NodeId,
        port: &'b str,
        input: bool,
    ) -> Option<&'b PortGeometry<'b>> {
        self.ports.get(&(node, input, port))
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
