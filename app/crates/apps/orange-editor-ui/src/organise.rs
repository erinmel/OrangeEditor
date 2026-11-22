//! Organising the canvas from the editor: grouping the selection, changing and taking
//! away groups, making the selection a function after a preview of its interface, and
//! arranging the selection along its wires. Each is one edit of the draft.
use crate::canvas::GroupEdit;
use crate::controls::{Action, Surface};
use crate::i18n::{t, tf};
use crate::{Editor, FileRequest, controls};
use orange_host::{Extraction, FunctionPackagePreview, Group, Host, HostError, NodeId, Publish};

#[derive(Debug, Clone)]
pub(crate) struct NodeRename {
    pub(crate) node: NodeId,
    pub(crate) name: String,
    pub(crate) focus: bool,
}

impl NodeRename {
    pub(crate) fn new(node: NodeId, name: String) -> Self {
        Self {
            node,
            name,
            focus: true,
        }
    }

    // Some(true) commits the name. Some(false) discards the pending edit.
    pub(crate) fn field(
        &mut self,
        ui: &mut egui::Ui,
        font: impl Into<egui::FontSelection>,
    ) -> Option<bool> {
        let id = ui.make_persistent_id(("rename-node", self.node));
        let height = ui.available_height();
        rename_field(
            ui,
            id,
            &mut self.name,
            &mut self.focus,
            font,
            height,
            Some(t("rename-node-default")),
        )
    }
}

/// Edits a title in place. Enter commits; Escape and focus loss discard it.
pub(crate) fn rename_field(
    ui: &mut egui::Ui,
    id: egui::Id,
    name: &mut String,
    focus: &mut bool,
    font: impl Into<egui::FontSelection>,
    height: f32,
    hint: Option<&str>,
) -> Option<bool> {
    let [save, cancel] = if ui.memory(|memory| memory.has_focus(id)) {
        controls::consume_plain_keys(ui, [egui::Key::Enter, egui::Key::Escape])
    } else {
        [false, false]
    };
    let end = name.chars().count();
    let mut field = egui::TextEdit::singleline(name)
        .id(id)
        .font(font)
        .align(egui::Align2::LEFT_CENTER)
        .min_size(egui::vec2(0.0, height))
        .margin(egui::Margin::symmetric(3, 0))
        .text_color(crate::canvas::node_title_color(ui.visuals()))
        .desired_width(f32::INFINITY)
        .hint_text(hint.unwrap_or_default())
        .show(ui);
    let response = field
        .response
        .response
        .on_hover_text(t("rename-inline-hint"));
    if std::mem::take(focus) {
        use egui::text::{CCursor, CCursorRange};
        response.request_focus();
        field
            .state
            .cursor
            .set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(end))));
        field.state.store(ui.ctx(), id);
    } else if save || cancel || response.lost_focus() {
        response.surrender_focus();
        return Some(save && !cancel);
    }
    None
}

#[derive(Debug)]
pub(crate) struct FunctionExport {
    function: String,
    label: String,
    package: String,
    version: String,
    node: String,
    preview: Result<FunctionPackagePreview, String>,
    error: Option<String>,
}

/// A function being made of the selected nodes: their preview and the name typed.
#[derive(Debug)]
pub(crate) struct Extracting {
    nodes: Vec<NodeId>,
    name: String,
    preview: Result<Extraction, String>,
}

impl Editor {
    pub(crate) fn call_function(&self, node: NodeId) -> Option<&str> {
        let document = self.document.as_ref()?.draft.document();
        let instance = document.root().nodes.get(&node)?;
        if instance.definition.id.as_str() == "core/call" {
            instance.config.get("function")?.as_str()
        } else {
            None
        }
    }

