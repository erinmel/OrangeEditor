//! A card's rows, built once from its node (22 §3.1). Size, port positions, wires,
//! hit tests, painting and the controls all read this one model, so nothing else
//! places anything on a card.
use super::*;

/// Every card's width, in graph units; a literal's is narrower, and a card whose body
/// is a spectrum chart is wider.
pub(super) const WIDTH: f32 = 272.0;
const METER_WIDTH: f32 = 320.0;
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
    /// An input whose definition asks for a slider or a dial over a given range
    /// (ADR-33).
    HintedInput(&'a PortView, &'a orange_host::controls::Control),
    /// An inline text area with a fixed height for this frame.
    TextArea(&'a PortView, crate::properties::TextInput, f32),
    /// A file or folder on an input, chosen with the system's chooser.
    Path(&'a PortView, &'a orange_host::PathInput),
    /// A setting of the node, by name and schema.
    Setting(
        &'a str,
        &'a serde_json::Value,
        Option<&'a orange_host::controls::Control>,
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
            Self::Input(port)
            | Self::HintedInput(port, _)
            | Self::TextArea(port, ..)
            | Self::Path(port, _) => Some(port),
            _ => None,
        }
    }

    pub(super) fn control_height(self) -> f32 {
        match self {
            Self::TextArea(_, _, height) => height,
            Self::Setting(_, _, Some(_)) => DIAL_CONTROL,
            Self::HintedInput(_, control)
                if control.widget == orange_host::controls::Widget::Dial =>
            {
                DIAL_CONTROL
            }
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
    paired_input_anchor: Option<f32>,
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
        if let Some(anchor) = self.paired_input_anchor {
            return anchor;
        }
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
        let y = std::cell::Cell::new(BODY_TOP + spectrum::chart_height(node, chart_open));
        let push =
            |rows: &mut Vec<Row<'a>>, left: Option<Left<'a>>, right: Option<&'a PortView>| {
                let height = match left {
                    Some(Left::Field { editor, named }) => {
                        (if named { NAME } else { 0.0 }) + editor.control_height() + GAP
                    }
                    _ => LINE,
                };
                rows.push(Row {
                    top: y.get(),
                    height,
                    left,
                    right,
                    field_inset: CARD_PADDING,
                    field_end: None,
                    paired_input_anchor: None,
                });
                y.set(y.get() + height);
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
                && !crate::properties::advanced_input(node, port)
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
                _ if crate::properties::advanced_input(node, port) => Left::Port(port),
                _ if port.connected || node.missing.is_some() => Left::Port(port),
                Some(path) => Left::Field {
                    editor: FieldOf::Path(port, path),
                    named: true,
                },
                // A stream is only ever wired: nothing can be typed for it.
                None if !stream_type(port.ty.as_deref()) && typed(port, details) => {
                    match crate::properties::input_hint(node, port) {
                        Some(control) => Left::Field {
                            editor: FieldOf::HintedInput(port, control),
                            // A dial names itself.
                            named: control.widget != orange_host::controls::Widget::Dial,
                        },
                        None => Left::Field {
                            editor: FieldOf::Input(port),
                            named: !literal,
                        },
                    }
                }
                None => Left::Port(port),
            });
        }
        lefts.extend(settings::fields(node).map(|(name, schema)| {
            let dial = crate::properties::dial_of(node, name);
            Left::Field {
                editor: FieldOf::Setting(name, schema, dial),
                named: dial.is_none(),
            }
        }));
        // Fields hinted `header` come first, so they share the outputs' rows (ADR-33).
        if !node.controls.regions.is_empty() {
            lefts.sort_by_key(|left| {
                crate::properties::field_order(node, field_name(left).unwrap_or(""))
            });
        }
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
        let mut lefts = lefts.into_iter().peekable();
        while let Some(left) = lefts.next() {
            let hosts = !matches!(left, Left::Field { named: false, .. }) || literal;
            let right = if hosts { outputs.next() } else { None };
            push(&mut rows, Some(left), right);
            // Two fields of a region arranged in a row share one, each taking half.
            let paired = field_placement(node, &left);
            if let Some(place) = paired
                && let Some(next) = lefts.peek()
                && field_placement(node, next) == Some(place)
                && let Some(next) = lefts.next()
                && let Some(first) = rows.last_mut()
            {
                let middle = f32::midpoint(CARD_PADDING, width - CARD_PADDING);
                first.field_end = Some(middle - 6.0);
                let height = match next {
                    Left::Field { editor, named } => {
                        (if named { NAME } else { 0.0 }) + editor.control_height() + GAP
                    }
                    _ => LINE,
                };
                let (top, first_height) = (first.top, first.height);
                // Both inputs need their own points on the leading edge.
                let both_inputs = first.input().is_some()
                    && matches!(next, Left::Field { editor, .. } if editor.port().is_some());
                if both_inputs {
                    first.paired_input_anchor = Some(top + 9.0);
                }
                rows.push(Row {
                    top,
                    height: first_height.max(height),
                    left: Some(next),
                    right: None,
                    field_inset: middle,
                    field_end: None,
                    paired_input_anchor: both_inputs.then_some(top + 27.0),
                });
                if height > first_height {
                    y.set(y.get() + height - first_height);
                }
            }
        }
        for output in outputs {
            push(&mut rows, None, Some(output));
        }
        for add in growing {
            push(&mut rows, Some(add), None);
        }
        if crate::palette::signal(node).is_some() {
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
            size: Vec2::new(width, y.get() + END),
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
                paired_input_anchor: None,
            });
            y += LINE;
        }
        // Beside the meter: the outputs, then the dials side by side. Under it, the
        // stream input and every other setting, across the whole card.
        let middle = f32::midpoint(METER_FIELDS_X, METER_WIDTH - CARD_PADDING);
        let mut dial_top = y;
        let mut beside = y;
        let mut stream_y = BODY_TOP + spectrum::NOISE_CHART;
        let mut inputs = Vec::new();
        for port in node.ports.iter().filter(|port| port.input) {
            inputs.push(Row {
                top: stream_y,
                height: LINE,
                left: Some(Left::Port(port)),
                right: None,
                field_inset: CARD_PADDING,
                field_end: None,
                paired_input_anchor: None,
            });
            stream_y += LINE;
        }
        let mut under = stream_y;
        // Fields hinted `aside` (or `header`) sit beside the meter, in pairs when the
        // region is a row; every other field runs the card's width under it (ADR-33).
        let mut beside_column = 0;
        for (name, schema) in settings::fields(node) {
            use orange_host::controls::{Arrange, Region};
            let dial = crate::properties::dial_of(node, name);
            let editor = FieldOf::Setting(name, schema, dial);
            // A dial names itself, so it needs no name line.
            let name_line = if dial.is_some() { 0.0 } else { NAME };
            let height = name_line + editor.control_height() + GAP;
            let (top, inset, end) = match node.controls.region_of(name) {
                Some((Region::Aside | Region::Header, arrange)) => {
                    let paired = arrange != Arrange::Column;
                    if !paired || beside_column == 0 {
                        dial_top = beside;
                    }
                    beside = beside.max(dial_top + height);
                    let placed = if !paired {
                        (dial_top, METER_FIELDS_X, None)
                    } else if beside_column == 0 {
                        (dial_top, METER_FIELDS_X, Some(middle - 6.0))
                    } else {
                        (dial_top, middle, None)
                    };
                    beside_column = if paired { 1 - beside_column } else { 0 };
                    placed
                }
                _ => {
                    let top = under;
                    under += height;
                    (top, CARD_PADDING, None)
                }
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
                paired_input_anchor: None,
            });
        }
        rows.extend(inputs);
        let y = beside.max(under);
        Self {
            size: Vec2::new(METER_WIDTH, y + END),
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

fn field_name<'a>(left: &Left<'a>) -> Option<&'a str> {
    match left {
        Left::Field {
            editor: FieldOf::Setting(name, ..),
            ..
        } => Some(name),
        Left::Field { editor, .. } => Some(editor.port()?.id.as_str()),
        Left::Port(port) => Some(&port.id),
        _ => None,
    }
}

