//! Edits the shared body behind a local function call.
use crate::canvas::{Canvas, CanvasResponse};
use crate::i18n::{t, tf};
use crate::{Editor, properties};
use orange_host::intrinsic;
use orange_host::{AuthoringView, Found, Host, HostError, NodeId, NodeView};

#[derive(Debug)]
pub(crate) struct FunctionBody {
    id: String,
    name: String,
    locale: String,
    revision: u64,
    view: AuthoringView,
    pub(crate) canvas: Canvas,
    /// What to select once the view shows the edit that made it (a new or pasted node).
    select_after_refresh: Vec<NodeId>,
    search: Option<NodeSearch>,
    insertion: Option<[f32; 2]>,
    error: Option<String>,
}

#[derive(Debug, Default)]
struct NodeSearch {
    query: String,
    choices: Option<Vec<Found>>,
}

impl FunctionBody {
    pub(crate) fn node(&self, id: NodeId) -> Option<&NodeView> {
        self.view
            .graph
            .nodes
            .iter()
            .find(|node| node.id == id)
            .map(|node| node.as_ref())
    }
}

impl Editor {
    /// Opens the shared graph behind a call in a separate canvas.
    pub fn open_function(&mut self, host: &Host, call: NodeId) -> bool {
        let Some(id) = self.call_function(call).map(str::to_owned) else {
            return false;
        };
        self.keep_layout();
        let Some(document) = &self.document else {
            return false;
        };
        let locale = self.language().to_owned();
        let Some((name, view)) = host.function_authoring_view(&document.draft, &id, &locale) else {
            return false;
        };
        let mut canvas = Canvas::default();
        canvas.load(&view.graph);
        canvas.set_failure_routes(view.failure_routes.clone());
        canvas.set_port_details(view.port_details.clone());
        canvas.set_channels(&document.declared);
        canvas.enums =
            crate::canvas::enums_of(host, &document.draft, &view.graph, &document.declared);
        self.function_body = Some(FunctionBody {
            id,
            name,
            locale,
            revision: document.draft.document().revision(),
            view,
            canvas,
            select_after_refresh: Vec::new(),
            search: None,
            insertion: None,
            error: None,
        });
        self.quick = None;
        self.menu = None;
        true
    }

