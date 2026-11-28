//! Portless, editor-only canvas comments.
use super::*;
use orange_host::CanvasComment;

pub(super) const DEFINITION: &str = "editor/comment";

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
    Save(CanvasComment),
    Cancel(String),
    Delete(String),
}

pub(super) fn card(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    comment: &CanvasComment,
    rect: egui::Rect,
    zoom: f32,
    editing_comment: Option<&mut CanvasComment>,
    focus: bool,
) -> Option<Action> {
    let id = ui.scope_id().with(("canvas-comment", &comment.id));
    let editing_comment = editing_comment.filter(|editing| editing.id == comment.id);
    let current = editing_comment
        .as_deref()
        .cloned()
        .unwrap_or_else(|| comment.clone());
    let response = ui.interact(
        rect.intersect(painter.clip_rect()),
        id,
        egui::Sense::click_and_drag(),
    );
    let response = response.accessible_name(t("canvas-comment"));
    let drag_id = id.with("drag");
    let move_delta = if let Some(delta) = response.total_drag_delta() {
        ui.ctx().data_mut(|data| data.insert_temp(drag_id, delta));
        delta
    } else if response.drag_stopped() {
        ui.ctx()
            .data_mut(|data| data.remove_temp::<egui::Vec2>(drag_id))
            .unwrap_or_default()
    } else {
        egui::Vec2::ZERO
    };
    let resize_id = id.with("resize");
    let handle_size = (12.0 * zoom).clamp(8.0, 16.0);
    let resize_rect = egui::Rect::from_min_size(
        rect.right_bottom() - egui::vec2(handle_size, handle_size),
        egui::vec2(handle_size, handle_size),
    );
    let resize = ui.interact(resize_rect, resize_id, egui::Sense::drag());
    let resize_delta = if let Some(delta) = resize.total_drag_delta() {
        ui.ctx()
            .data_mut(|data| data.insert_temp(resize_id.with("delta"), delta));
        delta
    } else if resize.drag_stopped() {
        ui.ctx()
            .data_mut(|data| data.remove_temp::<egui::Vec2>(resize_id.with("delta")))
            .unwrap_or_default()
    } else {
        egui::Vec2::ZERO
    };
    let resizing = resize.dragged() || resize.drag_stopped();
    let rect = egui::Rect::from_min_size(
        rect.min
            + if resizing {
                egui::Vec2::ZERO
            } else {
                move_delta
            },
        egui::vec2(
            (current.size[0] + resize_delta.x / zoom)
                .clamp(CanvasComment::MIN_SIZE[0], CanvasComment::MAX_SIZE[0]),
            (current.size[1] + resize_delta.y / zoom)
                .clamp(CanvasComment::MIN_SIZE[1], CanvasComment::MAX_SIZE[1]),
        ) * zoom,
    );
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
    let body_rect = egui::Rect::from_min_max(
        inset.min + egui::vec2(0.0, 23.0 * zoom),
        inset.max - egui::vec2(0.0, handle_size),
    );
    let mut result = None;
    if let Some(editing) = editing_comment {
        let field = ui.put(
            body_rect,
            egui::TextEdit::multiline(&mut editing.text)
                .id(id.with("text"))
                .font(egui::FontId::proportional(13.0 * zoom))
                .desired_width(body_rect.width())
                .desired_rows(3)
                .char_limit(4096)
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .hint_text(t("canvas-comment-help")),
        );
        if focus {
            field.request_focus();
        }
        let (save, cancel) = ui.input(|input| {
            (
                field.has_focus() && input.key_pressed(egui::Key::Enter) && input.modifiers.command,
                field.has_focus() && input.key_pressed(egui::Key::Escape),
            )
        });
        if cancel {
            result = Some(Action::Cancel(editing.id.clone()));
        } else if save || field.lost_focus() {
            result = Some(Action::Save(editing.clone()));
        }
    } else {
        let text = if current.text.is_empty() {
            t("canvas-comment-edit-hint")
        } else {
            &current.text
        };
        let color = if current.text.is_empty() {
            ui.visuals().weak_text_color()
        } else {
            ink
        };
        let body = painter.layout(
            text.to_owned(),
            egui::FontId::proportional(13.0 * zoom),
            color,
            body_rect.width(),
        );
        painter.galley(body_rect.min, body, color);
        if ui
            .interact(body_rect, id.with("edit"), egui::Sense::click())
            .clicked()
        {
            result = Some(Action::Edit(current.clone()));
        }
    }

    let corner = rect.right_bottom();
    let grip = (5.0 * zoom).clamp(3.0, 6.0);
    painter.line_segment(
        [
            corner - egui::vec2(grip * 3.0, grip),
            corner - egui::vec2(grip, grip * 3.0),
        ],
        egui::Stroke::new(
            1.0,
            ui.visuals()
                .weak_text_color()
                .gamma_multiply(if resize.hovered() { 1.0 } else { 0.55 }),
        ),
    );

    if resize.drag_stopped() && resize_delta != egui::Vec2::ZERO {
        let mut resized = current.clone();
        resized.size = [
            (current.size[0] + resize_delta.x / zoom)
                .clamp(CanvasComment::MIN_SIZE[0], CanvasComment::MAX_SIZE[0]),
            (current.size[1] + resize_delta.y / zoom)
                .clamp(CanvasComment::MIN_SIZE[1], CanvasComment::MAX_SIZE[1]),
        ];
        result = Some(Action::Move(resized));
    } else if response.drag_stopped() && move_delta != egui::Vec2::ZERO && !resizing {
        let mut moved = current.clone();
        moved.at[0] += move_delta.x / zoom;
        moved.at[1] += move_delta.y / zoom;
        result = Some(Action::Move(moved));
    }
    response.context_menu(|ui| {
        if ui.button(t("canvas-comment-edit")).clicked() {
            result = Some(Action::Edit(current.clone()));
            ui.close();
        }
        if ui.button(t("delete")).clicked() {
            result = Some(Action::Delete(current.id.clone()));
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
                    size: CanvasComment::DEFAULT_SIZE,
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
            Action::Move(comment) | Action::Save(comment) => {
                if self
                    .comment_edit
                    .as_ref()
                    .is_some_and(|editing| editing.id == comment.id)
                {
                    self.comment_edit = None;
                }
                document.draft.update_comment(&comment)
            }
            Action::Cancel(id) => {
                if self
                    .comment_edit
                    .as_ref()
                    .is_some_and(|editing| editing.id == id)
                {
                    self.comment_edit = None;
                }
                return;
            }
            Action::Delete(id) => document.draft.remove_comment(&id),
        };
        match result {
            Ok(changed) => document.unsaved |= changed,
            Err(error) => self.say(error.to_string()),
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
            size: CanvasComment::DEFAULT_SIZE,
            text: String::new(),
        };
        let rect = egui::Rect::from_min_size(
            egui::pos2(50.0, 50.0),
            egui::vec2(
                CanvasComment::DEFAULT_SIZE[0],
                CanvasComment::DEFAULT_SIZE[1],
            ) * 0.5,
        );
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
                    action = card(ui, &painter, &comment, rect, 0.5, None, false);
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
    fn resizing_a_comment_commits_bounded_graph_dimensions() {
        let ctx = egui::Context::default();
        let comment = CanvasComment {
            id: "test".into(),
            at: [10.0, 20.0],
            size: CanvasComment::DEFAULT_SIZE,
            text: String::new(),
        };
        let rect = egui::Rect::from_min_size(
            egui::pos2(50.0, 50.0),
            egui::vec2(
                CanvasComment::DEFAULT_SIZE[0],
                CanvasComment::DEFAULT_SIZE[1],
            ),
        );
        let start = rect.right_bottom() - egui::vec2(5.0, 5.0);
        let end = start + egui::vec2(40.0, 35.0);
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
                    action = card(ui, &painter, &comment, rect, 1.0, None, false);
                },
            )
            .drop_without_applying_deltas();
        }
        let Some(Action::Move(resized)) = action else {
            panic!("expected a layout resize");
        };
        assert_eq!(resized.size, [284.0, 151.0]);
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
        editor.comment_edit = Some(comment.clone());
        editor.apply_comment_action(Action::Save(comment.clone()));
        assert!(editor.comment_edit.is_none());
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
