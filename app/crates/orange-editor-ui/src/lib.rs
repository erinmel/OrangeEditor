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
}

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
}
impl Default for Panels {
    fn default() -> Self {
        Self {
            nodes: true,
            packages: false,
            log: true,
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
        self.toolbar(ui, host);
        if self.panels.nodes {
            egui::Panel::left("orange-nodes")
                .resizable(true)
                .default_size(250.0)
                .show(ui, |ui| self.nodes_panel(ui, host));
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

    fn nodes_panel(&mut self, ui: &mut egui::Ui, host: &Host) {
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
                            let _ = ui
                                .label(item.label.as_str())
                                .on_hover_text(item.id.as_str());
                        }
                    });
            }
        });
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

    fn canvas_panel(&mut self, ui: &mut egui::Ui, host: &Host) {
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

    #[test]
    fn panels_can_be_switched_off_leaving_the_canvas() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        let bound = host.bind(&graph()).unwrap();
        editor.open(&mut host, "graph.json", bound);
        let all = draw(&mut editor, &mut host);
        editor.panels = Panels {
            nodes: false,
            packages: false,
            log: false,
        };
        let bare = draw(&mut editor, &mut host);
        assert!(bare > 0 && bare < all, "{bare} < {all}");
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