fn field_placement(node: &NodeView, left: &Left<'_>) -> Option<usize> {
    if !matches!(left, Left::Field { .. }) {
        return None;
    }
    let name = field_name(left)?;
    let (index, placement) = node
        .controls
        .regions
        .iter()
        .enumerate()
        .find(|(_, placement)| placement.fields.iter().any(|field| field == name))?;
    (placement.arrange != orange_host::controls::Arrange::Column).then_some(index)
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

    fn row_of<'c>(card: &'c Card<'_>, id: &str) -> &'c Row<'c> {
        card.rows
            .iter()
            .find(|row| match row.left {
                Some(Left::Field { editor, .. }) => match editor {
                    FieldOf::Setting(name, ..) => name == id,
                    other => other.port().is_some_and(|port| port.id == id),
                },
                _ => false,
            })
            .unwrap_or_else(|| panic!("no row for {id}"))
    }

    /// Map Range's hints (ADR-33) put each range's two ends side by side, and its
    /// value on the result's row.
    #[test]
    fn map_range_pairs_each_range_on_one_row() {
        let host = host();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        let id = host
            .add_node(&mut draft, "org.orange.math/map-range", [0.0, 0.0])
            .unwrap();
        // Typed, its inputs become fields that can be laid out.
        host.choose_type(&mut draft, id, "T", Some("core/f64@1"))
            .unwrap();
        let source = host
            .add_node(&mut draft, "core/number", [-600.0, 0.0])
            .unwrap();
        let graph = host.draft_view(&draft).unwrap();
        let node = graph.nodes.iter().find(|node| node.id == id).unwrap();
        let card = Card::new(node);
        let mut canvas = crate::canvas::Canvas::new();
        canvas.load(&graph);
        {
            let card = &card;
            for (low, high) in [("from_min", "from_max"), ("to_min", "to_max")] {
                let (low, high) = (row_of(card, low), row_of(card, high));
                assert_eq!(low.top, high.top, "a pair shares its row");
                assert!(low.field_end.is_some_and(|end| end < high.field_inset));
                assert!(high.input_anchor() - low.input_anchor() >= 16.0);
                for row in [low, high] {
                    let port = row.input().unwrap();
                    assert_eq!(
                        canvas.input_at(&graph, Pos2::new(0.0, row.input_anchor()), false, source),
                        Some((id, port.id.clone())),
                    );
                }
            }
            assert_ne!(row_of(card, "from_min").top, row_of(card, "to_min").top);
            assert!(
                row_of(card, "value")
                    .right
                    .is_some_and(|port| port.id == "result")
            );
        }
    }

    /// A setting hinted as a dial is a dial on any card, and names itself.
    #[test]
    fn a_dial_hint_makes_a_dial_on_an_ordinary_card() {
        card_of(orange_audio::GAIN, |_, card| {
            let row = row_of(card, "gain_db");
            assert!(matches!(
                row.left,
                Some(Left::Field {
                    editor: FieldOf::Setting(_, _, Some(_)),
                    named: false
                })
            ));
        });
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
