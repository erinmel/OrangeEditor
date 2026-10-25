mod common;

use std::collections::BTreeMap;

use common::minimal_document;
use orange_model::catalog::RegistrySnapshot;
use orange_model::validation::{DiagnosticCode, HostOrigin, validate_streams};
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
