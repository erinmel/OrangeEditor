//! Groups on the canvas: a named, coloured frame around nodes, moved as one by its title
//! and collapsed to a small card that hides them; and arranging selected nodes along
//! their wires. The groups themselves belong to the document ([`orange_host::Group`]);
//! the canvas draws them and says what a person asked of them.
use super::{CanvasResponse, NODE_WIDTH, Palette, auto_layout, node_size};
use crate::canvas::Canvas;
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
    }

    /// The groups the canvas draws.
    pub fn groups(&self) -> &[Group] {
        &self.groups
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
            graph.nodes.iter().map(|node| (node.id, node)).collect();
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
        let top = around.min - Vec2::new(GROUP_PAD, GROUP_PAD + GROUP_TITLE);
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
    /// them, a double click collapses or opens the group, and the second button offers
    /// renaming, colours and ungrouping.
    pub(super) fn group_input(
        &mut self,
        ui: &egui::Ui,
        graph: &GraphView,
        screen: impl Fn(Rect) -> Rect,
        base: egui::Id,
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
                self.moving = false;
                done.moved = true;
            }
            if touched.double_clicked() {
                let mut changed = group.clone();
                changed.collapsed = !changed.collapsed;
                done.group = Some(GroupEdit::Change(changed));
            } else if touched.clicked() && !group.collapsed {
                self.select_many(&group.nodes);
            }
            let _ = touched.context_menu(|ui| {
                let renaming = self
                    .renaming
                    .get_or_insert_with(|| (group.id.clone(), group.name.clone()));
                if renaming.0 != group.id {
                    *renaming = (group.id.clone(), group.name.clone());
                }
                ui.label("Name");
                let typed = ui.text_edit_singleline(&mut renaming.1);
                let name = renaming.1.trim().to_owned();
                if typed.lost_focus() && !name.is_empty() && name != group.name {
                    let mut changed = group.clone();
                    changed.name = name;
                    done.group = Some(GroupEdit::Change(changed));
                }
                ui.horizontal(|ui| {
                    for choice in GROUP_COLORS {
                        let [r, g, b] = choice;
                        let swatch = egui::Button::new("  ")
                            .fill(Color32::from_rgb(r, g, b))
                            .selected(choice == group.color);
                        if ui.add(swatch).clicked() {
                            let mut changed = group.clone();
                            changed.color = choice;
                            done.group = Some(GroupEdit::Change(changed));
                        }
                    }
                });
                let fold = if group.collapsed {
                    "Expand"
                } else {
                    "Collapse"
                };
                if ui.button(fold).clicked() {
                    let mut changed = group.clone();
                    changed.collapsed = !changed.collapsed;
                    done.group = Some(GroupEdit::Change(changed));
                    ui.close();
                }
                if ui.button("Select its nodes").clicked() && !group.collapsed {
                    self.select_many(&group.nodes);
                    ui.close();
                }
                if ui.button("Ungroup").clicked() {
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
            painter.rect_stroke(
                shown,
                radius,
                Stroke::new(1.5 * zoom.max(0.6), tint),
                StrokeKind::Inside,
            );
            let font = FontId::proportional(13.0 * zoom.max(0.5));
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
            let _ = painter.text(
                Pos2::new(caret.right() + 4.0 * zoom, title.center().y),
                Align2::LEFT_CENTER,
                group.name.as_str(),
                font.clone(),
                colors.text,
            );
            if group.collapsed {
                let count = group.nodes.len();
                let _ = painter.text(
                    Pos2::new(shown.left() + 10.0 * zoom, title.bottom() + 15.0 * zoom),
                    Align2::LEFT_CENTER,
                    format!("{count} node{}", if count == 1 { "" } else { "s" }),
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
                .map(|node| NodeView {
                    position: None,
                    ..node.clone()
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
