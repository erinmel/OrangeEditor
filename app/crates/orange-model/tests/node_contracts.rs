//! Per-node template overlays stay tied to the exact node and definition version.

mod common;

use common::generic::{add, catalog as generic_catalog, data_edge, document};
use common::node_id;
use orange_model::DocIndex;
use orange_model::document::{DefinitionRef, NodeInstance, NodeKind, PortDescriptor, TypeParam};
use orange_model::id::NodeId;
use orange_model::inference::{GenericDefinition, ProviderKey, TemplateCatalog, infer_components};
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
            serde_json::from_value::<PortDescriptor>(json!({
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
    templates
        .insert_node(&doc, context_node(&doc, 1), contract("core/i64@1"))
        .unwrap();
    let before_insert = templates.generation();
    templates
        .insert_node(&doc, context_node(&doc, 3), contract("core/i32@1"))
        .unwrap();
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
fn stale_overlay_fails_closed_while_unoverlaid_node_uses_global_catalog() {
    let doc = document(vec![context_source(1)], vec![]);
    let mut templates = catalog();
    templates
        .insert_node(&doc, context_node(&doc, 1), contract("core/i64@1"))
        .unwrap();

    assert_eq!(
        templates
            .get_for_node(&doc, context_node(&doc, 1))
            .unwrap()
            .unwrap()
            .ports[0]
            .ty
            .as_ref()
            .unwrap()
            .as_str(),
        "core/i64@1"
    );

    let mut changed_version = context_node(&doc, 1).clone();
    changed_version.definition.version = 2;
    assert!(templates.get_for_node(&doc, &changed_version).is_err());
    assert_eq!(
        templates
            .get_for_node(&doc, context_node(&doc, 1))
            .unwrap()
            .unwrap()
            .ports[0]
            .ty
            .as_ref()
            .unwrap()
            .as_str(),
        "core/i64@1"
    );

    let mut rebound = context_node(&doc, 1).clone();
    rebound.definition.id = "org.test/other-source".parse().unwrap();
    assert!(templates.get_for_node(&doc, &rebound).is_err());

    let unoverlaid = context_source(2);
    let doc_with_unoverlaid = document(vec![context_source(1), unoverlaid], vec![]);
    let unoverlaid_node = doc_with_unoverlaid.node(id(2)).unwrap();
    assert_eq!(
        templates
            .get_for_node(&doc_with_unoverlaid, unoverlaid_node)
            .unwrap()
            .unwrap()
            .ports[0]
            .ty
            .as_ref()
            .unwrap()
            .as_str(),
        "$T"
    );

    let mut no_fallback = stale_catalog();
    no_fallback
        .insert_node(&doc, context_node(&doc, 1), contract("core/i64@1"))
        .unwrap();
    assert!(no_fallback.get_for_node(&doc, &changed_version).is_err());
}

#[test]
fn ordinary_connection_changes_preserve_overlay_but_node_metadata_changes_stale_it() {
    let nodes = vec![context_source(1), add(2, json!({}))];
    let before = document(nodes.clone(), vec![]);
    let mut templates = catalog();
    templates
        .insert_node(&before, context_node(&before, 1), contract("core/i64@1"))
        .unwrap();

    let connected = document(nodes, vec![data_edge(1, (1, "value"), (2, "a"))]);
    assert!(
        templates
            .get_for_node(&connected, connected.node(id(1)).unwrap())
            .unwrap()
            .is_some()
    );

    let mut changed_config = connected.node(id(1)).unwrap().clone();
    changed_config.config = serde_json::from_value(json!({"mode":"changed"})).unwrap();
    assert!(templates.get_for_node(&connected, &changed_config).is_err());

    let stale_copy = before.node(id(1)).unwrap().clone();
    let mut changed_node = context_source(1);
    changed_node["config"] = json!({"mode":"changed"});
    let changed_document = document(vec![changed_node], vec![]);
    assert!(
        templates
            .get_for_node(&changed_document, &stale_copy)
            .is_err()
    );

    let mut changed_config_version = connected.node(id(1)).unwrap().clone();
    changed_config_version.config_version += 1;
    assert!(
        templates
            .get_for_node(&connected, &changed_config_version)
            .is_err()
    );

    let removed = document(vec![], vec![]);
    assert!(templates.ensure_fresh(&removed).is_ok());
    assert!(
        templates
            .get_for_node(&removed, before.node(id(1)).unwrap())
            .is_err()
    );
}

#[test]
fn contextual_call_overlay_tracks_reachable_functions_not_unrelated_functions() {
    let call = json!({
        "id":node_id(1),"definition_id":"core/call","definition_version":1,
        "config_version":1,"config":{"function":"30000000-0000-4000-8000-000000000001"}
    });
    let function = |id: &str, kind: &str, nodes: serde_json::Value| {
        json!({
            "id":id,"name":"target","kind":kind,"nodes":nodes,"edges":[]
        })
    };
    let nested_call = json!({
        "id":node_id(2),"definition_id":"core/call","definition_version":1,
        "config_version":1,"config":{"function":"30000000-0000-4000-8000-000000000002"}
    });
    let make_document = |callee_kind: &str, unrelated_kind: &str| {
        serde_json::from_value::<orange_model::GraphDocument>(json!({
            "format_version":1,"graph_id":"a0f3822f-9cf8-4ab6-a2bf-e21fd345c3c6",
            "revision":0,"nodes":[call.clone()],"edges":[],"functions":[
                function("30000000-0000-4000-8000-000000000001", "pure", json!([nested_call.clone()])),
                function("30000000-0000-4000-8000-000000000002", callee_kind, json!([])),
                function("30000000-0000-4000-8000-000000000003", unrelated_kind, json!([]))
            ]
        })).unwrap()
    };
    let before = make_document("pure", "pure");
    let mut templates = TemplateCatalog::default();
    templates
        .insert_node(&before, before.node(id(1)).unwrap(), contract("core/i64@1"))
        .unwrap();

    let unrelated_changed = make_document("pure", "action");
    assert!(
        templates
            .get_for_node(&unrelated_changed, unrelated_changed.node(id(1)).unwrap())
            .unwrap()
            .is_some()
    );

    let callee_changed = make_document("action", "pure");
    assert!(
        templates
            .get_for_node(&callee_changed, callee_changed.node(id(1)).unwrap())
            .is_err()
    );
}

#[test]
fn package_digest_is_part_of_overlay_identity_and_duplicate_locks_fail_closed() {
    let provider_document = |digest: &str, duplicate: bool| {
        let mut node = context_source(1);
        node["provider"] = json!({
            "package":"org.test.provider","version":"1.0.0","id":"implementation"
        });
        let lock = json!({
            "id":"org.test.provider","version":"1.0.0",
            "content_digest":format!("sha256:{}", "a".repeat(64))
        });
        let mut locks = vec![lock];
        if duplicate {
            locks.push(json!({
                "id":"org.test.provider","version":"1.0.0","content_digest":digest
            }));
        } else {
            locks[0]["content_digest"] = json!(digest);
        }
        serde_json::from_value::<orange_model::GraphDocument>(json!({
            "format_version":1,"graph_id":"a0f3822f-9cf8-4ab6-a2bf-e21fd345c3c6",
            "revision":0,"package_lock":locks,"nodes":[node],"edges":[]
        }))
        .unwrap()
    };
    let digest_a = format!("sha256:{}", "a".repeat(64));
    let before = provider_document(&digest_a, false);
    let mut templates = TemplateCatalog::default();
    templates
        .insert_node(&before, before.node(id(1)).unwrap(), contract("core/i64@1"))
        .unwrap();

    let changed = provider_document(&format!("sha256:{}", "b".repeat(64)), false);
    assert!(
        templates
            .get_for_node(&changed, changed.node(id(1)).unwrap())
            .is_err()
    );

    let duplicate = provider_document(&format!("sha256:{}", "b".repeat(64)), true);
    let binding = duplicate.node(id(1)).unwrap().provider.as_ref().unwrap();
    assert!(ProviderKey::of(&duplicate, binding).is_none());
}