    pub(crate) fn begin_function_export(&mut self, host: &Host, node: NodeId) -> bool {
        let Some(function) = self.call_function(node).map(str::to_owned) else {
            return false;
        };
        let Some(label) = self.graph().and_then(|graph| {
            graph
                .nodes
                .iter()
                .find(|view| view.id == node)
                .map(|view| view.name.clone())
        }) else {
            return false;
        };
        let Some(text) = self.text() else {
            return false;
        };
        let slug = label
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("-")
            .to_ascii_lowercase();
        let preview = host
            .function_package_preview(&text, &function)
            .map_err(|error| export_error(&error));
        self.function_export = Some(FunctionExport {
            function,
            label,
            package: String::new(),
            version: "0.1.0".to_owned(),
            node: slug,
            preview,
            error: None,
        });
        true
    }

    pub(crate) fn function_export_dialog(&mut self, ctx: &egui::Context, host: &Host) {
        let Some(export) = &mut self.function_export else {
            return;
        };
        let modal = egui::Modal::new(egui::Id::unique("function-export")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.heading(t("export-function-title"));
            ui.strong(&export.label);
            let Ok(preview) = &export.preview else {
                if let Err(error) = &export.preview {
                    ui.label(error);
                }
                return (
                    controls::button(
                        ui,
                        Surface::Dialog,
                        Action::Outline,
                        true,
                        egui::Button::new(t("close")),
                    )
                    .clicked(),
                    false,
                );
            };
            egui::Grid::new("function-package-fields")
                .num_columns(2)
                .show(ui, |ui| {
                    for (label, field) in [
                        (t("function-package-id"), &mut export.package),
                        (t("function-package-version"), &mut export.version),
                        (t("function-package-node"), &mut export.node),
                    ] {
                        let label = ui.label(label);
                        ui.text_edit_singleline(field).labelled_by(label.id);
                        ui.end_row();
                    }
                });
            egui::ScrollArea::vertical()
                .max_height(200.0)
                .show(ui, |ui| {
                    preview_ports(ui, &preview.inputs, &preview.outputs);
                    ui.strong(t("function-package-dependencies"));
                    if preview.dependencies.is_empty() {
                        ui.weak(t("function-package-none"));
                    }
                    for (id, version) in &preview.dependencies {
                        ui.label(format!("{id} {version}"));
                    }
                    if let Some(adapter) = &preview.connection_adapter {
                        ui.label(tf(
                            "function-package-connection",
                            &[("adapter", adapter.as_str().into())],
                        ));
                    }
                });
            if let Some(error) = &export.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            let (cancel, save) = controls::dialog_actions(
                ui,
                Action::Primary,
                [&export.package, &export.version, &export.node]
                    .iter()
                    .all(|field| !field.trim().is_empty()),
                egui::Button::new(t("function-package-save")),
            );
            (cancel.clicked(), save.clicked())
        });
        let (cancel, save) = modal.inner;
        if cancel || modal.should_close() {
            self.function_export = None;
        } else if save {
            let text = self.text();
            let Some(export) = &mut self.function_export else {
                return;
            };
            let Some(text) = text else { return };
            let publish = Publish {
                package: export.package.trim(),
                version: export.version.trim(),
                node: export.node.trim(),
                label: Some(&export.label),
            };
            match host.export_function_package(&text, &export.function, &publish) {
                Ok(archive) => {
                    self.request = Some(FileRequest::ExportPackage {
                        name: format!("{}-{}.orangeplug", publish.package, publish.version),
                        archive,
                    });
                    self.function_export = None;
                }
                Err(error) => export.error = Some(export_error(&error)),
            }
        }
    }

    /// Renames a node for this graph. A blank name restores its default label.
    pub fn rename_node(&mut self, host: &mut Host, node: NodeId, name: &str) -> bool {
        let Some(draft) = self.editing() else {
            return false;
        };
        let changed = host.rename_node(draft, node, name);
        self.edited(host, changed)
    }

    pub(crate) fn begin_node_rename(&mut self, node: NodeId) -> bool {
        let Some(name) = self
            .graph()
            .and_then(|graph| graph.nodes.iter().find(|view| view.id == node))
            .map(|view| view.name.clone())
        else {
            return false;
        };
        self.canvas.cancel_node_rename();
        self.renaming_node = Some(NodeRename::new(node, name));
        self.canvas.select(Some(node));
        self.panels.properties = true;
        self.panels.packages = false;
        true
    }

    /// Groups the selected nodes under `name`, as one edit. Returns the group's ID.
    pub fn group_selection(&mut self, host: &mut Host, name: &str) -> Option<String> {
        let selection = self.canvas.selection();
        let draft = self.editing()?;
        let result = draft.group(&selection, name);
        self.edited_value(host, result)
    }

    /// Renames, recolours, collapses or opens a group, as one edit.
    pub fn change_group(&mut self, host: &mut Host, group: &Group) -> bool {
        let Some(draft) = self.editing() else {
            return false;
        };
        let result = draft.set_group(group);
        self.edited(host, result)
    }

    /// Takes a group's frame away and leaves its nodes.
    pub fn ungroup(&mut self, host: &mut Host, id: &str) -> bool {
        let Some(draft) = self.editing() else {
            return false;
        };
        let result = draft.ungroup(id);
        self.edited(host, result)
    }

    /// The groups of the open document.
    pub fn groups(&self) -> Vec<Group> {
        self.document
            .as_ref()
            .map(|document| document.draft.groups())
            .unwrap_or_default()
    }

    pub(crate) fn group_edit(&mut self, host: &mut Host, edit: GroupEdit) {
        let _ = match edit {
            GroupEdit::Change(group) => self.change_group(host, &group),
            GroupEdit::Ungroup(id) => self.ungroup(host, &id),
        };
    }

    /// Opens the preview of a function made of the selected nodes: its kind and typed
    /// inputs and outputs, or why the selection cannot be one.
    pub fn preview_function(&mut self, host: &Host) {
        let Some(document) = &self.document else {
            return;
        };
        let nodes = self.canvas.selection();
        let preview = host
            .extraction(&document.draft, &nodes)
            .map_err(|error| match error {
                HostError::Document(reason) => reason,
                other => other.to_string(),
            });
        let count = document.draft.document().functions().len();
        self.extracting = Some(Extracting {
            nodes,
            name: tf("function-name", &[("number", (count + 1).into())]),
            preview,
        });
    }

    /// Makes the selected nodes a function named `name`, with a call in their place,
    /// as one edit, and selects the call.
    pub fn extract_selection(&mut self, host: &mut Host, name: &str) -> Option<NodeId> {
        let nodes = self.canvas.selection();
        self.extract(host, &nodes, name)
    }

    fn extract(&mut self, host: &mut Host, nodes: &[NodeId], name: &str) -> Option<NodeId> {
        let places = self.canvas.places();
        let [left, top] = places
            .iter()
            .filter(|(node, _)| nodes.contains(node))
            .fold(None, |at: Option<[f32; 2]>, (_, [x, y])| {
                Some(at.map_or([*x, *y], |[ax, ay]| [ax.min(*x), ay.min(*y)]))
            })?;
        // The call goes where the selection began, but after every card that feeds
        // it, so its incoming wires run forward.
        let graph = self.graph()?;
        let feeding = graph
            .wires
            .iter()
            .filter(|wire| nodes.contains(&wire.to.0) && !nodes.contains(&wire.from.0))
            .filter_map(|wire| {
                let source = graph.nodes.iter().find(|node| node.id == wire.from.0)?;
                let [x, _] = places.iter().find(|(node, _)| *node == source.id)?.1;
                Some(x + crate::canvas::node_size(source).x)
            })
            .fold(f32::NEG_INFINITY, f32::max);
        let at = [left.max(feeding + FUNCTION_GAP), top];
        let draft = self.editing()?;
        match host.extract(draft, nodes, name, at) {
            Ok(call) => {
                let _ = self.edited(host, Ok(true));
                self.canvas.select(Some(call));
                Some(call)
            }
            Err(error) => {
                let _ = self.edited(host, Err(error));
                None
            }
        }
    }

    /// Lays the selected nodes out in columns along their wires, as one edit.
    pub fn arrange_selection(&mut self) -> bool {
        // Places dragged before are kept first, so the arrangement is a step of its own.
        self.keep_layout();
        let Some(document) = &self.document else {
            return false;
        };
        let selection = self.canvas.selection();
        if !self.canvas.arrange(&document.graph, &selection) {
            return false;
        }
        self.keep_layout();
        true
    }

    /// The window that previews a function before it is made.
    pub(crate) fn function_window(&mut self, ctx: &egui::Context, host: &mut Host) {
        let Some(extracting) = &mut self.extracting else {
            return;
        };
        let mut open = true;
        let mut make = false;
        let mut cancel = false;
        egui::Window::new(t("make-a-function"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| match &extracting.preview {
                Err(why) => {
                    ui.label(tf("not-a-function", &[("why", why.to_string().into())]));
                    if controls::button(
                        ui,
                        Surface::Dialog,
                        Action::Outline,
                        true,
                        egui::Button::new(t("close")),
                    )
                    .clicked()
                    {
                        cancel = true;
                    }
                }
                Ok(preview) => {
                    ui.label(t("make-function-help"));
                    ui.horizontal(|ui| {
                        ui.label(t("name"));
                        let _ = ui.text_edit_singleline(&mut extracting.name);
                    });
                    ui.label(tf(
                        "function-summary",
                        &[
                            (
                                "kind",
                                match preview.kind.as_str() {
                                    "pure" => t("function-kind-pure"),
                                    "volatile" => t("function-kind-volatile"),
                                    "action" => t("function-kind-action"),
                                    "async" => t("function-kind-async"),
                                    other => other,
                                }
                                .into(),
                            ),
                            ("count", extracting.nodes.len().into()),
                        ],
                    ));
                    preview_ports(ui, &preview.inputs, &preview.outputs);
                    let (dismiss, confirm) = controls::dialog_actions(
                        ui,
                        Action::Primary,
                        !extracting.name.trim().is_empty(),
                        egui::Button::new(t("make-function")),
                    );
                    make = confirm.clicked();
                    cancel = dismiss.clicked();
                }
            });
        // Enter makes the function once it has a name; Escape cancels.
        let (enter, escape) = ctx.input(|input| {
            (
                input.key_pressed(egui::Key::Enter),
                input.key_pressed(egui::Key::Escape),
            )
        });
        if enter && extracting.preview.is_ok() && !extracting.name.trim().is_empty() {
            make = true;
        }
        cancel |= escape;
        if make {
            let nodes = extracting.nodes.clone();
            let name = extracting.name.clone();
            self.extracting = None;
            let _ = self.extract(host, &nodes, &name);
        } else if cancel || !open {
            self.extracting = None;
        }
    }
}

