//! Groups on the canvas: a named, coloured frame around nodes, moved as one by its title
//! and collapsed to a small card that hides them; and arranging selected nodes along
//! their wires. The groups themselves belong to the document ([`Group`]);
//! the canvas draws them and says what a person asked of them.
use super::{CanvasResponse, NODE_WIDTH, Palette, TITLE_ZOOM, auto_layout, node_size, snapped};
use crate::canvas::Canvas;
use crate::i18n::{t, tf};
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use orange_host::{GROUP_COLORS, GraphView, Group, NodeId, NodeView, WireView};
use std::collections::{HashMap, HashSet};

/// Room between a group's frame and its nodes, in graph units.
pub const GROUP_PAD: f32 = 16.0;
/// Height of a group's title bar.
pub const GROUP_TITLE: f32 = 28.0;
/// Height of a collapsed group's card.
const COLLAPSED_HEIGHT: f32 = 58.0;

/// A change to a group that a person asked for on the canvas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupEdit {
    /// The group with a new name, colour or collapsed state.
    Change(Group),
    /// Take the frame away and leave its nodes.
    Ungroup(String),
}

fn color(group: &Group) -> Color32 {
    let [r, g, b] = group.color;
    Color32::from_rgb(r, g, b)
}

impl Canvas {
    /// Takes the document's groups. Nodes of a collapsed group are hidden: not drawn,
    /// not picked by a box and their wires end on the group's card.
    pub fn set_groups(&mut self, groups: Vec<Group>) {
        self.hidden = groups
            .iter()
            .filter(|group| group.collapsed)
            .flat_map(|group| group.nodes.iter().copied())
            .collect();
        let hidden = &self.hidden;
        self.selection.retain(|node| !hidden.contains(node));
        if self.selected.is_some_and(|node| hidden.contains(&node)) {
            self.selected = None;
        }
        self.groups = groups;
        if let Some(id) = &self.selected_group_frame {
            match self.groups.iter().find(|group| &group.id == id) {
                Some(group) if !group.collapsed => {
                    let nodes = group.nodes.clone();
                    self.select_many(&nodes);
                }
                Some(_) => {}
                None => self.selected_group_frame = None,
            }
        }
    }

    /// The groups the canvas draws.
    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    pub(crate) fn selected_group(&self) -> Option<&Group> {
        self.groups.iter().find(|group| {
            self.selected_group_frame.as_ref() == Some(&group.id)
                || !group.nodes.is_empty()
                    && group.nodes.len() == self.selection.len()
                    && group.nodes.iter().all(|node| self.selection.contains(node))
        })
    }

    /// Whether a node is hidden in a collapsed group.
    pub fn is_hidden(&self, node: NodeId) -> bool {
        self.hidden.contains(&node)
    }

    /// The frame of a group in graph units: around its nodes with its title above, or,
    /// collapsed, a small card where its nodes begin. `None` when none of its nodes is
    /// placed.
    pub fn group_frame(&self, graph: &GraphView, group: &Group) -> Option<Rect> {
        let nodes: HashMap<NodeId, &NodeView> =
            graph.nodes.iter().map(|node| (node.id, &**node)).collect();
        let around = group
            .nodes
            .iter()
            .filter_map(|id| {
                Some(Rect::from_min_size(
                    *self.places.get(id)?,
                    node_size(nodes.get(id)?),
                ))
            })
            .reduce(|all, rect| all.union(rect))?;
        // Offsets from the first member to the frame are whole grid intervals.
        // The frame and members that were aligned can then snap together.
        let step = f32::from(self.grid_step);
        let pad = |space: f32| (space / step).ceil() * step;
        let top = around.min - Vec2::new(pad(GROUP_PAD), pad(GROUP_PAD + GROUP_TITLE));
        Some(if group.collapsed {
            Rect::from_min_size(top, Vec2::new(NODE_WIDTH, COLLAPSED_HEIGHT))
        } else {
            Rect::from_min_max(top, around.max + Vec2::splat(GROUP_PAD))
        })
    }

    /// Where a wire to or from a hidden node ends: the side of its group's card.
    pub(super) fn hidden_anchor(
        &self,
        frames: &HashMap<NodeId, Rect>,
        node: NodeId,
        input: bool,
    ) -> Option<Pos2> {
        let frame = frames.get(&node)?;
        Some(if input {
            frame.left_center()
        } else {
            frame.right_center()
        })
    }

