//! Shared helpers for the integration tests.
#![allow(dead_code)] // each test binary uses a different subset

pub mod strategies;

use std::path::PathBuf;

use orange_model::document::{DefinitionRef, Edge, NodeInstance, PortRef};
use orange_model::id::{EdgeId, NodeId};
use orange_model::literal::Literal;
use orange_model::{GraphDocument, to_json};
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

// Typed builders for transaction tests.

pub fn nid(n: u8) -> NodeId {
    node_id(n).parse().unwrap()
}

pub fn eid(n: u8) -> EdgeId {
    edge_id(n).parse().unwrap()
}

/// An empty document with the shared test graph id.
pub fn empty_document() -> GraphDocument {
    GraphDocument::new(GRAPH_ID.parse().unwrap())
}

/// A `core/branch` node instance; definition semantics do not matter to the model.
pub fn node(n: u8) -> NodeInstance {
    NodeInstance {
        id: nid(n),
        definition: DefinitionRef {
            id: "core/branch".parse().unwrap(),
            version: 1,
        },
        config_version: 1,
        config: Default::default(),
        type_args: Default::default(),
        provider: None,
        signature: None,
        inputs: Default::default(),
        variadic: Default::default(),
        activation: None,
        extensions: Default::default(),
    }
}

pub fn port(n: u8, key: &str) -> PortRef {
    PortRef {
        node: nid(n),
        port: key.parse().unwrap(),
    }
}

pub fn typed_edge(n: u8, from: PortRef, to: PortRef) -> Edge {
    Edge {
        id: eid(n),
        from,
        to,
        extensions: Default::default(),
    }
}

pub fn i64_literal(value: i64) -> Literal {
    Literal::value("core/i64@1".parse().unwrap(), json!(value.to_string()))
}

pub fn saved(document: &GraphDocument) -> String {
    to_json(document).unwrap()
}

/// The saved document without `revision`, which every transaction (undo included) bumps.
pub fn content(document: &GraphDocument) -> Json {
    let mut value = parse_json(&saved(document));
    value.as_object_mut().unwrap().shift_remove("revision");
    value
}
