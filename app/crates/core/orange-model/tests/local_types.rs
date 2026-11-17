//! Record and enum types a document declares itself (11 §5.4, 12 §6.5): they need no
//! package, keep their identity when renamed, and check, decode and key maps like
//! package types with structural derives.

use crate::behaviour::common;

use std::sync::Arc;

use crate::catalog::RegistrySnapshot;
use crate::document::{TypeShape, VariantDecl, local_type};
use crate::id::TypeDeclId;
use crate::local_types::with_document_types;
use crate::validation::{DiagnosticCode, HostOrigin};
use crate::{DocCommand, GraphDocument, analyze, from_json, to_json};
use common::{minimal_document, node_id};
use serde_json::{Value, json};

const MOOD: &str = "6c0e4a52-8b1f-4a5e-9c3d-1e2f3a4b5c6d";
const POINT: &str = "7d1f5b63-9c20-4b6f-8d4e-2f3a4b5c6d7e";

fn base() -> Arc<RegistrySnapshot> {
    RegistrySnapshot::core(HostOrigin::new("0.1.0", format!("sha256:{}", "b".repeat(64))).unwrap())
}

fn ty(id: &str) -> String {
    local_type(id.parse().unwrap()).to_string()
}

/// `Mood` (happy, sad, level(f64)) and `Point` (x: f64, y: f64, label?: string,
/// mood: Mood), with a variable of each and one of `map<Mood, u32>`.
fn document(types: Value, variables: Vec<Value>) -> GraphDocument {
    from_json(&minimal_document(
        json!({"types": types, "variables": variables}),
    ))
    .unwrap()
}

fn mood_and_point() -> Value {
    json!([
        {"id": MOOD, "name": "Mood", "kind": "enum", "variants": [
            {"id": "happy", "name": "Happy"},
            {"id": "sad", "name": "Sad"},
            {"id": "level", "name": "Level", "payload": "core/f64@1"}
        ]},
        {"id": POINT, "name": "Point", "kind": "record", "fields": [
            {"id": "x", "name": "X", "type": "core/f64@1"},
            {"id": "y", "name": "Y", "type": "core/f64@1"},
            {"id": "label", "name": "Label", "type": "core/string@1", "optional": true},
            {"id": "mood", "name": "Mood", "type": ty(MOOD)}
        ]}
    ])
}

fn variable(n: u8, ty: &str, value: Option<Value>) -> Value {
    let mut variable = json!({"id": node_id(n), "name": "v", "type": ty, "scope": "run"});
    if let Some(value) = value {
        variable["init"] = json!({"type": ty, "value": value});
    }
    variable
}

fn codes(document: &GraphDocument, registry: &RegistrySnapshot) -> Vec<DiagnosticCode> {
    analyze(document, registry)
        .report()
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect()
}

#[test]
fn a_document_declares_an_enum_and_a_record_and_uses_them_without_a_package() {
    let half = format!("{:016x}", 0.5f64.to_bits());
    let light = "9f3b7d85-be42-4d81-af60-4b5c6d7e8f90";
    let keyed = format!("core/map@1<{},core/u32@1>", ty(light));
    let mut types = mood_and_point();
    types
        .as_array_mut()
        .unwrap()
        .push(json!({"id": light, "name": "Light", "kind": "enum",
        "variants": [{"id": "on", "name": "On"}, {"id": "off", "name": "Off"}]}));
    let document = document(
        types,
        vec![
            variable(
                1,
                &ty(MOOD),
                Some(json!({"variant": "level", "payload": half})),
            ),
            variable(2, &ty(MOOD), Some(json!({"variant": "happy"}))),
            variable(
                3,
                &ty(POINT),
                Some(json!({"x": half, "y": half, "mood": {"variant": "sad"}})),
            ),
            // Local enums derive `hash` and `eq` when their members have them, so
            // `Light` keys maps.
            variable(
                4,
                &keyed,
                Some(json!([[{"variant": "on"}, 1], [{"variant": "off"}, 2]])),
            ),
        ],
    );
    let base = base();
    let registry = with_document_types(&base, &document);
    assert_eq!(codes(&document, &registry), []);
    // `Mood` has a float payload, so it has no `eq` or `hash` and keys no map.
    let mood_keys = format!("core/map@1<{},core/u32@1>", ty(MOOD));
    let mut by_mood = document.clone();
    let _ = by_mood
        .apply([DocCommand::UpsertVariable(
            serde_json::from_value(variable(5, &mood_keys, None)).unwrap(),
        )])
        .unwrap();
    assert_eq!(
        codes(&by_mood, &with_document_types(&base, &by_mood)),
        [DiagnosticCode::DeclarationType]
    );
    // The package registry alone does not know them.
    assert!(codes(&document, &base).contains(&DiagnosticCode::DeclarationType));
    // A document without types checks against the package registry itself.
    let plain = from_json(&minimal_document(json!({}))).unwrap();
    assert!(Arc::ptr_eq(&with_document_types(&base, &plain), &base));

    // Saving keeps the declarations; loading them again gives the same document.
    let saved = to_json(&document).unwrap();
    assert_eq!(from_json(&saved).unwrap(), document);
}