    /// The frames of collapsed groups by each node they hide, in graph units.
    pub(super) fn collapsed_frames(&self, graph: &GraphView) -> HashMap<NodeId, Rect> {
        let mut frames = HashMap::new();
        for group in self.groups.iter().filter(|group| group.collapsed) {
            if let Some(frame) = self.group_frame(graph, group) {
                for node in &group.nodes {
                    let _ = frames.insert(*node, frame);
                }
            }
        }
        frames
    }

    /// Whether a wire runs inside one collapsed group, and so is not drawn.
    pub(super) fn wire_hidden(&self, wire: &WireView) -> bool {
        self.groups.iter().any(|group| {
            group.collapsed
                && group.nodes.contains(&wire.from.0)
                && group.nodes.contains(&wire.to.0)
        })
    }

    /// Handles the title bars of groups: dragging one moves its nodes, clicking selects
    /// the group, a double click collapses or opens it, and the second button offers
    /// renaming, colours and ungrouping.
    pub(super) fn group_input(
        &mut self,
        ui: &egui::Ui,
        graph: &GraphView,
        screen: impl Fn(Rect) -> Rect,
        base: egui::Id,
        dismiss_menu: bool,
        done: &mut CanvasResponse,
    ) {
        let zoom = self.zoom;
        for group in self.groups.clone() {
            let Some(frame) = self.group_frame(graph, &group) else {
                continue;
            };
            let title = if group.collapsed {
                frame
            } else {
                Rect::from_min_size(frame.min, Vec2::new(frame.width(), GROUP_TITLE))
            };
            let touched = ui.interact(
                screen(title),
                base.with(("group", group.id.as_str())),
                Sense::click_and_drag(),
            );
            if touched.dragged() {
                let by = touched.drag_delta() / zoom;
                for node in &group.nodes {
                    if let Some(place) = self.places.get_mut(node) {
                        *place += by;
                    }
                }
                self.moving = true;
            }
            if touched.drag_stopped() && self.moving {
                if self.snap && !ui.input(|input| input.modifiers.alt) {
                    let by = snapped(frame.min, self.grid_step) - frame.min;
                    for node in &group.nodes {
                        if let Some(place) = self.places.get_mut(node) {
                            *place += by;
                        }
                    }
                }
                self.moving = false;
                done.moved = true;
            }
            if touched.double_clicked() {
                let mut changed = group.clone();
                changed.collapsed = !changed.collapsed;
                done.group = Some(GroupEdit::Change(changed));
            } else if touched.clicked() {
                if group.collapsed {
                    self.select(None);
                    self.selected_group_frame = Some(group.id.clone());
                } else {
                    self.select_many(&group.nodes);
                }
            }
            if dismiss_menu {
                let popup_id = egui::Popup::default_response_id(&touched);
                if egui::Popup::is_id_open(ui.ctx(), popup_id) {
                    egui::Popup::close_id(ui.ctx(), popup_id);
                    ui.memory_mut(|memory| memory.surrender_focus(touched.id.with("group-name")));
                    self.renaming = None;
                }
                continue;
            }
            let opening = touched.secondary_clicked();
            if opening {
                self.renaming = Some((group.id.clone(), group.name.clone(), true));
            }
            let touched = touched.on_hover_text_at_pointer(&group.name);
            let popup = egui::Popup::context_menu(&touched)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
            let _ = popup.show(|ui| {
                ui.set_width(244.0);
                let renaming = self
                    .renaming
                    .get_or_insert_with(|| (group.id.clone(), group.name.clone(), true));
                if renaming.0 != group.id {
                    *renaming = (group.id.clone(), group.name.clone(), true);
                }
                let (_, name, focus) = renaming;
                let height = ui.spacing().interact_size.y;
                let result = crate::organise::rename_field(
                    ui,
                    touched.id.with("group-name"),
                    name,
                    focus,
                    egui::TextStyle::Body,
                    height,
                    Some(t("name")),
                );
                if result == Some(true) {
                    let name = name.trim();
                    if !name.is_empty() && name != group.name {
                        let mut changed = group.clone();
                        changed.name = name.to_owned();
                        done.group = Some(GroupEdit::Change(changed));
                        ui.close();
                        return;
                    }
                }
                if result.is_some() {
                    name.clone_from(&group.name);
                }
                ui.add_space(6.0);
                let custom_id = touched.id.with("custom-group-color");
                let was_open =
                    !opening && ui.data(|data| data.get_temp::<bool>(custom_id).unwrap_or(false));
                let custom_clicked = ui
                    .horizontal(|ui| {
                        for (choice, label) in GROUP_COLORS.into_iter().zip([
                            "group-color-blue",
                            "group-color-green",
                            "group-color-orange",
                            "group-color-purple",
                            "group-color-red",
                            "group-color-teal",
                        ]) {
                            let [r, g, b] = choice;
                            let (rect, response) =
                                ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
                            let selected = choice == group.color;
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::Role::RadioButton,
                                    true,
                                    selected,
                                    t(label),
                                )
                            });
                            let response = response.on_hover_text(t(label));
                            let fill = Color32::from_rgb(r, g, b);
                            ui.painter().rect_filled(rect.shrink(3.0), 4.0, fill);
                            if selected || response.hovered() || response.has_focus() {
                                ui.painter().rect_stroke(
                                    rect,
                                    5.0,
                                    Stroke::new(1.0, ui.visuals().text_color()),
                                    StrokeKind::Inside,
                                );
                            }
                            if response.clicked() {
                                let mut changed = group.clone();
                                changed.color = choice;
                                done.group = Some(GroupEdit::Change(changed));
                                ui.close();
                            }
                        }
                        let custom = ui
                            .add(
                                egui::Button::new(crate::icons::centered(crate::icons::ADD))
                                    .min_size(Vec2::splat(26.0))
                                    .selected(was_open || !GROUP_COLORS.contains(&group.color)),
                            )
                            .on_hover_text(t("group-custom-color"));
                        custom.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::Role::Button,
                                true,
                                was_open,
                                t("group-custom-color"),
                            )
                        });
                        custom.clicked()
                    })
                    .inner;
                let custom_open = was_open ^ custom_clicked;
                if opening || custom_clicked {
                    ui.data_mut(|data| {
                        data.insert_temp(custom_id, custom_open);
                        orange_ui_controls::color_picker::reset(data, custom_id);
                    });
                }
                if custom_open {
                    let [r, g, b] = group.color;
                    if let Some([r, g, b, _]) = orange_ui_controls::color_picker::inline(
                        ui,
                        custom_id,
                        [r, g, b, 255],
                        false,
                        crate::properties::color_picker_labels(),
                    ) {
                        let mut changed = group.clone();
                        changed.color = [r, g, b];
                        done.group = Some(GroupEdit::Change(changed));
                    }
                    return;
                }
                let fold = if group.collapsed {
                    t("expand")
                } else {
                    t("collapse")
                };
                ui.add_space(6.0);
                let menu_action = |ui: &mut egui::Ui, icon, label, enabled| {
                    use crate::controls::{self, Action, Surface};
                    let button = egui::Button::new((crate::icons::centered(icon), label))
                        .min_size(Vec2::new(ui.available_width(), 28.0));
                    controls::button(ui, Surface::Panel, Action::Menu, enabled, button).clicked()
                };
                let fold_icon = if group.collapsed {
                    crate::icons::EXPAND
                } else {
                    crate::icons::COLLAPSE
                };
                if menu_action(ui, fold_icon, fold, true) {
                    let mut changed = group.clone();
                    changed.collapsed = !changed.collapsed;
                    done.group = Some(GroupEdit::Change(changed));
                    ui.close();
                }
                if menu_action(
                    ui,
                    crate::icons::SELECT_ALL,
                    t("group-select-nodes"),
                    !group.collapsed,
                ) {
                    self.select_many(&group.nodes);
                    ui.close();
                }
                if menu_action(ui, crate::icons::UNGROUP, t("ungroup"), true) {
                    done.group = Some(GroupEdit::Ungroup(group.id.clone()));
                    ui.close();
                }
            });
        }
    }

    /// Draws the frames of open groups behind the cards, and collapsed groups as cards.
    pub(super) fn draw_groups(
        &self,
        painter: &egui::Painter,
        graph: &GraphView,
        screen: impl Fn(Rect) -> Rect,
        colors: &Palette,
    ) {
        let zoom = self.zoom;
        if !self.groups.is_empty() {
            // A canvas shown without the editor still has its carets.
            crate::icons::install(painter.ctx());
        }
        let selected_group = self.selected_group().map(|group| group.id.as_str());
        for group in &self.groups {
            let Some(frame) = self.group_frame(graph, group) else {
                continue;
            };
            let tint = color(group);
            let shown = screen(frame);
            let radius = 10.0 * zoom;
            let title_height = GROUP_TITLE * zoom;
            if group.collapsed {
                painter.rect_filled(shown, radius, colors.card);
            } else {
                painter.rect_filled(shown, radius, tint.gamma_multiply(0.10));
            }
            let title = Rect::from_min_size(shown.min, Vec2::new(shown.width(), title_height));
            painter.rect_filled(
                title,
                egui::CornerRadius {
                    nw: radius as u8,
                    ne: radius as u8,
                    sw: 0,
                    se: 0,
                },
                tint.gamma_multiply(0.55),
            );
            let selected = selected_group == Some(group.id.as_str());
            painter.rect_stroke(
                shown,
                radius,
                Stroke::new(
                    if selected { 2.0 } else { 1.5 } * zoom.max(0.6),
                    if selected { colors.selected } else { tint },
                ),
                StrokeKind::Inside,
            );
            if zoom < TITLE_ZOOM {
                continue;
            }
            let painter = painter.with_clip_rect(shown);
            let font = FontId::proportional(13.0 * zoom);
            // The caret is from the bundled icon font, the name from the text font.
            let mark = if group.collapsed {
                crate::icons::EXPAND
            } else {
                crate::icons::COLLAPSE
            };
            let caret = painter.text(
                title.left_center() + Vec2::new(10.0 * zoom, 0.0),
                Align2::LEFT_CENTER,
                mark,
                FontId::new(font.size, crate::icons::family()),
                colors.text,
            );
            let name_at = Pos2::new(caret.right() + 4.0 * zoom, title.center().y);
            let mut name =
                egui::text::LayoutJob::simple_singleline(group.name.clone(), font, colors.text);
            name.wrap.max_width = (title.right() - name_at.x - 10.0 * zoom).max(0.0);
            name.wrap.max_rows = 1;
            let name = painter.layout_job(name);
            painter.galley(
                name_at - Vec2::new(0.0, name.size().y / 2.0),
                name,
                colors.text,
            );
            if group.collapsed {
                let count = group.nodes.len();
                let _ = painter.text(
                    Pos2::new(shown.left() + 10.0 * zoom, title.bottom() + 15.0 * zoom),
                    Align2::LEFT_CENTER,
                    tf("node-count", &[("count", count.into())]),
                    FontId::proportional(11.0 * zoom.max(0.5)),
                    colors.weak,
                );
            }
        }
    }

    /// Lays `nodes` out in columns along the wires between them, from where the top
    /// left one is now. Returns whether any of them moved; the places are then worth
    /// saving, as one edit.
    pub fn arrange(&mut self, graph: &GraphView, nodes: &[NodeId]) -> bool {
        let chosen: HashSet<NodeId> = nodes
            .iter()
            .copied()
            .filter(|node| self.places.contains_key(node))
            .collect();
        if chosen.len() < 2 {
            return false;
        }
        let start = chosen
            .iter()
            .filter_map(|node| self.places.get(node))
            .fold(Pos2::new(f32::INFINITY, f32::INFINITY), |at, place| {
                Pos2::new(at.x.min(place.x), at.y.min(place.y))
            });
        // The chosen nodes alone, unplaced, so the layout puts each one.
        let part = GraphView {
            nodes: graph
                .nodes
                .iter()
                .filter(|node| chosen.contains(&node.id))
                .map(|node| {
                    std::sync::Arc::new(NodeView {
                        position: None,
                        ..NodeView::clone(node)
                    })
                })
                .collect(),
            wires: graph
                .wires
                .iter()
                .filter(|wire| chosen.contains(&wire.from.0) && chosen.contains(&wire.to.0))
                .cloned()
                .collect(),
        };
        let mut moved = false;
        for (node, at) in auto_layout(&part) {
            let at = start + at.to_vec2();
            if let Some(place) = self.places.get_mut(&node)
                && *place != at
            {
                *place = at;
                moved = true;
            }
        }
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orange_host::{Host, HostOptions};

    #[test]
    fn the_group_menu_commits_renaming_and_a_custom_color() {
        let (host, _) = Host::new(HostOptions::default()).unwrap();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        let node = host
            .add_node(&mut draft, "core/whole-number", [72.0, 108.0])
            .unwrap();
        let graph = host.draft_view(&draft).unwrap();
        let mut group = Group {
            id: "group".into(),
            name: "Original".into(),
            color: GROUP_COLORS[0],
            nodes: vec![node],
            collapsed: true,
        };
        let mut canvas = Canvas::new();
        canvas.load(&graph);
        canvas.set_groups(vec![group.clone()]);
        let frame_group = group.clone();
        let title_at = std::cell::Cell::new(Pos2::ZERO);
        let context = egui::Context::default();
        crate::icons::install(&context);
        let mut frame = |events, replacement: Option<&Group>| {
            if let Some(replacement) = replacement {
                canvas.set_groups(vec![replacement.clone()]);
            }
            let mut result = CanvasResponse::default();
            let output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 680.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let canvas_start = ui.cursor().min;
                    result = canvas.show(ui, &graph, &[]);
                    let title = canvas.group_frame(&graph, &frame_group).unwrap().center();
                    title_at.set(canvas_start + (title - canvas.origin) * canvas.zoom);
                },
            );
            (output, result.group)
        };
        let pointer = |pos, button, pressed| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let button_center = |output: &egui::FullOutput, label: &str| {
            orange_testkit::ui::text_center(&output.shapes, label).expect(label)
        };
        frame(vec![], None).0.drop_without_applying_deltas();
        let at = title_at.get();
        orange_testkit::ui::click(at, egui::PointerButton::Secondary, |events| {
            frame(events, None).0.drop_without_applying_deltas();
        });
        frame(vec![], None).0.drop_without_applying_deltas();
        frame(
            vec![
                key(egui::Key::A, egui::Modifiers::COMMAND),
                egui::Event::Text("QA playback".into()),
            ],
            None,
        )
        .0
        .drop_without_applying_deltas();
        let (output, changed) = frame(vec![key(egui::Key::Enter, egui::Modifiers::NONE)], None);
        output.drop_without_applying_deltas();
        group.name = "QA playback".into();
        assert_eq!(changed, Some(GroupEdit::Change(group.clone())));

        frame(vec![], Some(&group)).0.drop_without_applying_deltas();
        orange_testkit::ui::click(at, egui::PointerButton::Secondary, |events| {
            frame(events, None).0.drop_without_applying_deltas();
        });
        let (output, _) = frame(vec![], None);
        let custom = button_center(&output, &crate::icons::ADD.to_string());
        output.drop_without_applying_deltas();
        let mut click = |pos| {
            frame(
                vec![
                    egui::Event::PointerMoved(pos),
                    pointer(pos, egui::PointerButton::Primary, true),
                ],
                None,
            )
            .0
            .drop_without_applying_deltas();
            frame(
                vec![pointer(pos, egui::PointerButton::Primary, false)],
                None,
            )
        };
        click(custom).0.drop_without_applying_deltas();
        let (output, _) = frame(vec![], None);
        assert!(orange_testkit::ui::text_center(&output.shapes, "#569CD6").is_some());
        output.drop_without_applying_deltas();

        let (output, changed) = frame(
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: Vec2::new(0.0, 80.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: egui::Modifiers::NONE,
            }],
            None,
        );
        assert!(orange_testkit::ui::text_center(&output.shapes, "#569CD6").is_none());
        assert_eq!(changed, None);
        output.drop_without_applying_deltas();
        orange_testkit::ui::click(title_at.get(), egui::PointerButton::Secondary, |events| {
            frame(events, None).0.drop_without_applying_deltas();
        });
        let (output, _) = frame(vec![], None);
        let custom = button_center(&output, &crate::icons::ADD.to_string());
        output.drop_without_applying_deltas();
        orange_testkit::ui::click(custom, egui::PointerButton::Primary, |events| {
            frame(events, None).0.drop_without_applying_deltas();
        });
        let (output, _) = frame(vec![], None);
        let field = button_center(&output, "#569CD6");
        output.drop_without_applying_deltas();
        frame(
            vec![
                egui::Event::PointerMoved(field),
                pointer(field, egui::PointerButton::Primary, true),
            ],
            None,
        )
        .0
        .drop_without_applying_deltas();
        frame(
            vec![pointer(field, egui::PointerButton::Primary, false)],
            None,
        )
        .0
        .drop_without_applying_deltas();
        let (output, _) = frame(
            vec![
                key(egui::Key::A, egui::Modifiers::COMMAND),
                egui::Event::Text("#193B75".into()),
            ],
            None,
        );
        let apply = button_center(&output, t("color-apply"));
        output.drop_without_applying_deltas();
        frame(
            vec![
                egui::Event::PointerMoved(apply),
                pointer(apply, egui::PointerButton::Primary, true),
            ],
            None,
        )
        .0
        .drop_without_applying_deltas();
        let (output, changed) = frame(
            vec![pointer(apply, egui::PointerButton::Primary, false)],
            None,
        );
        output.drop_without_applying_deltas();
        group.color = [25, 59, 117];
        assert_eq!(changed, Some(GroupEdit::Change(group)));
    }

    #[test]
    fn a_collapsed_group_has_a_clipped_title_and_hides_text_at_overview_zoom() {
        let (host, _) = Host::new(HostOptions::default()).unwrap();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        let node = host
            .add_node(&mut draft, "core/whole-number", [72.0, 108.0])
            .unwrap();
        let graph = host.draft_view(&draft).unwrap();
        let group = Group {
            id: "group".into(),
            name: "An intentionally long group title that has nowhere to go".into(),
            color: GROUP_COLORS[0],
            nodes: vec![node],
            collapsed: true,
        };
        let mut canvas = Canvas::new();
        canvas.load(&graph);
        canvas.set_groups(vec![group.clone()]);
        let frame = canvas.group_frame(&graph, &group).unwrap();
        let context = egui::Context::default();
        for zoom in [1.0, 0.25] {
            canvas.zoom = zoom;
            let scale = |rect: Rect| {
                Rect::from_min_max(
                    Pos2::ZERO + rect.min.to_vec2() * zoom,
                    Pos2::ZERO + rect.max.to_vec2() * zoom,
                )
            };
            let output = context.run_ui(egui::RawInput::default(), |ui| {
                canvas.draw_groups(ui.painter(), &graph, scale, &Palette::of(ui));
            });
            let title = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().starts_with("An intentionally") => {
                    Some((text, shape.clip_rect))
                }
                _ => None,
            });
            if zoom >= TITLE_ZOOM {
                let (text, clip) = title.unwrap();
                assert!(text.galley.elided);
                assert!(clip.right() <= scale(frame).right());
            } else {
                assert!(title.is_none());
            }
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn dragging_the_group_title_snaps_the_frame_and_moves_members_together() {
        let (host, _) = Host::new(HostOptions::default()).unwrap();
        let mut draft = host.draft(&orange_host::new_document().unwrap()).unwrap();
        let first = host
            .add_node(&mut draft, "core/whole-number", [72.0, 108.0])
            .unwrap();
        let second = host
            .add_node(&mut draft, "core/whole-number", [432.0, 144.0])
            .unwrap();
        let graph = host.draft_view(&draft).unwrap();
        for (collapsed, alt, expected) in [
            (false, false, Vec2::new(36.0, 36.0)),
            (true, false, Vec2::new(36.0, 36.0)),
            (false, true, Vec2::new(25.0, 51.0)),
        ] {
            let mut canvas = Canvas::new();
            canvas.load(&graph);
            canvas.grid_step = 36;
            let group = Group {
                id: "group".into(),
                name: "Examples".into(),
                color: [100, 100, 100],
                nodes: vec![first, second],
                collapsed,
            };
            canvas.set_groups(vec![group.clone()]);
            let start = canvas.group_frame(&graph, &group).unwrap().min;
            let from = start + Vec2::splat(10.0);
            let to = from + Vec2::new(25.0, 51.0);
            let modifiers = egui::Modifiers {
                alt,
                ..Default::default()
            };
            let click = |pos, pressed| egui::Event::PointerButton {
                pos,
                pressed,
                modifiers,
                button: egui::PointerButton::Primary,
            };
            let ctx = egui::Context::default();
            let mut frame = |mut events: Vec<egui::Event>| {
                events.insert(0, egui::Event::ModifiersChanged(modifiers));
                let mut done = CanvasResponse::default();
                ctx.run_ui(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        canvas.group_input(
                            ui,
                            &graph,
                            |rect| rect,
                            egui::Id::unique("group-test"),
                            false,
                            &mut done,
                        );
                    },
                )
                .drop_without_applying_deltas();
                (done.moved, canvas.selected_group().is_some())
            };
            frame(vec![]);
            frame(vec![egui::Event::PointerMoved(from), click(from, true)]);
            frame(vec![egui::Event::PointerMoved(to)]);
            assert!(frame(vec![click(to, false)]).0);
            if collapsed {
                let at = from + expected;
                frame(vec![egui::Event::PointerMoved(at), click(at, true)]);
                assert!(frame(vec![click(at, false)]).1);
                assert!(canvas.selection().is_empty());
            }
            assert_eq!(
                canvas.group_frame(&graph, &group).unwrap().min,
                start + expected
            );
            assert_eq!(canvas.places[&first], Pos2::new(72.0, 108.0) + expected);
            assert_eq!(canvas.places[&second], Pos2::new(432.0, 144.0) + expected);
        }
    }
}
