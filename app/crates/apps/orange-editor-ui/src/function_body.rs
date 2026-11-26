//! Edits the shared body behind a local function call.
use crate::canvas::{Canvas, CanvasResponse};
use crate::i18n::{t, tf};
use crate::{Editor, properties};
#[cfg(test)]
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
    select_after_refresh: Option<NodeId>,
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
            select_after_refresh: None,
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
            if let Some(node) = body.select_after_refresh.take() {
                body.canvas.select(Some(node));
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
        if let Err(error) = self.apply_function_edit(host, response, add)
            && let Some(body) = &mut self.function_body
        {
            body.error = Some(tf(
                "function-edit-error",
                &[("error", error.to_string().into())],
            ));
            ui.ctx().request_repaint();
        }
    }

    fn apply_function_edit(
        &mut self,
        host: &mut Host,
        done: CanvasResponse,
        add: Option<(String, [f32; 2])>,
    ) -> Result<(), HostError> {
        let Some(body) = &mut self.function_body else {
            return Ok(());
        };
        let Some(document) = &mut self.document else {
            return Ok(());
        };
        let draft = &mut document.draft;
        if done.moved && draft.place(&body.canvas.places())? {
            document.unsaved = true;
        }
        let changed = if let Some((definition, at)) = add {
            let node = host.add_function_node(draft, &body.id, &definition, at)?;
            body.select_after_refresh = Some(node);
            true
        } else if let Some((from, to)) = done.connect {
            host.connect(draft, (from.0, &from.1), (to.0, &to.1))?
        } else if let Some((node, (from, to))) = done.insert {
            host.insert_into_wire(draft, node, (from.0, &from.1), (to.0, &to.1))?
        } else if let Some((from, to)) = done.remove_wire {
            host.disconnect_wire(draft, (from.0, &from.1), (to.0, &to.1))?
        } else if let Some((node, port, input)) = done.disconnect {
            host.disconnect(draft, (node, &port), input)?
        } else if let Some((node, port, value)) = done.set_input {
            host.set_input(draft, (node, &port), Some(&value))?
        } else if let Some((node, pointer, value)) = done.set_config {
            let Some(view) = body.node(node) else {
                return Ok(());
            };
            let mut settings = view.settings.clone();
            properties::set_at(&mut settings, &pointer, Some(value));
            host.set_config(draft, node, &settings)?
        } else if let Some((node, name)) = done.set_name {
            host.rename_node(draft, node, &name)?
        } else if let Some((node, port)) = done.add_port {
            let _ = host.add_port(draft, node, &port)?;
            true
        } else if let Some((node, port)) = done.remove_port {
            host.remove_port(draft, (node, &port))?
        } else if !done.remove.is_empty() {
            let nodes: Vec<_> = done
                .remove
                .into_iter()
                .filter(|node| {
                    body.node(*node).is_some_and(|view| {
                        !matches!(
                            view.definition.as_str(),
                            orange_host::intrinsic::FUNCTION_ENTRY
                                | orange_host::intrinsic::FUNCTION_OUTPUTS
                                | orange_host::intrinsic::RETURN
                        )
                    })
                })
                .collect();
            !nodes.is_empty() && host.remove_nodes(draft, &nodes)?
        } else {
            false
        };
        if changed {
            body.error = None;
            self.refresh(host);
        }
        if let Some((node, port)) = done.edit_literal {
            self.begin_value_input(node, &port);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orange_host::{HostOptions, PortKey, Value};

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
