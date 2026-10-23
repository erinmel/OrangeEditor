use orange_model::document::{DefinitionRef, NodeKind, ResolvedPort, SignatureSnapshot};
use orange_model::inference::{GenericDefinition, ProviderKey, TemplateCatalog};
use orange_model::types::TypeRegistry;
use orange_model::validation::{DiagnosticCode, HostOrigin, ProviderCatalog, validate_nodes};
use orange_model::{DocCommand, GraphDocument};
use serde_json::{Map, Value, json};

const NODE_A: &str = "10000000-0000-4000-8000-000000000001";
const NODE_B: &str = "10000000-0000-4000-8000-000000000002";
const DIGEST: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn origin() -> HostOrigin {
    HostOrigin::new("0.1.0", format!("sha256:{}", "c".repeat(64))).unwrap()
}

fn definition() -> DefinitionRef {
    DefinitionRef {
        id: "example/value".parse().unwrap(),
        version: 1,
    }
}

fn provider() -> ProviderKey {
    ProviderKey {
        package: "example.pkg".into(),
        version: "1.0.0".into(),
        content_digest: DIGEST.into(),
        id: "value-provider".into(),
    }
}

fn snapshot(ty: &str) -> SignatureSnapshot {
    SignatureSnapshot {
        kind: NodeKind::Pure,
        suspends: None,
        ports: vec![
            serde_json::from_value::<ResolvedPort>(json!({
                "port_id":"value",
                "direction":"output",
                "kind":"data",
                "type":ty,
                "source":"computed"
            }))
            .unwrap(),
        ],
        extensions: Default::default(),
    }
}

fn node(id: &str, config: Value, signature: &SignatureSnapshot) -> Value {
    json!({
        "id":id,
        "definition_id":"example/value",
        "definition_version":1,
        "config_version":1,
        "config":config,
        "signature":orange_model::signature::semantic_key(signature).to_string(),
        "provider":{"package":"example.pkg","version":"1.0.0","id":"value-provider"}
    })
}

fn document(nodes: Vec<Value>, snapshots: &[SignatureSnapshot]) -> GraphDocument {
    let mut signatures = Map::new();
    for snapshot in snapshots {
        signatures.insert(
            orange_model::signature::semantic_key(snapshot).to_string(),
            serde_json::to_value(snapshot).unwrap(),
        );
    }
    serde_json::from_value(json!({
        "format_version":1,
        "graph_id":"20000000-0000-4000-8000-000000000001",
        "revision":0,
        "package_lock":[{
            "id":"example.pkg",
            "version":"1.0.0",
            "content_digest":DIGEST
        }],
        "nodes":nodes,
        "edges":[],
        "signatures":signatures
    }))
    .unwrap()
}

fn contract(snapshot: &SignatureSnapshot) -> GenericDefinition {
    GenericDefinition {
        kind: snapshot.kind,
        suspends: snapshot.suspends,
        type_params: vec![],
        ports: snapshot.ports.clone(),
    }
}

fn providers() -> ProviderCatalog {
    let mut providers = ProviderCatalog::default();
    providers.insert(definition(), 1, provider());
    providers
}

#[test]
fn same_definition_nodes_keep_distinct_contextual_contracts() {
    let a = snapshot("core/i64@1");
    let b = snapshot("core/string@1");
    let document = document(
        vec![
            node(NODE_A, json!({"context":"a"}), &a),
            node(NODE_B, json!({"context":"b"}), &b),
        ],
        &[a.clone(), b.clone()],
    );
    let mut templates = TemplateCatalog::default();
    templates
        .insert_node(
            &document,
            document.node(NODE_A.parse().unwrap()).unwrap(),
            contract(&a),
        )
        .unwrap();
    templates
        .insert_node(
            &document,
            document.node(NODE_B.parse().unwrap()).unwrap(),
            contract(&b),
        )
        .unwrap();

    let report = validate_nodes(
        &document,
        &templates,
        &TypeRegistry::core(),
        &providers(),
        &origin(),
    );
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert_eq!(
        report.resolved[&NODE_A.parse().unwrap()].ports[0]
            .ty
            .as_ref()
            .unwrap()
            .as_str(),
        "core/i64@1"
    );
    assert_eq!(
        report.resolved[&NODE_B.parse().unwrap()].ports[0]
            .ty
            .as_ref()
            .unwrap()
            .as_str(),
        "core/string@1"
    );
}

#[test]
fn config_edit_after_overlay_capture_is_metadata_stale_and_unresolved() {
    let snapshot = snapshot("core/i64@1");
    let mut document = document(
        vec![node(NODE_A, json!({"mode":"before"}), &snapshot)],
        std::slice::from_ref(&snapshot),
    );
    let mut templates = TemplateCatalog::default();
    templates
        .insert_node(
            &document,
            document.node(NODE_A.parse().unwrap()).unwrap(),
            contract(&snapshot),
        )
        .unwrap();
    let _ = document
        .apply([DocCommand::SetConfig {
            node: NODE_A.parse().unwrap(),
            config_version: 1,
            config: serde_json::from_value(json!({"mode":"after"})).unwrap(),
        }])
        .unwrap();

    let report = validate_nodes(
        &document,
        &templates,
        &TypeRegistry::core(),
        &providers(),
        &origin(),
    );
    let id = NODE_A.parse().unwrap();
    assert_eq!(
        report
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<Vec<_>>(),
        vec![DiagnosticCode::MetadataStale]
    );
    assert!(report.incomplete_nodes.contains(&id));
    assert!(!report.resolved.contains_key(&id));
}
