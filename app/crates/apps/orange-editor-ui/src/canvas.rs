//! The graph canvas: nodes as cards with their ports, wires between them, panning,
//! zooming, selecting and moving nodes, and controls on the cards for the values of
//! inputs.
//!
//! It is a widget of its own. An application that has its own lists and panels shows
//! only this: it asks its host for a [`GraphView`], calls [`Canvas::show`] each frame
//! and acts on the [`CanvasResponse`]. Nothing here knows about files, flows or the
//! other panels of the editor.
use crate::i18n::{t, tf};
use egui::epaint::CubicBezierShape;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, UiBuilder, Vec2};
use orange_host::{GraphView, NodeId, NodeView, PortView};
use std::collections::{BTreeMap, BTreeSet};

/// Width of a node card, in graph units (one unit is one point at zoom 1).
pub const NODE_WIDTH: f32 = card::WIDTH;
/// Height of a card's title.
pub const HEADER_HEIGHT: f32 = 30.0;
/// Where the first row of ports starts: under the title, with a little room.
pub const BODY_TOP: f32 = HEADER_HEIGHT + 6.0;
/// Height of one row of ports.
pub const ROW_HEIGHT: f32 = card::LINE;
/// How round a card's corners are, in graph units.
const CARD_RADIUS: f32 = 8.0;

/// Time for a run pulse to travel from one execution port to the next.
const PULSE_SECONDS: f32 = 0.65;

/// Space under the last row.
pub const FOOTER_HEIGHT: f32 = card::END;
/// Distance between the columns and rows of an automatic layout.
pub const LAYOUT_GAP: Vec2 = Vec2::new(90.0, 36.0);
/// The zoom range.
pub const ZOOM_RANGE: std::ops::RangeInclusive<f32> = 0.2..=2.5;
const GRID: f32 = 24.0;
/// How near a wire the pointer has to be to take it, in points at zoom 1.
const WIRE_GRAB: f32 = 7.0;
/// How much one point of wheel movement zooms.
const WHEEL_ZOOM: f32 = 0.0015;
const PORT_RADIUS: f32 = 5.0;
/// Below this zoom, cards show their values without controls.
const CONTROLS_ZOOM: f32 = 0.75;
/// Space between a card's edge and what is inside it.
const CARD_PADDING: f32 = 10.0;
/// The choices of the enum types a document's inputs use, by type.
pub(crate) type Enums = std::collections::HashMap<String, Vec<orange_host::EnumChoice>>;

/// The choices of every enum type a port of `graph`, a variable or the document's
/// own types use.
pub(crate) fn enums_of(
    host: &orange_host::Host,
    draft: &orange_host::Draft,
    graph: &GraphView,
    declared: &orange_host::Declarations,
) -> Enums {
    let mut enums = Enums::new();
    let ports = graph
        .nodes
        .iter()
        .flat_map(|node| &node.ports)
        .filter(|port| !port.exec)
        .filter_map(|port| port.ty.clone());
    let variables = declared
        .variables
        .iter()
        .map(|variable| variable.ty.clone());
    let own = declared.types.iter().map(|decl| decl.ty().to_string());
    for ty in ports.chain(variables).chain(own) {
        if !ty.starts_with("core/")
            && !enums.contains_key(&ty)
            && let Some(choices) = host.enum_choices(draft, &ty)
        {
            let _ = enums.insert(ty, choices);
        }
    }
    enums
}

/// `value` of type `ty` as a person reads it: an enum's choice by its name.
pub(crate) fn value_text(enums: &Enums, ty: Option<&str>, value: &orange_host::Value) -> String {
    if let orange_host::Value::Enum { tag, payload: None } = value
        && let Some(choice) = ty
            .and_then(|ty| enums.get(ty))
            .and_then(|choices| choices.get(usize::try_from(*tag).ok()?))
    {
        return choice.name.clone();
    }
    orange_host::plain(value)
}

/// What conversions are called on their cards: the type they take and the type they
/// give, "Int to Text".
fn titles(graph: &GraphView) -> std::collections::HashMap<NodeId, String> {
    graph
        .nodes
        .iter()
        .filter(|node| node.definition == "core/convert")
        .filter_map(|node| {
            let ty = |input: bool| {
                node.ports
                    .iter()
                    .find(|port| port.input == input && !port.exec)
                    .and_then(|port| port.ty.clone())
            };
            Some((node.id, crate::conversion_name(&ty(true)?, &ty(false)?)))
        })
        .collect()
}

/// The size of a node's card in graph units: a title and one row for each input beside
/// an output. A node with no ports yet keeps one row, which says to set it up.
pub fn node_size(node: &NodeView) -> Vec2 {
    card::Card::new(node).size
}

fn comparison_formula(definition: &str) -> Option<&'static str> {
    orange_host::formula(definition).filter(|formula| formula.contains(['<', '>', '=']))
}

/// A port position from the same template used to paint the card.
pub fn port_offset(node: &NodeView, port: &str, input: bool) -> Option<Vec2> {
    let port = node
        .ports
        .iter()
        .find(|candidate| candidate.input == input && candidate.id == port)?;
    card::Card::new(node).offset(port)
}

mod card;
pub(crate) use settings::advanced as advanced_setting;
#[cfg(test)]
mod field_tests;
mod fields;
mod layout;
pub use layout::auto_layout;
mod geometry;
mod groups;
mod settings;
use geometry::{Geometry, near_wire, wire_points, wire_visible};
pub use groups::{GROUP_PAD, GROUP_TITLE, GroupEdit};

/// What the person did on the canvas this frame.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CanvasResponse {
    /// The trigger whose button was pressed.
    pub trigger: Option<NodeId>,
    /// Nodes were let go at new places: [`Canvas::places`] is worth saving.
    pub moved: bool,
    /// The problem marker was clicked.
    pub show_problems: bool,
    /// The marker for a failed execution step was clicked.
    pub show_runtime_log: bool,
    /// The node that was clicked.
    pub selected: Option<NodeId>,
    /// A value was written on an input with its control on the card, typed as a
    /// person types it.
    pub set_input: Option<(NodeId, String, String)>,
    /// A file or folder chooser was requested from an input on a node card.
    pub browse_path: Option<(NodeId, String)>,
    /// A device chooser was requested at a node card.
    pub configure: Option<(NodeId, Pos2)>,
    /// A schema-backed configuration field edited directly on the card.
    pub set_config: Option<(NodeId, String, serde_json::Value)>,
    /// "New variable…" was chosen in a card's variable picker: the node and the
    /// setting the new variable goes into.
    pub new_variable: Option<(NodeId, String)>,
    /// A device selected from an adapter-provided choice list.
    pub device: Option<(NodeId, Option<String>)>,
    /// One more instance of a variadic port was asked for on a card.
    pub add_port: Option<(NodeId, String)>,
    /// The selected nodes are to be copied, or cut, to the clipboard.
    pub copy: bool,
    pub cut: bool,
    /// Text was pasted on the canvas.
    pub paste: Option<String>,
    /// A node kind was dropped on the canvas from a list: the kind and where, in graph
    /// units.
    pub dropped: Option<(String, [f32; 2])>,
    /// A palette node was released over this existing wire.
    pub drop_wire: Option<WireEnds>,
    /// A wire was drawn from an output to an input: the two ends.
    pub connect: Option<((NodeId, String), (NodeId, String))>,
    /// The wires at a port were taken off: the node, the port and whether it is an input.
    pub disconnect: Option<(NodeId, String, bool)>,
    /// The selected wire was deleted: its two ends.
    pub remove_wire: Option<WireEnds>,
    /// A node was let go over a wire: the node and the wire it goes into.
    pub insert: Option<(NodeId, WireEnds)>,
    /// The selected nodes were deleted.
    pub remove: Vec<NodeId>,
    /// A wire was let go where no input takes it: the output it comes from and where,
    /// in graph units and on the screen, so a new node can be chosen to take it.
    pub loose: Option<((NodeId, String), [f32; 2], Pos2)>,
    /// Tab or Space was pressed over the canvas to add a node at the pointer: where, in
    /// graph units and on the screen.
    pub quick_add: Option<([f32; 2], Pos2)>,
    /// A group was changed or taken away on the canvas.
    pub group: Option<GroupEdit>,
    /// Ctrl+G was pressed: the selected nodes are to be grouped.
    pub group_selection: bool,
    /// The second mouse button was clicked: on what, and where in graph units and on
    /// the screen, for the right-click menu.
    pub context: Option<(crate::MenuTarget, [f32; 2], Pos2)>,
}

/// A value a run put on a port, short for the canvas and in full for the inspector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub short: String,
    pub full: String,
}

#[derive(Debug, Clone)]
struct PortSample {
    shown: Shown,
    epoch: u64,
    preview: bool,
}

/// A problem shown on the canvas, on the node or wire it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    /// An error stops the graph from running; anything else is a warning.
    pub error: bool,
    /// What the problem says, as a person reads it.
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
    /// By node and port name: the port a problem is about is ringed.
    pub ports: std::collections::HashMap<(NodeId, String), Mark>,
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

