//! The Types tab: enums and records the open graph declares itself, so a person can
//! make their own choices (Mood: calm, happy) and structures without a package
//! (12 §6.5). Values store member IDs, so names can change freely.
use super::*;
use crate::controls::{Dropdown as _, Surface};
use orange_host::{FieldDecl, LocalTypeDecl, TypeShape, VariantDecl};
use std::cell::RefCell;

thread_local! {
    /// Each local type's expression and name, for showing types by name everywhere.
    static NAMES: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
}

/// Remembers the open graph's types, so [`with_local_names`] can show them by name.
pub(crate) fn remember(types: &[LocalTypeDecl]) {
    NAMES.with(|names| {
        *names.borrow_mut() = types
            .iter()
            .map(|decl| (decl.ty().to_string(), decl.name.clone()))
            .collect();
    });
}

/// `ty` with every local type of the open graph replaced by its name:
/// `core/list@1<local/t6c0e…@1>` reads `core/list@1<Mood>`.
pub(crate) fn with_local_names(ty: &str) -> String {
    if !ty.contains("local/") {
        return ty.to_owned();
    }
    NAMES.with(|names| {
        names
            .borrow()
            .iter()
            .fold(ty.to_owned(), |text, (expr, name)| text.replace(expr, name))
    })
}

/// The open graph's types as type choices: expression and name.
pub(crate) fn choices() -> Vec<(String, String)> {
    NAMES.with(|names| names.borrow().clone())
}

/// A member ID made from `name`, unused by `taken`: `Very happy` is `very_happy`.
fn member_id(name: &str, taken: &[&str]) -> String {
    let mut base: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if !base.starts_with(|c: char| c.is_ascii_lowercase()) {
        base.insert(0, 'm');
    }
    base.truncate(48);
    let mut id = base.clone();
    let mut n = 2;
    while taken.contains(&id.as_str()) {
        id = format!("{base}_{n}");
        n += 1;
    }
    id
}

/// What a change in the tab asks for, applied after drawing.
enum Change {
    Declare(LocalTypeDecl),
    Remove(orange_host::TypeDeclId),
    New(bool),
}

impl Editor {
    /// How many variables and nodes of the open graph use type `expr`, alone or
    /// inside another type such as a list.
    fn uses_of(&self, expr: &str) -> usize {
        let Some(document) = &self.document else {
            return 0;
        };
        let variables = document
            .declared
            .variables
            .iter()
            .filter(|variable| variable.ty.contains(expr))
            .count();
        let nodes = document
            .graph
            .nodes
            .iter()
            .filter(|node| {
                node.ports
                    .iter()
                    .any(|port| port.ty.as_deref().is_some_and(|ty| ty.contains(expr)))
            })
            .count();
        variables + nodes
    }

