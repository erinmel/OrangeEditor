//! Structural import bounds, ambiguous-lock rejection and the checked Serde boundary.

mod common;

use common::{branch_node, minimal_document};
use orange_model::document::codec::{
    ImportBudget, ImportLimits, from_json_with_budget, from_json_with_limits,
};
use orange_model::error::ImportResource;
use orange_model::{CodecError, GraphDocument, from_json, to_json};
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
        }) if identity.reveal() == "core/i64@1"
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

fn function(nodes: Value, edges: Value) -> Value {
    json!({"id": "50000000-0000-4000-8000-000000000001", "name": "f", "kind": "pure", "nodes": nodes, "edges": edges})
}

#[test]
fn node_quota_rejects_before_the_body_is_built() {
    // The last node is malformed: building the body would fail on it, so the quota error
    // shows the count was checked before any node was materialized.
    let mut nodes: Vec<Value> = (1..=200).map(branch_node).collect();
    nodes.push(json!({}));
    let text = minimal_document(json!({"nodes": nodes}));
    assert!(matches!(
        from_json_with_limits(&text, ImportLimits::new(text.len(), 1, 0)),
        Err(CodecError::ImportLimitExceeded {
            resource: ImportResource::Nodes,
            max: 1,
            actual: 201,
        })
    ));
}

#[test]
fn quotas_count_function_bodies() {
    let text = minimal_document(json!({
        "nodes": [branch_node(1)],
        "functions": [function(json!([branch_node(2), branch_node(3)]), json!([common::edge(1, 2, 3)]))],
        // Look-alike keys outside graph bodies are not counted.
        "editor": {"nodes": [1, 2, 3], "edges": [1]},
        "future": {"functions": [{"nodes": [1], "edges": [1]}]}
    }));
    let limits = |nodes, edges| ImportLimits::new(text.len(), nodes, edges);
    assert!(from_json_with_limits(&text, limits(3, 1)).is_ok());
    assert!(matches!(
        from_json_with_limits(&text, limits(2, 1)),
        Err(CodecError::ImportLimitExceeded {
            resource: ImportResource::Nodes,
            max: 2,
            actual: 3,
        })
    ));
    assert!(matches!(
        from_json_with_limits(&text, limits(3, 0)),
        Err(CodecError::ImportLimitExceeded {
            resource: ImportResource::Edges,
            max: 0,
            actual: 1,
        })
    ));
}

#[test]
fn newer_version_is_reported_before_quotas() {
    let text = minimal_document(json!({"format_version": 2, "nodes": [branch_node(1)]}));
    assert!(matches!(
        from_json_with_limits(&text, ImportLimits::new(text.len(), 0, 0)),
        Err(CodecError::UnsupportedVersion { found: 2 })
    ));
}

fn json_profile(
    text: &str,
    values: usize,
    keys: usize,
    strings: usize,
) -> Result<GraphDocument, CodecError> {
    from_json_with_budget(
        text,
        ImportLimits::new(text.len(), 100_000, 500_000),
        ImportBudget::new(values, keys, strings),
    )
}

fn json_totals(value: &Value) -> (usize, usize, usize) {
    match value {
        Value::Object(map) => {
            let (mut count, mut keys, mut strings) = (1, 0, 0);
            for (key, value) in map {
                keys += key.len();
                let (v, k, s) = json_totals(value);
                count += v;
                keys += k;
                strings += s;
            }
            (count, keys, strings)
        }
        Value::Array(items) => {
            let mut result = (1, 0, 0);
            for item in items {
                let (v, k, s) = json_totals(item);
                result.0 += v;
                result.1 += k;
                result.2 += s;
            }
            result
        }
        Value::String(value) => (1, 0, value.len()),
        _ => (1, 0, 0),
    }
}

#[test]
fn json_budgets_count_root_containers_primitives_keys_and_string_values() {
    let text = minimal_document(json!({
        "future": {"items": [null, true, 9, "héllo", {"nested": []}]},
        "editor": {"extension": [false, "editor-value"]}
    }));
    let parsed: Value = serde_json::from_str(&text).unwrap();
    let (values, keys, strings) = json_totals(&parsed);
    let within = json_profile(&text, values, keys, strings);
    assert!(
        within.is_ok(),
        "values={values}, keys={keys}, strings={strings}, error={within:?}"
    );

    for (max, resource, expected) in [
        (values - 1, ImportResource::JsonValues, values),
        (keys - 1, ImportResource::JsonKeyBytes, keys),
        (strings - 1, ImportResource::JsonStringBytes, strings),
    ] {
        let result = match resource {
            ImportResource::JsonValues => json_profile(&text, max, keys, strings),
            ImportResource::JsonKeyBytes => json_profile(&text, values, max, strings),
            ImportResource::JsonStringBytes => json_profile(&text, values, keys, max),
            _ => unreachable!(),
        };
        let error = result.as_ref().unwrap_err();
        assert!(!format!("{error:?}{error}").contains("héllo"));
        assert!(
            matches!(result, Err(CodecError::ImportLimitExceeded { resource: found, max: _, actual }) if found == resource && actual == expected)
        );
    }
}

#[test]
fn zero_value_budget_rejects_the_root_object() {
    assert!(matches!(
        from_json_with_budget(
            "{}",
            ImportLimits::new(2, 0, 0),
            ImportBudget::new(0, usize::MAX, usize::MAX)
        ),
        Err(CodecError::ImportLimitExceeded {
            resource: ImportResource::JsonValues,
            max: 0,
            actual: 1
        })
    ));
}

