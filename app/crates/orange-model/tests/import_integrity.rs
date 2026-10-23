//! Structural import bounds and ambiguous-lock rejection.

mod common;

use common::{branch_node, minimal_document};
use orange_model::document::codec::{ImportLimits, from_json_with_limits};
use orange_model::error::ImportResource;
use orange_model::{CodecError, from_json};
use serde_json::{Value, json};

fn package(id: &str, version: &str, digest: &str) -> Value {
    json!({"id": id, "version": version, "content_digest": digest})
}

#[test]
fn duplicate_package_lock_identity_is_rejected_for_equal_or_conflicting_payloads() {
    let digest_a = format!("sha256:{}", "a".repeat(64));
    let digest_b = format!("sha256:{}", "b".repeat(64));
    for pair in [
        [
            package("org.example.pkg", "1.0.0", &digest_a),
            package("org.example.pkg", "1.0.0", &digest_b),
        ],
        [
            package("org.example.pkg", "1.0.0", &digest_b),
            package("org.example.pkg", "1.0.0", &digest_a),
        ],
        [
            package("org.example.pkg", "1.0.0", &digest_a),
            package("org.example.pkg", "1.0.0", &digest_a),
        ],
    ] {
        let text = minimal_document(json!({"package_lock": pair}));
        assert!(matches!(
            from_json(&text),
            Err(CodecError::DuplicateLock {
                kind: "package",
                ..
            })
        ));
    }
}

#[test]
fn duplicate_type_lock_identity_is_rejected_even_when_codec_differs() {
    let text = minimal_document(json!({
        "type_lock": [
            {"type": "core/i64@1", "codec": "orange.value.v1"},
            {"type": "core/i64@1", "codec": "org.example.other-v1"}
        ]
    }));

    assert!(matches!(
        from_json(&text),
        Err(CodecError::DuplicateLock {
            kind: "type",
            identity,
        }) if identity == "core/i64@1"
    ));
}

#[test]
fn import_limits_accept_exact_boundaries_and_report_typed_overages() {
    let mut document = json!({"nodes": [branch_node(1)], "edges": []});
    let text = minimal_document(document.clone());
    let exact = ImportLimits::new(text.len(), 1, 0);
    assert!(from_json_with_limits(&text, exact).is_ok());

    assert!(matches!(
        from_json_with_limits(&text, ImportLimits::new(text.len() - 1, 1, 0)),
        Err(CodecError::ImportLimitExceeded {
            resource: ImportResource::Bytes,
            max,
            actual,
        }) if max == text.len() - 1 && actual == text.len()
    ));

    assert!(matches!(
        from_json_with_limits(&text, ImportLimits::new(text.len(), 0, 0)),
        Err(CodecError::ImportLimitExceeded {
            resource: ImportResource::Nodes,
            max: 0,
            actual: 1,
        })
    ));

    document["nodes"] = json!([]);
    document["edges"] = json!([common::edge(1, 1, 2)]);
    let edge_text = minimal_document(document);
    assert!(matches!(
        from_json_with_limits(&edge_text, ImportLimits::new(edge_text.len(), 0, 0)),
        Err(CodecError::ImportLimitExceeded {
            resource: ImportResource::Edges,
            max: 0,
            actual: 1,
        })
    ));
}

#[test]
fn byte_limit_precedes_json_parsing() {
    assert!(matches!(
        from_json_with_limits("not json", ImportLimits::new(0, 0, 0)),
        Err(CodecError::ImportLimitExceeded {
            resource: ImportResource::Bytes,
            max: 0,
            actual: 8,
        })
    ));
}