#[derive(Debug, Clone, Copy)]
enum PulseRoute {
    Sole,
    Port(&'static str),
}

#[derive(Debug, Clone, Copy)]
struct StepActivity {
    time: std::time::Instant,
    failed: bool,
    route: Option<PulseRoute>,
}

/// The view of one graph: where it is scrolled to, how far it is zoomed and where its
/// nodes are.
#[derive(Debug, Clone)]
pub struct Canvas {
    pub(crate) keys: crate::keymap::Keymap,
    pub(crate) shortcuts_enabled: bool,
    pub(crate) device_pickers: BTreeMap<NodeId, crate::card_sources::DevicePicker>,
    /// The document's variables, as (ID, name), for the pickers on cards.
    pub(crate) variables: Vec<(String, String)>,
    /// The choices of the enum types the document's inputs take.
    pub(crate) enums: Enums,
    /// The wheel scrolls instead of zooming (Ctrl+wheel then zooms), for trackpads.
    pub scroll_pans: bool,
    /// Nodes let go after a move land on the grid; Alt while dragging moves freely.
    pub snap: bool,
    /// The graph point at the top left corner of the canvas.
    origin: Pos2,
    zoom: f32,
    places: BTreeMap<NodeId, Pos2>,
    /// The selected nodes, and the one whose properties are shown when it is the only one.
    selection: BTreeSet<NodeId>,
    selected: Option<NodeId>,
    front: Option<NodeId>,
    /// Where a box selection started, in graph units.
    boxing: Option<Pos2>,
    /// Where the pointer is, in graph units, while it is over the canvas.
    pointer: Option<Pos2>,
    /// A number being dragged on a card, shown before it is let go and written.
    number_drag: Option<(NodeId, String, f32)>,
    /// Text being typed on a card, and whether its field has had the keys yet.
    typing: Option<(NodeId, String, String, bool)>,
    /// A whole number being dragged: its node, port, value before and pixels moved.
    /// A node was dragged and not yet let go.
    moving: bool,
    /// Show the whole graph at the next frame.
    fit: bool,
    wiring: Option<Wiring>,
    /// The wire under the pointer at the last frame, and the selected wire.
    hover_wire: Option<WireEnds>,
    selected_wire: Option<WireEnds>,
    /// Problems drawn on nodes and wires.
    marks: Marks,
    help_revision: Option<(u64, String)>,
    descriptions: std::collections::HashMap<String, String>,
    labels: std::collections::HashMap<String, String>,
    /// Titles of nodes named by what they hold rather than their kind: a conversion is
    /// "Int to Text".
    titles: std::collections::HashMap<NodeId, String>,
    connection_labels: std::collections::HashMap<String, String>,
    /// When each node of a running flow last took a step, and whether it failed.
    activity: std::collections::HashMap<NodeId, StepActivity>,
    /// The latest value a running flow put on each output, as a person reads it.
    values: std::collections::HashMap<NodeId, std::collections::HashMap<String, PortSample>>,
    value_epoch: u64,
    /// Nodes whose last step failed, with why; kept until the node succeeds or a new
    /// run starts.
    failures: std::collections::HashMap<NodeId, String>,
    /// The values and failures are from a run that has ended or a graph since changed.
    stale: bool,
    /// The size of the canvas when it was last shown, in points.
    size: Vec2,
    /// The document's groups, the nodes hidden in collapsed ones and a group being
    /// renamed with what is typed.
    groups: Vec<orange_host::Group>,
    hidden: std::collections::HashSet<NodeId>,
    renaming: Option<(String, String)>,
}
impl Default for Canvas {
    fn default() -> Self {
        Self {
            keys: crate::keymap::Keymap::default(),
            shortcuts_enabled: true,
            device_pickers: BTreeMap::new(),
            variables: Vec::new(),
            enums: Enums::new(),
            origin: Pos2::new(-40.0, -40.0),
            zoom: 1.0,
            places: BTreeMap::new(),
            selection: BTreeSet::new(),
            selected: None,
            front: None,
            boxing: None,
            pointer: None,
            number_drag: None,
            typing: None,
            moving: false,
            fit: true,
            wiring: None,
            hover_wire: None,
            selected_wire: None,
            marks: Marks::default(),
            activity: std::collections::HashMap::new(),
            help_revision: None,
            descriptions: std::collections::HashMap::new(),
            labels: std::collections::HashMap::new(),
            titles: std::collections::HashMap::new(),
            connection_labels: std::collections::HashMap::new(),
            values: std::collections::HashMap::new(),
            failures: std::collections::HashMap::new(),
            stale: false,
            value_epoch: 0,
            size: Vec2::new(800.0, 600.0),
            scroll_pans: false,
            snap: true,
            groups: Vec::new(),
            hidden: std::collections::HashSet::new(),
            renaming: None,
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
            grid: mix(visuals.extreme_bg_color, visuals.text_color(), 0.21),
            card: visuals.window_fill,
            header: visuals.faint_bg_color,
            outline: visuals.widgets.noninteractive.bg_stroke.color,
            selected: visuals.selection.stroke.color,
            text: visuals.strong_text_color(),
            weak: visuals.weak_text_color(),
            exec: if dark {
                Color32::from_rgb(247, 187, 105)
            } else {
                Color32::from_rgb(142, 79, 10)
            },
        }
    }
}

/// The color of a value port, by the family of its type. The shape of the dot, not
/// the color, tells a value from a step.
pub(crate) fn type_color(ty: Option<&str>) -> Color32 {
    match ty.unwrap_or_default().split('@').next().unwrap_or_default() {
        "core/string" => Color32::from_rgb(214, 120, 170),
        "core/bool" => Color32::from_rgb(68, 177, 160),
        "core/i32" | "core/i64" | "core/u32" | "core/u64" | "core/f32" | "core/f64"
        | "core/duration" | "core/timestamp" => Color32::from_rgb(96, 160, 222),
        "core/list" | "core/map" | "core/stream" | "core/option" | "core/bytes" => {
            Color32::from_rgb(170, 132, 214)
        }
        other if other.starts_with("core/") || other.is_empty() => Color32::from_rgb(150, 160, 170),
        // The graph's own types share one colour; a package's types share one picked
        // from its namespace, so no package needs the editor to know it.
        other if other.starts_with("local/") => Color32::from_rgb(224, 178, 72),
        other => {
            const PACKAGE_COLORS: [Color32; 5] = [
                Color32::from_rgb(170, 132, 214),
                Color32::from_rgb(120, 190, 120),
                Color32::from_rgb(230, 150, 110),
                Color32::from_rgb(110, 180, 210),
                Color32::from_rgb(210, 120, 120),
            ];
            let namespace = other.split('/').next().unwrap_or(other);
            let hash = namespace.bytes().fold(2_166_136_261u32, |hash, byte| {
                (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
            });
            PACKAGE_COLORS[hash as usize % PACKAGE_COLORS.len()]
        }
    }
}

impl Canvas {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn update_help(&mut self, host: &orange_host::Host, language: &str) {
        let generation = host.registry().generation();
        if self
            .help_revision
            .as_ref()
            .is_some_and(|(old, locale)| *old == generation && locale == language)
        {
            return;
        }
        let entries = host.search_nodes("", language);
        self.labels = entries
            .iter()
            .map(|item| (item.definition.clone(), item.label.clone()))
            .collect();
        self.descriptions = entries
            .into_iter()
            .filter(|item| !item.purpose.is_empty())
            .map(|item| (item.definition, item.purpose))
            .collect();
        crate::i18n::set_port_names(host.port_names(language));
        self.help_revision = Some((generation, language.to_owned()));
        self.connection_labels = host
            .nodes()
            .filter_map(|item| {
                let id = item.definition.definition.id.as_str();
                Some((id.to_owned(), host.connection_label(id)?))
            })
            .collect();
    }

    /// What a node kind does, in the editor's language, when its package says.
    pub(crate) fn description(&self, definition: &str) -> Option<&str> {
        self.descriptions.get(definition).map(String::as_str)
    }

    pub(crate) fn display_name<'a>(&'a self, node: &'a NodeView) -> &'a str {
        if let Some(title) = self.titles.get(&node.id) {
            return title;
        }
        if node.name == node.kind {
            self.kind_name(node)
        } else {
            &node.name
        }
    }

    /// What a node's kind is called in the editor's language, such as "Escribir
    /// archivo de texto".
    pub(crate) fn kind_name<'a>(&'a self, node: &'a NodeView) -> &'a str {
        self.labels
            .get(&node.definition)
            .map_or(node.kind.as_str(), String::as_str)
    }

    /// Takes the places of a graph's nodes: saved ones, and automatic ones for the
    /// rest. Call it when a document is opened or replaced.
    pub fn load(&mut self, graph: &GraphView) {
        self.titles = titles(graph);
        self.places = auto_layout(graph);
        self.forget_missing();
        self.fit = true;
    }

    /// Follows an edit of the graph: nodes that are new take the place the document
    /// gives them or an automatic one, nodes that are gone are forgotten, and every
    /// other node stays where it is. The view is not moved.
    pub fn update(&mut self, graph: &GraphView) {
        self.titles = titles(graph);
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
            .filter(|id| self.places.contains_key(id) && !self.hidden.contains(id))
            .collect();
        self.selected = match self.selection.len() {
            1 => self.selection.first().copied(),
            _ => None,
        };
        if self.selected.is_some() {
            self.front = self.selected;
        }
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
            .filter(|node| !self.hidden.contains(&node.id))
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
        for node in node_order(graph, self.front).rev() {
            let Some(place) = self.places.get(&node.id).copied() else {
                continue;
            };
            if node.id == from || self.hidden.contains(&node.id) {
                continue;
            }
            for port in node
                .ports
                .iter()
                .filter(|port| port.input && port.exec == exec)
            {
                let layout = card::Card::new(node);
                let Some(found) = layout.row_of(port) else {
                    continue;
                };
                // From a little left of the dot across the input's whole row.
                let row = Rect::from_min_max(
                    place + Vec2::new(-14.0, found.top),
                    place + Vec2::new(layout.size.x, found.top + found.height),
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

    /// Records a recent step for canvas feedback and the inspector.
    pub fn pulse(&mut self, node: NodeId, failed: bool) {
        self.record_step(node, failed, orange_host::ControlOutcome::Then);
    }

    pub(crate) fn record_step(
        &mut self,
        node: NodeId,
        failed: bool,
        outcome: orange_host::ControlOutcome,
    ) {
        use orange_host::ControlOutcome::{Body, Completed, False, Then, True};
        let route = if failed {
            None
        } else {
            match outcome {
                Then | Completed => Some(PulseRoute::Sole),
                True => Some(PulseRoute::Port("true")),
                False => Some(PulseRoute::Port("false")),
                Body => Some(PulseRoute::Port("body")),
                _ => None,
            }
        };
        let _ = self.activity.insert(
            node,
            StepActivity {
                time: std::time::Instant::now(),
                failed,
                route,
            },
        );
    }

    /// A running flow put a value on an output: `short` shows beside the port, `full`
    /// in the inspector. It stays until another comes; when the run ends it is dimmed.
    pub fn show_value(&mut self, node: NodeId, port: String, short: String, full: String) {
        self.stale = false;
        let _ = self.values.entry(node).or_default().insert(
            port,
            PortSample {
                shown: Shown { short, full },
                epoch: self.value_epoch,
                preview: false,
            },
        );
    }

    /// Authoring samples stay current independently of a running flow.
    pub(crate) fn show_preview(&mut self, node: NodeId, port: String, short: String, full: String) {
        self.show_value(node, port.clone(), short, full);
        if let Some(sample) = self
            .values
            .get_mut(&node)
            .and_then(|ports| ports.get_mut(&port))
        {
            sample.preview = true;
        }
    }

    pub(crate) fn clear_previews(&mut self) {
        self.values.retain(|_, ports| {
            ports.retain(|_, sample| !sample.preview);
            !ports.is_empty()
        });
    }

    /// A node's step failed, for `why`: the node keeps an error badge until it
    /// succeeds or a new run starts.
    pub fn fail(&mut self, node: NodeId, why: String) {
        let _ = self.failures.insert(node, why);
    }

    /// A node took a step that did not fail: its earlier failure is over.
    pub fn recover(&mut self, node: NodeId) {
        let _ = self.failures.remove(&node);
    }

    /// Why the node's last step failed, if it did.
    pub fn failure(&self, node: NodeId) -> Option<&str> {
        self.failures.get(&node).map(String::as_str)
    }

    /// The run the values came from has ended, or its graph changed: they stay, dimmed
    /// and said to be from the last run, until a new run starts.
    pub fn mark_stale(&mut self) {
        self.stale = true;
        self.value_epoch = self.value_epoch.wrapping_add(1);
        if self.value_epoch == 0 {
            self.values.clear();
        }
    }

    /// Whether the run or graph changed since the last output value was received.
    pub fn is_stale(&self) -> bool {
        self.stale
    }

    /// The value last on a port: on an output, what the node put there; on a wired
    /// input, what the output it is wired to holds.
    pub fn port_value(
        &self,
        graph: &GraphView,
        node: NodeId,
        port: &str,
        input: bool,
    ) -> Option<&Shown> {
        self.port_sample(graph, node, port, input)
            .map(|(shown, _)| shown)
    }

    pub(crate) fn port_sample(
        &self,
        graph: &GraphView,
        node: NodeId,
        port: &str,
        input: bool,
    ) -> Option<(&Shown, bool)> {
        let (node, port) = if input {
            let wire = graph
                .wires
                .iter()
                .find(|wire| wire.to.0 == node && wire.to.1 == port)?;
            (wire.from.0, wire.from.1.as_str())
        } else {
            (node, port)
        };
        let sample = self.output_sample(node, port)?;
        Some((
            &sample.shown,
            !sample.preview && sample.epoch != self.value_epoch,
        ))
    }

    fn output_sample(&self, node: NodeId, port: &str) -> Option<&PortSample> {
        self.values.get(&node)?.get(port)
    }

    fn frame_port_value(
        &self,
        geometry: &Geometry<'_>,
        node: NodeId,
        port: &str,
        input: bool,
    ) -> Option<&PortSample> {
        let (node, port) = if input {
            geometry.source(node, port)?
        } else {
            (node, port)
        };
        self.output_sample(node, port)
    }

    /// Whether a running flow's node has taken a step, and whether that step failed.
    pub fn last_step(&self, node: NodeId) -> Option<bool> {
        self.activity.get(&node).map(|step| step.failed)
    }

    /// The value shown beside an output, if any.
    pub fn shown_value(&self, node: NodeId, port: &str) -> Option<&str> {
        self.output_sample(node, port)
            .map(|sample| sample.shown.short.as_str())
    }

    /// A new run starts: nothing from an earlier one is shown.
    pub fn clear_run(&mut self) {
        self.activity.clear();
        self.values.clear();
        self.failures.clear();
        self.stale = false;
    }

    pub(crate) fn begin_run(&mut self) {
        self.activity.clear();
        self.failures.clear();
        self.stale = false;
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
        self.show_with_path_controls(ui, graph, triggers, false)
    }

    pub(crate) fn show_with_path_controls(
        &mut self,
        ui: &mut egui::Ui,
        graph: &GraphView,
        triggers: &[NodeId],
        file_dialogs: bool,
    ) -> CanvasResponse {
        crate::icons::install(ui.ctx());
        let mut done = CanvasResponse::default();
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let area = response.rect;
        let previous_size = std::mem::replace(&mut self.size, area.size());
        let colors = Palette::of(ui);
        if self.fit {
            self.fit = false;
            if let Some(bounds) = self.bounds(graph) {
                (self.zoom, self.origin) = Self::frame(bounds, area.size());
            }
        } else {
            // Resizing the window or a dock keeps the same graph point in view.
            self.origin += (previous_size - self.size) / (2.0 * self.zoom);
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
        // A right-click on the background; a wire under it is found as wires are drawn.
        let background_menu = response
            .secondary_clicked()
            .then(|| response.interact_pointer_pos())
            .flatten();
        let menu_at = |screen: Pos2, origin: Pos2, zoom: f32| {
            let at = graph_point(screen, origin, zoom);
            ([at.x, at.y], screen)
        };
        self.pointer = response
            .hover_pos()
            .map(|at| graph_point(at, self.origin, self.zoom));
        // A node kind dragged from a list and let go here.
        let palette_item = response.dnd_hover_payload::<crate::palette::PaletteDrag>();
        let dropped = if palette_item.is_some() {
            response
                .dnd_release_payload::<crate::palette::PaletteDrag>()
                .map(|drag| drag.item.id.clone())
        } else {
            response
                .dnd_release_payload::<String>()
                .map(|kind| (*kind).clone())
        };
        if let (Some(kind), Some(at)) = (dropped, ui.input(|input| input.pointer.interact_pos())) {
            let mut at = graph_point(at, self.origin, self.zoom);
            if palette_item.is_some() && self.snap && !ui.input(|input| input.modifiers.alt) {
                at = snapped(at);
            }
            done.dropped = Some((kind, [at.x, at.y]));
        }
        if let Some(pointer) = response.hover_pos() {
            let (mut scroll, mut factor, modifiers) = ui.input(|input| {
                (
                    input.smooth_scroll_delta,
                    input.zoom_delta(),
                    input.modifiers,
                )
            });
            // The wheel zooms, as in most node editors; Shift scrolls sideways and Ctrl
            // up and down. `scroll_pans` gives the wheel back to scrolling (trackpads).
            if !self.scroll_pans && (factor - 1.0).abs() <= f32::EPSILON && scroll != Vec2::ZERO {
                if modifiers.shift {
                    scroll = Vec2::new(scroll.x + scroll.y, 0.0);
                } else if !modifiers.command {
                    factor = (scroll.y * WHEEL_ZOOM).exp();
                    scroll = Vec2::ZERO;
                }
            }
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
        if step >= 14.0 {
            // Batch the dots into one mesh. Their positions match the snap grid.
            let mut dots = egui::Mesh::default();
            let first_x = area.min.x - (origin.x * zoom).rem_euclid(step);
            let mut y = area.min.y - (origin.y * zoom).rem_euclid(step);
            while y < area.max.y {
                let mut x = first_x;
                while x < area.max.x {
                    let dot = Rect::from_center_size(Pos2::new(x, y), Vec2::splat(1.35));
                    dots.add_colored_rect(dot, colors.grid);
                    x += step;
                }
                y += step;
            }
            painter.add(egui::Shape::mesh(dots));
        }

        // Groups: their title bars are handled under the cards, and their frames drawn
        // behind them.
        let screen = |rect: Rect| Rect::from_min_max(to_screen(rect.min), to_screen(rect.max));
        self.group_input(ui, graph, screen, response.id, &mut done);
        self.draw_groups(&painter, graph, screen, &colors);
        let collapsed = self.collapsed_frames(graph);

        // Nodes are handled before anything is drawn, so a wire follows the node
        // being moved in the same frame.
        for node in node_order(graph, self.front) {
            let Some(place) = self.places.get(&node.id).copied() else {
                continue;
            };
            if self.hidden.contains(&node.id) {
                continue;
            }
            let rect = Rect::from_min_size(to_screen(place), geometry.size(node.id) * zoom);
            if !area.intersects(rect) {
                continue;
            }
            let touched = ui.interact(rect, response.id.with(node.id), Sense::click_and_drag());
            if touched.drag_started() && !self.selection.contains(&node.id) {
                self.select(Some(node.id));
            }
            if touched.dragged() {
                self.front = Some(node.id);
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
                // One node let go over a wire that is not its own goes into it.
                if self.selection.len() == 1
                    && let Some(wire) = self.hover_wire.clone()
                    && wire.0.0 != node.id
                    && wire.1.0 != node.id
                {
                    done.insert = Some((node.id, wire));
                }
                let free = ui.input(|input| input.modifiers.alt);
                if self.snap && !free {
                    for id in &self.selection {
                        if let Some(place) = self.places.get_mut(id) {
                            *place = snapped(*place);
                        }
                    }
                }
            }
            if touched.clicked() {
                if shift || command {
                    self.toggle(node.id);
                } else {
                    self.select(Some(node.id));
                }
                done.selected = self.selected;
            }
            if touched.secondary_clicked()
                && let Some(screen) = touched.interact_pointer_pos()
            {
                let (at, screen) = menu_at(screen, self.origin, self.zoom);
                self.front = Some(node.id);
                done.context = Some((crate::MenuTarget::Node(node.id), at, screen));
            }
        }

        // Ports: dragging from an output draws a wire; the second mouse button opens
        // the port's menu.
        let grab = (PORT_RADIUS + 5.0) * zoom.max(0.6);
        let front = self.front;
        let front_rect = front
            .filter(|id| !self.hidden.contains(id) && graph.nodes.iter().any(|node| node.id == *id))
            .and_then(|id| {
                self.places
                    .get(&id)
                    .map(|place| Rect::from_min_size(to_screen(*place), geometry.size(id) * zoom))
            });
        for node in node_order(graph, front) {
            let Some(place) = self.places.get(&node.id).copied() else {
                continue;
            };
            if self.hidden.contains(&node.id) {
                continue;
            }
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
                if front != Some(node.id) && front_rect.is_some_and(|rect| rect.contains(at)) {
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
                if touched.secondary_clicked() {
                    let target = crate::MenuTarget::Port {
                        node: node.id,
                        port: port.id.clone(),
                        input: port.input,
                    };
                    let (at, screen) = menu_at(at, origin, zoom);
                    done.context = Some((target, at, screen));
                }
                if touched.hovered() && self.wiring.is_none() {
                    let label = port_title(port);
                    let detail = if port.exec {
                        Some(t("port-execution").to_owned())
                    } else {
                        port.ty.as_deref().map(|ty| {
                            if ty.starts_with('$') {
                                port_type_label(&node.definition, ty)
                            } else {
                                crate::type_name(ty)
                            }
                        })
                    };
                    let hint = match detail {
                        Some(ty) => format!("{label} · {ty}"),
                        None => label,
                    };
                    let _ = touched.on_hover_text(hint);
                }
            }
        }
        let pointer = ui
            .input(|input| input.pointer.interact_pos())
            .filter(|at| area.contains(*at));
        if let Some(wiring) = self.wiring.clone()
            && ui.input(|input| input.pointer.any_released())
        {
            if let Some(pointer) = pointer {
                let at = origin + (pointer - area.min) / zoom;
                done.connect = self
                    .input_at(graph, at, wiring.exec, wiring.from.0)
                    .map(|to| (wiring.from.clone(), to));
                // Let go on the background, not on a node: ask for a node to take it.
                let on_node = self.nodes_in(graph, Rect::from_center_size(at, Vec2::splat(1.0)));
                if done.connect.is_none() && on_node.is_empty() && area.contains(pointer) {
                    done.loose = Some((wiring.from, [at.x, at.y], pointer));
                }
            }
            self.wiring = None;
        }
        // Keys, unless a text field has them: Delete removes the selection, Ctrl+A
        // selects everything, and copy, cut and paste go through the clipboard.
        if self.shortcuts_enabled
            && ui.input(|input| input.focused)
            && !ui.ctx().egui_wants_keyboard_input()
        {
            let (delete, all, add, events) = ui.input(|input| {
                done.group_selection =
                    self.keys.pressed(input, "group") && !self.selection.is_empty();
                (
                    self.keys.pressed(input, "delete"),
                    self.keys.pressed(input, "select-all"),
                    self.keys.pressed(input, "add"),
                    input.events.clone(),
                )
            });
            done.copy =
                ui.input(|input| self.keys.pressed(input, "copy")) && !self.selection.is_empty();
            done.cut =
                ui.input(|input| self.keys.pressed(input, "cut")) && !self.selection.is_empty();
            if ui.input(|input| self.keys.pressed(input, "paste")) {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            }
            if add && let Some(screen) = response.hover_pos() {
                let at = graph_point(screen, self.origin, self.zoom);
                done.quick_add = Some(([at.x, at.y], screen));
            }
            if delete && let Some(wire) = self.selected_wire.take() {
                done.remove_wire = Some(wire);
            } else if delete && !self.selection.is_empty() {
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
            if self.hidden.contains(&node) {
                return self.hidden_anchor(&collapsed, node, input).map(to_screen);
            }
            let place = *self.places.get(&node)?;
            Some(to_screen(place + geometry.port(node, port, input)?.offset))
        };
        let mut nearest: Option<(f32, WireEnds)> = None;
        let mut pulsing = false;
        for wire in &graph.wires {
            if self.wire_hidden(wire) {
                continue;
            }
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
            if done.context.is_none()
                && let Some(click) = background_menu
                && near_wire(points, click, 6.0 * zoom.max(0.6))
            {
                let (at, screen) = menu_at(click, origin, zoom);
                let ends = (wire.from.clone(), wire.to.clone());
                done.context = Some((crate::MenuTarget::Wire(ends), at, screen));
            }
            let width = if wire.exec { 3.0 } else { 1.8 } * zoom.max(0.6)
                + if mark.is_some() {
                    1.5 * zoom.max(0.6)
                } else {
                    0.0
                };
            if !wire_visible(&points, painter.clip_rect(), width) {
                continue;
            }
            let ends = (wire.from.clone(), wire.to.clone());
            if let Some(at) = pointer
                && Rect::from_points(&points)
                    .expand(WIRE_GRAB * zoom.max(0.6))
                    .contains(at)
            {
                let away = bezier_distance(&points, at);
                if nearest.as_ref().is_none_or(|(best, _)| away < *best) {
                    nearest = Some((away, ends.clone()));
                }
            }
            // The wire under the pointer thickens; the selected one is drawn in the
            // selection color.
            let hovered = self.hover_wire.as_ref() == Some(&ends);
            let picked = self.selected_wire.as_ref() == Some(&ends);
            let (width, color) = match (picked, hovered) {
                (true, _) => (width + 2.0 * zoom.max(0.6), colors.selected),
                (false, true) => (width + 1.5 * zoom.max(0.6), color),
                _ => (width, color),
            };
            let curve = CubicBezierShape::from_points_stroke(
                points,
                false,
                Color32::TRANSPARENT,
                Stroke::new(width, color),
            );
            let passage = if wire.exec
                && let Some(step) = self.activity.get(&wire.from.0)
            {
                let port = match step.route {
                    Some(PulseRoute::Sole) => geometry.sole_exec_output(wire.from.0),
                    Some(PulseRoute::Port(port)) => Some(port),
                    None => None,
                };
                let progress = step.time.elapsed().as_secs_f32() / PULSE_SECONDS;
                (port == Some(wire.from.1.as_str()) && progress < 1.0)
                    .then(|| curve.sample(progress))
            } else {
                None
            };
            painter.add(curve);
            if let Some(at) = passage {
                painter.circle_filled(at, 4.0 * zoom.max(0.65), colors.text);
                pulsing = true;
            }
        }

        if done.context.is_none()
            && let Some(click) = background_menu
        {
            let (at, screen) = menu_at(click, origin, zoom);
            done.context = Some((crate::MenuTarget::Canvas, at, screen));
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
        self.hover_wire = nearest
            .filter(|(away, _)| *away <= WIRE_GRAB * zoom.max(0.6))
            .map(|(_, ends)| ends);
        if palette_item.is_some() && done.dropped.is_some() {
            done.drop_wire.clone_from(&self.hover_wire);
        }
        // A click on the background selects the wire under it, or none.
        if response.clicked() {
            self.selected_wire = self.hover_wire.clone();
        }
        if self.selected_wire.as_ref().is_some_and(|ends| {
            !graph
                .wires
                .iter()
                .any(|w| (&w.from, &w.to) == (&ends.0, &ends.1))
        }) {
            self.selected_wire = None;
        }

        // Each card owns its marks. A card in front covers the card and its marks.
        let mut port_marks = BTreeMap::<_, Vec<_>>::new();
        for ((node, port), mark) in &self.marks.ports {
            for input in [true, false] {
                if let Some(found) = geometry.port(*node, port, input) {
                    port_marks
                        .entry(*node)
                        .or_default()
                        .push((found.offset, mark.error));
                }
            }
        }
        for node in node_order(graph, front) {
            let Some(place) = self.places.get(&node.id).copied() else {
                continue;
            };
            if self.hidden.contains(&node.id) {
                continue;
            }
            let rect = Rect::from_min_size(to_screen(place), geometry.size(node.id) * zoom);
            if !area.intersects(rect) {
                continue;
            }
            let selected = self.selection.contains(&node.id);
            // On manual always has its button; it fires only while the graph runs.
            let fire = (node.definition == "core/on-manual" || triggers.contains(&node.id))
                .then(|| triggers.contains(&node.id));
            let controls = zoom >= CONTROLS_ZOOM;
            // An operator says what it computes, with values in place of names.
            let formula = comparison_formula(&node.definition).map(|formula| {
                fill_formula(formula, |port| {
                    node.ports
                        .iter()
                        .find(|p| p.input && p.id == port)
                        .and_then(|p| {
                            if p.connected {
                                self.frame_port_value(&geometry, node.id, port, true)
                                    .filter(|sample| sample.epoch == self.value_epoch)
                                    .map(|sample| sample.shown.short.clone())
                            } else {
                                p.value.clone()
                            }
                        })
                })
            });
            let connection = self
                .connection_labels
                .get(&node.definition)
                .map(String::as_str);
            let marked =
                self.failures.contains_key(&node.id) || self.marks.nodes.contains_key(&node.id);
            let (pressed, slots) = draw_node(
                ui,
                &painter,
                node,
                (self.display_name(node), self.kind_name(node)),
                rect,
                zoom,
                &colors,
                (selected, fire, controls, marked),
                response.id,
                formula.as_deref(),
                self.descriptions.get(&node.definition).map(String::as_str),
                connection,
            );
            if pressed {
                done.trigger = Some(node.id);
            }
            // Values a run put on ports: before each output's name, and after the name
            // of each wired input. Cut short here; the inspector has them whole.
            let layout = card::Card::new(node);
            let label_font = FontId::proportional(12.5 * zoom);
            let inset = CARD_PADDING * zoom;
            for row in &layout.rows {
                let wired = match row.left {
                    Some(card::Left::Port(port)) if !port.exec => Some(port),
                    _ => None,
                };
                let output = row.right.filter(|port| !port.exec && !card::literal(node));
                for (port, input) in [(wired, true), (output, false)] {
                    let Some(port) = port else { continue };
                    let Some(value) = self.frame_port_value(&geometry, node.id, &port.id, input)
                    else {
                        continue;
                    };
                    if zoom < 0.6 {
                        continue;
                    }
                    let name =
                        painter.layout_no_wrap(port_title(port), label_font.clone(), colors.text);
                    let name_width = name.size().x.min(rect.width() / 2.0 - 1.5 * inset);
                    let y = rect.top() + row.anchor() * zoom;
                    let (at, align, room) = if input {
                        let room = if output.is_some() {
                            rect.width() / 2.0
                        } else {
                            rect.width()
                        } - name_width
                            - 3.0 * inset;
                        (
                            Pos2::new(rect.left() + inset + name_width + 8.0 * zoom, y),
                            Align2::LEFT_CENTER,
                            room,
                        )
                    } else {
                        (
                            Pos2::new(rect.right() - inset - name_width - 8.0 * zoom, y),
                            Align2::RIGHT_CENTER,
                            rect.width() / 2.0 - 2.0 * inset,
                        )
                    };
                    if room < 24.0 * zoom {
                        continue;
                    }
                    let font = FontId::monospace(11.5 * zoom);
                    let tint = if value.preview || value.epoch == self.value_epoch {
                        colors.text
                    } else {
                        colors.weak
                    };
                    let (shown, _) = elide(&painter, &value.shown.short, &font, room);
                    let galley = painter.layout_no_wrap(shown, font, tint);
                    let bounds = align.anchor_rect(Rect::from_min_size(at, galley.size()));
                    painter.rect_filled(
                        bounds.expand2(Vec2::new(5.0, 2.0) * zoom),
                        3.0 * zoom,
                        colors.background,
                    );
                    painter.galley(bounds.min, galley, tint);
                }
            }
            for (field, slot) in slots {
                if !area.contains_rect(slot)
                    || (front != Some(node.id)
                        && front_rect.is_some_and(|rect| rect.contains(slot.center())))
                {
                    continue;
                }
                let id = response.id.with(("field", node.id, slot.min.y as i32 / 4));
                let Some(edit) = self.field(ui, node, field, slot, id, file_dialogs) else {
                    continue;
                };
                self.front = Some(node.id);
                match edit {
                    fields::FieldEdit::Input(port, text) => {
                        done.set_input = Some((node.id, port, text));
                    }
                    fields::FieldEdit::Setting(pointer, value) => {
                        done.set_config = Some((node.id, pointer, value));
                    }
                    fields::FieldEdit::NewVariable(pointer) => {
                        done.new_variable = Some((node.id, pointer));
                    }
                    fields::FieldEdit::Device(device) => done.device = Some((node.id, device)),
                    fields::FieldEdit::Configure(at) => done.configure = Some((node.id, at)),
                    fields::FieldEdit::Browse(port) => {
                        done.browse_path = Some((node.id, port));
                    }
                    fields::FieldEdit::AddPort(port) => done.add_port = Some((node.id, port)),
                }
            }
            let node = node.id;
            for &(offset, error) in port_marks.get(&node).into_iter().flatten() {
                painter.circle_stroke(
                    rect.min + offset * zoom,
                    (PORT_RADIUS + 2.5) * zoom,
                    Stroke::new(1.5 * zoom.max(0.6), colors.problem(error)),
                );
            }
            if front != Some(node)
                && front_rect.is_some_and(|front| front.contains(mark_position(rect, zoom)))
            {
                continue;
            }
            if let Some(why) = self.failures.get(&node) {
                let failed = Mark {
                    error: true,
                    text: tf("failed-last-run", &[("why", why.as_str().into())]),
                };
                done.show_runtime_log |= draw_mark(
                    ui,
                    &painter,
                    rect,
                    zoom,
                    &colors,
                    &failed,
                    response.id.with(("failed", node)),
                );
            } else if let Some(mark) = self.marks.nodes.get(&node) {
                done.show_problems |= draw_mark(
                    ui,
                    &painter,
                    rect,
                    zoom,
                    &colors,
                    mark,
                    response.id.with(("mark", node)),
                );
            }
        }
        if pulsing {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
        // While a wire is drawn, the inputs it could go to are ringed.
        if let Some(wiring) = &self.wiring {
            let source = geometry
                .port(wiring.from.0, &wiring.from.1, false)
                .and_then(|port| port.ty);
            for node in graph
                .nodes
                .iter()
                .filter(|node| node.id != wiring.from.0 && !self.hidden.contains(&node.id))
            {
                let Some(place) = self.places.get(&node.id).copied() else {
                    continue;
                };
                for port in node.ports.iter().filter(|port| port.input) {
                    if port.exec != wiring.exec || (!port.exec && !fits(source, port.ty.as_deref()))
                    {
                        continue;
                    }
                    let Some(found) = geometry.port(node.id, &port.id, true) else {
                        continue;
                    };
                    let at = to_screen(place + found.offset);
                    if area.contains(at) {
                        painter.circle_stroke(
                            at,
                            (PORT_RADIUS + 3.0) * zoom.max(0.6),
                            Stroke::new(2.0 * zoom.max(0.6), colors.selected),
                        );
                    }
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
        if let (None, Some(drag), Some(pointer)) = (&done.dropped, palette_item, pointer) {
            let mut place = graph_point(pointer, origin, zoom);
            if self.snap && !ui.input(|input| input.modifiers.alt) {
                place = snapped(place);
            }
            let at = to_screen(place);
            if let Some(card) = &drag.card {
                let rect = Rect::from_min_size(at, node_size(card) * zoom);
                let _ = draw_node(
                    ui,
                    &painter,
                    card,
                    (self.display_name(card), self.kind_name(card)),
                    rect,
                    zoom,
                    &colors,
                    (true, None, false, false),
                    response.id,
                    None,
                    None,
                    None,
                );
            } else {
                let rect = Rect::from_min_size(at, Vec2::new(NODE_WIDTH, HEADER_HEIGHT) * zoom);
                painter.rect_filled(rect, CARD_RADIUS * zoom, colors.header);
                let font = FontId::proportional(14.5 * zoom);
                let (label, _) = elide(
                    &painter,
                    &drag.item.label,
                    &font,
                    rect.width() - 2.0 * CARD_PADDING * zoom,
                );
                let _ = painter.text(
                    at + Vec2::new(CARD_PADDING, HEADER_HEIGHT / 2.0) * zoom,
                    Align2::LEFT_CENTER,
                    label,
                    font,
                    colors.text,
                );
            }
            if self.hover_wire.is_some() {
                let _ = painter.text(
                    at - Vec2::new(0.0, 8.0),
                    Align2::LEFT_BOTTOM,
                    t("insert-into-wire"),
                    FontId::proportional(12.0),
                    colors.text,
                );
            }
        }
        done
    }
}

/// The values a whole-number type holds.
fn whole_range(ty: &str) -> (i128, i128) {
    match ty {
        "core/u64@1" => (0, i128::from(u64::MAX)),
        "core/i32@1" => (i128::from(i32::MIN), i128::from(i32::MAX)),
        "core/u32@1" => (0, i128::from(u32::MAX)),
        _ => (i128::from(i64::MIN), i128::from(i64::MAX)),
    }
}

/// A number typed on a card as the input keeps it: a whole number that `ty` holds, or
/// a finite decimal.
fn number_text(text: &str, whole: bool, ty: &str) -> Option<String> {
    if whole {
        return parse_whole(text, ty).map(|value| value.to_string());
    }
    let value: f64 = text.trim().parse().ok()?;
    value.is_finite().then(|| value.to_string())
}

/// A number field's value after a drag of `pixels` from `value`: one step for each
/// five pixels for a whole number, a hundredth for each pixel for a decimal.
fn stepped(value: Option<&str>, pixels: f32, whole: bool, ty: &str) -> String {
    if whole {
        let base = value.and_then(|text| parse_whole(text, ty)).unwrap_or(0);
        return step_whole(base, pixels, ty).to_string();
    }
    let base: f64 = value.and_then(|text| text.parse().ok()).unwrap_or(0.0);
    let moved = base + f64::from(pixels) * 0.01;
    ((moved * 100.0).round() / 100.0).to_string()
}

/// A number's field on a card: the value framed like every field, across the whole
/// field, with its unit dimmed at the right end. A click types; a sideways drag steps.
fn number_field(
    ui: &mut egui::Ui,
    slot: Rect,
    id: egui::Id,
    value: Option<&str>,
    default: Option<&str>,
    unit: &str,
) -> egui::Response {
    let colors = Palette::of(ui);
    let zoom = slot.height() / card::CONTROL;
    // An input with no value shows its default dimmed, or a dash when it has none.
    let (text, ink) = match (value, default) {
        (Some(value), _) => (value, colors.text),
        (None, Some(default)) => (default, colors.weak),
        (None, None) => ("—", colors.weak),
    };
    let touched = ui
        .interact(slot, id, Sense::click_and_drag())
        .on_hover_text(t("number-drag-hint"));
    let active = touched.hovered() || touched.dragged();
    if active {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    let painter = ui.painter();
    fields::frame(painter, slot, zoom, &colors, active);
    let suffix = (!unit.is_empty()).then(|| (unit, colors.weak, FontId::proportional(11.5 * zoom)));
    let _ = fields::field_text_in(painter, slot, zoom, text, ink, suffix);
    touched
}

/// A whole number typed as text, if it is one that `ty` holds.
pub(crate) fn parse_whole(text: &str, ty: &str) -> Option<i128> {
    let value: i128 = text.trim().parse().ok()?;
    let (least, most) = whole_range(ty);
    (least..=most).contains(&value).then_some(value)
}

/// A whole number moved by a drag of `pixels`: one step for each five, exactly, held to
/// what `ty` holds.
pub(crate) fn step_whole(base: i128, pixels: f32, ty: &str) -> i128 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a drag is a few thousand pixels"
    )]
    let steps = (pixels / 5.0).round() as i128;
    let (least, most) = whole_range(ty);
    base.saturating_add(steps).clamp(least, most)
}

/// `text` cut to fit in `width` with an ellipsis, and whether it was cut.
fn elide(painter: &egui::Painter, text: &str, font: &FontId, width: f32) -> (String, bool) {
    let fits = |text: &str| {
        painter
            .layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
            .size()
            .x
            <= width
    };
    if fits(text) {
        return (text.to_owned(), false);
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        let _ = chars.pop();
        let shorter = format!("{}\u{2026}", chars.iter().collect::<String>().trim_end());
        if fits(&shorter) {
            return (shorter, true);
        }
    }
    ("\u{2026}".to_owned(), true)
}

/// Whether a value of type `source` may go to an input of type `target`, as far as the
/// canvas can tell before asking: types it does not know yet, or a type the node leaves
/// open, fit anything. The language still has the last word when the wire is made.
pub fn fits(source: Option<&str>, target: Option<&str>) -> bool {
    match (source, target) {
        (Some(source), Some(target)) => {
            source == target || source.contains('$') || target.contains('$')
        }
        _ => true,
    }
}

/// Marks a problem in the header and reports a click.
fn draw_mark(
    ui: &egui::Ui,
    painter: &egui::Painter,
    rect: Rect,
    zoom: f32,
    colors: &Palette,
    mark: &Mark,
    id: egui::Id,
) -> bool {
    let center = mark_position(rect, zoom);
    let color = colors.problem(mark.error);
    painter.rect_stroke(
        rect,
        CARD_RADIUS * zoom,
        Stroke::new(1.5 * zoom.max(0.6), color),
        StrokeKind::Inside,
    );
    if zoom >= 0.45 {
        let _ = painter.text(
            center,
            Align2::CENTER_CENTER,
            if mark.error {
                crate::icons::ERROR
            } else {
                crate::icons::WARNING
            },
            FontId::new(MARK_SIZE * zoom, crate::icons::family()),
            color,
        );
    }
    let area = Rect::from_center_size(center, Vec2::splat((MARK_SIZE + 4.0) * zoom.max(0.75)));
    let response = ui.interact(area, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &mark.text));
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
        .on_hover_ui(|ui| {
            ui.set_max_width(360.0);
            ui.strong(if mark.error {
                t("node-needs-attention")
            } else {
                t("node-warning")
            });
            ui.label(&mark.text);
            ui.small(t("open-details-hint"));
        })
        .clicked()
}

/// How big a card's problem marker is, in graph units.
const MARK_SIZE: f32 = 15.0;

/// Where a card's problem marker sits: the right end of its header, inside the padding.
fn mark_position(rect: Rect, zoom: f32) -> Pos2 {
    rect.right_top()
        + Vec2::new(
            -(CARD_PADDING + MARK_SIZE / 2.0) * zoom,
            HEADER_HEIGHT * zoom / 2.0,
        )
}

fn node_order(
    graph: &GraphView,
    front: Option<NodeId>,
) -> impl DoubleEndedIterator<Item = &NodeView> {
    graph
        .nodes
        .iter()
        .filter(move |node| Some(node.id) != front)
        .chain(
            graph
                .nodes
                .iter()
                .filter(move |node| Some(node.id) == front),
        )
}

/// Draws one card. Returns whether its trigger button was pressed.
/// `flags` are whether the card is selected, has a trigger button and shows controls
/// for its values. Also returns where each input's control goes.
#[expect(clippy::too_many_arguments, reason = "one card's drawing state")]
fn draw_node<'a>(
    ui: &egui::Ui,
    painter: &egui::Painter,
    node: &'a NodeView,
    (name, kind): (&str, &str),
    rect: Rect,
    zoom: f32,
    colors: &Palette,
    (selected, fire, controls, marked): (bool, Option<bool>, bool, bool),
    canvas: egui::Id,
    formula: Option<&str>,
    description: Option<&str>,
    connection: Option<&str>,
) -> (bool, Vec<(card::FieldOf<'a>, Rect)>) {
    let radius = CARD_RADIUS * zoom;
    let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), HEADER_HEIGHT * zoom));
    painter.rect_filled(rect, radius, colors.card);
    // The header carries the node family's color: a tint, and a line along its foot,
    // so nodes of one package are told apart at a glance, even zoomed out.
    let family = if node.entry {
        colors.exec
    } else {
        family_color(&node.definition)
    };
    let header_color = mix(colors.card, family, 0.25);
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

    let layout = card::Card::new(node);
    // Text smaller than this is a smudge: far out, a card is only its shape and dots.
    let readable = zoom >= 0.45;
    let pad = CARD_PADDING * zoom;
    let font = FontId::proportional(12.5 * zoom);
    let mut pressed = false;
    let mut slots = Vec::new();
    if readable {
        let title = FontId::proportional(14.0 * zoom);
        let clip = header.shrink2(Vec2::new(pad, 0.0));
        let header_painter = painter.with_clip_rect(clip.intersect(painter.clip_rect()));
        let mut left = clip.left_center();
        if node.entry {
            draw_arrow(&header_painter, left, 11.0 * zoom, colors.exec, zoom);
            left.x += 19.0 * zoom;
        }
        let marker = if marked { MARK_SIZE + 6.0 } else { 0.0 };
        let (shown, cut) = elide(painter, name, &title, clip.right() - left.x - marker * zoom);
        let _ = header_painter.text(left, Align2::LEFT_CENTER, &shown, title, colors.text);
        let detail = node
            .detail
            .as_deref()
            .or_else(|| (node.name != node.kind).then_some(kind));
        if cut || detail.is_some() || description.is_some() || formula.is_some() {
            let _ = ui
                .interact(clip, canvas.with(("title", node.id)), Sense::hover())
                .on_hover_ui(|ui| {
                    ui.set_max_width(280.0);
                    ui.strong(name);
                    if let Some(detail) = detail {
                        ui.label(detail);
                    }
                    if let Some(formula) = formula {
                        ui.monospace(formula);
                    }
                    if let Some(description) = description {
                        ui.label(description);
                    }
                });
        }
        if layout.empty {
            let _ = painter.text(
                Pos2::new(rect.center().x, rect.top() + layout.rows[0].anchor() * zoom),
                Align2::CENTER_CENTER,
                t("set-up-in-properties"),
                FontId::proportional(11.5 * zoom),
                colors.weak,
            );
        }
    }
    let half = rect.width() / 2.0 - 1.5 * pad;
    for row in &layout.rows {
        let y = rect.top() + row.anchor() * zoom;
        // The output first: an input's name on the same line leaves it room.
        let mut left_room = rect.width() - 2.0 * pad;
        if let Some(port) = row.right {
            draw_dot(painter, port, Pos2::new(rect.right(), y), zoom, colors);
            let literal_output = matches!(row.left, Some(card::Left::Field { named: false, .. }));
            if readable && !literal_output {
                let (label, _) = elide(painter, &port_title(port), &font, half);
                let end = Pos2::new(rect.right() - pad, y);
                if port.exec {
                    let arrow = 11.0 * zoom;
                    draw_arrow(
                        painter,
                        end - Vec2::new(arrow, 0.0),
                        arrow,
                        colors.exec,
                        zoom,
                    );
                    let _ = painter.text(
                        end - Vec2::new(arrow + 6.0 * zoom, 0.0),
                        Align2::RIGHT_CENTER,
                        label,
                        font.clone(),
                        colors.exec,
                    );
                } else {
                    let _ =
                        painter.text(end, Align2::RIGHT_CENTER, label, font.clone(), colors.text);
                }
                left_room = half;
            }
        }
        match row.left {
            Some(card::Left::Fire) => {
                let Some(live) = fire else { continue };
                let size = Vec2::new(64.0, 22.0) * zoom;
                let button =
                    Rect::from_center_size(Pos2::new(rect.left() + pad + size.x / 2.0, y), size);
                let sense = if live { Sense::click() } else { Sense::hover() };
                let touched = ui
                    .interact(button, canvas.with(("trigger", node.id)), sense)
                    .on_hover_text(if live {
                        t("fire-now-hint")
                    } else {
                        t("fire-run-first-hint")
                    });
                let fill = match (live, touched.hovered()) {
                    (false, _) => colors.weak.gamma_multiply(0.35),
                    (true, true) => colors.selected,
                    (true, false) => colors.exec,
                };
                painter.rect_filled(button, 5.0 * zoom, fill);
                if readable {
                    let _ = painter.text(
                        button.center(),
                        Align2::CENTER_CENTER,
                        t("fire"),
                        FontId::proportional(11.5 * zoom),
                        if live { colors.card } else { colors.weak },
                    );
                }
                touched.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::Button, live, t("fire-manual-trigger"))
                });
                pressed = live && touched.clicked();
            }
            Some(card::Left::Port(port)) => {
                let at = Pos2::new(rect.left(), y);
                draw_dot(painter, port, at, zoom, colors);
                if !readable {
                    continue;
                }
                let label = port_title(port);
                if port.exec {
                    // The way in is a tinted chip inside the card.
                    let arrow = 11.0 * zoom;
                    let (label, _) = elide(painter, &label, &font, left_room - arrow - 20.0 * zoom);
                    let text = painter.layout_no_wrap(label, font.clone(), colors.text);
                    let height = 22.0 * zoom;
                    let chip = Rect::from_min_size(
                        Pos2::new(rect.left() + pad - 4.0 * zoom, y - height / 2.0),
                        Vec2::new(text.size().x + arrow + 20.0 * zoom, height),
                    );
                    painter.rect_filled(chip, 5.0 * zoom, colors.exec.gamma_multiply(0.16));
                    draw_arrow(
                        painter,
                        Pos2::new(chip.min.x + 7.0 * zoom, y),
                        arrow,
                        colors.exec,
                        zoom,
                    );
                    let place =
                        Pos2::new(chip.min.x + 13.0 * zoom + arrow, y - text.size().y / 2.0);
                    painter.galley(place, text, colors.text);
                } else {
                    let (label, _) = elide(painter, &label, &font, left_room);
                    let _ = painter.text(
                        at + Vec2::new(pad, 0.0),
                        Align2::LEFT_CENTER,
                        label,
                        font.clone(),
                        colors.text,
                    );
                }
            }
            Some(card::Left::Field { editor, named }) => {
                if let Some(port) = editor.port() {
                    draw_dot(painter, port, Pos2::new(rect.left(), y), zoom, colors);
                }
                if !readable {
                    continue;
                }
                if named {
                    let label = fields::name(node, editor, connection);
                    let (label, _) = elide(painter, &label, &font, left_room);
                    let _ = painter.text(
                        Pos2::new(rect.left() + pad, y),
                        Align2::LEFT_CENTER,
                        label,
                        FontId::proportional(12.0 * zoom),
                        colors.weak,
                    );
                }
                if let Some(control) = row.control(layout.size.x) {
                    let slot = Rect::from_min_max(
                        rect.min + control.min.to_vec2() * zoom,
                        rect.min + control.max.to_vec2() * zoom,
                    );
                    if controls {
                        slots.push((editor, slot));
                    } else {
                        fields::paint_still(painter, node, editor, slot, zoom, colors);
                    }
                }
            }
            None => {}
        }
    }
    (pressed, slots)
}

/// Draws a port's dot: a diamond for a step, a circle for a value, filled when wired.
fn draw_dot(painter: &egui::Painter, port: &PortView, at: Pos2, zoom: f32, colors: &Palette) {
    let radius = PORT_RADIUS * zoom;
    let color = if port.exec {
        colors.exec
    } else {
        type_color(port.ty.as_deref())
    };
    let fill = if port.connected { color } else { colors.card };
    let outline = if port.connected {
        Stroke::NONE
    } else {
        Stroke::new(1.5 * zoom.max(0.6), color)
    };
    if port.exec {
        let outer = radius * 1.2;
        painter.add(egui::Shape::convex_polygon(
            vec![
                at - Vec2::new(outer, 0.0),
                at - Vec2::new(0.0, outer),
                at + Vec2::new(outer, 0.0),
                at + Vec2::new(0.0, outer),
            ],
            fill,
            outline,
        ));
    } else {
        painter.circle(at, radius, fill, outline);
    }
}

/// A value as its field on a card shows it: text without its quotes, others as
/// written.
fn field_text(value: &str) -> String {
    serde_json::from_str::<String>(value).unwrap_or_else(|_| value.to_owned())
}

/// What a card calls the way into a node and the way on from it: "Enter" and "Exit".
/// Only the words shown change; the ports keep their IDs `in` and `then`.
fn port_title(port: &PortView) -> String {
    step_label(port)
        .or_else(|| port.label.clone())
        .unwrap_or_else(|| crate::port_label(&port.id))
}

fn step_label(port: &PortView) -> Option<String> {
    if let Some(label) = &port.label {
        return Some(label.clone());
    }
    match (port.exec, port.input, port.id.as_str()) {
        (true, true, "in") => Some(t("port-enter").to_owned()),
        (true, false, "then") => Some(t("port-exit").to_owned()),
        _ => None,
    }
}

/// An arrow pointing right, `width` long, from `from` (its tail, at mid height).
fn draw_arrow(painter: &egui::Painter, from: Pos2, width: f32, color: Color32, zoom: f32) {
    painter.text(
        from + Vec2::new(width / 2.0, 0.0),
        Align2::CENTER_CENTER,
        crate::icons::FLOW,
        FontId::new(16.0 * zoom, crate::icons::family()),
        color,
    );
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
fn port_type_label(definition: &str, ty: &str) -> String {
    if ty == "$T" {
        match definition {
            "org.orange.math/greater"
            | "org.orange.math/greater-or-equal"
            | "org.orange.math/less"
            | "org.orange.math/less-or-equal" => return t("type-ordered").into(),
            "org.orange.math/float-greater"
            | "org.orange.math/float-greater-or-equal"
            | "org.orange.math/float-less"
            | "org.orange.math/float-less-or-equal" => return t("type-float").into(),
            _ => {}
        }
    }
    short_type(ty)
}

fn short_type(ty: &str) -> String {
    let name = crate::type_name(ty.strip_prefix('$').unwrap_or(ty));
    let mut short: String = name.chars().take(14).collect();
    if short.len() < name.len() {
        short.push('…');
    }
    short
}

/// The color of a node's family: the namespace of its kind (`core`, `org.orange.math`),
/// so every node of one package shares it.
fn family_color(definition: &str) -> Color32 {
    match definition {
        "core/get-variable"
        | "core/set-variable"
        | "core/update-variable"
        | "core/set-shared"
        | "core/update-shared" => return Color32::from_rgb(75, 154, 120),
        _ => {}
    }
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
            default: None,
            label: None,
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
            paths: Vec::new(),
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
    fn a_connection_picker_comes_before_the_inputs_and_outputs_share_its_name_line() {
        let mut source = node(
            1,
            vec![
                port("in", true, true),
                port("out", false, true),
                port("gain", true, false),
                port("level", false, false),
            ],
        );
        source.definition = "org.example.analysis/detect".into();
        source
            .references
            .push(("/source".into(), orange_host::ReferenceKind::Connection));
        let layout = card::Card::new(&source);
        assert!(matches!(
            layout.rows[1].left,
            Some(card::Left::Field {
                editor: card::FieldOf::Connection,
                named: true
            })
        ));
        assert_eq!(
            layout.rows[1].right.map(|port| port.id.as_str()),
            Some("level")
        );
        let graph = GraphView {
            nodes: vec![source],
            wires: vec![],
        };
        let geometry = Geometry::new(&graph);
        assert_eq!(geometry.size(id(1)), node_size(&graph.nodes[0]));
    }

    #[test]
    fn a_card_is_as_tall_as_its_rows_and_ports_sit_on_its_edges() {
        let graph = graph();
        assert_eq!(
            node_size(&graph.nodes[0]).y,
            BODY_TOP + ROW_HEIGHT + FOOTER_HEIGHT
        );
        // The way in, then a field block for each of a and b.
        let third = &graph.nodes[2];
        let layout = card::Card::new(third);
        assert_eq!(layout.rows.len(), 3);
        assert!(layout.rows[1].height > ROW_HEIGHT);
        assert_eq!(
            node_size(third).y,
            layout.rows[2].top + layout.rows[2].height + FOOTER_HEIGHT
        );
        assert_eq!(
            port_offset(third, "b", true),
            Some(Vec2::new(0.0, layout.rows[2].anchor()))
        );
        assert_eq!(
            port_offset(&graph.nodes[1], "then", false),
            Some(Vec2::new(
                node_size(&graph.nodes[1]).x,
                BODY_TOP + 0.5 * ROW_HEIGHT
            ))
        );
        // A port is looked for on its own side only.
        assert_eq!(port_offset(third, "b", false), None);
    }

    #[test]
    fn the_last_selected_card_stays_in_front_for_wire_drops() {
        let mut graph = graph();
        graph.nodes[1].position = Some([0.0, 0.0]);
        graph.nodes[2].position = Some([0.0, 0.0]);
        let mut canvas = Canvas::new();
        canvas.load(&graph);
        let at = Pos2::new(0.0, BODY_TOP + ROW_HEIGHT / 2.0);
        canvas.select(Some(graph.nodes[1].id));
        canvas.select(None);
        assert_eq!(
            canvas
                .input_at(&graph, at, true, graph.nodes[0].id)
                .map(|(node, _)| node),
            Some(graph.nodes[1].id)
        );
        canvas.select(Some(graph.nodes[2].id));
        assert_eq!(
            canvas
                .input_at(&graph, at, true, graph.nodes[0].id)
                .map(|(node, _)| node),
            Some(graph.nodes[2].id)
        );
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
    fn resizing_keeps_the_same_graph_point_in_the_middle() {
        let graph = graph();
        let mut canvas = Canvas::new();
        canvas.load(&graph);
        let context = egui::Context::default();
        let mut draw = |width, height| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, height))),
                ..Default::default()
            };
            context
                .run_ui(input, |ui| {
                    let _ = canvas.show(ui, &graph, &[]);
                })
                .drop_without_applying_deltas();
            canvas.center()
        };
        let [x, y] = draw(1280.0, 800.0);
        let [smaller_x, smaller_y] = draw(740.0, 570.0);
        assert!((smaller_x - x).abs() < 0.01);
        assert!((smaller_y - y).abs() < 0.01);
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
        let layout = card::Card::new(&graph.nodes[2]);
        let row = |n: usize| {
            let row = &layout.rows[n];
            third + Vec2::new(20.0, row.top + row.height - 2.0)
        };
        // Row 0 is the step way in; rows 1 and 2 are the values a and b, each across
        // its whole block, field included.
        assert_eq!(
            canvas.input_at(&graph, row(0), true, id(1)),
            Some((id(3), "in".into()))
        );
        assert_eq!(canvas.input_at(&graph, row(0), false, id(1)), None);
        assert_eq!(
            canvas.input_at(&graph, row(2), false, id(1)),
            Some((id(3), "b".into()))
        );
        // Not on the node the wire started from, not past the card, not on empty
        // canvas.
        assert_eq!(canvas.input_at(&graph, row(0), true, id(3)), None);
        let past = third + Vec2::new(NODE_WIDTH + 20.0, BODY_TOP + 0.5 * ROW_HEIGHT);
        assert_eq!(canvas.input_at(&graph, past, true, id(1)), None);
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

    #[test]
    fn dragging_a_palette_item_previews_it_and_both_payloads_can_be_dropped() {
        let context = egui::Context::default();
        let graph = GraphView {
            nodes: vec![
                node(1, vec![port("then", false, true)]),
                node(2, vec![port("in", true, true)]),
            ],
            wires: vec![wire(1, 2)],
        };
        let mut canvas = Canvas::new();
        canvas.origin = Pos2::ZERO;
        canvas.fit = false;
        canvas.places.insert(id(1), Pos2::new(0.0, 96.0));
        canvas.places.insert(id(2), Pos2::new(460.0, 96.0));
        let pointer = Pos2::new(350.0, 96.0 + BODY_TOP + ROW_HEIGHT / 2.0);
        let mut card = node(
            3,
            vec![port("a", true, false), port("result", false, false)],
        );
        card.name = "AND placement".into();
        let mut response = CanvasResponse::default();
        let mut frame = |events| {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                events,
                ..Default::default()
            };
            let output = context.run_ui(input, |ui| response = canvas.show(ui, &graph, &[]));
            (output, response.dropped.clone(), response.drop_wire.clone())
        };
        let button = |pressed| egui::Event::PointerButton {
            pos: pointer,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(pointer)])
            .0
            .drop_without_applying_deltas();
        egui::DragAndDrop::set_payload(
            &context,
            crate::palette::PaletteDrag {
                card: Some(card),
                item: crate::PaletteItem {
                    id: "core/and".into(),
                    label: "AND placement".into(),
                    package: String::new(),
                    purpose: String::new(),
                },
            },
        );
        let (output, dropped, _) = frame(vec![button(true)]);
        assert!(dropped.is_none());
        assert!(output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "AND placement")
        }));
        output.drop_without_applying_deltas();
        let (output, dropped, spliced) = frame(vec![button(false)]);
        output.drop_without_applying_deltas();
        assert_eq!(dropped.unwrap().0, "core/and");
        assert_eq!(
            spliced,
            Some(((id(1), "then".into()), (id(2), "in".into())))
        );
        frame(vec![button(true)]).0.drop_without_applying_deltas();
        egui::DragAndDrop::set_payload(&context, "variable:sample".to_owned());
        let (output, dropped, spliced) = frame(vec![button(false)]);
        output.drop_without_applying_deltas();
        assert_eq!(dropped.unwrap().0, "variable:sample");
        assert!(spliced.is_none());
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
        let port = |ty: &str| PortView {
            id: "v".into(),
            input: true,
            exec: false,
            ty: Some(ty.into()),
            value: None,
            connected: false,
            default: None,
            label: None,
        };
        let kind = |ty: &str| fields::Kind::of_port(&port(ty), &Enums::new());
        assert_eq!(kind("core/bool@1"), fields::Kind::Switch);
        assert_eq!(kind("core/i64@1"), fields::Kind::Whole);
        assert_eq!(kind("core/f64@1"), fields::Kind::Number);
        assert_eq!(kind("core/string@1"), fields::Kind::Text);
        // A type nobody types gets no field at all.
        assert!(!card::typed(&port("core/list@1<core/i64@1>")));
    }

    #[test]
    fn cards_name_types_shortly_and_color_families_alike() {
        assert_eq!(short_type("core/string@1"), "Text");
        assert_eq!(short_type("org.orange.audio/stream@1"), "Audio stream");
        assert_eq!(short_type("$T"), "T");
        assert_eq!(port_type_label("org.orange.math/greater", "$T"), "Ordered");
        assert_eq!(
            port_type_label("org.orange.math/float-greater", "$T"),
            "Float"
        );
        // The way in and the way on read as Enter and Exit; other ports keep their names.
        assert_eq!(
            step_label(&port("in", true, true)).as_deref(),
            Some("Enter")
        );
        assert_eq!(
            step_label(&port("then", false, true)).as_deref(),
            Some("Exit")
        );
        // A definition that names its way on, such as a send that only queues, says so.
        let queued = PortView {
            label: Some("Queued".into()),
            ..port("then", false, true)
        };
        assert_eq!(step_label(&queued).as_deref(), Some("Queued"));
        let labeled_data = PortView {
            label: Some("Response status".into()),
            ..port("status", false, false)
        };
        assert_eq!(port_title(&labeled_data), "Response status");
        assert_eq!(step_label(&port("in", true, false)), None);
        assert_eq!(step_label(&port("completed", false, true)), None);
        assert_eq!(short_type("core/list@1<core/string@1>"), "list<string>");
        assert_eq!(short_type("org.example.geo/point@1"), "Geo point");
        // A package's types share a colour picked from its namespace, not a list.
        assert_eq!(
            type_color(Some("org.example.geo/point@1")),
            type_color(Some("org.example.geo/route@1"))
        );
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

#[cfg(test)]
mod fit_tests {
    use super::fits;

    #[test]
    fn a_wire_fits_the_same_type_an_open_type_or_one_not_known_yet() {
        assert!(fits(Some("core/i64@1"), Some("core/i64@1")));
        assert!(!fits(Some("core/i64@1"), Some("core/string@1")));
        assert!(fits(Some("core/i64@1"), Some("$T")));
        assert!(fits(
            Some("core/list@1<$T>"),
            Some("core/list@1<core/i64@1>")
        ));
        assert!(fits(None, Some("core/string@1")));
        assert!(fits(Some("core/string@1"), None));
    }
}

#[cfg(test)]
mod whole_tests {
    use super::{parse_whole, step_whole};

    #[test]
    fn whole_numbers_are_parsed_and_stepped_exactly_within_their_type() {
        let big = 9_007_199_254_740_993_i128;
        assert_eq!(parse_whole("9007199254740993", "core/i64@1"), Some(big));
        assert_eq!(step_whole(big, 5.0, "core/i64@1"), big + 1);
        assert_eq!(step_whole(big, -10.0, "core/i64@1"), big - 2);
        let most = i128::from(i64::MAX);
        assert_eq!(step_whole(most, 500.0, "core/i64@1"), most);
        assert_eq!(parse_whole("9223372036854775808", "core/i64@1"), None);
        assert_eq!(
            parse_whole("18446744073709551615", "core/u64@1"),
            Some(i128::from(u64::MAX))
        );
        assert_eq!(parse_whole("-1", "core/u64@1"), None);
        assert_eq!(step_whole(0, -50.0, "core/u32@1"), 0);
        assert_eq!(parse_whole("2147483648", "core/i32@1"), None);
        assert_eq!(parse_whole("1.5", "core/i64@1"), None);
    }
}

#[cfg(test)]
mod run_state_tests {
    use super::*;

    fn node(n: u32) -> NodeId {
        format!("10000000-0000-4000-8000-{n:012x}").parse().unwrap()
    }

    #[test]
    fn a_wired_input_shows_its_sources_value_and_failures_last_until_success() {
        let (a, b) = (node(1), node(2));
        let graph = GraphView {
            nodes: Vec::new(),
            wires: vec![orange_host::WireView {
                from: (a, "result".into()),
                to: (b, "value".into()),
                exec: false,
            }],
        };
        let mut canvas = Canvas::new();
        canvas.show_value(a, "result".into(), "42".into(), "42".into());
        assert_eq!(
            canvas.port_value(&graph, b, "value", true).unwrap().short,
            "42"
        );
        assert_eq!(
            canvas.port_value(&graph, a, "result", false).unwrap().full,
            "42"
        );
        assert!(canvas.port_value(&graph, b, "other", true).is_none());

        canvas.fail(b, "the request failed".into());
        assert_eq!(canvas.failure(b), Some("the request failed"));
        // The run ends: values stay, dimmed; the failure stays too.
        canvas.mark_stale();
        assert!(canvas.is_stale() && canvas.failure(b).is_some());
        canvas.show_value(b, "result".into(), "9".into(), "9".into());
        assert_eq!(
            canvas
                .port_sample(&graph, b, "value", true)
                .map(|(_, stale)| stale),
            Some(true)
        );
        assert_eq!(
            canvas
                .port_sample(&graph, b, "result", false)
                .map(|(_, stale)| stale),
            Some(false)
        );
        canvas.recover(b);
        assert!(canvas.failure(b).is_none());
        // A new run starts from nothing.
        canvas.fail(b, "again".into());
        canvas.clear_run();
        assert!(!canvas.is_stale() && canvas.failure(b).is_none());
        assert!(canvas.port_value(&graph, b, "value", true).is_none());
    }
}

#[cfg(test)]
mod elide_tests {
    use super::*;

    #[test]
    fn a_title_too_long_for_its_room_ends_in_an_ellipsis() {
        let context = egui::Context::default();
        let mut result = None;
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            let font = FontId::proportional(14.0);
            let short = elide(ui.painter(), "Int", &font, 200.0);
            let long = elide(ui.painter(), "When the microphone hears sound", &font, 90.0);
            result = Some((short, long));
        });
        output.drop_without_applying_deltas();
        let (short, long) = result.unwrap();
        assert_eq!(short, ("Int".to_owned(), false));
        assert!(
            long.1 && long.0.ends_with('\u{2026}') && long.0.len() < 31,
            "{long:?}"
        );
    }
}

