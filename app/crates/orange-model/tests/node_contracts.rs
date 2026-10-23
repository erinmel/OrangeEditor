//! Contextual node contracts are computed from the node's declarations, never cached.

mod common;

use common::generic::{add, core_converters, data_edge, registry_builder};
use common::{minimal_document, node_id};
use orange_model::catalog::ProviderKey;
use orange_model::id::NodeId;
use orange_model::inference::infer_components;
use orange_model::{DocIndex, GraphDocument, from_json};
use serde_json::json;

fn id(n: u8) -> NodeId {
    node_id(n).parse().unwrap()
}

fn get_variable(n: u8, variable: u8) -> serde_json::Value {
    json!({
        "id":node_id(n),"definition_id":"core/get-variable","definition_version":1,
        "config_version":1,"config":{"variable":node_id(variable)}
    })
}

fn variable(n: u8, ty: &str) -> serde_json::Value {
    json!({"id":node_id(n),"name":"value","type":ty,"scope":"run"})
}

#[test]
fn same_definition_nodes_use_independent_declared_contracts_for_inference() {
    let doc = from_json(&minimal_document(json!({
        "variables":[variable(10, "core/i64@1"), variable(11, "core/i32@1")],
        "nodes":[get_variable(1, 10), add(2, json!({})), get_variable(3, 11), add(4, json!({}))],
        "edges":[
            data_edge(1, (1, "value"), (2, "a")),
            data_edge(2, (3, "value"), (4, "a")),
        ],
    })))
    .unwrap();
    let registry = registry_builder(core_converters(), None).build().unwrap();

    let inferred = infer_components(&doc, &DocIndex::build(&doc), [id(1), id(3)], &registry);
    assert_eq!(inferred.len(), 2);
    assert_eq!(inferred[0].type_args[&id(2)]["T"].as_str(), "core/i64@1");
    assert_eq!(inferred[1].type_args[&id(4)]["T"].as_str(), "core/i32@1");
    assert!(inferred.iter().all(|component| component.issues.is_empty()));
}

#[test]
fn duplicate_package_locks_have_no_provider_key() {
    let lock = |digest: char| {
        json!({
            "id":"org.test.provider","version":"1.0.0",
            "content_digest":format!("sha256:{}", digest.to_string().repeat(64))
        })
    };
    let node = json!({
        "id":node_id(1),"definition_id":"org.test/context-source","definition_version":1,
        "config_version":1,"config":{},
        "provider":{"package":"org.test.provider","version":"1.0.0","id":"implementation"}
    });
    let document: GraphDocument = serde_json::from_value(json!({
        "format_version":1,"graph_id":"a0f3822f-9cf8-4ab6-a2bf-e21fd345c3c6","revision":0,
        "package_lock":[lock('a'), lock('b')],"nodes":[node],"edges":[]
    }))
    .unwrap();
    let binding = document.node(id(1)).unwrap().provider.as_ref().unwrap();
    assert!(ProviderKey::of(&document, binding).is_none());
}
