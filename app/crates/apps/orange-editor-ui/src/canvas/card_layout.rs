//! Card dimensions and port rows shared by painting, wires and hit testing.
use super::*;

pub(super) const PATH_ROW: f32 = 30.0;
pub(super) const SOURCE_ROW: f32 = 32.0;

/// Narrowest and widest a card is, in graph units (22 §3.1).
const MIN_WIDTH: f32 = 160.0;
const MAX_WIDTH: f32 = 320.0;

/// About how wide text of a card's 12.5-unit font is: a layout estimate made without a
/// painter, generous enough that measured text fits.
fn text_width(text: &str) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "short labels")]
    let count = text.chars().count() as f32;
    count * 7.0
}

/// Whether an input of this port shows a typed field on the card.
fn has_field(port: &PortView) -> bool {
    !port.exec
        && !port.connected
        && port.ty.as_deref().is_some_and(|ty| {
            matches!(
                ty.split('@').next().unwrap_or_default(),
                "core/string"
                    | "core/i64"
                    | "core/u64"
                    | "core/i32"
                    | "core/u32"
                    | "core/f64"
                    | "core/f32"
                    | "core/bool"
                    | "core/duration"
            ) || !ty.starts_with("core/")
        })
}

/// The width of a card's input names, where its fields start.
pub(super) fn input_column(node: &NodeView) -> f32 {
    node.ports
        .iter()
        .filter(|port| port.input && !port.exec && !path_port(node, port))
        .map(|port| text_width(&super::port_title(port)))
        .fold(0.0, f32::max)
        .min(110.0)
}

pub(super) fn literal(node: &NodeView) -> bool {
    matches!(
        node.definition.as_str(),
        "core/whole-number" | "core/number" | "core/true-or-false" | "core/text"
    )
}

pub(super) fn path_port(node: &NodeView, port: &PortView) -> bool {
    port.input && node.paths.iter().any(|path| path.port == port.id)
}

/// As wide as the card's title and its widest row need (22 §3.1).
pub(super) fn width(node: &NodeView) -> f32 {
    if literal(node) {
        return MIN_WIDTH;
    }
    let pad = 2.0 * CARD_PADDING;
    // The title, the trigger arrow and the settings menu.
    let title = text_width(&node.name) * 1.16 + pad + 44.0;
    let column = input_column(node);
    let side = |input: bool| -> Vec<&PortView> {
        node.ports
            .iter()
            .filter(|port| port.input == input && !path_port(node, port))
            .collect()
    };
    let (inputs, outputs) = (side(true), side(false));
    let mut widest = title;
    for row in 0..inputs.len().max(outputs.len()) {
        let left = inputs.get(row).map_or(0.0, |port| {
            if port.exec {
                text_width(&super::port_title(port)) + 36.0
            } else {
                let value = port.value.as_deref().unwrap_or("—");
                let field = if has_field(port) {
                    (text_width(value) + 14.0).clamp(44.0, 140.0)
                } else {
                    0.0
                };
                column + 8.0 + field
            }
        });
        let right = outputs.get(row).map_or(0.0, |port| {
            // An output's name, and room for the value a run shows beside it.
            text_width(&super::port_title(port)) + if port.exec { 22.0 } else { 64.0 }
        });
        widest = widest.max(pad + left + 16.0 + right);
    }
    // Pickers, paths and settings need room for a list or a field beside their name.
    if !node.paths.is_empty() || !node.references.is_empty() || settings::height(node) > 0.0 {
        widest = widest.max(240.0);
    }
    widest.clamp(MIN_WIDTH, MAX_WIDTH)
}

#[derive(Debug)]
pub(super) struct CardLayout {
    pub size: Vec2,
    pub offsets: Vec<Vec2>,
    pub source_top: f32,
    /// Where the pickers of what the node names start, in graph units from the top.
    pub reference_top: f32,
}

impl CardLayout {
    pub fn new(node: &NodeView) -> Self {
        let width = width(node);
        // The pickers of what the node names come after the row of steps (Enter and
        // Exit) and before the values, so Set variable reads: in, which variable,
        // which value.
        let references = settings::reference_height(node);
        // Steps come first in a node's ports.
        let first_shifted = usize::from(node.ports.first().is_some_and(|port| port.exec));
        #[expect(clippy::cast_precision_loss, reason = "finite visible node rows")]
        let reference_top = BODY_TOP + first_shifted as f32 * ROW_HEIGHT;
        let input_count = node
            .ports
            .iter()
            .filter(|port| port.input && !path_port(node, port))
            .count();
        let output_count = node.ports.iter().filter(|port| !port.input).count();
        // A node that is only waiting for its variable shows the picker alone.
        let rows = match input_count.max(output_count) {
            0 if settings::has_references(node) => 0,
            rows => rows.max(1),
        };
        #[expect(clippy::cast_precision_loss, reason = "finite visible node rows")]
        let body_end = BODY_TOP + references + rows as f32 * ROW_HEIGHT;
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
                    let shift = if *row >= first_shifted {
                        references
                    } else {
                        0.0
                    };
                    let y = BODY_TOP + shift + (*row as f32 + 0.5) * ROW_HEIGHT;
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
            reference_top,
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
        assert!((MIN_WIDTH..=MAX_WIDTH).contains(&layout.size.x));
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
                (false, None, Some(&Enums::new())),
                egui::Id::unique("test"),
                None,
                None,
            );
            for (port, slot) in slots {
                let at = port_offset(node, &port.id, true).unwrap();
                assert!((slot.center().y - (rect.top() + at.y)).abs() < 0.1);
                if path_port(node, port) {
                    assert!(slot.width() >= layout.size.x - 2.0 * CARD_PADDING - 1.0);
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
                (false, Some(true), Some(&Enums::new())),
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

#[cfg(test)]
mod reference_tests {
    use super::*;

    /// A variable node chooses its variable on the card: before it names one, the
    /// picker is its whole body, and once it does, the picker comes before its ports.
    #[test]
    fn variable_nodes_carry_their_picker_on_the_card() {
        let (host, _) = orange_host::Host::new(orange_host::HostOptions::default()).unwrap();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        let id = host
            .add_node(&mut draft, "core/get-variable", [0.0, 0.0])
            .unwrap();
        let graph = host.draft_view(&draft).unwrap();
        let node = graph.nodes.iter().find(|node| node.id == id).unwrap();
        assert!(node.ports.is_empty());
        assert!(settings::has_references(node));
        let layout = CardLayout::new(node);
        assert_eq!(
            layout.source_top,
            BODY_TOP + settings::reference_height(node)
        );
        assert!(settings::reference_height(node) > 0.0);
    }
}
