//! The shared typed-value form for card inputs, Properties, and manual entries.
use super::*;
use orange_host::{InputState, Literal, LiteralEditor, LiteralPayload, ManualPayload, TypeExpr};

enum Target {
    Input {
        graph: orange_host::GraphId,
        revision: u64,
        node: NodeId,
        port: String,
    },
    Manual(ManualPayload),
}

pub(crate) struct ValueForm {
    target: Target,
    title: String,
    ty: TypeExpr,
    editor: LiteralEditor,
    text: String,
    some: bool,
    focus: bool,
    error: Option<String>,
    json_error: Option<(usize, usize)>,
}

impl std::fmt::Debug for ValueForm {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("ValueForm").finish_non_exhaustive()
    }
}

impl ValueForm {
    fn new(
        target: Target,
        title: String,
        ty: TypeExpr,
        editor: LiteralEditor,
        current: Option<&Literal>,
    ) -> Self {
        let value = current.and_then(|literal| match &literal.payload {
            LiteralPayload::Value(value) => Some(value),
            LiteralPayload::Bits(_) => None,
        });
        let some = value.and_then(|value| value.get("some"));
        let text = match editor {
            LiteralEditor::Option { .. } => some,
            LiteralEditor::Structured => value,
            _ => None,
        }
        .and_then(|value| serde_json::to_string_pretty(value).ok())
        .unwrap_or_default();
        Self {
            target,
            title,
            ty,
            editor,
            text,
            some: some.is_some(),
            focus: true,
            error: None,
            json_error: None,
        }
    }

    fn literal(&self) -> Result<Literal, String> {
        let nested = || {
            serde_json::from_str::<serde_json::Value>(&self.text)
                .map_err(|error| json_syntax_message(error.line(), error.column()))
        };
        let value = match self.editor {
            LiteralEditor::Plain if self.ty.as_str() == "core/string@1" => {
                serde_json::Value::String(self.text.clone())
            }
            LiteralEditor::Plain => {
                let text = orange_host::literal(self.ty.as_str(), &self.text)?;
                return serde_json::from_str(&text).map_err(|error| error.to_string());
            }
            LiteralEditor::Option { some: true } if self.some => {
                serde_json::json!({"some": nested()?})
            }
            LiteralEditor::Option { .. } => serde_json::json!({"none": true}),
            LiteralEditor::Structured => nested()?,
            LiteralEditor::Unavailable => return Err(t("literal-unavailable").to_owned()),
        };
        Ok(Literal::value(self.ty.clone(), value))
    }

    fn fields(&mut self, ui: &mut egui::Ui) -> bool {
        ui.strong(&self.title);
        ui.weak(type_name(self.ty.as_str()));
        let was_some = self.some;
        if let LiteralEditor::Option { some } = self.editor {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.some, false, t("literal-none"));
                ui.add_enabled_ui(some, |ui| {
                    ui.selectable_value(&mut self.some, true, t("literal-some"))
                });
            });
        }
        if self.some != was_some {
            self.error = None;
        }
        let shows_text = !matches!(self.editor, LiteralEditor::Option { .. }) || self.some;
        if shows_text {
            let response = if self.editor == LiteralEditor::Plain {
                ui.add(egui::TextEdit::singleline(&mut self.text).desired_width(f32::INFINITY))
            } else {
                self.json_editor(ui)
            };
            if response.changed() {
                self.error = None;
            }
            if std::mem::take(&mut self.focus) {
                response.request_focus();
            }
        }
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        let action = if matches!(self.target, Target::Manual(_)) {
            t("fire-with-value")
        } else {
            t("apply")
        };
        ui.add_enabled(
            !shows_text || self.json_error.is_none(),
            egui::Button::new(action),
        )
        .clicked()
    }

    fn json_editor(&mut self, ui: &mut egui::Ui) -> egui::Response {
        let height = (ui.ctx().content_rect().height() * 0.45).clamp(64.0, 240.0);
        let response = egui::ScrollArea::vertical()
            .id_salt("literal-json")
            .max_height(height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.text)
                        .code_editor()
                        .desired_rows(8)
                        .desired_width(f32::INFINITY)
                        .hint_text(t("literal-json-hint")),
                )
            })
            .inner;
        if response.changed() {
            self.json_error = serde_json::from_str::<serde_json::Value>(&self.text)
                .err()
                .map(|error| (error.line(), error.column()));
        }
        let can_format = self.json_error.is_none() && !self.text.trim().is_empty();
        if ui
            .add_enabled(can_format, egui::Button::new(t("literal-format-json")))
            .clicked()
        {
            self.format_json();
        }
        if let Some((line, column)) = self.json_error {
            ui.colored_label(
                ui.visuals().error_fg_color,
                json_syntax_message(line, column),
            );
        }
        response
    }

    fn format_json(&mut self) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&self.text)
            && let Ok(text) = serde_json::to_string_pretty(&value)
        {
            self.text = text;
        }
    }
}

pub(crate) fn json_syntax_message(line: usize, column: usize) -> String {
    tf(
        "literal-json-error",
        &[
            ("line", line.max(1).into()),
            ("column", column.max(1).into()),
        ],
    )
}

pub(crate) fn state_text(state: InputState) -> &'static str {
    t(match state {
        InputState::Wired { shadowed: true } => "input-shadowed",
        InputState::Wired { shadowed: false } => "from-a-wire",
        InputState::Literal => "input-literal",
        InputState::Default => "input-default",
        InputState::ImplicitNone => "input-implicit-none",
        InputState::Required => "input-required",
    })
}

