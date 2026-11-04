//! Provider specializations published by package transactions (11 §5.5, 12 §8): a package
//! restricts which instantiations its generic provider supports, and inference reports
//! `E-NO-SPECIALIZATION` for any other.

use std::sync::Arc;

use orange_model::binding::{NewNode, new_node_edit};
use orange_model::catalog::{NodeDefinition, ProviderKey, RegistrySnapshot, TypeArgs};
use orange_model::document::DefinitionRef;
use orange_model::validation::{DiagnosticCode, HostOrigin};
use orange_model::{GraphDocument, validate};
use orange_packages::{
    HOST_API_VERSION, MemoryJournal, PackageContents, PackageManifest, PackageStore,
    TransactionError,
};
use serde_json::json;

const PACKAGE: &str = "org.example.math";
const ECHO: &str = "org.example.math/echo";
const HEX: &str = "1111111111111111111111111111111111111111111111111111111111111111";

fn echo() -> DefinitionRef {
    DefinitionRef {
        id: ECHO.parse().unwrap(),
        version: 1,
    }
}

fn manifest() -> PackageManifest {
    let value = json!({
        "format": "orange.plugin", "format_version": 1,
        "id": PACKAGE, "version": "1.0.0", "publisher": "org.example",
        "host_api": {"major": 1, "min_minor": 0, "max_minor": 0},
        "contributions": {
            "nodes": [ECHO],
            "providers": [{"slot": format!("node:{ECHO}@1"), "id": "impl"}]
        },
        "integrity": {"algorithm": "sha256", "payload_digest": HEX}
    });
    PackageManifest::from_json(&serde_json::to_vec(&value).unwrap()).unwrap()
}

/// A generic pure definition: `a: $T` in, `out: $T` out.
fn definition() -> NodeDefinition {
    NodeDefinition::from_json(
        &json!({
            "definition_id": ECHO, "definition_version": 1, "config_version": 1,
            "kind": "pure", "determinism": "deterministic",
            "type_params": [{"name": "T"}],
            "ports": [
                {"port_id": "a", "direction": "input", "kind": "data", "type": "$T"},
                {"port_id": "out", "direction": "output", "kind": "data", "type": "$T",
                 "source": "computed"}
            ]
        })
        .to_string(),
    )
    .unwrap()
}

fn args(ty: &str) -> TypeArgs {
    TypeArgs::from([("T".into(), ty.parse().unwrap())])
}

fn provider() -> ProviderKey {
    ProviderKey {
        package: PACKAGE.into(),
        version: "1.0.0".into(),
        content_digest: format!("sha256:{HEX}"),
        id: "impl".into(),
    }
}

fn store() -> PackageStore {
    let origin = HostOrigin::new("0.1.0", format!("sha256:{}", "c".repeat(64))).unwrap();
    PackageStore::new(
        RegistrySnapshot::core(origin),
        HOST_API_VERSION,
        MemoryJournal::new(),
    )
}

/// One echo node bound to the default provider, pinned to `ty` with a literal of that type.
fn graph(registry: &RegistrySnapshot, ty: &str, value: serde_json::Value) -> GraphDocument {
    let mut document = GraphDocument::new("20000000-0000-4000-8000-000000000001".parse().unwrap());
    let mut node = NewNode::new(
        "10000000-0000-4000-8000-000000000001".parse().unwrap(),
        echo(),
    );
    node.type_pins.insert("T".into(), ty.parse().unwrap());
    node.inputs.insert(
        "a".parse().unwrap(),
        serde_json::from_value(json!({"type": ty, "value": value})).unwrap(),
    );
    let edit = new_node_edit(&document, registry, node).unwrap();
    let _ = document.apply(edit.commands).unwrap();
    document
}

fn codes(document: &GraphDocument, registry: &RegistrySnapshot) -> Vec<DiagnosticCode> {
    validate(document, registry)
        .diagnostics
        .into_iter()
        .map(|item| item.code)
        .collect()
}

#[test]
fn published_specializations_restrict_resolution() {
    let mut store = store();
    let contents = PackageContents::new(manifest(), vec![definition()]).with_specializations(
        "impl",
        echo(),
        vec![args("core/i64@1")],
    );
    assert_eq!(
        contents.specializations("impl", &echo()),
        Some([args("core/i64@1")].as_slice())
    );
    let _ = store.install(contents).unwrap();
    let registry: Arc<RegistrySnapshot> = store.select_default(echo(), provider()).unwrap();
    assert_eq!(
        registry.specializations(&echo(), Some(&provider())),
        Some([args("core/i64@1")].as_slice())
    );

    assert_eq!(
        codes(&graph(&registry, "core/i64@1", json!("5")), &registry),
        []
    );
    assert_eq!(
        codes(&graph(&registry, "core/i32@1", json!(5)), &registry),
        [DiagnosticCode::NoSpecialization]
    );
}

#[test]
fn without_a_list_the_provider_delegates_to_constraints() {
    let mut store = store();
    let _ = store
        .install(PackageContents::new(manifest(), vec![definition()]))
        .unwrap();
    let registry = store.select_default(echo(), provider()).unwrap();
    assert_eq!(registry.specializations(&echo(), Some(&provider())), None);
    assert_eq!(
        codes(&graph(&registry, "core/i32@1", json!(5)), &registry),
        []
    );
}

#[test]
fn specializations_must_name_a_declared_provider_slot() {
    let mut store = store();
    let before = Arc::clone(store.current());
    let undeclared = PackageContents::new(manifest(), vec![definition()]).with_specializations(
        "other",
        echo(),
        vec![args("core/i64@1")],
    );
    let expected = TransactionError::UndeclaredSpecialization {
        provider: "other".into(),
        definition: echo(),
    };
    assert_eq!(store.install(undeclared).unwrap_err(), expected);
    assert!(Arc::ptr_eq(store.current(), &before));
}
