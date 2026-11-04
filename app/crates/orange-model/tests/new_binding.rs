//! New nodes bound to the registry's new-binding default (12 §8, ADR-07).

use std::sync::Arc;

use orange_model::binding::{NewNode, NewNodeError, new_node_edit};
use orange_model::catalog::{
    Determinism, NodeDefinition, Ports, ProviderDescriptor, ProviderKey, RegistryBuilder,
    RegistrySnapshot,
};
use orange_model::document::{DefinitionRef, NodeKind, TypeParam};
use orange_model::id::NodeId;
use orange_model::inference::apply_inferred;
use orange_model::validation::{HostOrigin, Severity};
use orange_model::{DocCommand, GraphDocument, TxError, validate};
use serde_json::json;

const NODE: &str = "10000000-0000-4000-8000-000000000001";
const DIGEST: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const I64: &str = "core/i64@1";

fn dref(id: &str) -> DefinitionRef {
    DefinitionRef {
        id: id.parse().unwrap(),
        version: 1,
    }
}

fn node_id() -> NodeId {
    NODE.parse().unwrap()
}

fn definition(id: &str, generic: bool) -> NodeDefinition {
    let ty = if generic { "$T" } else { I64 };
    NodeDefinition {
        definition: dref(id),
        config_version: 3,
        kind: NodeKind::Pure,
        volatility: None,
        determinism: Determinism::Deterministic,
        suspends: None,
        type_params: if generic {
            vec![serde_json::from_value(json!({"name": "T"})).unwrap()]
        } else {
            Vec::<TypeParam>::new()
        },
        ports: Ports::Declared(vec![
            serde_json::from_value(json!({
                "port_id": "a", "direction": "input", "kind": "data", "type": ty
            }))
            .unwrap(),
            serde_json::from_value(json!({
                "port_id": "out", "direction": "output", "kind": "data", "type": ty,
                "source": "computed", "label": {"key": "example/out", "fallback": "Out"}
            }))
            .unwrap(),
        ]),
        references: vec![],
        specializations: None,
        config_schema: None,
    }
}

fn key(id: &str) -> ProviderKey {
    ProviderKey {
        package: "example.pkg".into(),
        version: "1.0.0".into(),
        content_digest: DIGEST.into(),
        id: id.into(),
    }
}

fn registry() -> Arc<RegistrySnapshot> {
    let implements = |id: &str| ProviderDescriptor {
        key: key(id),
        implements: [(dref("example/plain"), 3), (dref("example/echo"), 3)].into(),
        specializations: Default::default(),
    };
    RegistryBuilder::core(HostOrigin::new("0.1.0", format!("sha256:{}", "c".repeat(64))).unwrap())
        .definition(definition("example/plain", false))
        .definition(definition("example/echo", true))
        .provider(implements("first"))
        .provider(implements("second"))
        .new_binding_default(dref("example/plain"), key("second"))
        .new_binding_default(dref("example/echo"), key("first"))
        .build()
        .unwrap()
}

fn empty() -> GraphDocument {
    GraphDocument::new("20000000-0000-4000-8000-000000000001".parse().unwrap())
}

fn placed(definition: &str) -> NewNode {
    let mut node = NewNode::new(node_id(), dref(definition));
    node.inputs.insert(
        "a".parse().unwrap(),
        serde_json::from_value(json!({"type": I64, "value": "5"})).unwrap(),
    );
    node
}

fn errors(document: &GraphDocument, registry: &RegistrySnapshot) -> Vec<String> {
    validate(document, registry)
        .diagnostics
        .into_iter()
        .filter(|item| item.severity == Severity::Error)
        .map(|item| format!("{:?}: {}", item.code, item.message))
        .collect()
}

#[test]
fn a_new_node_binds_the_default_and_analyses_clean() {
    let registry = registry();
    let mut document = empty();
    let edit = new_node_edit(&document, &registry, placed("example/plain")).unwrap();
    assert_eq!(edit.provider, key("second"));
    assert!(matches!(
        edit.commands.as_slice(),
        [
            DocCommand::PutPackageLock(_),
            DocCommand::PutSignature { .. },
            DocCommand::AddNode { .. }
        ]
    ));
    let revision = document.revision();
    let _ = document.apply(edit.commands).unwrap();
    assert_eq!(
        document.revision(),
        revision + 1,
        "one undoable transaction"
    );

    let node = document.node(node_id()).unwrap();
    let binding = node.provider.as_ref().unwrap();
    assert_eq!(
        ProviderKey::of(&document, binding),
        Some(key("second")),
        "binding and lock name the default exactly"
    );
    assert_eq!(node.config_version, 3);
    let snapshot = &document.signatures()[node.signature.as_ref().unwrap()];
    assert!(
        snapshot.ports[1].extensions.is_empty(),
        "presentation is projected out"
    );
    assert_eq!(errors(&document, &registry), Vec::<String>::new());
}

#[test]
fn pins_specialize_the_signature_and_inference_commits_with_the_edit() {
    let registry = registry();
    let mut document = empty();
    let mut node = placed("example/echo");
    node.type_pins.insert("T".into(), I64.parse().unwrap());
    let edit = new_node_edit(&document, &registry, node).unwrap();
    assert_eq!(edit.provider, key("first"));
    let DocCommand::PutSignature { snapshot, .. } = &edit.commands[1] else {
        panic!("the second command puts the signature")
    };
    assert_eq!(snapshot.ports[0].ty, Some(I64.parse().unwrap()));

    let committed = apply_inferred(&mut document, edit.commands, &registry).unwrap();
    assert_eq!(committed.issues, []);
    let node = document.node(node_id()).unwrap();
    assert_eq!(node.type_args.get("T"), Some(&I64.parse().unwrap()));
    assert_eq!(errors(&document, &registry), Vec::<String>::new());
}

#[test]
fn no_default_unknown_definitions_and_lock_conflicts_are_refused() {
    let registry = registry();
    let document = empty();
    // Installing a provider selects nothing; only an explicit default binds new nodes.
    let unselected = RegistryBuilder::from_snapshot(&registry)
        .definition(definition("example/fresh", false))
        .build()
        .unwrap();
    assert_eq!(
        new_node_edit(&document, &unselected, placed("example/fresh")).unwrap_err(),
        NewNodeError::NoDefault(dref("example/fresh"))
    );
    assert_eq!(
        new_node_edit(&document, &registry, placed("example/missing")).unwrap_err(),
        NewNodeError::UnknownDefinition(dref("example/missing"))
    );

    // The document already locks the package version to another digest.
    let mut locked = empty();
    let _ = locked
        .apply([DocCommand::PutPackageLock(
            serde_json::from_value(json!({
                "id": "example.pkg", "version": "1.0.0",
                "content_digest": format!("sha256:{}", "e".repeat(64))
            }))
            .unwrap(),
        )])
        .unwrap();
    let edit = new_node_edit(&locked, &registry, placed("example/plain")).unwrap();
    let before = locked.clone();
    assert!(matches!(
        locked.apply(edit.commands).unwrap_err(),
        TxError::PackageLockConflict { .. }
    ));
    assert_eq!(locked, before);
}
