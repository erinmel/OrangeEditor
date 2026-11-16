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
mod declarations;
pub mod icons;
mod menu;
mod organise;
mod properties;
mod quick_add;

pub use properties::{COMMON_TYPES, port_label, set_at};
pub use quick_add::QuickChoice;

use canvas::Canvas;
use orange_host::{
    Declarations, Draft, FlowEvent, FlowId, GraphView, Host, HostError, HostEvent, Names, NodeId,
    PackageKind, StopMode, describe, lines,
};
use std::collections::VecDeque;
use std::sync::mpsc::Receiver;

/// The nodes the Variables list adds for a variable: the button and the kind.
const VARIABLE_NODES: [(&str, &str); 2] =
    [("Get", "core/get-variable"), ("Set", "core/set-variable")];

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
pub fn type_name(ty: &str) -> String {
    match ty {
        "core/string@1" => "Text".to_owned(),
        "core/bool@1" => "Bool".to_owned(),
        "core/i64@1" | "core/u64@1" | "core/i32@1" | "core/u32@1" => "Int".to_owned(),
        "core/f64@1" | "core/f32@1" => "Float".to_owned(),
        "core/bytes@1" => "Bytes".to_owned(),
        "core/duration@1" => "Duration".to_owned(),
        // Core types lose their namespace everywhere in the expression.
        other => other.replace("core/", "").replace("@1", ""),
    }
}

/// A type exactly, for a tooltip beside its short name: `core/i64@1` is `i64`.
pub fn exact_type(ty: &str) -> String {
    ty.replace("core/", "").replace("@1", "")
}

