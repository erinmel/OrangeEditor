//! Node-specific authoring interactions over the existing graph contracts.
mod build_map;

use super::*;
use orange_host::{ForEachModeImpact, Literal, NodeView, WireEnds};

pub(super) use build_map::BuildMapRows;
pub(crate) use build_map::paired_map_available;

pub(crate) fn loop_role(node: &NodeView) -> Option<&'static str> {
    Some(t(match node.definition.as_str() {
        "core/repeat-begin" | "core/map-begin" | "core/fold-begin" | "core/filter-begin" => {
            "loop-role-begin"
        }
        "core/repeat-end" | "core/map-end" | "core/fold-end" | "core/filter-end" => "loop-role-end",
        _ => return None,
    }))
}

pub(crate) fn loop_result(node: &NodeView, port: &orange_host::PortView) -> Option<&'static str> {
    (!port.input && port.id == "result" && loop_role(node).is_some()).then(|| t("loop-role-result"))
}

#[derive(Debug)]
enum ModeChange {
    Connect(WireEnds),
    Setting(NodeId),
}

#[derive(Debug)]
pub(super) struct ForEachConfirmation {
    graph: orange_host::GraphId,
    revision: u64,
    impact: ForEachModeImpact,
    change: ModeChange,
}

impl Editor {
    pub(super) fn has_node_ergonomics_modal(&self) -> bool {
        self.pending_for_each.is_some() || self.build_map_rows.is_some()
    }

    /// Canvas wiring may reconfigure an existing For Each. Let the author review
    /// anything the host will remove before using its ordinary wire transaction.
    pub(super) fn wire_with_mode_preview(&mut self, host: &mut Host, ends: WireEnds) {
        let (from, to) = &ends;
        if let Some(document) = &self.document
            && let Some(impact) =
                host.for_each_wire_impact(&document.draft, (from.0, &from.1), (to.0, &to.1))
            && !impact.is_empty()
        {
            self.pending_for_each = Some(ForEachConfirmation {
                graph: document.draft.document().graph_id(),
                revision: document.draft.document().revision(),
                impact,
                change: ModeChange::Connect(ends),
            });
            return;
        }
        let _ = self.connect(host, (from.0, &from.1), (to.0, &to.1));
    }

    pub(super) fn setting_with_mode_preview(
        &mut self,
        host: &mut Host,
        node: NodeId,
        pointer: &str,
        value: Option<serde_json::Value>,
    ) -> bool {
        if pointer == "/mode"
            && let Some(mode) = value.as_ref().and_then(serde_json::Value::as_str)
            && let Some(document) = &self.document
            && let Some(impact) = host.for_each_mode_impact(&document.draft, node, mode)
            && !impact.is_empty()
        {
            self.pending_for_each = Some(ForEachConfirmation {
                graph: document.draft.document().graph_id(),
                revision: document.draft.document().revision(),
                impact,
                change: ModeChange::Setting(node),
            });
            return false;
        }
        self.set_setting(host, node, pointer, value)
    }

    fn confirm_for_each_change(&mut self, host: &mut Host) {
        let Some(pending) = self.pending_for_each.take() else {
            return;
        };
        if !self.document.as_ref().is_some_and(|doc| {
            doc.draft.document().graph_id() == pending.graph
                && doc.draft.document().revision() == pending.revision
        }) {
            self.say(t("node-edit-preview-stale").to_owned());
            return;
        }
        match pending.change {
            ModeChange::Connect((from, to)) => {
                let _ = self.connect(host, (from.0, &from.1), (to.0, &to.1));
            }
            ModeChange::Setting(node) => {
                let _ = self.set_setting(host, node, "/mode", Some(pending.impact.mode.into()));
            }
        }
    }

    pub(super) fn for_each_mode_dialog(&mut self, ctx: &egui::Context, host: &mut Host) {
        let Some(pending) = &self.pending_for_each else {
            return;
        };
        let modal = orange_ui_controls::modal(ctx, egui::Id::unique("orange-for-each-mode")).show(
            ctx,
            |ui| {
                ui.set_max_width(420.0);
                ui.heading(t("for-each-switch-mode"));
                let impact = &pending.impact;
                let mode = if impact.mode == "map" {
                    t("for-each-map")
                } else {
                    t("for-each-list")
                };
                ui.label(tf("for-each-mode-impact", &[("mode", mode.into())]));
                if impact.items_wire {
                    ui.label(t("for-each-remove-items-wire"));
                }
                if impact.items_literal {
                    ui.label(t("for-each-remove-items-literal"));
                }
                if impact.body_wires != 0 {
                    ui.label(tf(
                        "for-each-remove-body-wires",
                        &[("count", impact.body_wires.into())],
                    ));
                }
                if impact.type_pins {
                    ui.label(t("for-each-remove-type-pins"));
                }
                controls::dialog_actions(
                    ui,
                    controls::Action::Destructive,
                    true,
                    egui::Button::new(t("for-each-switch-mode")),
                )
            },
        );
        if modal.inner.1.clicked() {
            self.confirm_for_each_change(host);
        } else if modal.inner.0.clicked() || modal.should_close() {
            self.pending_for_each = None;
        }
    }

    pub(super) fn omit_record_field(
        &mut self,
        host: &mut Host,
        node: NodeId,
        port: &str,
        ty: orange_host::TypeExpr,
    ) {
        if let Some(draft) = self.editing() {
            let literal = Literal::value(ty, serde_json::json!({"none": true}));
            let result = host.set_input_literals(draft, [((node, port), literal)]);
            let _ = self.edited(host, result);
        }
    }
}
