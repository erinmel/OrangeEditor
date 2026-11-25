//! A card's rows, built once from its node (22 §3.1). Size, port positions, wires,
//! hit tests, painting and the controls all read this one model, so nothing else
//! places anything on a card.
use super::*;

/// Every card's width, in graph units; a literal's is narrower, and a card whose body
/// is a spectrum chart is wider.
pub(super) const WIDTH: f32 = 272.0;
const METER_WIDTH: f32 = 354.0;
pub(super) const METER_FIELDS_X: f32 = 116.0;
const LITERAL_WIDTH: f32 = 180.0;
const CHART_WIDTH: f32 = 400.0;
/// A row of one line: a flow row, an output, an input without a field.
pub(super) const LINE: f32 = 28.0;
/// The name line of a field block.
const NAME: f32 = 18.0;
/// A field's control.
pub(super) const CONTROL: f32 = 30.0;
/// A dial, with its name, value and unit inside it.
const DIAL_CONTROL: f32 = 66.0;
/// The space under a field block's control.
const GAP: f32 = 8.0;
/// The space under a card's last row.
pub(super) const END: f32 = 8.0;

/// What changes a value in a field block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum FieldOf<'a> {
    /// A value typed on an input that no wire feeds.
    Input(&'a PortView),
    /// An inline text area with a fixed height for this frame.
    TextArea(&'a PortView, crate::properties::TextInput, f32),
    /// A file or folder on an input, chosen with the system's chooser.
    Path(&'a PortView, &'a orange_host::PathInput),
    /// A setting of the node, by name and schema.
    Setting(
        &'a str,
        &'a serde_json::Value,
        Option<crate::properties::AudioDial>,
    ),
    /// The variable named by the setting at this JSON pointer.
    Variable(&'a str),
    /// The connection or device the node uses.
    Connection,
    /// One more instance of a port the node has as many of as it is given, such as
    /// a Fork's branches.
    AddPort(&'a orange_host::VariadicView),
    AddSignalReceiver,
    MapPairs,
}

impl<'a> FieldOf<'a> {
    /// The input this field edits.
    pub(super) const fn port(&self) -> Option<&'a PortView> {
        match self {
            Self::Input(port) | Self::TextArea(port, ..) | Self::Path(port, _) => Some(port),
            _ => None,
        }
    }

    pub(super) fn control_height(self) -> f32 {
        match self {
            Self::TextArea(_, _, height) => height,
            Self::Setting(_, _, Some(_)) => DIAL_CONTROL,
            _ => CONTROL,
        }
    }
}

/// The left side of a row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Left<'a> {
    /// On manual's Fire button.
    Fire,
    LoopRole(&'static str),
    /// What a variable write does, such as `Counter ← Counter + 1`.
    Operation,
    FailureRoutes {
        expanded: bool,
    },
    /// A port with only its name: the way in, a wired input, or an input nobody types.
    Port(&'a PortView),
    /// An editable value: a name line, unless `named` is false, and its control.
    Field {
        editor: FieldOf<'a>,
        named: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Row<'a> {
    /// From the card's top, in graph units.
    pub top: f32,
    pub height: f32,
    pub left: Option<Left<'a>>,
    /// An output on the row's first line.
    pub right: Option<&'a PortView>,
    pub field_inset: f32,
    pub field_end: Option<f32>,
}

impl<'a> Row<'a> {
    pub(super) fn input(&self) -> Option<&'a PortView> {
        match self.left {
            Some(Left::Port(port)) => Some(port),
            Some(Left::Field { editor, .. }) => editor.port(),
            _ => None,
        }
    }

    /// Whether input `port` sits on this row, as a port or as a field's port.
    fn holds_input(&self, port: &PortView) -> bool {
        self.input().is_some_and(|left| std::ptr::eq(left, port))
    }

    /// The middle of the row's first line, for labels and output ports.
    pub(super) fn anchor(&self) -> f32 {
        match self.left {
            Some(Left::Field { named: true, .. }) => self.top + NAME / 2.0,
            Some(Left::Field { named: false, .. }) => self.top + CONTROL / 2.0,
            _ => self.top + LINE / 2.0,
        }
    }

    /// Center an editable input between its label and the bottom of its control.
    pub(super) fn input_anchor(&self) -> f32 {
        match self.left {
            Some(Left::Field { editor, named }) => {
                self.top + (editor.control_height() + if named { NAME } else { 0.0 }) / 2.0
            }
            _ => self.anchor(),
        }
    }

    /// Where the row's control is, from the card's top-left, when it has one.
    pub(super) fn control(&self, width: f32) -> Option<Rect> {
        let Some(Left::Field { editor, named }) = self.left else {
            return None;
        };
        let top = self.top + if named { NAME } else { 0.0 };
        // A literal's output dot sits beside its field.
        let right = if let Some(end) = self.field_end {
            end
        } else if self.right.is_some() && !named {
            width - CARD_PADDING - 10.0
        } else {
            width - CARD_PADDING
        };
        Some(Rect::from_min_max(
            Pos2::new(self.field_inset, top),
            Pos2::new(right, top + editor.control_height()),
        ))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Card<'a> {
    pub size: Vec2,
    pub rows: Vec<Row<'a>>,
    pub sole_primary_exit: Option<&'a str>,
    /// The node has nothing to show yet: its body says to set it up.
    pub empty: bool,
}

#[derive(Clone, Copy)]
pub(super) struct FailureRows<'a> {
    pub ports: orange_host::FailureRoutes,
    pub expanded: bool,
    pub pending: Option<&'a str>,
    pub diagnostic: Option<&'a str>,
}

/// Whether `ty` is a stream: a connection between nodes, never a value to type or show.
pub(crate) fn stream_type(ty: Option<&str>) -> bool {
    ty.is_some_and(|ty| {
        ty.starts_with("core/stream@1<")
            || ty
                .split('@')
                .next()
                .is_some_and(|name| name.ends_with("/stream"))
    })
}

/// Whether a value of this input's type is typed on the card.
fn typed(port: &PortView, details: &[orange_host::PortDetails]) -> bool {
    let details = details
        .iter()
        .find(|detail| detail.input && detail.id == port.id);
    port.literal_editor(details) != orange_host::LiteralEditor::Unavailable
}

pub(super) fn literal(node: &NodeView) -> bool {
    matches!(
        node.definition.as_str(),
        "core/whole-number" | "core/number" | "core/true-or-false" | "core/text"
    )
}

impl<'a> Card<'a> {
    pub fn new(node: &'a NodeView) -> Self {
        Self::with_controls(node, true, None, &[], |_| None)
    }

    pub fn with_controls(
        node: &'a NodeView,
        chart_open: bool,
        failure: Option<FailureRows<'_>>,
        details: &[orange_host::PortDetails],
        text_height: impl Fn(&PortView) -> Option<f32>,
    ) -> Self {
        if spectrum::is_noise(node) {
            return Self::meter(node);
        }
        let primary = |port: &&PortView| {
            port.input || failure.is_none_or(|routes| !routes.ports.contains(&port.id))
        };
        let literal = literal(node);
        let width = if literal {
            LITERAL_WIDTH
        } else if spectrum::combined(node) {
            CHART_WIDTH
        } else {
            WIDTH
        };
        let mut rows = Vec::new();
        // An audio analysis card's chart comes first, under the header; its rows follow.
        let mut y = BODY_TOP + spectrum::chart_height(node, chart_open);
        let mut push =
            |rows: &mut Vec<Row<'a>>, left: Option<Left<'a>>, right: Option<&'a PortView>| {
                let height = match left {
                    Some(Left::Field { editor, named }) => {
                        (if named { NAME } else { 0.0 }) + editor.control_height() + GAP
                    }
                    _ => LINE,
                };
                rows.push(Row {
                    top: y,
                    height,
                    left,
                    right,
                    field_inset: CARD_PADDING,
                    field_end: None,
                });
                y += height;
            };
        if let Some(role) = crate::node_ergonomics::loop_role(node) {
            push(&mut rows, Some(Left::LoopRole(role)), None);
        }

        // Flow: the way in and the first way out share the first row.
        let mut ins: Vec<Left<'a>> = Vec::new();
        if node.definition == "core/on-manual" {
            ins.push(Left::Fire);
        }
        ins.extend(
            node.ports
                .iter()
                .filter(|port| port.exec && port.input)
                // The gate accepts both spellings. Keep existing alias wires
                // reachable without offering two identical inputs on new cards.
                .filter(|port| {
                    node.definition != "core/gate" || port.id != "enter" || port.connected
                })
                .map(Left::Port),
        );
        let outs: Vec<_> = node
            .ports
            .iter()
            .filter(|port| port.exec && !port.input)
            .filter(primary)
            .collect();
        let sole_primary_exit = match outs.as_slice() {
            [port] => Some(port.id.as_str()),
            _ => None,
        };
        for index in 0..ins.len().max(outs.len()) {
            push(&mut rows, ins.get(index).copied(), outs.get(index).copied());
        }

        if crate::node_ergonomics::operation_caption(node).is_some() {
            push(&mut rows, Some(Left::Operation), None);
        }

        // What the node names, then its inputs in declared order, then its settings.
        let mut lefts: Vec<Left<'a>> = settings::variable_pointers(node)
            .map(|pointer| Left::Field {
                editor: FieldOf::Variable(pointer),
                named: true,
            })
            .collect();
        if node
            .references
            .iter()
            .any(|(_, kind)| *kind == orange_host::ReferenceKind::Connection)
        {
            lefts.push(Left::Field {
                editor: FieldOf::Connection,
                named: true,
            });
        }
        let paired_map = crate::node_ergonomics::paired_map_available(node);
        for port in crate::properties::input_ports(node) {
            if paired_map && matches!(port.id.as_str(), "keys" | "values") {
                lefts.push(Left::Port(port));
                continue;
            }
            if node.missing.is_none()
                && !port.connected
                && let Some(kind) = crate::properties::TextInput::for_port(node, port)
            {
                lefts.push(Left::Field {
                    editor: FieldOf::TextArea(
                        port,
                        kind,
                        text_height(port).unwrap_or_else(|| {
                            let text = port
                                .value
                                .as_deref()
                                .or(port.default.as_deref())
                                .unwrap_or("");
                            let text = crate::properties::text_area_preview(text, kind);
                            crate::properties::text_area_height(
                                &text,
                                width - 2.0 * CARD_PADDING,
                                kind,
                            )
                        }),
                    ),
                    named: !literal,
                });
                continue;
            }
            let path = node.paths.iter().find(|path| path.port == port.id);
            lefts.push(match path {
                _ if port.connected || node.missing.is_some() => Left::Port(port),
                Some(path) => Left::Field {
                    editor: FieldOf::Path(port, path),
                    named: true,
                },
                // A stream is only ever wired: nothing can be typed for it.
                None if !stream_type(port.ty.as_deref()) && typed(port, details) => Left::Field {
                    editor: FieldOf::Input(port),
                    named: !literal,
                },
                None => Left::Port(port),
            });
        }
        lefts.extend(settings::fields(node).map(|(name, schema)| Left::Field {
            editor: FieldOf::Setting(name, schema, None),
            named: true,
        }));
        if paired_map {
            lefts.push(Left::Field {
                editor: FieldOf::MapPairs,
                named: false,
            });
        }
        // A port the node may have more of ends the card with a button that adds one;
        // a chart card adds its band ports from its chart instead.
        let growing: Vec<Left<'a>> = node
            .variadic
            .iter()
            .filter(|_| !spectrum::combined(node))
            .filter(|variadic| variadic.max.is_none_or(|max| variadic.count < max))
            .map(|variadic| Left::Field {
                editor: FieldOf::AddPort(variadic),
                named: false,
            })
            .collect();
        // A picker that is the only thing to set needs no name: the title says it.
        if let [Left::Field { editor, named }] = lefts.as_mut_slice()
            && matches!(editor, FieldOf::Variable(_) | FieldOf::Connection)
        {
            *named = false;
        }

        // Each output takes the first line of the next row that has one free.
        let mut outputs: Vec<&PortView> = node
            .ports
            .iter()
            .filter(|port| !port.exec && !port.input)
            .filter(primary)
            .collect();
        // A splitter's band outputs read from low to high frequency, whatever order the
        // bands were drawn in.
        if spectrum::combined(node) {
            let low = |port: &PortView| spectrum::band_low(node, &port.id);
            outputs.sort_by(|a, b| {
                low(a)
                    .partial_cmp(&low(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        let mut outputs = outputs.into_iter().peekable();
        for left in lefts {
            let hosts = !matches!(left, Left::Field { named: false, .. }) || literal;
            let right = if hosts { outputs.next() } else { None };
            push(&mut rows, Some(left), right);
        }
        for output in outputs {
            push(&mut rows, None, Some(output));
        }
        for add in growing {
            push(&mut rows, Some(add), None);
        }
        if crate::palette::signal(node).is_some_and(|(sender, _)| sender) {
            push(
                &mut rows,
                Some(Left::Field {
                    editor: FieldOf::AddSignalReceiver,
                    named: false,
                }),
                None,
            );
        }
        let empty = rows.is_empty();
        if empty {
            push(&mut rows, None, None);
        }
        if let Some(routes) = failure {
            push(
                &mut rows,
                Some(Left::FailureRoutes {
                    expanded: routes.expanded,
                }),
                None,
            );
            let mut auxiliary = [None; 3];
            for port in node.ports.iter().filter(|port| !port.input) {
                if !routes.ports.contains(&port.id) {
                    continue;
                }
                match port.id.as_str() {
                    "failed" => auxiliary[0] = Some(port),
                    "cancelled" => auxiliary[1] = Some(port),
                    "error" => auxiliary[2] = Some(port),
                    _ => {}
                }
            }
            let error_wired = auxiliary[2].is_some_and(|port| port.connected);
            for port in auxiliary.into_iter().flatten() {
                if routes.expanded
                    || port.connected
                    || routes.pending == Some(port.id.as_str())
                    || routes.diagnostic == Some(port.id.as_str())
                    || (error_wired && port.id == "failed")
                {
                    push(&mut rows, None, Some(port));
                }
            }
        }
        Self {
            size: Vec2::new(width, y + END),
            rows,
            sole_primary_exit,
            empty,
        }
    }

    fn meter(node: &'a NodeView) -> Self {
        let mut rows = Vec::new();
        let mut y = BODY_TOP;
        for port in node.ports.iter().filter(|port| !port.input) {
            rows.push(Row {
                top: y,
                height: LINE,
                left: None,
                right: Some(port),
                field_inset: METER_FIELDS_X,
                field_end: None,
            });
            y += LINE;
        }
        let mut dial_count = 0;
        let mut dial_y = y;
        let middle = f32::midpoint(METER_FIELDS_X, METER_WIDTH - CARD_PADDING);
        for (name, schema) in settings::fields(node) {
            let dial = crate::properties::AudioDial::for_setting(node, name);
            let editor = FieldOf::Setting(name, schema, dial);
            // A dial names itself, so it needs no name line.
            let name_line = if dial.is_some() { 0.0 } else { NAME };
            let height = name_line + editor.control_height() + GAP;
            let (top, inset, end) = if dial.is_some() {
                if dial_count == 0 {
                    dial_y = y;
                    y += height;
                }
                dial_count += 1;
                if dial_count == 1 {
                    (dial_y, METER_FIELDS_X, Some(middle - 6.0))
                } else {
                    (dial_y, middle, None)
                }
            } else {
                let top = y;
                y += height;
                (top, METER_FIELDS_X, None)
            };
            rows.push(Row {
                top,
                height,
                left: Some(Left::Field {
                    editor,
                    named: dial.is_none(),
                }),
                right: None,
                field_inset: inset,
                field_end: end,
            });
        }
        let mut stream_y = BODY_TOP + spectrum::NOISE_CHART;
        for port in node.ports.iter().filter(|port| port.input) {
            rows.push(Row {
                top: stream_y,
                height: LINE,
                left: Some(Left::Port(port)),
                right: None,
                field_inset: CARD_PADDING,
                field_end: None,
            });
            stream_y += LINE;
        }
        Self {
            size: Vec2::new(METER_WIDTH, y.max(stream_y) + END),
            rows,
            sole_primary_exit: None,
            empty: false,
        }
    }

    /// Where a port's dot is, from the card's top-left.
    pub fn offset(&self, port: &PortView) -> Option<Vec2> {
        self.rows.iter().find_map(|row| {
            let here = if port.input {
                row.holds_input(port)
            } else {
                row.right.is_some_and(|right| std::ptr::eq(right, port))
            };
            here.then(|| {
                if port.input {
                    Vec2::new(0.0, row.input_anchor())
                } else {
                    Vec2::new(self.size.x, row.anchor())
                }
            })
        })
    }

    /// The row an input sits on, where a dropped wire lands on it.
    pub fn row_of(&self, port: &PortView) -> Option<&Row<'a>> {
        self.rows.iter().find(|row| row.holds_input(port))
    }

    /// The row an output sits on, where a wire dragged back from an input lands on it.
    pub fn row_of_output(&self, port: &PortView) -> Option<&Row<'a>> {
        self.rows
            .iter()
            .find(|row| row.right.is_some_and(|right| std::ptr::eq(right, port)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> orange_host::Host {
        orange_host::Host::with_services(
            orange_host::HostOptions::default(),
            vec![orange_audio::package()],
        )
        .unwrap()
        .0
    }

    fn card_of(definition: &str, test: impl FnOnce(&NodeView, &Card<'_>)) {
        let host = host();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        let id = host.add_node(&mut draft, definition, [0.0, 0.0]).unwrap();
        let graph = host.draft_view(&draft).unwrap();
        let node = graph.nodes.iter().find(|node| node.id == id).unwrap();
        test(node, &Card::new(node));
    }

    /// Every port has one dot, inputs on the left edge and outputs on the right, and
    /// rows follow each other without gaps or overlaps.
    #[test]
    fn every_port_has_one_place_and_rows_stack() {
        for definition in [
            orange_audio::PLAY,
            "core/on-manual",
            "org.orange.math/add",
            "core/set-variable",
            "core/get-variable",
            "core/whole-number",
        ] {
            card_of(definition, |node, card| {
                for port in &node.ports {
                    let at = card
                        .offset(port)
                        .unwrap_or_else(|| panic!("{definition}: {} has no place", port.id));
                    assert_eq!(at.x, if port.input { 0.0 } else { card.size.x });
                }
                let mut y = BODY_TOP;
                for row in &card.rows {
                    assert_eq!(row.top, y, "{definition}");
                    y += row.height;
                }
                assert_eq!(card.size.y, y + END);
            });
        }
    }

    /// A stream input is only ever wired, so its row is the port alone, never a field
    /// asking for a value.
    #[test]
    fn a_stream_input_never_gets_a_field() {
        card_of("org.orange.audio/transient", |node, card| {
            let stream = node
                .ports
                .iter()
                .find(|port| port.input && port.id == "stream")
                .unwrap();
            let row = card.row_of(stream).unwrap();
            assert!(matches!(row.left, Some(Left::Port(_))), "{:?}", row.left);
        });
    }

    #[test]
    fn editable_input_is_centered_across_label_and_control() {
        card_of("org.orange.color/color", |node, card| {
            let input = node.ports.iter().find(|port| port.input).unwrap();
            let row = card.row_of(input).unwrap();
            let control = row.control(card.size.x).unwrap();
            let center = (row.top + control.bottom()) / 2.0;
            assert_eq!(card.offset(input).unwrap().y, center);
            assert_ne!(row.anchor(), center);
        });
    }

    /// Fields span the card's inner width; Enter and the first Exit share a row.
    #[test]
    fn fields_are_full_width_and_flow_shares_the_first_row() {
        card_of(orange_audio::PLAY, |node, card| {
            let first = &card.rows[0];
            assert!(matches!(first.left, Some(Left::Port(port)) if port.exec));
            assert!(first.right.is_some_and(|port| port.exec));
            let controls: Vec<_> = card
                .rows
                .iter()
                .filter_map(|row| row.control(card.size.x))
                .collect();
            assert!(!controls.is_empty(), "{}", node.definition);
            for control in controls {
                assert_eq!(control.width(), WIDTH - 2.0 * CARD_PADDING);
            }
        });
    }

    /// A picker that is the card's only setting goes without its name.
    #[test]
    fn a_lone_picker_has_no_name_line() {
        card_of("core/get-variable", |_, card| {
            assert!(matches!(
                card.rows[0].left,
                Some(Left::Field {
                    editor: FieldOf::Variable(_),
                    named: false
                })
            ));
        });
        card_of("core/set-variable", |node, _| {
            // Once the variable gives the node a value input, the picker is named.
            let mut node = node.clone();
            node.ports.push(PortView {
                id: "value".into(),
                input: true,
                exec: false,
                ty: Some("core/i64@1".into()),
                value: None,
                default: None,
                connected: false,
                label: None,
            });
            let card = Card::new(&node);
            assert!(card.rows.iter().any(|row| matches!(
                row.left,
                Some(Left::Field {
                    editor: FieldOf::Variable(_),
                    named: true
                })
            )));
        });
    }
}
