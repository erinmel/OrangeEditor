//! A card's rows, built once from its node (22 §3.1). Size, port positions, wires,
//! hit tests, painting and the controls all read this one model, so nothing else
//! places anything on a card.
use super::*;

/// Every card's width, in graph units; a literal's is narrower.
pub(super) const WIDTH: f32 = 232.0;
const LITERAL_WIDTH: f32 = 180.0;
/// A row of one line: a flow row, an output, an input without a field.
pub(super) const LINE: f32 = 26.0;
/// The name line of a field block.
const NAME: f32 = 18.0;
/// A field's control.
pub(super) const CONTROL: f32 = 24.0;
/// The space under a field block's control.
const GAP: f32 = 6.0;
/// The space under a card's last row.
pub(super) const END: f32 = 8.0;

/// What changes a value in a field block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum FieldOf<'a> {
    /// A value typed on an input that no wire feeds.
    Input(&'a PortView),
    /// A file or folder on an input, chosen with the system's chooser.
    Path(&'a PortView, &'a orange_host::PathInput),
    /// A setting of the node, by name and schema.
    Setting(&'a str, &'a serde_json::Value),
    /// The variable named by the setting at this JSON pointer.
    Variable(&'a str),
    /// The connection or device the node uses.
    Connection,
    /// One more instance of a port the node has as many of as it is given, such as
    /// a Fork's branches.
    AddPort(&'a orange_host::VariadicView),
}

impl FieldOf<'_> {
    /// The input this field edits, whose dot sits on the field's name line.
    pub(super) const fn port(&self) -> Option<&PortView> {
        match self {
            Self::Input(port) | Self::Path(port, _) => Some(port),
            _ => None,
        }
    }
}

/// The left side of a row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Left<'a> {
    /// On manual's Fire button.
    Fire,
    /// A port with only its name: the way in, a wired input, or an input nobody types.
    Port(&'a PortView),
    /// An editable value: a name line, unless `named` is false, and its control.
    Field { editor: FieldOf<'a>, named: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Row<'a> {
    /// From the card's top, in graph units.
    pub top: f32,
    pub height: f32,
    pub left: Option<Left<'a>>,
    /// An output on the row's first line.
    pub right: Option<&'a PortView>,
}

impl Row<'_> {
    /// The middle of the row's first line, where its dots are.
    pub(super) fn anchor(&self) -> f32 {
        match self.left {
            Some(Left::Field { named: true, .. }) => self.top + NAME / 2.0,
            Some(Left::Field { named: false, .. }) => self.top + CONTROL / 2.0,
            _ => self.top + LINE / 2.0,
        }
    }

    /// Where the row's control is, from the card's top-left, when it has one.
    pub(super) fn control(&self, width: f32) -> Option<Rect> {
        let Some(Left::Field { named, .. }) = self.left else {
            return None;
        };
        let top = self.top + if named { NAME } else { 0.0 };
        // A literal's output dot sits beside its field.
        let right = if self.right.is_some() && !named {
            width - CARD_PADDING - 10.0
        } else {
            width - CARD_PADDING
        };
        Some(Rect::from_min_max(
            Pos2::new(CARD_PADDING, top),
            Pos2::new(right, top + CONTROL),
        ))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Card<'a> {
    pub size: Vec2,
    pub rows: Vec<Row<'a>>,
    /// The node has nothing to show yet: its body says to set it up.
    pub empty: bool,
}

/// Whether a value of this input's type is typed on the card.
pub(super) fn typed(port: &PortView) -> bool {
    !port.exec
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
            ) || (!ty.starts_with("core/")
                && !ty.starts_with('$')
                // A stream is a connection between nodes, never typed.
                && !ty.split('@').next().is_some_and(|name| name.ends_with("/stream")))
        })
}

pub(super) fn literal(node: &NodeView) -> bool {
    matches!(
        node.definition.as_str(),
        "core/whole-number" | "core/number" | "core/true-or-false" | "core/text"
    )
}

impl<'a> Card<'a> {
    pub fn new(node: &'a NodeView) -> Self {
        let literal = literal(node);
        let width = if literal { LITERAL_WIDTH } else { WIDTH };
        let mut rows = Vec::new();
        let mut y = BODY_TOP;
        let mut push = |rows: &mut Vec<Row<'a>>, left: Option<Left<'a>>, right| {
            let height = match left {
                Some(Left::Field { named: true, .. }) => NAME + CONTROL + GAP,
                Some(Left::Field { named: false, .. }) => CONTROL + GAP,
                _ => LINE,
            };
            rows.push(Row {
                top: y,
                height,
                left,
                right,
            });
            y += height;
        };

        // Flow: the way in and the first way out share the first row.
        let mut ins: Vec<Left<'a>> = Vec::new();
        if node.definition == "core/on-manual" {
            ins.push(Left::Fire);
        }
        ins.extend(
            node.ports
                .iter()
                .filter(|port| port.exec && port.input)
                .map(Left::Port),
        );
        let outs: Vec<_> = node
            .ports
            .iter()
            .filter(|port| port.exec && !port.input)
            .collect();
        for index in 0..ins.len().max(outs.len()) {
            push(&mut rows, ins.get(index).copied(), outs.get(index).copied());
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
        for port in node.ports.iter().filter(|port| port.input && !port.exec) {
            let path = node.paths.iter().find(|path| path.port == port.id);
            lefts.push(match path {
                _ if port.connected => Left::Port(port),
                Some(path) => Left::Field {
                    editor: FieldOf::Path(port, path),
                    named: true,
                },
                None if typed(port) => Left::Field {
                    editor: FieldOf::Input(port),
                    named: !literal,
                },
                None => Left::Port(port),
            });
        }
        lefts.extend(settings::fields(node).map(|(name, schema)| Left::Field {
            editor: FieldOf::Setting(name, schema),
            named: true,
        }));
        // A port the node may have more of ends the card with a button that adds one.
        let growing: Vec<Left<'a>> = node
            .variadic
            .iter()
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
        let mut outputs = node
            .ports
            .iter()
            .filter(|port| !port.exec && !port.input)
            .peekable();
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
        let empty = rows.is_empty();
        if empty {
            push(&mut rows, None, None);
        }
        Self {
            size: Vec2::new(width, y + END),
            rows,
            empty,
        }
    }

    /// Where a port's dot is, from the card's top-left.
    pub fn offset(&self, port: &PortView) -> Option<Vec2> {
        self.rows.iter().find_map(|row| {
            let here = if port.input {
                match row.left {
                    Some(Left::Port(left)) => std::ptr::eq(left, port),
                    Some(Left::Field { editor, .. }) => {
                        editor.port().is_some_and(|left| std::ptr::eq(left, port))
                    }
                    _ => false,
                }
            } else {
                row.right.is_some_and(|right| std::ptr::eq(right, port))
            };
            here.then(|| Vec2::new(if port.input { 0.0 } else { self.size.x }, row.anchor()))
        })
    }

    /// The row an input sits on, where a dropped wire lands on it.
    pub fn row_of(&self, port: &PortView) -> Option<&Row<'a>> {
        self.rows.iter().find(|row| match row.left {
            Some(Left::Port(left)) => std::ptr::eq(left, port),
            Some(Left::Field { editor, .. }) => {
                editor.port().is_some_and(|left| std::ptr::eq(left, port))
            }
            _ => false,
        })
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
