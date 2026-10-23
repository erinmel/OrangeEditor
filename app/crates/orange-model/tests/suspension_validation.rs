use std::collections::BTreeSet;
use std::sync::Arc;

use orange_model::GraphDocument;
use orange_model::catalog::{
    NodeDefinition, Ports, ProviderDescriptor, ProviderKey, RegistryBuilder, RegistryError,
};
use orange_model::document::{DefinitionRef, NodeKind, SignatureSnapshot, Suspends};
use orange_model::validation::{HostOrigin, validate_nodes};
use serde_json::json;

const NODE: &str = "10000000-0000-4000-8000-000000000001";
const DIGEST: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn origin() -> HostOrigin {
    HostOrigin::new(
        "0.1.0",
        "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    )
    .unwrap()
}

fn definition(kind: NodeKind, suspends: Option<Suspends>) -> NodeDefinition {
    NodeDefinition {
        definition: DefinitionRef {
            id: "example/value".parse().unwrap(),
            version: 1,
        },
        config_version: 1,
        kind,
        suspends,
        type_params: vec![],
        ports: Ports::Declared(vec![]),
        references: vec![],
        specializations: None,
        config_schema: None,
    }
}

fn registry(
    kind: NodeKind,
    suspends: Option<Suspends>,
) -> Result<Arc<orange_model::catalog::RegistrySnapshot>, RegistryError> {
    let definition = definition(kind, suspends);
    let key = ProviderKey {
        package: "example.pkg".into(),
        version: "1.0.0".into(),
        content_digest: DIGEST.into(),
        id: "impl".into(),
    };
    RegistryBuilder::core(origin())
        .definition(definition.clone())
        .provider(ProviderDescriptor {
            key,
            implements: BTreeSet::from([(definition.definition, 1)]),
            specializations: Default::default(),
        })
        .build()
}

fn document(kind: NodeKind, suspends: Option<Suspends>) -> GraphDocument {
    let snapshot = SignatureSnapshot {
        kind,
        suspends,
        ports: vec![],
        extensions: Default::default(),
    };
    let signature = orange_model::signature::semantic_key(&snapshot);
    let signature = signature.to_string();
    serde_json::from_value(json!({
        "format_version":1,
        "graph_id":"20000000-0000-4000-8000-000000000001",
        "revision":0,
        "nodes":[{
            "id":NODE,
            "definition_id":"example/value",
            "definition_version":1,
            "config_version":1,
            "config":{},
            "signature":signature,
            "provider":{"package":"example.pkg","version":"1.0.0","id":"impl"}
        }],
        "edges":[],
        "package_lock":[{"id":"example.pkg","version":"1.0.0","content_digest":DIGEST}],
        "signatures":{signature:snapshot}
    }))
    .unwrap()
}

fn assert_valid(kind: NodeKind, suspends: Option<Suspends>) {
    let registry = registry(kind, suspends).unwrap();
    let document = document(kind, suspends);
    let bytes = serde_json::to_vec(&document).unwrap();
    let revision = document.revision();
    let state_id = document.state_id();
    let report = validate_nodes(&document, &registry);
    assert!(report.is_valid(), "diagnostics: {:?}", report.diagnostics);
    assert_eq!(report.resolved.len(), 1);
    assert_eq!(serde_json::to_vec(&document).unwrap(), bytes);
    assert_eq!(document.revision(), revision);
    assert_eq!(document.state_id(), state_id);
}

#[test]
fn declared_async_and_iterator_contracts_require_maybe_or_always() {
    for kind in [NodeKind::Async, NodeKind::Iterator] {
        for suspends in [Suspends::Maybe, Suspends::Always] {
            assert_valid(kind, Some(suspends));
        }
        assert!(matches!(
            registry(kind, None),
            Err(RegistryError::SuspendsMissing { kind: actual, .. }) if actual == kind
        ));
        assert!(matches!(
            registry(kind, Some(Suspends::Never)),
            Err(RegistryError::SuspendsInvalid { kind: actual, suspends: Suspends::Never, .. }) if actual == kind
        ));
    }
}

#[test]
fn synchronous_kinds_allow_absent_or_never_and_reject_maybe_or_always() {
    for kind in [
        NodeKind::Pure,
        NodeKind::Volatile,
        NodeKind::Action,
        NodeKind::Entry,
    ] {
        for suspends in [None, Some(Suspends::Never)] {
            assert_valid(kind, suspends);
        }
        for suspends in [Suspends::Maybe, Suspends::Always] {
            assert!(matches!(
                registry(kind, Some(suspends)),
                Err(RegistryError::SuspendsInvalid { kind: actual, suspends: value, .. })
                    if actual == kind && value == suspends
            ));
        }
    }
}
