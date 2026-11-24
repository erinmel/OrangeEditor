//! The graph canvas: nodes as cards with their ports, wires between them, panning,
//! zooming, selecting and moving nodes, and controls on the cards for the values of
//! inputs.
//!
//! It is a widget of its own. An application that has its own lists and panels shows
//! only this: it asks its host for a [`GraphView`], calls [`Canvas::show`] each frame
//! and acts on the [`CanvasResponse`]. Nothing here knows about files, flows or the
//! other panels of the editor.
use crate::diagnostics::ProblemLevel;
use crate::i18n::{t, tf};
use crate::organise::NodeRename;
use egui::epaint::CubicBezierShape;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, UiBuilder, Vec2};
use orange_host::{GraphView, NodeId, NodeView, PortView};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

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

/// Time for the full execution wire to fade after a step.
const PULSE_SECONDS: f32 = 0.55;

/// Space under the last row.
pub const FOOTER_HEIGHT: f32 = card::END;
/// Distance between the columns and rows of an automatic layout.
pub const LAYOUT_GAP: Vec2 = Vec2::new(90.0, 36.0);
/// The zoom range.
pub const ZOOM_RANGE: std::ops::RangeInclusive<f32> = 0.2..=1.75;
pub(crate) const GRID_STEPS: [u16; 4] = [12, 24, 36, 48];
pub(crate) const DEFAULT_GRID_STEP: u16 = 24;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GridStyle {
    #[default]
    Dots,
    Lines,
    Hidden,
}

impl GridStyle {
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::Dots => "dots",
            Self::Lines => "lines",
            Self::Hidden => "hidden",
        }
    }

    pub(crate) fn from_key(key: Option<&str>) -> Self {
        match key {
            Some("lines") => Self::Lines,
            Some("hidden") => Self::Hidden,
            _ => Self::Dots,
        }
    }
}
/// How near a wire the pointer has to be to take it, in points at zoom 1.
const WIRE_GRAB: f32 = 7.0;
/// How much one point of wheel movement zooms.
const WHEEL_ZOOM: f32 = 0.0015;
const PORT_RADIUS: f32 = 5.0;
/// Below this zoom, cards show their values without controls.
const CONTROLS_ZOOM: f32 = 0.60;
/// Space between a card's edge and what is inside it.
const CARD_PADDING: f32 = 10.0;
#[expect(
    clippy::cast_possible_truncation,
    reason = "animation phase is in 0..1"
)]
fn animation_phase(time: f64) -> f32 {
    (time / 3.6).rem_euclid(1.0) as f32
}

