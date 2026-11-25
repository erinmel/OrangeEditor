//! The editor interface of OrangeEditor, as an egui library.
//!
//! It is made of parts an application takes as it needs:
//!
//! - [`Canvas`] draws one graph and nothing else. An application with its own
//!   lists and panels shows only this.
//! - [`palette()`] is the data of a node list, for an application that draws its own.
//! - [`Editor`] is the whole editor: a toolbar, the canvas, and panels for the node
//!   kinds, the packages, the problems and the log, each of which can be switched off
//!   ([`Panels`]).
//!
//! Everything reaches the product through `orange-host` alone: the palette is the
//! host's node list, a document is checked and run by the host, the canvas draws the
//! host's view of it, and the log is the host's events. Where nodes are placed is kept
//! in the document itself, so a layout travels with the file.
//!
//! Buttons call the methods below, which also let tests and other hosts drive the
//! editor without a window. The open document is a host [`Draft`]: edits change it in
//! place, and its problems are checked again only where an edit reached. Event polling
//! is nonblocking and consumes a finite batch per call.
// Product code reports failures as typed errors; each remaining panic is a documented
// internal invariant (10, "Panic policy").
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

pub mod bundle;
pub mod canvas;
mod card_sources;
pub mod controls;
mod declarations;
mod diagnostics;
#[cfg(test)]
mod edit_timing;
pub mod i18n;
pub mod icons;
#[cfg(test)]
mod interaction_regressions;
mod keymap;
mod menu;
mod node_ergonomics;
mod organise;
mod package_import;
mod port_values;
mod properties;
mod quick_add;
mod settings;
mod theme;
pub use theme::apply as apply_theme;
mod types_tab;

pub use properties::{COMMON_TYPES, port_label, set_at};
pub use quick_add::QuickChoice;

use crate::controls::{Dropdown as _, Surface};
use canvas::Canvas;
use i18n::{t, tf};
use orange_host::{
    Declarations, Draft, FlowEvent, FlowId, GraphView, Host, HostError, Names, NodeId, PackageKind,
    StopMode, lines,
};
use std::collections::{HashMap, HashSet, VecDeque};

/// Lets a window without the system's frame be resized from its edges and corners,
/// as a framed one is: the cursor shows the direction and a press hands the resize to
/// the operating system. Nothing while maximized or full screen. The bands sit above
/// every other layer, so the toolbar's drag region does not take the press.
fn resize_borders(ctx: &egui::Context) {
    use egui::{CursorIcon, ResizeDirection as To};
    let fixed = ctx.input(|input| {
        let viewport = input.viewport();
        viewport.maximized.unwrap_or(false) || viewport.fullscreen.unwrap_or(false)
    });
    if fixed {
        return;
    }
    const BAND: f32 = 5.0;
    const CORNER: f32 = 14.0;
    let screen = ctx.viewport_rect();
    let (l, r, t, b) = (screen.left(), screen.right(), screen.top(), screen.bottom());
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| {
        egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1))
    };
    let borders = [
        (
            To::North,
            CursorIcon::ResizeVertical,
            rect(l + CORNER, t, r - CORNER, t + BAND),
        ),
        (
            To::South,
            CursorIcon::ResizeVertical,
            rect(l + CORNER, b - BAND, r - CORNER, b),
        ),
        (
            To::West,
            CursorIcon::ResizeHorizontal,
            rect(l, t + CORNER, l + BAND, b - CORNER),
        ),
        (
            To::East,
            CursorIcon::ResizeHorizontal,
            rect(r - BAND, t + CORNER, r, b - CORNER),
        ),
        (
            To::NorthWest,
            CursorIcon::ResizeNwSe,
            rect(l, t, l + CORNER, t + BAND),
        ),
        (
            To::NorthWest,
            CursorIcon::ResizeNwSe,
            rect(l, t, l + BAND, t + CORNER),
        ),
        (
            To::NorthEast,
            CursorIcon::ResizeNeSw,
            rect(r - CORNER, t, r, t + BAND),
        ),
        (
            To::NorthEast,
            CursorIcon::ResizeNeSw,
            rect(r - BAND, t, r, t + CORNER),
        ),
        (
            To::SouthWest,
            CursorIcon::ResizeNeSw,
            rect(l, b - BAND, l + CORNER, b),
        ),
        (
            To::SouthWest,
            CursorIcon::ResizeNeSw,
            rect(l, b - CORNER, l + BAND, b),
        ),
        (
            To::SouthEast,
            CursorIcon::ResizeNwSe,
            rect(r - CORNER, b - BAND, r, b),
        ),
        (
            To::SouthEast,
            CursorIcon::ResizeNwSe,
            rect(r - BAND, b - CORNER, r, b),
        ),
    ];
    for (index, (direction, cursor, band)) in borders.into_iter().enumerate() {
        let _ = egui::Area::new(egui::Id::unique("orange-resize").with(index))
            .order(egui::Order::Foreground)
            .fixed_pos(band.min)
            .interactable(true)
            .show(ctx, |ui| {
                let (_, response) = ui.allocate_exact_size(band.size(), egui::Sense::drag());
                if response.hovered() || response.dragged() {
                    ctx.set_cursor_icon(cursor);
                }
                if response.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
                }
            });
    }
}

/// What a card shows of a value on a port, short and whole; nothing for a stream, which
/// is a connection rather than a value (22 §3.1).
fn presented_value(
    enums: &canvas::Enums,
    ty: Option<&str>,
    value: &orange_host::Value,
) -> Option<(String, String)> {
    if let orange_host::Value::Record(record) = value
        && record
            .ty
            .as_str()
            .split('@')
            .next()
            .is_some_and(|name| name.ends_with("/stream"))
    {
        return None;
    }
    let short = match value {
        orange_host::Value::Record(record) if record.ty.as_str() == "org.orange.audio/format@1" => {
            match record.fields.as_ref() {
                [
                    Some(orange_host::Value::U64(rate)),
                    Some(orange_host::Value::U64(channels)),
                ] => tf(
                    "audio-stream-info-value",
                    &[
                        ("rate", rate.to_string().into()),
                        ("channels", channels.to_string().into()),
                    ],
                ),
                _ => canvas::value_text(enums, ty, value),
            }
        }
        orange_host::Value::F64(number) => decimal(*number),
        orange_host::Value::F32(number) => decimal(f64::from(*number)),
        _ => canvas::value_text(enums, ty, value),
    };
    Some((short, orange_host::inspect(value)))
}

/// A decimal as a card shows it: at most two places, without trailing zeros.
fn decimal(number: f64) -> String {
    if !number.is_finite() {
        return number.to_string();
    }
    let text = format!("{number:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

/// Most lines the log keeps; older ones are dropped.
pub const MAX_LOG_LINES: usize = 500;

/// Maximum host events consumed by one poll, including events for other flows.
/// Poll again on a later frame while events keep arriving.
pub const MAX_EVENTS_PER_POLL: usize = 256;

mod palette;
pub use menu::{Command, MenuEntry, MenuTarget};
pub use palette::{PaletteItem, palette, palette_in};

/// A type as a person reads it: `core/string@1` is "text". Other types keep their name
/// without the version.
/// A conversion's name from the type it takes to the type it gives: "Int to Text".
/// Two types that read alike, such as i32 and i64, are named exactly.
pub(crate) fn conversion_name(from: &str, to: &str) -> String {
    let (mut from_name, mut to_name) = (type_name(from), type_name(to));
    if from_name == to_name {
        let exact = |ty: &str| {
            ty.trim_start_matches("core/")
                .split('@')
                .next()
                .unwrap_or(ty)
                .to_owned()
        };
        (from_name, to_name) = (exact(from), exact(to));
    }
    tf(
        "conversion-title",
        &[("from", from_name.into()), ("to", to_name.into())],
    )
}

pub fn type_name(ty: &str) -> String {
    let named = types_tab::with_local_names(ty);
    match named.as_str() {
        "core/string@1" => t("type-text").to_owned(),
        "core/bool@1" => t("type-bool").to_owned(),
        "core/i64@1" | "core/u64@1" | "core/i32@1" | "core/u32@1" => t("type-int").to_owned(),
        "core/f64@1" | "core/f32@1" => t("type-float").to_owned(),
        "core/bytes@1" => t("type-bytes").to_owned(),
        "core/duration@1" => t("type-duration").to_owned(),
        "core/timestamp@1" => t("type-timestamp").to_owned(),
        orange_host::RGBA_TYPE => t("type-rgba").to_owned(),
        // A package's own type reads as its package and name:
        // `org.orange.audio/stream@1` is "Audio stream".
        other if !other.starts_with("core/") && !other.contains(['<', '$']) => {
            package_type_name(other).unwrap_or_else(|| other.replace("@1", ""))
        }
        // Core types lose their namespace everywhere in the expression.
        other => other.replace("core/", "").replace("@1", ""),
    }
}

/// `namespace/name@major` as "Last-namespace-part name": `org.orange.audio/stream@1`
/// is "Audio stream", `org.example.geo/bounding-box@1` is "Geo bounding box".
fn package_type_name(ty: &str) -> Option<String> {
    let (namespace, rest) = ty.split_once('/')?;
    let name = rest.split('@').next()?;
    let package = namespace.rsplit('.').next()?;
    let mut text = format!("{package} {}", name.replace('-', " "));
    if let Some(first) = text.get(..1) {
        text.replace_range(..1, &first.to_uppercase());
    }
    Some(text)
}

/// A type exactly, for a tooltip beside its short name: `core/i64@1` is `i64`.
pub fn exact_type(ty: &str) -> String {
    ty.replace("core/", "").replace("@1", "")
}

/// How much a message a graph logs matters.
pub use diagnostics::NoteLevel;
use diagnostics::{LogLine, ProblemLevel, ProblemLocation};

/// A wire that needs a conversion chosen: its two ends, the conversions that join
/// them, and where the list is shown.
#[derive(Debug, Clone)]
struct ConversionChoice {
    from: (NodeId, String),
    to: (NodeId, String),
    choices: Vec<orange_host::Conversion>,
    at: Option<egui::Pos2>,
}

/// The left panel's tabs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Tab {
    #[default]
    Nodes,
    Variables,
    Connections,
    Types,
}

impl Tab {
    const ALL: [(Self, &str); 4] = [
        (Self::Nodes, "nodes"),
        (Self::Variables, "variables"),
        (Self::Connections, "connections"),
        (Self::Types, "types"),
    ];

    /// The name preferences keep the tab by.
    fn key(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(tab, _)| *tab == self)
            .map_or("nodes", |(_, key)| key)
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(_, known)| *known == key)
            .map(|(tab, _)| *tab)
    }
}

struct ObservedVariable {
    shown: String,
    exact_text: Option<orange_host::Value>,
}

impl std::fmt::Debug for ObservedVariable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObservedVariable").finish_non_exhaustive()
    }
}

/// The document being edited.
#[derive(Debug)]
struct Document {
    path: String,
    /// A new graph that has no file yet: saving it asks where.
    untitled: bool,
    /// The document as it is being edited, with its undo history.
    draft: Draft,
    names: Names,
    /// What the canvas draws.
    graph: GraphView,
    node_indices: HashMap<NodeId, usize>,
    /// Its variables and connections.
    declared: Declarations,
    /// The layout changed since the document was opened or saved.
    unsaved: bool,
    /// What checking found, one line each; empty when the document is valid.
    problems: Vec<String>,
    /// The top-level node each problem is about, in the same order.
    problem_locations: Vec<ProblemLocation>,
    run_blocked: bool,
    run_omitted: Vec<NodeId>,
    /// The flow running this document, with the entries a person can fire.
    flow: Option<(FlowId, Vec<NodeId>)>,
    /// The latest value of each variable a run wrote, by variable ID, as text.
    values: std::collections::BTreeMap<String, ObservedVariable>,
}

impl Document {
    fn replace_graph(&mut self, graph: GraphView) -> GraphView {
        self.node_indices.clear();
        self.node_indices.extend(
            graph
                .nodes
                .iter()
                .enumerate()
                .map(|(index, node)| (node.id, index)),
        );
        std::mem::replace(&mut self.graph, graph)
    }

    fn node_view(&self, id: NodeId) -> Option<&orange_host::NodeView> {
        let index = *self.node_indices.get(&id)?;
        self.graph.nodes.get(index).map(AsRef::as_ref)
    }

    /// The runnable copy omits draft cards while keeping them in the editor.
    fn runtime_text(&self, host: &Host) -> Result<String, HostError> {
        let text = self.draft.text()?;
        if self.run_omitted.is_empty() {
            return Ok(text);
        }
        let mut runnable = host.draft(&text)?;
        host.remove_nodes(&mut runnable, &self.run_omitted)?;
        runnable.text()
    }
}

pub use orange_host::MAX_UNDO;

/// Which panels the editor shows around the canvas. The canvas and the toolbar are
/// always there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Panels {
    /// The node kinds, on the left.
    pub nodes: bool,
    /// The packages, on the right.
    pub packages: bool,
    /// Problems and the log, at the bottom.
    pub log: bool,
    /// The values of the selected node, on the right.
    pub properties: bool,
}

impl Default for Panels {
    fn default() -> Self {
        Self {
            nodes: true,
            properties: true,
            packages: false,
            log: false,
        }
    }
}

/// A file action the editor leaves to the application, which owns its file dialogs
/// (or has none: an application that keeps graphs elsewhere opens them itself).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileRequest {
    /// Start a new graph.
    New,
    /// Choose a graph file to open.
    Open,
    /// Choose where to save the open graph.
    SaveAs,
    /// Choose an expansion package archive to install.
    ImportPackage,
    /// Choose where to write a validated expansion package archive.
    ExportPackage { name: String, archive: Vec<u8> },
    /// Choose where to write a graph shared with its packages
    /// ([`bundle::BUNDLE_EXTENSION`]).
    ExportBundle { name: String, bundle: Vec<u8> },
    /// Choose a shared graph to open and hand to [`Editor::review_bundle`].
    ImportBundle,
    /// Show the native plugin libraries and the full access each was given.
    NativePlugins,
    /// Choose the root directory for the open folder connection form.
    ChooseConnectionFolder { current: String },
    /// Choose a file or folder for a node's text input that holds a path, such as the
    /// sound file of Play sound, and hand it to [`Editor::choose_path`].
    ChoosePath {
        node: NodeId,
        port: String,
        /// A folder instead of a file.
        folder: bool,
        /// A file the node writes: ask where to save it rather than for one that exists.
        save: bool,
        /// File name extensions to offer, without the dot; empty offers every file.
        extensions: Vec<String>,
        /// The path the input holds now, to start the dialog from.
        current: String,
    },
}

/// A connection's live reading while its form shows it, such as an input's level.
pub(crate) struct Probing {
    /// The adapter and the device settings it was started with; a change to either
    /// starts it again.
    pub(crate) adapter: String,
    pub(crate) settings: String,
    pub(crate) probe: Result<Box<dyn orange_host::Probe>, String>,
}
impl std::fmt::Debug for Probing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Probing")
            .field("adapter", &self.adapter)
            .finish_non_exhaustive()
    }
}

/// The state of the editor between frames.
#[derive(Debug, Default)]
pub struct Editor {
    value_form: Option<port_values::ValueForm>,
    function_export: Option<organise::FunctionExport>,
    search: String,
    palette_highlighted: usize,
    log_events: bool,
    /// Whether successful steps and live values are written to the Log as they happen;
    /// off, the Log has the run's lifecycle, warnings, faults and the graph's own
    /// messages, and nothing is formatted for the rest.
    live_trace: bool,
    /// The last message a graph logged and how many times in a row, so repeats share
    /// one line.
    last_note: bool,
    log_filter: Option<NoteLevel>,
    log_search: String,
    problems_search: String,
    problems_filter: Option<ProblemLevel>,
    /// Problem row indices selected for copy or bulk inspection.
    selected_problems: std::collections::BTreeSet<usize>,
    declaration_search: [String; 3],
    node_search_previews: bool,
    pending_package: Option<package_import::PackageImport>,
    pending_bundle: Option<bundle::BundleImport>,
    pending_package_removal: Option<orange_host::PackageKey>,
    packages_tab: bool,
    /// What the Packages tab is filtered by.
    package_search: String,
    /// A wire let go between two kinds of value, waiting for its conversion.
    conversion_choice: Option<ConversionChoice>,
    pending_for_each: Option<node_ergonomics::ForEachConfirmation>,
    build_map_rows: Option<node_ergonomics::BuildMapRows>,
    palette: palette::PaletteCache,
    previews: palette::PreviewCache,
    settings_open: bool,
    settings_search: String,
    settings_tab: settings::Section,
    preferred_scale: Option<f32>,
    preferred_theme: Option<egui::ThemePreference>,
    scale_applied: bool,
    shortcut_edit: Option<keymap::ShortcutEdit>,
    node_config_menu: Option<(NodeId, egui::Pos2)>,
    inspected_port: Option<(NodeId, String, bool)>,
    inspected_variable: Option<String>,
    document: Option<Document>,
    canvas: Canvas,
    log: VecDeque<LogLine>,
    /// The node whose values the properties panel is editing, and what is typed in
    /// each of its fields.
    inspected: Option<NodeId>,
    typed: std::collections::BTreeMap<String, String>,
    /// The left panel shows the document's declarations instead of the node kinds.
    tab: Tab,
    preferred_tab: Tab,
    forms: declarations::Forms,
    /// The setting choices of each connection adapter (its devices, say), listed when
    /// first shown and again when a person asks: listing may ask the operating system.
    adapter_choices: std::collections::BTreeMap<String, Vec<orange_host::SettingChoices>>,
    /// The panels shown; a host application may set them, and the toolbar switches them.
    pub panels: Panels,
    /// Whether the application shows file dialogs: the toolbar then has New, Open and
    /// Save as, which it hands over through [`Editor::take_request`].
    pub file_dialogs: bool,
    /// Whether the application loads native plugin libraries: the Packages panel then
    /// offers [`FileRequest::NativePlugins`].
    pub native_plugins: bool,
    /// Draw native window commands and a drag region in the toolbar. Opt in only
    /// when the embedding window disables its own decorations.
    pub window_toolbar: bool,
    request: Option<FileRequest>,
    /// The search to add a node at the pointer, while it is open.
    quick: Option<quick_add::QuickAdd>,
    /// Run was pressed this frame: it starts once the fields have handed over what was
    /// being typed, at the end of the frame.
    run_asked: bool,
    /// The live reading of the connection being set up, while a person asked to see it.
    pub(crate) probing: Option<Probing>,
    /// A function being made of the selected nodes, while its preview is open.
    extracting: Option<organise::Extracting>,
    renaming_node: Option<organise::NodeRename>,
    /// The language the node search also matches in, as a BCP 47 tag; empty is English.
    language: String,
    refresh_problem_text: bool,
    /// The right-click menu, while it is open.
    menu: Option<menu::Menu>,
    /// What this editor copied last, for Paste from the menu.
    clip: Option<String>,
    /// Copied text for the system clipboard, handed over on the next frame.
    clip_out: Option<String>,
    /// Paste from the menu with nothing copied here: the system clipboard is asked.
    paste_asked: bool,
}

