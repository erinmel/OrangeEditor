mod common;

use std::{collections::BTreeMap, sync::Arc};

use common::minimal_document;
use orange_model::catalog::{RegistryBuilder, RegistrySnapshot};
use orange_model::type_term::TypeTerm;
use orange_model::types::TypeRegistryBuilder;
use orange_model::validation::{DiagnosticCode, HostOrigin, validate_streams};
use orange_model::value::literal_codec::{EnumSchema, FieldSchema, RecordSchema, VariantSchema};
use orange_model::{GraphDocument, from_json};
use serde_json::{Value, json};

const VAR: &str = "50000000-0000-4000-8000-000000000001";
const STREAM: &str = "core/stream@1<core/i64@1>";
const HANDLE: &str = "core/handle@1<org.orange.example/session@1>";

fn codes(ty: &str, scope: &str, extra: Value) -> Vec<DiagnosticCode> {
    let mut decl = json!({"id": VAR, "name": "v", "type": ty, "scope": scope});
    decl.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let doc: GraphDocument = from_json(&minimal_document(json!({"variables": [decl]}))).unwrap();
    let registry = RegistrySnapshot::core(
        HostOrigin::new("0.1.0", format!("sha256:{}", "b".repeat(64))).unwrap(),
    );
    validate_streams(&doc, &BTreeMap::new(), &registry)
        .iter()
        .map(|d| d.code)
        .collect()
}

#[test]
fn streams_are_stored_in_no_variable_even_nested() {
    for scope in ["local", "run", "workspace"] {
        assert_eq!(
            codes(STREAM, scope, json!({})),
            [DiagnosticCode::StreamStore],
            "{scope}"
        );
    }
    assert_eq!(
        codes(&format!("core/list@1<{STREAM}>"), "run", json!({})),
        [DiagnosticCode::StreamStore]
    );
}

#[test]
fn handles_may_not_be_persistent_or_workspace_variables() {
    assert!(codes(HANDLE, "run", json!({})).is_empty());
    assert!(codes(HANDLE, "local", json!({})).is_empty());
    assert_eq!(
        codes(HANDLE, "workspace", json!({})),
        [DiagnosticCode::HandlePersist]
    );
    assert_eq!(
        codes(HANDLE, "run", json!({"persistent": true})),
        [DiagnosticCode::HandlePersist]
    );
    assert!(codes("core/i64@1", "workspace", json!({"persistent": true})).is_empty());
}

#[test]
fn resource_variables_reject_an_initializer_literal() {
    let init = json!({"init": {"type": HANDLE, "value": "x"}});
    let result = codes(HANDLE, "run", init);
    assert_eq!(result, [DiagnosticCode::HandleLiteral]);
}

fn nominal_registry() -> Arc<RegistrySnapshot> {
    let term = |text: &str| TypeTerm::parse(&text.parse().unwrap()).unwrap();
    let mut types = TypeRegistryBuilder::core();
    types
        .register_handle_kind("org.resource", &"org.resource/session@1".parse().unwrap())
        .unwrap();
    types
        .register_enum(
            "org.resource",
            &"org.resource/choice@1".parse().unwrap(),
            EnumSchema {
                variants: vec![
                    VariantSchema {
                        variant_id: "stream".into(),
                        payload: Some(term(STREAM)),
                    },
                    VariantSchema {
                        variant_id: "handle".into(),
                        payload: Some(term("core/handle@1<org.resource/session@1>")),
                    },
                ],
            },
        )
        .unwrap();
    for (name, payload) in [("packet", "org.resource/choice@1"), ("clean", "core/i64@1")] {
        let ty = format!("org.resource/{name}@1");
        types
            .register_record(
                "org.resource",
                &ty.parse().unwrap(),
                RecordSchema {
                    fields: vec![
                        FieldSchema {
                            field_id: "next".into(),
                            ty: term(&ty),
                            optional: true,
                        },
                        FieldSchema {
                            field_id: "payload".into(),
                            ty: term(payload),
                            optional: false,
                        },
                    ],
                },
            )
            .unwrap();
    }
    RegistryBuilder::core(HostOrigin::new("0.1.0", format!("sha256:{}", "b".repeat(64))).unwrap())
        .types(Arc::new(types.build().unwrap()))
        .build()
        .unwrap()
}

#[test]
fn recursive_nominal_schemas_cannot_hide_resources_from_storage_rules() {
    let registry = nominal_registry();
    let document = from_json(&minimal_document(json!({"variables": [
        {"id":VAR,"name":"v","type":"org.resource/packet@1","scope":"workspace",
            "init":{"type":"org.resource/packet@1","value":{}}}
    ]})))
    .unwrap();
    let diagnostics = validate_streams(&document, &BTreeMap::new(), &registry);
    for code in [
        DiagnosticCode::StreamStore,
        DiagnosticCode::HandlePersist,
        DiagnosticCode::HandleLiteral,
    ] {
        assert!(
            diagnostics.iter().any(|item| item.code == code),
            "{diagnostics:?}"
        );
    }
    let document = from_json(&minimal_document(json!({"variables": [
        {"id":VAR,"name":"v","type":"org.resource/clean@1","scope":"workspace"}
    ]})))
    .unwrap();
    assert!(validate_streams(&document, &BTreeMap::new(), &registry).is_empty());
}

#[test]
fn registers_check_resources_in_the_fresh_nominal_cell_type() {
    let registry = nominal_registry();
    let document = from_json(&minimal_document(json!({"nodes": [{
        "id":common::node_id(1),"definition_id":"core/register","definition_version":1,
        "config_version":1,"config":{},"type_pins":{"T":"org.resource/packet@1"},
        "type_args":{"T":"core/i64@1"}
    }]})))
    .unwrap();
    let report = orange_model::validation::analyze(&document, &registry).into_report();
    assert!(
        report
            .diagnostics
            .iter()
            .any(|item| item.code == DiagnosticCode::StreamStore)
    );
}
