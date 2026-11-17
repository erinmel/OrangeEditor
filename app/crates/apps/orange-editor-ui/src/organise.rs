//! Organising the canvas from the editor: grouping the selection, changing and taking
//! away groups, making the selection a function after a preview of its interface, and
//! arranging the selection along its wires. Each is one edit of the draft.
use crate::Editor;
use crate::canvas::GroupEdit;
use crate::i18n::{t, tf};
use orange_host::{Extraction, Group, Host, NodeId};

/// A function being made of the selected nodes: their preview and the name typed.
#[derive(Debug)]
pub(crate) struct Extracting {
    nodes: Vec<NodeId>,
    name: String,
    preview: Result<Extraction, String>,
}

impl Editor {
    /// Groups the selected nodes under `name`, as one edit. Returns the group's ID.
    pub fn group_selection(&mut self, host: &mut Host, name: &str) -> Option<String> {
        let selection = self.canvas.selection();
        let draft = self.editing()?;
        match draft.group(&selection, name) {
            Ok(id) => {
                let _ = self.edited(host, Ok(true));
                Some(id)
            }
            Err(error) => {
                let _ = self.edited(host, Err(error));
                None
            }
        }
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
            .map_err(|error| error.to_string());
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
        let at = places
            .iter()
            .filter(|(node, _)| nodes.contains(node))
            .fold(None, |at: Option<[f32; 2]>, (_, [x, y])| {
                Some(at.map_or([*x, *y], |[ax, ay]| [ax.min(*x), ay.min(*y)]))
            })?;
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
            .show(ctx, |ui| match &extracting.preview {
                Err(why) => {
                    ui.label(tf("not-a-function", &[("why", why.to_string().into())]));
                    if ui.button(t("close")).clicked() {
                        cancel = true;
                    }
                }
                Ok(preview) => {
                    ui.horizontal(|ui| {
                        ui.label(t("name"));
                        let _ = ui.text_edit_singleline(&mut extracting.name);
                    });
                    ui.label(tf(
                        "function-summary",
                        &[
                            ("kind", preview.kind.to_string().into()),
                            ("count", extracting.nodes.len().into()),
                        ],
                    ));
                    for (title, ports) in [
                        (t("inputs"), &preview.inputs),
                        (t("outputs"), &preview.outputs),
                    ] {
                        ui.strong(title);
                        if ports.is_empty() {
                            ui.weak("none");
                        }
                        for (name, ty) in ports {
                            ui.label(format!("{name}: {}", crate::type_name(ty)));
                        }
                    }
                    ui.horizontal(|ui| {
                        let named = !extracting.name.trim().is_empty();
                        if ui
                            .add_enabled(named, egui::Button::new(t("make-function")))
                            .clicked()
                        {
                            make = true;
                        }
                        if ui.button(t("cancel")).clicked() {
                            cancel = true;
                        }
                    });
                }
            });
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
        assert!(editor.problems().is_empty(), "{:?}", editor.problems());
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
        assert!(editor.problems().is_empty(), "{:?}", editor.problems());

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