/// Places a node of `definition` at `at`: a conversion, a loop with its end, or one node.
/// Returns the node and, for a loop, its end.
fn place_node(
    host: &Host,
    draft: &mut Draft,
    definition: &str,
    at: [f32; 2],
) -> Result<(NodeId, Option<NodeId>), HostError> {
    // A Bridge In bound to an existing Bridge Out in one step.
    if let Some(out) = definition.strip_prefix(palette::BIND_BRIDGE_IN) {
        let out: NodeId = out
            .parse()
            .map_err(|_| HostError::Document(format!("no Bridge Out `{out}`")))?;
        let channel = draft
            .document()
            .node(out)
            .and_then(|node| {
                node.config
                    .get("channel")
                    .and_then(|c| c.as_str())
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        let node = host.add_node(draft, palette::BRIDGE_IN, at)?;
        let _ = host.set_config(draft, node, &serde_json::json!({ "channel": channel }))?;
        let _ = host.connect(draft, (out, "result"), (node, "value"))?;
        return Ok((node, None));
    }
    // A new Bridge Out gets a channel name no other one has.
    if definition == palette::BRIDGE_OUT {
        let taken: Vec<String> = draft
            .document()
            .root()
            .nodes
            .values()
            .filter(|node| node.definition.id.as_str() == palette::BRIDGE_OUT)
            .filter_map(|node| {
                node.config
                    .get("channel")
                    .and_then(|c| c.as_str())
                    .map(str::to_owned)
            })
            .collect();
        let channel = (1..)
            .map(|n| format!("Bridge {n}"))
            .find(|name| !taken.contains(name))
            .unwrap_or_default();
        let node = host.add_node(draft, definition, at)?;
        let _ = host.set_config(draft, node, &serde_json::json!({ "channel": channel }))?;
        return Ok((node, None));
    }
    // A Listen bound to an existing channel in one step.
    if let Some(event) = definition.strip_prefix(palette::BIND_LISTEN) {
        let node = host.add_node(draft, "core/on-event", at)?;
        let _ = host.set_config(draft, node, &serde_json::json!({ "event": event }))?;
        return Ok((node, None));
    }
    if definition.starts_with(palette::CONVERSION) {
        return match palette::conversion_named(host, definition) {
            Some(conversion) => host
                .add_conversion(draft, &conversion, at)
                .map(|node| (node, None)),
            None => Err(HostError::Document(format!("no conversion `{definition}`"))),
        };
    }
    let end = [at[0] + canvas::NODE_WIDTH + 120.0, at[1]];
    host.place_loop(draft, definition, at, end)
        .and_then(|pair| match pair {
            Some([begin, end]) => Ok((begin, Some(end))),
            None => host
                .add_node(draft, definition, at)
                .map(|node| (node, None)),
        })
}

fn search_field(ui: &mut egui::Ui, id: &str, query: &mut String, hint: &str) {
    let _ = controls::search(ui, id, query, hint);
}

fn contains_query(query: &str, fields: &[&str]) -> bool {
    query.is_empty()
        || query.split_whitespace().all(|word| {
            fields
                .iter()
                .any(|field| field.to_lowercase().contains(word))
        })
}

impl Editor {
    pub fn new() -> Self {
        Self::default()
    }

    /// The language the editor shows its text in and the node search matches in,
    /// besides English, such as `es`.
    pub fn language(&self) -> &str {
        if self.language.is_empty() {
            orange_host::SOURCE_LANGUAGE
        } else {
            &self.language
        }
    }

    /// Shows the editor in `language` (a BCP 47 tag) and makes the node search match in
    /// it and English. Nothing about the graph changes with it.
    pub fn set_language(&mut self, language: &str) {
        let language = language.trim();
        if self.language != language {
            language.clone_into(&mut self.language);
            self.previews.invalidate();
            self.refresh_problem_text = true;
        }
        i18n::set_language(self.language());
    }

    /// A port of the open graph's top level, as the canvas draws it.
    fn port_view(&self, node: NodeId, port: &str, input: bool) -> Option<&orange_host::PortView> {
        self.graph()?
            .nodes
            .iter()
            .find(|shown| shown.id == node)?
            .ports
            .iter()
            .find(|shown| shown.id == port && shown.input == input)
    }

    /// Shows a document: `text` is the saved graph and `path` what it is called. It is
    /// checked at once; a flow of the previous document is closed.
    /// A text that is not a document opens nothing, and the log says why.
    pub fn open(&mut self, host: &mut Host, path: impl Into<String>, text: String) {
        self.pending_for_each = None;
        self.build_map_rows = None;
        self.node_config_menu = None;
        self.renaming_node = None;
        self.inspected_port = None;
        self.inspected_variable = None;
        self.forms = declarations::Forms::default();
        self.probing = None;
        self.tab = self.preferred_tab;
        self.previews.invalidate();
        self.stop(host);
        let path = path.into();
        let draft = match host.draft(&text) {
            Ok(draft) => draft,
            Err(error) => {
                self.document = None;
                self.say(tf(
                    "not-opened",
                    &[
                        ("path", path.to_string().into()),
                        ("error", error.to_string().into()),
                    ],
                ));
                return;
            }
        };
        let orange_host::AuthoringView {
            graph,
            failure_routes,
            port_details,
        } = host
            .draft_authoring_view_in(&draft, self.language())
            .unwrap_or_default();
        self.value_form = None;
        self.canvas.set_failure_routes(failure_routes);
        self.canvas.set_port_details(port_details);
        self.canvas.load(&graph);
        let declared = host.declarations(&draft);
        self.canvas.enums = canvas::enums_of(host, &draft, &graph, &declared);
        self.canvas.channels = canvas::channels_of(&declared);
        self.canvas.set_groups(draft.groups());
        self.canvas.waypoints = draft.wire_waypoints().collect();
        self.document = Some(Document {
            path,
            untitled: false,
            declared: {
                let declared = host.declarations(&draft);
                types_tab::remember(&declared.types);
                declared
            },
            names: Names::of_document(host, draft.document()).without_ids(),
            draft,
            node_indices: graph
                .nodes
                .iter()
                .enumerate()
                .map(|(index, node)| (node.id, index))
                .collect(),
            graph,
            unsaved: false,
            problems: Vec::new(),
            problem_locations: Vec::new(),
            run_blocked: false,
            run_omitted: Vec::new(),
            flow: None,
            values: std::collections::BTreeMap::new(),
        });
        self.check(host);
        self.preview_values(host);
    }

    /// What the canvas shows of the open document.
    pub fn graph(&self) -> Option<&GraphView> {
        self.document.as_ref().map(|document| &document.graph)
    }

    /// The canvas, for an application that frames or moves it itself.
    pub fn canvas(&mut self) -> &mut Canvas {
        &mut self.canvas
    }

    /// The text of the open document, with its layout as the canvas has it.
    pub fn text(&mut self) -> Option<String> {
        self.keep_layout();
        self.document
            .as_ref()
            .and_then(|document| document.draft.text().ok())
    }

    /// Whether the layout changed since the document was opened or saved.
    pub fn is_unsaved(&self) -> bool {
        self.document
            .as_ref()
            .is_some_and(|document| document.unsaved)
    }

    /// Writes where the canvas has the nodes into the document's layout, as one edit
    /// that can be undone. Layout is not part of what a graph means, so a running flow
    /// is not touched.
    pub fn keep_layout(&mut self) {
        let Some(document) = &mut self.document else {
            return;
        };
        if document.draft.place(&self.canvas.places()).unwrap_or(false) {
            document.unsaved = true;
        }
    }

    /// After the draft changed: the canvas, the problems and a running flow follow.
    fn refresh(&mut self, host: &mut Host) {
        self.previews.invalidate();
        if let Some(quick) = &mut self.quick {
            quick.invalidate();
        }
        let locale = self.language().to_owned();
        let Some(document) = &mut self.document else {
            return;
        };
        document.names = Names::of_document(host, document.draft.document()).without_ids();
        let orange_host::AuthoringView {
            graph,
            failure_routes,
            port_details,
        } = host
            .draft_authoring_view_in(&document.draft, &locale)
            .unwrap_or_default();
        let before = document.replace_graph(graph);
        // Only what the edit touched, and what follows it, can preview differently.
        let touched = touched_by_edit(&before, &document.graph);
        document.declared = host.declarations(&document.draft);
        types_tab::remember(&document.declared.types);
        document.unsaved = true;
        self.canvas.set_failure_routes(failure_routes);
        self.canvas.set_port_details(port_details);
        self.canvas.update(&document.graph);
        self.canvas.enums =
            canvas::enums_of(host, &document.draft, &document.graph, &document.declared);
        self.canvas.channels = canvas::channels_of(&document.declared);
        self.canvas.set_groups(document.draft.groups());
        self.canvas.waypoints = document.draft.wire_waypoints().collect();
        // The fields of the properties panel are read again from the new document.
        self.inspected = None;
        self.check(host);
        let mut said = None;
        if let Some(document) = &mut self.document
            && let Some(flow) = document.flow.as_ref().map(|(flow, _)| *flow)
        {
            match document
                .runtime_text(host)
                .and_then(|text| host.update(flow, &text))
            {
                Ok(updated) => {
                    if updated.changed {
                        self.canvas.set_manual_values(host.info(flow));
                    }
                    let entries = host
                        .info(flow)
                        .map(|info| info.manual_entries.iter().map(|(node, _)| *node).collect())
                        .unwrap_or_default();
                    document.flow = Some((flow, entries));
                }
                // The flow keeps running the graph it had.
                Err(error) => {
                    said = Some(tf(
                        "edit-not-running",
                        &[
                            ("flow", flow.to_string().into()),
                            ("error", error.to_string().into()),
                        ],
                    ))
                }
            }
        }
        if let Some(line) = said {
            self.say(line);
        }
        self.preview_scope(host, touched.as_ref());
    }

    /// Follows up an edit of the draft. A refused edit says why in the log and changed
    /// nothing. Returns whether the document changed.
    fn edited(&mut self, host: &mut Host, result: Result<bool, HostError>) -> bool {
        let Some(document) = &mut self.document else {
            return false;
        };
        match result {
            Ok(false) => false,
            Ok(true) => {
                self.refresh(host);
                true
            }
            Err(error) => {
                for line in Self::problems_of(&error, &document.names) {
                    self.say(tf("not-done", &[("why", line.to_string().into())]));
                }
                false
            }
        }
    }

    fn edited_value<T>(&mut self, host: &mut Host, result: Result<T, HostError>) -> Option<T> {
        match result {
            Ok(value) => {
                self.refresh(host);
                Some(value)
            }
            Err(error) => {
                let _ = self.edited(host, Err(error));
                None
            }
        }
    }

    /// The draft an edit changes, with the canvas's layout written into it first.
    fn editing(&mut self) -> Option<&mut Draft> {
        self.keep_layout();
        self.document.as_mut().map(|document| &mut document.draft)
    }

    /// Adds a node of kind `definition` in the middle of the canvas and selects it.
    pub fn add_node(&mut self, host: &mut Host, definition: &str) -> Option<NodeId> {
        let [x, y] = self.canvas.center();
        // Not exactly on top of a node added just before.
        #[expect(clippy::cast_precision_loss, reason = "a document has few nodes")]
        let step = (self.graph().map_or(0, |graph| graph.nodes.len()) % 8) as f32 * 24.0;
        let at = [x - canvas::NODE_WIDTH / 2.0 + step, y - 40.0 + step];
        self.add_node_at(host, definition, at)
    }

    /// Binds a Bridge In to a Bridge Out: the edge that carries the value, and the
    /// channel name the Bridge In keeps should its Bridge Out go.
    pub(crate) fn bind_bridge(&mut self, host: &mut Host, reading: NodeId, out: NodeId) -> bool {
        let Some(channel) = self.graph().and_then(|graph| {
            graph
                .nodes
                .iter()
                .find(|node| node.id == out)
                .and_then(|node| palette::bridge_channel(node).map(str::to_owned))
        }) else {
            return false;
        };
        let Some(draft) = self.editing() else {
            return false;
        };
        let result = host
            .connect(draft, (out, "result"), (reading, "value"))
            .and_then(|_| {
                host.set_config(draft, reading, &serde_json::json!({ "channel": channel }))
            });
        self.edited(host, result)
    }

    /// The graph's Bridge Outs (node, channel name).
    pub(crate) fn bridge_outs(&self) -> Vec<(NodeId, String)> {
        self.graph()
            .map(|graph| {
                graph
                    .nodes
                    .iter()
                    .filter(|node| node.definition == palette::BRIDGE_OUT)
                    .map(|node| {
                        (
                            node.id,
                            palette::bridge_channel(node).unwrap_or_default().to_owned(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The channels (event ID, name) that a Dispatch in the open graph sends on.
    pub(crate) fn dispatch_channels(&self) -> Vec<(String, String)> {
        let (Some(declared), Some(graph)) = (self.declarations(), self.graph()) else {
            return Vec::new();
        };
        declared
            .events
            .iter()
            .filter(|event| {
                graph.nodes.iter().any(|node| {
                    node.definition == "core/emit"
                        && node.settings["event"].as_str() == Some(event.id.to_string().as_str())
                })
            })
            .map(|event| (event.id.to_string(), event.name.clone()))
            .collect()
    }

    /// Adds a node of kind `definition` with its top left corner at `at` (graph units)
    /// and selects it.
    pub fn add_node_at(
        &mut self,
        host: &mut Host,
        definition: &str,
        at: [f32; 2],
    ) -> Option<NodeId> {
        let draft = self.editing()?;
        let multi = matches!(
            definition,
            "preset:signal" | "preset:wireless-pulse" | "preset:wireless-value" | "preset:watchdog"
        );
        let before = multi.then(|| {
            draft
                .document()
                .root()
                .nodes
                .keys()
                .copied()
                .collect::<HashSet<_>>()
        });
        let result = place_node(host, draft, definition, at);
        let created = before.map(|before| {
            draft
                .document()
                .root()
                .nodes
                .keys()
                .filter(|id| !before.contains(id))
                .copied()
                .collect::<Vec<_>>()
        });
        let placed = self.placed(host, result)?;
        if let Some(created) = created {
            self.canvas.select_many(&created);
        }
        Some(placed)
    }

    /// Selects what a placement made, or reports why it was refused, leaving the
    /// selection as it was.
    fn placed(
        &mut self,
        host: &mut Host,
        result: Result<(NodeId, Option<NodeId>), HostError>,
    ) -> Option<NodeId> {
        match result {
            Ok((node, companion)) => {
                let _ = self.edited(host, Ok(true));
                if let Some(companion) = companion {
                    self.canvas.select_many(&[node, companion]);
                } else {
                    self.canvas.select(Some(node));
                }
                Some(node)
            }
            Err(error) => {
                let _ = self.edited(host, Err(error));
                self.panels.log = true;
                self.log_events = true;
                None
            }
        }
    }

    /// Removes a node with its wires.
    pub fn remove_node(&mut self, host: &mut Host, node: NodeId) -> bool {
        self.remove_nodes(host, &[node])
    }

    /// Removes nodes with their wires, as one edit.
    pub fn remove_nodes(&mut self, host: &mut Host, nodes: &[NodeId]) -> bool {
        // A deleted Bridge Out leaves its Bridge Ins Unbound, naming its channel as it
        // is now, so they can be rebound or the emitter recreated.
        let keep: Vec<(NodeId, String)> = self
            .graph()
            .map(|graph| {
                graph
                    .wires
                    .iter()
                    .filter(|wire| {
                        nodes.contains(&wire.from.0)
                            && wire.from.1 == "result"
                            && wire.to.1 == "value"
                    })
                    .filter_map(|wire| {
                        let out = graph.nodes.iter().find(|node| node.id == wire.from.0)?;
                        let reading = graph.nodes.iter().find(|node| node.id == wire.to.0)?;
                        (out.definition == palette::BRIDGE_OUT
                            && reading.definition == palette::BRIDGE_IN
                            && !nodes.contains(&reading.id))
                        .then(|| {
                            (
                                reading.id,
                                palette::bridge_channel(out).unwrap_or_default().to_owned(),
                            )
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let Some(draft) = self.editing() else {
            return false;
        };
        for (reading, channel) in keep {
            let _ = host.set_config(draft, reading, &serde_json::json!({ "channel": channel }));
        }
        let result = host.remove_nodes(draft, nodes);
        self.edited(host, result)
    }

    /// The selected nodes as clipboard text, with the wires between them and the
    /// declarations they name. `None` when nothing is selected.
    pub fn copy(&mut self, host: &Host) -> Option<String> {
        let selection = self.canvas.selection();
        if selection.is_empty() {
            return None;
        }
        let draft = self.editing()?;
        match host.copy(draft, &selection) {
            Ok(clip) => Some(clip),
            Err(error) => {
                self.say(tf("not-copied", &[("error", error.to_string().into())]));
                None
            }
        }
    }

    /// Pastes clipboard text from [`Editor::copy`] with its top left node at `at`, and
    /// selects what was pasted. Text that is not copied nodes changes nothing.
    pub fn paste(&mut self, host: &mut Host, clip: &str, at: [f32; 2]) -> Vec<NodeId> {
        let Some(draft) = self.editing() else {
            return Vec::new();
        };
        match host.paste(draft, clip, at) {
            Ok(fresh) => {
                let _ = self.edited(host, Ok(!fresh.is_empty()));
                self.canvas.select_many(&fresh);
                fresh
            }
            Err(error) => {
                let _ = self.edited(host, Err(error));
                Vec::new()
            }
        }
    }

    /// Wires an output to an input. Two kinds of value are joined through a
    /// conversion: the one that loses nothing goes in by itself, otherwise the choices
    /// are offered where the wire was let go. A wire the language does not allow is
    /// refused with the reason in the log.
    pub fn connect(&mut self, host: &mut Host, from: (NodeId, &str), to: (NodeId, &str)) -> bool {
        let Some(draft) = self.editing() else {
            return false;
        };
        let result = host.connect_through(draft, from, to, None);
        match result {
            Ok(orange_host::Connected::Choose(choices)) => {
                self.conversion_choice = Some(ConversionChoice {
                    from: (from.0, from.1.to_owned()),
                    to: (to.0, to.1.to_owned()),
                    choices,
                    at: None,
                });
                false
            }
            other => self.edited(host, other.map(|_| true)),
        }
    }

    /// Connects the wire waiting for a conversion through the one chosen.
    fn connect_chosen(&mut self, host: &mut Host, choice: &orange_host::Conversion) -> bool {
        let Some(waiting) = self.conversion_choice.take() else {
            return false;
        };
        let Some(draft) = self.editing() else {
            return false;
        };
        let result = host.connect_through(
            draft,
            (waiting.from.0, &waiting.from.1),
            (waiting.to.0, &waiting.to.1),
            Some(choice),
        );
        self.edited(host, result.map(|_| true))
    }

    /// The list of conversions for a wire that needs one, beside where it was let go.
    fn conversion_menu(&mut self, ctx: &egui::Context, host: &mut Host) {
        let Some(waiting) = self.conversion_choice.as_mut() else {
            return;
        };
        let at = *waiting
            .at
            .get_or_insert_with(|| ctx.pointer_latest_pos().unwrap_or_default());
        let choices = &waiting.choices;
        let mut chosen = None;
        let mut close = false;
        let shown = orange_ui_controls::anchored_popup(
            ctx,
            egui::Id::unique("orange-conversion-choice"),
            at,
            312.0,
            |ui| {
                ui.strong(t("choose-conversion"));
                ui.weak(t("choose-conversion-hint"));
                ui.add_space(orange_ui_controls::SPACE_SMALL);
                egui::ScrollArea::vertical()
                    .max_height((ctx.content_rect().height() * 0.45).min(320.0))
                    .show(ui, |ui| {
                        for choice in choices {
                            let button = egui::Button::new(palette::conversion_label(choice))
                                .min_size(egui::vec2(ui.available_width(), 0.0));
                            if controls::button(
                                ui,
                                Surface::Dialog,
                                controls::Action::Outline,
                                true,
                                button,
                            )
                            .clicked()
                            {
                                chosen = Some(choice.clone());
                            }
                            ui.weak(palette::conversion_detail(choice));
                            ui.add_space(orange_ui_controls::SPACE_SMALL);
                        }
                    });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close = controls::button(
                        ui,
                        Surface::Dialog,
                        controls::Action::Outline,
                        true,
                        egui::Button::new(t("cancel")),
                    )
                    .clicked();
                });
            },
        )
        .response;
        if shown.clicked_elsewhere() || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        if let Some(choice) = chosen {
            let _ = self.connect_chosen(host, &choice);
        } else if close {
            self.conversion_choice = None;
        }
    }

    /// Takes one wire off, by its two ends.
    pub fn disconnect_wire(
        &mut self,
        host: &mut Host,
        from: (NodeId, &str),
        to: (NodeId, &str),
    ) -> bool {
        let Some(draft) = self.editing() else {
            return false;
        };
        let result = host.disconnect_wire(draft, from, to);
        self.edited(host, result)
    }

    /// Puts `node` into the wire from `from` to `to`; a node that does not fit says why
    /// in the log and nothing changes.
    pub fn insert_into_wire(
        &mut self,
        host: &mut Host,
        node: NodeId,
        from: (NodeId, &str),
        to: (NodeId, &str),
    ) -> bool {
        let Some(draft) = self.editing() else {
            return false;
        };
        let result = host.insert_into_wire(draft, node, from, to);
        self.edited(host, result)
    }

    fn add_node_into_wire(
        &mut self,
        host: &mut Host,
        definition: &str,
        at: [f32; 2],
        wire: &canvas::WireEnds,
    ) -> Option<NodeId> {
        let draft = self.editing()?;
        match host.add_node_into_wire(
            draft,
            definition,
            at,
            (wire.0.0, &wire.0.1),
            (wire.1.0, &wire.1.1),
        ) {
            Ok(node) => {
                let _ = self.edited(host, Ok(true));
                self.canvas.select(Some(node));
                Some(node)
            }
            Err(error) => {
                let _ = self.edited(host, Err(error));
                self.panels.log = true;
                self.log_events = true;
                None
            }
        }
    }

    /// Takes the wires off one port.
    pub fn disconnect(&mut self, host: &mut Host, at: (NodeId, &str), input: bool) -> bool {
        let Some(draft) = self.editing() else {
            return false;
        };
        let result = host.disconnect(draft, at, input);
        self.edited(host, result)
    }

    /// Writes a value on an input, typed plainly; `None` takes it off.
    pub fn set_input(&mut self, host: &mut Host, at: (NodeId, &str), typed: Option<&str>) -> bool {
        let Some(draft) = self.editing() else {
            return false;
        };
        let result = host.set_input(draft, at, typed);
        self.edited(host, result)
    }

    /// Adds an instance of a variadic port, such as another branch of a fork.
    /// Returns its name, such as `branch#1`.
    pub fn add_port(&mut self, host: &mut Host, node: NodeId, port: &str) -> Option<String> {
        let draft = self.editing()?;
        match host.add_port(draft, node, port) {
            Ok(added) => {
                let _ = self.edited(host, Ok(true));
                Some(added)
            }
            Err(error) => {
                let _ = self.edited(host, Err(error));
                None
            }
        }
    }

    /// Takes off one instance of a variadic port, with its wires.
    pub fn remove_port(&mut self, host: &mut Host, at: (NodeId, &str)) -> bool {
        let Some(draft) = self.editing() else {
            return false;
        };
        let result = host.remove_port(draft, at);
        self.edited(host, result)
    }

    /// Adds a node of kind `definition` that names the variable `variable`, such as a
    /// Get variable node for `count`, and selects it.
    pub fn add_variable_node(
        &mut self,
        host: &mut Host,
        definition: &str,
        variable: &str,
    ) -> Option<NodeId> {
        let node = self.add_node(host, definition)?;
        let _ = self.set_setting(
            host,
            node,
            "/variable",
            Some(serde_json::Value::from(variable)),
        );
        self.canvas.select(Some(node));
        Some(node)
    }

    /// Shows the Variables tab with the form for a new variable open.
    pub fn new_variable_form(&mut self) {
        self.panels.nodes = true;
        self.tab = Tab::Variables;
        self.forms.reveal_variable(None);
    }

    /// Whether there is an edit to undo, and one to do again.
    pub fn can_undo(&self) -> (bool, bool) {
        self.document
            .as_ref()
            .map_or((false, false), |document| document.draft.can_undo())
    }

    /// Takes back the last edit.
    pub fn undo(&mut self, host: &mut Host) {
        let Some(draft) = self.editing() else {
            return;
        };
        if draft.undo() {
            self.reload(host);
        }
    }

    /// Does again the edit last taken back.
    pub fn redo(&mut self, host: &mut Host) {
        let Some(document) = &mut self.document else {
            return;
        };
        if document.draft.redo() {
            self.reload(host);
        }
    }

    /// After undo or redo: a step back may move nodes too, so the canvas takes the
    /// document's places again.
    fn reload(&mut self, host: &mut Host) {
        self.refresh(host);
        if let Some(document) = &self.document {
            self.canvas.restore(&document.graph);
        }
    }

    /// Shows a new graph that has no file yet, as `untitled.json`. With file dialogs,
    /// saving it asks where.
    pub fn open_untitled(&mut self, host: &mut Host, text: String) {
        self.open(host, "untitled.json", text);
        if let Some(document) = &mut self.document {
            document.untitled = true;
        }
    }

    /// Saves the open document to its path, keeping the previous version beside it.
    /// A new graph with no file yet asks where instead, when the application has file
    /// dialogs.
    pub fn save(&mut self) -> Result<(), String> {
        self.keep_layout();
        if self.file_dialogs
            && self
                .document
                .as_ref()
                .is_some_and(|document| document.untitled)
        {
            self.ask(FileRequest::SaveAs);
            return Ok(());
        }
        let Some(document) = &mut self.document else {
            return Ok(());
        };
        let path = std::path::Path::new(&document.path);
        let text = document.draft.text().map_err(|error| error.to_string())?;
        orange_host::save_document(path, &text).map_err(|error| error.to_string())?;
        document.unsaved = false;
        let line = tf("saved", &[("path", document.path.to_string().into())]);
        self.say(line);
        Ok(())
    }

    /// Saves the open document to `path` and keeps it as the document's path from then on.
    pub fn save_as(&mut self, path: impl Into<String>) -> Result<(), String> {
        let Some(document) = &mut self.document else {
            return Ok(());
        };
        let before = std::mem::replace(&mut document.path, path.into());
        let untitled = std::mem::replace(&mut document.untitled, false);
        let saved = self.save();
        if saved.is_err()
            && let Some(document) = &mut self.document
        {
            document.path = before;
            document.untitled = untitled;
        }
        saved
    }

    /// Where the open document is saved.
    pub fn path(&self) -> Option<&str> {
        self.document
            .as_ref()
            .map(|document| document.path.as_str())
    }

    /// Asks the application for a file action, as the toolbar does.
    pub fn ask(&mut self, request: FileRequest) {
        self.request = Some(request);
    }

    /// Puts the path a person chose for a [`FileRequest::ChoosePath`] on the input it was
    /// asked for, and in its field.
    pub fn choose_path(&mut self, host: &mut Host, node: NodeId, port: &str, path: &str) -> bool {
        let set = self.set_input(host, (node, port), Some(path));
        if set && self.inspected == Some(node) {
            let _ = self.typed.insert(port.to_owned(), path.to_owned());
        }
        set
    }

    /// The file action asked for since the last call, for the application to carry out
    /// with its dialogs: [`Editor::open`] with what was chosen, or [`Editor::save_as`].
    pub fn take_request(&mut self) -> Option<FileRequest> {
        self.request.take()
    }

    /// What the last check found; empty when the document is valid or none is open.
    pub fn problems(&self) -> &[String] {
        self.document
            .as_ref()
            .map_or(&[], |document| &document.problems)
    }

    /// The log, oldest line first.
    pub fn log(&self) -> impl Iterator<Item = &str> {
        self.log.iter().map(LogLine::display)
    }

    /// Whether the open document is running.
    pub fn is_running(&self) -> bool {
        self.document
            .as_ref()
            .is_some_and(|document| document.flow.is_some())
    }

    fn problems_of(error: &HostError, names: &Names) -> Vec<String> {
        match error {
            HostError::Invalid(diagnostics) => diagnostics
                .iter()
                .map(|diagnostic| {
                    format!(
                        "{}: {}",
                        diagnostic.code,
                        diagnostics::problem_text(diagnostic, names)
                    )
                })
                .collect(),
            other => vec![other.to_string()],
        }
    }

    /// Checks the open document against the host's packages.
    pub fn check(&mut self, host: &Host) {
        self.refresh_problem_text = false;
        let Some(document) = &mut self.document else {
            return;
        };
        self.canvas.set_marks(document.check_problems(host));
        let mut drafts = document
            .run_omitted
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        for problem in &document.problem_locations {
            if problem.level != ProblemLevel::Draft
                && let Some(node) = problem.node
            {
                drafts.remove(&node);
            }
        }
        self.canvas.set_draft_marks(drafts);
    }

    /// Starts the open document as a flow. A document that cannot start says why in
    /// its problems.
    pub fn run(&mut self, host: &mut Host) {
        self.check(host);
        self.preview_values(host);
        let Some(document) = &mut self.document else {
            return;
        };
        if document.flow.is_some() || document.run_blocked {
            return;
        }
        // Variables shared by graphs live in the host. The first graph that declares
        // them declares them there too; later graphs read and write the same ones.
        let shares = document
            .declared
            .variables
            .iter()
            .any(|variable| variable.scope == orange_host::VarScope::Workspace);
        match document.runtime_text(host).and_then(|text| {
            if shares && host.shared_variables().is_empty() {
                host.share_variables(&text)?;
            }
            host.open(&text)
        }) {
            Ok(flow) => {
                self.canvas.begin_run();
                let _ = host.observe(flow, true);
                self.canvas.set_manual_values(host.info(flow));
                let entries = host
                    .info(flow)
                    .map(|info| info.manual_entries.iter().map(|(node, _)| *node).collect())
                    .unwrap_or_default();
                document.flow = Some((flow, entries));
                let line = tf(
                    "flow-opened",
                    &[
                        ("flow", flow.to_string().into()),
                        ("path", document.path.to_string().into()),
                    ],
                );
                let skipped = !document.run_omitted.is_empty();
                self.say(line);
                if skipped {
                    self.say(t("run-isolated-problems").to_owned());
                }
            }
            Err(error) => {
                document.set_problem_error(&error);
                self.panels.log = true;
                self.log_events = false;
            }
        }
    }

    /// Stops and closes the flow of the open document.
    pub fn stop(&mut self, host: &mut Host) {
        self.clear_manual_value_form();
        let Some((flow, _)) = self
            .document
            .as_mut()
            .and_then(|document| document.flow.take())
        else {
            return;
        };
        let _ = host.stop(flow, StopMode::Graceful);
        let _ = host.close(flow);
        // What the run showed stays, dimmed, until the next run.
        self.canvas.audio_running = false;
        self.canvas.mark_stale();
        self.say(tf("flow-closed", &[("flow", flow.to_string().into())]));
    }

    /// Fires a manual entry of the running document.
    pub fn trigger(&mut self, host: &Host, node: NodeId) {
        let Some((flow, _)) = self
            .document
            .as_ref()
            .and_then(|document| document.flow.as_ref())
        else {
            return;
        };
        if let Some(payload) = host.manual_payload(*flow, node) {
            self.begin_manual_payload(payload);
            return;
        }
        if let Err(error) = host.trigger(*flow, node, None) {
            self.say_at(NoteLevel::Error, format!("[{flow}] {error}"));
        }
    }

    /// Takes up to [`MAX_EVENTS_PER_POLL`] host events into the log. Returns whether
    /// anything arrived, so a window knows to draw and poll again. The finite batch
    /// leaves time for input and painting even when producers continuously refill it.
    pub fn take_events(&mut self, events: &orange_host::Events) -> bool {
        self.drain_event_batch(events) != 0
    }

    /// Processes one batch and reports whether the window should poll again immediately.
    pub fn poll_events(&mut self, events: &orange_host::Events) -> bool {
        self.drain_event_batch(events) == MAX_EVENTS_PER_POLL
    }

    /// Reconciles the active run after the observer queue is drained. Terminal
    /// state is available here even when its notification did not fit.
    pub fn reconcile_run_state(&mut self, host: &Host) -> bool {
        let Some(document) = &mut self.document else {
            return false;
        };
        let Some((flow, _)) = &document.flow else {
            return false;
        };
        if host.state(*flow).is_some_and(|state| {
            !matches!(
                state,
                orange_host::RunState::Stopped
                    | orange_host::RunState::Faulted
                    | orange_host::RunState::Disposed
            )
        }) {
            return false;
        }
        document.flow = None;
        self.clear_manual_value_form();
        self.canvas.audio_running = false;
        self.canvas.mark_stale();
        true
    }

    fn drain_event_batch(&mut self, events: &orange_host::Events) -> usize {
        let mut received = 0;
        for _ in 0..MAX_EVENTS_PER_POLL {
            let Ok(event) = events.try_recv() else {
                break;
            };
            received += 1;
            let Some(document) = &mut self.document else {
                continue;
            };
            let ours = document
                .flow
                .as_ref()
                .is_some_and(|(flow, _)| *flow == event.flow);
            if !ours {
                continue;
            }
            // Only what the Log will show is formatted: successful steps and live values
            // are written only while Live trace is on.
            let failed_step = matches!(&event.kind, FlowEvent::Node(step)
            if step.fault.is_some()
                || matches!(
                    step.outcome,
                    orange_host::ControlOutcome::Failed | orange_host::ControlOutcome::TimedOut
                ));
            let logged = match &event.kind {
                FlowEvent::Node(_) => self.live_trace || failed_step,
                FlowEvent::Ports(_) | FlowEvent::PortsCleared(_) | FlowEvent::Variables(_) => {
                    self.live_trace
                }
                _ => true,
            };
            let mut said = if logged {
                lines(&event, &document.names)
            } else {
                Vec::new()
            };
            let file_fault = match &event.kind {
                FlowEvent::Node(step) => step
                    .fault
                    .as_ref()
                    .map(|fault| (fault.source, fault.fault.code.as_str())),
                FlowEvent::SourceFault { node, fault } => Some((*node, fault.code.as_str())),
                _ => None,
            };
            if let Some((
                node,
                code @ ("org.orange.files/Path"
                | "org.orange.files/NotFound"
                | "org.orange.files/ReadOnly"),
            )) = file_fault
                && let Some(view) = document.node_view(node)
                && let Some(folder) =
                    properties::connection_named(view, &document.declared.connections)
            {
                let key = match code {
                    "org.orange.files/Path" => "folder-invalid-path",
                    "org.orange.files/NotFound" => "folder-missing-path",
                    _ => "folder-read-only",
                };
                said.push(tf(key, &[("folder", folder.name.as_str().into())]));
            }
            // The canvas follows the run: a step can pulse along an execution wire, and the
            // latest value on an output shows beside it.
            match &event.kind {
                FlowEvent::Node(step) => {
                    let failed = step.fault.is_some()
                        || matches!(
                            step.outcome,
                            orange_host::ControlOutcome::Failed
                                | orange_host::ControlOutcome::TimedOut
                        );
                    self.canvas.record_step(step.node, failed, step.outcome);
                    if let Some(port) = &step.route {
                        self.canvas.record_entry_route(step.node, port.to_string());
                    }
                    // A failure stays on its node, with what the log says about it,
                    // until the node takes a step that does not fail.
                    if failed {
                        let why = said.join("; ");
                        self.canvas.fail(step.node, why);
                    } else {
                        self.canvas.recover(step.node);
                    }
                }
                FlowEvent::Ports(values) => {
                    for (node, port, value) in values {
                        let id = port.to_string();
                        let view = document.node_view(*node);
                        if id == "spectrum"
                            && view.is_some_and(|view| {
                                view.definition == "org.orange.audio/multiband-spectrum"
                            })
                            && let orange_host::Value::List(values) = value
                        {
                            let bins = self.canvas.spectra.entry(*node).or_default();
                            bins.clear();
                            // The chart spreads however many logarithmic buckets
                            // arrive (246 today) across 20 Hz to 24 kHz.
                            bins.extend((0..values.len().min(1024)).filter_map(|index| {
                                match values.get(index) {
                                    Some(orange_host::Value::F64(db)) => Some(db),
                                    _ => None,
                                }
                            }));
                            // Display bins have their own canvas presentation. An
                            // explicitly wired spectrum still needs its live sample.
                            if !view.is_some_and(|view| {
                                view.ports
                                    .iter()
                                    .any(|port| !port.input && port.id == id && port.connected)
                            }) {
                                continue;
                            }
                        }
                        if id == "level"
                            && view.is_some_and(|view| {
                                view.definition == "org.orange.audio/noise-level"
                            })
                            && let orange_host::Value::F64(level) = value
                        {
                            self.canvas.noise_readings.insert(*node, *level);
                        }
                        if id == "channel_levels"
                            && view.is_some_and(|view| {
                                view.definition == "org.orange.audio/noise-level"
                            })
                        {
                            if let Some(reading) =
                                canvas::spectrum::ChannelLevels::from_value(value)
                            {
                                self.canvas.noise_channels.insert(*node, reading);
                            } else {
                                self.canvas.noise_channels.remove(node);
                            }
                            if !view.is_some_and(|view| {
                                view.ports
                                    .iter()
                                    .any(|port| !port.input && port.id == id && port.connected)
                            }) {
                                continue;
                            }
                        }
                        let ty = view
                            .and_then(|view| view.ports.iter().find(|shown| shown.id == id))
                            .and_then(|shown| shown.ty.as_deref());
                        if ty.is_some_and(|ty| {
                            ty == "org.orange.audio/stream@1" || ty.starts_with("core/stream@1<")
                        }) {
                            self.canvas.show_value(
                                *node,
                                port.to_string(),
                                String::new(),
                                String::new(),
                            );
                            continue;
                        }
                        let Some((short, full)) = presented_value(&self.canvas.enums, ty, value)
                        else {
                            continue;
                        };
                        let decibels = view.is_some_and(|view| {
                            view.definition.starts_with("org.orange.audio/")
                                && (id == "level"
                                    || id.starts_with("band_level#")
                                    || id == "payload"
                                        && view.definition == "org.orange.audio/transient")
                        });
                        let short = if decibels {
                            format!("{short} dBFS")
                        } else {
                            short
                        };
                        self.canvas.show_observed_value(
                            *node,
                            id,
                            canvas::Shown { short, full },
                            value,
                            false,
                        );
                    }
                }
                FlowEvent::PortsCleared(ports) => {
                    for (node, port) in ports {
                        self.canvas.clear_value(*node, &port.to_string());
                    }
                }
                FlowEvent::Variables(changed) => {
                    for (variable, value) in changed {
                        let id = variable.to_string();
                        let ty = document
                            .declared
                            .variables
                            .iter()
                            .find(|declared| declared.id.as_deref() == Some(id.as_str()))
                            .map(|declared| declared.ty.as_str());
                        let shown = value.as_ref().map_or_else(
                            || "unset".to_owned(),
                            |value| canvas::value_text(&self.canvas.enums, ty, value),
                        );
                        let exact_text = match value {
                            Some(
                                text @ (orange_host::Value::Str(_) | orange_host::Value::Json(_)),
                            ) => Some(text.clone()),
                            _ => None,
                        };
                        let _ = document
                            .values
                            .insert(id, ObservedVariable { shown, exact_text });
                    }
                }
                // A trigger or live value whose provider would not start, or failed
                // while running, stays marked on its node like a failed step.
                FlowEvent::SourceRefused { node, .. } | FlowEvent::SourceFault { node, .. } => {
                    self.canvas.fail(*node, said.join("; "));
                }
                // Values from before an edit or the end of the run are kept, dimmed.
                FlowEvent::Updated { .. } => self.canvas.mark_stale(),
                FlowEvent::Finished => {
                    document.flow = None;
                    self.clear_manual_value_form();
                    self.canvas.audio_running = false;
                    self.canvas.mark_stale();
                }
                _ => {}
            }
            let level = match &event.kind {
                FlowEvent::SourceRefused { .. } | FlowEvent::SourceFault { .. } => NoteLevel::Error,
                FlowEvent::Node(_) if failed_step => NoteLevel::Error,
                FlowEvent::TraceLost(_) => NoteLevel::Warning,
                FlowEvent::Node(_)
                | FlowEvent::Ports(_)
                | FlowEvent::PortsCleared(_)
                | FlowEvent::Variables(_) => NoteLevel::Trace,
                _ => NoteLevel::Info,
            };
            for line in said {
                self.say_at(level, line);
            }
        }
        received
    }

    /// Draws the editor into `ui` and applies what the person did.
    pub fn ui(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        self.ui_with_layout(ui, host, Self::default_layout);
    }

    /// Runs the editor's frame around a host-supplied layout. The closure can
    /// place any of the editor panels in the host application's own containers.
    pub fn ui_with_layout(
        &mut self,
        ui: &mut egui::Ui,
        host: &mut Host,
        layout: impl FnOnce(&mut Self, &mut egui::Ui, &mut Host),
    ) {
        i18n::set_language(self.language());
        icons::install(ui.ctx());
        if !self.scale_applied {
            if let Some(scale) = self.preferred_scale {
                ui.ctx().set_zoom_factor(scale);
            }
            if let Some(theme) = self.preferred_theme {
                ui.ctx().set_theme(theme);
            }
            self.scale_applied = true;
        }
        self.preferred_scale = Some(ui.ctx().zoom_factor());
        self.canvas.update_help(host, &self.language);
        if self.refresh_problem_text {
            let locale = self.language().to_owned();
            if let Some(document) = self.document.as_mut()
                && let Ok(view) = host.draft_authoring_view_in(&document.draft, &locale)
            {
                document.replace_graph(view.graph);
                self.canvas.set_failure_routes(view.failure_routes);
                self.canvas.set_port_details(view.port_details);
            }
            self.check(host);
        }
        if let Some(clip) = self.clip_out.take() {
            ui.ctx().copy_text(clip);
        }
        if std::mem::take(&mut self.paste_asked) {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        }
        self.canvas.shortcuts_enabled = !self.settings_open
            && !self.forms.open_connection
            && !self.has_node_ergonomics_modal()
            && self.value_form.is_none();
        self.keyboard(ui, host);
        layout(self, ui, host);
        if self.window_toolbar {
            resize_borders(ui.ctx());
        }
        let _ = self.settings_window(ui.ctx());
        self.package_import_dialog(ui.ctx(), host);
        self.bundle_import_dialog(ui.ctx(), host);
        self.package_removal_dialog(ui.ctx(), host);
        self.function_export_dialog(ui.ctx(), host);
        self.show_connection_dialog(ui.ctx(), host);
        self.variable_inspector_window(ui.ctx());
        self.value_dialog(ui.ctx(), host);
        self.for_each_mode_dialog(ui.ctx(), host);
        self.build_map_rows_dialog(ui.ctx(), host);
        if std::mem::take(&mut self.run_asked) {
            self.run(host);
        }
    }

    fn default_layout(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        self.toolbar(ui, host);
        self.workspace_bar(ui);
        if self.panels.log {
            egui::Panel::bottom("orange-log")
                .resizable(true)
                .default_size(170.0)
                .min_size(90.0)
                .show(ui, |ui| self.log_panel(ui));
        }
        if self.panels.nodes {
            egui::Panel::left("orange-nodes")
                .resizable(true)
                .default_size(280.0)
                .min_size(240.0)
                .show(ui, |ui| self.nodes_panel(ui, host));
        }
        if self.panels.properties || self.panels.packages {
            egui::Panel::right("orange-inspector")
                .resizable(true)
                .default_size(280.0)
                .min_size(240.0)
                .show(ui, |ui| self.right_panel(ui, host));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.canvas_panel(ui, host));
    }

    /// Whether a host-owned focused keyboard source may use this key. The host
    /// must also suppress source input while egui edits text or lacks window focus.
    pub fn accepts_graph_key(&self, key: egui::Key, modifiers: egui::Modifiers) -> bool {
        !self.settings_open
            && !self.forms.open_connection
            && self.quick.is_none()
            && self.menu.is_none()
            && self.node_config_menu.is_none()
            && !self.has_node_ergonomics_modal()
            && self.value_form.is_none()
            && !self.canvas.keys.0.values().flatten().any(|binding| {
                binding.logical_key == key && modifiers.matches_logically(binding.modifiers)
            })
    }

    /// Draws the editor's top command bar in a top panel.
    pub fn toolbar(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        icons::install(ui.ctx());
        egui::Panel::top("orange-toolbar").show(ui, |ui| {
            ui.spacing_mut().interact_size.y = 28.0;
            if ui.available_width() < if self.window_toolbar { 820.0 } else { 720.0 } {
                ui.spacing_mut().item_spacing.x = 2.0;
            }
            ui.horizontal(|ui| {
                let open = self.document.is_some();
                let gap = ui.spacing().item_spacing.x;
                let button = icons::BUTTON_WIDTH + gap;
                let separator = 6.0 + gap;
                let leading_buttons = if self.file_dialogs { 13.0 } else { 10.0 };
                let trailing_width = if self.window_toolbar {
                    6.0 * button + 2.0 * separator
                } else {
                    3.0 * button + separator
                };
                if ui.available_width()
                    < leading_buttons * button + 4.0 * separator + trailing_width
                {
                    egui::ScrollArea::horizontal()
                        .id_salt("toolbar-edit-commands")
                        .max_width((ui.available_width() - trailing_width).max(1.0))
                        .min_scrolled_width(1.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| self.toolbar_edit_commands(ui, host, open))
                        });
                } else {
                    self.toolbar_edit_commands(ui, host, open);
                }
                let rest = ui.available_rect_before_wrap();
                if self.window_toolbar {
                    let drag = egui::Rect::from_min_max(
                        rest.min,
                        egui::pos2((rest.right() - 300.0).max(rest.left()), rest.bottom()),
                    );
                    if drag.width() > 100.0 {
                        let name = self
                            .path()
                            .and_then(|path| std::path::Path::new(path).file_name())
                            .map_or_else(|| "OrangeEditor".into(), |name| name.to_string_lossy());
                        let painter = ui.painter().with_clip_rect(drag.shrink(8.0));
                        painter.text(
                            drag.center(),
                            egui::Align2::CENTER_CENTER,
                            name,
                            egui::FontId::proportional(12.0),
                            ui.visuals().weak_text_color(),
                        );
                    }
                    let response = ui.interact(
                        drag,
                        ui.scope_id().with("window-drag"),
                        egui::Sense::click_and_drag(),
                    );
                    if response.double_clicked() {
                        let maximized =
                            ui.input(|input| input.viewport().maximized.unwrap_or(false));
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                    } else if response.drag_started() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.window_toolbar {
                        if icons::button(ui, true, icons::CLOSE, t("close-window"), "Alt+F4")
                            .clicked()
                        {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        let maximized =
                            ui.input(|input| input.viewport().maximized.unwrap_or(false));
                        if icons::button(
                            ui,
                            true,
                            if maximized {
                                icons::RESTORE
                            } else {
                                icons::MAXIMIZE
                            },
                            if maximized {
                                t("restore-window")
                            } else {
                                t("maximize-window")
                            },
                            "",
                        )
                        .clicked()
                        {
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                        }
                        if icons::button(ui, true, icons::MINIMIZE, t("minimize-window"), "")
                            .clicked()
                        {
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                        ui.separator();
                    }
                    if icons::button(
                        ui,
                        true,
                        icons::SETTINGS,
                        t("settings"),
                        &self.canvas.keys.label("settings"),
                    )
                    .clicked()
                    {
                        self.settings_open = true;
                    }
                    ui.separator();
                    let running = self.is_running();
                    let blocked = self
                        .document
                        .as_ref()
                        .is_some_and(|document| document.run_blocked);
                    let (hint, key) = if running {
                        ("stop-with-key", "stop")
                    } else {
                        ("run-with-key", "run")
                    };
                    if icons::run_button(ui, running || open && !blocked, running)
                        .on_hover_text(tf(hint, &[("key", self.canvas.keys.label(key).into())]))
                        .on_disabled_hover_text(t(if blocked {
                            "run-blocked-hint"
                        } else {
                            "run-hint"
                        }))
                        .clicked()
                    {
                        if running {
                            self.stop(host);
                        } else {
                            ui.memory_mut(egui::Memory::stop_text_input);
                            self.run_asked = true;
                        }
                    }
                    if icons::button(ui, open, icons::CHECK, t("check-graph-hint"), "").clicked() {
                        self.check(host);
                    }
                });
            });
        });
    }

    fn toolbar_edit_commands(&mut self, ui: &mut egui::Ui, host: &mut Host, open: bool) {
        // File
        if self.file_dialogs {
            if icons::button(
                ui,
                true,
                icons::NEW,
                t("new-graph"),
                &self.canvas.keys.label("new"),
            )
            .clicked()
            {
                self.ask(FileRequest::New);
            }
            if icons::button(
                ui,
                true,
                icons::OPEN,
                t("open-graph"),
                &self.canvas.keys.label("open"),
            )
            .clicked()
            {
                self.ask(FileRequest::Open);
            }
        }
        if icons::button(
            ui,
            open,
            icons::SAVE,
            t("save"),
            &self.canvas.keys.label("save"),
        )
        .clicked()
            && let Err(message) = self.save()
        {
            self.say(message);
        }
        if self.file_dialogs
            && icons::button(
                ui,
                open,
                icons::SAVE_AS,
                t("save-as"),
                &self.canvas.keys.label("save-as"),
            )
            .clicked()
        {
            self.ask(FileRequest::SaveAs);
        }
        ui.separator();
        // Edit
        let (undo, redo) = self.can_undo();
        if icons::button(
            ui,
            undo,
            icons::UNDO,
            t("undo"),
            &self.canvas.keys.label("undo"),
        )
        .clicked()
        {
            self.undo(host);
        }
        if icons::button(
            ui,
            redo,
            icons::REDO,
            t("redo"),
            &self.canvas.keys.label("redo"),
        )
        .clicked()
        {
            self.redo(host);
        }
        ui.separator();
        self.canvas_toolbar(ui, host);
        ui.separator();
    }

    fn canvas_toolbar(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        let open = self.document.is_some();
        let chosen = self.canvas.selection().len();
        let group = self.canvas.selected_group().map(|group| group.id.clone());
        let wire = if chosen == 0 && group.is_none() {
            self.canvas.selected_wire().cloned()
        } else {
            None
        };
        if icons::button(
            ui,
            open && chosen > 0 && group.is_none(),
            icons::GROUP,
            t("group-selection"),
            &self.canvas.keys.label("group"),
        )
        .clicked()
        {
            let _ = self.group_selection(host, t("group"));
        }
        if icons::button(
            ui,
            group.is_some(),
            icons::UNGROUP,
            t("ungroup"),
            &self.canvas.keys.label("ungroup"),
        )
        .clicked()
            && let Some(id) = group
        {
            let _ = self.ungroup(host, &id);
        }
        if icons::button(
            ui,
            open && chosen > 1,
            icons::ARRANGE,
            t("arrange-selection"),
            &self.canvas.keys.label("arrange"),
        )
        .clicked()
        {
            let _ = self.arrange_selection();
        }
        if icons::button(
            ui,
            wire.is_some(),
            icons::DISCONNECT,
            t("unlink-wire"),
            &self.canvas.keys.label("delete"),
        )
        .clicked()
            && let Some(wire) = wire
        {
            let _ = self.remove_wire(host, &wire);
        }
        if icons::button(
            ui,
            open,
            icons::FIT,
            t("fit-to-view"),
            &self.canvas.keys.label("fit"),
        )
        .clicked()
        {
            self.canvas.fit();
        }
        ui.separator();
        if icons::toggle_button(
            ui,
            icons::SNAP,
            self.canvas.snap,
            t("snap-grid"),
            &self.canvas.keys.label("snap"),
        )
        .on_hover_text(t("snap-grid-hint"))
        .clicked()
        {
            self.canvas.snap = !self.canvas.snap;
        }
        let grid = icons::button(ui, true, icons::GRID, t("grid-settings"), "");
        let _ = egui::Popup::menu(&grid)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_width(ui.available_width().min(250.0));
                self.grid_menu(ui);
            });
    }

    /// Draws the workspace status and panel controls in a bottom panel.
    pub fn workspace_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("orange-workspace-bar").show(ui, |ui| {
            ui.spacing_mut().interact_size.y = 28.0;
            if ui.available_width() >= 780.0 {
                let height = ui.spacing().interact_size.y;
                ui.columns(3, |columns| {
                    columns[0].horizontal(|ui| self.workspace_left(ui));
                    columns[1].horizontal(|ui| self.workspace_diagnostics(ui, true));
                    let size = egui::vec2(columns[2].available_width(), height);
                    columns[2].allocate_ui_with_layout(
                        size,
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| self.workspace_right(ui),
                    );
                });
            } else {
                ui.horizontal_wrapped(|ui| {
                    self.workspace_left(ui);
                    ui.separator();
                    self.workspace_diagnostics(ui, false);
                    ui.separator();
                    self.workspace_right(ui);
                });
            }
        });
    }

    fn workspace_left(&mut self, ui: &mut egui::Ui) {
        if icons::dock_toggle(
            ui,
            self.panels.nodes,
            egui::Align::Min,
            t("left-panel-hint"),
            &self.canvas.keys.label("left-panel"),
        )
        .clicked()
        {
            self.toggle_left_panel();
        }
    }

    fn toggle_left_panel(&mut self) {
        self.panels.nodes = !self.panels.nodes;
    }

    fn toggle_right_panel(&mut self) {
        if self.panels.properties || self.panels.packages {
            self.renaming_node = None;
            self.packages_tab = self.panels.packages && !self.panels.properties;
            self.panels.properties = false;
            self.panels.packages = false;
        } else {
            self.panels.properties = !self.packages_tab;
            self.panels.packages = self.packages_tab;
        }
    }

    fn workspace_diagnostics(&mut self, ui: &mut egui::Ui, centered: bool) {
        let count = self.document.as_ref().map_or(0, |document| {
            document
                .problem_locations
                .iter()
                .filter(|problem| problem.level != ProblemLevel::Draft)
                .count()
        });
        let problems = if count == 0 {
            t("problems").to_owned()
        } else {
            tf("problems-count", &[("count", count.into())])
        };
        let tabs = [(false, problems.as_str()), (true, t("log"))];
        if centered {
            let font = egui::TextStyle::Button.resolve(ui.style());
            let width = tabs
                .iter()
                .map(|(_, label)| {
                    let text_width = ui
                        .painter()
                        .layout_no_wrap(
                            (*label).to_owned(),
                            font.clone(),
                            ui.visuals().text_color(),
                        )
                        .size()
                        .x;
                    text_width + 2.0 * ui.spacing().button_padding.x
                })
                .sum::<f32>()
                + ui.spacing().item_spacing.x;
            ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
        }
        for (events, label) in tabs {
            let active = self.panels.log && self.log_events == events;
            if ui.selectable_label(active, label).clicked() {
                self.panels.log = !active;
                self.log_events = events;
            }
        }
    }

    fn workspace_right(&mut self, ui: &mut egui::Ui) {
        let open = self.panels.properties || self.panels.packages;
        if icons::dock_toggle(
            ui,
            open,
            egui::Align::Max,
            t("right-panel-hint"),
            &self.canvas.keys.label("right-panel"),
        )
        .clicked()
        {
            self.toggle_right_panel();
        }
        if self.document.is_some() {
            ui.menu_button(format!("{:.0}%", self.canvas.zoom() * 100.0), |ui| {
                let label = ui.label(t("canvas-zoom"));
                let mut zoom = self.canvas.zoom();
                let start = *canvas::ZOOM_RANGE.start();
                let end = *canvas::ZOOM_RANGE.end();
                let guides = [
                    orange_ui_controls::SliderGuide {
                        fraction: ((0.50 - f64::from(start)) / (f64::from(end) - f64::from(start)))
                            as f32,
                        label: "50%",
                    },
                    orange_ui_controls::SliderGuide {
                        fraction: ((0.75 - f64::from(start)) / (f64::from(end) - f64::from(start)))
                            as f32,
                        label: "75%",
                    },
                    orange_ui_controls::SliderGuide {
                        fraction: ((1.00 - f64::from(start)) / (f64::from(end) - f64::from(start)))
                            as f32,
                        label: "100%",
                    },
                    orange_ui_controls::SliderGuide {
                        fraction: ((1.25 - f64::from(start)) / (f64::from(end) - f64::from(start)))
                            as f32,
                        label: "125%",
                    },
                    orange_ui_controls::SliderGuide {
                        fraction: ((1.50 - f64::from(start)) / (f64::from(end) - f64::from(start)))
                            as f32,
                        label: "150%",
                    },
                    orange_ui_controls::SliderGuide {
                        fraction: ((1.75 - f64::from(start)) / (f64::from(end) - f64::from(start)))
                            as f32,
                        label: "175%",
                    },
                ];
                if orange_ui_controls::guided_slider(
                    ui,
                    Surface::Dialog,
                    &mut zoom,
                    canvas::ZOOM_RANGE,
                    orange_ui_controls::SliderBehavior::Free,
                    orange_ui_controls::SliderMarks::NotchesAndValues,
                    &guides,
                )
                .labelled_by(label.id)
                .changed()
                {
                    self.canvas.zoom_around_center(zoom);
                }
                ui.separator();
                if ui.button(t("fit-to-view")).clicked() {
                    self.canvas.fit();
                    ui.close();
                }
                if ui.button(t("actual-size")).clicked() {
                    self.canvas.reset_zoom();
                    ui.close();
                }
            })
            .response
            .on_hover_text(t("zoom-controls"));
        }
        let state = if self.is_running() {
            t("running")
        } else if self.document.is_some() {
            t("stopped")
        } else {
            t("no-graph")
        };
        ui.label(state);
        if self.is_unsaved() {
            ui.weak(t("unsaved"));
        }
    }

    /// Draws the Inspector and Packages tabs in the supplied container.
    pub fn right_panel(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        let mut packages = self.panels.packages && !self.panels.properties;
        let _ = orange_ui_controls::notebook_flex(
            ui,
            Surface::Panel,
            orange_ui_controls::NotebookOrientation::Horizontal,
            orange_ui_controls::NotebookPadding::Compact,
            &mut packages,
            &[(false, t("properties")), (true, t("packages"))],
        );
        ui.separator();
        self.panels.properties = !packages;
        self.panels.packages = packages;
        if packages {
            self.renaming_node = None;
            self.packages_panel(ui, host);
        } else {
            self.properties_panel(ui, host);
        }
    }

    /// The language of the editor and its node search, from the editor's catalogues.
    fn language_choice(&mut self, ui: &mut egui::Ui) {
        let languages = i18n::catalogs().languages();
        let current = self.language().to_owned();
        let name = languages
            .iter()
            .find(|(tag, _)| *tag == current)
            .map_or(current.as_str(), |(_, name)| name.as_ref())
            .to_owned();
        let mut chosen = None;
        let _ = egui::ComboBox::from_id_salt("orange-language")
            .selected_text((icons::text(icons::LANGUAGE), format!(" {name}")))
            .show_select(ui, Surface::Dialog, |ui| {
                for (tag, name) in &languages {
                    if ui
                        .selectable_label(*tag == current, name.as_ref())
                        .clicked()
                    {
                        chosen = Some((*tag).to_owned());
                    }
                }
            })
            .response
            .on_hover_text(t("language-hint"));
        if let Some(tag) = chosen {
            self.set_language(&tag);
        }
    }

    /// Draws the node catalogue and declaration tabs in the supplied container.
    pub fn nodes_panel(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        let options = Tab::ALL.map(|(tab, label)| (tab, t(label)));
        let previous = self.tab;
        let response = orange_ui_controls::notebook_flex(
            ui,
            Surface::Panel,
            orange_ui_controls::NotebookOrientation::Horizontal,
            orange_ui_controls::NotebookPadding::Compact,
            &mut self.tab,
            &options,
        );
        if response.changed() || previous != self.tab {
            self.preferred_tab = self.tab;
        }
        ui.separator();
        match self.tab {
            Tab::Nodes => {}
            Tab::Variables => {
                self.variables_tab(ui, host);
                return;
            }
            Tab::Connections => {
                self.connections_tab(ui, host);
                return;
            }
            Tab::Types => {
                self.types_tab(ui, host);
                return;
            }
        }
        let mut add = None;
        let search_salt = "node-palette-search";
        let search_id = ui.make_persistent_id(search_salt);
        let (up, down, enter) =
            if ui.memory(|memory| memory.has_focus(search_id)) && !self.search.trim().is_empty() {
                ui.input_mut(|input| {
                    (
                        input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                        input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                        input.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                    )
                })
            } else {
                (false, false, false)
            };
        if up || down {
            ui.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
        }
        let field = controls::search(ui, search_salt, &mut self.search, t("search-hint"));
        if field.changed() {
            self.palette_highlighted = 0;
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            let searching = !self.search.trim().is_empty();
            let language = if self.language.is_empty() {
                orange_host::SOURCE_LANGUAGE
            } else {
                self.language.as_str()
            };
            let channels = self.dispatch_channels();
            let bridges: Vec<(String, String)> = self
                .bridge_outs()
                .into_iter()
                .map(|(node, channel)| (node.to_string(), channel))
                .collect();
            let results = self
                .palette
                .get(host, &self.search, language, &channels, &bridges);
            let found = results.groups;
            if searching && found.is_empty() {
                ui.weak(t("no-node-matches"));
            }
            let found_count = found.iter().map(|(_, items)| items.len()).sum::<usize>();
            let last = found_count.saturating_sub(1);
            self.palette_highlighted = self.palette_highlighted.min(last);
            if down {
                self.palette_highlighted = self.palette_highlighted.saturating_add(1).min(last);
            }
            if up {
                self.palette_highlighted = self.palette_highlighted.saturating_sub(1);
            }
            let mut index = 0;
            for (category, items) in found {
                let Some(first) = items.first() else {
                    continue;
                };
                let (heading, help) = palette::category_heading(category, &first.id, language);
                orange_ui_controls::tree_section(
                    ui,
                    Surface::Panel,
                    egui::CollapsingHeader::new(heading)
                        .id_salt((category.as_str(), searching))
                        .default_open(searching),
                    |ui| {
                        let mut section = None;
                        for item in items {
                            let next = palette::catalog_section(&item.id);
                            if !searching
                                && section != next
                                && let Some((title, description)) = next
                            {
                                ui.weak(t(title)).on_hover_text(t(description));
                            }
                            section = next;
                            let active =
                                searching && field.has_focus() && index == self.palette_highlighted;
                            index += 1;
                            let draft = self.document.as_ref().map(|document| &document.draft);
                            let card = self
                                .node_search_previews
                                .then(|| self.previews.card(host, None, &item.id))
                                .flatten();
                            let row = palette::result_row(
                                ui,
                                &item.label,
                                &item.purpose,
                                None,
                                active,
                                card,
                                Some(&item.id),
                            );
                            if row.drag_started() {
                                let card = self.previews.card(host, draft, &item.id).cloned();
                                row.dnd_set_drag_payload(palette::PaletteDrag {
                                    item: item.clone(),
                                    card,
                                });
                            }
                            if active && (up || down) {
                                row.scroll_to_me(None);
                            }
                            if row.clicked() || (active && enter) {
                                add = Some(item.id.clone());
                            }
                        }
                    },
                )
                .header_response
                .on_hover_text(t(help));
            }
        });
        if let Some(definition) = add {
            if self.document.is_some() {
                let _ = self.add_node(host, &definition);
            } else {
                self.say(t("open-graph-before-adding").to_owned());
            }
        }
    }

    /// The packages this host has, found by name, ID or description: each a framed row
    /// with its name and version, what it adds, and whether it is in use.
    fn packages_panel(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        if self.file_dialogs
            && controls::button(
                ui,
                Surface::Panel,
                controls::Action::Secondary,
                true,
                egui::Button::new((icons::text(icons::ADD), t("import-package"))),
            )
            .clicked()
        {
            self.ask(FileRequest::ImportPackage);
        }
        if self.file_dialogs {
            ui.horizontal_wrapped(|ui| {
                if controls::button(
                    ui,
                    Surface::Panel,
                    controls::Action::Secondary,
                    self.document.is_some(),
                    egui::Button::new(t("share-with-packages")),
                )
                .on_hover_text(t("share-with-packages-hint"))
                .clicked()
                {
                    self.share_with_packages(host);
                }
                if controls::button(
                    ui,
                    Surface::Panel,
                    controls::Action::Secondary,
                    true,
                    egui::Button::new((icons::text(icons::OPEN), t("open-shared-graph-ellipsis"))),
                )
                .clicked()
                {
                    self.ask(FileRequest::ImportBundle);
                }
                if self.native_plugins
                    && controls::button(
                        ui,
                        Surface::Panel,
                        controls::Action::Secondary,
                        true,
                        egui::Button::new((icons::text(icons::WARNING), t("native-plugins"))),
                    )
                    .on_hover_text(t("native-plugins-hint"))
                    .clicked()
                {
                    self.ask(FileRequest::NativePlugins);
                }
            });
        }
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        let _ = controls::search(
            ui,
            "package-search",
            &mut self.package_search,
            t("search-packages-hint"),
        );
        ui.add_space(4.0);
        let wanted = self.package_search.trim().to_lowercase();
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut shown = 0;
            let mut access_change = None;
            for package in host.packages() {
                let name = package.name().unwrap_or(package.id);
                let about = package.description().unwrap_or_default();
                if !wanted.is_empty()
                    && ![name, package.id, about]
                        .iter()
                        .any(|text| text.to_lowercase().contains(&wanted))
                {
                    continue;
                }
                shown += 1;
                let kind = match package.kind {
                    PackageKind::Base => "base",
                    PackageKind::Native => "built-in",
                    _ => "installed",
                };
                let _ = egui::Frame::new()
                    .fill(ui.visuals().faint_bg_color)
                    .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                    .corner_radius(6)
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .outer_margin(egui::Margin {
                        bottom: 6,
                        ..egui::Margin::ZERO
                    })
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.weak(package.version);
                                    ui.with_layout(
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            let _ = ui
                                                .add(
                                                    egui::Label::new(
                                                        egui::RichText::new(name).strong(),
                                                    )
                                                    .truncate(),
                                                )
                                                .on_hover_text(package.id);
                                        },
                                    );
                                },
                            );
                        });
                        if !about.is_empty() {
                            let _ =
                                ui.add(egui::Label::new(egui::RichText::new(about).small()).wrap());
                        }
                        ui.weak(tf(
                            "package-status",
                            &[
                                ("kind", kind.into()),
                                ("active", if package.active { "yes" } else { "no" }.into()),
                            ],
                        ));
                        // What the package may reach: unticking a box revokes it for
                        // graphs started from now on.
                        if let (Ok(id), Ok(version)) = (package.id.parse(), package.version.parse())
                        {
                            let key = orange_host::PackageKey { id, version };
                            if let (Some(requested), Some(access)) =
                                (host.requested_grants(&key), host.access(&key))
                                && requested != orange_host::Grants::default()
                            {
                                let mut grants = access.grants;
                                if package_import::grant_choices(ui, requested, &mut grants) {
                                    access_change = Some((
                                        key,
                                        orange_host::PackageAccess {
                                            grants,
                                            ..access.clone()
                                        },
                                    ));
                                }
                            }
                        }
                        if matches!(package.kind, PackageKind::Installed)
                            && let (Ok(id), Ok(version)) =
                                (package.id.parse(), package.version.parse())
                            && controls::button(
                                ui,
                                Surface::Panel,
                                controls::Action::Destructive,
                                true,
                                egui::Button::new((
                                    icons::text(icons::DELETE),
                                    t("remove-package"),
                                )),
                            )
                            .clicked()
                        {
                            self.pending_package_removal =
                                Some(orange_host::PackageKey { id, version });
                        }
                    });
            }
            if let Some((key, access)) = access_change.take() {
                match host.set_access(&key, access) {
                    Ok(()) => {
                        self.previews.invalidate();
                        self.say(tf(
                            "package-access-changed",
                            &[("package", key.to_string().into())],
                        ));
                    }
                    Err(error) => self.say_at(NoteLevel::Error, error.to_string()),
                }
            }
            if shown == 0 {
                ui.weak(t("no-package-matches"));
            }
        });
    }

    /// Draws the editable canvas and handles its actions in the supplied container.
    pub fn canvas_panel(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        self.prepare_devices(host);
        let Some(document) = &self.document else {
            ui.centered_and_justified(|ui| ui.label(t("open-graph-file")));
            return;
        };
        let triggers = document
            .flow
            .as_ref()
            .map_or(&[][..], |(_, entries)| entries.as_slice());
        let stored_text =
            |node, port: &str| properties::stored_input_text(host, &document.draft, node, port);
        let done = self.canvas.show_with_path_controls(
            ui,
            &document.graph,
            triggers,
            self.file_dialogs,
            Some(&stored_text),
        );
        if let Some(target) = self.canvas.pending_inbound_givers() {
            let matches = host.output_givers(&document.draft, &document.graph, target);
            self.canvas.set_inbound_givers(matches);
            ui.ctx().request_repaint();
        }
        if let Some((node, port)) = done.browse_path
            && let Some(view) = document.graph.nodes.iter().find(|view| view.id == node)
            && let Some(path) = view.paths.iter().find(|path| path.port == port)
        {
            let current = view
                .ports
                .iter()
                .find(|input| input.input && input.id == port)
                .map_or_else(String::new, Self::shown_input);
            self.request = Some(FileRequest::ChoosePath {
                node,
                port,
                folder: path.folder,
                save: path.save,
                extensions: path.extensions.clone(),
                current,
            });
        }
        if let Some((node, at)) = done.configure {
            self.canvas.select(Some(node));
            self.node_config_menu = Some((node, at));
        }
        if let Some(port) = done.inspect_value {
            self.inspected_variable = None;
            self.inspected_port = Some(port);
        }
        if let Some(selected) = done.selected
            && document
                .graph
                .nodes
                .iter()
                .any(|node| node.id == selected && node.ports.is_empty())
        {
            self.panels.properties = true;
            self.panels.packages = false;
        }
        if done.rename_node.is_some() {
            self.renaming_node = None;
        }
        if let Some((node, name)) = done.set_name {
            self.rename_node(host, node, &name);
        }
        if done.show_problems || done.show_runtime_log {
            self.panels.log = true;
            self.log_events = done.show_runtime_log;
            if done.show_problems {
                self.problems_filter = None;
                self.problems_search.clear();
            }
        }
        if done.moved {
            self.keep_layout();
        }
        if let Some(node) = done.trigger {
            self.trigger(host, node);
        }
        if let Some(ends) = done.connect {
            self.wire_with_mode_preview(host, ends);
        }
        self.conversion_menu(ui.ctx(), host);
        if let Some((node, sender)) = done.add_signal_peer {
            self.add_signal_endpoint(host, node, sender);
        }
        if let Some((reading, out)) = done.bind_bridge {
            let _ = self.bind_bridge(host, reading, out);
        }
        if let Some((node, port)) = done.add_port {
            let _ = self.add_port(host, node, &port);
        }
        if let Some((node, port, input)) = done.disconnect {
            let _ = self.disconnect(host, (node, &port), input);
        }
        if let Some((from, to)) = done.remove_wire {
            let _ = self.disconnect_wire(host, (from.0, &from.1), (to.0, &to.1));
        }
        if let Some((node, (from, to))) = done.insert {
            let _ = self.insert_into_wire(host, node, (from.0, &from.1), (to.0, &to.1));
        }
        if !done.remove.is_empty() {
            let _ = self.remove_nodes(host, &done.remove);
        }
        if let Some((node, port, typed)) = done.set_input {
            let _ = self.set_input(host, (node, &port), Some(&typed));
        }
        if let Some((node, port)) = done.edit_literal {
            self.begin_value_input(node, &port);
        }
        if let Some(node) = done.edit_map_pairs {
            self.open_map_pairs(node);
        }
        if let Some((node, port)) = done.remove_port {
            let _ = self.remove_port(host, (node, &port));
        }
        if (done.copy || done.cut)
            && let Some(clip) = self.copy(host)
        {
            self.clip = Some(clip.clone());
            ui.ctx().copy_text(clip);
            if done.cut {
                let selection = self.canvas.selection();
                let _ = self.remove_nodes(host, &selection);
            }
        }
        if let Some(clip) = done.paste {
            let at = self.canvas.paste_place();
            let _ = self.paste(host, &clip, at);
        }
        if let Some((definition, at)) = done.dropped {
            // A variable dragged from the Variables list becomes a Get, or a Set with Alt.
            match definition.strip_prefix(declarations::VARIABLE_DRAG) {
                Some(variable) => {
                    let set = ui.input(|input| input.modifiers.alt);
                    let _ = self.drop_variable(host, variable, set, at);
                }
                None => {
                    if let Some(wire) = done.drop_wire {
                        let _ = self.add_node_into_wire(host, &definition, at, &wire);
                    } else {
                        let _ = self.add_node_at(host, &definition, at);
                    }
                }
            }
        }
        if let Some((from, at, screen)) = done.loose {
            self.open_quick_add(Some(from), at, screen);
        }
        if let Some((to, at, screen)) = done.loose_into {
            self.open_quick_add_into(to, at, screen);
        }
        if let Some((at, screen)) = done.quick_add {
            self.open_quick_add(None, at, screen);
        }
        if let Some((wire, at)) = done.waypoint {
            self.set_waypoint(host, &wire, Some(at));
        }
        if let Some(edit) = done.group {
            self.group_edit(host, edit);
        }
        if done.group_selection {
            let _ = self.group_selection(host, t("group"));
        }
        if let Some((target, at, screen)) = done.context {
            self.open_menu(target, at, screen);
        }
        self.menu_popup(ui.ctx(), host);
        self.quick_add_popup(ui.ctx(), host);
        if let Some((node, pointer, value)) = done.set_config
            && self.setting_with_mode_preview(host, node, &pointer, Some(value))
        {
            ui.ctx().request_repaint();
        }
        if let Some((node, pointer)) = done.new_variable {
            self.new_variable_for_node(node, &pointer);
        }
        if let Some((node, device)) = done.device {
            self.choose_device(host, node, device);
        }
        if let Some((node, edit)) = done.spectrum_edit
            && let Some(draft) = self.editing()
        {
            let result = match edit {
                canvas::BandEdit::Add(range) => host.add_audio_band(draft, node, range),
                canvas::BandEdit::Change(band, settings) => {
                    host.change_audio_band(draft, node, band, settings)
                }
            };
            let _ = self.edited(host, result);
        }
        if let Some(adapter) = done.refresh_devices {
            let _ = self.adapter_choices.remove(&adapter);
            ui.ctx().request_repaint();
        }
        self.node_config_window(ui.ctx(), host);
        self.event_dialog(ui.ctx(), host);
        self.port_inspector_window(ui.ctx());
        self.function_window(ui.ctx(), host);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orange_host::HostOptions;
    use serde_json::json;
    use std::time::{Duration, Instant};

    fn id(n: u32) -> String {
        format!("10000000-0000-4000-8000-{n:012x}")
    }

    /// A manual entry named `go` that sets the variable `count` to 7.
    pub(crate) fn graph() -> String {
        json!({"format": "orange.graph", "format_version": 1,
            "graph_id": "20000000-0000-4000-8000-0000000000e9", "revision": 1,
            "package_lock": [], "type_lock": [], "events": [], "connections": [],
            "signatures": {}, "functions": [], "exports": [], "editor": {},
            "variables": [{"id": id(100), "name": "count", "type": "core/i64@1",
                "scope": "run", "persistent": false, "conflict": "reject"}],
            "nodes": [
                {"id": id(1), "definition_id": "core/on-manual", "definition_version": 1,
                    "config_version": 1, "config": {}, "name": "go"},
                {"id": id(2), "definition_id": "core/set-variable", "definition_version": 1,
                    "config_version": 1, "config": {"variable": id(100)},
                    "inputs": {"value": {"type": "core/i64@1", "value": "7"}}}],
            "edges": [{"id": "30000000-0000-4000-8000-000000000001",
                "from": {"node": id(1), "port": "then"}, "to": {"node": id(2), "port": "in"}}]})
        .to_string()
    }

    fn draw(editor: &mut Editor, host: &mut Host) -> usize {
        let context = egui::Context::default();
        let output = context.run_ui(egui::RawInput::default(), |ui| editor.ui(ui, host));
        let shapes = output.shapes.len();
        // No renderer takes the textures of a frame drawn for a test.
        output.drop_without_applying_deltas();
        shapes
    }

    #[test]
    fn telemetry_poll_timing() {
        for count in [500, 5000] {
            let (mut host, _) = Host::new(HostOptions::default()).unwrap();
            let seed = host.bind(&graph()).unwrap();
            let flow = host.open(&seed).unwrap();
            let mut text: serde_json::Value = serde_json::from_str(&seed).unwrap();
            let nodes = text["nodes"].as_array_mut().unwrap();
            nodes.extend((10..10 + count).map(|index| orange_testkit::core(index, "whole-number")));
            let mut editor = Editor::new();
            editor.open(&mut host, "telemetry.json", text.to_string());
            let document = editor.document.as_mut().unwrap();
            assert_eq!(document.graph.nodes.len(), count as usize + 2);
            // Use the host's identity with synthetic events. The cards under
            // measurement need no running producer.
            document.flow = Some((flow, Vec::new()));
            let (sender, receiver) = std::sync::mpsc::channel();
            let receiver = orange_host::Events::from(receiver);
            let mut next = 0;
            let port = "result".parse::<orange_host::PortKey>().unwrap();
            orange_testkit::measure_batches(
                &format!("32 polls with 128 port samples on {count} extra cards"),
                || {
                    for _ in 0..32 {
                        next += 1;
                        let values = (10 + count - 128..10 + count)
                            .map(|index| {
                                (
                                    orange_testkit::node(index),
                                    port.clone(),
                                    orange_host::Value::I64(next),
                                )
                            })
                            .collect();
                        sender
                            .send(orange_host::HostEvent {
                                flow,
                                kind: FlowEvent::Ports(values),
                            })
                            .unwrap();
                        assert!(!editor.poll_events(&receiver));
                    }
                },
            );
            assert!(
                editor
                    .canvas
                    .shown_value(orange_testkit::node(10 + count - 1), "result")
                    .is_some()
            );
        }
    }

    #[test]
    fn telemetry_card_lookup_tracks_reordered_and_removed_nodes() {
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open(&mut host, "events.json", graph());
        let document = editor.document.as_mut().unwrap();
        let first = document.graph.nodes[0].id;
        let second = document.graph.nodes[1].id;
        let mut reordered = document.graph.clone();
        reordered.nodes.reverse();
        let original = document.replace_graph(reordered);
        assert_eq!(document.node_view(first).unwrap().id, first);
        let mut removed = document.graph.clone();
        removed.nodes.retain(|node| node.id != second);
        document.replace_graph(removed);
        assert!(document.node_view(second).is_none());
        document.replace_graph(original);
        assert_eq!(document.node_view(second).unwrap().id, second);
    }

    #[test]
    fn polling_a_busy_event_queue_yields_and_preserves_the_terminal_event() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        let text = host.bind(&graph()).unwrap();
        editor.open(&mut host, "events.json", text);
        editor.run(&mut host);
        let flow = editor.document.as_ref().unwrap().flow.as_ref().unwrap().0;
        editor.log.clear();
        let (sender, receiver) = std::sync::mpsc::channel();
        let receiver = orange_host::Events::from(receiver);
        for n in 0..MAX_EVENTS_PER_POLL {
            sender
                .send(orange_host::HostEvent {
                    flow,
                    kind: FlowEvent::TraceLost(n as u64),
                })
                .unwrap();
        }
        sender
            .send(orange_host::HostEvent {
                flow,
                kind: FlowEvent::Finished,
            })
            .unwrap();
        assert!(editor.take_events(&receiver));
        assert_eq!(editor.log.len(), MAX_EVENTS_PER_POLL);
        assert!(
            editor.is_running(),
            "terminal event remains queued after the batch"
        );
        assert!(editor.take_events(&receiver));
        assert!(!editor.is_running());
        assert!(!editor.take_events(&receiver));

        // Discarded events count too; an unrelated producer cannot monopolize a poll.
        let mut empty = Editor::new();
        for _ in 0..=MAX_EVENTS_PER_POLL {
            sender
                .send(orange_host::HostEvent {
                    flow,
                    kind: FlowEvent::Finished,
                })
                .unwrap();
        }
        assert!(empty.take_events(&receiver));
        assert!(empty.take_events(&receiver));
        assert!(!empty.take_events(&receiver));
    }

    #[test]
    fn the_palette_lists_the_hosts_nodes_by_category_and_narrows_with_a_search() {
        let (host, _events) = Host::new(HostOptions::default()).unwrap();
        let all = palette(&host, "");
        let count = |groups: &[(String, Vec<PaletteItem>)]| {
            groups.iter().map(|(_, items)| items.len()).sum::<usize>()
        };
        let offered = host
            .nodes()
            .filter(|info| !info.presentation.is_some_and(|shown| shown.hidden))
            .filter(|info| !orange_host::UNLISTED.contains(&info.definition.definition.id.as_str()))
            .count();
        let presets = all
            .iter()
            .flat_map(|(_, items)| items)
            .filter(|item| item.id.starts_with("preset:"))
            .count();
        assert_eq!(count(&all), offered + host.conversions().len() + presets);
        // Conversions are found by Spanish words too.
        assert!(
            palette(&host, "convertir entero texto")
                .iter()
                .flat_map(|(_, items)| items)
                .any(|item| item.id.starts_with(palette::CONVERSION))
        );
        let to_text = palette(&host, "whole number text");
        assert!(to_text.iter().flat_map(|(_, items)| items).any(|item| {
            item.id.starts_with(palette::CONVERSION) && item.label.contains("Text")
        }));
        let rank = |name: &str| match name {
            "Flow & triggers" => 0,
            "Data & text" => 1,
            "Math & logic" => 2,
            "Files & network" => 3,
            "Audio" => 4,
            "Utilities" => 5,
            _ => 6,
        };
        assert!(
            all.windows(2)
                .all(|pair| rank(&pair[0].0) <= rank(&pair[1].0))
        );
        // Every word must match, in any letter case, in the label, definition or category.
        let found = palette(&host, "MATH add");
        assert!(count(&found) >= 1 && count(&found) < count(&all));
        assert!(
            found
                .iter()
                .flat_map(|(_, items)| items)
                .any(|item| item.id == "org.orange.math/add" && item.package == "org.orange.math")
        );
        assert!(palette(&host, "no such node anywhere").is_empty());
    }

    /// Without Live trace, a run still updates what the editor shows but writes no
    /// line for each value; the Log keeps the run's lifecycle.
    #[test]
    fn a_run_without_live_trace_updates_values_but_logs_no_value_lines() {
        let (mut host, events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        let bound = host.bind(&graph()).unwrap();
        editor.open(&mut host, "graph.json", bound);
        editor.run(&mut host);
        let go = editor.graph().unwrap().nodes[0].id;
        editor.trigger(&host, go);
        let started = Instant::now();
        loop {
            let _ = editor.take_events(&events);
            let set = editor
                .document
                .as_ref()
                .and_then(|document| document.values.get(&id(100)))
                .is_some_and(|value| value.shown == "7");
            if set {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "the variable was set"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!editor.log().any(|line| line.contains("count =")));
        assert!(editor.log().count() > 0, "the run's start is still logged");
    }

    /// A message a graph logs again and again shares one line with a count.
    #[test]
    fn repeated_graph_messages_share_one_line() {
        let mut editor = Editor::new();
        editor.note(NoteLevel::Warning, "loud");
        editor.note(NoteLevel::Warning, "loud");
        editor.note(NoteLevel::Warning, "loud");
        editor.note(NoteLevel::Info, "quiet");
        let lines: Vec<_> = editor.log().collect();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines[0].contains("loud") && lines[0].contains('3'),
            "{lines:?}"
        );
        assert_eq!(lines[1], "quiet");
    }

    #[test]
    fn a_document_is_shown_checked_run_and_triggered_without_a_window() {
        let (mut host, events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        // Nothing is open: the editor still draws.
        assert!(draw(&mut editor, &mut host) > 0);

        // A hand-written document is not valid until it is bound.
        editor.open(&mut host, "graph.json", graph());
        let graph = editor.graph().unwrap().clone();
        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(
            (graph.nodes[0].name.as_str(), graph.nodes[0].entry),
            ("go", true)
        );
        assert_eq!(graph.nodes[1].name, "Set variable");
        // The trigger has one way out; the other node a way in, a value and a way on.
        let ports = |n: usize| -> Vec<(&str, bool, bool)> {
            graph.nodes[n]
                .ports
                .iter()
                .map(|port| (port.id.as_str(), port.input, port.exec))
                .collect()
        };
        assert_eq!(ports(0), [("then", false, true)]);
        assert!(ports(1).contains(&("in", true, true)));
        assert!(ports(1).contains(&("value", true, false)));
        let value = graph.nodes[1]
            .ports
            .iter()
            .find(|port| port.id == "value")
            .unwrap();
        assert_eq!(value.value.as_deref(), Some("7"));
        assert_eq!(graph.wires.len(), 1);
        assert!(graph.wires[0].exec);
        assert_eq!(editor.problems().len(), 1);
        assert!(editor.problems()[0].contains("— go"));
        // The canvas marks the node the problem is about, and the list can take you there.
        let go = editor.graph().unwrap().nodes[0].id;
        let mark = editor.canvas().marks().nodes.get(&go).cloned().unwrap();
        assert!(mark.error && mark.text.contains("— go"));
        editor.canvas().select(None);
        let graph = editor.graph().unwrap().clone();
        editor.canvas().focus(&graph, go);
        assert_eq!(editor.canvas().selected(), Some(go));
        editor.run(&mut host);
        assert!(!editor.is_running());

        let bound = host.bind(&self::graph()).unwrap();
        editor.open(&mut host, "graph.json", bound);
        assert!(editor.problems().is_empty());
        // Variable changes are written to the Log only while Live trace is on.
        editor.live_trace = true;
        editor.run(&mut host);
        assert!(editor.is_running());
        let go = editor.graph().unwrap().nodes[0].id;
        editor.trigger(&host, go);
        let started = Instant::now();
        while !editor.log().any(|line| line.ends_with("count = 7")) {
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "the variable was set"
            );
            let _ = editor.take_events(&events);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(editor.log().any(|line| line.ends_with("go: done")));
        // The canvas saw the trigger take its step.
        assert_eq!(editor.canvas().last_step(go), Some(false));
        // The running document draws, with more on screen than the empty editor.
        assert!(draw(&mut editor, &mut host) > 0);

        editor.stop(&mut host);
        assert!(!editor.is_running());
        assert_eq!(host.flows().count(), 0);
    }

    #[test]
    fn a_moved_node_is_kept_in_the_documents_layout_and_comes_back_when_opened() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let bound = host.bind(&graph()).unwrap();
        let mut editor = Editor::new();
        editor.open(&mut host, "graph.json", bound.clone());
        assert!(!editor.is_unsaved());
        let go = editor.graph().unwrap().nodes[0].id;
        editor.canvas().move_node(go, egui::Vec2::new(300.0, 120.0));
        editor.keep_layout();
        assert!(editor.is_unsaved());
        let text = editor.text().unwrap().to_owned();
        // Layout is not part of what the graph means: it still checks, and a flow of
        // the document before the move takes it as no change.
        assert!(host.check(&text).unwrap().is_empty());
        let flow = host.open(&bound).unwrap();
        assert!(!host.update(flow, &text).unwrap().changed);
        host.close(flow).unwrap();

        let mut again = Editor::new();
        again.open(&mut host, "graph.json", text);
        let placed = again.graph().unwrap().nodes[0].position.unwrap();
        assert_eq!(placed, [300.0, 120.0]);
        let places = again.canvas().places();
        assert_eq!(
            places.iter().find(|(node, _)| *node == go).unwrap().1,
            placed
        );
        // The node that was not moved kept its automatic place.
        assert!(again.graph().unwrap().nodes[1].position.is_some());
    }

    /// A document with only the variable `count`.
    fn blank() -> String {
        let mut saved: serde_json::Value = serde_json::from_str(&graph()).unwrap();
        saved["nodes"] = json!([]);
        saved["edges"] = json!([]);
        saved.to_string()
    }

    #[test]
    fn a_graph_is_built_edited_undone_and_run_from_the_editor() {
        let (mut host, events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open(&mut host, "new.json", blank());
        let trigger = editor.add_node(&mut host, "core/on-manual").unwrap();
        let join = editor
            .add_node(&mut host, "org.orange.text/concat")
            .unwrap();
        // The new node is selected and sits where the canvas is looking.
        assert_eq!(editor.canvas().selected(), Some(join));
        assert_eq!(editor.graph().unwrap().nodes.len(), 2);
        assert!(editor.is_unsaved());
        assert!(editor.set_input(&mut host, (join, "a"), Some("hello ")));
        assert!(editor.set_input(&mut host, (join, "b"), Some("there")));
        let value = |editor: &Editor, port: &str| {
            let graph = editor.graph().unwrap();
            let node = graph.nodes.iter().find(|node| node.id == join).unwrap();
            node.ports
                .iter()
                .find(|found| found.id == port)
                .unwrap()
                .value
                .clone()
        };
        assert_eq!(value(&editor, "b").as_deref(), Some("\"there\""));

        // A wire that cannot be is refused: the log says so and nothing changes.
        let before = editor.text().unwrap().to_owned();
        assert!(!editor.connect(&mut host, (trigger, "then"), (join, "a")));
        assert_eq!(editor.text().unwrap(), before);
        assert!(editor.log().any(|line| line.starts_with("not done:")));

        // Undo takes back one edit at a time; redo does it again.
        assert_eq!(editor.can_undo(), (true, false));
        editor.undo(&mut host);
        assert_eq!(value(&editor, "b"), None);
        assert_eq!(editor.can_undo(), (true, true));
        editor.redo(&mut host);
        assert_eq!(value(&editor, "b").as_deref(), Some("\"there\""));
        assert!(editor.set_input(&mut host, (join, "b"), None));
        assert_eq!(value(&editor, "b"), None);
        // A new edit after an undo leaves nothing to redo.
        editor.undo(&mut host);
        assert!(editor.remove_node(&mut host, join));
        assert!(!editor.can_undo().1);
        assert_eq!(editor.graph().unwrap().nodes.len(), 1);
        assert_eq!(editor.canvas().selected(), None);
        editor.undo(&mut host);
        assert_eq!(editor.graph().unwrap().nodes.len(), 2);

        // An edit made while the document runs reaches the flow: a second trigger
        // added while running can be fired.
        let set = editor.add_node(&mut host, "core/set-variable").unwrap();
        // A variable node names nothing until one is chosen.
        let declared = editor.declarations().unwrap().variables.clone();
        assert_eq!(declared.len(), 1);
        assert_eq!(declared[0].name, "count");
        assert!(!editor.problems().is_empty());
        let count = declared[0].id.clone().unwrap();
        assert!(editor.set_setting(&mut host, set, "/variable", Some(json!(count))));
        let node = editor
            .graph()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.id == set)
            .cloned();
        assert_eq!(node.unwrap().settings["variable"], json!(count));
        assert!(editor.set_input(&mut host, (set, "value"), Some("3")));
        assert!(editor.connect(&mut host, (trigger, "then"), (set, "in")));
        assert!(editor.problems().is_empty(), "{:?}", editor.problems());
        // Variable changes are written to the Log only while Live trace is on.
        editor.live_trace = true;
        editor.run(&mut host);
        assert!(editor.is_running());
        assert!(editor.set_input(&mut host, (set, "value"), Some("4")));
        editor.trigger(&host, trigger);
        let started = Instant::now();
        while !editor.log().any(|line| line.ends_with("count = 4")) {
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "the edited value ran"
            );
            let _ = editor.take_events(&events);
            std::thread::sleep(Duration::from_millis(5));
        }
        // The whole editor draws with a node selected and the properties shown.
        editor.canvas().select(Some(set));
        assert!(draw(&mut editor, &mut host) > 0);
        editor.stop(&mut host);
    }

    #[test]
    fn a_value_node_from_the_palette_is_typed_wired_and_run() {
        let (mut host, events) = Host::new(HostOptions::default()).unwrap();
        let found = palette(&host, "whole number");
        let item = found
            .iter()
            .flat_map(|(_, items)| items)
            .find(|item| item.label == "Int")
            .unwrap();
        assert_eq!(
            (item.id.as_str(), item.package.as_str()),
            ("core/whole-number", "")
        );
        let mut editor = Editor::new();
        editor.open(&mut host, "new.json", blank());
        let trigger = editor.add_node(&mut host, "core/on-manual").unwrap();
        let number = editor.add_node(&mut host, &item.id).unwrap();
        let set = editor.add_node(&mut host, "core/set-variable").unwrap();
        let count = editor.declarations().unwrap().variables[0].id.clone();
        assert!(editor.set_setting(&mut host, set, "/variable", Some(json!(count.unwrap()))));
        // The card offers a whole-number field for the value.
        let shown = editor.graph().unwrap().clone();
        let node = shown.nodes.iter().find(|node| node.id == number).unwrap();
        let port = node.ports.iter().find(|port| port.id == "value").unwrap();
        assert_eq!(port.ty.as_deref(), Some("core/i64@1"));
        assert!(editor.set_input(&mut host, (number, "value"), Some("12")));
        assert!(editor.connect(&mut host, (number, "result"), (set, "value")));
        assert!(editor.connect(&mut host, (trigger, "then"), (set, "in")));
        assert!(editor.problems().is_empty(), "{:?}", editor.problems());
        // Variable changes are written to the Log only while Live trace is on.
        editor.live_trace = true;
        editor.run(&mut host);
        editor.trigger(&host, trigger);
        let started = Instant::now();
        while !editor.log().any(|line| line.ends_with("count = 12")) {
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "the typed value ran"
            );
            let _ = editor.take_events(&events);
            std::thread::sleep(Duration::from_millis(5));
        }
        editor.stop(&mut host);
    }

    #[test]
    fn declarations_settings_and_types_are_changed_from_the_editor() {
        use orange_host::{Connection, ConnectionScope, VarScope, Variable};
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open(&mut host, "new.json", blank());
        let total = Variable {
            id: None,
            name: "total".into(),
            ty: "core/f64@1".into(),
            scope: VarScope::Workspace,
            persistent: true,
            initial: Some("0.5".into()),
        };
        let made = editor.declare_variable(&mut host, &total).unwrap();
        assert_eq!(editor.declarations().unwrap().variables.len(), 2);
        editor.undo(&mut host);
        assert_eq!(editor.declarations().unwrap().variables.len(), 1);
        editor.redo(&mut host);
        assert!(editor.remove_variable(&mut host, &made));

        // A secret typed into a connection is refused and never reaches the document.
        let mut api = Connection {
            id: None,
            name: "api".into(),
            adapter: "org.orange.net/http".into(),
            scope: ConnectionScope::Graph,
            settings: json!({"password": "hunter2"}),
        };
        assert!(editor.declare_connection(&mut host, &api).is_none());
        assert!(!editor.text().unwrap().contains("hunter2"));
        assert!(editor.log().any(|line| line.contains("credential://")));
        api.settings = json!({"password": "credential://api"});
        assert!(editor.declare_connection(&mut host, &api).is_some());

        // A timer's interval is a setting; a generic node's type is chosen.
        let timer = editor.add_node(&mut host, "core/on-timer").unwrap();
        assert!(editor.set_setting(&mut host, timer, "/interval", Some(json!("1000000"))));
        let collect = editor.add_node(&mut host, "core/collect").unwrap();
        assert!(editor.choose_type(&mut host, collect, "T", Some("core/i64@1")));
        let node = |editor: &Editor, id| {
            editor
                .graph()
                .unwrap()
                .nodes
                .iter()
                .find(|node| node.id == id)
                .cloned()
                .unwrap()
        };
        assert_eq!(node(&editor, timer).settings["interval"], "1000000");
        assert_eq!(
            node(&editor, collect).types[0].chosen.as_deref(),
            Some("core/i64@1")
        );
        // The panels draw the settings, the types and the Document tab.
        editor.canvas().select(Some(collect));
        assert!(draw(&mut editor, &mut host) > 0);
        editor.tab = Tab::Variables;
        assert!(draw(&mut editor, &mut host) > 0);
        editor.tab = Tab::Connections;
        assert!(draw(&mut editor, &mut host) > 0);
    }

    #[test]
    fn nodes_are_copied_pasted_dropped_and_removed_together() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open(&mut host, "new.json", blank());
        let trigger = editor.add_node(&mut host, "core/on-manual").unwrap();
        let set = editor
            .add_node_at(&mut host, "core/set-variable", [400.0, 0.0])
            .unwrap();
        assert!(editor.connect(&mut host, (trigger, "then"), (set, "in")));
        assert_eq!(editor.copy(&host), editor.copy(&host));
        editor.canvas().select_many(&[trigger, set]);
        let clip = editor.copy(&host).unwrap();
        let fresh = editor.paste(&mut host, &clip, [0.0, 600.0]);
        assert_eq!(fresh.len(), 2);
        let graph = editor.graph().unwrap();
        assert_eq!((graph.nodes.len(), graph.wires.len()), (4, 2));
        // What was pasted is selected, and one undo takes the paste back.
        assert_eq!(editor.canvas().selection().len(), 2);
        assert!(
            fresh
                .iter()
                .all(|node| editor.canvas().selection().contains(node))
        );
        editor.undo(&mut host);
        assert_eq!(editor.graph().unwrap().nodes.len(), 2);
        assert!(editor.paste(&mut host, "not nodes", [0.0, 0.0]).is_empty());
        // Removing several is one edit.
        assert!(editor.remove_nodes(&mut host, &[trigger, set]));
        assert!(editor.graph().unwrap().nodes.is_empty());
        editor.undo(&mut host);
        assert_eq!(editor.graph().unwrap().nodes.len(), 2);
        // A node kind dropped on the canvas is added where it was let go.
        let dropped = editor
            .add_node_at(&mut host, "core/on-manual", [50.0, 900.0])
            .unwrap();
        let placed = editor
            .graph()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.id == dropped);
        assert_eq!(placed.unwrap().position, Some([50.0, 900.0]));
    }

    #[test]
    fn save_as_moves_the_document_and_file_actions_go_to_the_application() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open(&mut host, "new.json", blank());
        let folder = std::env::temp_dir().join(format!("orange-save-as-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("moved.json").to_string_lossy().into_owned();
        assert!(editor.add_node(&mut host, "core/on-manual").is_some());
        editor.save_as(path.clone()).unwrap();
        assert_eq!(editor.path(), Some(path.as_str()));
        assert!(!editor.is_unsaved());
        let saved = std::fs::read_to_string(&path).unwrap();
        assert_eq!(host.view(&saved).unwrap().nodes.len(), 1);
        // A path that cannot be written keeps the one the document had.
        let nowhere = folder
            .join("missing")
            .join("x.json")
            .to_string_lossy()
            .into_owned();
        assert!(editor.save_as(nowhere).is_err());
        assert_eq!(editor.path(), Some(path.as_str()));
        std::fs::remove_dir_all(&folder).unwrap();

        // The toolbar and keys ask; the application takes the request once.
        assert_eq!(editor.take_request(), None);
        editor.ask(FileRequest::Open);
        assert_eq!(editor.take_request(), Some(FileRequest::Open));
        assert_eq!(editor.take_request(), None);
        editor.file_dialogs = true;
        assert!(draw(&mut editor, &mut host) > 0);
        // A new graph asks where to be saved the first time.
        editor.open_untitled(&mut host, blank());
        editor.save().unwrap();
        assert_eq!(editor.take_request(), Some(FileRequest::SaveAs));
        assert!(!std::path::Path::new("untitled.json").exists());
    }

    #[test]
    fn panels_can_be_switched_off_leaving_the_canvas() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        let bound = host.bind(&graph()).unwrap();
        editor.open(&mut host, "graph.json", bound);
        assert!(editor.panels.nodes && editor.panels.properties);
        editor.panels = Panels {
            nodes: false,
            properties: false,
            packages: false,
            log: false,
        };
        assert!(draw(&mut editor, &mut host) > 0);
        let alone = editor.canvas().size();
        editor.panels = Panels {
            nodes: true,
            packages: false,
            log: true,
            properties: true,
        };
        assert!(draw(&mut editor, &mut host) > 0);
        let with_panels = editor.canvas().size();
        assert!(alone[0] > with_panels[0] && alone[1] > with_panels[1]);
        editor.panels = Panels {
            nodes: false,
            properties: false,
            packages: false,
            log: false,
        };
        assert!(draw(&mut editor, &mut host) > 0);
        assert_eq!(editor.canvas().size(), alone);
    }

    /// Draws one frame in a window `size` points large.
    fn draw_at(editor: &mut Editor, host: &mut Host, size: [f32; 2]) -> usize {
        let context = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size[0], size[1]),
            )),
            ..Default::default()
        };
        let output = context.run_ui(input, |ui| editor.ui(ui, host));
        let shapes = output.shapes.len();
        output.drop_without_applying_deltas();
        shapes
    }

    #[test]
    fn diagnostics_tabs_fill_the_panel_and_switching_tabs_preserves_canvas_height() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        let bound = host.bind(&graph()).unwrap();
        editor.open(&mut host, "graph.json", bound);
        editor.panels.log = true;
        editor.say("A line in the log".into());
        let context = egui::Context::default();
        let mut draw = |editor: &mut Editor| {
            context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    ..Default::default()
                },
                |ui| editor.ui(ui, &mut host),
            )
        };
        // Font insertion takes effect on the next frame. Measure tab switching
        // after that initialization, with the same font metrics on both sides.
        draw(&mut editor).drop_without_applying_deltas();
        let output = draw(&mut editor);
        let mut group = egui::Rect::NOTHING;
        for clipped in &output.shapes {
            if let egui::Shape::Text(text) = &clipped.shape
                && matches!(text.galley.text(), "Problems" | "Log")
            {
                group = group.union(egui::Rect::from_min_size(text.pos, text.galley.size()));
            }
        }
        assert!(group.left() >= 8.0, "{group:?}");
        assert!(group.right() <= 1272.0, "{group:?}");
        output.drop_without_applying_deltas();
        let height = editor.canvas.size()[1];
        editor.log_events = true;
        draw(&mut editor).drop_without_applying_deltas();
        assert_eq!(editor.canvas.size()[1], height);
    }

    #[test]
    fn every_node_kind_of_the_host_is_added_with_ports_or_refused_with_a_reason() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let kinds: Vec<String> = host
            .nodes()
            .map(|info| info.definition.definition.id.as_str().to_owned())
            .collect();
        let mut editor = Editor::new();
        editor.open(&mut host, "all.json", blank());
        for kind in kinds.iter().map(String::as_str) {
            let before = editor.log().count();
            let node_count = editor.graph().unwrap().nodes.len();
            match editor.add_node(&mut host, kind) {
                Some(node) => {
                    let graph = editor.graph().unwrap();
                    let expected = if matches!(
                        kind,
                        "core/repeat-begin"
                            | "core/map-begin"
                            | "core/filter-begin"
                            | "core/fold-begin"
                    ) {
                        2
                    } else {
                        1
                    };
                    assert_eq!(
                        graph.nodes.len(),
                        node_count + expected,
                        "adding {kind} must create {expected} nodes",
                    );
                    let view = graph.nodes.iter().find(|found| found.id == node).unwrap();
                    // A node shows each port once.
                    let mut seen = std::collections::BTreeSet::new();
                    assert!(
                        view.ports
                            .iter()
                            .all(|port| seen.insert((port.input, &port.id))),
                        "{kind}: {:?}",
                        view.ports
                    );
                    // Only a variable node that names no variable yet has no ports.
                    assert!(
                        !view.ports.is_empty() || !view.references.is_empty(),
                        "{kind} has no ports"
                    );
                }
                // The base host declares no record or enum type; the log says why.
                None => assert!(
                    editor
                        .log()
                        .skip(before)
                        .any(|line| line.contains("declare one in the Types tab")),
                    "{kind} was refused without the reason"
                ),
            }
        }
        // The whole graph of every kind draws, in a small window too.
        assert!(draw(&mut editor, &mut host) > 0);
        assert!(draw_at(&mut editor, &mut host, [480.0, 320.0]) > 0);
    }

    #[test]
    fn a_variable_is_declared_where_a_node_needs_it_and_used_from_the_list() {
        use orange_host::{VarScope, Variable};
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open(&mut host, "new.json", blank());
        // A node that needs a variable declares one from its properties.
        let set = editor.add_node(&mut host, "core/set-variable").unwrap();
        let total = Variable {
            id: None,
            name: "total".into(),
            ty: "core/f64@1".into(),
            scope: VarScope::Run,
            persistent: false,
            initial: None,
        };
        let id = editor
            .declare_variable_for(&mut host, set, "/variable", &total)
            .unwrap();
        let node = |editor: &Editor, node| {
            editor
                .graph()
                .unwrap()
                .nodes
                .iter()
                .find(|found| found.id == node)
                .cloned()
                .unwrap()
        };
        assert_eq!(node(&editor, set).settings["variable"], json!(id));
        assert_eq!(node(&editor, set).detail.as_deref(), Some("total"));
        assert_eq!(editor.canvas().selected(), Some(set));
        assert!(
            node(&editor, set)
                .ports
                .iter()
                .any(|port| port.id == "value")
        );
        // The Variables list adds a node already naming the variable.
        let get = editor
            .add_variable_node(&mut host, "core/get-variable", &id)
            .unwrap();
        assert_eq!(node(&editor, get).detail.as_deref(), Some("total"));
        assert!(!node(&editor, get).ports.is_empty());
        // The toolbar opens the new-variable form under Variables.
        editor.tab = Tab::Nodes;
        editor.new_variable_form();
        assert!(editor.tab == Tab::Variables && editor.panels.nodes);
        assert!(draw(&mut editor, &mut host) > 0);
        // A node uses the same form on the Variables tab.
        editor.tab = Tab::Nodes;
        editor.new_variable_for_node(set, "/variable");
        assert!(editor.tab == Tab::Variables && editor.panels.nodes);
        assert!(draw_at(&mut editor, &mut host, [520.0, 360.0]) > 0);
    }

    #[test]
    fn condition_groups_open_with_two_inputs_and_allow_more() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open(&mut host, "new.json", blank());
        for definition in ["core/all-conditions", "core/any-conditions"] {
            let id = editor.add_node(&mut host, definition).unwrap();
            let conditions = |editor: &Editor| {
                let node = editor
                    .graph()
                    .unwrap()
                    .nodes
                    .iter()
                    .find(|node| node.id == id)
                    .unwrap();
                node.ports
                    .iter()
                    .filter(|port| port.input)
                    .map(|port| port.id.clone())
                    .collect::<Vec<_>>()
            };
            assert_eq!(conditions(&editor), ["condition#0", "condition#1"]);
            assert_eq!(
                editor.add_port(&mut host, id, "condition").as_deref(),
                Some("condition#2")
            );
            assert_eq!(
                conditions(&editor),
                ["condition#0", "condition#1", "condition#2"]
            );
            assert!(editor.remove_port(&mut host, (id, "condition#2")));
            assert!(!editor.remove_port(&mut host, (id, "condition#1")));
        }
    }

    #[test]
    fn branches_are_added_and_taken_off_from_the_editor() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open(&mut host, "new.json", blank());
        let fork = editor.add_node(&mut host, "core/fork").unwrap();
        // The variadic count and the branch ports the card shows. The authoring view also
        // offers the fork's failure route (`failed`, `error`), which adding or removing a
        // branch leaves alone.
        let branches = |editor: &Editor| {
            let graph = editor.graph().unwrap();
            let node = graph.nodes.iter().find(|found| found.id == fork).unwrap();
            let shown = node
                .ports
                .iter()
                .filter(|port| port.id.starts_with("branch#"))
                .count();
            let others: Vec<&str> = node
                .ports
                .iter()
                .filter(|port| !port.id.starts_with("branch#"))
                .map(|port| port.id.as_str())
                .collect();
            assert_eq!(others, ["in", "completed", "failed", "error"]);
            (node.variadic[0].count, shown)
        };
        assert_eq!(branches(&editor), (1, 1));
        assert_eq!(
            editor.add_port(&mut host, fork, "branch").as_deref(),
            Some("branch#1")
        );
        assert_eq!(branches(&editor), (2, 2));
        assert!(editor.remove_port(&mut host, (fork, "branch#1")));
        assert_eq!(branches(&editor), (1, 1));
        // The last one a fork needs stays, and the log says why.
        assert!(!editor.remove_port(&mut host, (fork, "branch#0")));
        assert!(editor.log().any(|line| line.contains("at least 1")));
        editor.canvas().select(Some(fork));
        assert!(draw(&mut editor, &mut host) > 0);
    }

    #[test]
    fn ports_and_settings_are_named_for_people() {
        assert_eq!(port_label("branch#0"), "branch 1");
        assert_eq!(port_label("max_items"), "max items");
        assert_eq!(port_label("odd#x"), "odd#x");
        assert_eq!(properties::words("max_iterations"), "Max iterations");
    }

    #[test]
    fn types_are_named_for_people() {
        assert_eq!(type_name("core/string@1"), "Text");
        assert_eq!(type_name("core/i64@1"), "Int");
        assert_eq!(type_name("core/f64@1"), "Float");
        assert_eq!(type_name("core/bool@1"), "Bool");
        assert_eq!(type_name("org.orange.audio/stream@1"), "Audio stream");
        assert_eq!(exact_type("core/u64@1"), "u64");
        assert_eq!(type_name("core/list@1<core/i64@1>"), "list<i64>");
        assert_eq!(type_name("org.example.geo/point@1"), "Geo point");
        assert_eq!(
            type_name("org.example.geo/bounding-box@1"),
            "Geo bounding box"
        );
    }

    #[test]
    fn the_log_keeps_only_its_newest_lines() {
        let mut editor = Editor::new();
        for n in 0..MAX_LOG_LINES + 10 {
            editor.say(format!("line {n}"));
        }
        assert_eq!(editor.log().count(), MAX_LOG_LINES);
        assert_eq!(editor.log().next(), Some("line 10"));
    }
}

