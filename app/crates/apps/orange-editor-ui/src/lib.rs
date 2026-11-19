//! The editor interface of OrangeEditor, as an egui library.
//!
//! It is made of parts an application takes as it needs:
//!
//! - [`canvas::Canvas`] draws one graph and nothing else. An application with its own
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

pub mod canvas;
mod card_sources;
mod declarations;
pub mod i18n;
pub mod icons;
#[cfg(test)]
mod interaction_regressions;
mod keymap;
mod menu;
mod organise;
mod properties;
mod quick_add;
mod settings;
mod theme;
pub use theme::apply as apply_theme;
mod types_tab;

pub use properties::{COMMON_TYPES, port_label, set_at};
pub use quick_add::QuickChoice;

use canvas::Canvas;
use i18n::{t, tf};
use orange_host::{
    Declarations, Draft, FlowEvent, FlowId, GraphView, Host, HostError, Names, NodeId, PackageKind,
    StopMode, describe, lines,
};
use std::collections::VecDeque;

/// The nodes the Variables list adds for a variable: the button's message, its hint's
/// message and the kind.
const VARIABLE_NODES: [(&str, &str, &str); 2] = [
    ("variable-get", "variable-get-hint", "core/get-variable"),
    ("variable-set", "variable-set-hint", "core/set-variable"),
];

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteLevel {
    Info,
    Warning,
    Error,
}

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
    /// Its variables and connections.
    declared: Declarations,
    /// The layout changed since the document was opened or saved.
    unsaved: bool,
    /// What checking found, one line each; empty when the document is valid.
    problems: Vec<String>,
    /// The top-level node each problem is about, in the same order.
    problem_nodes: Vec<Option<NodeId>>,
    run_blocked: bool,
    run_omitted: Vec<NodeId>,
    /// The flow running this document, with the entries a person can fire.
    flow: Option<(FlowId, Vec<NodeId>)>,
    /// The latest value of each variable a run wrote, by variable ID, as text.
    values: std::collections::BTreeMap<String, String>,
}