#[test]
fn renaming_a_type_or_a_variant_keeps_saved_values() {
    let mut document = document(
        mood_and_point(),
        vec![variable(1, &ty(MOOD), Some(json!({"variant": "happy"})))],
    );
    let id: TypeDeclId = MOOD.parse().unwrap();
    let mut renamed = document.types()[&id].clone();
    renamed.name = "Feeling".into();
    if let TypeShape::Enum { variants } = &mut renamed.shape {
        variants[0].name = "Glad".into();
        variants.push(VariantDecl {
            id: "calm".into(),
            name: "Calm".into(),
            payload: None,
        });
    }
    let _ = document.apply([DocCommand::UpsertType(renamed)]).unwrap();
    let registry = with_document_types(&base(), &document);
    assert_eq!(codes(&document, &registry), []);
    assert_eq!(document.types()[&id].ty().to_string(), ty(MOOD));

    // Removing the type leaves its users unresolved, and undo would bring it back.
    let _ = document.apply([DocCommand::RemoveType(id)]).unwrap();
    let registry = with_document_types(&base(), &document);
    assert!(codes(&document, &registry).contains(&DiagnosticCode::DeclarationType));
}

#[test]
fn malformed_declarations_are_reported_and_do_not_hide_the_rest() {
    for broken in [
        json!({"id": MOOD, "name": " ", "kind": "enum", "variants": [{"id": "a", "name": "A"}]}),
        json!({"id": MOOD, "name": "Mood", "kind": "enum", "variants": []}),
        json!({"id": MOOD, "name": "Mood", "kind": "enum", "variants": [
            {"id": "a", "name": "A"}, {"id": "a", "name": "Again"}]}),
        json!({"id": MOOD, "name": "Mood", "kind": "enum", "variants": [
            {"id": "Upper", "name": "A"}]}),
        json!({"id": MOOD, "name": "Mood", "kind": "enum", "variants": [
            {"id": "a", "name": "A", "payload": "org.example/missing@1"}]}),
    ] {
        let point = mood_and_point()[1].clone();
        let document = document(json!([broken, point]), vec![variable(2, &ty(POINT), None)]);
        let registry = with_document_types(&base(), &document);
        let codes = codes(&document, &registry);
        assert!(codes.contains(&DiagnosticCode::TypeDeclaration), "{broken}");
        // `Point` itself still resolves; only the declarations are reported.
        assert!(
            !codes.contains(&DiagnosticCode::DeclarationType),
            "{broken}: {codes:?}"
        );
    }
}

#[test]
fn a_type_may_name_itself() {
    let tree = "8e2a6c74-ad31-4c70-9e5f-3a4b5c6d7e8f";
    let list = format!("core/list@1<{}>", ty(tree));
    let document = document(
        json!([{"id": tree, "name": "Tree", "kind": "record", "fields": [
            {"id": "children", "name": "Children", "type": list}
        ]}]),
        vec![variable(
            1,
            &ty(tree),
            Some(json!({"children": [{"children": []}]})),
        )],
    );
    let registry = with_document_types(&base(), &document);
    assert_eq!(codes(&document, &registry), []);
}
