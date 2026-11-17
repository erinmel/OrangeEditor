//! The Types tab: enums and records the open graph declares itself, so a person can
//! make their own choices (Mood: calm, happy) and structures without a package
//! (12 §6.5). Values store member IDs, so names can change freely.
use super::*;
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
    pub(crate) fn types_tab(&mut self, ui: &mut egui::Ui, host: &mut Host) {
        let Some(declared) = self.declarations().cloned() else {
            ui.weak("Open a graph to see its types.");
            return;
        };
        let mut change = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Types");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button("+ Record")
                        .on_hover_text("A value with named fields, such as a point with x and y.")
                        .clicked()
                    {
                        change = Some(Change::New(false));
                    }
                    if ui
                        .button("+ Enum")
                        .on_hover_text("One of a few choices, such as a mood or a mode.")
                        .clicked()
                    {
                        change = Some(Change::New(true));
                    }
                });
            });
            if declared.types.is_empty() {
                ui.weak("This graph declares no types. Variables and values can use one once it is added.");
            }
            for decl in &declared.types {
                ui.separator();
                if let Some(asked) = self.type_card(ui, decl) {
                    change = Some(asked);
                }
            }
        });
        let Some(change) = change else {
            return;
        };
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
                            name: "First".into(),
                            payload: None,
                        }],
                    }
                } else {
                    TypeShape::Record { fields: Vec::new() }
                };
                let name = if is_enum { "New enum" } else { "New record" };
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

    /// One type: its name, its members with their types, and add and remove buttons.
    fn type_card(&mut self, ui: &mut egui::Ui, decl: &LocalTypeDecl) -> Option<Change> {
        let key = decl.id.to_string();
        let mut changed = decl.clone();
        let mut asked = None;
        ui.horizontal(|ui| {
            let kind = match decl.shape {
                TypeShape::Enum { .. } => "Enum",
                TypeShape::Record { .. } => "Record",
            };
            if let Some(name) = self.inline_field(ui, (key.clone(), "type-name"), &decl.name)
                && !name.trim().is_empty()
            {
                changed.name = name;
            }
            ui.weak(kind);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("x")
                    .on_hover_text("Remove the type; what uses it shows as a problem.")
                    .clicked()
                {
                    asked = Some(Change::Remove(decl.id));
                }
            });
        });
        if asked.is_some() {
            return asked;
        }
        match &mut changed.shape {
            TypeShape::Enum { variants } => self.variant_rows(ui, &key, variants),
            TypeShape::Record { fields } => self.field_rows(ui, &key, fields),
        }
        (changed != *decl).then_some(Change::Declare(changed))
    }

    fn variant_rows(&mut self, ui: &mut egui::Ui, key: &str, variants: &mut Vec<VariantDecl>) {
        let mut remove = None;
        for (index, variant) in variants.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                let field = (format!("{key}/{}", variant.id), "variant-name");
                if let Some(name) = self.inline_field(ui, field, &variant.name)
                    && !name.trim().is_empty()
                {
                    variant.name = name;
                }
                let salt = (key, variant.id.as_str(), "payload");
                variant.payload = payload_choice(ui, salt, variant.payload.take());
                if ui
                    .small_button("x")
                    .on_hover_text("Remove this choice")
                    .clicked()
                {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove.filter(|_| variants.len() > 1) {
            let _ = variants.remove(index);
        }
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            if ui.small_button("+ choice").clicked() {
                let taken: Vec<&str> = variants.iter().map(|v| v.id.as_str()).collect();
                let name = format!("Choice {}", variants.len() + 1);
                let id = member_id(&name, &taken);
                variants.push(VariantDecl {
                    id,
                    name,
                    payload: None,
                });
            }
        });
    }

    fn field_rows(&mut self, ui: &mut egui::Ui, key: &str, fields: &mut Vec<FieldDecl>) {
        let mut remove = None;
        for (index, field) in fields.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                let edit = (format!("{key}/{}", field.id), "field-name");
                if let Some(name) = self.inline_field(ui, edit, &field.name)
                    && !name.trim().is_empty()
                {
                    field.name = name;
                }
                let salt = (key, field.id.as_str(), "type");
                if let Some(ty) = payload_choice(ui, salt, Some(field.ty.clone())) {
                    field.ty = ty;
                }
                let _ = ui
                    .checkbox(&mut field.optional, "optional")
                    .on_hover_text("A value may leave this field out.");
                if ui
                    .small_button("x")
                    .on_hover_text("Remove this field")
                    .clicked()
                {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            let _ = fields.remove(index);
        }
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            if ui.small_button("+ field").clicked() {
                let taken: Vec<&str> = fields.iter().map(|f| f.id.as_str()).collect();
                let name = format!("Field {}", fields.len() + 1);
                let id = member_id(&name, &taken);
                if let Ok(ty) = "core/string@1".parse() {
                    fields.push(FieldDecl {
                        id,
                        name,
                        ty,
                        optional: false,
                    });
                }
            }
        });
    }
}

/// A type chooser for a payload or field: nothing (payloads only), the common types and
/// the graph's own types.
fn payload_choice(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    current: Option<orange_host::TypeExpr>,
) -> Option<orange_host::TypeExpr> {
    let shown = current
        .as_ref()
        .map_or_else(|| "no value".to_owned(), |ty| type_name(ty.as_str()));
    let mut chosen = current.as_ref().map(ToString::to_string);
    let _ = egui::ComboBox::from_id_salt(salt)
        .width(90.0)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            let _ = ui.selectable_value(&mut chosen, None, "no value");
            for common in COMMON_TYPES {
                let _ =
                    ui.selectable_value(&mut chosen, Some(common.to_owned()), type_name(common));
            }
            for (expr, name) in choices() {
                let _ = ui.selectable_value(&mut chosen, Some(expr), name);
            }
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
        assert!(type_name(&expr).starts_with("local/"));
    }

    #[test]
    fn member_ids_come_from_names_and_never_repeat() {
        assert_eq!(member_id("Very happy", &[]), "very_happy");
        assert_eq!(member_id("Very happy", &["very_happy"]), "very_happy_2");
        assert_eq!(member_id("1st", &[]), "m1st");
        assert!(member_id(&"x".repeat(100), &[]).len() <= 48);
    }
}