#[cfg(test)]
mod quick_add_tests {
    use super::*;
    use orange_host::HostOptions;

    #[test]
    fn numeric_wire_search_offers_comparisons_and_unit_converters() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open(
            &mut host,
            "graph.json",
            orange_host::new_document().unwrap(),
        );
        let number = editor
            .add_node_at(&mut host, "core/number", [0.0, 0.0])
            .unwrap();
        editor.open_quick_add(
            Some((number, "result".into())),
            [300.0, 0.0],
            egui::Pos2::ZERO,
        );
        // A decimal has an order but no exact equality (11 §5.3): Greater takes it,
        // Equal does not.
        assert!(editor.quick_choices(&host, "greater").iter().any(|choice| {
            choice.definition == "org.orange.math/greater" && choice.input.as_deref() == Some("a")
        }));
        assert!(
            editor
                .quick_choices(&host, "equal")
                .iter()
                .all(|choice| choice.definition != "org.orange.math/equal")
        );
        let choices = editor.quick_choices(&host, "duration milliseconds");
        let conversion = choices
            .iter()
            .find(|choice| choice.definition == "convert:core/f64-to-duration?unit=milliseconds")
            .unwrap();
        let node = editor.pick_quick(&mut host, conversion).unwrap();
        assert!(
            editor
                .graph()
                .unwrap()
                .wires
                .iter()
                .any(|wire| { wire.from.0 == number && wire.to.0 == node && wire.to.1 == "value" })
        );
    }

    #[test]
    fn a_wire_let_go_on_the_canvas_offers_the_kinds_that_take_it_and_wires_the_choice() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        let text = host.bind(&tests::graph()).unwrap();
        editor.open(&mut host, "graph.json", text);
        let go = editor.graph().unwrap().nodes[0].id;
        editor.open_quick_add(Some((go, "then".into())), [600.0, 0.0], egui::Pos2::ZERO);
        let all = editor.quick_choices(&host, "");
        assert!(all.iter().all(|choice| choice.input.is_some()));
        let sequence = editor.quick_choices(&host, "seque");
        assert_eq!(sequence.len(), 1, "{sequence:?}");
        let node = editor.pick_quick(&mut host, &sequence[0]).unwrap();
        assert!(!editor.is_quick_add_open());
        let graph = editor.graph().unwrap();
        assert!(
            graph
                .wires
                .iter()
                .any(|wire| wire.from == (go, "then".to_owned()) && wire.to.0 == node)
        );
        assert_eq!(
            graph
                .nodes
                .iter()
                .find(|found| found.id == node)
                .unwrap()
                .position,
            Some([600.0, 0.0])
        );
        // Placing and wiring were one action: one step back takes off both.
        editor.undo(&mut host);
        assert!(
            editor
                .graph()
                .unwrap()
                .wires
                .iter()
                .all(|wire| wire.to.0 != node)
        );
        assert!(
            !editor
                .graph()
                .unwrap()
                .nodes
                .iter()
                .any(|found| found.id == node)
        );

        // Without a wire, every kind is offered and nothing is wired.
        editor.open_quick_add(None, [0.0, 300.0], egui::Pos2::ZERO);
        let every = editor.quick_choices(&host, "");
        assert!(every.len() > sequence.len() && every.iter().all(|choice| choice.input.is_none()));
    }
}