    pub(crate) fn types_tab(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        let Some(declared) = self.declarations().cloned() else {
            ui.weak(t("open-graph-types"));
            return;
        };
        search_field(
            ui,
            "types-search",
            &mut self.declaration_search[2],
            t("search-types"),
        );
        let query = orange_host::SearchQuery::new(&self.declaration_search[2]);
        let mut change = None;
        egui::ScrollArea::vertical()
            .id_salt("types-list")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(t("types"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button((icons::text(icons::ADD), t("record")))
                            .on_hover_text(t("record-hint"))
                            .clicked()
                        {
                            change = Some(Change::New(false));
                        }
                        if ui
                            .button((icons::text(icons::ADD), t("enum")))
                            .on_hover_text(t("enum-hint"))
                            .clicked()
                        {
                            change = Some(Change::New(true));
                        }
                    });
                });
                ui.add_space(4.0);
                if declared.types.is_empty() {
                    ui.weak(t("no-types"));
                }
                let mut visible: Vec<_> = declared
                    .types
                    .iter()
                    .filter(|decl| {
                        let kind = match decl.shape {
                            TypeShape::Enum { .. } => t("enum"),
                            TypeShape::Record { .. } => t("record"),
                        };
                        query.matches([decl.name.as_str(), kind])
                    })
                    .collect();
                visible.sort_by_cached_key(|decl| decl.name.to_lowercase());
                if visible.is_empty() && !declared.types.is_empty() {
                    ui.weak(t("no-declaration-matches"));
                }
                let open_by_default = visible.len() <= 4;
                for decl in visible {
                    let uses = self.uses_of(&decl.ty().to_string());
                    if let Some(asked) = self.type_card(ui, decl, uses, open_by_default) {
                        change = Some(asked);
                    }
                }
            });
        let Some(change) = change else {
            return;
        };
        if matches!(change, Change::New(_)) {
            self.declaration_search[2].clear();
        }
        let Some(draft) = self.editing() else {
            return;
        };
        let result = match change {
            Change::Declare(decl) => host.declare_type(draft, decl),
            Change::Remove(id) => host.remove_type(draft, id),
            Change::New(is_enum) => host.new_type_id(draft).and_then(|id| {
                let shape = if is_enum {
                    TypeShape::Enum {
                        variants: vec![VariantDecl {
                            id: "first".into(),
                            name: t("first-choice").into(),
                            payload: None,
                        }],
                    }
                } else {
                    TypeShape::Record { fields: Vec::new() }
                };
                let name = if is_enum {
                    t("new-enum")
                } else {
                    t("new-record")
                };
                host.declare_type(
                    draft,
                    LocalTypeDecl {
                        id,
                        name: name.into(),
                        shape,
                    },
                )
            }),
        };
        let _ = self.edited(host, result);
    }

    /// One type as a card: a header with its kind, its name, how much uses it and a
    /// delete button, then its choices or fields.
    fn type_card(
        &mut self,
        ui: &mut egui::Ui,
        decl: &LocalTypeDecl,
        uses: usize,
        open_by_default: bool,
    ) -> Option<Change> {
        let key = decl.id.to_string();
        let mut changed = decl.clone();
        let mut asked = None;
        let visuals = ui.visuals().clone();
        let accent = canvas::type_color(Some(&decl.ty().to_string()));
        let kind = match decl.shape {
            TypeShape::Enum { .. } => t("enum"),
            TypeShape::Record { .. } => t("record"),
        };
        let _ = egui::Frame::new()
            .fill(visuals.faint_bg_color)
            .stroke(visuals.widgets.noninteractive.bg_stroke)
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let id = ui.make_persistent_id(("orange-type", key.as_str()));
                let state = egui::collapsing_header::CollapsingState::load_with_default_open(
                    ui.ctx(),
                    id,
                    open_by_default,
                );
                let header = state.show_header(ui, |ui| {
                    badge(ui, kind, accent);
                    let width = (ui.available_width() - 96.0).max(60.0);
                    if let Some(name) =
                        self.inline_field_sized(ui, (key.clone(), "type-name"), &decl.name, width)
                        && !name.trim().is_empty()
                    {
                        changed.name = name;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if quiet_icon(ui, icons::DELETE, t("remove-type-hint"), true) {
                            asked = Some(Change::Remove(decl.id));
                        }
                        ui.weak(tf("type-uses", &[("count", uses.into())]));
                    });
                });
                let _ = header.body(|ui| match &mut changed.shape {
                    TypeShape::Enum { variants } => self.variant_rows(ui, &key, variants, accent),
                    TypeShape::Record { fields } => self.field_rows(ui, &key, fields, accent),
                });
            });
        ui.add_space(6.0);
        if asked.is_some() {
            return asked;
        }
        (changed != *decl).then_some(Change::Declare(changed))
    }

    fn variant_rows(
        &mut self,
        ui: &mut egui::Ui,
        key: &str,
        variants: &mut Vec<VariantDecl>,
        accent: egui::Color32,
    ) {
        let mut remove = None;
        let mut moved = None;
        let only = variants.len() == 1;
        for (index, variant) in variants.iter_mut().enumerate() {
            let row = ui.horizontal(|ui| {
                let hint = tf("choice-hint", &[("id", variant.id.as_str().into())]);
                drag_handle(ui, key, index, &variant.id, &hint);
                let width = (ui.available_width() - 120.0).max(60.0);
                let field = (format!("{key}/{}", variant.id), "variant-name");
                if let Some(name) = self.inline_field_sized(ui, field, &variant.name, width)
                    && !name.trim().is_empty()
                {
                    variant.name = name;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if quiet_icon(ui, icons::CLOSE, t("remove-choice"), !only) {
                        remove = Some(index);
                    }
                    let salt = (key, variant.id.as_str(), "payload");
                    variant.payload = payload_choice(ui, salt, variant.payload.take(), true);
                });
            });
            if let Some(from) = dropped(&row.response, key, ui, accent) {
                moved = Some((from, index));
            }
        }
        if let Some(index) = remove.filter(|_| variants.len() > 1) {
            let _ = variants.remove(index);
        }
        apply_row_move(variants, moved);
        if add_button(ui, t("add-choice")) {
            let taken: Vec<&str> = variants.iter().map(|v| v.id.as_str()).collect();
            let name = tf("choice-name", &[("number", (variants.len() + 1).into())]);
            let id = member_id(&name, &taken);
            self.forms
                .focus_inline((format!("{key}/{id}"), "variant-name"));
            variants.push(VariantDecl {
                id,
                name,
                payload: None,
            });
        }
    }

    fn field_rows(
        &mut self,
        ui: &mut egui::Ui,
        key: &str,
        fields: &mut Vec<FieldDecl>,
        accent: egui::Color32,
    ) {
        let mut remove = None;
        let mut moved = None;
        for (index, field) in fields.iter_mut().enumerate() {
            let row = ui.horizontal(|ui| {
                let hint = tf("field-hint", &[("id", field.id.as_str().into())]);
                drag_handle(ui, key, index, &field.id, &hint);
                let width = (ui.available_width() - 140.0).max(60.0);
                let edit = (format!("{key}/{}", field.id), "field-name");
                if let Some(name) = self.inline_field_sized(ui, edit, &field.name, width)
                    && !name.trim().is_empty()
                {
                    field.name = name;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if quiet_icon(ui, icons::CLOSE, t("remove-field"), true) {
                        remove = Some(index);
                    }
                    let _ = ui
                        .selectable_label(field.optional, "?")
                        .on_hover_text(t("optional-field-hint"))
                        .clicked()
                        .then(|| field.optional = !field.optional);
                    let salt = (key, field.id.as_str(), "type");
                    if let Some(ty) = payload_choice(ui, salt, Some(field.ty.clone()), false) {
                        field.ty = ty;
                    }
                });
            });
            if let Some(from) = dropped(&row.response, key, ui, accent) {
                moved = Some((from, index));
            }
        }
        if let Some(index) = remove {
            let _ = fields.remove(index);
        }
        apply_row_move(fields, moved);
        if add_button(ui, t("add-field")) {
            let taken: Vec<&str> = fields.iter().map(|f| f.id.as_str()).collect();
            let name = tf("field-name", &[("number", (fields.len() + 1).into())]);
            let id = member_id(&name, &taken);
            if let Ok(ty) = "core/string@1".parse() {
                self.forms
                    .focus_inline((format!("{key}/{id}"), "field-name"));
                fields.push(FieldDecl {
                    id,
                    name,
                    ty,
                    optional: false,
                });
            }
        }
    }
}

