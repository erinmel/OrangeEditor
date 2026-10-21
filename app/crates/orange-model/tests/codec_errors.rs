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