#[cfg(test)]
mod glyph_tests {
    /// Every symbol the editor draws on its own, as a button or mark.
    pub(crate) const SYMBOLS: &[char] = &['×', '—', '…', '!', '+', '·', '−', '“', '”', 'µ'];

    #[test]
    fn every_symbol_the_editor_draws_is_in_its_fonts() {
        let context = egui::Context::default();
        let output = context.run_ui(egui::RawInput::default(), |_| {});
        output.drop_without_applying_deltas();
        let missing: Vec<char> = context.fonts_mut(|fonts| {
            SYMBOLS
                .iter()
                .copied()
                .filter(|symbol| !fonts.has_glyph(&egui::FontId::proportional(14.0), *symbol))
                .collect()
        });
        assert!(missing.is_empty(), "drawn as boxes: {missing:?}");
    }

    #[test]
    fn every_icon_is_in_the_bundled_font_and_ordinary_text_is_not_taken_by_it() {
        let context = egui::Context::default();
        crate::icons::install(&context);
        let output = context.run_ui(egui::RawInput::default(), |_| {});
        output.drop_without_applying_deltas();
        let icons = egui::FontId::new(14.0, crate::icons::family());
        let text = egui::FontId::proportional(14.0);
        context.fonts_mut(|fonts| {
            let missing: Vec<char> = crate::icons::ALL
                .iter()
                .copied()
                .filter(|icon| !fonts.has_glyph(&icons, *icon))
                .collect();
            assert!(missing.is_empty(), "icons drawn as boxes: {missing:?}");
            // Icons in ordinary text find the icon font too.
            assert!(
                crate::icons::ALL
                    .iter()
                    .all(|icon| fonts.has_glyph(&text, *icon))
            );
        });
        // The icons are distinct, and installing again changes nothing.
        let mut seen = crate::icons::ALL.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), crate::icons::ALL.len());
        let installed = context.fonts(|fonts| fonts.definitions().font_data.len());
        // Material and Phosphor are fonts. Lucide is rendered from SVG assets.
        assert_eq!(
            installed,
            egui::FontDefinitions::default().font_data.len() + 2
        );
        crate::icons::install(&context);
        context
            .run_ui(Default::default(), |_| {})
            .drop_without_applying_deltas();
        assert_eq!(
            context.fonts(|fonts| fonts.definitions().font_data.len()),
            installed
        );
    }
}