/// A point moved to the nearest grid crossing.
pub(crate) fn snapped(point: Pos2) -> Pos2 {
    Pos2::new(
        (point.x / GRID).round() * GRID,
        (point.y / GRID).round() * GRID,
    )
}

#[cfg(test)]
mod snap_tests {
    use super::*;

    #[test]
    fn a_point_snaps_to_the_nearest_grid_crossing() {
        assert_eq!(snapped(Pos2::new(13.0, 35.0)), Pos2::new(24.0, 24.0));
        assert_eq!(snapped(Pos2::new(-13.0, 11.0)), Pos2::new(-24.0, 0.0));
        assert!(Canvas::new().snap && !Canvas::new().scroll_pans);
    }
}

/// How far `at` is from the curve through `points`, sampled finely enough for a pointer.
pub(crate) fn bezier_distance(points: &[Pos2; 4], at: Pos2) -> f32 {
    const STEPS: usize = 32;
    let point = |t: f32| {
        let u = 1.0 - t;
        let v = points[0].to_vec2() * (u * u * u)
            + points[1].to_vec2() * (3.0 * u * u * t)
            + points[2].to_vec2() * (3.0 * u * t * t)
            + points[3].to_vec2() * (t * t * t);
        v.to_pos2()
    };
    let mut best = f32::INFINITY;
    let mut last = point(0.0);
    for step in 1..=STEPS {
        #[expect(clippy::cast_precision_loss, reason = "a few steps")]
        let next = point(step as f32 / STEPS as f32);
        // Distance to the segment between two samples.
        let along = next - last;
        let t = ((at - last).dot(along) / along.length_sq().max(f32::EPSILON)).clamp(0.0, 1.0);
        best = best.min((last + along * t).distance(at));
        last = next;
    }
    best
}

