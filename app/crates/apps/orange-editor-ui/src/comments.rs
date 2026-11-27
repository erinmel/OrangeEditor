//! Portless, editor-only canvas comments.
use super::*;
use orange_host::CanvasComment;

pub(super) const DEFINITION: &str = "editor/comment";
pub(super) const SIZE: egui::Vec2 = egui::vec2(244.0, 116.0);

pub(super) fn matches(search: &str) -> bool {
    orange_host::SearchQuery::new(search).matches([
        t("canvas-comment"), t("canvas-comment-aliases"), t("canvas-comment-help"),
        "comment documentation note notes annotation annotations remark comentario comentarios nota notas anotación メモ 注釈",
    ])
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Action {
    Edit(CanvasComment),
    Move(CanvasComment),
    Delete(String),
}

pub(super) fn card(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    comment: &CanvasComment,
    rect: egui::Rect,
    zoom: f32,
) -> Option<Action> {
    let id = ui.scope_id().with(("canvas-comment", &comment.id));
    let response = ui.interact(
        rect.intersect(painter.clip_rect()),
        id,
        egui::Sense::click_and_drag(),
    );
    let response = response.accessible_name(t("canvas-comment"));
    let drag_id = id.with("drag");
    let delta = if let Some(delta) = response.total_drag_delta() {
        ui.ctx().data_mut(|data| data.insert_temp(drag_id, delta));
        delta
    } else if response.drag_stopped() {
        ui.ctx()
            .data_mut(|data| data.remove_temp::<egui::Vec2>(drag_id))
            .unwrap_or_default()
    } else {
        egui::Vec2::ZERO
    };
    let rect = rect.translate(delta);
    let ink = ui.visuals().text_color();
    let fill = ui
        .visuals()
        .panel_fill
        .lerp_to_gamma(egui::Color32::from_rgb(201, 157, 66), 0.22);
    painter.rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, fill);
    let inset = rect.shrink(9.0 * zoom);
    let painter = painter.with_clip_rect(inset);
    icons::paint_centered(
        &painter,
        inset.min + egui::vec2(7.0, 9.0) * zoom,
        icons::COMMENT,
        15.0 * zoom,
        ink,
    );
    painter.text(
        inset.min + egui::vec2(20.0, 0.0) * zoom,
        egui::Align2::LEFT_TOP,
        t("canvas-comment"),
        egui::FontId::proportional(14.0 * zoom),
        ink,
    );
    let text = if comment.text.is_empty() {
        t("canvas-comment-edit-hint")
    } else {
        &comment.text
    };
    let body = painter.layout(
        text.to_owned(),
        egui::FontId::proportional(13.0 * zoom),
        ink,
        inset.width(),
    );
    painter.galley(inset.min + egui::vec2(0.0, 23.0 * zoom), body, ink);

    let mut result = response
        .double_clicked()
        .then(|| Action::Edit(comment.clone()));
    if response.drag_stopped() && delta != egui::Vec2::ZERO {
        let mut moved = comment.clone();
        moved.at[0] += delta.x / zoom;
        moved.at[1] += delta.y / zoom;
        result = Some(Action::Move(moved));
    }
    response.context_menu(|ui| {
        if ui.button(t("canvas-comment-edit")).clicked() {
            result = Some(Action::Edit(comment.clone()));
            ui.close();
        }
        if ui.button(t("delete")).clicked() {
            result = Some(Action::Delete(comment.id.clone()));
            ui.close();
        }
    });
    result
}

impl Editor {
    pub(super) fn add_canvas_comment(&mut self, at: [f32; 2]) -> bool {
        let Some(document) = &mut self.document else {
            return false;
        };
        match document.draft.add_comment(at, "") {
            Ok(id) => {
                document.unsaved = true;
                self.comment_focus = true;
                self.comment_edit = Some(CanvasComment {
                    id,
                    at,
                    text: String::new(),
                });
                true
            }
            Err(error) => {
                self.say(error.to_string());
                false
            }
        }
    }

    pub(super) fn apply_comment_action(&mut self, action: Action) {
        let Some(document) = &mut self.document else {
            return;
        };
        let result = match action {
            Action::Edit(comment) => {
                self.comment_edit = Some(comment);
                self.comment_focus = true;
                return;
            }
            Action::Move(comment) => document.draft.update_comment(&comment),
            Action::Delete(id) => document.draft.remove_comment(&id),
        };
        match result {
            Ok(changed) => document.unsaved |= changed,
            Err(error) => self.say(error.to_string()),
        }
    }