impl Document {
    /// The draft retains every node. Only the runnable copy omits isolated errors.
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
    search: String,
    palette_highlighted: usize,
    log_events: bool,
    /// Whether successful steps and live values are written to the Log as they happen;
    /// off, the Log has the run's lifecycle, warnings, faults and the graph's own
    /// messages, and nothing is formatted for the rest.
    live_trace: bool,
    /// The last message a graph logged and how many times in a row, so repeats share
    /// one line.
    last_note: Option<(String, u32)>,
    packages_tab: bool,
    /// What the Packages tab is filtered by.
    package_search: String,
    /// A wire let go between two kinds of value, waiting for its conversion.
    conversion_choice: Option<ConversionChoice>,
    palette: palette::PaletteCache,
    previews: palette::PreviewCache,
    settings_open: bool,
    settings_search: String,
    settings_tab: settings::Section,
    preferred_scale: Option<f32>,
    scale_applied: bool,
    shortcut_search: String,
    shortcut_edit: Option<keymap::ShortcutEdit>,
    node_config_menu: Option<(NodeId, egui::Pos2)>,
    document: Option<Document>,
    canvas: Canvas,
    log: VecDeque<String>,
    /// The node whose values the properties panel is editing, and what is typed in
    /// each of its fields.
    inspected: Option<NodeId>,
    typed: std::collections::BTreeMap<String, String>,
    /// A variable being declared from the properties of the node that will name it.
    quick_variable: Option<properties::QuickVariable>,
    /// The left panel shows the document's declarations instead of the node kinds.
    tab: Tab,
    forms: declarations::Forms,
    /// The setting choices of each connection adapter (its devices, say), listed when
    /// first shown and again when a person asks: listing may ask the operating system.
    adapter_choices: std::collections::BTreeMap<String, Vec<orange_host::SettingChoices>>,
    /// The panels shown; a host application may set them, and the toolbar switches them.
    pub panels: Panels,
    /// Whether the application shows file dialogs: the toolbar then has New, Open and
    /// Save as, which it hands over through [`Editor::take_request`].
    pub file_dialogs: bool,
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
    /// The language the node search also matches in, as a BCP 47 tag; empty is English.
    language: String,
    /// The right-click menu, while it is open.
    menu: Option<menu::Menu>,
    /// What this editor copied last, for Paste from the menu.
    clip: Option<String>,
    /// Copied text for the system clipboard, handed over on the next frame.
    clip_out: Option<String>,
    /// Paste from the menu with nothing copied here: the system clipboard is asked.
    paste_asked: bool,
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
        self.language = language.trim().to_owned();
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
        self.node_config_menu = None;
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
        let graph = host.draft_view(&draft).unwrap_or_default();
        self.canvas.load(&graph);
        self.canvas.enums = canvas::enums_of(host, &draft, &graph, &host.declarations(&draft));
        self.canvas.set_groups(draft.groups());
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
            graph,
            unsaved: false,
            problems: Vec::new(),
            problem_nodes: Vec::new(),
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
        let Some(document) = &mut self.document else {
            return;
        };
        document.names = Names::of_document(host, document.draft.document()).without_ids();
        document.graph = host.draft_view(&document.draft).unwrap_or_default();
        document.declared = host.declarations(&document.draft);
        types_tab::remember(&document.declared.types);
        document.unsaved = true;
        self.canvas.update(&document.graph);
        self.canvas.enums =
            canvas::enums_of(host, &document.draft, &document.graph, &document.declared);
        self.canvas.set_groups(document.draft.groups());
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
                Ok(_) => {
                    let entries = host
                        .info(flow)
                        .map(|info| {
                            info.manual_entries
                                .iter()
                                .filter(|(_, payload)| !payload)
                                .map(|(node, _)| *node)
                                .collect()
                        })
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
        self.preview_values(host);
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

    /// Adds a node of kind `definition` with its top left corner at `at` (graph units)
    /// and selects it.
    pub fn add_node_at(
        &mut self,
        host: &mut Host,
        definition: &str,
        at: [f32; 2],
    ) -> Option<NodeId> {
        let draft = self.editing()?;
        let result = if definition.starts_with(palette::CONVERSION) {
            match palette::conversion_named(host, definition) {
                Some(conversion) => host.add_conversion(draft, &conversion, at),
                None => Err(orange_host::HostError::Document(format!(
                    "no conversion `{definition}`"
                ))),
            }
        } else {
            host.add_node(draft, definition, at)
        };
        match result {
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

    /// Removes a node with its wires.
    pub fn remove_node(&mut self, host: &mut Host, node: NodeId) -> bool {
        self.remove_nodes(host, &[node])
    }

    /// Removes nodes with their wires, as one edit.
    pub fn remove_nodes(&mut self, host: &mut Host, nodes: &[NodeId]) -> bool {
        let Some(draft) = self.editing() else {
            return false;
        };
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
        let choices = waiting.choices.clone();
        let mut chosen = None;
        let mut close = false;
        let shown = egui::Area::new(egui::Id::unique("orange-conversion-choice"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(320.0);
                    ui.strong(t("choose-conversion"));
                    ui.weak(t("choose-conversion-hint"));
                    ui.separator();
                    for choice in &choices {
                        let detail = if choice.lossy {
                            t("conversion-lossy")
                        } else if choice.checked {
                            t("conversion-checked")
                        } else {
                            t("conversion-exact")
                        };
                        if ui
                            .button(palette::conversion_label(choice))
                            .on_hover_text(detail)
                            .clicked()
                        {
                            chosen = Some(choice.clone());
                        }
                    }
                    ui.separator();
                    close = ui.button(t("cancel")).clicked();
                });
            })
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
        self.forms.reveal_variable();
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
        self.log.iter().map(String::as_str)
    }

    /// Whether the open document is running.
    pub fn is_running(&self) -> bool {
        self.document
            .as_ref()
            .is_some_and(|document| document.flow.is_some())
    }

    /// A message a graph logged: `level` is info, warning or error. A message the same
    /// as the last one adds to its count instead of a new line.
    pub fn note(&mut self, level: NoteLevel, text: &str) {
        let line = match level {
            NoteLevel::Info => text.to_owned(),
            NoteLevel::Warning => tf("log-warning", &[("text", text.into())]),
            NoteLevel::Error => tf("log-error", &[("text", text.into())]),
        };
        if let Some((last, count)) = &mut self.last_note
            && *last == line
            && let Some(shown) = self.log.back_mut()
        {
            *count += 1;
            *shown = tf(
                "log-repeated",
                &[("line", line.as_str().into()), ("count", (*count).into())],
            );
            return;
        }
        self.say(line.clone());
        self.last_note = Some((line, 1));
    }

    /// Says that `count` messages a graph logged were dropped because they came faster
    /// than the Log takes them.
    pub fn notes_dropped(&mut self, count: u64) {
        if count > 0 {
            self.say(tf("log-dropped", &[("count", count.into())]));
        }
    }

    fn say(&mut self, line: String) {
        self.last_note = None;
        if self.log.len() >= MAX_LOG_LINES {
            let _ = self.log.pop_front();
        }
        self.log.push_back(line);
    }

    fn problems_of(error: &HostError, names: &Names) -> Vec<String> {
        match error {
            HostError::Invalid(diagnostics) => diagnostics
                .iter()
                .map(|diagnostic| describe(diagnostic, names))
                .collect(),
            other => vec![other.to_string()],
        }
    }

    /// Checks the open document against the host's packages.
    pub fn check(&mut self, host: &Host) {
        let Some(document) = &mut self.document else {
            return;
        };
        let mut marks = canvas::Marks::default();
        document.problem_nodes.clear();
        document.run_blocked = false;
        document.run_omitted.clear();
        let wired: std::collections::BTreeSet<_> = document
            .graph
            .wires
            .iter()
            .flat_map(|wire| [wire.from.0, wire.to.0])
            .collect();
        document.problems = match host.problems(&mut document.draft) {
            Ok(diagnostics) => diagnostics
                .iter()
                .map(|diagnostic| {
                    let text = describe(diagnostic, &document.names);
                    let error = diagnostic.severity == orange_host::Severity::Error;
                    // Only the top level is on the canvas.
                    let node = diagnostic
                        .subject
                        .node
                        .filter(|_| diagnostic.subject.function.is_none());
                    let wire = host.wire_of(&document.draft, diagnostic);
                    if error {
                        if let Some(node) = node.filter(|id| {
                            wire.is_none()
                                && !wired.contains(id)
                                && document.graph.nodes.iter().any(|view| view.id == *id)
                        }) {
                            document.run_omitted.push(node);
                        } else {
                            document.run_blocked = true;
                        }
                    }
                    if let Some(wire) = wire {
                        canvas::Marks::add(&mut marks.wires, wire, error, text.clone());
                    } else if let Some(node) = node {
                        canvas::Marks::add(&mut marks.nodes, node, error, text.clone());
                        // The port at fault is ringed as well as the node marked.
                        if let Some(port) = &diagnostic.subject.port {
                            canvas::Marks::add(
                                &mut marks.ports,
                                (node, port.to_string()),
                                error,
                                text.clone(),
                            );
                        }
                    }
                    document.problem_nodes.push(node);
                    text
                })
                .collect(),
            Err(error) => {
                document.run_blocked = true;
                Self::problems_of(&error, &document.names)
            }
        };
        document.run_omitted.sort_unstable();
        document.run_omitted.dedup();
        document.run_blocked |= !document.run_omitted.is_empty()
            && document.run_omitted.len() == document.graph.nodes.len();
        if document.run_blocked {
            document.run_omitted.clear();
        }
        self.canvas.set_marks(marks);
    }

    /// Starts the open document as a flow. A document that cannot start says why in
    /// its problems.
    pub fn run(&mut self, host: &mut Host) {
        self.preview_values(host);
        let Some(document) = &mut self.document else {
            return;
        };
        if document.flow.is_some() || document.run_blocked {
            return;
        }
        match document
            .runtime_text(host)
            .and_then(|text| host.open(&text))
        {
            Ok(flow) => {
                self.canvas.begin_run();
                let _ = host.observe(flow, true);
                let entries = host
                    .info(flow)
                    .map(|info| {
                        info.manual_entries
                            .iter()
                            // An entry that takes a payload needs a value to fire.
                            .filter(|(_, payload)| !payload)
                            .map(|(node, _)| *node)
                            .collect()
                    })
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
            Err(error) => document.problems = Self::problems_of(&error, &document.names),
        }
    }

    /// Stops and closes the flow of the open document.
    pub fn stop(&mut self, host: &mut Host) {
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
        if let Err(error) = host.trigger(*flow, node, None) {
            self.say(format!("[{flow}] {error}"));
        }
    }

    /// Takes up to [`MAX_EVENTS_PER_POLL`] host events into the log. Returns whether
    /// anything arrived, so a window knows to draw and poll again. The finite batch
    /// leaves time for input and painting even when producers continuously refill it.
    pub fn take_events(&mut self, events: &orange_host::Events) -> bool {
        let mut any = false;
        for _ in 0..MAX_EVENTS_PER_POLL {
            let Ok(event) = events.try_recv() else {
                break;
            };
            any = true;
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
            let said = if logged {
                lines(&event, &document.names)
            } else {
                Vec::new()
            };
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
                        let view = document.graph.nodes.iter().find(|view| view.id == *node);
                        if id == "spectrum"
                            && view.is_some_and(|view| {
                                matches!(
                                    view.definition.as_str(),
                                    "org.orange.audio/spectrum"
                                        | "org.orange.audio/multiband-spectrum"
                                )
                            })
                            && let orange_host::Value::List(values) = value
                        {
                            if !self.canvas.collapsed_spectra.contains(node) {
                                let bins = (0..values.len().min(96))
                                    .filter_map(|index| match values.get(index) {
                                        Some(orange_host::Value::F64(db)) => Some(db),
                                        _ => None,
                                    })
                                    .collect();
                                let _ = self.canvas.spectra.insert(*node, bins);
                            }
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
                                        && matches!(
                                            view.definition.as_str(),
                                            "org.orange.audio/band-trigger"
                                                | "org.orange.audio/transient"
                                                | "org.orange.audio/on-band-level"
                                                | "org.orange.audio/on-transient"
                                        ))
                        });
                        let short = if decibels {
                            format!("{short} dBFS")
                        } else {
                            short
                        };
                        self.canvas.show_value(*node, id, short, full);
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
                        let _ = document.values.insert(variable.to_string(), shown);
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
                    self.canvas.mark_stale();
                }
                _ => {}
            }
            for line in said {
                self.say(line);
            }
        }
        any
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
            self.scale_applied = true;
        }
        self.preferred_scale = Some(ui.ctx().zoom_factor());
        self.canvas.update_help(host, &self.language);
        if let Some(clip) = self.clip_out.take() {
            ui.ctx().copy_text(clip);
        }
        if std::mem::take(&mut self.paste_asked) {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        }
        self.canvas.shortcuts_enabled = !self.settings_open;
        self.keyboard(ui, host);
        layout(self, ui, host);
        if self.window_toolbar {
            resize_borders(ui.ctx());
        }
        let _ = self.settings_window(ui.ctx());
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
            && self.quick.is_none()
            && self.menu.is_none()
            && self.node_config_menu.is_none()
            && !self.canvas.keys.0.values().flatten().any(|binding| {
                binding.logical_key == key && modifiers.matches_logically(binding.modifiers)
            })
    }

    /// Draws the editor's top command bar in a top panel.
    pub fn toolbar(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        egui::Panel::top("orange-toolbar").show(ui, |ui| {
            ui.spacing_mut().interact_size.y = 28.0;
            ui.horizontal(|ui| {
                // File
                let open = self.document.is_some();
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
                let chosen = self.canvas.selection().len();
                ui.menu_button(t("graph"), |ui| {
                    for (enabled, icon, label, shortcut) in [
                        (chosen > 0, icons::GROUP, t("group-selection"), "group"),
                        (chosen > 0, icons::FUNCTION, t("make-function"), "function"),
                        (
                            chosen > 1,
                            icons::ARRANGE,
                            t("arrange-selection"),
                            "arrange",
                        ),
                        (open, icons::VARIABLE, t("new-variable"), ""),
                    ] {
                        let button = egui::Button::new((icons::text(icon), label)).shortcut_text(
                            if shortcut.is_empty() {
                                String::new()
                            } else {
                                self.canvas.keys.label(shortcut)
                            },
                        );
                        if ui.add_enabled(enabled, button).clicked() {
                            match icon {
                                icons::GROUP => {
                                    let _ = self.group_selection(host, t("group"));
                                }
                                icons::FUNCTION => self.preview_function(host),
                                icons::ARRANGE => {
                                    let _ = self.arrange_selection();
                                }
                                icons::VARIABLE => self.new_variable_form(),
                                _ => {}
                            }
                            ui.close();
                        }
                    }
                });
                ui.separator();
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
        )
        .clicked()
        {
            self.panels.nodes = !self.panels.nodes;
        }
        let open = self.document.is_some();
        if icons::button(
            ui,
            open,
            icons::FIT,
            t("fit-graph-hint"),
            &self.canvas.keys.label("fit"),
        )
        .clicked()
        {
            self.canvas.fit();
        }
        if ui
            .add_enabled(
                open,
                egui::Button::new(format!("{:.0}%", self.canvas.zoom() * 100.0)),
            )
            .on_hover_text(t("actual-size"))
            .clicked()
        {
            self.canvas.reset_zoom();
        }
    }

    fn workspace_diagnostics(&mut self, ui: &mut egui::Ui, centered: bool) {
        let count = self.problems().len();
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
        if icons::dock_toggle(ui, open, egui::Align::Max, t("right-panel-hint")).clicked() {
            if open {
                self.packages_tab = self.panels.packages && !self.panels.properties;
                self.panels.properties = false;
                self.panels.packages = false;
            } else {
                self.panels.properties = !self.packages_tab;
                self.panels.packages = self.packages_tab;
            }
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
        ui.horizontal(|ui| {
            let _ = ui.selectable_value(&mut packages, false, t("properties"));
            let _ = ui.selectable_value(&mut packages, true, t("packages"));
        });
        ui.separator();
        self.panels.properties = !packages;
        self.panels.packages = packages;
        if packages {
            Self::packages_panel(ui, host, &mut self.package_search);
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
            .show_ui(ui, |ui| {
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
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().button_padding.x = 6.0;
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.spacing_mut().interact_size.x = 0.0;
            let _ = ui.selectable_value(&mut self.tab, Tab::Nodes, t("nodes"));
            let _ = ui
                .selectable_value(&mut self.tab, Tab::Variables, t("variables"))
                .on_hover_text(t("variables-hint"));
            let _ = ui
                .selectable_value(&mut self.tab, Tab::Connections, t("connections"))
                .on_hover_text(t("connections-hint"));
            let _ = ui
                .selectable_value(&mut self.tab, Tab::Types, t("types"))
                .on_hover_text(t("types-hint"));
        });
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
        let mut variable_node = None;
        let mut new_variable = false;
        let search_id = ui.scope_id().with("node-palette-search");
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
        let field = ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .id(search_id)
                .event_filter(egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                })
                .hint_text(t("search-hint"))
                .desired_width(f32::INFINITY),
        );
        if field.changed() {
            self.palette_highlighted = 0;
        }
        let variables = self.variables_to_use();
        egui::ScrollArea::vertical().show(ui, |ui| {
            let searching = !self.search.trim().is_empty();
            // The graph's own variables first: reading or writing one is the most
            // common node, and declaring one starts here too.
            if self.document.is_some() && !searching {
                egui::CollapsingHeader::new(tf(
                    "variables-count",
                    &[("count", variables.len().into())],
                ))
                .id_salt("orange-variables")
                .default_open(variables.len() <= 4)
                .show(ui, |ui| {
                    for (id, name, ty) in &variables {
                        // The buttons take their room first; the name is cut to what
                        // is left, so a long name never runs under them.
                        ui.horizontal(|ui| {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    for (label, hint, definition) in VARIABLE_NODES.iter().rev() {
                                        if ui
                                            .small_button(t(label))
                                            .on_hover_text(tf(
                                                hint,
                                                &[("name", name.as_str().into())],
                                            ))
                                            .clicked()
                                        {
                                            variable_node = Some((*definition, id.clone()));
                                        }
                                    }
                                    ui.with_layout(
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            let _ = ui
                                                .add(egui::Label::new(name.as_str()).truncate())
                                                .on_hover_text(format!(
                                                    "{name}
{}",
                                                    type_name(ty)
                                                ));
                                        },
                                    );
                                },
                            );
                        });
                    }
                    new_variable = ui.button(t("new-variable-ellipsis")).clicked();
                });
            }
            let language = if self.language.is_empty() {
                orange_host::SOURCE_LANGUAGE
            } else {
                self.language.as_str()
            };
            let results = self.palette.get(host, &self.search, language);
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
                egui::CollapsingHeader::new(category.as_str())
                    .id_salt((category.as_str(), searching))
                    .default_open(searching)
                    .show(ui, |ui| {
                        for item in items {
                            let active =
                                searching && field.has_focus() && index == self.palette_highlighted;
                            index += 1;
                            let row = palette::result_row(
                                ui,
                                &item.label,
                                &item.purpose,
                                &item.id,
                                active,
                            );
                            if row.drag_started() {
                                let card = self
                                    .previews
                                    .card(
                                        host,
                                        self.document.as_ref().map(|document| &document.draft),
                                        &item.id,
                                    )
                                    .cloned();
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
                    });
            }
        });
        if new_variable {
            self.new_variable_form();
        }
        if let Some((definition, variable)) = variable_node {
            let _ = self.add_variable_node(host, definition, &variable);
        }
        if let Some(definition) = add {
            if self.document.is_some() {
                let _ = self.add_node(host, &definition);
            } else {
                self.say(t("open-graph-before-adding").to_owned());
            }
        }
    }

    /// The open document's variables that nodes can name, as (ID, name, type).
    fn variables_to_use(&self) -> Vec<(String, String, String)> {
        self.declarations()
            .map(|declared| {
                declared
                    .variables
                    .iter()
                    .filter_map(|variable| {
                        Some((
                            variable.id.clone()?,
                            variable.name.clone(),
                            variable.ty.clone(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The packages this host has, found by name, ID or description: each a framed row
    /// with its name and version, what it adds, and whether it is in use.
    fn packages_panel(ui: &mut egui::Ui, host: &Host, search: &mut String) {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        let _ = ui.add(
            egui::TextEdit::singleline(search)
                .hint_text(t("search-packages-hint"))
                .desired_width(f32::INFINITY),
        );
        ui.add_space(4.0);
        let wanted = search.trim().to_lowercase();
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut shown = 0;
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
                    });
            }
            if shown == 0 {
                ui.weak(t("no-package-matches"));
            }
        });
    }

    /// Draws the current diagnostics tab in the supplied container.
    pub fn log_panel(&mut self, ui: &mut egui::Ui) {
        ui.set_min_height(ui.available_height());
        if self.log_events {
            let _ = ui
                .checkbox(&mut self.live_trace, t("live-trace"))
                .on_hover_text(t("live-trace-hint"));
            egui::ScrollArea::vertical()
                .id_salt("editor-events")
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in &self.log {
                        ui.monospace(line.as_str());
                    }
                });
            return;
        }
        let problems = self.problems().to_vec();
        let nodes = self
            .document
            .as_ref()
            .map(|document| document.problem_nodes.clone())
            .unwrap_or_default();
        let mut focus = None;
        egui::ScrollArea::vertical()
            .id_salt("editor-problems")
            .show(ui, |ui| {
                if problems.is_empty() {
                    ui.weak(t("no-problems"));
                } else {
                    for (index, problem) in problems.iter().enumerate() {
                        // A problem about a node takes you to it.
                        match nodes.get(index).copied().flatten() {
                            Some(node) => {
                                if ui
                                    .link(problem.as_str())
                                    .on_hover_text(t("show-node"))
                                    .clicked()
                                {
                                    focus = Some(node);
                                }
                            }
                            None => {
                                ui.label(problem.as_str());
                            }
                        }
                    }
                }
            });
        if let (Some(node), Some(document)) = (focus, &self.document) {
            self.canvas.focus(&document.graph, node);
        }
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
        let done =
            self.canvas
                .show_with_path_controls(ui, &document.graph, triggers, self.file_dialogs);
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
        if done.show_problems || done.show_runtime_log {
            self.panels.log = true;
            self.log_events = done.show_runtime_log;
        }
        if done.moved {
            self.keep_layout();
        }
        if let Some(node) = done.trigger {
            self.trigger(host, node);
        }
        if let Some((from, to)) = done.connect {
            let _ = self.connect(host, (from.0, &from.1), (to.0, &to.1));
        }
        self.conversion_menu(ui.ctx(), host);
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
        if let Some((at, screen)) = done.quick_add {
            self.open_quick_add(None, at, screen);
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
        if let Some((node, pointer, value)) = done.set_config {
            self.set_setting(host, node, &pointer, Some(value));
        }
        if let Some((node, pointer)) = done.new_variable {
            // Naming the new variable needs a form, which the properties panel has.
            self.canvas.select(Some(node));
            self.panels.properties = true;
            self.panels.packages = false;
            self.quick_variable = Some(properties::QuickVariable::new(node, &pointer));
        }
        if let Some((node, device)) = done.device {
            self.choose_device(host, node, device);
        }
        if let Some((node, edit)) = done.spectrum_edit
            && let Some(draft) = self.editing()
        {
            let result = match edit {
                canvas::BandEdit::Add(range) => host.add_filtered_audio_band(draft, node, range),
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
        // Every node kind search offers, and every conversion in a category of its own.
        let offered = host
            .nodes()
            .filter(|info| !info.presentation.is_some_and(|shown| shown.hidden))
            .count();
        assert_eq!(count(&all), offered + host.conversions().len());
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
        assert!(all.windows(2).all(|pair| pair[0].0 < pair[1].0));
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
        let bound = host.bind(&self::graph()).unwrap();
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
                .is_some_and(|value| value == "7");
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
        assert!(editor.problems()[0].contains("(node go)"));
        // The canvas marks the node the problem is about, and the list can take you there.
        let go = editor.graph().unwrap().nodes[0].id;
        let mark = editor.canvas().marks().nodes.get(&go).cloned().unwrap();
        assert!(mark.error && mark.text.contains("(node go)"));
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
        let bound = host.bind(&self::graph()).unwrap();
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
    fn diagnostics_are_centered_and_switching_tabs_preserves_canvas_height() {
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
        assert!((group.center().x - 640.0).abs() <= 8.0, "{group:?}");
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
        for kind in &kinds {
            let before = editor.log().count();
            let node_count = editor.graph().unwrap().nodes.len();
            match editor.add_node(&mut host, kind) {
                Some(node) => {
                    let graph = editor.graph().unwrap();
                    assert_eq!(graph.nodes.len(), node_count + 1, "{kind}");
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
        // The quick form draws in the properties of the node it is for.
        editor.tab = Tab::Nodes;
        editor.canvas().select(Some(set));
        editor.quick_variable = Some(properties::QuickVariable::for_test(set));
        assert!(draw_at(&mut editor, &mut host, [520.0, 360.0]) > 0);
        assert!(editor.quick_variable.is_some());
    }

    #[test]
    fn branches_are_added_and_taken_off_from_the_editor() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open(&mut host, "new.json", blank());
        let fork = editor.add_node(&mut host, "core/fork").unwrap();
        let branches = |editor: &Editor| {
            let graph = editor.graph().unwrap();
            let node = graph.nodes.iter().find(|found| found.id == fork).unwrap();
            (node.variadic[0].count, node.ports.len())
        };
        assert_eq!(branches(&editor), (1, 3));
        assert_eq!(
            editor.add_port(&mut host, fork, "branch").as_deref(),
            Some("branch#1")
        );
        assert_eq!(branches(&editor), (2, 4));
        assert!(editor.remove_port(&mut host, (fork, "branch#1")));
        assert_eq!(branches(&editor), (1, 3));
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
        // One step back takes the wire off, another the node.
        editor.undo(&mut host);
        assert!(
            editor
                .graph()
                .unwrap()
                .nodes
                .iter()
                .any(|found| found.id == node)
        );
        editor.undo(&mut host);
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
        crate::icons::install(&context);
        assert_eq!(
            context.fonts(|fonts| fonts.definitions().font_data.len()),
            egui::FontDefinitions::default().font_data.len() + 1
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
        let run = drawn_at(&shapes, &crate::icons::RUN.to_string()).expect("the toolbar shows Run");
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
        let Some(document) = &self.document else {
            return;
        };
        let Ok(text) = document.draft.text() else {
            return;
        };
        let wanted: Vec<(NodeId, orange_host::PortKey)> = document
            .graph
            .nodes
            .iter()
            // A pure generic kernel still needs a concrete type before it can run.
            // Skip unbound cards rather than retrying their previews on every edit.
            .filter(|node| {
                !node.entry
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
        if document.flow.is_none() {
            self.canvas.clear_run();
        } else {
            self.canvas.clear_previews();
        }
        if wanted.is_empty() {
            return;
        }
        // Try one batch first. Unrelated effects or invalid nodes can make the
        // complete document unsuitable, even when a pure branch is ready.
        let found = match host.preview(&text, &wanted) {
            Ok(found) => found,
            Err(_) => {
                let Ok(saved) = serde_json::from_str(&text) else {
                    return;
                };
                let mut upstream = std::collections::HashMap::<NodeId, Vec<NodeId>>::new();
                for wire in &document.graph.wires {
                    upstream.entry(wire.to.0).or_default().push(wire.from.0);
                }
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
                self.canvas.show_preview(
                    previewed.node,
                    previewed.port.to_string(),
                    orange_host::plain(value),
                    orange_host::inspect(value),
                );
            }
        }
    }
}

/// Retain the requested root nodes and their upstream dependencies. The host still
/// validates and compiles the projection, including the original providers and types.
fn preview_dependencies(
    saved: &serde_json::Value,
    upstream: &std::collections::HashMap<NodeId, Vec<NodeId>>,
    outputs: impl Iterator<Item = NodeId>,
) -> Option<serde_json::Value> {
    let mut nodes = std::collections::BTreeSet::new();
    let mut pending: Vec<_> = outputs.collect();
    while let Some(node) = pending.pop() {
        if nodes.insert(node)
            && let Some(sources) = upstream.get(&node)
        {
            pending.extend(sources);
        }
    }
    let contains = |id: &serde_json::Value| {
        id.as_str()
            .and_then(|id| id.parse::<NodeId>().ok())
            .is_some_and(|id| nodes.contains(&id))
    };
    let mut scoped = saved.clone();
    scoped["nodes"]
        .as_array_mut()?
        .retain(|node| contains(&node["id"]));
    scoped["edges"]
        .as_array_mut()?
        .retain(|edge| contains(&edge["from"]["node"]) && contains(&edge["to"]["node"]));
    Some(scoped)
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
                        let search = ui.scope_id().with("node-palette-search");
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