    pub(crate) fn function_body_panel(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        let Some(mut body) = self.function_body.take() else {
            return;
        };
        let Some(document) = &self.document else {
            return;
        };
        let locale = self.language();
        let revision = document.draft.document().revision();
        if body.revision != revision || body.locale != locale {
            let Some((name, view)) =
                host.function_authoring_view(&document.draft, &body.id, locale)
            else {
                return;
            };
            body.name = name;
            if body.locale != locale {
                locale.clone_into(&mut body.locale);
                if let Some(search) = &mut body.search {
                    search.choices = None;
                }
            }
            body.revision = revision;
            body.canvas.restore(&view.graph);
            let chosen = std::mem::take(&mut body.select_after_refresh);
            if !chosen.is_empty() {
                body.canvas.select_many(&chosen);
            }
            body.canvas.set_failure_routes(view.failure_routes.clone());
            body.canvas.set_port_details(view.port_details.clone());
            body.canvas.set_channels(&document.declared);
            body.canvas.enums =
                crate::canvas::enums_of(host, &document.draft, &view.graph, &document.declared);
            body.view = view;
        }
        body.canvas.update_help(host, locale);
        body.canvas.shortcuts_enabled = self.value_form.is_none();
        let (can_undo, can_redo) = document.draft.can_undo();
        let (mut back, mut undo, mut redo, mut save) = (false, false, false, false);
        ui.horizontal_wrapped(|ui| {
            back = ui.button(t("back-to-graph")).clicked();
            ui.heading(&body.name);
            if ui.button(t("add-node-ellipsis")).clicked() {
                body.search = body.search.is_none().then(NodeSearch::default);
                body.insertion = None;
            }
            if ui.button(t("fit-to-view")).clicked() {
                body.canvas.fit();
            }
            undo = ui
                .add_enabled(can_undo, egui::Button::new(t("undo")))
                .clicked();
            redo = ui
                .add_enabled(can_redo, egui::Button::new(t("redo")))
                .clicked();
            save = ui.button(t("save")).clicked();
        });
        if back {
            ui.ctx().request_repaint();
            return;
        }
        if undo || redo {
            self.function_body = Some(body);
            if undo {
                self.undo(host);
            } else {
                self.redo(host);
            }
            ui.ctx().request_repaint();
            return;
        }
        if save {
            self.function_body = Some(body);
            if let Err(error) = self.save()
                && let Some(body) = &mut self.function_body
            {
                body.error = Some(error);
            }
            ui.ctx().request_repaint();
            return;
        }

        let mut add = None;
        if let Some(search) = &mut body.search {
            let id = ui.make_persistent_id("function-node-search");
            let enter = ui.memory(|memory| memory.has_focus(id))
                && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            let new_search = search.choices.is_none();
            let field = ui.add(
                egui::TextEdit::singleline(&mut search.query)
                    .id(id)
                    .hint_text(t("add-node-ellipsis")),
            );
            if new_search {
                field.request_focus();
            }
            if field.changed() || new_search {
                search.choices = Some(host.search_function_nodes(
                    &document.draft,
                    &body.id,
                    &search.query,
                    locale,
                ));
            }
            egui::ScrollArea::vertical()
                .max_height(140.0)
                .show(ui, |ui| {
                    for (index, choice) in search.choices.iter().flatten().enumerate() {
                        if ui
                            .button(choice.label.as_str())
                            .on_hover_text(choice.purpose.as_str())
                            .clicked()
                            || (index == 0 && enter)
                        {
                            add = Some(choice.definition.clone());
                        }
                    }
                });
        }
        if let Some(error) = &body.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        let stored =
            |node, port: &str| properties::stored_input_text(host, &document.draft, node, port);
        let response = ui
            .push_id(("function-body", &body.id), |ui| {
                body.canvas
                    .show_with_path_controls(ui, &body.view.graph, &[], false, Some(&stored))
            })
            .inner;
        if let Some((at, _)) = response.quick_add {
            body.insertion = Some(at);
            body.search = Some(NodeSearch::default());
            ui.ctx().request_repaint();
        }
        let graph = &body.view.graph;
        if let Some(target) = body.canvas.pending_inbound_givers() {
            body.canvas
                .set_inbound_givers(host.output_givers(&document.draft, graph, target));
            ui.ctx().request_repaint();
        }
        let add = add.map(|definition| {
            body.search = None;
            let at = body
                .insertion
                .take()
                .unwrap_or_else(|| body.canvas.center());
            (definition, at)
        });
        self.function_body = Some(body);
        let (copied, applied) = self.apply_function_edit(host, response, add);
        if let Some(clip) = copied {
            self.clip = Some(clip.clone());
            ui.ctx().copy_text(clip);
        }
        if let Err(error) = applied
            && let Some(body) = &mut self.function_body
        {
            body.error = Some(tf(
                "function-edit-error",
                &[("error", error.to_string().into())],
            ));
            ui.ctx().request_repaint();
        }
    }

    /// Applies everything the function canvas asked for this frame, in the order the
    /// main canvas does: moves, a node to add, wiring and values, renames and ports,
    /// then clipboard actions and removals. Each is its own undo step. The first edit
    /// the host refuses stops the rest, and what was already applied is shown. Returns
    /// the text copied or cut, for the system clipboard.
    fn apply_function_edit(
        &mut self,
        host: &mut Host,
        mut done: CanvasResponse,
        add: Option<(String, [f32; 2])>,
    ) -> (Option<String>, Result<(), HostError>) {
        let literal = done.edit_literal.take();
        let (Some(body), Some(document)) = (&mut self.function_body, &mut self.document) else {
            return (None, Ok(()));
        };
        let mut edits = BodyEdits::default();
        let result = edits.apply(host, &mut document.draft, body, done, add);
        if edits.changed {
            document.unsaved = true;
            if result.is_ok() {
                body.error = None;
            }
            self.refresh(host);
        }
        if let Some((node, port)) = literal {
            self.begin_value_input(node, &port);
        }
        (edits.copied, result)
    }
}

/// What applying one frame's edits to a function body did.
#[derive(Debug, Default)]
struct BodyEdits {
    changed: bool,
    copied: Option<String>,
}

