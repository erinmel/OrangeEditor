//! Unknown keys and unresolved package nodes survive load→save (FR-05; structural basis
//! for G-29). The codec never needs the definition to keep a node.

mod common;

use common::{branch_node, minimal_document, node_id, parse_json};
use orange_model::{from_json, to_json};
use serde_json::json;

#[test]
fn unknown_keys_survive_at_every_level() {
    let mut node = branch_node(1);
    node["x_node_future"] = json!({"nested": [1, 2, 3]});
    let mut edge = common::edge(1, 1, 1);
    edge["x_edge_future"] = json!("kept");
    let source = minimal_document(json!({
        "x_doc_b": true,
        "x_doc_a": {"z": 1, "a": 2},
        "nodes": [node],
        "edges": [edge],
        "signatures": {"s1": {"kind": "pure", "x_sig": 1, "ports": [
            {"port_id": "a", "direction": "input", "kind": "data", "type": "core/i64@1", "optional": false, "label": {"key": "k", "fallback": "A"}}
        ]}}
    }));

    let document = from_json(&source).unwrap();
    let keys: Vec<&str> = document.extensions.keys().map(String::as_str).collect();
    assert_eq!(keys, ["x_doc_b", "x_doc_a"], "source order kept");

    let saved = to_json(&document).unwrap();
    assert_eq!(parse_json(&saved), parse_json(&source));
    // Nested object key order inside an unknown value is preserved too.
    assert!(saved.contains(r#""x_doc_a":{"z":1,"a":2}"#));
}

#[test]
fn unresolved_package_node_keeps_signature_snapshot() {
    let source = minimal_document(json!({
        "package_lock": [{"id": "org.example.gone", "version": "2.0.0", "content_digest": "sha256:00"}],
        "signatures": {"s-gone": {"kind": "action", "ports": [
            {"port_id": "value", "direction": "input", "kind": "data", "type": "org.example.gone/thing@2"}
        ]}},
        "nodes": [{
            "id": node_id(1), "definition_id": "org.example.gone/do-thing", "definition_version": 3,
            "config_version": 2, "config": {"mode": "fast"},
            "provider": {"package": "org.example.gone", "version": "2.0.0", "id": "worker"},
            "signature": "s-gone"
        }],
        "edges": [{"id": common::edge_id(1), "from": {"node": node_id(1), "port": "then"}, "to": {"node": node_id(9), "port": "missing_port"}}]
    }));

    let saved = to_json(&from_json(&source).unwrap()).unwrap();
    assert_eq!(
        parse_json(&saved),
        parse_json(&source),
        "orphan edge and unknown node kept"
    );
}