fn apply_row_move<T>(rows: &mut Vec<T>, moved: Option<(usize, usize)>) {
    if let Some((from, to)) = moved.filter(|(from, to)| from != to) {
        let row = rows.remove(from);
        rows.insert(to.min(rows.len()), row);
    }
}

/// A member being dragged to a new place: the type it belongs to and where it was.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MemberDrag {
    key: String,
    index: usize,
}

/// The handle a member is dragged by, which also says what values store.
fn drag_handle(ui: &mut egui::Ui, key: &str, index: usize, member: &str, hint: &str) {
    let weak = ui.visuals().weak_text_color();
    let id = ui.make_persistent_id(("orange-member-drag", key, member));
    let payload = MemberDrag {
        key: key.to_owned(),
        index,
    };
    let _ = ui
        .dnd_drag_source(id, payload, |ui| {
            ui.label(icons::text(icons::DRAG).color(weak))
        })
        .response
        .on_hover_text(hint);
}

/// Where a member of type `key` was let go on `row`, marking the place while it is
/// held over it.
fn dropped(row: &egui::Response, key: &str, ui: &egui::Ui, accent: egui::Color32) -> Option<usize> {
    if row
        .dnd_hover_payload::<MemberDrag>()
        .is_some_and(|drag| drag.key == key)
    {
        ui.painter().hline(
            row.rect.x_range(),
            row.rect.top(),
            egui::Stroke::new(2.0, accent),
        );
    }
    row.dnd_release_payload::<MemberDrag>()
        .filter(|drag| drag.key == key)
        .map(|drag| drag.index)
}