impl BodyEdits {
    fn apply(
        &mut self,
        host: &Host,
        draft: &mut orange_host::Draft,
        body: &mut FunctionBody,
        done: CanvasResponse,
        add: Option<(String, [f32; 2])>,
    ) -> Result<(), HostError> {
        if done.moved {
            self.changed |= draft.place(&body.canvas.places())?;
        }
        if let Some((definition, at)) = add {
            let node = host.add_function_node(draft, &body.id, &definition, at)?;
            body.select_after_refresh = vec![node];
            self.changed = true;
        }
        if let Some((from, to)) = done.connect {
            self.changed |= host.connect(draft, (from.0, &from.1), (to.0, &to.1))?;
        }
        if let Some((node, (from, to))) = done.insert {
            self.changed |= host.insert_into_wire(draft, node, (from.0, &from.1), (to.0, &to.1))?;
        }
        if let Some((from, to)) = done.remove_wire {
            self.changed |= host.disconnect_wire(draft, (from.0, &from.1), (to.0, &to.1))?;
        }
        if let Some((node, port, input)) = done.disconnect {
            self.changed |= host.disconnect(draft, (node, &port), input)?;
        }
        if let Some((node, port, value)) = done.set_input {
            self.changed |= host.set_input(draft, (node, &port), Some(&value))?;
        }
        if let Some((node, pointer, value)) = done.set_config
            && let Some(view) = body.node(node)
        {
            let mut settings = view.settings.clone();
            properties::set_at(&mut settings, &pointer, Some(value));
            self.changed |= host.set_config(draft, node, &settings)?;
        }
        if let Some((node, name)) = done.set_name {
            self.changed |= host.rename_node(draft, node, &name)?;
        }
        if let Some((node, port)) = done.add_port {
            let _ = host.add_port(draft, node, &port)?;
            self.changed = true;
        }
        if let Some((node, port)) = done.remove_port {
            self.changed |= host.remove_port(draft, (node, &port))?;
        }
        // The function's own entry and result cards are its interface: they are never
        // copied, cut or deleted, so a copy can be pasted into any function.
        let editable = |node: &NodeId| {
            body.node(*node).is_some_and(|view| {
                !matches!(
                    view.definition.as_str(),
                    intrinsic::FUNCTION_ENTRY | intrinsic::FUNCTION_OUTPUTS | intrinsic::RETURN
                )
            })
        };
        let selection: Vec<NodeId> = body
            .canvas
            .selection()
            .into_iter()
            .filter(&editable)
            .collect();
        let mut remove = done.remove;
        remove.retain(&editable);
        if (done.copy || done.cut) && !selection.is_empty() {
            self.copied = Some(host.copy(draft, &selection)?);
        }
        if done.cut {
            remove.extend(selection);
        }
        if let Some(clip) = done.paste {
            let pasted =
                host.paste_into_function(draft, &body.id, &clip, body.canvas.paste_place())?;
            self.changed |= !pasted.is_empty();
            body.select_after_refresh = pasted;
        }
        remove.sort_unstable();
        remove.dedup();
        if !remove.is_empty() {
            self.changed |= host.remove_nodes(draft, &remove)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orange_host::{HostOptions, PortKey, Value};

    /// An editor with a function made of a number and a pass-through, its body open.
    fn open_body(host: &mut Host) -> (Editor, NodeId, NodeId) {
        let mut editor = Editor::new();
        let text = host.bind(&crate::tests::graph()).unwrap();
        editor.open(host, "graph.json", text);
        let sink = editor.graph().unwrap().nodes[1].id;
        let number = editor
            .add_node_at(host, intrinsic::WHOLE_NUMBER, [900.0, 700.0])
            .unwrap();
        let pass = editor
            .add_node_at(host, intrinsic::VALUE, [1200.0, 700.0])
            .unwrap();
        assert!(editor.set_input(host, (number, "value"), Some("9")));
        assert!(editor.connect(host, (number, "result"), (pass, "value")));
        assert!(editor.connect(host, (pass, "result"), (sink, "value")));
        editor.canvas().select_many(&[number, pass]);
        let call = editor.extract_selection(host, "Shared number").unwrap();
        assert!(editor.open_function(host, call));
        (editor, number, pass)
    }

    fn body_nodes(editor: &Editor) -> Vec<NodeId> {
        let body = editor.function_body.as_ref().unwrap();
        let document = editor.document.as_ref().unwrap().draft.document();
        let function = document
            .functions()
            .values()
            .find(|function| function.id.to_string() == body.id)
            .unwrap();
        function.body.nodes.keys().copied().collect()
    }

    #[test]
    fn every_edit_of_one_frame_is_applied_to_the_body() {
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        let (mut editor, number, pass) = open_body(&mut host);
        let extra = host
            .add_function_node(
                &mut editor.document.as_mut().unwrap().draft,
                &editor.function_body.as_ref().unwrap().id,
                intrinsic::WHOLE_NUMBER,
                [0.0, 0.0],
            )
            .unwrap();
        // A value and a wire in the same frame: both land.
        let (_, applied) = editor.apply_function_edit(
            &mut host,
            CanvasResponse {
                set_input: Some((extra, "value".to_owned(), "4".to_owned())),
                connect: Some(((extra, "result".to_owned()), (pass, "value".to_owned()))),
                ..CanvasResponse::default()
            },
            None,
        );
        applied.unwrap();
        let draft = &editor.document.as_ref().unwrap().draft;
        let text = draft.text().unwrap();
        assert!(text.contains("\"4\""), "the value was set");
        let id = &editor.function_body.as_ref().unwrap().id;
        let body = &draft
            .document()
            .functions()
            .values()
            .find(|function| function.id.to_string() == *id)
            .unwrap()
            .body;
        assert!(
            body.edges
                .values()
                .any(|edge| edge.from.node == extra && edge.to.node == pass),
            "the wire was made"
        );
        assert!(
            !body.edges.values().any(|edge| edge.from.node == number),
            "the new wire replaced the old one"
        );
    }

    #[test]
    fn copy_paste_and_cut_work_inside_a_function_body() {
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        let (mut editor, number, pass) = open_body(&mut host);
        let before = body_nodes(&editor);
        let root = editor.graph().unwrap().nodes.len();
        editor
            .function_body
            .as_mut()
            .unwrap()
            .canvas
            .select_many(&[number, pass]);
        let (copied, applied) = editor.apply_function_edit(
            &mut host,
            CanvasResponse {
                copy: true,
                ..CanvasResponse::default()
            },
            None,
        );
        applied.unwrap();
        let clip = copied.expect("copying gives clipboard text");
        let (_, applied) = editor.apply_function_edit(
            &mut host,
            CanvasResponse {
                paste: Some(clip),
                ..CanvasResponse::default()
            },
            None,
        );
        applied.unwrap();
        // The copies go into the function, not the graph around it.
        assert_eq!(body_nodes(&editor).len(), before.len() + 2);
        assert_eq!(editor.graph().unwrap().nodes.len(), root);

        // Cutting the interface cards with the rest leaves the interface in place.
        let everything = body_nodes(&editor);
        editor
            .function_body
            .as_mut()
            .unwrap()
            .canvas
            .select_many(&everything);
        let (copied, applied) = editor.apply_function_edit(
            &mut host,
            CanvasResponse {
                cut: true,
                ..CanvasResponse::default()
            },
            None,
        );
        applied.unwrap();
        assert!(copied.is_some());
        let left = body_nodes(&editor);
        assert!(!left.is_empty() && left.len() < everything.len());
        assert!(!left.contains(&number) && !left.contains(&pass));
        // One undo brings the cut nodes back.
        editor.undo(&mut host);
        assert_eq!(body_nodes(&editor).len(), everything.len());

        // A trigger cannot be part of a pure function, so pasting one changes nothing.
        let manual = editor
            .add_node_at(&mut host, intrinsic::ON_MANUAL, [0.0, 0.0])
            .unwrap();
        let function = editor.function_body.as_ref().unwrap().id.clone();
        let draft = &mut editor.document.as_mut().unwrap().draft;
        let clip = host.copy(draft, &[manual]).unwrap();
        let unchanged = draft.text().unwrap();
        assert!(
            host.paste_into_function(draft, &function, &clip, [0.0, 0.0])
                .is_err()
        );
        assert_eq!(draft.text().unwrap(), unchanged);
    }

    #[test]
    fn shared_function_edits_survive_reopen_and_compile_for_every_call() {
        let (mut host, _) = Host::new(HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        let text = host.bind(&crate::tests::graph()).unwrap();
        editor.open(&mut host, "graph.json", text);
        let sink = editor.graph().unwrap().nodes[1].id;
        let number = editor
            .add_node_at(&mut host, intrinsic::WHOLE_NUMBER, [900.0, 700.0])
            .unwrap();
        let pass = editor
            .add_node_at(&mut host, intrinsic::VALUE, [1200.0, 700.0])
            .unwrap();
        assert!(editor.set_input(&mut host, (number, "value"), Some("9")));
        assert!(editor.connect(&mut host, (number, "result"), (pass, "value")));
        assert!(editor.connect(&mut host, (pass, "result"), (sink, "value")));
        editor.canvas().select_many(&[number, pass]);
        let call = editor
            .extract_selection(&mut host, "Shared number")
            .unwrap();
        let function = editor.call_function(call).unwrap().to_owned();
        let output: PortKey = editor
            .graph()
            .unwrap()
            .nodes
            .iter()
            .find(|node| node.id == call)
            .unwrap()
            .ports
            .iter()
            .find(|port| !port.input && !port.exec)
            .unwrap()
            .id
            .parse()
            .unwrap();
        editor.canvas().select(Some(call));
        assert!(editor.duplicate(&mut host));
        let other = editor.canvas().selected().unwrap();
        assert_ne!(call, other);
        assert_eq!(editor.call_function(other), Some(function.as_str()));
        let results = |host: &mut Host, editor: &Editor| {
            let text = editor.document.as_ref().unwrap().draft.text().unwrap();
            host.preview(&text, &[(call, output.clone()), (other, output.clone())])
                .unwrap()
                .into_iter()
                .map(|result| result.value.unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(results(&mut host, &editor), [Value::I64(9), Value::I64(9)]);

        editor.open_menu(crate::MenuTarget::Node(call), [0.0, 0.0], egui::Pos2::ZERO);
        assert!(editor.run_command(&mut host, crate::Command::OpenFunction));
        let body = editor.function_body.as_ref().unwrap();
        assert_eq!(body.id, function);
        assert!(body.node(number).is_some());
        assert!(body.view.graph.nodes.iter().any(|node| {
            node.definition == intrinsic::FUNCTION_OUTPUTS && node.name == "Function outputs"
        }));
        assert!(body.node(sink).is_none());
        assert!(
            body.view
                .graph
                .wires
                .iter()
                .any(|wire| wire.from.0 == number && wire.to.0 == pass)
        );
        let ctx = egui::Context::default();
        let frame = ctx.run_ui(egui::RawInput::default(), |ui| editor.ui(ui, &mut host));
        frame.drop_without_applying_deltas();

        let draft = &mut editor.document.as_mut().unwrap().draft;
        let unchanged = draft.text().unwrap();
        assert!(
            host.connect(draft, (number, "result"), (sink, "value"))
                .is_err()
        );
        assert_eq!(draft.text().unwrap(), unchanged);
        editor
            .apply_function_edit(
                &mut host,
                CanvasResponse::default(),
                Some((intrinsic::WHOLE_NUMBER.to_owned(), [600.0, 500.0])),
            )
            .1
            .unwrap();
        let frame = ctx.run_ui(egui::RawInput::default(), |ui| editor.ui(ui, &mut host));
        frame.drop_without_applying_deltas();
        let new = editor
            .function_body
            .as_ref()
            .unwrap()
            .canvas
            .selected()
            .unwrap();
        editor
            .apply_function_edit(
                &mut host,
                CanvasResponse {
                    set_input: Some((new, "value".to_owned(), "17".to_owned())),
                    ..CanvasResponse::default()
                },
                None,
            )
            .1
            .unwrap();
        editor
            .apply_function_edit(
                &mut host,
                CanvasResponse {
                    connect: Some(((new, "result".to_owned()), (pass, "value".to_owned()))),
                    ..CanvasResponse::default()
                },
                None,
            )
            .1
            .unwrap();
        assert_eq!(
            results(&mut host, &editor),
            [Value::I64(17), Value::I64(17)]
        );

        editor.function_body = None;
        editor.undo(&mut host);
        assert_eq!(results(&mut host, &editor), [Value::I64(9), Value::I64(9)]);
        editor.redo(&mut host);
        let saved = editor.text().unwrap();
        editor.open_untitled(&mut host, saved);
        assert_eq!(editor.call_function(other), Some(function.as_str()));
        assert_eq!(
            results(&mut host, &editor),
            [Value::I64(17), Value::I64(17)]
        );
    }
}
