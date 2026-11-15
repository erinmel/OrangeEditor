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
mod properties;

pub use properties::{COMMON_TYPES, port_label, set_at};

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
pub use palette::{PaletteItem, palette};

/// A type as a person reads it: `core/string@1` is "text". Other types keep their name
/// without the version.
pub fn type_name(ty: &str) -> String {
    match ty {
        "core/string@1" => "text".to_owned(),
        "core/bool@1" => "true or false".to_owned(),
        "core/i64@1" | "core/u64@1" | "core/i32@1" | "core/u32@1" => "whole number".to_owned(),
        "core/f64@1" | "core/f32@1" => "number".to_owned(),
        "core/bytes@1" => "bytes".to_owned(),
        "core/duration@1" => "length of time".to_owned(),
        // Core types lose their namespace everywhere in the expression.
        other => other.replace("core/", "").replace("@1", ""),
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
    /// The flow running this document, with the entries a person can fire.
    flow: Option<(FlowId, Vec<NodeId>)>,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileRequest {
    /// Start a new graph.
    New,
    /// Choose a graph file to open.
    Open,
    /// Choose where to save the open graph.
    SaveAs,
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
    document_tab: bool,
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
}

impl Editor {
    pub fn new() -> Self {
        Self::default()
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
        self.document = Some(Document {
            path,
            untitled: false,
            declared: host.declarations(&draft),
            names: Names::of_document(host, draft.document()),
            draft,
            graph,
            unsaved: false,
            problems: Vec::new(),
            problem_nodes: Vec::new(),
            flow: None,
        });
        self.check(host);
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
        document.names = Names::of_document(host, document.draft.document());
        document.graph = host.draft_view(&document.draft).unwrap_or_default();
        document.declared = host.declarations(&document.draft);
        document.unsaved = true;
        self.canvas.update(&document.graph);
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

    /// Shows the Document tab with the form for a new variable open.
    pub fn new_variable_form(&mut self) {
        self.panels.nodes = true;
        self.document_tab = true;
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
            if matches!(event.kind, FlowEvent::Finished) {
                document.flow = None;
            }
            for line in said {
                self.say(line);
            }
        }
        any
    }

    /// Draws the editor into `ui` and applies what the person did.
    pub fn ui(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        // Undo, redo and the file keys, unless a text field is being typed in.
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
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        egui::Panel::top("orange-toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                let _ = ui.toggle_value(&mut self.panels.nodes, "Nodes");
                let _ = ui.toggle_value(&mut self.panels.properties, "Properties");
                let _ = ui.toggle_value(&mut self.panels.packages, "Packages");
                let _ = ui.toggle_value(&mut self.panels.log, "Log");
                ui.separator();
                if self.file_dialogs {
                    if ui.button("New").on_hover_text("Ctrl+N").clicked() {
                        self.ask(FileRequest::New);
                    }
                    if ui.button("Open…").on_hover_text("Ctrl+O").clicked() {
                        self.ask(FileRequest::Open);
                    }
                }
                let Some(document) = &self.document else {
                    ui.label("No document is open.");
                    return;
                };
                let (running, valid) = (document.flow.is_some(), document.problems.is_empty());
                let title = if document.unsaved {
                    format!("{} (not saved)", document.path)
                } else {
                    document.path.clone()
                };
                let path = document.path.clone();
                if ui.button("Save").on_hover_text("Ctrl+S").clicked()
                    && let Err(message) = self.save()
                {
                    self.say(message);
                }
                if self.file_dialogs
                    && ui
                        .button("Save as…")
                        .on_hover_text("Ctrl+Shift+S")
                        .clicked()
                {
                    self.ask(FileRequest::SaveAs);
                }
                if ui.button("Check").clicked() {
                    self.check(host);
                }
                if ui
                    .button("New variable…")
                    .on_hover_text("Declare a variable of this graph.")
                    .clicked()
                {
                    self.new_variable_form();
                }
                if running {
                    if ui.button("Stop").clicked() {
                        self.stop(host);
                    }
                } else if ui.button("Run").clicked() {
                    self.run(host);
                }
                ui.separator();
                let (undo, redo) = self.can_undo();
                if ui.add_enabled(undo, egui::Button::new("Undo")).clicked() {
                    self.undo(host);
                }
                if ui.add_enabled(redo, egui::Button::new("Redo")).clicked() {
                    self.redo(host);
                }
                ui.separator();
                if ui.button("Fit").clicked() {
                    self.canvas.fit();
                }
                if ui.button("Actual size").clicked() {
                    self.canvas.reset_zoom();
                }
                ui.label(format!("{:.0}%", self.canvas.zoom() * 100.0));
                ui.separator();
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

    fn nodes_panel(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        ui.horizontal(|ui| {
            let _ = ui.selectable_value(&mut self.document_tab, false, "Nodes");
            let _ = ui
                .selectable_value(&mut self.document_tab, true, "Document")
                .on_hover_text("The variables and connections of this graph.");
        });
        ui.separator();
        if self.document_tab {
            self.document_tab(ui, host);
            return;
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
            for (category, items) in self.palette.get(host, &self.search) {
                egui::CollapsingHeader::new(format!("{category} ({})", items.len()))
                    .id_salt(category.as_str())
                    .default_open(searching)
                    .show(ui, |ui| {
                        for item in items {
                            let hint =
                                format!("{}\nClick to add it, or drag it onto the graph.", item.id);
                            // Dragged, it carries its kind to where it is let go.
                            let id = ui.scope_id().with(("orange-kind", item.id.as_str()));
                            let shown = ui.dnd_drag_source(id, item.id.clone(), |ui| {
                                ui.selectable_label(false, item.label.as_str())
                            });
                            if shown.inner.on_hover_text(hint).clicked() {
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
        if !done.remove.is_empty() {
            let _ = self.remove_nodes(host, &done.remove);
        }
        if let Some((node, port, typed)) = done.set_input {
            let _ = self.set_input(host, (node, &port), Some(&typed));
        }
        if (done.copy || done.cut)
            && let Some(clip) = self.copy(host)
        {
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
            let _ = self.add_node_at(host, &definition, at);
        }
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
    fn graph() -> String {
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
            .find(|item| item.label == "Whole number")
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
        editor.document_tab = true;
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
        // The toolbar opens the new-variable form under Document.
        editor.document_tab = false;
        editor.new_variable_form();
        assert!(editor.document_tab && editor.panels.nodes);
        assert!(draw(&mut editor, &mut host) > 0);
        // The quick form draws in the properties of the node it is for.
        editor.document_tab = false;
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
        assert_eq!(type_name("core/string@1"), "text");
        assert_eq!(type_name("core/i64@1"), "whole number");
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