#[cfg(test)]
mod run_tests {
    use super::tests::graph;
    use super::*;
    use orange_host::HostOptions;

    fn id(n: u32) -> String {
        format!("10000000-0000-4000-8000-{n:012x}")
    }

    /// Where the text `text` is drawn in `shapes`, the rightmost when it is drawn more
    /// than once (the properties panel is on the right), else the leftmost.
    fn drawn_at(shapes: &[egui::epaint::ClippedShape], text: &str) -> Option<egui::Pos2> {
        drawn_where(shapes, text, true)
    }

    fn drawn_where(
        shapes: &[egui::epaint::ClippedShape],
        text: &str,
        rightmost: bool,
    ) -> Option<egui::Pos2> {
        let mut found: Vec<egui::Pos2> = Vec::new();
        for clipped in shapes {
            let mut stack = vec![&clipped.shape];
            while let Some(shape) = stack.pop() {
                match shape {
                    egui::Shape::Text(drawn) if drawn.galley.text() == text => {
                        found.push(drawn.visual_bounding_rect().center());
                    }
                    egui::Shape::Vec(inner) => stack.extend(inner.iter()),
                    _ => {}
                }
            }
        }
        if rightmost {
            found.into_iter().max_by(|a, b| a.x.total_cmp(&b.x))
        } else {
            found.into_iter().min_by(|a, b| a.x.total_cmp(&b.x))
        }
    }