#[test]
fn json_budget_stops_before_a_malformed_later_payload() {
    let text = r#"{"format":"orange.graph","format_version":1,"future":["never-read""#;
    assert!(matches!(
        from_json_with_budget(
            text,
            ImportLimits::new(text.len(), 0, 0),
            ImportBudget::new(3, usize::MAX, usize::MAX)
        ),
        Err(CodecError::ImportLimitExceeded {
            resource: ImportResource::JsonValues,
            max: 3,
            actual: 4
        })
    ));
}

#[test]
fn decoded_escape_and_utf8_byte_counts_are_used() {
    let text = r#"{"format":"orange.graph","format_version":1,"graph_id":"a0f3822f-9cf8-4ab6-a2bf-e21fd345c3c6","revision":0,"package_lock":[],"type_lock":[],"variables":[],"events":[],"signatures":{},"nodes":[],"edges":[],"functions":[],"exports":[],"editor":{},"future":{"é":"\u00e9"}}"#;
    let parsed: Value = serde_json::from_str(text).unwrap();
    let (_, keys, strings) = json_totals(&parsed);
    assert!(json_profile(text, 100, keys, strings).is_ok());
    assert!(matches!(
        json_profile(text, 100, keys - 1, strings),
        Err(CodecError::ImportLimitExceeded {
            resource: ImportResource::JsonKeyBytes,
            ..
        })
    ));
    assert!(matches!(
        json_profile(text, 100, keys, strings - 1),
        Err(CodecError::ImportLimitExceeded {
            resource: ImportResource::JsonStringBytes,
            ..
        })
    ));
}

#[test]
fn json_budget_precedes_later_format_version_and_duplicate_errors() {
    let text = r#"{"format":"other","format_version":2,"nodes":[],"nodes":[],"future":[0,1,2]}"#;
    assert!(matches!(
        from_json_with_budget(
            text,
            ImportLimits::new(text.len(), 10, 10),
            ImportBudget::new(4, usize::MAX, usize::MAX)
        ),
        Err(CodecError::ImportLimitExceeded {
            resource: ImportResource::JsonValues,
            actual: 5,
            ..
        })
    ));
}

#[test]
fn default_million_value_budget_stops_wide_input_and_explicit_budget_can_raise_it() {
    let values = "0,".repeat(1_000_000) + "0";
    let text =
        format!("{{\"format\":\"orange.graph\",\"format_version\":1,\"future\":[{values}]}}");
    assert!(matches!(
        from_json(&text),
        Err(CodecError::ImportLimitExceeded {
            resource: ImportResource::JsonValues,
            max: 1_000_000,
            actual: 1_000_001
        })
    ));
    assert!(matches!(
        from_json_with_limits(&text, ImportLimits::new(text.len(), 0, 0)),
        Err(CodecError::ImportLimitExceeded {
            resource: ImportResource::JsonValues,
            max: 1_000_000,
            actual: 1_000_001
        })
    ));

    let moderate = minimal_document(json!({"future": [0, 1, 2, 3]}));
    assert!(matches!(
        json_profile(&moderate, 10, usize::MAX, usize::MAX),
        Err(CodecError::ImportLimitExceeded {
            resource: ImportResource::JsonValues,
            ..
        })
    ));
    assert!(
        from_json_with_budget(
            &moderate,
            ImportLimits::new(moderate.len(), 0, 0),
            ImportBudget::new(100, usize::MAX, usize::MAX)
        )
        .is_ok()
    );
}

/// Each document the importer refuses, with the message Serde surfaces for it.
fn refused_documents() -> Vec<(String, &'static str)> {
    let digest = format!("sha256:{}", "a".repeat(64));
    let lock = package("org.example.pkg", "1.0.0", &digest);
    let ty = json!({"type": "core/i64@1", "codec": "orange.value.v1"});
    vec![
        (
            minimal_document(json!({"format_version": 999})),
            "unsupported format_version 999",
        ),
        (
            minimal_document(json!({"format": "other.graph"})),
            "unsupported document format",
        ),
        (
            minimal_document(json!({
                "nodes": [branch_node(1)],
                "functions": [function(json!([branch_node(1)]), json!([]))]
            })),
            "duplicate node id",
        ),
        (
            minimal_document(json!({"package_lock": [lock.clone(), lock]})),
            "duplicate package lock identity",
        ),
        (
            minimal_document(json!({"type_lock": [ty.clone(), ty]})),
            "duplicate type lock identity",
        ),
    ]
}

#[test]
fn serde_construction_refuses_what_the_importer_refuses() {
    for (text, message) in refused_documents() {
        assert!(from_json(&text).is_err(), "importer accepted {text}");
        let value: Value = serde_json::from_str(&text).unwrap();
        for error in [
            serde_json::from_str::<GraphDocument>(&text).unwrap_err(),
            serde_json::from_value::<GraphDocument>(value).unwrap_err(),
        ] {
            assert!(error.to_string().contains(message), "{error} for {text}");
        }
    }
}

#[test]
fn serde_construction_matches_the_importer_on_valid_documents() {
    let text = minimal_document(json!({
        "nodes": [branch_node(1)],
        "functions": [function(json!([branch_node(2)]), json!([]))],
        "future": {"kept": true}
    }));
    let imported = from_json(&text).unwrap();
    // The envelope tag is accepted and dropped, as the importer does; unknown fields stay.
    let from_file: GraphDocument = serde_json::from_str(&text).unwrap();
    assert_eq!(from_file, imported);
    assert!(!from_file.extensions().contains_key("format"));
    assert_eq!(from_file.extensions()["future"], json!({"kept": true}));
    // Serde's own output has no envelope and reads back unchanged.
    let bare: GraphDocument =
        serde_json::from_value(serde_json::to_value(&imported).unwrap()).unwrap();
    assert_eq!(bare, imported);
    assert_eq!(to_json(&bare).unwrap(), to_json(&imported).unwrap());
}
