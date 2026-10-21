//! Generic-node fixtures for the inference tests: `org.test/add<T: num>(a, b) -> sum` and a
//! non-generic `org.test/source` with an `i64` output `value`.

use orange_model::document::{DefinitionRef, NodeKind, ResolvedPort, TypeParam};
use orange_model::inference::{GenericDefinition, TemplateCatalog};
use orange_model::{GraphDocument, from_json};
use serde_json::{Value as Json, json};

use super::{edge_id, minimal_document, node_id};

pub const ADD: &str = "org.test/add";
const SOURCE: &str = "org.test/source";

pub fn port(port_id: &str, direction: &str, ty: &str) -> ResolvedPort {
    serde_json::from_value(
        json!({"port_id": port_id, "direction": direction, "kind": "data", "type": ty}),
    )
    .unwrap()
}

/// `org.test/add<T: num>(a: T, b: T) -> sum: T`, optionally limited to listed specializations.
pub fn catalog(specializations: Option<Vec<&str>>) -> TemplateCatalog {
    let mut catalog = TemplateCatalog::default();
    catalog.insert(
        DefinitionRef {
            id: ADD.parse().unwrap(),
            version: 1,
        },
        GenericDefinition {
            kind: NodeKind::Pure,
            type_params: vec![
                serde_json::from_value::<TypeParam>(json!({"name": "T", "constraints": ["num"]}))
                    .unwrap(),
            ],
            ports: vec![
                port("a", "input", "$T"),
                port("b", "input", "$T"),
                port("sum", "output", "$T"),
            ],
            specializations: specializations.map(|types| {
                types
                    .into_iter()
                    .map(|ty| [("T".to_owned(), ty.parse().unwrap())].into())
                    .collect()
            }),
        },
    );
    catalog
}

pub fn add(n: u8, extra: Json) -> Json {
    let mut node = json!({"id": node_id(n), "definition_id": ADD, "definition_version": 1,
        "config_version": 1, "config": {}});
    node.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    node
}

/// A non-generic node whose snapshot declares an `i64` output `value`.
pub fn source(n: u8) -> Json {
    json!({"id": node_id(n), "definition_id": SOURCE, "definition_version": 1,
        "config_version": 1, "config": {}, "signature": "s-source"})
}

pub fn data_edge(n: u8, from: (u8, &str), to: (u8, &str)) -> Json {
    json!({"id": edge_id(n), "from": {"node": node_id(from.0), "port": from.1},
        "to": {"node": node_id(to.0), "port": to.1}})
}

pub fn document(nodes: Vec<Json>, edges: Vec<Json>) -> GraphDocument {
    from_json(&minimal_document(json!({
        "signatures": signatures(),
        "nodes": nodes,
        "edges": edges,
    })))
    .unwrap()
}

/// A non-generic node with a single data port `value` of type `core/<ty>@1`: an output
/// (`typed_source`) or an input (`typed_sink`). Types must be one of `TYPED`.
pub const TYPED: [&str; 3] = ["i32", "i64", "f64"];

pub fn typed_source(n: u8, ty: &str) -> Json {
    json!({"id": node_id(n), "definition_id": format!("org.test/out-{ty}"), "definition_version": 1,
        "config_version": 1, "config": {}, "signature": format!("s-out-{ty}")})
}

pub fn typed_sink(n: u8, ty: &str, extra: Json) -> Json {
    let mut node = json!({"id": node_id(n), "definition_id": format!("org.test/in-{ty}"),
        "definition_version": 1, "config_version": 1, "config": {}, "signature": format!("s-in-{ty}")});
    node.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    node
}

fn typed_signatures() -> Json {
    let mut signatures = serde_json::Map::new();
    for ty in TYPED {
        for (key, direction) in [("out", "output"), ("in", "input")] {
            signatures.insert(
                format!("s-{key}-{ty}"),
                json!({"kind": "pure", "ports": [
                    {"port_id": "value", "direction": direction, "kind": "data",
                     "type": format!("core/{ty}@1")}
                ]}),
            );
        }
    }
    Json::Object(signatures)
}

fn signatures() -> Json {
    let mut signatures = typed_signatures();
    signatures["s-source"] = json!({"kind": "pure", "ports": [
        {"port_id": "value", "direction": "output", "kind": "data", "type": "core/i64@1"}
    ]});
    signatures
}
