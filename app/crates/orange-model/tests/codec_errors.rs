//! Structurally invalid documents fail to load with a specific error.

mod common;

use common::{GRAPH_ID, branch_node, edge, minimal_document, node_id};
use orange_model::{CodecError, from_json};
use serde_json::json;

/// Asserts a parse-level failure whose message mentions `needle`.
fn assert_json_error(source: &str, needle: &str) {
    match from_json(source) {
        Err(CodecError::Json(error)) => {
            assert!(
                error.to_string().contains(needle),
                "expected `{needle}` in: {error}"
            );
        }
        other => panic!("expected a JSON error mentioning `{needle}`, got {other:?}"),
    }
}

#[test]
fn wrong_format_tag() {
    let source = json!({"format": "orange.type", "format_version": 1}).to_string();
    assert!(
        matches!(from_json(&source), Err(CodecError::UnsupportedFormat { found }) if found == "orange.type")
    );
}

#[test]
fn newer_version_is_reported_before_shape_errors() {
    // The rest of the body is not v1-shaped; the version must still be what is reported.
    let source = json!({"format": "orange.graph", "format_version": 2, "graph": {}}).to_string();
    assert!(matches!(
        from_json(&source),
        Err(CodecError::UnsupportedVersion { found: 2 })
    ));
}

#[test]
fn missing_required_field() {
    let source =
        json!({"format": "orange.graph", "format_version": 1, "graph_id": GRAPH_ID}).to_string();
    assert_json_error(&source, "revision");
}

#[test]
fn duplicate_ids_within_a_body() {
    assert_json_error(
        &minimal_document(json!({"nodes": [branch_node(1), branch_node(1)]})),
        "duplicate id",
    );
    assert_json_error(
        &minimal_document(json!({"edges": [edge(1, 1, 2), edge(1, 2, 3)]})),
        "duplicate id",
    );
}

#[test]
fn duplicate_ids_across_bodies() {
    let function = |nodes, edges| json!({"id": "50000000-0000-4000-8000-000000000001", "name": "f", "kind": "pure", "nodes": nodes, "edges": edges});
    let source = minimal_document(
        json!({"nodes": [branch_node(1)], "functions": [function(json!([branch_node(1)]), json!([]))]}),
    );
    assert!(
        matches!(from_json(&source), Err(CodecError::DuplicateId { kind: "node", id }) if id == node_id(1))
    );

    let source = minimal_document(
        json!({"edges": [edge(1, 1, 2)], "functions": [function(json!([]), json!([edge(1, 3, 4)]))]}),
    );
    assert!(matches!(
        from_json(&source),
        Err(CodecError::DuplicateId { kind: "edge", .. })
    ));
}

#[test]
fn invalid_identifiers() {
    let mut node = branch_node(1);
    node["id"] = json!("n1");
    assert_json_error(
        &minimal_document(json!({"nodes": [node]})),
        "canonical lowercase hyphenated UUID",
    );

    let mut bad_port = edge(1, 1, 2);
    bad_port["to"]["port"] = json!("item#01");
    assert_json_error(
        &minimal_document(json!({"edges": [bad_port]})),
        "not a port key",
    );

    let mut bad_definition = branch_node(1);
    bad_definition["definition_id"] = json!("branch");
    assert_json_error(
        &minimal_document(json!({"nodes": [bad_definition]})),
        "not a definition id",
    );
}

#[test]
fn literal_with_value_and_bits() {
    let mut node = branch_node(1);
    node["inputs"] =
        json!({"condition": {"type": "core/f64@1", "value": "1", "bits": "3ff0000000000000"}});
    assert_json_error(
        &minimal_document(json!({"nodes": [node]})),
        "exactly one of `value` or `bits`",
    );
}

#[test]
fn unknown_enum_token() {
    let variable = json!({"id": "30000000-0000-4000-8000-000000000001", "name": "x", "type": "core/i64@1", "scope": "global"});
    assert_json_error(
        &minimal_document(json!({"variables": [variable]})),
        "unknown variant `global`",
    );
}

#[test]
fn duplicate_object_keys_are_rejected_at_any_depth() {
    let duplicate = |source: &str| match from_json(source) {
        Err(CodecError::DuplicateKey { key }) => key,
        other => panic!("expected a duplicate key error, got {other:?}"),
    };
    let base = minimal_document(json!({"nodes": [branch_node(1)]}));
    assert!(from_json(&base).is_ok());
    // Top level, a node's config object and the signatures map; serde kept the last value.
    let top = base.replacen("\"revision\":0", "\"revision\":0,\"revision\":1", 1);
    assert_eq!(duplicate(&top), "revision");
    let config = base.replacen("\"config\":{}", "\"config\":{\"x\":1,\"x\":2}", 1);
    assert_eq!(duplicate(&config), "x");
    let signatures = base.replacen("\"signatures\":{}", "\"signatures\":{\"k\":{},\"k\":{}}", 1);
    assert_eq!(duplicate(&signatures), "k");
    // Keys compare after unescaping; the same key in sibling objects is fine.
    let escaped = base.replacen("\"config\":{}", r#""config":{"a":1,"a":2}"#, 1);
    assert_eq!(duplicate(&escaped), "a");
    let siblings = base.replacen(
        "\"config\":{}",
        "\"config\":{\"x\":{\"y\":1},\"z\":{\"y\":1}}",
        1,
    );
    assert!(!matches!(
        from_json(&siblings),
        Err(CodecError::DuplicateKey { .. })
    ));
    // Objects past the short-list size switch to a hash set without losing keys.
    let wide: Vec<String> = (0..20).map(|n| format!("\"k{n}\":{n}")).collect();
    let config = |keys: &[String]| format!("\"config\":{{{}}}", keys.join(","));
    let unique = base.replacen("\"config\":{}", &config(&wide), 1);
    assert!(from_json(&unique).is_ok());
    let repeated = [wide.clone(), vec!["\"k3\":0".to_owned()]].concat();
    assert_eq!(
        duplicate(&base.replacen("\"config\":{}", &config(&repeated), 1)),
        "k3"
    );
    // The first repeat in document order is reported, cut to 64 characters.
    let long = "k".repeat(100);
    let first = base.replacen(
        "\"config\":{}",
        &format!("\"config\":{{\"{long}\":1,\"{long}\":2,\"b\":1,\"b\":2}}"),
        1,
    );
    assert_eq!(duplicate(&first), "k".repeat(64));
}

#[test]
fn newer_version_is_reported_before_duplicate_keys() {
    let source = r#"{"format":"orange.graph","format_version":2,"a":1,"a":2}"#;
    assert!(matches!(
        from_json(source),
        Err(CodecError::UnsupportedVersion { found: 2 })
    ));
}