#[cfg(test)]
mod wire_hit_tests {
    use super::*;

    #[test]
    fn a_point_on_or_beside_a_wire_is_near_it() {
        let points = [
            Pos2::new(0.0, 0.0),
            Pos2::new(50.0, 0.0),
            Pos2::new(50.0, 100.0),
            Pos2::new(100.0, 100.0),
        ];
        assert!(bezier_distance(&points, Pos2::new(0.0, 0.0)) < 0.5);
        assert!(bezier_distance(&points, Pos2::new(50.0, 50.0)) < 2.0);
        assert!(bezier_distance(&points, Pos2::new(0.0, 100.0)) > 30.0);
    }
}

/// A formula with each input's value, when `value_of` knows it, in place of its name;
/// a short value is shown whole, a long one as its start.
pub(crate) fn fill_formula(formula: &str, value_of: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut word = String::new();
    let flush = |word: &mut String, out: &mut String| {
        if word.is_empty() {
            return;
        }
        match value_of(word) {
            Some(value) => {
                let mut shown: String = value.chars().take(10).collect();
                if shown.len() < value.len() {
                    shown.push('\u{2026}');
                }
                out.push_str(&shown);
            }
            None => out.push_str(word),
        }
        word.clear();
    };
    for c in formula.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            flush(&mut word, &mut out);
            out.push(c);
        }
    }
    flush(&mut word, &mut out);
    out
}

#[cfg(test)]
mod formula_tests {
    use super::fill_formula;

    #[test]
    fn known_inputs_take_the_place_of_their_names() {
        let both = |port: &str| match port {
            "a" => Some("12".to_owned()),
            "b" => Some("7".to_owned()),
            _ => None,
        };
        assert_eq!(fill_formula("a > b", both), "12 > 7");
        assert_eq!(
            fill_formula("a > b", |port| (port == "a").then(|| "3".to_owned())),
            "3 > b"
        );
        assert_eq!(fill_formula("min(a, b)", |_| None), "min(a, b)");
    }
}