    #[test]
    fn one_click_on_run_writes_the_field_being_typed_in_and_starts_the_run() {
        typed_then_run(true);
    }

    #[test]
    fn one_click_on_run_writes_a_value_being_typed_on_a_card_and_starts_the_run() {
        typed_then_run(false);
    }

    #[test]
    fn clicking_a_request_output_opens_the_inspector_and_preserves_the_complete_body() {
        let (mut host, _) =
            Host::with_services(HostOptions::default(), vec![orange_net::package()]).unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        editor.panels.nodes = false;
        editor.panels.properties = false;
        editor.panels.log = false;
        let request = editor
            .add_node(&mut host, orange_net::HTTP_REQUEST)
            .unwrap();
        let body = format!("{}\nlast line", "a".repeat(orange_host::MAX_INSPECT_CHARS));
        let value = orange_host::Value::Str(std::sync::Arc::new(body.clone()));
        editor.canvas.show_observed_value(
            request,
            "response".into(),
            canvas::Shown {
                short: "Response preview".into(),
                full: orange_host::inspect(&value),
            },
            &value,
            false,
        );
        let context = egui::Context::default();
        let frame = |editor: &mut Editor, host: &mut Host, events| {
            let output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100.0, 760.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| editor.ui(ui, host),
            );
            let shapes = output.shapes.clone();
            output.drop_without_applying_deltas();
            shapes
        };
        let shapes = frame(&mut editor, &mut host, vec![]);
        let at = drawn_at(&shapes, "response").expect("response label");
        let _ = frame(&mut editor, &mut host, vec![egui::Event::PointerMoved(at)]);
        for pressed in [true, false] {
            let _ = frame(
                &mut editor,
                &mut host,
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        assert_eq!(
            editor.inspected_port,
            Some((request, "response".into(), false))
        );
        let graph = editor.graph().unwrap();
        let preview = editor
            .canvas
            .port_value(graph, request, "response", false)
            .unwrap();
        assert!(!preview.full.contains("last line"));
        assert_eq!(
            editor
                .canvas
                .copyable_port_text(graph, request, "response", false),
            Some(body.as_str()),
        );
    }

    #[test]
    fn the_method_picker_writes_a_method_on_every_http_request_variant() {
        let (mut host, _) =
            Host::with_services(HostOptions::default(), vec![orange_net::package()]).unwrap();
        for definition in [
            orange_net::HTTP_REQUEST,
            orange_net::HTTP_REQUEST_BYTES,
            orange_net::HTTP_STREAM,
        ] {
            let mut editor = Editor::new();
            editor.open_untitled(&mut host, orange_host::new_document().unwrap());
            let node = editor.add_node(&mut host, definition).unwrap();
            let context = egui::Context::default();
            let mut frame = |events| {
                let output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(420.0, 850.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| editor.properties_panel(ui, &mut host),
                );
                let shapes = output.shapes.clone();
                output.drop_without_applying_deltas();
                shapes
            };
            let click = |at| {
                [true, false].map(|pressed| {
                    vec![
                        egui::Event::PointerMoved(at),
                        egui::Event::PointerButton {
                            pos: at,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                })
            };
            let get = drawn_at(&frame(vec![]), "GET").expect("method control");
            for events in click(get) {
                let _ = frame(events);
            }
            let post = drawn_at(&frame(vec![]), "POST").expect("method menu choice");
            for events in click(post) {
                let _ = frame(events);
            }
            let method = editor.port_view(node, "method", true).unwrap();
            assert_eq!(Editor::shown_input(method), "POST");
        }
    }

    /// Types 9 after the 7 of the set-variable node, in the properties panel or on its
    /// card, then clicks Run once.
    fn typed_then_run(panel: bool) {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        let text = host.bind(&graph()).unwrap();
        editor.open(&mut host, "typed.json", text);
        editor.panels.nodes = false;
        editor.panels.log = false;
        editor.panels.properties = panel;
        editor.canvas.select(Some(id(2).parse().unwrap()));
        let context = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
        let frame = |editor: &mut Editor, host: &mut Host, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let output = context.run_ui(input, |ui| editor.ui(ui, host));
            let shapes = output.shapes.clone();
            output.drop_without_applying_deltas();
            shapes
        };
        let click = |at: egui::Pos2| {
            [
                vec![egui::Event::PointerMoved(at)],
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            ]
        };
        let shapes = frame(&mut editor, &mut host, Vec::new());
        let field = drawn_where(&shapes, "7", panel).expect("the value field shows 7");
        // A card's whole number is typed after a double click.
        let clicks = if panel { 1 } else { 2 };
        for _ in 0..clicks {
            for events in click(field) {
                let _ = frame(&mut editor, &mut host, events);
            }
        }
        let _ = frame(&mut editor, &mut host, Vec::new());
        assert!(
            context.egui_wants_keyboard_input(),
            "the field has the keys"
        );
        let shapes = frame(
            &mut editor,
            &mut host,
            vec![
                egui::Event::Key {
                    key: egui::Key::End,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::Text("9".into()),
            ],
        );
        // Typed, not yet written.
        assert!(!editor.document.as_ref().unwrap().unsaved);
        // A quick click: the system hands over the press and the release in one frame.
        let run = drawn_at(&shapes, &icons::RUN.to_string()).expect("the toolbar shows Run");
        let _ = frame(&mut editor, &mut host, click(run).concat());
        let document = editor.document.as_ref().unwrap();
        assert!(document.flow.is_some(), "one click started the run");
        let saved = document.draft.text().unwrap();
        assert!(saved.contains("\"79\""), "the typed value was written");
        // And the run has it: the entry sets `count` to what was typed, not to 7.
        let flow = document.flow.as_ref().unwrap().0;
        host.trigger(flow, id(1).parse().unwrap(), None).unwrap();
        let count: orange_host::VariableId = id(100).parse().unwrap();
        let started = std::time::Instant::now();
        let mut value = None;
        while value.is_none() && started.elapsed() < std::time::Duration::from_secs(10) {
            value = host
                .inspect(flow, move |run| run.variable(count).cloned())
                .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(value, Some(orange_host::Value::I64(79)));
    }
}

impl Editor {
    /// While nothing runs, shows beside each output of a pure node (a value, an
    /// operator) what it computes from what is wired into it, without running the
    /// graph: Greater wired to two Ints shows true or false as soon as it is wired.
    /// Outputs that depend on a trigger, a variable or an effect are left blank.
    pub fn preview_values(&mut self, host: &mut Host) {
        self.preview_scope(host, None);
    }

    /// Previews the outputs of `scope`'s nodes only, keeping every other preview, or
    /// of the whole graph without a scope.
    fn preview_scope(
        &mut self,
        host: &mut Host,
        scope: Option<&std::collections::BTreeSet<NodeId>>,
    ) {
        let Some(document) = &self.document else {
            return;
        };
        let wanted: Vec<(NodeId, orange_host::PortKey)> = document
            .graph
            .nodes
            .iter()
            // A pure generic kernel still needs a concrete type before it can run.
            // Skip unbound cards rather than retrying their previews on every edit.
            .filter(|node| scope.is_none_or(|scope| scope.contains(&node.id)))
            .filter(|node| {
                !node.entry
                    && node.missing.is_none()
                    && node.ports.iter().all(|port| {
                        !port.exec && port.ty.as_deref().is_none_or(|ty| !ty.contains('$'))
                    })
            })
            .flat_map(|node| {
                node.ports
                    .iter()
                    .filter(|port| !port.input)
                    .filter_map(move |port| Some((node.id, port.id.parse().ok()?)))
            })
            .collect();
        match scope {
            Some(scope) => self.canvas.clear_previews_of(scope),
            None if document.flow.is_none() => self.canvas.clear_run(),
            None => self.canvas.clear_previews(),
        }
        if wanted.is_empty() {
            return;
        }
        // The whole graph is tried in one batch; a scope previews only its own nodes
        // and what they read, never compiling the rest. Unrelated effects or invalid
        // nodes can make the complete document unsuitable, even when a pure branch is
        // ready, so a batch that fails is split.
        let whole = if scope.is_none() {
            let Ok(text) = document.draft.text() else {
                return;
            };
            host.preview(&text, &wanted).ok()
        } else {
            None
        };
        let found = match whole {
            Some(found) => found,
            None => {
                let mut upstream = HashMap::<NodeId, Vec<NodeId>>::new();
                for wire in &document.graph.wires {
                    upstream.entry(wire.to.0).or_default().push(wire.from.0);
                }
                // Only what the wanted outputs read is written out; batches are cut
                // again from that.
                let needed = dependencies(&upstream, wanted.iter().map(|(node, _)| *node));
                let Ok(text) = document.draft.text_of(|node| needed.contains(&node)) else {
                    return;
                };
                let Ok(saved) = serde_json::from_str(&text) else {
                    return;
                };
                let mut found = Vec::new();
                let mut batches = vec![wanted.as_slice()];
                while let Some(batch) = batches.pop() {
                    let scoped = preview_dependencies(
                        &saved,
                        &upstream,
                        batch.iter().map(|(node, _)| *node),
                    );
                    if let Some(values) =
                        scoped.and_then(|graph| host.preview(&graph.to_string(), batch).ok())
                    {
                        found.extend(values);
                    } else if batch.len() > 1 {
                        let (left, right) = batch.split_at(batch.len() / 2);
                        batches.extend([right, left]);
                    }
                }
                found
            }
        };
        for previewed in found {
            if let Ok(value) = &previewed.value {
                let port = previewed.port.to_string();
                // The same text a running flow shows: enum choices by name.
                let ty = self
                    .document
                    .as_ref()
                    .and_then(|document| {
                        document
                            .graph
                            .nodes
                            .iter()
                            .find(|node| node.id == previewed.node)
                    })
                    .and_then(|node| node.ports.iter().find(|p| !p.input && p.id == port))
                    .and_then(|port| port.ty.as_deref());
                let (short, full) = presented_value(&self.canvas.enums, ty, value)
                    .unwrap_or_else(|| (orange_host::plain(value), orange_host::inspect(value)));
                self.canvas.show_observed_value(
                    previewed.node,
                    port,
                    canvas::Shown { short, full },
                    value,
                    true,
                );
            }
        }
    }
}

/// Each node's incoming wires.
fn incoming(graph: &GraphView) -> HashMap<NodeId, Vec<&orange_host::WireView>> {
    let mut wires: HashMap<NodeId, Vec<&orange_host::WireView>> = HashMap::new();
    for wire in &graph.wires {
        wires.entry(wire.to.0).or_default().push(wire);
    }
    wires
}

/// The nodes an edit may have changed the preview of: those whose card or incoming
/// wires changed, and every node downstream of them. `None` when the graph was not
/// shown before, so everything is previewed.
fn touched_by_edit(
    before: &GraphView,
    after: &GraphView,
) -> Option<std::collections::BTreeSet<NodeId>> {
    use std::collections::{BTreeSet, HashMap};
    if before.nodes.is_empty() {
        return None;
    }
    let old: HashMap<NodeId, &std::sync::Arc<orange_host::NodeView>> =
        before.nodes.iter().map(|node| (node.id, node)).collect();
    let (old_in, new_in) = (incoming(before), incoming(after));
    let mut touched: BTreeSet<NodeId> = after
        .nodes
        .iter()
        .filter(|node| {
            // A card the view kept is the same one; only one built again is compared.
            old.get(&node.id)
                .is_none_or(|old| !std::sync::Arc::ptr_eq(old, node) && **old != **node)
                || old_in.get(&node.id) != new_in.get(&node.id)
        })
        .map(|node| node.id)
        .collect();
    // What reads a touched node is touched too.
    let mut outgoing: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for wire in &after.wires {
        outgoing.entry(wire.from.0).or_default().push(wire.to.0);
    }
    let mut pending: Vec<NodeId> = touched.iter().copied().collect();
    while let Some(node) = pending.pop() {
        for next in outgoing.get(&node).into_iter().flatten() {
            if touched.insert(*next) {
                pending.push(*next);
            }
        }
    }
    Some(touched)
}

/// Retain the requested root nodes and their upstream dependencies. The host still
/// validates and compiles the projection, including the original providers and types.
fn preview_dependencies(
    saved: &serde_json::Value,
    upstream: &HashMap<NodeId, Vec<NodeId>>,
    outputs: impl Iterator<Item = NodeId>,
) -> Option<serde_json::Value> {
    let mut scoped = saved.clone();
    keep_dependencies(&mut scoped, upstream, outputs).then_some(scoped)
}

/// The `outputs` nodes and every node they read, through any number of wires.
fn dependencies(
    upstream: &HashMap<NodeId, Vec<NodeId>>,
    outputs: impl Iterator<Item = NodeId>,
) -> std::collections::BTreeSet<NodeId> {
    let mut nodes = std::collections::BTreeSet::new();
    let mut pending: Vec<_> = outputs.collect();
    while let Some(node) = pending.pop() {
        if nodes.insert(node)
            && let Some(sources) = upstream.get(&node)
        {
            pending.extend(sources);
        }
    }
    nodes
}

/// Cuts `document` down, in place, to the `outputs` nodes and what they read. Returns
/// false when it has no node or edge list.
fn keep_dependencies(
    document: &mut serde_json::Value,
    upstream: &HashMap<NodeId, Vec<NodeId>>,
    outputs: impl Iterator<Item = NodeId>,
) -> bool {
    let nodes = dependencies(upstream, outputs);
    let contains = |id: &serde_json::Value| {
        id.as_str()
            .and_then(|id| id.parse::<NodeId>().ok())
            .is_some_and(|id| nodes.contains(&id))
    };
    let Some(kept) = document["nodes"].as_array_mut() else {
        return false;
    };
    kept.retain(|node| contains(&node["id"]));
    let Some(edges) = document["edges"].as_array_mut() else {
        return false;
    };
    edges.retain(|edge| contains(&edge["from"]["node"]) && contains(&edge["to"]["node"]));
    true
}

#[cfg(test)]
mod preview_tests {
    use super::*;
    use orange_host::HostOptions;

    #[test]
    fn greater_compares_every_number_kind() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        // Greater takes whole numbers and decimals alike.
        for (definition, valid, invalid) in [
            ("org.orange.math/greater", "core/i64@1", None),
            ("org.orange.math/greater", "core/f64@1", None),
        ] {
            let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
            let node = host.add_node(&mut draft, definition, [0.0, 0.0]).unwrap();
            host.choose_type(&mut draft, node, "T", Some(valid))
                .unwrap();
            host.set_input(&mut draft, (node, "a"), Some("5")).unwrap();
            host.set_input(&mut draft, (node, "b"), Some("3")).unwrap();
            let wanted = [(node, "result".parse().unwrap())];
            let values = host.preview(&draft.text().unwrap(), &wanted).unwrap();
            assert_eq!(values[0].value, Ok(orange_host::Value::Bool(true)));
            let Some(invalid) = invalid else {
                continue;
            };
            let mut invalid_draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
            let node = host
                .add_node(&mut invalid_draft, definition, [0.0, 0.0])
                .unwrap();
            if host
                .choose_type(&mut invalid_draft, node, "T", Some(invalid))
                .is_ok()
            {
                assert!(
                    host.problems(&mut invalid_draft)
                        .unwrap()
                        .iter()
                        .any(|problem| { problem.code.as_str() == "E-CONSTRAINT-UNSATISFIED" })
                );
            }
        }
    }

    #[test]
    fn a_long_mixed_operator_chain_previews_past_the_old_output_limit() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let text = host.bind(&tests::graph()).unwrap();
        let mut draft = host.draft(&text).unwrap();
        let left = host
            .add_node(&mut draft, "core/whole-number", [0.0, 0.0])
            .unwrap();
        let source = host
            .add_node(&mut draft, "core/whole-number", [0.0, 0.0])
            .unwrap();
        assert!(
            host.set_input(&mut draft, (left, "value"), Some("12"))
                .unwrap()
        );
        assert!(
            host.set_input(&mut draft, (source, "value"), Some("-18"))
                .unwrap()
        );
        let mut previous = source;
        let mut chain = Vec::new();
        for definition in std::iter::repeat_n("org.orange.math/absolute", 70)
            .chain(["org.orange.math/negate", "org.orange.math/absolute"])
        {
            let next = host.add_node(&mut draft, definition, [0.0, 0.0]).unwrap();
            assert!(
                host.connect(&mut draft, (previous, "result"), (next, "value"))
                    .unwrap()
            );
            chain.push(next);
            previous = next;
        }
        let greater = host
            .add_node(&mut draft, "org.orange.math/greater", [0.0, 0.0])
            .unwrap();
        assert!(
            host.connect(&mut draft, (left, "result"), (greater, "a"))
                .unwrap()
        );
        assert!(
            host.connect(&mut draft, (previous, "result"), (greater, "b"))
                .unwrap()
        );

        let mut editor = Editor::new();
        editor.open(&mut host, "long-chain.json", draft.text().unwrap());
        for (index, node) in chain.iter().enumerate() {
            let expected = if index == 70 { "-18" } else { "18" };
            assert_eq!(editor.canvas().shown_value(*node, "result"), Some(expected));
        }
        assert_eq!(
            editor.canvas().shown_value(greater, "result"),
            Some("false")
        );
        assert!(editor.set_input(&mut host, (source, "value"), Some("-8")));
        assert_eq!(editor.canvas().shown_value(previous, "result"), Some("8"));
        assert_eq!(editor.canvas().shown_value(greater, "result"), Some("true"));
    }

    #[test]
    fn an_absolute_between_int_and_greater_keeps_live_values() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        let activity_graph = host.bind(&tests::graph()).unwrap();
        editor.open(&mut host, "graph.json", activity_graph);
        let a = editor.add_node(&mut host, "core/whole-number").unwrap();
        let b = editor.add_node(&mut host, "core/whole-number").unwrap();
        let greater = editor
            .add_node(&mut host, "org.orange.math/greater")
            .unwrap();
        assert!(editor.set_input(&mut host, (a, "value"), Some("12")));
        assert!(editor.set_input(&mut host, (b, "value"), Some("-18")));
        assert!(editor.connect(&mut host, (a, "result"), (greater, "a")));
        assert!(editor.connect(&mut host, (b, "result"), (greater, "b")));
        let absolute = editor
            .add_node_into_wire(
                &mut host,
                "org.orange.math/absolute",
                [50.0, 100.0],
                &((b, "result".into()), (greater, "b".into())),
            )
            .unwrap();
        let text = editor.text().unwrap();
        let expected = [(a, "12"), (b, "-18"), (absolute, "18"), (greater, "false")];
        for (node, value) in expected {
            assert_eq!(
                editor.canvas().shown_value(node, "result"),
                Some(value),
                "{:?}",
                host.preview(&text, &[(node, "result".parse().unwrap())])
            );
        }
    }

    #[test]
    fn an_operator_shows_its_result_while_it_is_wired_without_running() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        let text = orange_host::new_document().unwrap();
        editor.open(&mut host, "graph.json", text);
        let a = editor
            .add_node_at(&mut host, "core/whole-number", [0.0, 0.0])
            .unwrap();
        let b = editor
            .add_node_at(&mut host, "core/whole-number", [0.0, 120.0])
            .unwrap();
        let greater = editor
            .add_node_at(&mut host, "org.orange.math/greater", [300.0, 60.0])
            .unwrap();
        assert!(editor.set_input(&mut host, (a, "value"), Some("12")));
        assert!(editor.set_input(&mut host, (b, "value"), Some("7")));
        assert!(editor.connect(&mut host, (a, "result"), (greater, "a")));
        assert!(editor.connect(&mut host, (b, "result"), (greater, "b")));
        assert_eq!(editor.canvas().shown_value(greater, "result"), Some("true"));
        assert_eq!(editor.canvas().shown_value(a, "result"), Some("12"));
        // What Greater receives shows on its inputs too.
        let graph = editor.graph().unwrap().clone();
        assert_eq!(
            editor
                .canvas()
                .port_value(&graph, greater, "b", true)
                .map(|v| v.short.clone()),
            Some("7".to_owned())
        );
        assert!(editor.set_input(&mut host, (b, "value"), Some("30")));
        assert_eq!(
            editor.canvas().shown_value(greater, "result"),
            Some("false")
        );
    }

