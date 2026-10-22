//! Per-node template overlays stay tied to the exact node and definition version.

mod common;

use common::generic::{add, catalog as generic_catalog, data_edge, document};
use common::node_id;
use orange_model::DocIndex;
use orange_model::document::{DefinitionRef, NodeInstance, NodeKind, ResolvedPort, TypeParam};
use orange_model::id::NodeId;
use orange_model::inference::{GenericDefinition, TemplateCatalog, infer_components};
use orange_model::types::TypeRegistry;
use serde_json::json;

fn id(n: u8) -> NodeId {
    node_id(n).parse().unwrap()
}

fn contract(ty: &str) -> GenericDefinition {
    GenericDefinition {
        kind: NodeKind::Pure,
        suspends: None,
        type_params: vec![],
        ports: vec![
            serde_json::from_value::<ResolvedPort>(json!({
                "port_id":"value","direction":"output","kind":"data","type":ty
            }))
            .unwrap(),
        ],
    }
}

fn definition(version: u32) -> DefinitionRef {
    DefinitionRef {
        id: "org.test/context-source".parse().unwrap(),
        version,
    }
}

fn catalog() -> TemplateCatalog {
    let mut catalog = generic_catalog(None);
    let fallback = GenericDefinition {
        kind: NodeKind::Pure,
        suspends: None,
        type_params: vec![TypeParam {
            name: "T".into(),
            constraints: vec![],
            extensions: Default::default(),
        }],
        ports: vec![
            serde_json::from_value(json!({
                "port_id":"value","direction":"output","kind":"data","type":"$T"
            }))
            .unwrap(),
        ],
    };
    catalog.insert(definition(1), fallback.clone());
    catalog.insert(definition(2), fallback);
    catalog
}

fn stale_catalog() -> TemplateCatalog {
    let mut catalog = TemplateCatalog::default();
    catalog.insert(
        definition(1),
        GenericDefinition {
            kind: NodeKind::Pure,
            suspends: None,
            type_params: vec![TypeParam {
                name: "T".into(),
                constraints: vec![],
                extensions: Default::default(),
            }],
            ports: vec![
                serde_json::from_value(json!({
                    "port_id":"value","direction":"output","kind":"data","type":"$T"
                }))
                .unwrap(),
            ],
        },
    );
    catalog
}

fn context_source(n: u8) -> serde_json::Value {
    json!({
        "id":node_id(n),
        "definition_id":"org.test/context-source",
        "definition_version":1,
        "config_version":1,
        "config":{}
    })
}

fn context_node(document: &orange_model::GraphDocument, n: u8) -> &NodeInstance {
    document.node(id(n)).unwrap()
}

#[test]
fn same_definition_nodes_use_independent_concrete_contracts_for_inference() {
    let doc = document(
        vec![
            context_source(1),
            add(2, json!({})),
            context_source(3),
            add(4, json!({})),
        ],
        vec![
            data_edge(1, (1, "value"), (2, "a")),
            data_edge(2, (3, "value"), (4, "a")),
        ],
    );
    let mut templates = catalog();
    templates.insert_node(id(1), definition(1), contract("core/i64@1"));
    let before_insert = templates.generation();
    templates.insert_node(id(3), definition(1), contract("core/i32@1"));
    assert_ne!(templates.generation(), before_insert);

    let inferred = infer_components(
        &doc,
        &DocIndex::build(&doc),
        [id(1), id(3)],
        &templates,
        &TypeRegistry::core(),
    );
    assert_eq!(inferred.len(), 2);
    assert_eq!(inferred[0].type_args[&id(2)]["T"].as_str(), "core/i64@1");
    assert_eq!(inferred[1].type_args[&id(4)]["T"].as_str(), "core/i32@1");
    assert!(inferred.iter().all(|component| component.issues.is_empty()));
}

#[test]
fn overlay_requires_exact_definition_version_and_otherwise_falls_back() {
    let doc = document(vec![context_source(1)], vec![]);
    let mut templates = catalog();
    templates.insert_node(id(1), definition(1), contract("core/i64@1"));

    assert_eq!(
        templates.get_for_node(context_node(&doc, 1)).unwrap().ports[0]
            .ty
            .as_ref()
            .unwrap()
            .as_str(),
        "core/i64@1"
    );

    let mut changed_version = context_node(&doc, 1).clone();
    changed_version.definition.version = 2;
    assert_eq!(
        templates.get_for_node(&changed_version).unwrap().ports[0]
            .ty
            .as_ref()
            .unwrap()
            .as_str(),
        "$T",
        "mismatched overlay must fall back to the global exact-version catalog"
    );

    let mut rebound = context_node(&doc, 1).clone();
    rebound.definition.id = "org.test/other-source".parse().unwrap();
    assert!(templates.get_for_node(&rebound).is_none());

    let mut no_fallback = stale_catalog();
    no_fallback.insert_node(id(1), definition(1), contract("core/i64@1"));
    assert!(no_fallback.get_for_node(&changed_version).is_none());
}