/// The left panel's tabs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Tab {
    #[default]
    Nodes,
    Variables,
    Connections,
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
    /// The flow running this document, with the entries a person can fire.
    flow: Option<(FlowId, Vec<NodeId>)>,
    /// The latest value of each variable a run wrote, by variable ID, as text.
    values: std::collections::BTreeMap<String, String>,
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
            packages: false,
            log: true,
            properties: true,
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
    palette: palette::PaletteCache,
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

    /// The language the node search matches in besides English, such as `es`.
    pub fn language(&self) -> &str {
        if self.language.is_empty() {
            orange_host::SOURCE_LANGUAGE
        } else {
            &self.language
        }
    }

    /// Makes the node search match in `language` (a BCP 47 tag) and English. Nothing
    /// about the graph changes with it.
    pub fn set_language(&mut self, language: &str) {
        self.language = language.trim().to_owned();
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
        self.stop(host);
        let path = path.into();
        let draft = match host.draft(&text) {
            Ok(draft) => draft,
            Err(error) => {
                self.document = None;
                self.say(format!("not opened: {path}: {error}"));
                return;
            }
        };
        let graph = host.draft_view(&draft).unwrap_or_default();
        self.canvas.load(&graph);
        self.canvas.set_groups(draft.groups());
        self.document = Some(Document {
            path,
            untitled: false,
            declared: host.declarations(&draft),
            names: Names::of_document(host, draft.document()).without_ids(),
            draft,
            graph,
            unsaved: false,
            problems: Vec::new(),
            problem_nodes: Vec::new(),
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
        let Some(document) = &mut self.document else {
            return;
        };
        document.names = Names::of_document(host, document.draft.document()).without_ids();
        document.graph = host.draft_view(&document.draft).unwrap_or_default();
        document.declared = host.declarations(&document.draft);
        document.unsaved = true;
        self.canvas.update(&document.graph);
        self.canvas.set_groups(document.draft.groups());
        // The fields of the properties panel are read again from the new document.
        self.inspected = None;
        let mut said = None;
        if let Some((flow, entries)) = &mut document.flow {
            match document
                .draft
                .text()
                .and_then(|text| host.update(*flow, &text))
            {
                Ok(_) => {
                    *entries = host
                        .info(*flow)
                        .map(|info| {
                            info.manual_entries
                                .iter()
                                .filter(|(_, payload)| !payload)
                                .map(|(node, _)| *node)
                                .collect()
                        })
                        .unwrap_or_default();
                }
                // The flow keeps running the graph it had.
                Err(error) => said = Some(format!("[{flow}] the edit is not running yet: {error}")),
            }
        }
        if let Some(line) = said {
            self.say(line);
        }
        self.check(host);
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
                    self.say(format!("not done: {line}"));
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
        match host.add_node(draft, definition, at) {
            Ok(node) => {
                let _ = self.edited(host, Ok(true));
                self.canvas.select(Some(node));
                Some(node)
            }
            Err(error) => {
                let _ = self.edited(host, Err(error));
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
                self.say(format!("not copied: {error}"));
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

    /// Wires an output to an input. A wire the language does not allow is refused
    /// with the reason in the log.
    pub fn connect(&mut self, host: &mut Host, from: (NodeId, &str), to: (NodeId, &str)) -> bool {
        let Some(draft) = self.editing() else {
            return false;
        };
        let result = host.connect(draft, from, to);
        self.edited(host, result)
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
        let line = format!("saved {}", document.path);
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

    fn say(&mut self, line: String) {
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
                    if let Some(wire) = host.wire_of(&document.draft, diagnostic) {
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
            Err(error) => Self::problems_of(&error, &document.names),
        };
        self.canvas.set_marks(marks);
    }

    /// Starts the open document as a flow. A document that cannot start says why in
    /// its problems.
    pub fn run(&mut self, host: &mut Host) {
        let Some(document) = &mut self.document else {
            return;
        };
        if document.flow.is_some() {
            return;
        }
        match document.draft.text().and_then(|text| host.open(&text)) {
            Ok(flow) => {
                self.canvas.clear_run();
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
                document.problems.clear();
                let line = format!("[{flow}] opened {}", document.path);
                self.say(line);
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
        self.say(format!("[{flow}] closed"));
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
    pub fn take_events(&mut self, events: &Receiver<HostEvent>) -> bool {
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
            let said = lines(&event, &document.names);
            // The canvas follows the run: a node glows as it takes a step, and the
            // latest value on an output shows beside it.
            match &event.kind {
                FlowEvent::Node(step) => {
                    let failed = step.fault.is_some()
                        || matches!(
                            step.outcome,
                            orange_host::ControlOutcome::Failed
                                | orange_host::ControlOutcome::TimedOut
                        );
                    self.canvas.pulse(step.node, failed);
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
                        self.canvas.show_value(
                            *node,
                            port.to_string(),
                            orange_host::plain(value),
                            orange_host::inspect(value),
                        );
                    }
                }
                FlowEvent::Variables(changed) => {
                    for (variable, value) in changed {
                        let shown = value
                            .as_ref()
                            .map_or_else(|| "unset".to_owned(), orange_host::plain);
                        let _ = document.values.insert(variable.to_string(), shown);
                    }
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
        icons::install(ui.ctx());
        if let Some(clip) = self.clip_out.take() {
            ui.ctx().copy_text(clip);
        }
        if std::mem::take(&mut self.paste_asked) {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        }
        // Undo, redo, the file keys and the canvas commands' keys, unless a text field
        // is being typed in.
        if !ui.ctx().egui_wants_keyboard_input() {
            let (undo, redo, save, save_as, open, new) = ui.input(|input| {
                let command = input.modifiers.command;
                let shift = input.modifiers.shift;
                let z = input.key_pressed(egui::Key::Z);
                let key = |key| command && input.key_pressed(key);
                (
                    command && z && !shift,
                    command && (input.key_pressed(egui::Key::Y) || (z && shift)),
                    key(egui::Key::S) && !shift,
                    key(egui::Key::S) && shift,
                    key(egui::Key::O),
                    key(egui::Key::N),
                )
            });
            let (duplicate, arrange, function, fit) = ui.input(|input| {
                let key = |key| input.modifiers.command && input.key_pressed(key);
                (
                    key(egui::Key::D),
                    key(egui::Key::L),
                    key(egui::Key::E),
                    input.key_pressed(egui::Key::Home),
                )
            });
            if duplicate {
                let _ = self.duplicate(host);
            }
            if arrange {
                let _ = self.arrange_selection();
            }
            if function && !self.canvas.selection().is_empty() {
                self.preview_function(host);
            }
            if fit {
                self.canvas.fit();
            }
            if undo {
                self.undo(host);
            }
            if redo {
                self.redo(host);
            }
            if save && let Err(message) = self.save() {
                self.say(message);
            }
            if self.file_dialogs {
                let asked = [
                    (save_as, FileRequest::SaveAs),
                    (open, FileRequest::Open),
                    (new, FileRequest::New),
                ];
                if let Some((_, request)) = asked.into_iter().find(|(pressed, _)| *pressed) {
                    self.ask(request);
                }
            }
        }
        self.toolbar(ui, host);
        if self.panels.nodes {
            egui::Panel::left("orange-nodes")
                .resizable(true)
                .default_size(250.0)
                .show(ui, |ui| self.nodes_panel(ui, host));
        }
        if self.panels.properties {
            egui::Panel::right("orange-properties")
                .resizable(true)
                .default_size(240.0)
                .show(ui, |ui| self.properties_panel(ui, host));
        }
        if self.panels.packages {
            egui::Panel::right("orange-packages")
                .resizable(true)
                .default_size(230.0)
                .show(ui, |ui| Self::packages_panel(ui, host));
        }
        if self.panels.log {
            egui::Panel::bottom("orange-log")
                .resizable(true)
                .default_size(170.0)
                .show(ui, |ui| self.log_panel(ui));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.canvas_panel(ui, host));
        // The panels have now taken in what a field was being typed, so the run
        // starts with it.
        if std::mem::take(&mut self.run_asked) {
            self.run(host);
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        egui::Panel::top("orange-toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                // File
                let open = self.document.is_some();
                if self.file_dialogs {
                    if icons::button(ui, true, icons::NEW, "New graph", "Ctrl+N").clicked() {
                        self.ask(FileRequest::New);
                    }
                    if icons::button(ui, true, icons::OPEN, "Open a graph…", "Ctrl+O").clicked() {
                        self.ask(FileRequest::Open);
                    }
                }
                if icons::button(ui, open, icons::SAVE, "Save", "Ctrl+S").clicked()
                    && let Err(message) = self.save()
                {
                    self.say(message);
                }
                if self.file_dialogs
                    && icons::button(ui, open, icons::SAVE_AS, "Save as…", "Ctrl+Shift+S").clicked()
                {
                    self.ask(FileRequest::SaveAs);
                }
                ui.separator();
                // Edit
                let (undo, redo) = self.can_undo();
                if icons::button(ui, undo, icons::UNDO, "Undo", "Ctrl+Z").clicked() {
                    self.undo(host);
                }
                if icons::button(ui, redo, icons::REDO, "Redo", "Ctrl+Y").clicked() {
                    self.redo(host);
                }
                let chosen = self.canvas.selection().len();
                if icons::button(
                    ui,
                    chosen > 0,
                    icons::GROUP,
                    "Frame the selected nodes in a named group",
                    "Ctrl+G",
                )
                .clicked()
                {
                    let _ = self.group_selection(host, "Group");
                }
                if icons::button(
                    ui,
                    chosen > 0,
                    icons::FUNCTION,
                    "Make a reusable function of the selected nodes…",
                    "Ctrl+E",
                )
                .clicked()
                {
                    self.preview_function(host);
                }
                if icons::button(
                    ui,
                    chosen > 1,
                    icons::ARRANGE,
                    "Lay the selected nodes out along their wires",
                    "Ctrl+L",
                )
                .clicked()
                {
                    let _ = self.arrange_selection();
                }
                if icons::button(
                    ui,
                    open,
                    icons::VARIABLE,
                    "Declare a variable of this graph…",
                    "",
                )
                .clicked()
                {
                    self.new_variable_form();
                }
                ui.separator();
                // Run
                let running = self
                    .document
                    .as_ref()
                    .is_some_and(|document| document.flow.is_some());
                if icons::button(ui, open, icons::CHECK, "Check the graph for problems", "")
                    .clicked()
                {
                    self.check(host);
                }
                if running {
                    if icons::button(ui, true, icons::STOP, "Stop", "").clicked() {
                        self.stop(host);
                    }
                } else if icons::button(ui, open, icons::RUN, "Run", "").clicked() {
                    // A field being typed in ends its typing now, in this frame, and the
                    // run starts after it has been written: one click does both.
                    ui.memory_mut(egui::Memory::stop_text_input);
                    self.run_asked = true;
                }
                ui.separator();
                // View
                let _ = icons::toggle(ui, &mut self.panels.nodes, icons::NODES, "Nodes panel");
                let _ = icons::toggle(
                    ui,
                    &mut self.panels.properties,
                    icons::PROPERTIES,
                    "Properties panel",
                );
                let _ = icons::toggle(
                    ui,
                    &mut self.panels.packages,
                    icons::PACKAGES,
                    "Packages panel",
                );
                let _ = icons::toggle(ui, &mut self.panels.log, icons::LOG, "Problems and log");
                if icons::button(ui, open, icons::FIT, "Fit the graph in view", "Home").clicked() {
                    self.canvas.fit();
                }
                if icons::button(ui, open, icons::ACTUAL_SIZE, "Actual size", "").clicked() {
                    self.canvas.reset_zoom();
                }
                let _ = icons::toggle(ui, &mut self.canvas.snap, icons::SNAP, "Snap to grid");
                ui.label(format!("{:.0}%", self.canvas.zoom() * 100.0));
                self.language_choice(ui, host);
                ui.separator();
                let Some(document) = &self.document else {
                    ui.label("No document is open.");
                    return;
                };
                let valid = document.problems.is_empty();
                let title = if document.unsaved {
                    format!("{} (not saved)", document.path)
                } else {
                    document.path.clone()
                };
                let path = document.path.clone();
                ui.label(match (running, valid) {
                    (true, _) => "Running",
                    (false, true) => "Ready",
                    (false, false) => "Has problems",
                });
                // The file's name; its whole path when pointed at.
                let name = title
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or(&title)
                    .to_owned();
                let _ = ui.strong(name).on_hover_text(path);
            });
        });
    }

    /// The language the node search also understands, from the host's search words.
    fn language_choice(&mut self, ui: &mut egui::Ui, host: &Host) {
        let languages = host.search_words().languages();
        let current = self.language().to_owned();
        let name = languages
            .iter()
            .find(|(tag, _)| *tag == current)
            .map_or(current.as_str(), |(_, name)| name)
            .to_owned();
        let mut chosen = None;
        let _ = egui::ComboBox::from_id_salt("orange-language")
            .selected_text((icons::text(icons::LANGUAGE), format!(" {name}")))
            .show_ui(ui, |ui| {
                for (tag, name) in &languages {
                    if ui.selectable_label(*tag == current, *name).clicked() {
                        chosen = Some((*tag).to_owned());
                    }
                }
            })
            .response
            .on_hover_text("The language node search understands, besides English.");
        if let Some(tag) = chosen {
            self.set_language(&tag);
        }
    }

    fn nodes_panel(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        ui.horizontal(|ui| {
            let _ = ui.selectable_value(&mut self.tab, Tab::Nodes, "Nodes");
            let _ = ui
                .selectable_value(&mut self.tab, Tab::Variables, "Variables")
                .on_hover_text("The values this graph keeps.");
            let _ = ui
                .selectable_value(&mut self.tab, Tab::Connections, "Connections")
                .on_hover_text("The devices and services this graph uses.");
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
        }
        let mut add = None;
        let mut variable_node = None;
        let mut new_variable = false;
        let _ = ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Search nodes")
                .desired_width(f32::INFINITY),
        );
        let variables = self.variables_to_use();
        egui::ScrollArea::vertical().show(ui, |ui| {
            let searching = !self.search.trim().is_empty();
            // The graph's own variables first: reading or writing one is the most
            // common node, and declaring one starts here too.
            if self.document.is_some() && !searching {
                egui::CollapsingHeader::new(format!("Variables ({})", variables.len()))
                    .id_salt("orange-variables")
                    .default_open(true)
                    .show(ui, |ui| {
                        for (id, name, ty) in &variables {
                            ui.horizontal(|ui| {
                                let _ = ui
                                    .add(egui::Label::new(name.as_str()).truncate())
                                    .on_hover_text(type_name(ty));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        for (label, definition) in VARIABLE_NODES.iter().rev() {
                                            if ui
                                                .small_button(*label)
                                                .on_hover_text(format!(
                                                    "Add a {} node for {name}.",
                                                    label.to_lowercase()
                                                ))
                                                .clicked()
                                            {
                                                variable_node = Some((*definition, id.clone()));
                                            }
                                        }
                                    },
                                );
                            });
                        }
                        new_variable = ui.button("New variable…").clicked();
                    });
            }
            let language = if self.language.is_empty() {
                orange_host::SOURCE_LANGUAGE
            } else {
                self.language.as_str()
            };
            let found = self.palette.get(host, &self.search, language);
            if searching && found.is_empty() {
                ui.weak("No node matches. Try a name or a purpose, such as int, timer or text.");
            }
            for (category, items) in found {
                // A search opens every section it found something in.
                egui::CollapsingHeader::new(format!("{category} ({})", items.len()))
                    .id_salt(category.as_str())
                    .default_open(searching)
                    .open(searching.then_some(true))
                    .show(ui, |ui| {
                        for item in items {
                            let hint = if item.purpose.is_empty() {
                                format!("{}\nClick to add it, or drag it onto the graph.", item.id)
                            } else {
                                format!(
                                    "{}\n{}\nClick to add it, or drag it onto the graph.",
                                    item.purpose, item.id
                                )
                            };
                            // Dragged, it carries its kind to where it is let go.
                            let id = ui.scope_id().with(("orange-kind", item.id.as_str()));
                            let shown = ui.dnd_drag_source(id, item.id.clone(), |ui| {
                                ui.selectable_label(false, item.label.as_str())
                            });
                            if shown.inner.on_hover_text(hint).clicked() {
                                add = Some(item.id.clone());
                            }
                            // A search says what each found node is for.
                            if searching && !item.purpose.is_empty() {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(item.purpose.as_str()).small().weak(),
                                    )
                                    .truncate(),
                                );
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
                self.say("open a graph before adding nodes".to_owned());
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

    fn packages_panel(ui: &mut egui::Ui, host: &Host) {
        ui.heading("Packages");
        egui::ScrollArea::vertical().show(ui, |ui| {
            for package in host.packages() {
                let kind = match package.kind {
                    PackageKind::Base => "base",
                    PackageKind::Native => "built in",
                    _ => "installed",
                };
                let state = if package.active { "on" } else { "off" };
                ui.label(format!(
                    "{} {} ({kind}, {state})",
                    package.name().unwrap_or(package.id),
                    package.version
                ));
            }
        });
    }

    fn log_panel(&mut self, ui: &mut egui::Ui) {
        let problems = self.problems().to_vec();
        let nodes = self
            .document
            .as_ref()
            .map(|document| document.problem_nodes.clone())
            .unwrap_or_default();
        let mut focus = None;
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .show(ui, |ui| {
                if !problems.is_empty() {
                    ui.heading(format!("Problems ({})", problems.len()));
                    for (index, problem) in problems.iter().enumerate() {
                        // A problem about a node takes you to it.
                        match nodes.get(index).copied().flatten() {
                            Some(node) => {
                                if ui
                                    .link(problem.as_str())
                                    .on_hover_text("Show the node")
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
                    ui.separator();
                }
                ui.heading("Log");
                for line in &self.log {
                    ui.monospace(line.as_str());
                }
            });
        if let (Some(node), Some(document)) = (focus, &self.document) {
            self.canvas.focus(&document.graph, node);
        }
    }

    fn canvas_panel(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        let Some(document) = &self.document else {
            ui.centered_and_justified(|ui| ui.label("Open a graph file to see it here."));
            return;
        };
        let triggers = document
            .flow
            .as_ref()
            .map_or(&[][..], |(_, entries)| entries.as_slice());
        let done = self.canvas.show(ui, &document.graph, triggers);
        if done.moved {
            self.keep_layout();
        }
        if let Some(node) = done.trigger {
            self.trigger(host, node);
        }
        if let Some((from, to)) = done.connect {
            let _ = self.connect(host, (from.0, &from.1), (to.0, &to.1));
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
                    let _ = self.add_node_at(host, &definition, at);
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
            let _ = self.group_selection(host, "Group");
        }
        if let Some((target, at, screen)) = done.context {
            self.open_menu(target, at, screen);
        }
        self.menu_popup(ui.ctx(), host);
        self.quick_add_popup(ui.ctx(), host);
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
        for n in 0..MAX_EVENTS_PER_POLL {
            sender
                .send(HostEvent {
                    flow,
                    kind: FlowEvent::TraceLost(n as u64),
                })
                .unwrap();
        }
        sender
            .send(HostEvent {
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
                .send(HostEvent {
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
        assert_eq!(count(&all), host.nodes().count());
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
        assert!(draw(&mut editor, &mut host) > 0);
        let with_panels = editor.canvas().size();
        editor.panels = Panels {
            nodes: false,
            packages: false,
            log: false,
            properties: false,
        };
        assert!(draw(&mut editor, &mut host) > 0);
        // The canvas takes the room the panels gave up.
        let alone = editor.canvas().size();
        assert!(alone[0] > with_panels[0] && alone[1] > with_panels[1]);
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
            match editor.add_node(&mut host, kind) {
                Some(node) => {
                    let graph = editor.graph().unwrap();
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
                        .any(|line| line.contains("no active package declares")),
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
        assert_eq!(exact_type("core/u64@1"), "u64");
        assert_eq!(type_name("core/list@1<core/i64@1>"), "list<i64>");
        assert_eq!(
            type_name("org.example.geo/point@1"),
            "org.example.geo/point"
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

/// Most outputs previewed one at a time when they cannot all be previewed together.
const MAX_PREVIEWED: usize = 64;

impl Editor {
    /// While nothing runs, shows beside each output of a pure node (a value, an
    /// operator) what it computes from what is wired into it, without running the
    /// graph: Greater wired to two Ints shows true or false as soon as it is wired.
    /// Outputs that depend on a trigger, a variable or an effect are left blank.
    pub fn preview_values(&mut self, host: &mut Host) {
        let Some(document) = &self.document else {
            return;
        };
        if document.flow.is_some() {
            return;
        }
        let Ok(text) = document.draft.text() else {
            return;
        };
        let wanted: Vec<(NodeId, orange_host::PortKey)> = document
            .graph
            .nodes
            .iter()
            .filter(|node| !node.entry && node.ports.iter().all(|port| !port.exec))
            .flat_map(|node| {
                node.ports
                    .iter()
                    .filter(|port| !port.input)
                    .filter_map(move |port| Some((node.id, port.id.parse().ok()?)))
            })
            .take(MAX_PREVIEWED)
            .collect();
        self.canvas.clear_run();
        if wanted.is_empty() {
            return;
        }
        // All at once when every one can be previewed; otherwise one by one, so one
        // output that cannot does not blank the rest.
        let found = match host.preview(&text, &wanted) {
            Ok(found) => found,
            Err(_) => wanted
                .iter()
                .filter_map(|one| host.preview(&text, std::slice::from_ref(one)).ok())
                .flatten()
                .collect(),
        };
        for previewed in found {
            if let Ok(value) = &previewed.value {
                self.canvas.show_value(
                    previewed.node,
                    previewed.port.to_string(),
                    orange_host::plain(value),
                    orange_host::inspect(value),
                );
            }
        }
    }
}

#[cfg(test)]
mod preview_tests {
    use super::*;
    use orange_host::HostOptions;

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
}