/// Room between the cards that feed a new function call and the call.
const FUNCTION_GAP: f32 = 60.0;

fn export_error(error: &HostError) -> String {
    if let HostError::GraphInvalid(_, problems) = error
        && let Some(problem) = problems
            .iter()
            .find(|problem| problem.severity == orange_host::Severity::Error)
    {
        let names = orange_host::Names::default();
        return format!(
            "{}: {}",
            problem.code,
            crate::diagnostics::problem_text(problem, &names)
        );
    }
    error.to_string()
}

fn preview_ports(ui: &mut egui::Ui, inputs: &[(String, String)], outputs: &[(String, String)]) {
    for (heading, ports) in [(t("inputs"), inputs), (t("outputs"), outputs)] {
        ui.strong(heading);
        if ports.is_empty() {
            ui.weak(t("function-package-none"));
        }
        for (name, ty) in ports {
            ui.label(format!("{name}: {}", crate::type_name(ty)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orange_host::HostOptions;

    fn draw(editor: &mut Editor, host: &mut Host) -> usize {
        let context = egui::Context::default();
        let output = context.run_ui(egui::RawInput::default(), |ui| editor.ui(ui, host));
        let shapes = output.shapes.len();
        output.drop_without_applying_deltas();
        shapes
    }

    /// The test graph (`go` sets `count`) with a whole number far below it wired into
    /// the value: returns the editor, the number and the setter.
    fn opened(host: &mut Host) -> (Editor, NodeId, NodeId) {
        let mut editor = Editor::new();
        let text = host.bind(&crate::tests::graph()).unwrap();
        editor.open(host, "graph.json", text);
        let set = editor.graph().unwrap().nodes[1].id;
        let number = editor
            .add_node_at(host, "core/whole-number", [900.0, 700.0])
            .unwrap();
        assert!(editor.set_input(host, (number, "value"), Some("9")));
        assert!(editor.connect(host, (number, "result"), (set, "value")));
        assert!(!editor.document.as_ref().unwrap().run_blocked);
        assert!(
            editor
                .problems()
                .iter()
                .any(|problem| problem.contains("shadowed"))
        );
        (editor, number, set)
    }

    #[test]
    fn the_selection_is_grouped_collapsed_and_ungrouped_from_the_editor_with_undo() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let (mut editor, number, set) = opened(&mut host);
        editor.canvas().select_many(&[number, set]);
        let id = editor.group_selection(&mut host, "Setting").unwrap();
        let groups = editor.groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].nodes, [set, number]);
        assert_eq!(editor.canvas().groups(), groups.as_slice());
        assert!(editor.text().unwrap().contains("\"groups\""));
        assert!(draw(&mut editor, &mut host) > 0);

        // Collapsed, its nodes are hidden and a box no longer picks them.
        let mut collapsed = groups[0].clone();
        collapsed.collapsed = true;
        editor.group_edit(&mut host, GroupEdit::Change(collapsed));
        assert!(editor.canvas().is_hidden(set) && editor.canvas().is_hidden(number));
        let graph = editor.graph().unwrap().clone();
        let everywhere = egui::Rect::from_min_max(egui::pos2(-1e5, -1e5), egui::pos2(1e5, 1e5));
        assert_eq!(editor.canvas().nodes_in(&graph, everywhere).len(), 1);
        let group = editor.groups().remove(0);
        let frame = editor.canvas().group_frame(&graph, &group).unwrap();
        assert!(frame.width() > 0.0 && frame.height() < 100.0);
        assert!(draw(&mut editor, &mut host) > 0);

        editor.undo(&mut host);
        assert!(!editor.canvas().is_hidden(set));
        editor.group_edit(&mut host, GroupEdit::Ungroup(id));
        assert!(editor.groups().is_empty());
        editor.undo(&mut host);
        assert_eq!(editor.groups().len(), 1);
    }

    #[test]
    fn selected_nodes_are_arranged_along_their_wires_as_one_step() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let (mut editor, number, set) = opened(&mut host);
        let place = |editor: &mut Editor, node: NodeId| {
            editor
                .canvas()
                .places()
                .into_iter()
                .find(|(id, _)| *id == node)
                .unwrap()
                .1
        };
        let before = (place(&mut editor, number), place(&mut editor, set));
        editor.canvas().select_many(&[number]);
        assert!(!editor.arrange_selection());
        editor.canvas().select_many(&[number, set]);
        assert!(editor.arrange_selection());
        let (n, s) = (place(&mut editor, number), place(&mut editor, set));
        // The number feeds the setter, so it is the column to its left, at the top
        // left corner the two had.
        assert!(n[0] < s[0]);
        assert_eq!(n[1], s[1]);
        assert_eq!(
            [n[0], n[1]],
            [before.1[0].min(before.0[0]), before.1[1].min(before.0[1])]
        );
        assert!(!editor.arrange_selection());
        editor.undo(&mut host);
        assert_eq!(
            (place(&mut editor, number), place(&mut editor, set)),
            before
        );
    }

    #[test]
    fn the_export_menu_reviews_ports_and_hands_an_archive_to_the_application() {
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        let (mut editor, number, _) = opened(&mut host);
        editor.canvas().select(Some(number));
        let call = editor.extract_selection(&mut host, "Nine").unwrap();
        editor.open_menu(crate::MenuTarget::Node(call), [0.0; 2], egui::Pos2::ZERO);
        assert!(editor.run_command(&mut host, crate::Command::ExportFunction));
        let export = editor.function_export.as_mut().unwrap();
        assert_eq!(
            export.preview.as_ref().unwrap().outputs,
            [("result".into(), "core/i64@1".into())]
        );
        export.package = "org.example.numbers".into();

        let ctx = egui::Context::default();
        let frame = |editor: &mut Editor, events| {
            ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    editor.function_export_dialog(ui.ctx(), &host);
                },
            )
        };
        frame(&mut editor, vec![]).drop_without_applying_deltas();
        let output = frame(&mut editor, vec![]);
        let save = output
            .shapes
            .iter()
            .find_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape
                    && text.galley.text() == t("function-package-save")
                {
                    return Some(text.pos + text.galley.rect.center().to_vec2());
                }
                None
            })
            .unwrap();
        output.drop_without_applying_deltas();
        orange_testkit::ui::click(save, egui::PointerButton::Primary, |events| {
            frame(&mut editor, events).drop_without_applying_deltas();
        });
        let Some(FileRequest::ExportPackage { name, archive }) = editor.take_request() else {
            panic!("the application did not receive the archive");
        };
        assert_eq!(name, "org.example.numbers-0.1.0.orangeplug");
        assert!(
            host.install(&archive, orange_host::Grants::default())
                .is_ok()
        );
    }

    #[test]
    fn a_previewed_function_is_made_of_the_selection_and_taken_back() {
        let (mut host, _events) = Host::new(HostOptions::default()).unwrap();
        let (mut editor, number, set) = opened(&mut host);
        editor.canvas().select_many(&[number]);
        editor.preview_function(&host);
        let preview = editor.extracting.as_ref().unwrap().preview.clone().unwrap();
        assert_eq!(preview.kind, "pure");
        assert!(preview.inputs.is_empty());
        assert_eq!(
            preview.outputs,
            [("result".to_owned(), "core/i64@1".to_owned())]
        );
        assert!(draw(&mut editor, &mut host) > 0);
        editor.extracting = None;

        let call = editor.extract_selection(&mut host, "Nine").unwrap();
        assert_eq!(editor.canvas().selection(), [call]);
        let graph = editor.graph().unwrap();
        let shown = graph.nodes.iter().find(|node| node.id == call).unwrap();
        assert_eq!(shown.name, "Nine");
        assert!(graph.nodes.iter().all(|node| node.id != number));
        assert!(
            graph
                .wires
                .iter()
                .any(|wire| wire.from.0 == call && wire.to == (set, "value".to_owned()))
        );
        assert!(!editor.document.as_ref().unwrap().run_blocked);

        // A trigger cannot go into a function; the preview says so and nothing changes.
        let go = editor.graph().unwrap().nodes[0].id;
        editor.canvas().select_many(&[go]);
        editor.preview_function(&host);
        assert!(editor.extracting.as_ref().unwrap().preview.is_err());
        assert!(editor.extract_selection(&mut host, "Go").is_none());

        editor.undo(&mut host);
        assert!(
            editor
                .graph()
                .unwrap()
                .nodes
                .iter()
                .any(|node| node.id == number)
        );
    }
}