/// The kind of a type as a small coloured tag.
fn badge(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    let _ = egui::Frame::new()
        .fill(color.gamma_multiply(0.18))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text.to_uppercase())
                    .size(10.0)
                    .strong()
                    .color(color),
            )
        });
}

/// An icon button without a frame, dimmed until the pointer is on it. Returns whether
/// it was pressed.
fn quiet_icon(ui: &mut egui::Ui, icon: char, hint: &str, enabled: bool) -> bool {
    let weak = ui.visuals().weak_text_color();
    ui.add_enabled(
        enabled,
        egui::Button::new(icons::text(icon).color(weak)).frame(false),
    )
    .on_hover_text(hint)
    .clicked()
}

/// A wide, quiet button that adds a member. Returns whether it was pressed.
fn add_button(ui: &mut egui::Ui, label: &str) -> bool {
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    ui.add_space(2.0);
    ui.add_sized(
        [ui.available_width(), 22.0],
        egui::Button::new((icons::text(icons::ADD), label))
            .fill(egui::Color32::TRANSPARENT)
            .stroke(stroke),
    )
    .clicked()
}

/// A type chooser for a payload or field, as a coloured chip: nothing (payloads
/// only), the common types and the graph's own types.
fn payload_choice(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    current: Option<orange_host::TypeExpr>,
    payload: bool,
) -> Option<orange_host::TypeExpr> {
    let shown = current.as_ref().map_or_else(
        || egui::RichText::new("—").weak(),
        |ty| {
            egui::RichText::new(type_name(ty.as_str())).color(canvas::type_color(Some(ty.as_str())))
        },
    );
    let mut chosen = current.as_ref().map(ToString::to_string);
    let _ = egui::ComboBox::from_id_salt(salt)
        .width(78.0)
        .selected_text(shown)
        .show_select(ui, Surface::Panel, |ui| {
            if payload {
                let _ = ui.selectable_value(&mut chosen, None, t("no-value"));
            }
            for common in COMMON_TYPES {
                let _ =
                    ui.selectable_value(&mut chosen, Some(common.to_owned()), type_name(common));
            }
            for (expr, name) in choices() {
                let _ = ui.selectable_value(&mut chosen, Some(expr), name);
            }
        })
        .response
        .on_hover_text(if payload {
            t("payload-hint")
        } else {
            t("type")
        });
    chosen.and_then(|text| text.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_open_graphs_types_are_shown_by_name() {
        let id = "6c0e4a52-8b1f-4a5e-9c3d-1e2f3a4b5c6d".parse().unwrap();
        let mood = LocalTypeDecl {
            id,
            name: "Mood".into(),
            shape: TypeShape::Enum {
                variants: vec![VariantDecl {
                    id: "calm".into(),
                    name: "Calm".into(),
                    payload: None,
                }],
            },
        };
        let expr = mood.ty().to_string();
        remember(std::slice::from_ref(&mood));
        assert_eq!(type_name(&expr), "Mood");
        assert_eq!(type_name(&format!("core/list@1<{expr}>")), "list<Mood>");
        assert_eq!(choices(), [(expr.clone(), "Mood".to_owned())]);
        remember(&[]);
        assert_ne!(
            type_name(&expr),
            "Mood",
            "a closed graph's names are forgotten"
        );
    }

    #[test]
    fn member_ids_come_from_names_and_never_repeat() {
        assert_eq!(member_id("Very happy", &[]), "very_happy");
        assert_eq!(member_id("Very happy", &["very_happy"]), "very_happy_2");
        assert_eq!(member_id("1st", &[]), "m1st");
        assert!(member_id(&"x".repeat(100), &[]).len() <= 48);
    }
}
