//! The editor interface of OrangeEditor, as an egui library.
//!
//! It is made of parts an application takes as it needs:
//!
//! - [`canvas::Canvas`] draws one graph and nothing else. An application with its own
//!   lists and panels shows only this.
//! - [`palette`] is the data of a node list, for an application that draws its own.
//! - [`Editor`] is the whole editor: a toolbar, the canvas, and panels for the node
//!   kinds, the packages, the problems and the log, each of which can be switched off
//!   ([`Panels`]).
//!
//! Everything reaches the product through `orange-host` alone: the palette is the
//! host's node list, a document is checked and run by the host, the canvas draws the
//! host's view of it, and the log is the host's events. Where nodes are placed is kept
//! in the document itself, so a layout travels with the file.
//!
//! Drawing never waits: every button calls one of the methods below, which are also
//! how tests and other hosts drive the editor without a window.
// Product code reports failures as typed errors; each remaining panic is a documented
// internal invariant (10, "Panic policy").
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

pub mod canvas;

use canvas::Canvas;
use orange_host::{
    FlowEvent, FlowId, GraphView, Host, HostError, HostEvent, Names, NodeId, PackageKind, StopMode,
    describe, lines, place,
};
use std::collections::VecDeque;
use std::sync::mpsc::Receiver;

/// Most lines the log keeps; older ones are dropped.
pub const MAX_LOG_LINES: usize = 500;

/// One node kind a document can use, as the palette lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteItem {
    /// The definition, such as `org.orange.files/write-text`.
    pub id: String,
    pub label: String,
    /// The package that provides it; empty for the language's own nodes.
    pub package: String,
}

/// The host's nodes grouped by category, both in alphabetical order, keeping those
/// whose label, definition or category contains every word of `search` in any letter
/// case. An empty search keeps all.
pub fn palette(host: &Host, search: &str) -> Vec<(String, Vec<PaletteItem>)> {
    let search = search.to_lowercase();
    let words: Vec<&str> = search.split_whitespace().collect();
    let mut groups: std::collections::BTreeMap<String, Vec<PaletteItem>> = Default::default();
    for info in host.nodes() {
        let (label, category) = (info.label().into_owned(), info.category().into_owned());
        let id = info.definition.definition.id.as_str().to_owned();
        let text = format!("{label} {id} {category}").to_lowercase();
        if !words.iter().all(|word| text.contains(word)) {
            continue;
        }
        groups.entry(category).or_default().push(PaletteItem {
            id,
            label,
            package: info
                .provider
                .map(|provider| provider.package.clone())
                .unwrap_or_default(),
        });
    }
    groups
        .into_iter()
        .map(|(category, mut items)| {
            items.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.id.cmp(&b.id)));
            (category, items)
        })
        .collect()
}

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

/// The kinds of node that name one of the document's variables in their settings.
const VARIABLE_NODES: [&str; 4] = [
    "core/get-variable",
    "core/set-variable",
    "core/update-variable",
    "core/on-variable-changed",
];

/// The document being edited.
#[derive(Debug)]
struct Document {
    path: String,
    text: String,
    names: Names,
    /// What the canvas draws.
    graph: GraphView,
    /// The layout changed since the document was opened or saved.
    unsaved: bool,
    /// What checking found, one line each; empty when the document is valid.
    problems: Vec<String>,
    /// The flow running this document, with the entries a person can fire.
    flow: Option<(FlowId, Vec<NodeId>)>,
    /// The document before each edit, oldest first, and the edits undone since.
    undo: Vec<String>,
    redo: Vec<String>,
}

/// Most edits that can be undone.
pub const MAX_UNDO: usize = 200;

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

/// The state of the editor between frames.
#[derive(Debug, Default)]
pub struct Editor {
    search: String,
    document: Option<Document>,
    canvas: Canvas,
    log: VecDeque<String>,
    /// The node whose values the properties panel is editing, and what is typed in
    /// each of its fields.
    inspected: Option<NodeId>,
    typed: std::collections::BTreeMap<String, String>,
    /// The panels shown; a host application may set them, and the toolbar switches them.
    pub panels: Panels,
}

