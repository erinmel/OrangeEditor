//! Card dimensions and port rows shared by painting, wires and hit testing.
use super::*;

pub(super) const PATH_ROW: f32 = 52.0;
pub(super) const SOURCE_ROW: f32 = 80.0;

pub(super) fn literal(node: &NodeView) -> bool {
    matches!(
        node.definition.as_str(),
        "core/whole-number" | "core/number" | "core/true-or-false" | "core/text"
    )
}

pub(super) fn path_port(node: &NodeView, port: &PortView) -> bool {
    port.input && node.paths.iter().any(|path| path.port == port.id)
}

pub(super) fn width(node: &NodeView) -> f32 {
    if !node.paths.is_empty()
        || node
            .references
            .iter()
            .any(|(_, kind)| *kind == orange_host::ReferenceKind::Connection)
    {
        360.0
    } else if literal(node) {
        200.0
    } else if node.definition == "core/on-manual" {
        220.0
    } else {
        NODE_WIDTH
    }
}

#[derive(Debug)]
pub(super) struct CardLayout {
    pub size: Vec2,
    pub offsets: Vec<Vec2>,
    pub source_top: f32,
}

impl CardLayout {
    pub fn new(node: &NodeView) -> Self {
        let width = width(node);
        let input_count = node
            .ports
            .iter()
            .filter(|port| port.input && !path_port(node, port))
            .count();
        let output_count = node.ports.iter().filter(|port| !port.input).count();
        #[expect(clippy::cast_precision_loss, reason = "finite visible node rows")]
        let body_end = BODY_TOP + input_count.max(output_count).max(1) as f32 * ROW_HEIGHT;
        let mut sides = [0usize; 2];
        let mut path_index = 0usize;
        let offsets = node
            .ports
            .iter()
            .map(|port| {
                #[expect(clippy::cast_precision_loss, reason = "finite visible node rows")]
                let y = if path_port(node, port) {
                    let y = body_end + path_index as f32 * PATH_ROW + PATH_ROW / 2.0;
                    path_index += 1;
                    y
                } else {
                    let row = &mut sides[usize::from(port.input)];
                    let y = BODY_TOP + (*row as f32 + 0.5) * ROW_HEIGHT;
                    *row += 1;
                    y
                };
                Vec2::new(if port.input { 0.0 } else { width }, y)
            })
            .collect();
        #[expect(clippy::cast_precision_loss, reason = "finite visible node rows")]
        let source_top = body_end + path_index as f32 * PATH_ROW;
        let source = super::connection_footer(node);
        Self {
            size: Vec2::new(
                width,
                source_top + source + settings::height(node) + FOOTER_HEIGHT,
            ),
            offsets,
            source_top,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_controls_use_a_full_row_and_share_the_ports_geometry() {
        let (host, _) = orange_host::Host::with_services(
            orange_host::HostOptions::default(),
            vec![orange_audio::package()],
        )
        .unwrap();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        let id = host
            .add_node(&mut draft, orange_audio::PLAY, [0.0, 0.0])
            .unwrap();
        let graph = host.draft_view(&draft).unwrap();
        let node = graph.nodes.iter().find(|node| node.id == id).unwrap();
        let layout = CardLayout::new(node);
        assert_eq!(layout.size.x, 360.0);
        let geometry = Geometry::new(&graph);
        let output_bottom = node
            .ports
            .iter()
            .zip(&layout.offsets)
            .filter(|(port, _)| !port.input)
            .map(|(_, at)| at.y)
            .fold(0.0, f32::max);
        for (port, offset) in node.ports.iter().zip(&layout.offsets) {
            assert_eq!(
                geometry.port(id, &port.id, port.input).unwrap().offset,
                *offset
            );
            if path_port(node, port) {
                assert!(offset.y > output_bottom);
            }
        }
        let ctx = egui::Context::default();
        ctx.run_ui(Default::default(), |ui| {
            let rect = Rect::from_min_size(egui::pos2(20.0, 20.0), layout.size);
            let (_, slots) = draw_node(
                ui,
                ui.painter(),
                node,
                (&node.name, &node.kind),
                rect,
                1.0,
                &Palette::of(ui),
                (false, None, true),
                egui::Id::unique("test"),
                None,
                None,
            );
            for (port, slot) in slots {
                let at = port_offset(node, &port.id, true).unwrap();
                assert!((slot.center().y - (rect.top() + at.y)).abs() < 0.1);
                if path_port(node, port) {
                    assert!(slot.width() > 320.0);
                }
            }
        })
        .drop_without_applying_deltas();
    }

    #[test]
    fn manual_action_is_centered_on_its_exit_and_policies_do_not_add_body_rows() {
        let (host, _) = orange_host::Host::new(orange_host::HostOptions::default()).unwrap();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        let manual = host
            .add_node(&mut draft, "core/on-manual", [0.0, 0.0])
            .unwrap();
        let math = host
            .add_node(&mut draft, "org.orange.math/add", [0.0, 0.0])
            .unwrap();
        let graph = host.draft_view(&draft).unwrap();
        let manual = graph.nodes.iter().find(|node| node.id == manual).unwrap();
        let math = graph.nodes.iter().find(|node| node.id == math).unwrap();
        assert!(settings::has_advanced(math));
        assert_eq!(settings::height(math), 0.0);
        assert_eq!(node_size(manual).y, BODY_TOP + ROW_HEIGHT + FOOTER_HEIGHT);
        let ctx = egui::Context::default();
        let output = ctx.run_ui(Default::default(), |ui| {
            let rect = Rect::from_min_size(egui::Pos2::ZERO, node_size(manual));
            draw_node(
                ui,
                ui.painter(),
                manual,
                (&manual.name, &manual.kind),
                rect,
                1.0,
                &Palette::of(ui),
                (false, Some(true), true),
                egui::Id::unique("manual-test"),
                None,
                None,
            );
        });
        let fire = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "Fire" => Some(text),
                _ => None,
            })
            .unwrap();
        let center = fire.pos.y + fire.galley.rect.center().y;
        assert!((center - port_offset(manual, "then", false).unwrap().y).abs() < 0.1);
        output.drop_without_applying_deltas();
    }
}