/// The choices of the enum types a document's inputs use, by type.
pub(crate) type Enums = HashMap<String, Vec<orange_host::EnumChoice>>;

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
    if let orange_host::Value::Duration(nanos) = value
        && let Some(ms) = fields::duration_millis(&nanos.to_string())
    {
        return format!("{ms} ms");
    }
    if matches!(value, orange_host::Value::Json(_)) {
        return orange_host::inspect(value);
    }
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
fn titles(graph: &GraphView) -> HashMap<NodeId, String> {
    graph
        .nodes
        .iter()
        .filter(|node| node.definition == "core/convert")
        .filter_map(|node| {
            if let Some(label) = node.settings["converter"]
                .as_str()
                .and_then(crate::palette::utf8_label)
            {
                return Some((node.id, label.to_owned()));
            }
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
pub(crate) use card::stream_type;
pub(crate) use settings::advanced as advanced_setting;
#[cfg(test)]
mod field_tests;
mod fields;
mod layout;
pub use layout::auto_layout;
mod geometry;
mod groups;
mod settings;
pub(crate) mod spectrum;
use geometry::{Geometry, near_wire, wire_points, wire_visible};
pub use groups::{GROUP_PAD, GROUP_TITLE, GroupEdit};
pub(crate) use spectrum::BandEdit;

type StoredText<'a> = dyn Fn(NodeId, &str) -> Option<String> + 'a;

#[derive(Clone, Debug)]
struct ExpandedText {
    node: NodeId,
    port: String,
    value: String,
    focused: bool,
}

/// What the person did on the canvas this frame.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CanvasResponse {
    pub(crate) add_signal_receiver: Option<NodeId>,
    pub(crate) waypoint: Option<(WireEnds, [f32; 2])>,
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
    /// A node title was double-clicked.
    pub rename_node: Option<NodeId>,
    /// The name applied by editing a title directly on its card.
    pub set_name: Option<(NodeId, String)>,
    /// A value was written on an input with its control on the card, typed as a
    /// person types it.
    pub set_input: Option<(NodeId, String, String)>,
    /// A file or folder chooser was requested from an input on a node card.
    pub browse_path: Option<(NodeId, String)>,
    /// A device chooser was requested at a node card.
    pub configure: Option<(NodeId, Pos2)>,
    pub(crate) inspect_value: Option<(NodeId, String, bool)>,
    /// A schema-backed configuration field edited directly on the card.
    pub set_config: Option<(NodeId, String, serde_json::Value)>,
    /// "New variable…" was chosen in a card's variable picker: the node and the
    /// setting the new variable goes into.
    pub new_variable: Option<(NodeId, String)>,
    /// A device selected from an adapter-provided choice list.
    pub device: Option<(NodeId, Option<String>)>,
    /// One more instance of a variadic port was asked for on a card.
    pub add_port: Option<(NodeId, String)>,
    /// An added port was removed from its card.
    pub remove_port: Option<(NodeId, String)>,
    /// Refresh the adapter choices for a node's device picker.
    pub(crate) refresh_devices: Option<String>,
    pub(crate) spectrum_edit: Option<(NodeId, BandEdit)>,
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
    /// A wire dragged back from an input was let go on the background: the input, and
    /// where (graph units and screen).
    pub loose_into: Option<((NodeId, String), [f32; 2], Pos2)>,
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
    raw_text: Option<orange_host::Value>,
    epoch: u64,
    preview: bool,
}

fn text_value(value: &orange_host::Value) -> Option<&str> {
    match value {
        orange_host::Value::Str(text) => Some(text.as_str()),
        orange_host::Value::Json(text) => Some(text.as_str()),
        _ => None,
    }
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
pub use orange_host::WireEnds;

/// The problems the canvas shows on nodes and wires.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Marks {
    pub nodes: HashMap<NodeId, Mark>,
    /// By the wire's two ends, output first.
    pub wires: HashMap<WireEnds, Mark>,
    /// By node, port name and known direction.
    pub ports: HashMap<(NodeId, String, Option<orange_host::PortDirection>), Mark>,
}

impl Marks {
    /// Adds a problem to what is already marked on a node or wire: the texts are
    /// kept one per line, and an error outweighs a warning.
    pub fn add<K: std::hash::Hash + Eq>(
        marks: &mut HashMap<K, Mark>,
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
    /// The port the drag started at: an output, or an input when `backward`.
    from: (NodeId, String),
    exec: bool,
    /// Dragged back from an input, looking for an output to feed it.
    backward: bool,
}

#[derive(Debug, Clone, Copy)]
enum PulseRoute {
    Default(&'static str),
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
#[derive(Debug, Clone, Copy, PartialEq)]
struct GridCacheKey {
    area: Rect,
    origin: Pos2,
    zoom: f32,
    scale: f32,
    step: u16,
    style: GridStyle,
    background: Color32,
    ink: Color32,
}

#[derive(Debug, Clone)]
pub struct Canvas {
    grid_cache: Option<(GridCacheKey, std::sync::Arc<egui::Mesh>)>,
    failure_routes: HashMap<NodeId, orange_host::FailureRoutes>,
    expanded_failure_routes: HashSet<NodeId>,
    focused_failure_port: Option<(NodeId, String)>,
    pub(crate) waypoints: HashMap<WireEnds, [f32; 2]>,
    pub(crate) audio_running: bool,
    pub(crate) spectra: BTreeMap<NodeId, Vec<f64>>,
    pub(crate) collapsed_spectra: BTreeSet<NodeId>,
    pub(crate) noise_readings: BTreeMap<NodeId, f64>,
    pub(crate) noise_channels: BTreeMap<NodeId, spectrum::ChannelLevels>,
    pub(crate) keys: crate::keymap::Keymap,
    pub(crate) shortcuts_enabled: bool,
    pub(crate) device_pickers: BTreeMap<NodeId, crate::card_sources::DevicePicker>,
    /// The document's variables, as (ID, name), for the pickers on cards.
    pub(crate) variables: Vec<(String, String)>,
    /// The choices of the enum types the document's inputs take.
    pub(crate) enums: Enums,
    /// The wheel scrolls instead of zooming (Ctrl+wheel then zooms), for trackpads.
    pub scroll_pans: bool,
    /// Nodes and groups land on the grid. Alt while dragging moves freely.
    pub snap: bool,
    pub(crate) grid_step: u16,
    pub(crate) grid_style: GridStyle,
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
    expanded_text: Option<ExpandedText>,
    text_heights: BTreeMap<NodeId, BTreeMap<String, f32>>,
    /// A whole number being dragged: its node, port, value before and pixels moved.
    /// A node was dragged and not yet let go.
    moving: bool,
    /// Show the whole graph at the next frame.
    fit: bool,
    /// The zoom and origin the last fit chose. While the view is still exactly that,
    /// a resize (the window settling, a dock opening) fits again instead of keeping
    /// the center, so an untouched graph stays whole.
    fitted: Option<(f32, Pos2)>,
    wiring: Option<Wiring>,
    /// The wire under the pointer at the last frame, and the selected wire.
    hover_wire: Option<WireEnds>,
    selected_wire: Option<WireEnds>,
    /// Problems drawn on nodes and wires.
    marks: Marks,
    draft_marks: BTreeSet<NodeId>,
    help_revision: Option<(u64, String)>,
    descriptions: HashMap<String, String>,
    labels: HashMap<String, String>,
    /// Titles of nodes named by what they hold rather than their kind: a conversion is
    /// "Int to Text".
    titles: HashMap<NodeId, String>,
    connection_labels: HashMap<String, String>,
    pub(super) connection_adapters: HashMap<String, String>,
    pub(super) connection_profiles: HashMap<String, Vec<(String, String)>>,
    /// When each node of a running flow last took a step, and whether it failed.
    activity: HashMap<NodeId, StepActivity>,
    entry_activity: BTreeMap<NodeId, BTreeMap<String, std::time::Instant>>,
    /// The latest value a running flow put on each output, as a person reads it.
    values: HashMap<NodeId, HashMap<String, PortSample>>,
    value_epoch: u64,
    /// Nodes whose last step failed, with why; kept until the node succeeds or a new
    /// run starts.
    failures: HashMap<NodeId, String>,
    /// The values and failures are from a run that has ended or a graph since changed.
    stale: bool,
    /// The size of the canvas when it was last shown, in points.
    size: Vec2,
    /// The document's groups, the nodes hidden in collapsed ones and a group being
    /// renamed with what is typed.
    groups: Vec<orange_host::Group>,
    selected_group_frame: Option<String>,
    hidden: HashSet<NodeId>,
    renaming: Option<(String, String, bool)>,
    node_rename: Option<NodeRename>,
}
impl Default for Canvas {
    fn default() -> Self {
        Self {
            failure_routes: HashMap::new(),
            expanded_failure_routes: HashSet::new(),
            focused_failure_port: None,
            audio_running: false,
            spectra: BTreeMap::new(),
            collapsed_spectra: BTreeSet::new(),
            noise_readings: BTreeMap::new(),
            noise_channels: BTreeMap::new(),
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
            expanded_text: None,
            text_heights: BTreeMap::new(),
            moving: false,
            fit: true,
            fitted: None,
            wiring: None,
            hover_wire: None,
            selected_wire: None,
            marks: Marks::default(),
            draft_marks: BTreeSet::new(),
            activity: HashMap::new(),
            entry_activity: BTreeMap::new(),
            help_revision: None,
            descriptions: HashMap::new(),
            labels: HashMap::new(),
            titles: HashMap::new(),
            connection_labels: HashMap::new(),
            connection_adapters: HashMap::new(),
            connection_profiles: HashMap::new(),
            values: HashMap::new(),
            failures: HashMap::new(),
            stale: false,
            value_epoch: 0,
            size: Vec2::new(800.0, 600.0),
            scroll_pans: false,
            snap: true,
            grid_step: DEFAULT_GRID_STEP,
            grid_cache: None,
            grid_style: GridStyle::default(),
            groups: Vec::new(),
            selected_group_frame: None,
            hidden: HashSet::new(),
            renaming: None,
            waypoints: HashMap::new(),
            node_rename: None,
        }
    }
}

/// Colors of the canvas, taken from the surrounding egui style so it follows a light
/// or dark application.
struct Palette {
    error: Color32,
    warning: Color32,
    background: Color32,
    card: Color32,
    header: Color32,
    outline: Color32,
    selected: Color32,
    text: Color32,
    weak: Color32,
    exec: Color32,
}
impl Palette {
    fn port_color(&self, ty: Option<&str>, exec: bool) -> Color32 {
        let color = if exec { self.exec } else { type_color(ty) };
        let card = luminance(self.card);
        let background = luminance(self.background);
        let endpoint = if card > 0.18 {
            Color32::BLACK
        } else {
            Color32::WHITE
        };
        for amount in [0.0, 0.2, 0.4, 0.6, 0.8, 1.0] {
            let ink = color.lerp_to_gamma(endpoint, amount);
            let light = luminance(ink);
            let contrast = |surface: f32| (surface.max(light) + 0.05) / (surface.min(light) + 0.05);
            if contrast(card) >= 3.0 && contrast(background) >= 3.0 {
                return ink;
            }
        }
        endpoint
    }

    fn problem_level(&self, level: ProblemLevel) -> Color32 {
        match level {
            ProblemLevel::Blocker => self.error,
            ProblemLevel::Warning => self.warning,
            ProblemLevel::Draft => self.weak,
        }
    }

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
    fn mark_level(&self, node: NodeId, mark: &Mark) -> ProblemLevel {
        if mark.error {
            ProblemLevel::Blocker
        } else if self.draft_marks.contains(&node) {
            ProblemLevel::Draft
        } else {
            ProblemLevel::Warning
        }
    }

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

    fn profile_choices(&self, node: &NodeView) -> &[(String, String)] {
        self.connection_adapters
            .get(&node.definition)
            .and_then(|adapter| self.connection_profiles.get(adapter))
            .map_or(&[], Vec::as_slice)
    }

    fn connection_selection(&self, node: &NodeView) -> Option<&str> {
        if let Some(picker) = self.device_pickers.get(&node.id) {
            return Some(picker.label());
        }
        let pointer = node
            .references
            .iter()
            .find(|(_, kind)| *kind == orange_host::ReferenceKind::Connection)?
            .0
            .as_str();
        let id = node.settings.pointer(pointer)?.as_str()?;
        self.profile_choices(node)
            .iter()
            .find(|(candidate, _)| candidate == id)
            .map(|(_, name)| name.as_str())
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
        self.expanded_failure_routes.clear();
        self.focused_failure_port = None;
        self.expanded_text = None;
        self.text_heights.clear();
        self.selected_group_frame = None;
        self.collapsed_spectra.clear();
        self.titles = titles(graph);
        self.places = self.auto_layout(graph);
        self.forget_missing();
        self.fit = true;
    }

    /// Follows an edit of the graph: nodes that are new take the place the document
    /// gives them or an automatic one, nodes that are gone are forgotten, and every
    /// other node stays where it is. The view is not moved.
    pub fn update(&mut self, graph: &GraphView) {
        if let Some(edit) = &self.expanded_text
            && !graph.nodes.iter().any(|node| {
                node.id == edit.node
                    && node
                        .ports
                        .iter()
                        .any(|port| port.input && !port.connected && port.id == edit.port)
            })
        {
            self.expanded_text = None;
        }
        self.titles = titles(graph);
        // Most edits only change values or wires. Layout is needed only for a new
        // node without a saved position; existing nodes keep their dragged places.
        let needs_layout = graph
            .nodes
            .iter()
            .any(|node| !self.places.contains_key(&node.id) && node.position.is_none());
        let automatic = needs_layout.then(|| self.auto_layout(graph));
        let retained: HashSet<_> = graph.nodes.iter().map(|node| node.id).collect();
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
        self.collapsed_spectra.retain(|id| places.contains_key(id));
        self.text_heights.retain(|id, _| places.contains_key(id));
        self.selection.retain(|id| places.contains_key(id));
        if !self.selected.is_some_and(|id| self.selection.contains(&id)) {
            self.selected = None;
        }
    }

    pub(crate) fn set_failure_routes(
        &mut self,
        routes: HashMap<NodeId, orange_host::FailureRoutes>,
    ) {
        self.expanded_failure_routes
            .retain(|node| routes.contains_key(node));
        if self
            .focused_failure_port
            .as_ref()
            .is_some_and(|(node, port)| {
                !routes.get(node).is_some_and(|routes| routes.contains(port))
            })
        {
            self.focused_failure_port = None;
        }
        self.failure_routes = routes;
    }

    pub(crate) fn has_failure_routes(&self, node: NodeId) -> bool {
        self.failure_routes.contains_key(&node)
    }

    pub(crate) fn failure_routes_expanded(&self, node: NodeId) -> bool {
        self.expanded_failure_routes.contains(&node)
    }

    pub(crate) fn toggle_failure_routes(&mut self, node: NodeId) {
        if self.has_failure_routes(node) && !self.expanded_failure_routes.remove(&node) {
            self.expanded_failure_routes.insert(node);
        }
    }

    fn auto_layout(&self, graph: &GraphView) -> BTreeMap<NodeId, Pos2> {
        layout::auto_layout_with(graph, |node| self.card(node).size)
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
        self.selected_group_frame = None;
        self.selection = nodes
            .iter()
            .copied()
            .filter(|id| self.places.contains_key(id) && !self.hidden.contains(id))
            .collect();
        self.selected = match self.selection.len() {
            1 => self.selection.first().copied(),
            _ => None,
        };
        if self
            .focused_failure_port
            .as_ref()
            .is_some_and(|(node, _)| Some(*node) != self.selected)
        {
            self.focused_failure_port = None;
        }
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
                    Rect::from_min_size(*place, self.card(node).size).intersects(area)
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
        self.port_at(graph, at, exec, from, true)
    }

    /// The port of the same sort a wire let go at `at` lands on: an input's row on the
    /// left of its card, or with `input` false an output's row on the right. A wire does
    /// not land on the node it started from.
    fn port_at(
        &self,
        graph: &GraphView,
        at: Pos2,
        exec: bool,
        from: NodeId,
        input: bool,
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
                .filter(|port| port.input == input && port.exec == exec)
            {
                let layout = self.card(node);
                let found = if input {
                    layout.row_of(port)
                } else {
                    layout.row_of_output(port)
                };
                let Some(found) = found else {
                    continue;
                };
                // Across the port's whole row, a little past its dot.
                let (left, right) = if input {
                    (-14.0, layout.size.x)
                } else {
                    (0.0, layout.size.x + 14.0)
                };
                let row = Rect::from_min_max(
                    place + Vec2::new(left, found.top),
                    place + Vec2::new(right, found.top + found.height),
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

    fn draw_grid(&mut self, painter: &egui::Painter, area: Rect, colors: &Palette) {
        if self.grid_style == GridStyle::Hidden {
            return;
        }
        let key = GridCacheKey {
            area,
            origin: self.origin,
            zoom: self.zoom,
            scale: painter.ctx().pixels_per_point(),
            step: self.grid_step,
            style: self.grid_style,
            background: colors.background,
            ink: colors.text,
        };
        if let Some((previous, mesh)) = &self.grid_cache
            && *previous == key
        {
            painter.add(egui::Shape::Mesh(mesh.clone()));
            return;
        }
        let mesh = std::sync::Arc::new(self.grid_mesh(area, key.scale, colors));
        painter.add(egui::Shape::Mesh(mesh.clone()));
        self.grid_cache = Some((key, mesh));
    }
    fn grid_mesh(&self, area: Rect, pixels_per_point: f32, colors: &Palette) -> egui::Mesh {
        const MAJOR: u32 = 4;
        let base_step = f32::from(self.grid_step) * self.zoom;
        let major_step = base_step * 4.0;
        let (step, stride) = if base_step < 12.0 {
            (major_step, MAJOR)
        } else {
            (base_step, 1)
        };
        // The first position in either direction is a major intersection in graph
        // coordinates. Panning and dropping minor marks keep the same alignment.
        let first = area.min
            - Vec2::new(
                (self.origin.x * self.zoom).rem_euclid(major_step),
                (self.origin.y * self.zoom).rem_euclid(major_step),
            );
        let positions = |start, end| {
            std::iter::successors(Some((start, 0_u32)), move |&(at, index)| {
                Some((at + step, index + stride))
            })
            .take_while(move |&(at, _)| at < end)
            .map(|(at, index)| (at, index.is_multiple_of(MAJOR)))
        };
        let dots = self.grid_style == GridStyle::Dots;
        let fade = ((base_step - 8.0) / 16.0).clamp(0.0, 1.0);
        let minor = mix(
            colors.background,
            colors.text,
            if dots { 0.32 } else { 0.22 } * fade,
        );
        let major = mix(
            colors.background,
            colors.text,
            if dots { 0.48 } else { 0.36 },
        );
        let color = |strong| if strong { major } else { minor };
        let pixel_edge = |at: f32| (at * pixels_per_point).floor() / pixels_per_point;
        let pixel = 1.0 / pixels_per_point;
        let mut mesh = egui::Mesh::default();
        let rows = positions(first.y, area.bottom());
        let columns = positions(first.x, area.right());
        if dots {
            for (y, strong_row) in rows {
                for (x, strong_column) in columns.clone() {
                    let strong = strong_row && strong_column;
                    let radius = if strong { 1.75 } else { 1.2 };
                    grid_dot(&mut mesh, Pos2::new(x, y), radius, color(strong));
                }
            }
        } else {
            for (y, strong) in rows {
                let line = Rect::from_min_size(
                    Pos2::new(area.left(), pixel_edge(y)),
                    Vec2::new(area.width(), pixel),
                );
                mesh.add_colored_rect(line, color(strong));
            }
            for (x, strong) in columns {
                let line = Rect::from_min_size(
                    Pos2::new(pixel_edge(x), area.top()),
                    Vec2::new(pixel, area.height()),
                );
                mesh.add_colored_rect(line, color(strong));
            }
        }
        mesh
    }

    /// Shows the whole graph at the next frame.
    pub fn fit(&mut self) {
        self.fit = true;
    }

    /// Returns to actual size, keeping the top left corner where it is.
    pub fn reset_zoom(&mut self) {
        self.zoom = 1.0;
    }

    pub(crate) fn zoom_around_center(&mut self, zoom: f32) {
        self.zoom_at(zoom, self.size / 2.0);
    }

    fn zoom_at(&mut self, zoom: f32, offset: Vec2) {
        let before = self.origin + offset / self.zoom;
        self.zoom = zoom.clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
        self.origin = before - offset / self.zoom;
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

    fn card<'a>(&self, node: &'a NodeView) -> card::Card<'a> {
        let failure = self
            .failure_routes
            .get(&node.id)
            .map(|&ports| card::FailureRows {
                ports,
                expanded: self.failure_routes_expanded(node.id),
                pending: self
                    .wiring
                    .as_ref()
                    .filter(|wire| wire.from.0 == node.id)
                    .map(|wire| wire.from.1.as_str()),
                diagnostic: self
                    .focused_failure_port
                    .as_ref()
                    .filter(|(id, _)| *id == node.id)
                    .map(|(_, port)| port.as_str()),
            });
        card::Card::with_controls(
            node,
            !self.collapsed_spectra.contains(&node.id),
            failure,
            |port| self.text_heights.get(&node.id)?.get(&port.id).copied(),
        )
    }

    fn bounds(&self, graph: &GraphView) -> Option<Rect> {
        graph
            .nodes
            .iter()
            .filter(|node| !self.is_hidden(node.id))
            .filter_map(|node| {
                Some(Rect::from_min_size(
                    *self.places.get(&node.id)?,
                    self.card(node).size,
                ))
            })
            .chain(
                self.groups
                    .iter()
                    .filter_map(|group| self.group_frame(graph, group)),
            )
            .reduce(|all, rect| all.union(rect))
    }

    pub(crate) fn selected_wire(&self) -> Option<&WireEnds> {
        self.selected_wire.as_ref()
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
                Then => Some(PulseRoute::Default("then")),
                Completed => Some(PulseRoute::Default("completed")),
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

    /// How brightly the way out `port` of `node` glows: 1 just as a step leaves through
    /// it, fading to nothing, wired or not.
    fn exit_glow(&self, geometry: &Geometry<'_>, node: NodeId, port: &str) -> f32 {
        let fading = |time: std::time::Instant| {
            (1.0 - time.elapsed().as_secs_f32() / PULSE_SECONDS).max(0.0)
        };
        let entry = self
            .entry_activity
            .get(&node)
            .and_then(|ports| ports.get(port))
            .map_or(0.0, |time| fading(*time));
        let step = self
            .activity
            .get(&node)
            .filter(|step| match step.route {
                Some(PulseRoute::Default(preferred)) => {
                    let exit = geometry
                        .port(node, preferred, false)
                        .filter(|port| port.exec)
                        .map(|_| preferred)
                        .or_else(|| geometry.sole_exec_output(node));
                    exit == Some(port)
                }
                Some(PulseRoute::Port(route)) => route == port,
                None => false,
            })
            .map_or(0.0, |step| fading(step.time));
        entry.max(step)
    }

    pub(crate) fn record_entry_route(&mut self, node: NodeId, port: String) {
        self.entry_activity
            .entry(node)
            .or_default()
            .insert(port, std::time::Instant::now());
    }

    /// A running flow put a value on an output: `short` shows beside the port, `full`
    /// in the inspector. It stays until another comes; when the run ends it is dimmed.
    pub fn show_value(&mut self, node: NodeId, port: String, short: String, full: String) {
        self.store_sample(node, port, Shown { short, full }, None, false);
    }

    pub(crate) fn show_observed_value(
        &mut self,
        node: NodeId,
        port: String,
        shown: Shown,
        value: &orange_host::Value,
        preview: bool,
    ) {
        let raw_text = text_value(value)
            .filter(|text| *text != shown.full)
            .map(|_| value.clone());
        self.store_sample(node, port, shown, raw_text, preview);
    }

    fn store_sample(
        &mut self,
        node: NodeId,
        port: String,
        shown: Shown,
        raw_text: Option<orange_host::Value>,
        preview: bool,
    ) {
        self.stale = false;
        let _ = self.values.entry(node).or_default().insert(
            port,
            PortSample {
                shown,
                raw_text,
                epoch: self.value_epoch,
                preview,
            },
        );
    }

    pub(crate) fn clear_value(&mut self, node: NodeId, port: &str) {
        if let Some(values) = self.values.get_mut(&node) {
            values.remove(port);
        }
        if port == "spectrum" {
            self.spectra.remove(&node);
        }
        if port == "level" {
            self.noise_readings.remove(&node);
        }
        if port == "channel_levels" {
            self.noise_channels.remove(&node);
        }
    }

    /// Forgets the previews of `nodes` only, keeping every other one.
    pub(crate) fn clear_previews_of(&mut self, nodes: &BTreeSet<NodeId>) {
        for node in nodes {
            if let Some(ports) = self.values.get_mut(node) {
                ports.retain(|_, sample| !sample.preview);
            }
        }
        self.values.retain(|_, ports| !ports.is_empty());
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
        self.spectra.clear();
        self.noise_readings.clear();
        self.noise_channels.clear();
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
        let sample = self.sample_at(graph, node, port, input)?;
        Some((
            &sample.shown,
            !sample.preview && sample.epoch != self.value_epoch,
        ))
    }

    pub(crate) fn copyable_port_text(
        &self,
        graph: &GraphView,
        node: NodeId,
        port: &str,
        input: bool,
    ) -> Option<&str> {
        let sample = self.sample_at(graph, node, port, input)?;
        Some(
            sample
                .raw_text
                .as_ref()
                .and_then(text_value)
                .unwrap_or(&sample.shown.full),
        )
    }

    fn sample_at(
        &self,
        graph: &GraphView,
        node: NodeId,
        port: &str,
        input: bool,
    ) -> Option<&PortSample> {
        let (node, port) = if input {
            let wire = graph
                .wires
                .iter()
                .find(|wire| wire.to.0 == node && wire.to.1 == port)?;
            (wire.from.0, wire.from.1.as_str())
        } else {
            (node, port)
        };
        self.output_sample(node, port)
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
        self.audio_running = false;
        self.spectra.clear();
        self.noise_readings.clear();
        self.noise_channels.clear();
        self.activity.clear();
        self.entry_activity.clear();
        self.values.clear();
        self.failures.clear();
        self.stale = false;
    }

    pub(crate) fn begin_run(&mut self) {
        self.audio_running = true;
        self.spectra.clear();
        self.noise_readings.clear();
        self.noise_channels.clear();
        self.activity.clear();
        self.entry_activity.clear();
        self.failures.clear();
        self.stale = false;
    }

    /// The problems to draw on nodes and wires from now on.
    pub fn set_marks(&mut self, marks: Marks) {
        self.marks = marks;
        self.draft_marks.clear();
    }

    pub(crate) fn set_draft_marks(&mut self, nodes: BTreeSet<NodeId>) {
        self.draft_marks = nodes;
    }

    /// Selects a node and moves the view so it is in the middle, at the same zoom.
    pub fn focus(&mut self, graph: &GraphView, node: NodeId) {
        let (Some(place), Some(shown)) = (
            self.places.get(&node).copied(),
            graph.nodes.iter().find(|found| found.id == node),
        ) else {
            return;
        };
        self.select(Some(node));
        let middle = place + self.card(shown).size / 2.0;
        self.origin = middle - self.size / (2.0 * self.zoom);
    }

    pub(crate) fn focus_problem(&mut self, graph: &GraphView, node: NodeId, port: Option<String>) {
        self.focused_failure_port = port
            .filter(|port| {
                self.failure_routes
                    .get(&node)
                    .is_some_and(|routes| routes.contains(port))
            })
            .map(|port| (node, port));
        self.focus(graph, node);
    }

    /// Draws the graph into the space left in `ui` and applies panning, zooming,
    /// selecting and moving. `triggers` are the nodes that get a button to fire them.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        graph: &GraphView,
        triggers: &[NodeId],
    ) -> CanvasResponse {
        self.show_with_path_controls(ui, graph, triggers, false, None)
    }

    pub(crate) fn cancel_node_rename(&mut self) {
        self.node_rename = None;
    }

    pub(crate) fn show_with_path_controls(
        &mut self,
        ui: &mut egui::Ui,
        graph: &GraphView,
        triggers: &[NodeId],
        file_dialogs: bool,
        stored_text: Option<&StoredText<'_>>,
    ) -> CanvasResponse {
        crate::icons::install(ui.ctx());
        let mut done = CanvasResponse::default();
        let alive = !self.stale;
        let animation_time = ui.input(|input| animation_phase(input.time));
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let area = response.rect;
        let previous_size = std::mem::replace(&mut self.size, area.size());
        let colors = Palette::of(ui);
        let untouched = self.fitted == Some((self.zoom, self.origin));
        if self.fit || (untouched && previous_size != self.size) {
            self.fit = false;
            if let Some(bounds) = self.bounds(graph) {
                (self.zoom, self.origin) = Self::frame(bounds, area.size());
                self.fitted = Some((self.zoom, self.origin));
            }
        } else {
            // Resizing the window or a dock keeps the same graph point in view.
            self.origin += (previous_size - self.size) / (2.0 * self.zoom);
        }

        // Dragging the background scrolls; the wheel scrolls, and zooms with the
        // zoom gesture or key, about the pointer.
        let previous_view = (self.origin, self.zoom);
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
                at = snapped(at, self.grid_step);
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
                self.zoom_at(self.zoom * factor, pointer - area.min);
            } else if scroll != Vec2::ZERO {
                self.origin -= scroll / self.zoom;
            }
        }

        let zoom = self.zoom;
        let origin = self.origin;
        let to_screen = |point: Pos2| area.min + (point - origin) * zoom;
        let geometry = Geometry::with_cards(graph, |node| self.card(node));

        painter.rect_filled(area, 0.0, colors.background);
        self.draw_grid(&painter, area, &colors);

        // Groups: their title bars are handled under the cards, and their frames drawn
        // behind them.
        let screen = |rect: Rect| Rect::from_min_max(to_screen(rect.min), to_screen(rect.max));
        let dismiss_group_menu = previous_view != (self.origin, self.zoom)
            || ui.input(|input| {
                input.events.iter().any(|event| matches!(event, egui::Event::MouseWheel { delta, .. } if *delta != Vec2::ZERO))
            });
        self.group_input(
            ui,
            graph,
            screen,
            response.id,
            dismiss_group_menu,
            &mut done,
        );
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
                if self.snap
                    && !free
                    && let Some(anchor) = self.places.get(&node.id).copied()
                {
                    let by = snapped(anchor, self.grid_step) - anchor;
                    for id in &self.selection {
                        if let Some(place) = self.places.get_mut(id) {
                            *place += by;
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
            if zoom >= TITLE_ZOOM
                && !shift
                && !command
                && touched.double_clicked_by(egui::PointerButton::Primary)
                && let Some(at) = touched.interact_pointer_pos()
                && title_area(
                    rect,
                    zoom,
                    self.failures.contains_key(&node.id) || self.marks.nodes.contains_key(&node.id),
                )
                .contains(at)
            {
                self.node_rename = Some(NodeRename::new(node.id, node.name.clone()));
                done.rename_node = Some(node.id);
            }
            if touched.secondary_clicked()
                && let Some(screen) = touched.interact_pointer_pos()
            {
                let (at, screen) = menu_at(screen, self.origin, self.zoom);
                self.front = Some(node.id);
                done.context = Some((crate::MenuTarget::Node(node.id), at, screen));
            }
        }

        if self.node_rename.as_ref().is_some_and(|rename| {
            Some(rename.node) != self.selected
                || zoom < TITLE_ZOOM
                || !graph.nodes.iter().any(|node| node.id == rename.node)
        }) {
            self.cancel_node_rename();
        }

        // Ports: dragging from an output draws a wire; the second mouse button opens
        // the port's menu.
        // Keep a 20-point target even when the connector is drawn small. Below
        // that spacing, constrain targets to their own row so neighbours stay usable.
        let grab = ((PORT_RADIUS + 5.0) * zoom).max(10.0);
        let front = self.front;
        let card_pointer = ui.input(|input| {
            input
                .pointer
                .press_origin()
                .or(input.pointer.interact_pos())
        });
        let card_at_pointer = card_pointer.and_then(|at| {
            node_order(graph, front)
                .rev()
                .find(|node| {
                    !self.hidden.contains(&node.id)
                        && self.places.get(&node.id).is_some_and(|place| {
                            Rect::from_min_size(to_screen(*place), geometry.size(node.id) * zoom)
                                .contains(at)
                        })
                })
                .map(|node| node.id)
        });
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
            for port in geometry
                .card(node.id)
                .rows
                .iter()
                .flat_map(|row| [row.input(), row.right].into_iter().flatten())
            {
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
                    Rect::from_center_size(
                        at,
                        Vec2::new(2.0 * grab, (2.0 * grab).min(card::LINE * zoom)),
                    ),
                    response.id.with((node.id, port.id.as_str(), port.input)),
                    Sense::click_and_drag(),
                );
                if touched.drag_started() {
                    self.wiring = Some(Wiring {
                        from: (node.id, port.id.clone()),
                        exec: port.exec,
                        backward: port.input,
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
                    let hint = if port.id == "error"
                        && self
                            .failure_routes
                            .get(&node.id)
                            .is_some_and(|routes| routes.contains(&port.id))
                    {
                        format!("{hint}\n{}", t("failure-error-hint"))
                    } else {
                        hint
                    };
                    let _ = touched.on_hover_text(hint);
                }
            }
        }
        let pointer = ui
            .input(|input| input.pointer.interact_pos())
            .filter(|at| area.contains(*at));
        if ui.input(|input| input.pointer.any_released())
            && let Some(wiring) = self.wiring.take()
            && let Some(pointer) = pointer
        {
            let at = origin + (pointer - area.min) / zoom;
            done.connect = if wiring.backward {
                self.port_at(graph, at, wiring.exec, wiring.from.0, false)
                    .map(|from| (from, wiring.from.clone()))
            } else {
                self.input_at(graph, at, wiring.exec, wiring.from.0)
                    .map(|to| (wiring.from.clone(), to))
            };
            // Let go on the background, not on a node: ask for a node to take it, or,
            // dragged back from an input, for one to feed it.
            let on_node = self.nodes_in(graph, Rect::from_center_size(at, Vec2::splat(1.0)));
            if done.connect.is_none() && on_node.is_empty() && area.contains(pointer) {
                let loose = Some((wiring.from, [at.x, at.y], pointer));
                if wiring.backward {
                    done.loose_into = loose;
                } else {
                    done.loose = loose;
                }
            }
        }
        // Keys, unless a text field has them: Delete removes the selection, Ctrl+A
        // selects everything, and copy, cut and paste go through the clipboard.
        if self.shortcuts_enabled
            && ui.input(|input| input.focused)
            && !ui.ctx().egui_wants_keyboard_input()
        {
            let (delete, all, add) = ui.input(|input| {
                if let Some(group) = self.selected_group() {
                    if self.keys.pressed(input, "ungroup") {
                        done.group = Some(GroupEdit::Ungroup(group.id.clone()));
                    }
                } else {
                    done.group_selection =
                        self.keys.pressed(input, "group") && !self.selection.is_empty();
                }
                (
                    self.keys.pressed(input, "delete"),
                    self.keys.pressed(input, "select-all"),
                    self.keys.pressed(input, "add"),
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
            ui.input(|input| {
                for event in &input.events {
                    match event {
                        egui::Event::Copy => done.copy = !self.selection.is_empty(),
                        egui::Event::Cut => done.cut = !self.selection.is_empty(),
                        egui::Event::Paste(text) => done.paste = Some(text.clone()),
                        _ => {}
                    }
                }
            });
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
            let ends = (wire.from.clone(), wire.to.clone());
            let mark = self.marks.wires.get(&ends);
            let ty = geometry
                .port(wire.from.0, &wire.from.1, false)
                .and_then(|port| port.ty);
            let color = if let Some(mark) = mark {
                colors.problem(mark.error)
            } else {
                colors.port_color(ty, wire.exec)
            };
            let mut waypoint = self.waypoints.get(&ends).copied();
            if let Some([x, y]) = waypoint {
                let at = to_screen(Pos2::new(x, y));
                let handle = ui
                    .interact(
                        Rect::from_center_size(at, Vec2::splat(14.0 * zoom.max(0.6))),
                        response.id.with(("wire-bend", &ends)),
                        Sense::drag(),
                    )
                    .on_hover_cursor(egui::CursorIcon::Grab);
                if handle.dragged() || handle.drag_stopped() {
                    let drag = handle.id.with("drag-offset");
                    let offset = handle
                        .total_drag_delta()
                        .map(|offset| offset / zoom)
                        .or_else(|| ui.data(|data| data.get_temp::<Vec2>(drag)))
                        .unwrap_or(Vec2::ZERO);
                    waypoint = Some([x + offset.x, y + offset.y]);
                    if handle.drag_stopped() {
                        ui.data_mut(|data| data.remove::<Vec2>(drag));
                        done.waypoint = waypoint.map(|at| (ends.clone(), at));
                    } else {
                        ui.data_mut(|data| data.insert_temp(drag, offset));
                    }
                }
                if let Some([x, y]) = waypoint {
                    painter.circle_filled(to_screen(Pos2::new(x, y)), 4.0 * zoom.max(0.6), color);
                }
            }
            let bend = waypoint.map(|[x, y]| to_screen(Pos2::new(x, y)));
            let points = [
                Some(wire_points(from, bend.unwrap_or(to), zoom)),
                bend.map(|bend| wire_points(bend, to, zoom)),
            ];
            if done.context.is_none()
                && let Some(click) = background_menu
                && points
                    .into_iter()
                    .flatten()
                    .any(|points| near_wire(points, click, 6.0 * zoom.max(0.6)))
            {
                let (at, screen) = menu_at(click, origin, zoom);
                done.context = Some((crate::MenuTarget::Wire(ends.clone()), at, screen));
            }
            let width = if wire.exec { 3.0 } else { 1.8 } * zoom.max(0.6)
                + if mark.is_some() {
                    1.5 * zoom.max(0.6)
                } else {
                    0.0
                };
            for points in points.into_iter().flatten() {
                if !wire_visible(&points, painter.clip_rect(), width) {
                    continue;
                }
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
                let fade = if wire.exec && mark.is_none() {
                    self.exit_glow(&geometry, wire.from.0, &wire.from.1)
                } else {
                    0.0
                };
                if fade > 0.0 {
                    painter.add(CubicBezierShape::from_points_stroke(
                        points,
                        false,
                        Color32::TRANSPARENT,
                        Stroke::new(
                            width + 7.0 * zoom.max(0.6),
                            color.gamma_multiply(0.30 * fade),
                        ),
                    ));
                    painter.add(CubicBezierShape::from_points_stroke(
                        points,
                        false,
                        Color32::TRANSPARENT,
                        Stroke::new(width, mix(color, colors.text, fade * 0.75)),
                    ));
                    pulsing = true;
                } else {
                    painter.add(curve.clone());
                }
                if alive
                    && mark.is_none()
                    && card::stream_type(ty)
                    && !self.failures.contains_key(&wire.from.0)
                    && self
                        .output_sample(wire.from.0, &wire.from.1)
                        .is_some_and(|sample| !sample.preview && sample.epoch == self.value_epoch)
                {
                    for offset in [0.0, 0.25, 0.5, 0.75] {
                        let t = (animation_time + offset).fract();
                        let fade = (t.min(1.0 - t) * 8.0).min(1.0);
                        painter.circle_filled(
                            curve.sample(t),
                            1.65 * zoom.max(0.6),
                            mix(color, colors.text, 0.45).gamma_multiply(fade),
                        );
                    }
                    pulsing = true;
                }
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
            && let Some(from) = anchor(wiring.from.0, &wiring.from.1, wiring.backward)
        {
            // A wire runs from an output to an input, whichever end is held.
            let points = if wiring.backward {
                wire_points(to, from, zoom)
            } else {
                wire_points(from, to, zoom)
            };
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

        if self.wiring.is_none()
            && ui.input(|input| !input.pointer.any_down())
            && let Some(id) = card_at_pointer
            && let Some((sending, event)) = graph
                .nodes
                .iter()
                .find(|node| node.id == id)
                .and_then(|node| crate::palette::signal(node))
        {
            let edge_of = |node, output| {
                let position = to_screen(*self.places.get(&node)?);
                let size = geometry.size(node) * zoom;
                Some(position + Vec2::new(if output { size.x } else { 0.0 }, size.y * 0.5))
            };
            if let Some(here) = edge_of(id, sending) {
                let (_, color, _) = crate::palette::badge("core/emit");
                for peer in &graph.nodes {
                    if !self.hidden.contains(&peer.id)
                        && crate::palette::signal(peer) == Some((!sending, event))
                        && let Some(there) = edge_of(peer.id, !sending)
                        && let Some(points) = clip_signal_line(here, there, area)
                    {
                        painter.extend(egui::Shape::dashed_line(
                            &points,
                            Stroke::new(1.8, color),
                            7.0,
                            5.0,
                        ));
                    }
                }
            }
        }

        // Each card owns its marks. A card in front covers the card and its marks.
        let mut port_marks = BTreeMap::<_, Vec<_>>::new();
        for ((node, port, direction), mark) in &self.marks.ports {
            if let Some(found) = geometry.diagnostic_port(*node, port, *direction) {
                port_marks.entry(*node).or_default().push((
                    found.offset,
                    (found.exec, card::stream_type(found.ty)),
                    self.mark_level(*node, mark),
                ));
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
            // Paint every visible card in order. Suppress input through the card
            // above this one without hiding controls elsewhere on the card.
            let mut card_ui = ui.new_child(
                UiBuilder::new()
                    .max_rect(rect)
                    .id_salt(("card-controls", node.id)),
            );
            card_ui.set_clip_rect(card_ui.clip_rect().intersect(area));
            if card_at_pointer.is_some_and(|above| above != node.id)
                && card_pointer.is_some_and(|at| rect.contains(at))
            {
                card_ui.visuals_mut().disabled_alpha = 1.0;
                card_ui.disable();
            }
            let ui = &mut card_ui;
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
            let chart_open = !self.collapsed_spectra.contains(&node.id);
            let layout = geometry.card(node.id);
            let renaming = self
                .node_rename
                .as_mut()
                .filter(|rename| rename.node == node.id);
            let title = if renaming.is_some() {
                ""
            } else {
                self.display_name(node)
            };
            let (pressed, toggle_failure_routes, slots) = draw_node(
                ui,
                &painter,
                node,
                layout,
                (title, self.kind_name(node)),
                rect,
                zoom,
                &colors,
                (selected, fire, controls, marked),
                response.id,
                formula.as_deref(),
                self.descriptions.get(&node.definition).map(String::as_str),
                (connection, self.connection_selection(node)),
            );
            if let Some(rename) = self
                .node_rename
                .as_mut()
                .filter(|rename| rename.node == node.id)
            {
                let title = title_area(rect, zoom, marked).shrink2(Vec2::new(0.0, zoom));
                let font = FontId::proportional((14.0 * zoom).max(9.0));
                let mut field = ui.new_child(
                    UiBuilder::new()
                        .id_salt("inline-title")
                        .max_rect(title)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                field.set_clip_rect(field.clip_rect().intersect(title));
                if let Some(save) = rename.field(&mut field, font) {
                    if save {
                        done.set_name = Some((node.id, std::mem::take(&mut rename.name)));
                    }
                    self.cancel_node_rename();
                }
            }
            if pressed {
                done.trigger = Some(node.id);
            }
            if toggle_failure_routes {
                self.toggle_failure_routes(node.id);
                ui.ctx().request_repaint();
            }
            // A way out a step just left through glows, whether a wire takes it or not.
            for port in layout
                .rows
                .iter()
                .filter_map(|row| row.right)
                .filter(|port| port.exec)
            {
                let glow = self.exit_glow(&geometry, node.id, &port.id);
                if glow <= 0.0 {
                    continue;
                }
                if let Some(found) = geometry.port(node.id, &port.id, false) {
                    let at = to_screen(place + found.offset);
                    for (radius, alpha) in [(3.2, 0.18), (2.2, 0.35)] {
                        painter.circle_filled(
                            at,
                            PORT_RADIUS * radius * zoom,
                            colors.exec.gamma_multiply(alpha * glow),
                        );
                    }
                    pulsing = true;
                }
            }
            if spectrum::combined(node) && zoom >= 0.45 {
                let pointer = ui.input(|input| input.pointer.hover_pos());
                let mut pointed = None;
                for row in &layout.rows {
                    let Some(band) = row
                        .right
                        .and_then(|port| spectrum::primary_band(node, port))
                    else {
                        continue;
                    };
                    let top = rect.top() + row.top * zoom;
                    if pointer.is_some_and(|at| {
                        Rect::from_min_max(
                            Pos2::new(rect.left(), top),
                            Pos2::new(rect.right() + 8.0 * zoom, top + row.height * zoom),
                        )
                        .contains(at)
                    }) {
                        pointed = Some(band);
                    }
                    if let Some(edit) =
                        spectrum::band_row(ui, &painter, node, row, rect, zoom, controls, band)
                    {
                        done.spectrum_edit = Some((node.id, edit));
                    }
                }
                let toggle = response.id.with(("spectrum-chart", node.id));
                if spectrum::chart_toggle(ui, &painter, rect, zoom, toggle, chart_open) {
                    if chart_open {
                        self.collapsed_spectra.insert(node.id);
                    } else {
                        self.collapsed_spectra.remove(&node.id);
                    }
                    ui.ctx().request_repaint();
                } else if chart_open
                    && let Some(edit) = spectrum::draw_combined(
                        ui,
                        &painter,
                        node,
                        rect,
                        zoom,
                        self.spectra.get(&node.id).map(Vec::as_slice),
                        controls,
                        pointed,
                        self.audio_running,
                    )
                {
                    done.spectrum_edit = Some((node.id, edit));
                }
            } else if spectrum::is_noise(node)
                && zoom >= 0.45
                && let Some(threshold) = spectrum::draw_noise(
                    ui,
                    &painter,
                    node,
                    rect,
                    zoom,
                    (
                        self.noise_readings.get(&node.id).copied(),
                        self.noise_channels.get(&node.id).copied(),
                    ),
                    controls,
                    self.audio_running,
                )
            {
                done.set_config = Some((node.id, "/threshold_db".to_owned(), threshold.into()));
            }
            // Keep each value in its side of the row. Click a value to inspect it.
            let label_font = FontId::proportional(12.5 * zoom);
            let inset = CARD_PADDING * zoom;
            for row in &layout.rows {
                let part = row.input().and_then(|port| {
                    crate::properties::log_part_label(node, &port.id).map(|label| (port, label))
                });
                if controls && let Some((port, label)) = &part {
                    let target = Rect::from_center_size(
                        Pos2::new(
                            rect.right() - inset - 8.0 * zoom,
                            rect.top() + row.anchor() * zoom,
                        ),
                        Vec2::splat(18.0 * zoom),
                    );
                    let remove = tf("log-remove-part", &[("part", label.as_str().into())]);
                    let touched = ui.interact(
                        target,
                        response.id.with(("remove-log-part", node.id, &port.id)),
                        Sense::click(),
                    );
                    touched.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &remove)
                    });
                    let touched = touched
                        .on_hover_text(remove)
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    crate::icons::paint_centered(
                        &painter,
                        target.center(),
                        crate::icons::CLOSE,
                        14.0 * zoom,
                        if touched.hovered() {
                            colors.text
                        } else {
                            colors.weak
                        },
                    );
                    if touched.clicked() {
                        done.remove_port = Some((node.id, port.id.clone()));
                    }
                }
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
                    // A stream carries no value to show, and an empty one draws no box.
                    if zoom < 0.6
                        || card::stream_type(port.ty.as_deref())
                        || value.shown.short.trim().is_empty()
                    {
                        continue;
                    }
                    let label = part
                        .as_ref()
                        .filter(|_| input)
                        .map_or_else(|| port_title(port), |(_, label)| label.clone());
                    let name = painter.layout_no_wrap(label, label_font.clone(), colors.text);
                    let half = rect.width() / 2.0 - 1.5 * inset;
                    let gap = 8.0 * zoom;
                    let name_width = name.size().x.min(half);
                    let y = rect.top() + row.anchor() * zoom;
                    let (at, align, room) = if input {
                        let room = if output.is_some() {
                            rect.width() / 2.0 - inset
                        } else {
                            rect.width()
                        } - name_width
                            - 2.0 * inset
                            - gap
                            - if part.is_some() { 20.0 * zoom } else { 0.0 };
                        (
                            Pos2::new(rect.left() + inset + name_width + gap, y),
                            Align2::LEFT_CENTER,
                            room,
                        )
                    } else {
                        (
                            Pos2::new(rect.right() - inset - name_width - gap, y),
                            Align2::RIGHT_CENTER,
                            if row.left.is_some() {
                                half - name_width - gap
                            } else {
                                rect.width() - 2.0 * inset - name_width - gap
                            },
                        )
                    };
                    let font = FontId::monospace(11.5 * zoom);
                    let tint = if value.preview || value.epoch == self.value_epoch {
                        colors.text
                    } else {
                        colors.weak
                    };
                    let label_x = if input {
                        rect.left() + inset
                    } else {
                        rect.right() - inset
                    };
                    let label_bounds = align.anchor_rect(Rect::from_min_size(
                        Pos2::new(label_x, y),
                        Vec2::new(name_width, name.size().y),
                    ));
                    let target = if room >= 12.0 * zoom {
                        let (shown, _) = elide(&painter, &value.shown.short, &font, room);
                        let galley = painter.layout_no_wrap(shown, font, tint);
                        let bounds = align.anchor_rect(Rect::from_min_size(at, galley.size()));
                        let chip = bounds.expand2(Vec2::new(5.0, 2.0) * zoom);
                        painter.rect_filled(chip, 3.0 * zoom, colors.background);
                        painter.galley(bounds.min, galley, tint);
                        label_bounds.union(chip)
                    } else {
                        label_bounds
                    };
                    let touched = ui
                        .interact(
                            target,
                            response.id.with(("value", node.id, &port.id, input)),
                            Sense::click(),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(t("inspect-port-value"));
                    touched.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, true, t("inspect-port-value"))
                    });
                    if touched.clicked() {
                        done.inspect_value = Some((node.id, port.id.clone(), input));
                    }
                }
            }
            for (field, slot) in slots {
                // A card's fields stay usable where the card shows, also beside a card
                // in front of it.
                if !area.intersects(slot) {
                    continue;
                }
                let id = response.id.with(("field", node.id));
                let id = match field {
                    card::FieldOf::TextArea(port, ..)
                    | card::FieldOf::Input(port)
                    | card::FieldOf::Path(port, _) => id.with(("port", &port.id)),
                    card::FieldOf::Setting(name, ..) => id.with(("setting", name)),
                    card::FieldOf::Variable(pointer) => id.with(("variable", pointer)),
                    card::FieldOf::Connection => id.with("connection"),
                    card::FieldOf::AddPort(variadic) => id.with(("add", &variadic.port)),
                    card::FieldOf::AddSignalReceiver => id.with("add-signal"),
                };
                let Some(edit) = self.field(ui, node, field, slot, id, file_dialogs, stored_text)
                else {
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
                    fields::FieldEdit::AddSignalReceiver => {
                        done.add_signal_receiver = Some(node.id)
                    }
                    fields::FieldEdit::RefreshDevices(adapter) => {
                        done.refresh_devices = Some(adapter);
                    }
                }
            }
            let node = node.id;
            for &(offset, shape, level) in port_marks.get(&node).into_iter().flatten() {
                draw_port_shape(
                    &painter,
                    rect.min + offset * zoom,
                    (PORT_RADIUS + 2.5) * zoom,
                    shape,
                    Color32::TRANSPARENT,
                    Stroke::new(1.5 * zoom.max(0.6), colors.problem_level(level)),
                );
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
                    (&failed, ProblemLevel::Blocker),
                    response.id.with(("failed", node)),
                );
            } else if let Some(mark) = self.marks.nodes.get(&node) {
                done.show_problems |= draw_mark(
                    ui,
                    &painter,
                    rect,
                    zoom,
                    &colors,
                    (mark, self.mark_level(node, mark)),
                    response.id.with(("mark", node)),
                );
            }
        }
        if pulsing {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
        // While a wire is drawn from an output, the inputs it could go to are ringed.
        if let Some(wiring) = self.wiring.as_ref().filter(|wiring| !wiring.backward) {
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
                place = snapped(place, self.grid_step);
            }
            let at = to_screen(place);
            if let Some(card) = &drag.card {
                let rect = Rect::from_min_size(at, node_size(card) * zoom);
                let _ = draw_node(
                    ui,
                    &painter,
                    card,
                    &card::Card::new(card),
                    (self.display_name(card), self.kind_name(card)),
                    rect,
                    zoom,
                    &colors,
                    (
                        true,
                        (card.definition == "core/on-manual").then_some(false),
                        false,
                        false,
                    ),
                    response.id,
                    None,
                    None,
                    (
                        self.connection_labels
                            .get(&card.definition)
                            .map(String::as_str),
                        None,
                    ),
                );
                if zoom >= 0.45 {
                    ui.scope_builder(UiBuilder::new().id_salt("placement-preview"), |ui| {
                        ui.disable();
                        if spectrum::is_noise(card) {
                            let _ = spectrum::draw_noise(
                                ui,
                                &painter,
                                card,
                                rect,
                                zoom,
                                (None, None),
                                false,
                                false,
                            );
                        } else if spectrum::combined(card) {
                            let _ = spectrum::chart_toggle(
                                ui,
                                &painter,
                                rect,
                                zoom,
                                response.id.with("preview-chart"),
                                true,
                            );
                            let _ = spectrum::draw_combined(
                                ui, &painter, card, rect, zoom, None, false, None, false,
                            );
                        }
                    });
                }
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

/// Adds a round grid point to the shared grid mesh, without a shape per point.
fn grid_dot(mesh: &mut egui::Mesh, center: Pos2, radius: f32, color: Color32) {
    const D: f32 = std::f32::consts::FRAC_1_SQRT_2;
    const OUTLINE: [(f32, f32); 8] = [
        (1.0, 0.0),
        (D, D),
        (0.0, 1.0),
        (-D, D),
        (-1.0, 0.0),
        (-D, -D),
        (0.0, -1.0),
        (D, -D),
    ];
    let first = mesh.vertices.len() as u32;
    for (x, y) in OUTLINE {
        mesh.colored_vertex(center + Vec2::new(x, y) * radius, color);
    }
    for edge in 1..7 {
        mesh.add_triangle(first, first + edge, first + edge + 1);
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
    if ty == "core/duration@1" {
        return fields::duration_nanos(text).map(|_| text.to_owned());
    }
    if whole {
        return parse_whole(text, ty).map(|value| value.to_string());
    }
    let value: f64 = text.trim().parse().ok()?;
    value.is_finite().then(|| value.to_string())
}

/// A number field's value after a drag of `pixels` from `value`: one step for each
/// five pixels for a whole number, a hundredth for each pixel for a decimal.
fn stepped(value: Option<&str>, pixels: f32, whole: bool, ty: &str) -> String {
    if ty == "core/duration@1" {
        let start = value
            .and_then(fields::duration_nanos)
            .and_then(|ns| ns.parse::<i64>().ok())
            .unwrap_or(0);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "mouse steps are rounded to milliseconds"
        )]
        let delta = ((pixels / 4.0).round() as i64).saturating_mul(1_000_000);
        return fields::duration_millis(&start.saturating_add(delta).to_string())
            .unwrap_or_default();
    }
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
    // Bound text submitted to the font cache even when a document has huge labels.
    // Full values remain available to the inspector and copy actions.
    let end = text.floor_char_boundary(text.len().min(4096));
    let prefix = &text[..end];
    let fits = |value: &str| {
        painter
            .layout_no_wrap(value.to_owned(), font.clone(), Color32::WHITE)
            .size()
            .x
            <= width
    };
    if end == text.len() && fits(prefix) {
        return (prefix.to_owned(), false);
    }
    let mut low = 0;
    let mut high = prefix.len();
    let mut best = String::from("…");
    while low < high {
        let mid = prefix.ceil_char_boundary(low + (high - low).div_ceil(2));
        let candidate = format!("{}…", prefix[..mid].trim_end());
        if fits(&candidate) {
            low = mid;
            best = candidate;
        } else {
            high = prefix.floor_char_boundary(mid - 1);
        }
    }
    (best, true)
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
    (mark, level): (&Mark, ProblemLevel),
    id: egui::Id,
) -> bool {
    let center = mark_position(rect, zoom);
    let color = colors.problem_level(level);
    if zoom >= 0.45 {
        let _ = painter.text(
            center,
            Align2::CENTER_CENTER,
            level.glyph(),
            FontId::new(MARK_SIZE * zoom, crate::icons::family()),
            color,
        );
    } else {
        painter.circle_filled(center, 2.5 * zoom.max(0.6), color);
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
            ui.strong(level.label());
            ui.label(&mark.text);
            ui.small(t("open-details-hint"));
        })
        .clicked()
}

/// How big a card's problem marker is, in graph units.
const MARK_SIZE: f32 = 15.0;
const TITLE_ZOOM: f32 = 0.33;

fn title_area(rect: Rect, zoom: f32, marked: bool) -> Rect {
    let marker = if marked { MARK_SIZE + 6.0 } else { 0.0 };
    Rect::from_min_max(
        rect.min + Vec2::new(CARD_PADDING * zoom + (20.0 * zoom).max(11.0), 0.0),
        rect.right_top() + Vec2::new(-(CARD_PADDING + marker) * zoom, HEADER_HEIGHT * zoom),
    )
}

pub(crate) fn node_title_color(visuals: &egui::Visuals) -> Color32 {
    if visuals.dark_mode {
        Color32::WHITE
    } else {
        visuals.strong_text_color()
    }
}

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
        .map(|node| &**node)
}

/// Paints a read-only canvas card inside its search result.
pub(crate) fn preview_card(ui: &mut egui::Ui, node: &NodeView, name: &str, rect: Rect, zoom: f32) {
    let layout = card::Card::new(node);
    let painter = ui
        .painter()
        .with_clip_rect(ui.clip_rect().intersect(rect.expand(6.0)));
    let colors = Palette::of(ui);
    let _ = draw_node(
        ui,
        &painter,
        node,
        &layout,
        (name, name),
        rect,
        zoom,
        &colors,
        (false, None, false, false),
        ui.scope_id(),
        None,
        None,
        (None, None),
    );
    if zoom >= 0.45 {
        if spectrum::combined(node) {
            spectrum::chart_heading(&painter, rect, zoom, true, colors.text);
            let _ =
                spectrum::draw_combined(ui, &painter, node, rect, zoom, None, false, None, false);
        } else if spectrum::is_noise(node) {
            let _ =
                spectrum::draw_noise(ui, &painter, node, rect, zoom, (None, None), false, false);
        }
    }
}

/// Draws one card. Returns whether its trigger or failure routes button was pressed.
/// `flags` are whether the card is selected, has a trigger button and shows controls
/// for its values. Also returns where each input's control goes.
#[expect(clippy::too_many_arguments, reason = "one card's drawing state")]
fn draw_node<'a>(
    ui: &egui::Ui,
    painter: &egui::Painter,
    node: &'a NodeView,
    layout: &card::Card<'a>,
    (name, kind): (&str, &str),
    rect: Rect,
    zoom: f32,
    colors: &Palette,
    (selected, fire, controls, marked): (bool, Option<bool>, bool, bool),
    canvas: egui::Id,
    formula: Option<&str>,
    description: Option<&str>,
    (connection, profile): (Option<&str>, Option<&str>),
) -> (bool, bool, Vec<(card::FieldOf<'a>, Rect)>) {
    let radius = CARD_RADIUS * zoom;
    let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), HEADER_HEIGHT * zoom));
    painter.rect_filled(rect, radius, colors.card);
    // The header carries the node family's color: a tint, and a line along its foot,
    // so nodes of one package are told apart at a glance, even zoomed out.
    let family = family_color(&node.definition);
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

    // At overview zoom, the title remains readable while the body simplifies.
    let readable = zoom >= 0.45;
    let pad = CARD_PADDING * zoom;
    let font = FontId::proportional(12.5 * zoom);
    let mut pressed = false;
    let mut toggle_failure_routes = false;
    let mut slots = Vec::new();
    if zoom >= TITLE_ZOOM {
        let title = FontId::proportional((14.0 * zoom).max(9.0));
        let clip = header.shrink2(Vec2::new(pad, 0.0));
        let header_painter = painter.with_clip_rect(clip.intersect(painter.clip_rect()));
        let left = clip.left_center();
        crate::icons::paint_centered(
            &header_painter,
            left + Vec2::new((7.0 * zoom).max(4.0), 0.0),
            family_icon(node),
            (15.0 * zoom).max(8.0),
            colors.text,
        );
        let label = title_area(rect, zoom, marked);
        let (shown, cut) = elide(painter, name, &title, label.width());
        let _ = header_painter.text(
            label.left_center(),
            Align2::LEFT_CENTER,
            &shown,
            title,
            node_title_color(ui.visuals()),
        );
        // A node whose package is missing says which one first.
        let needs = node
            .missing
            .as_ref()
            .map(crate::properties::missing_package_text);
        let detail = needs
            .as_deref()
            .or(node.detail.as_deref())
            .or_else(|| (node.name != node.kind).then_some(kind));
        if cut || detail.is_some() || description.is_some() || formula.is_some() {
            let _ = ui
                .interact(clip, canvas.with(("title", node.id)), Sense::hover())
                .on_hover_ui(|ui| {
                    // One paragraph: what the node does, else what it is set to.
                    ui.set_max_width(300.0);
                    if let Some(detail) = detail {
                        ui.strong(detail);
                    }
                    if let Some(description) = description {
                        ui.label(description);
                    }
                    if detail.is_none() && description.is_none() && cut {
                        ui.label(name);
                    }
                    if let Some(formula) = formula {
                        ui.monospace(formula);
                    }
                });
        }
    }
    if readable && layout.empty {
        let _ = painter.text(
            Pos2::new(rect.center().x, rect.top() + layout.rows[0].anchor() * zoom),
            Align2::CENTER_CENTER,
            t("set-up-in-properties"),
            FontId::proportional(11.5 * zoom),
            colors.weak,
        );
    }
    let half = rect.width() / 2.0 - 1.5 * pad;
    for row in &layout.rows {
        let y = rect.top() + row.anchor() * zoom;
        // The output first: an input's name on the same line leaves it room.
        let mut left_room = rect.width() - 2.0 * pad;
        if let Some(port) = row.right {
            draw_dot(painter, port, Pos2::new(rect.right(), y), zoom, colors);
            let literal_output = matches!(row.left, Some(card::Left::Field { named: false, .. }));
            let band = spectrum::primary_band(node, port).is_some();
            if readable && !literal_output && !(band && controls) {
                let reserved = if port.exec {
                    17.0 * zoom
                } else if band {
                    23.0 * zoom
                } else {
                    0.0
                };
                let (label, _) = elide(
                    painter,
                    &port_title(port),
                    &font,
                    (half - reserved).max(0.0),
                );
                let end = Pos2::new(rect.right() - pad - if band { 23.0 * zoom } else { 0.0 }, y);
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
            Some(card::Left::FailureRoutes { expanded }) if readable => {
                let height = (row.height - 4.0) * zoom;
                let button = Rect::from_center_size(
                    Pos2::new(rect.center().x, y),
                    Vec2::new(rect.width() - 2.0 * pad, height),
                );
                toggle_failure_routes = failure_routes_button(
                    ui,
                    painter,
                    button,
                    zoom,
                    colors,
                    canvas.with(("failure-routes", node.id)),
                    expanded,
                );
            }
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
                let label = if node.definition == "core/gate" && port.id == "enter" {
                    t("port-enter-alias").to_owned()
                } else {
                    crate::properties::log_part_label(node, &port.id)
                        .unwrap_or_else(|| port_title(port))
                };
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
                    let at = Pos2::new(rect.left(), rect.top() + row.input_anchor() * zoom);
                    draw_dot(painter, port, at, zoom, colors);
                }
                if !readable {
                    continue;
                }
                if named {
                    let mut label = fields::name(node, editor, connection);
                    if editor
                        .port()
                        .is_some_and(|port| crate::properties::is_folder_path(node, &port.id))
                    {
                        label = crate::properties::folder_path_label(node, label, profile);
                    }
                    let (label, _) = elide(
                        painter,
                        &label,
                        &font,
                        row.field_end
                            .map_or(left_room - (row.field_inset * zoom - pad), |end| {
                                (end - row.field_inset) * zoom
                            }),
                    );
                    let _ = painter.text(
                        Pos2::new(rect.left() + row.field_inset * zoom, y),
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
                        fields::paint_still(ui, painter, node, editor, slot, zoom, colors, profile);
                    }
                }
            }
            Some(card::Left::FailureRoutes { .. }) | None => {}
        }
    }
    (pressed, toggle_failure_routes, slots)
}

fn failure_routes_button(
    ui: &egui::Ui,
    painter: &egui::Painter,
    rect: Rect,
    zoom: f32,
    colors: &Palette,
    id: egui::Id,
    expanded: bool,
) -> bool {
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_text(t("failure-routes-hint"))
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::Role::Button,
            ui.is_enabled(),
            expanded,
            t("failure-routes"),
        )
    });
    let tint = if response.hovered() || response.has_focus() {
        colors.selected
    } else {
        colors.text
    };
    if response.has_focus() {
        painter.rect_stroke(rect, 4.0 * zoom, Stroke::new(1.0, tint), StrokeKind::Inside);
    }
    let left = rect.left_center() + Vec2::X * (4.0 * zoom);
    painter.text(
        left,
        Align2::LEFT_CENTER,
        if expanded {
            crate::icons::COLLAPSE
        } else {
            crate::icons::EXPAND
        },
        FontId::new(14.0 * zoom, crate::icons::family()),
        tint,
    );
    painter.text(
        left + Vec2::X * (18.0 * zoom),
        Align2::LEFT_CENTER,
        t("failure-routes"),
        FontId::proportional(12.0 * zoom),
        tint,
    );
    response.clicked()
}

/// Draws a connector shape, filled when wired.
fn draw_dot(painter: &egui::Painter, port: &PortView, at: Pos2, zoom: f32, colors: &Palette) {
    let radius = PORT_RADIUS * zoom;
    let color = colors.port_color(port.ty.as_deref(), port.exec);
    let fill = if port.connected { color } else { colors.card };
    let outline = if port.connected {
        Stroke::NONE
    } else {
        Stroke::new(1.5 * zoom.max(0.6), color)
    };
    draw_port_shape(
        painter,
        at,
        radius,
        (port.exec, card::stream_type(port.ty.as_deref())),
        fill,
        outline,
    );
}

fn draw_port_shape(
    painter: &egui::Painter,
    at: Pos2,
    radius: f32,
    (exec, stream): (bool, bool),
    fill: Color32,
    outline: Stroke,
) {
    if exec {
        let corner = radius * 1.2;
        painter.add(egui::Shape::convex_polygon(
            vec![
                at - Vec2::X * corner,
                at - Vec2::Y * corner,
                at + Vec2::X * corner,
                at + Vec2::Y * corner,
            ],
            fill,
            outline,
        ));
    } else if stream {
        painter.rect(
            Rect::from_center_size(at, Vec2::splat(radius * 1.8)),
            0.0,
            fill,
            outline,
            StrokeKind::Inside,
        );
    } else {
        painter.circle(at, radius, fill, outline);
    }
}

fn clip_signal_line(from: Pos2, to: Pos2, area: Rect) -> Option<[Pos2; 2]> {
    let delta = to - from;
    let (mut first, mut last) = (0.0_f32, 1.0_f32);
    for (position, movement, min, max) in [
        (from.x, delta.x, area.left(), area.right()),
        (from.y, delta.y, area.top(), area.bottom()),
    ] {
        if movement == 0.0 {
            if !(min..=max).contains(&position) {
                return None;
            }
        } else {
            let a = (min - position) / movement;
            let b = (max - position) / movement;
            first = first.max(a.min(b));
            last = last.min(a.max(b));
        }
    }
    (first <= last).then(|| [from + delta * first, from + delta * last])
}

fn luminance(color: Color32) -> f32 {
    let linear = egui::Rgba::from(color);
    0.2126 * linear.r() + 0.7152 * linear.g() + 0.0722 * linear.b()
}

fn family_icon(node: &NodeView) -> char {
    let (icon, _, family) = crate::palette::badge(&node.definition);
    if node.entry && family == "node-family-package" {
        crate::icons::FLOW
    } else {
        icon
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
    let (_, color, category) = crate::palette::badge(definition);
    if category != "node-family-package" {
        return color;
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
    use std::sync::Arc;

    #[test]
    fn diagnostic_shapes_share_the_connector_center() {
        let context = egui::Context::default();
        let center = Pos2::new(51.0, 73.0);
        let outline = Color32::from_rgb(231, 19, 127);
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            for geometry in [(false, false), (false, true), (true, false)] {
                draw_port_shape(
                    ui.painter(),
                    center,
                    7.0,
                    geometry,
                    Color32::TRANSPARENT,
                    Stroke::new(1.0, outline),
                );
            }
        });
        let outlines: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Circle(shape) if shape.stroke.color == outline => {
                    Some((0, shape.center))
                }
                egui::Shape::Rect(shape) if shape.stroke.color == outline => {
                    Some((1, shape.rect.center()))
                }
                egui::Shape::Path(shape) if shape.closed && shape.points.len() == 4 => {
                    Some((2, Rect::from_points(&shape.points).center()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(outlines, [(0, center), (1, center), (2, center)]);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn connector_colors_contrast_on_the_default_light_and_dark_surfaces() {
        for visuals in [egui::Visuals::light(), egui::Visuals::dark()] {
            let ctx = egui::Context::default();
            ctx.run_ui(egui::RawInput::default(), |ui| {
                *ui.visuals_mut() = visuals.clone();
                let colors = Palette::of(ui);
                for ty in [
                    None,
                    Some("core/bool@1"),
                    Some("core/i64@1"),
                    Some("core/string@1"),
                    Some("core/stream@1<core/i64@1>"),
                    Some("local/custom@1"),
                ] {
                    for exec in [false, true] {
                        let ink = luminance(colors.port_color(ty, exec));
                        for surface in [colors.card, colors.background] {
                            let surface = luminance(surface);
                            assert!((ink.max(surface) + 0.05) / (ink.min(surface) + 0.05) >= 3.0);
                        }
                    }
                }
            })
            .drop_without_applying_deltas();
        }
    }

    #[test]
    fn a_successful_action_pulses_its_then_wire_even_with_a_failure_output() {
        let node = node(
            1,
            vec![port("then", false, true), port("failed", false, true)],
        );
        let id = node.id;
        let graph = GraphView {
            nodes: vec![Arc::new(node)],
            ..GraphView::default()
        };
        let geometry = Geometry::new(&graph);
        let mut canvas = Canvas::default();
        canvas.record_step(id, false, orange_host::ControlOutcome::Then);
        assert!(canvas.exit_glow(&geometry, id, "then") > 0.0);
        assert_eq!(canvas.exit_glow(&geometry, id, "failed"), 0.0);
        canvas.record_step(id, true, orange_host::ControlOutcome::Failed);
        assert_eq!(canvas.exit_glow(&geometry, id, "then"), 0.0);
    }

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
            missing: None,
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
                node(1, vec![port("then", false, true)]).into(),
                node(2, step()).into(),
                node(
                    3,
                    vec![
                        port("in", true, true),
                        port("a", true, false),
                        port("b", true, false),
                    ],
                )
                .into(),
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
            nodes: vec![source.into()],
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
            Some(Vec2::new(0.0, layout.rows[2].input_anchor()))
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
        Arc::make_mut(&mut graph.nodes[1]).position = Some([0.0, 0.0]);
        Arc::make_mut(&mut graph.nodes[2]).position = Some([0.0, 0.0]);
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
        Arc::make_mut(&mut graph.nodes[1])
            .ports
            .push(port("in", false, false));
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
        Arc::make_mut(&mut graph.nodes[1]).ports.reverse();
        Arc::make_mut(&mut graph.nodes[1]).ports[0].ty = Some("core/string@1".into());
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
            Arc::make_mut(node).position = Some([n as f32 * -100.0, 50.0]);
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
        Arc::make_mut(&mut graph.nodes[1]).ports[0].value = Some("changed".into());
        canvas.update(&graph);
        assert_eq!(canvas.places(), places);
        graph.nodes.remove(1);
        let mut added = node(4, vec![]);
        added.position = Some([-100.0, -200.0]);
        graph.nodes.push(added.into());
        canvas.update(&graph);
        assert_eq!(canvas.selected(), None);
        assert!(!canvas.places.contains_key(&id(2)));
        assert_eq!(canvas.places[&id(4)], Pos2::new(-100.0, -200.0));
        graph.nodes.push(node(5, vec![]).into());
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

        Arc::make_mut(&mut graph.nodes[1]).position = Some([500.0, -20.0]);
        assert_eq!(auto_layout(&graph)[&id(2)], Pos2::new(500.0, -20.0));

        // A cycle still gets places, and two nodes of one column do not overlap.
        graph.wires.push(wire(3, 1));
        assert_eq!(auto_layout(&graph).len(), 3);
        let fan = GraphView {
            nodes: vec![
                node(1, vec![]).into(),
                node(2, vec![]).into(),
                node(3, vec![]).into(),
            ],
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
        edited.nodes.push(added.into());
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
    fn drawing_a_band_and_setting_the_noise_threshold_use_the_canvas_widgets() {
        let (host, _) = orange_host::Host::with_services(
            orange_host::HostOptions::default(),
            vec![orange_audio::package()],
        )
        .unwrap();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        let spectrum = host
            .add_node(&mut draft, orange_audio::MULTIBAND_SPECTRUM, [50.0, 70.0])
            .unwrap();
        let noise = host
            .add_node(&mut draft, orange_audio::NOISE_LEVEL, [490.0, 70.0])
            .unwrap();
        let graph = host.draft_view(&draft).unwrap();
        let mut canvas = Canvas::new();
        canvas.load(&graph);
        canvas.origin = Pos2::ZERO;
        canvas.zoom = 1.0;
        canvas.fit = false;
        canvas.size = Vec2::new(1000.0, 740.0);
        let context = egui::Context::default();
        let mut frame = |events| {
            let mut response = CanvasResponse::default();
            let output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 740.0))),
                    events,
                    ..Default::default()
                },
                |ui| response = canvas.show(ui, &graph, &[]),
            );
            (output, response)
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for (left, width) in [(188.0, 18.0), (226.0, 4.0)] {
            let start = Pos2::new(left, 70.0 + BODY_TOP + 40.0);
            let end = start + Vec2::new(width, 0.0);
            frame(vec![egui::Event::PointerMoved(start)])
                .0
                .drop_without_applying_deltas();
            frame(vec![button(start, true)])
                .0
                .drop_without_applying_deltas();
            let (dragging, response) = frame(vec![egui::Event::PointerMoved(end)]);
            dragging.drop_without_applying_deltas();
            assert!(response.spectrum_edit.is_none());
            let (release, response) = frame(vec![button(end, false)]);
            release.drop_without_applying_deltas();
            assert!(!response.moved, "drawing a range must not move its card");
            let Some((owner, BandEdit::Add(range))) = response.spectrum_edit else {
                panic!("releasing a frequency range must yield one band edit");
            };
            assert_eq!(owner, spectrum);
            assert!(range[0] < range[1]);
            host.add_audio_band(&mut draft, owner, range).unwrap();
        }
        let placed = host.draft_view(&draft).unwrap();
        assert_eq!(placed.nodes.len(), 2);
        assert_eq!(
            placed
                .nodes
                .iter()
                .find(|node| node.id == spectrum)
                .unwrap()
                .ports
                .iter()
                .filter(
                    |port| !port.input && port.ty.as_deref() == Some("org.orange.audio/stream@1")
                )
                .map(|port| port.id.as_str())
                .collect::<Vec<_>>(),
            ["band_stream#0", "band_stream#1"]
        );

        let at = Pos2::new(526.0, 70.0 + BODY_TOP + 103.0);
        frame(vec![egui::Event::PointerMoved(at), button(at, true)])
            .0
            .drop_without_applying_deltas();
        let (output, response) = frame(vec![button(at, false)]);
        output.drop_without_applying_deltas();
        let (owner, pointer, value) = response.set_config.expect("meter threshold edit");
        assert_eq!((owner, pointer.as_str()), (noise, "/threshold_db"));
        assert!((-65.0..=-55.0).contains(&value.as_f64().unwrap()));

        let card = host
            .placement_preview(&draft, orange_audio::NOISE_LEVEL)
            .unwrap();
        egui::DragAndDrop::set_payload(
            &context,
            crate::palette::PaletteDrag {
                card: Some(card),
                item: crate::PaletteItem {
                    id: orange_audio::NOISE_LEVEL.into(),
                    label: "Audio Level Trigger".into(),
                    package: String::new(),
                    purpose: String::new(),
                },
            },
        );
        let (preview, _) = frame(vec![egui::Event::PointerMoved(Pos2::new(220.0, 220.0))]);
        for label in ["-100", t("audio-setting-hold-ms")] {
            assert!(
                preview.shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.pos.y > 260.0 && text.pos.x < 480.0
                        && text.galley.text() == label
                )),
                "drag preview must show {label}"
            );
        }
        preview.drop_without_applying_deltas();
    }

    #[test]
    fn overlapping_cards_keep_visible_settings() {
        let (host, _) = orange_host::Host::with_services(
            orange_host::HostOptions::default(),
            vec![orange_audio::package()],
        )
        .unwrap();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        let _ = host
            .add_node(&mut draft, orange_audio::NOISE_LEVEL, [50.0, 50.0])
            .unwrap();
        let front = host
            .add_node(&mut draft, "core/number", [200.0, 75.0])
            .unwrap();
        let graph = host.draft_view(&draft).unwrap();
        let mut canvas = Canvas::new();
        canvas.load(&graph);
        canvas.select(Some(front));
        canvas.origin = Pos2::ZERO;
        canvas.fit = false;
        canvas.size = Vec2::new(800.0, 600.0);
        let context = egui::Context::default();
        let output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, canvas.size)),
                ..Default::default()
            },
            |ui| {
                let _ = canvas.show(ui, &graph, &[]);
            },
        );
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text() == t("audio-setting-hold-ms")
        )));
        output.drop_without_applying_deltas();
    }

    #[test]
    fn spectrum_collapse_keeps_inline_band_controls_at_each_zoom() {
        let (host, _) = orange_host::Host::with_services(
            orange_host::HostOptions::default(),
            vec![orange_audio::package()],
        )
        .unwrap();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        let position = Pos2::new(100.0, 80.0);
        let node = host
            .add_node(
                &mut draft,
                orange_audio::MULTIBAND_SPECTRUM,
                [position.x, position.y],
            )
            .unwrap();
        host.add_audio_band(&mut draft, node, [400, 600]).unwrap();
        host.add_audio_band(&mut draft, node, [1000, 1100]).unwrap();
        let graph = host.draft_view(&draft).unwrap();
        let mut canvas = Canvas::new();
        canvas.load(&graph);
        canvas.spectra.insert(node, vec![-35.0; 17]);
        canvas.origin = Pos2::ZERO;
        canvas.fit = false;
        let size = Vec2::new(1100.0, 850.0);
        canvas.size = size;
        let context = egui::Context::default();
        crate::theme::apply(&context);
        crate::icons::install(&context);
        let mut frame = |events| {
            let mut response = CanvasResponse::default();
            let output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                    events,
                    ..Default::default()
                },
                |ui| response = canvas.show(ui, &graph, &[]),
            );
            let top = Pos2::ZERO + (position - canvas.origin) * canvas.zoom;
            let card = Rect::from_min_size(top, canvas.card(&graph.nodes[0]).size * canvas.zoom);
            let result = (output.shapes.clone(), response, card, canvas.zoom);
            output.drop_without_applying_deltas();
            result
        };
        let anchor = Pos2::new(40.0, 50.0);
        let (initial, _, expanded, _) = frame(vec![egui::Event::PointerMoved(anchor)]);
        let trace_drawn = |shapes: &[egui::epaint::ClippedShape]| {
            shapes.iter().any(
                |shape| matches!(&shape.shape, egui::Shape::Mesh(mesh) if mesh.vertices.len() >= 17 && mesh.vertices.len() <= 68),
            )
        };
        assert!(trace_drawn(&initial));
        let toggle = initial
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == t("spectrum-live-trace") => {
                    Some(text.pos + text.galley.size() / 2.0)
                }
                _ => None,
            })
            .expect("live trace toggle");
        let click = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(toggle)]);
        frame(vec![click(toggle, true)]);
        let (paused, _, _, _) = frame(vec![click(toggle, false)]);
        assert!(!trace_drawn(&paused));
        let (collapsed, _, folded, _) = frame(vec![egui::Event::PointerMoved(anchor)]);
        assert!(folded.height() < expanded.height());
        assert!(!collapsed.iter().any(
            |shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "10k")
        ));
        for (index, factor) in [0.62, 2.5].into_iter().enumerate() {
            let (shapes, _, card, zoom) = frame(vec![egui::Event::Zoom(factor)]);
            let text = |label: &str| {
                shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == label => {
                            let bounds = Rect::from_min_size(text.pos, text.galley.size());
                            assert!(
                                shape.clip_rect.expand(1.0).contains_rect(bounds),
                                "clipped {label} at zoom {zoom}"
                            );
                            Some(bounds)
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| {
                        let rendered: Vec<_> = shapes
                            .iter()
                            .filter_map(|shape| match &shape.shape {
                                egui::Shape::Text(text) => Some(text.galley.text()),
                                _ => None,
                            })
                            .collect();
                        panic!("missing {label} at zoom {zoom}. Rendered {rendered:?}");
                    })
            };
            let low = text("400");
            let high = text("600");
            let remove = text(&crate::icons::CLOSE.to_string());
            assert!(card.contains_rect(low) && card.contains_rect(remove));
            assert!(low.right() < high.left() && high.right() < remove.left());
            assert!((high.center().y - remove.center().y).abs() < 4.0 * zoom);
            if index == 0 {
                let from = low.center();
                let to = from + Vec2::new(12.0, 0.0);
                frame(vec![egui::Event::PointerMoved(from)]);
                frame(vec![click(from, true)]);
                frame(vec![egui::Event::PointerMoved(to)]);
                let (_, changed, _, _) = frame(vec![click(to, false)]);
                let Some((owner, BandEdit::Change(0, Some(settings)))) = changed.spectrum_edit
                else {
                    panic!("the inline low-frequency field did not update");
                };
                assert_eq!(owner, node);
                assert!(settings["low_hz"].as_f64().unwrap() > 400.0);
                assert_eq!(settings["high_hz"].as_f64(), Some(600.0));
            }
            let center = remove.center();
            frame(vec![egui::Event::PointerMoved(center)]);
            frame(vec![click(center, true)]);
            let (_, release, _, _) = frame(vec![click(center, false)]);
            assert_eq!(
                release.spectrum_edit,
                Some((node, BandEdit::Change(0, None)))
            );
            frame(vec![egui::Event::PointerMoved(anchor)]);
        }
    }

    /// A wire dragged back from an input lands on an output, or, let go on the
    /// background, asks for a node to feed the input.
    #[test]
    fn a_wire_dragged_back_from_an_input_connects_or_asks_for_a_feeder() {
        let graph = GraphView {
            nodes: vec![
                node(1, vec![port("then", false, true)]).into(),
                node(2, vec![port("in", true, true)]).into(),
            ],
            wires: Vec::new(),
        };
        let mut canvas = Canvas::new();
        canvas.origin = Pos2::ZERO;
        canvas.zoom = 1.0;
        canvas.fit = false;
        canvas.size = Vec2::new(900.0, 600.0);
        canvas.places.insert(id(1), Pos2::new(0.0, 96.0));
        canvas.places.insert(id(2), Pos2::new(460.0, 96.0));
        // Where a port's dot is on its card, in graph units.
        let dot = |n: usize, name: &str, place: Pos2| {
            let view = &graph.nodes[n];
            let found = view.ports.iter().find(|port| port.id == name).unwrap();
            place + canvas.card(view).offset(found).unwrap()
        };
        // Just outside the card, where only the dot is.
        let input = dot(1, "in", Pos2::new(460.0, 96.0)) - Vec2::new(4.0, 0.0);
        let output = dot(0, "then", Pos2::new(0.0, 96.0)) - Vec2::new(4.0, 0.0);
        let context = egui::Context::default();
        let drag = |canvas: &mut Canvas, from: Pos2, to: Pos2| {
            let input = from;
            let mut response = CanvasResponse::default();
            let mut frame = |events| {
                let output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0))),
                        events,
                        ..Default::default()
                    },
                    |ui| response = canvas.show(ui, &graph, &[]),
                );
                output.drop_without_applying_deltas();
            };
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame(vec![egui::Event::PointerMoved(input)]);
            frame(vec![button(input, true)]);
            frame(vec![egui::Event::PointerMoved(
                input + Vec2::new(-20.0, 10.0),
            )]);
            frame(vec![egui::Event::PointerMoved(to)]);
            frame(vec![button(to, false)]);
            response
        };
        let forward = drag(
            &mut canvas,
            output + Vec2::new(8.0, 0.0),
            Pos2::new(300.0, 450.0),
        );
        // A wire from an output still asks for a node to take it.
        assert_eq!(
            forward.loose.map(|(from, ..)| from),
            Some((id(1), "then".to_owned()))
        );
        assert!(forward.loose_into.is_none());
        let onto_output = drag(&mut canvas, input, output);
        assert_eq!(
            onto_output.connect,
            Some(((id(1), "then".to_owned()), (id(2), "in".to_owned())))
        );
        let background = Pos2::new(300.0, 450.0);
        let loose = drag(&mut canvas, input, background);
        assert!(loose.connect.is_none() && loose.loose.is_none());
        let Some((to, _, screen)) = loose.loose_into else {
            panic!("letting go on the background asks for a feeder");
        };
        assert_eq!(to, (id(2), "in".to_owned()));
        assert_eq!(screen, background);
    }
    #[test]
    fn dragging_a_palette_item_previews_it_and_both_payloads_can_be_dropped() {
        let context = egui::Context::default();
        let graph = GraphView {
            nodes: vec![
                node(1, vec![port("then", false, true)]).into(),
                node(2, vec![port("in", true, true)]).into(),
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
    fn clicking_add_receiver_on_the_sender_uses_the_canvas_action() {
        let (host, _) = orange_host::Host::new(orange_host::HostOptions::default()).unwrap();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        let sender = host
            .add_node(&mut draft, "preset:signal", [50.0, 80.0])
            .unwrap();
        let graph = host.draft_view(&draft).unwrap();
        let mut canvas = Canvas::new();
        canvas.load(&graph);
        canvas.fit = false;
        canvas.origin = Pos2::ZERO;
        let ctx = egui::Context::default();
        let mut draw = |events| {
            let mut action = None;
            let output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| action = canvas.show(ui, &graph, &[]).add_signal_receiver,
            );
            let button = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == t("signal-add-receiver") => {
                    Some(text.pos + text.galley.size() / 2.0)
                }
                _ => None,
            });
            output.drop_without_applying_deltas();
            (action, button)
        };
        let (_, at) = draw(vec![]);
        let at = at.expect("sender shows an Add signal receiver button");
        let _ = draw(vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        let (action, _) = draw(vec![egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert_eq!(action, Some(sender));
    }

    #[test]
    fn wireless_links_are_drawn_only_while_an_endpoint_is_hovered() {
        let (host, _) = orange_host::Host::new(orange_host::HostOptions::default()).unwrap();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        let sender = host
            .add_node(&mut draft, "preset:signal", [50.0, 80.0])
            .unwrap();
        let graph = host.draft_view(&draft).unwrap();
        let receiver = graph
            .nodes
            .iter()
            .find(|node| node.id != sender)
            .unwrap()
            .id;
        let mut canvas = Canvas::new();
        canvas.load(&graph);
        canvas.fit = false;
        canvas.origin = Pos2::ZERO;
        let ctx = egui::Context::default();
        let (_, accent, _) = crate::palette::badge("core/emit");
        let mut frame = |pointer| {
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                    events: vec![egui::Event::PointerMoved(pointer)],
                    ..Default::default()
                },
                |ui| {
                    canvas.show(ui, &graph, &[]);
                },
            );
            let drawn = output.shapes.iter().any(|shape| {
                matches!(
                    &shape.shape,
                    egui::Shape::LineSegment { stroke, .. } if stroke.color == accent
                        && (stroke.width - 1.8).abs() < f32::EPSILON
                )
            });
            output.drop_without_applying_deltas();
            drawn
        };
        let position = |id| {
            let node = graph.nodes.iter().find(|node| node.id == id).unwrap();
            let [x, y] = node.position.unwrap();
            Pos2::new(x + 30.0, y + 25.0)
        };
        assert!(!frame(Pos2::new(30.0, 450.0)));
        assert!(frame(position(sender)));
        assert!(frame(position(receiver)));
        assert!(!frame(Pos2::new(30.0, 450.0)));
    }

    #[test]
    fn drawing_culls_offscreen_wires_but_keeps_crossings_and_tracks_moved_endpoints() {
        let mut graph = GraphView {
            nodes: (1..=4)
                .map(|n| node(n, vec![port("in", true, true), port("then", false, true)]).into())
                .collect(),
            wires: vec![wire(1, 2), wire(3, 4)],
        };
        for (node, position) in graph.nodes.iter_mut().zip([
            [-400.0, 30.0],
            [1000.0, 30.0],
            [10000.0, 10000.0],
            [11000.0, 10000.0],
        ]) {
            Arc::make_mut(node).position = Some(position);
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
fn snapped(point: Pos2, step: u16) -> Pos2 {
    let step = f32::from(step);
    Pos2::new(
        (point.x / step).round() * step,
        (point.y / step).round() * step,
    )
}

#[cfg(test)]
mod snap_tests {
    use super::*;

    #[test]
    fn a_point_snaps_to_the_nearest_grid_crossing() {
        assert_eq!(snapped(Pos2::new(13.0, 35.0), 24), Pos2::new(24.0, 24.0));
        assert_eq!(snapped(Pos2::new(-13.0, 11.0), 24), Pos2::new(-24.0, 0.0));
        assert!(Canvas::new().snap && !Canvas::new().scroll_pans);
    }

    #[test]
    fn grid_keeps_major_marks_at_distant_zoom_and_aligns_lines_to_pixels() {
        let ctx = egui::Context::default();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let area = Rect::from_min_size(Pos2::ZERO, Vec2::splat(110.0));
            let mut canvas = Canvas::new();
            let colors = Palette::of(ui);
            canvas.draw_grid(ui.painter(), area, &colors);
            let first = canvas.grid_cache.as_ref().unwrap().1.clone();
            canvas.draw_grid(ui.painter(), area, &colors);
            assert!(std::sync::Arc::ptr_eq(
                &first,
                &canvas.grid_cache.as_ref().unwrap().1
            ));
            canvas.origin.x += 1.0;
            canvas.draw_grid(ui.painter(), area, &colors);
            assert!(!std::sync::Arc::ptr_eq(
                &first,
                &canvas.grid_cache.as_ref().unwrap().1
            ));
            canvas.origin.x -= 1.0;
            let near = canvas.grid_mesh(area, 2.0, &colors);
            let major = near.vertices[0].color;
            assert!(near.vertices.iter().any(|vertex| vertex.color != major));

            canvas.zoom = 0.25;
            canvas.origin = Pos2::new(6.0, 6.0);
            let far = canvas.grid_mesh(area, 2.0, &colors);
            assert!(!far.vertices.is_empty());
            assert!(far.vertices.iter().all(|vertex| vertex.color == major));
            assert!((far.vertices[0].pos.x - 0.25).abs() < f32::EPSILON);

            canvas.grid_style = GridStyle::Lines;
            canvas.origin = Pos2::new(7.0, 7.0);
            let lines = canvas.grid_mesh(area, 2.0, &colors);
            assert!(lines.vertices.iter().all(|vertex| {
                let twice = vertex.pos * 2.0;
                twice.x.fract().abs() < f32::EPSILON && twice.y.fract().abs() < f32::EPSILON
            }));
        });
        output.drop_without_applying_deltas();
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

    #[test]
    #[ignore = "manual release rendering measurement"]
    fn profile_audio_canvas() {
        use super::{Canvas, Pos2, Rect, Vec2};
        use std::time::Instant;
        let (host, _) = orange_host::Host::with_services(
            orange_host::HostOptions::default(),
            vec![orange_audio::package()],
        )
        .unwrap();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        let mut charts = Vec::new();
        let mut meters = Vec::new();
        for x in [30.0, 540.0] {
            let chart = host
                .add_node(&mut draft, orange_audio::MULTIBAND_SPECTRUM, [x, 30.0])
                .unwrap();
            host.add_audio_band(&mut draft, chart, [80, 240]).unwrap();
            host.add_audio_band(&mut draft, chart, [400, 1200]).unwrap();
            charts.push(chart);
            meters.push(
                host.add_node(&mut draft, orange_audio::NOISE_LEVEL, [x, 410.0])
                    .unwrap(),
            );
        }
        let mut graph = host.draft_view(&draft).unwrap();
        for node in &mut graph.nodes {
            std::sync::Arc::make_mut(node)
                .ports
                .iter_mut()
                .filter(|port| port.id == "stream" && port.input)
                .for_each(|port| port.connected = true);
        }
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        crate::icons::install(&ctx);
        let size = Vec2::new(1100.0, 850.0);
        for workload in ["idle", "live", "collapsed", "offscreen"] {
            let mut canvas = Canvas::new();
            canvas.load(&graph);
            canvas.fit = false;
            canvas.size = size;
            if workload == "collapsed" {
                canvas.collapsed_spectra.extend(charts.iter().copied());
            }
            if workload == "offscreen" {
                canvas.origin = Pos2::new(4000.0, 4000.0);
            }
            if workload != "idle" {
                canvas.begin_run();
            }
            let mut times = Vec::new();
            for frame in 0..400 {
                if workload != "idle" && frame % 3 == 0 {
                    for chart in &charts {
                        let bins = canvas
                            .spectra
                            .entry(*chart)
                            .or_insert_with(|| vec![0.0; 96]);
                        for (bin, value) in (0..96).zip(bins) {
                            *value = -20.0 - f64::from((bin + frame) % 73);
                        }
                    }
                    for meter in &meters {
                        canvas
                            .noise_readings
                            .insert(*meter, -20.0 - f64::from(frame % 40));
                        use orange_ui_controls::audio_meter::ChannelReading;
                        let swing = f32::from(u16::try_from(frame % 40).unwrap());
                        let channel = |rms, peak| ChannelReading {
                            rms,
                            peak,
                            clipped: false,
                        };
                        canvas.noise_channels.insert(
                            *meter,
                            super::spectrum::ChannelLevels::Stereo([
                                channel(-18.0 - swing, -9.0 - swing),
                                channel(-22.0 - swing, -14.0 - swing),
                            ]),
                        );
                    }
                }
                let now = Instant::now();
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                        time: Some(f64::from(frame) / 60.0),
                        ..Default::default()
                    },
                    |ui| {
                        let _ = canvas.show(ui, &graph, &[]);
                    },
                );
                std::hint::black_box(
                    ctx.tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point),
                );
                let elapsed = now.elapsed().as_secs_f64() * 1000.0;
                output.drop_without_applying_deltas();
                if frame >= 40 {
                    times.push(elapsed);
                }
            }
            times.sort_by(f64::total_cmp);
            eprintln!(
                "{workload}: median {:.3} ms p95 {:.3} ms",
                times[times.len() / 2],
                times[times.len() * 95 / 100]
            );
        }
    }
}
