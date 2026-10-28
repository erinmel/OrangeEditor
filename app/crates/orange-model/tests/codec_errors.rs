//! Structurally invalid documents fail to load with a specific error.

mod common;

use common::{GRAPH_ID, branch_node, edge, minimal_document, node_id};
use orange_model::{CodecError, from_json};
use serde_json::json;

/// Asserts a parse-level failure whose message mentions `needle`.
fn assert_json_error(source: &str, needle: &str) {
    match from_json(source) {
        Err(CodecError::Json(error)) => {
            // The parser detail is only reachable through the explicit reveal path.
            assert!(
                error.detail.reveal().contains(needle),
                "expected `{needle}` in: {}",
                error.detail.reveal()
            );
        }
        other => panic!("expected a JSON error mentioning `{needle}`, got {other:?}"),
    }
}

#[test]
fn wrong_format_tag() {
    let source = json!({"format": "orange.type", "format_version": 1}).to_string();
    assert!(
        matches!(from_json(&source), Err(CodecError::UnsupportedFormat { found }) if found.reveal() == "orange.type")
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
        Err(CodecError::DuplicateKey { key }) => key.reveal().to_owned(),
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

/// Rejected input reaches neither `Display` nor `Debug`: an imported field may hold
/// anything, a credential pasted by mistake included. Positions stay available.
#[test]
fn rejected_input_never_reaches_error_formatting() {
    const SENTINEL: &str = "SENTINEL-7c1e\u{7}\n";
    let base = minimal_document(json!({"nodes": [branch_node(1)]}));
    let lock = json!({"id": SENTINEL, "version": "1", "content_digest": format!("sha256:{}", "a".repeat(64))});
    let key = serde_json::to_string(SENTINEL).unwrap();
    let cases = [
        minimal_document(json!({"graph_id": SENTINEL})),
        json!({"format": SENTINEL, "format_version": 1}).to_string(),
        base.replacen(
            "\"config\":{}",
            &format!("\"config\":{{{key}:1,{key}:2}}"),
            1,
        ),
        minimal_document(json!({"package_lock": [lock.clone(), lock]})),
        minimal_document(
            json!({"nodes": [{"id": node_id(1), "definition_id": SENTINEL,
            "definition_version": 1, "config_version": 1, "config": {}}]}),
        ),
        minimal_document(
            json!({"variables": [{"id": "30000000-0000-4000-8000-000000000001",
            "name": "x", "type": SENTINEL, "scope": "run"}]}),
        ),
        minimal_document(
            json!({"variables": [{"id": "30000000-0000-4000-8000-000000000001",
            "name": "x", "type": "core/i64@1", "scope": SENTINEL}]}),
        ),
        minimal_document(json!({"nodes": [branch_node(1), branch_node(2)],
            "edges": [{"id": common::edge_id(1), "from": {"node": node_id(1), "port": SENTINEL},
                "to": {"node": node_id(2), "port": "in"}}]})),
    ];
    for text in cases {
        let error = from_json(&text).unwrap_err();
        for shown in [error.to_string(), format!("{error:?}")] {
            assert!(!shown.contains("SENTINEL"), "{shown}");
            assert!(!shown.chars().any(char::is_control), "{shown:?}");
        }
        assert!(error.message_key().starts_with("core/import-"));
        if let CodecError::Json(json) = &error {
            assert!(json.line > 0 && json.column > 0, "{json}");
            assert!(!json.detail.reveal().is_empty());
        }
    }
}