    #[test]
    fn and_shows_both_inputs_and_its_unwired_result() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open(
            &mut host,
            "graph.json",
            orange_host::new_document().unwrap(),
        );
        let a = editor
            .add_node_at(&mut host, "core/true-or-false", [0.0, 0.0])
            .unwrap();
        let b = editor
            .add_node_at(&mut host, "core/true-or-false", [0.0, 120.0])
            .unwrap();
        let and = editor
            .add_node_at(&mut host, "core/and", [300.0, 60.0])
            .unwrap();
        assert!(editor.set_input(&mut host, (a, "value"), Some("true")));
        assert!(editor.set_input(&mut host, (b, "value"), Some("false")));
        assert!(editor.connect(&mut host, (a, "result"), (and, "a")));
        assert!(editor.connect(&mut host, (b, "result"), (and, "b")));
        let graph = editor.graph().unwrap().clone();
        for (port, expected) in [("a", "true"), ("b", "false")] {
            assert_eq!(
                editor
                    .canvas()
                    .port_value(&graph, and, port, true)
                    .map(|value| value.short.as_str()),
                Some(expected)
            );
        }
        assert_eq!(editor.canvas().shown_value(and, "result"), Some("false"));
        assert!(editor.set_input(&mut host, (b, "value"), Some("true")));
        assert_eq!(editor.canvas().shown_value(and, "result"), Some("true"));
        editor
            .add_node_at(&mut host, "org.orange.math/greater", [600.0, 0.0])
            .unwrap();
        assert_eq!(editor.canvas().shown_value(and, "result"), Some("true"));
        assert!(editor.disconnect(&mut host, (and, "b"), true));
        assert_eq!(editor.canvas().shown_value(and, "result"), None);
    }
}

#[cfg(test)]
mod palette_keyboard_tests {
    use super::*;

    #[test]
    fn sidebar_search_adds_the_highlighted_result_by_keyboard() {
        let (mut host, _events) = Host::new(orange_host::HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open(
            &mut host,
            "graph.json",
            orange_host::new_document().unwrap(),
        );
        editor.search = "trigger".into();
        let expected = palette(&host, &editor.search)[0].1[1].id.clone();
        let context = egui::Context::default();
        for key in [None, Some(egui::Key::ArrowDown), Some(egui::Key::Enter)] {
            let events = key
                .into_iter()
                .map(|key| egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                })
                .collect();
            let input = egui::RawInput {
                events,
                ..Default::default()
            };
            context
                .run_ui(input, |ui| {
                    if key.is_none() {
                        let search = ui.make_persistent_id("node-palette-search");
                        ui.memory_mut(|memory| memory.request_focus(search));
                    }
                    editor.nodes_panel(ui, &mut host);
                })
                .drop_without_applying_deltas();
        }
        let graph = editor.graph().unwrap();
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].definition, expected);
    }
}
