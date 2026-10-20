//! Shared helpers for the codec integration tests.
#![allow(dead_code)] // each test binary uses a different subset

use std::path::PathBuf;

use serde_json::{Value as Json, json};

pub const GRAPH_ID: &str = "a0f3822f-9cf8-4ab6-a2bf-e21fd345c3c6";

pub fn node_id(n: u8) -> String {
    format!("10000000-0000-4000-8000-{n:012}")
}

pub fn edge_id(n: u8) -> String {
    format!("20000000-0000-4000-8000-{n:012}")
}

pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

pub fn read_fixture(name: &str) -> String {
    std::fs::read_to_string(fixture_path(name))
        .unwrap_or_else(|e| panic!("read fixture {name}: {e}"))
}

/// Empty document with every collection present (the canonical saved form), with `extra`
/// keys merged over it. Comparing it with its re-saved form therefore needs no defaults.
pub fn minimal_document(extra: Json) -> String {
    let mut document = json!({
        "format": "orange.graph", "format_version": 1, "graph_id": GRAPH_ID, "revision": 0,
        "package_lock": [], "type_lock": [], "variables": [], "events": [], "signatures": {},
        "nodes": [], "edges": [], "functions": [], "exports": [], "editor": {}
    });
    document
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    document.to_string()
}

/// A `core/branch` node with the given id number.
pub fn branch_node(n: u8) -> Json {
    json!({"id": node_id(n), "definition_id": "core/branch", "definition_version": 1, "config_version": 1, "config": {}})
}

pub fn edge(n: u8, from: u8, to: u8) -> Json {
    json!({"id": edge_id(n), "from": {"node": node_id(from), "port": "true"}, "to": {"node": node_id(to), "port": "in"}})
}

pub fn parse_json(text: &str) -> Json {
    serde_json::from_str(text).expect("valid JSON")
}
