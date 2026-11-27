//! Record and enum types a document declares itself (11 §5.4, 12 §6.5).
//!
//! A document's types live in its `types` section with stable IDs; each is the nominal
//! type `local/t<id>@1`, owned by the reserved `local` namespace. A host checks, compiles
//! and runs a document against [`with_document_types`]: the package registry with the
//! document's types added, each deriving `eq`, `hash`, `display` and `serialize`
//! structurally. Members may name core types, package types and the document's other
//! local types, including the type itself.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::catalog::{RegistryBuilder, RegistrySnapshot};
use crate::document::{GraphDocument, LOCAL_NAMESPACE, LocalTypeDecl, TypeShape};
use crate::type_term::TypeTerm;
use crate::types::{Constraint, TypeRegistrationError, TypeRegistry};
use crate::validation::{Diagnostic, DiagnosticCode, Subject};
use crate::value::literal_codec::{EnumSchema, FieldSchema, RecordSchema, VariantSchema};

/// Most variants or fields one local type may have.
pub const MAX_MEMBERS: usize = 256;

/// Whether `id` can name a variant or field: 1 to 64 of `a-z`, `0-9`, `_` and `-`,
/// starting with a letter. Saved values and Switch cases store it.
pub fn is_member_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id.starts_with(|c: char| c.is_ascii_lowercase())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// Whether `declaration` has a name, at least one variant when it is an enum, at most
/// [`MAX_MEMBERS`] members, and well-formed member IDs that do not repeat. Member types
/// are checked separately, once the document's type names are known.
fn well_shaped(declaration: &LocalTypeDecl) -> bool {
    let ids: Vec<&str> = match &declaration.shape {
        TypeShape::Enum { variants } => variants.iter().map(|v| v.id.as_str()).collect(),
        TypeShape::Record { fields } => fields.iter().map(|f| f.id.as_str()).collect(),
    };
    let enum_without_variants =
        matches!(&declaration.shape, TypeShape::Enum { variants } if variants.is_empty());
    let mut seen = BTreeSet::new();
    !declaration.name.trim().is_empty()
        && !enum_without_variants
        && ids.len() <= MAX_MEMBERS
        && ids.iter().all(|id| is_member_id(id) && seen.insert(*id))
}

/// The types `declaration`'s members name, parsed.
fn member_types(declaration: &LocalTypeDecl) -> Vec<Option<TypeTerm>> {
    let parse = |ty: &crate::id::TypeExpr| TypeTerm::parse(ty).ok();
    match &declaration.shape {
        TypeShape::Enum { variants } => variants
            .iter()
            .filter_map(|variant| variant.payload.as_ref().map(parse))
            .collect(),
        TypeShape::Record { fields } => fields.iter().map(|field| parse(&field.ty)).collect(),
    }
}

/// Checks every declaration against `registry`, which should be the one
/// [`with_document_types`] made for this document: there, member types that name the
/// document's own types resolve.
pub fn validate_local_types(
    document: &GraphDocument,
    registry: &RegistrySnapshot,
) -> Vec<Diagnostic> {
    document
        .types()
        .values()
        .filter(|declaration| {
            !well_shaped(declaration)
                || member_types(declaration).iter().any(|term| {
                    term.as_ref()
                        .is_none_or(|term| registry.types().validate(term).is_err())
                })
        })
        .map(|declaration| {
            Diagnostic::new(
                DiagnosticCode::TypeDeclaration,
                Subject::default().with_local_type(declaration.id),
                registry.origin(),
            )
        })
        .collect()
}

/// `base` with the types `document` declares added. A declaration with a malformed
/// shape is left out, and a member whose type does not resolve is dropped from its type,
/// so the rest of the document still checks; [`validate_local_types`] reports both.
/// A document without types gets `base` itself.
pub fn with_document_types(
    base: &Arc<RegistrySnapshot>,
    document: &GraphDocument,
) -> Arc<RegistrySnapshot> {
    if document.types().is_empty() {
        return Arc::clone(base);
    }
    let usable: Vec<&LocalTypeDecl> = document
        .types()
        .values()
        .filter(|declaration| well_shaped(declaration))
        .collect();
    // First every local type with no members, so member types can be checked against a
    // registry that knows all of the document's type names.
    let mut refused = BTreeSet::new();
    let Ok(names) = register(base.types(), &usable, None, &refused) else {
        return Arc::clone(base);
    };
    // A derive the members cannot support (a float has no `eq`) or that would need a
    // proof about the type itself (a type that contains itself) is left off; the type
    // keeps every other derive. Each refusal removes one, so this ends.
    let types = loop {
        match register(base.types(), &usable, Some(&names), &refused) {
            Ok(types) => break types,
            Err(
                TypeRegistrationError::RecursiveDerivation { ty, constraint }
                | TypeRegistrationError::InvalidDerivedConstraint { ty, constraint },
            ) if !refused.contains(&(ty.clone(), constraint.clone())) => {
                let _ = refused.insert((ty, constraint));
            }
            Err(_) => return Arc::clone(base),
        }
    };
    let types = Arc::new(types);
    RegistryBuilder::from_snapshot(base)
        .converters(base.converters().over(types))
        .build()
        .unwrap_or_else(|_| Arc::clone(base))
}

/// `base`'s types plus `declarations`. Without `names` each type is registered with no
/// payloads or fields, only to be named; with it, the members whose type `names`
/// resolves are kept.
fn register(
    base: &TypeRegistry,
    declarations: &[&LocalTypeDecl],
    names: Option<&TypeRegistry>,
    refused: &BTreeSet<(String, String)>,
) -> Result<TypeRegistry, TypeRegistrationError> {
    let first_pass = names.is_none();
    // Without `names` nothing is kept, so each type is only named.
    let keep = |term: &TypeTerm| names.is_some_and(|names| names.validate(term).is_ok());
    let mut types = base.extended();
    for declaration in declarations {
        let ty = declaration.ty();
        let registered = match &declaration.shape {
            TypeShape::Enum { variants } => {
                // A payload that does not resolve is left off; its variant stays.
                let variants = variants
                    .iter()
                    .map(|variant| VariantSchema {
                        variant_id: variant.id.clone(),
                        payload: variant
                            .payload
                            .as_ref()
                            .and_then(|payload| TypeTerm::parse(payload).ok())
                            .filter(keep),
                    })
                    .collect();
                types.register_enum(LOCAL_NAMESPACE, &ty, EnumSchema { variants })
            }
            TypeShape::Record { fields } => {
                let fields = if first_pass {
                    Vec::new()
                } else {
                    fields
                        .iter()
                        .filter_map(|field| {
                            Some(FieldSchema {
                                field_id: field.id.clone(),
                                ty: TypeTerm::parse(&field.ty).ok().filter(keep)?,
                                optional: field.optional,
                            })
                        })
                        .collect()
                };
                types.register_record(LOCAL_NAMESPACE, &ty, RecordSchema { fields })
            }
        };
        // A type the registry refuses is left out; checking reports its users.
        if registered.is_err() {
            continue;
        }
        for constraint in [
            Constraint::Eq,
            Constraint::Hash,
            Constraint::Display,
            Constraint::Serialize,
        ] {
            if refused.contains(&(ty.to_string(), constraint.id_name().to_owned())) {
                continue;
            }
            let _ = types.derive_constraint(LOCAL_NAMESPACE, &ty, constraint);
        }
    }
    types.build()
}