impl Editor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Shows a document: `text` is the saved graph and `path` what it is called. It is
    /// checked at once; a flow of the previous document is closed.
    pub fn open(&mut self, host: &mut Host, path: impl Into<String>, text: String) {
        self.stop(host);
        let names = Names::of(host, &text);
        // A text that is not a document has nothing to draw; checking says why.
        let graph = host.view(&text).unwrap_or_default();
        self.canvas.load(&graph);
        self.document = Some(Document {
            path: path.into(),
            text,
            names,
            graph,
            unsaved: false,
            undo: Vec::new(),
            redo: Vec::new(),
            problems: Vec::new(),
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
    pub fn text(&self) -> Option<&str> {
        self.document
            .as_ref()
            .map(|document| document.text.as_str())
    }

    /// Whether the layout changed since the document was opened or saved.
    pub fn is_unsaved(&self) -> bool {
        self.document
            .as_ref()
            .is_some_and(|document| document.unsaved)
    }

    /// Writes where the canvas has the nodes into the document's layout. Layout is
    /// not part of what a graph means, so a running flow is not touched.
    pub fn keep_layout(&mut self) {
        let Some(document) = &mut self.document else {
            return;
        };
        if let Ok(text) = place(&document.text, &self.canvas.places()) {
            document.unsaved |= text != document.text;
            document.text = text;
        }
    }

    /// Makes `text` the document: the canvas, the problems and a running flow follow.
    fn show_text(&mut self, host: &mut Host, text: String) {
        let Some(document) = &mut self.document else {
            return;
        };
        document.text = text;
        document.names = Names::of(host, &document.text);
        document.graph = host.view(&document.text).unwrap_or_default();
        document.unsaved = true;
        self.canvas.update(&document.graph);
        // The fields of the properties panel are read again from the new document.
        self.inspected = None;
        let mut said = None;
        if let Some((flow, entries)) = &mut document.flow {
            match host.update(*flow, &document.text) {
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

    /// Applies the result of an edit. A refused edit says why in the log and changes
    /// nothing. Returns whether the document changed.
    fn edited(&mut self, host: &mut Host, result: Result<String, HostError>) -> bool {
        let Some(document) = &mut self.document else {
            return false;
        };
        match result {
            Ok(text) if text == document.text => false,
            Ok(text) => {
                document.undo.push(document.text.clone());
                if document.undo.len() > MAX_UNDO {
                    let _ = document.undo.remove(0);
                }
                document.redo.clear();
                self.show_text(host, text);
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

    /// The text an edit starts from: the document with the canvas's layout in it.
    fn editing(&mut self) -> Option<String> {
        self.keep_layout();
        self.text().map(str::to_owned)
    }

    /// Adds a node of kind `definition` in the middle of the canvas and selects it.
    pub fn add_node(&mut self, host: &mut Host, definition: &str) -> Option<NodeId> {
        let text = self.editing()?;
        let [x, y] = self.canvas.center();
        // Not exactly on top of a node added just before.
        #[expect(clippy::cast_precision_loss, reason = "a document has few nodes")]
        let step = (self.graph().map_or(0, |graph| graph.nodes.len()) % 8) as f32 * 24.0;
        let at = [x - canvas::NODE_WIDTH / 2.0 + step, y - 40.0 + step];
        match host.add_node(&text, definition, at) {
            Ok((text, node)) => {
                let _ = self.edited(host, Ok(text));
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
        let Some(text) = self.editing() else {
            return false;
        };
        let result = host.remove_nodes(&text, &[node]);
        self.edited(host, result)
    }

    /// Wires an output to an input. A wire the language does not allow is refused
    /// with the reason in the log.
    pub fn connect(&mut self, host: &mut Host, from: (NodeId, &str), to: (NodeId, &str)) -> bool {
        let Some(text) = self.editing() else {
            return false;
        };
        let result = host.connect(&text, from, to);
        self.edited(host, result)
    }

    /// Takes the wires off one port.
    pub fn disconnect(&mut self, host: &mut Host, at: (NodeId, &str), input: bool) -> bool {
        let Some(text) = self.editing() else {
            return false;
        };
        let result = host.disconnect(&text, at, input);
        self.edited(host, result)
    }

    /// Writes a value on an input, typed plainly; `None` takes it off.
    pub fn set_input(&mut self, host: &mut Host, at: (NodeId, &str), typed: Option<&str>) -> bool {
        let Some(text) = self.editing() else {
            return false;
        };
        let result = host.set_input(&text, at, typed);
        self.edited(host, result)
    }

    /// The variables the open document declares, as (ID, name), and the one `node`
    /// names in its settings.
    pub fn variables(&self, node: NodeId) -> (Vec<(String, String)>, Option<String>) {
        let saved: serde_json::Value = self
            .text()
            .and_then(|text| serde_json::from_str(text).ok())
            .unwrap_or_default();
        let text = |value: &serde_json::Value| value.as_str().map(str::to_owned);
        let declared = saved["variables"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|variable| Some((text(&variable["id"])?, text(&variable["name"])?)))
            .collect();
        let id = node.to_string();
        let named = saved["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|found| found["id"] == id.as_str())
            .and_then(|found| text(&found["config"]["variable"]));
        (declared, named)
    }

    /// Makes a variable node name the variable with ID `variable`, keeping its other
    /// settings.
    pub fn set_variable(&mut self, host: &mut Host, node: NodeId, variable: &str) -> bool {
        let Some(text) = self.editing() else {
            return false;
        };
        let saved: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        let id = node.to_string();
        let mut config = saved["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|found| found["id"] == id.as_str())
            .map(|found| found["config"].clone())
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::json!({}));
        config["variable"] = serde_json::Value::String(variable.to_owned());
        let result = host.set_config(&text, node, &config);
        self.edited(host, result)
    }

    /// Whether there is an edit to undo, and one to do again.
    pub fn can_undo(&self) -> (bool, bool) {
        self.document.as_ref().map_or((false, false), |document| {
            (!document.undo.is_empty(), !document.redo.is_empty())
        })
    }

    /// Takes back the last edit.
    pub fn undo(&mut self, host: &mut Host) {
        let Some(document) = &mut self.document else {
            return;
        };
        if let Some(before) = document.undo.pop() {
            document.redo.push(document.text.clone());
            self.show_text(host, before);
        }
    }

    /// Does again the edit last taken back.
    pub fn redo(&mut self, host: &mut Host) {
        let Some(document) = &mut self.document else {
            return;
        };
        if let Some(after) = document.redo.pop() {
            document.undo.push(document.text.clone());
            self.show_text(host, after);
        }
    }

    /// Saves the open document to its path, keeping the previous version beside it.
    pub fn save(&mut self) -> Result<(), String> {
        self.keep_layout();
        let Some(document) = &mut self.document else {
            return Ok(());
        };
        let path = std::path::Path::new(&document.path);
        orange_host::save_document(path, &document.text).map_err(|error| error.to_string())?;
        document.unsaved = false;
        let line = format!("saved {}", document.path);
        self.say(line);
        Ok(())
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
        document.problems = match host.check(&document.text) {
            Ok(diagnostics) => diagnostics
                .iter()
                .map(|diagnostic| describe(diagnostic, &document.names))
                .collect(),
            Err(error) => Self::problems_of(&error, &document.names),
        };
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
        match host.open(&document.text) {
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

    /// Takes what the host reported since the last call into the log. Returns whether
    /// anything arrived, so a window knows to draw again.
    pub fn take_events(&mut self, events: &Receiver<HostEvent>) -> bool {
        let mut any = false;
        while let Ok(event) = events.try_recv() {
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
        // Undo and redo by keyboard, unless a text field is being typed in.
        if !ui.ctx().egui_wants_keyboard_input() {
            let (undo, redo) = ui.input(|input| {
                let command = input.modifiers.command;
                let z = input.key_pressed(egui::Key::Z);
                (
                    command && z && !input.modifiers.shift,
                    command && (input.key_pressed(egui::Key::Y) || (z && input.modifiers.shift)),
                )
            });
            if undo {
                self.undo(host);
            }
            if redo {
                self.redo(host);
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
                let Some(document) = &self.document else {
                    ui.label("No document is open.");
                    return;
                };
                let (running, valid) = (document.flow.is_some(), document.problems.is_empty());
                let title = if document.unsaved {
                    format!("{} (layout not saved)", document.path)
                } else {
                    document.path.clone()
                };
                let path = document.path.clone();
                if ui.button("Save").clicked()
                    && let Err(message) = self.save()
                {
                    self.say(message);
                }
                if ui.button("Check").clicked() {
                    self.check(host);
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
        let mut add = None;
        ui.heading("Nodes");
        let _ = ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Search nodes")
                .desired_width(f32::INFINITY),
        );
        egui::ScrollArea::vertical().show(ui, |ui| {
            let searching = !self.search.trim().is_empty();
            for (category, items) in palette(host, &self.search) {
                egui::CollapsingHeader::new(format!("{category} ({})", items.len()))
                    .id_salt(category.as_str())
                    .default_open(searching)
                    .show(ui, |ui| {
                        for item in items {
                            let hint = format!("{}\nClick to add it to the graph.", item.id);
                            if ui
                                .selectable_label(false, item.label.as_str())
                                .on_hover_text(hint)
                                .clicked()
                            {
                                add = Some(item.id);
                            }
                        }
                    });
            }
        });
        if let Some(definition) = add {
            if self.document.is_some() {
                let _ = self.add_node(host, &definition);
            } else {
                self.say("open a graph before adding nodes".to_owned());
            }
        }
    }

    fn properties_panel(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        ui.heading("Properties");
        let selected = self.canvas.selected().and_then(|id| {
            self.graph()?
                .nodes
                .iter()
                .find(|node| node.id == id)
                .cloned()
        });
        let Some(node) = selected else {
            self.inspected = None;
            ui.weak("Select a node to see and change its values.");
            return;
        };
        if self.inspected != Some(node.id) {
            self.inspected = Some(node.id);
            self.typed = node
                .ports
                .iter()
                .filter(|port| port.input && !port.exec)
                .map(|port| {
                    let shown = port.value.clone().unwrap_or_default();
                    // Text is shown in quotes on a card and typed without them.
                    let typed = serde_json::from_str::<String>(&shown).unwrap_or(shown);
                    (port.id.clone(), typed)
                })
                .collect();
        }
        ui.strong(node.name.as_str());
        ui.weak(node.kind.as_str());
        ui.weak(node.definition.as_str());
        ui.separator();
        if VARIABLE_NODES.contains(&node.definition.as_str()) {
            let (declared, named) = self.variables(node.id);
            let shown = named
                .as_ref()
                .and_then(|id| declared.iter().find(|(known, _)| known == id))
                .map_or("choose a variable", |(_, name)| name.as_str())
                .to_owned();
            ui.label("variable");
            let mut chosen = None;
            let _ = egui::ComboBox::from_id_salt("orange-variable")
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for (id, name) in &declared {
                        let current = named.as_deref() == Some(id.as_str());
                        if ui.selectable_label(current, name.as_str()).clicked() && !current {
                            chosen = Some(id.clone());
                        }
                    }
                    if declared.is_empty() {
                        ui.weak("the document declares no variables");
                    }
                });
            if let Some(variable) = chosen {
                let _ = self.set_variable(host, node.id, &variable);
            }
            ui.separator();
        }
        // What to do once the fields are drawn: a value to write or take off.
        let mut write: Option<(String, Option<String>)> = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for port in node.ports.iter().filter(|port| port.input && !port.exec) {
                let ty = port.ty.as_deref().map_or("value".to_owned(), type_name);
                ui.label(format!("{} ({ty})", port.id));
                if port.connected {
                    ui.weak("comes from a wire");
                    continue;
                }
                let typed = self.typed.entry(port.id.clone()).or_default();
                ui.horizontal(|ui| {
                    let field = ui.add(
                        egui::TextEdit::singleline(typed)
                            .hint_text("no value")
                            .desired_width(150.0),
                    );
                    let entered =
                        field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    if entered || ui.small_button("Set").clicked() {
                        write = Some((port.id.clone(), Some(typed.clone())));
                    }
                    if port.value.is_some() && ui.small_button("Clear").clicked() {
                        write = Some((port.id.clone(), None));
                    }
                });
            }
            ui.separator();
            if ui.button("Delete node").clicked() {
                let _ = self.remove_node(host, node.id);
            }
        });
        if let Some((port, typed)) = write {
            let _ = self.set_input(host, (node.id, &port), typed.as_deref());
        }
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

    fn log_panel(&self, ui: &mut egui::Ui) {
        let problems = self.problems();
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .show(ui, |ui| {
                if !problems.is_empty() {
                    ui.heading(format!("Problems ({})", problems.len()));
                    for problem in problems {
                        ui.label(problem.as_str());
                    }
                    ui.separator();
                }
                ui.heading("Log");
                for line in &self.log {
                    ui.monospace(line.as_str());
                }
            });
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
        if let Some(node) = done.remove {
            let _ = self.remove_node(host, node);
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
        let (declared, named) = editor.variables(set);
        assert_eq!((declared.len(), named), (1, None));
        assert_eq!(declared[0].1, "count");
        assert!(!editor.problems().is_empty());
        assert!(editor.set_variable(&mut host, set, &declared[0].0));
        assert_eq!(
            editor.variables(set).1.as_deref(),
            Some(declared[0].0.as_str())
        );
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