    pub(super) fn comment_window(&mut self, ctx: &egui::Context) {
        let Some(comment) = &mut self.comment_edit else {
            return;
        };
        let mut save = false;
        let mut cancel = false;
        let mut open = true;
        let focus = std::mem::take(&mut self.comment_focus);
        orange_ui_controls::dialog_window(ctx, t("canvas-comment-edit"))
            .id(egui::Id::unique("orange-comment-editor"))
            .open(&mut open)
            .collapsible(false)
            .show(ctx, |ui| {
                let field = ui.add(
                    egui::TextEdit::multiline(&mut comment.text)
                        .desired_width(360.0)
                        .desired_rows(5)
                        .char_limit(4096)
                        .hint_text(t("canvas-comment-help")),
                );
                if focus {
                    field.request_focus();
                }
                ui.horizontal(|ui| {
                    save = ui.button(t("save")).clicked();
                    cancel = ui.button(t("cancel")).clicked();
                });
            });
        if save && let Some(comment) = self.comment_edit.take() {
            // Resolve the live position so saving text never reverts a concurrent move.
            if let Some(mut current) = self.document.as_ref().and_then(|document| {
                document
                    .draft
                    .comments()
                    .find(|saved| saved.id == comment.id)
            }) {
                current.text = comment.text;
                self.apply_comment_action(Action::Move(current));
            }
        }
        if cancel || !open {
            self.comment_edit = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dragging_a_comment_commits_the_entire_distance_in_graph_units() {
        let ctx = egui::Context::default();
        let comment = CanvasComment {
            id: "test".into(),
            at: [10.0, 20.0],
            text: String::new(),
        };
        let rect = egui::Rect::from_min_size(egui::pos2(50.0, 50.0), SIZE * 0.5);
        let start = rect.min + egui::vec2(10.0, 10.0);
        let end = start + egui::vec2(42.0, 19.0);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: egui::Modifiers::NONE,
        };
        let mut action = None;
        for events in [
            vec![],
            vec![egui::Event::PointerMoved(start)],
            vec![button(start, true)],
            vec![egui::Event::PointerMoved(start + egui::vec2(12.0, 6.0))],
            vec![egui::Event::PointerMoved(end)],
            vec![button(end, false)],
        ] {
            ctx.run_ui(
                egui::RawInput {
                    events,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(500.0, 400.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let painter = ui.painter().clone();
                    action = card(ui, &painter, &comment, rect, 0.5);
                },
            )
            .drop_without_applying_deltas();
        }
        let Some(Action::Move(moved)) = action else {
            panic!("expected a layout move");
        };
        assert_eq!(moved.at, [94.0, 58.0]);
    }

    #[test]
    fn comments_can_be_found_shared_and_undone_without_adding_graph_nodes() {
        let (mut host, _) = Host::new(orange_host::HostOptions::default()).unwrap();
        let mut editor = Editor::new();
        editor.open_untitled(&mut host, orange_host::new_document().unwrap());
        editor.open_quick_add(None, [12.0, 48.0], egui::Pos2::ZERO);
        let choice = editor
            .quick_choices(&host, "notes")
            .into_iter()
            .find(|choice| choice.definition == DEFINITION)
            .unwrap();
        assert!(editor.pick_quick(&mut host, &choice).is_none());
        assert!(!editor.is_quick_add_open());
        let mut comment = editor.comment_edit.take().unwrap();
        comment.text = "This section publishes the result.".into();
        editor.apply_comment_action(Action::Move(comment.clone()));
        assert!(editor.is_unsaved());
        assert!(editor.graph().unwrap().nodes.is_empty());

        editor.share_with_packages(&host);
        let Some(FileRequest::ExportBundle { name, bundle }) = editor.take_request() else {
            panic!("expected a bundle export");
        };
        let mut imported = Editor::new();
        imported.review_bundle(&host, name, &bundle).unwrap();
        let pending = imported.pending_bundle.take().unwrap();
        imported.install_bundle(&mut host, pending);
        let draft = &imported.document.as_ref().unwrap().draft;
        assert_eq!(draft.comments().collect::<Vec<_>>(), [comment.clone()]);
        assert!(draft.document().root().nodes.is_empty());

        editor.apply_comment_action(Action::Delete(comment.id.clone()));
        assert_eq!(
            editor.document.as_ref().unwrap().draft.comments().count(),
            0
        );
        editor.undo(&mut host);
        assert_eq!(
            editor
                .document
                .as_ref()
                .unwrap()
                .draft
                .comments()
                .collect::<Vec<_>>(),
            [comment]
        );
    }
}