impl Editor {
    pub(crate) fn begin_value_input(&mut self, node: NodeId, port_id: &str) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let Some(view) = document
            .node_view(node)
            .filter(|view| view.missing.is_none())
        else {
            return;
        };
        let Some(port) = view
            .ports
            .iter()
            .find(|port| port.id == port_id && port.input && !port.connected)
        else {
            return;
        };
        let detail = self.canvas.port_details(node, port_id, true);
        let Some(ty) = port
            .ty
            .as_deref()
            .and_then(|ty| ty.parse::<TypeExpr>().ok())
        else {
            return;
        };
        let editor = port.literal_editor(detail);
        if !matches!(
            editor,
            LiteralEditor::Option { .. } | LiteralEditor::Structured
        ) {
            return;
        }
        let graph = document.draft.document();
        let current = graph
            .node(node)
            .and_then(|node| {
                node.inputs
                    .iter()
                    .find(|(key, _)| key.to_string() == port_id)
            })
            .map(|(_, literal)| literal)
            .or_else(|| detail.and_then(|detail| detail.default_literal.as_ref()));
        self.value_form = Some(ValueForm::new(
            Target::Input {
                graph: graph.graph_id(),
                revision: graph.revision(),
                node,
                port: port_id.to_owned(),
            },
            properties::input_caption(view, port),
            ty,
            editor,
            current,
        ));
    }

    pub(crate) fn begin_manual_payload(&mut self, payload: ManualPayload) {
        if payload.editor == LiteralEditor::Unavailable {
            self.say(t("literal-unavailable").to_owned());
            return;
        }
        let ty = payload.ty.clone();
        let editor = payload.editor;
        self.value_form = Some(ValueForm::new(
            Target::Manual(payload),
            t("fire-with-value").to_owned(),
            ty,
            editor,
            None,
        ));
    }

    pub(crate) fn value_dialog(&mut self, ctx: &egui::Context, host: &mut Host) {
        let Some(mut form) = self.value_form.take() else {
            return;
        };
        let modal = orange_ui_controls::modal(ctx, egui::Id::unique("orange-typed-value")).show(
            ctx,
            |ui| {
                ui.set_width(ui.available_width().min(420.0));
                let apply = form.fields(ui);
                (apply, ui.button(t("cancel")).clicked())
            },
        );
        if modal.inner.1 || modal.should_close() {
            return;
        }
        if modal.inner.0 {
            let result = form.literal().and_then(|literal| match &form.target {
                Target::Manual(payload) => host
                    .fire_manual_literal(payload, &literal)
                    .map_err(|error| error.to_string()),
                Target::Input {
                    graph,
                    revision,
                    node,
                    port,
                } => {
                    let document = self
                        .document
                        .as_mut()
                        .filter(|doc| {
                            let saved = doc.draft.document();
                            saved.graph_id() == *graph && saved.revision() == *revision
                        })
                        .ok_or_else(|| t("literal-edit-stale").to_owned())?;
                    let changed = host
                        .set_input_literals(
                            &mut document.draft,
                            [((*node, port.as_str()), literal)],
                        )
                        .map_err(|error| error.to_string())?;
                    let _ = self.edited(host, Ok(changed));
                    Ok(())
                }
            });
            match result {
                Ok(()) => return,
                Err(error) => form.error = Some(error),
            }
        }
        self.value_form = Some(form);
    }

    pub(crate) fn clear_manual_value_form(&mut self) {
        if self
            .value_form
            .as_ref()
            .is_some_and(|form| matches!(form.target, Target::Manual(_)))
        {
            self.value_form = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_manual_button_opens_a_transient_form_and_stop_clears_it() {
        let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        let node = host
            .add_node(&mut draft, "core/on-manual", [0.0, 0.0])
            .unwrap();
        host.set_config(
            &mut draft,
            node,
            &serde_json::json!({"payload": "core/i64@1"}),
        )
        .unwrap();
        let saved = draft.text().unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut host, saved.clone());
        editor.run(&mut host);
        editor.trigger(&host, node);
        let form = editor.value_form.as_mut().unwrap();
        assert!(form.literal().is_err());
        form.text = "14".to_owned();
        assert_eq!(
            form.literal().unwrap().payload,
            LiteralPayload::Value(serde_json::json!("14"))
        );
        assert_eq!(
            editor.document.as_ref().unwrap().draft.text().unwrap(),
            saved
        );
        assert_eq!(editor.can_undo(), (false, false));
        editor.stop(&mut host);
        assert!(editor.value_form.is_none());
    }

    #[test]
    fn a_nested_option_form_writes_exactly_the_selected_outer_case() {
        let ty = "core/option@1<core/option@1<core/string@1>>"
            .parse()
            .unwrap();
        let mut form = ValueForm::new(
            Target::Input {
                graph: "e248bb2c-090a-4625-9b61-8c5cfedf5323".parse().unwrap(),
                revision: 0,
                node: "e248bb2c-090a-4625-9b61-8c5cfedf5324".parse().unwrap(),
                port: "value".to_owned(),
            },
            "Option".to_owned(),
            ty,
            LiteralEditor::Option { some: true },
            None,
        );
        assert_eq!(
            form.literal().unwrap().payload,
            LiteralPayload::Value(serde_json::json!({"none":true}))
        );
        form.some = true;
        form.text = r#"{"none":true}"#.to_owned();
        form.format_json();
        assert!(form.text.contains('\n'));
        assert_eq!(
            form.literal().unwrap().payload,
            LiteralPayload::Value(serde_json::json!({"some":{"none":true}}))
        );
        form.text = "{\n  \"none\": ]".to_owned();
        let invalid = form.text.clone();
        form.format_json();
        assert_eq!(form.text, invalid);
        assert_eq!(form.literal().unwrap_err(), json_syntax_message(2, 11));
    }
}
