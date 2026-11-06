//! Registry generations: publishing a next snapshot, provider replacement and removal, and
//! new-binding defaults (04 "Installation, activation and override", 12 §8).

use std::sync::Arc;

use orange_model::GraphDocument;
use orange_model::catalog::{
    Determinism, NodeDefinition, Ports, ProviderDescriptor, ProviderKey, RegistryBuilder,
    RegistryError, RegistrySnapshot,
};
use orange_model::document::{DefinitionRef, NodeKind};
use orange_model::validation::{DiagnosticCode, HostOrigin, validate_nodes};
use serde_json::json;

const NODE: &str = "10000000-0000-4000-8000-000000000001";
const SIG: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const DIGEST_2: &str = "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

fn origin() -> HostOrigin {
    HostOrigin::new("0.1.0", format!("sha256:{}", "c".repeat(64))).unwrap()
}

fn dref(id: &str) -> DefinitionRef {
    DefinitionRef {
        id: id.parse().unwrap(),
        version: 1,
    }
}

fn definition(id: &str) -> NodeDefinition {
    NodeDefinition {
        definition: dref(id),
        config_version: 1,
        kind: NodeKind::Pure,
        volatility: None,
        determinism: Determinism::Deterministic,
        suspends: None,
        type_params: vec![],
        ports: Ports::Declared(vec![
            serde_json::from_value(json!({
                "port_id":"value", "direction":"output", "kind":"data",
                "type":"core/i64@1", "source":"computed"
            }))
            .unwrap(),
        ]),
        references: vec![],
        specializations: None,
        config_schema: None,
    }
}

fn key(version: &str, digest: &str, id: &str) -> ProviderKey {
    ProviderKey {
        package: "example.pkg".into(),
        version: version.into(),
        content_digest: digest.into(),
        id: id.into(),
    }
}

fn provider(key: ProviderKey) -> ProviderDescriptor {
    ProviderDescriptor {
        key,
        implements: [(dref("example/value"), 1)].into(),
        specializations: Default::default(),
    }
}

fn a() -> ProviderKey {
    key("1.0.0", DIGEST, "impl-a")
}

fn b() -> ProviderKey {
    key("1.0.0", DIGEST, "impl-b")
}

/// Generation one: the example definition with providers A and B of one package version.
fn first() -> Arc<RegistrySnapshot> {
    RegistryBuilder::core(origin())
        .definition(definition("example/value"))
        .provider(provider(a()))
        .provider(provider(b()))
        .build()
        .unwrap()
}

/// A document whose only node is locked to `example.pkg@1.0.0` provider A.
fn document() -> GraphDocument {
    serde_json::from_value(json!({
        "format_version": 1,
        "graph_id": "20000000-0000-4000-8000-000000000001",
        "revision": 1,
        "nodes": [{
            "id": NODE, "definition_id": "example/value", "definition_version": 1,
            "config_version": 1, "config": {}, "signature": SIG,
            "provider": {"package":"example.pkg", "version":"1.0.0", "id":"impl-a"}
        }],
        "edges": [],
        "package_lock": [{"id":"example.pkg", "version":"1.0.0", "content_digest":DIGEST}],
        "signatures": {SIG: {
            "kind":"pure",
            "ports":[{"port_id":"value","direction":"output","kind":"data",
                      "type":"core/i64@1","source":"computed"}]
        }}
    }))
    .unwrap()
}

fn codes(document: &GraphDocument, registry: &RegistrySnapshot) -> Vec<DiagnosticCode> {
    validate_nodes(document, registry)
        .diagnostics
        .iter()
        .map(|item| item.code)
        .collect()
}

#[test]
fn publishing_a_generation_leaves_the_previous_snapshot_unchanged() {
    let old = first();
    let next = RegistryBuilder::from_snapshot(&old)
        .definition(definition("example/other"))
        .replace_provider(a(), provider(key("2.0.0", DIGEST_2, "impl-a")))
        .build()
        .unwrap();

    assert!(next.generation() > old.generation());
    assert!(old.provider(&a()).is_some());
    assert!(old.definition(&dref("example/other")).is_none());
    assert!(next.provider(&a()).is_none());
    assert!(next.provides(&dref("example/value"), 1, &key("2.0.0", DIGEST_2, "impl-a")));
    assert!(next.definition(&dref("example/other")).is_some());
    assert!(next.definition(&dref("core/branch")).is_some());

    let unchanged = RegistryBuilder::from_snapshot(&next).build().unwrap();
    assert!(unchanged.generation() > next.generation());
}

#[test]
fn duplicate_unknown_and_conflicting_providers_are_rejected() {
    let old = first();
    let next = || RegistryBuilder::from_snapshot(&old);
    assert_eq!(
        next().provider(provider(a())).build().unwrap_err(),
        RegistryError::DuplicateProvider(a())
    );
    let missing = key("1.0.0", DIGEST, "impl-z");
    assert_eq!(
        next().remove_provider(missing.clone()).build().unwrap_err(),
        RegistryError::UnknownProvider(Box::new(missing))
    );
    assert_eq!(
        next()
            .provider(provider(key("1.0.0", DIGEST_2, "impl-c")))
            .build()
            .unwrap_err(),
        RegistryError::PackageDigestConflict {
            package: "example.pkg".into(),
            version: "1.0.0".into()
        }
    );
    assert_eq!(
        next()
            .definition(definition("example/value"))
            .build()
            .unwrap_err(),
        RegistryError::DuplicateDefinition(dref("example/value"))
    );
    assert!(old.provider(&a()).is_some());
}

#[test]
fn new_binding_defaults_select_new_bindings_without_rebinding_documents() {
    let old = first();
    let doc = document();
    let next = RegistryBuilder::from_snapshot(&old)
        .new_binding_default(dref("example/value"), b())
        .build()
        .unwrap();
    assert_eq!(old.new_binding_default(&dref("example/value")), None);
    assert_eq!(next.new_binding_default(&dref("example/value")), Some(&b()));
    assert_eq!(codes(&doc, &next), vec![]);
    assert_eq!(doc, document());

    let b2 = key("2.0.0", DIGEST_2, "impl-b");
    let replaced = RegistryBuilder::from_snapshot(&next)
        .replace_provider(b(), provider(b2.clone()))
        .build()
        .unwrap();
    assert_eq!(
        replaced.new_binding_default(&dref("example/value")),
        Some(&b2)
    );
    let removed = RegistryBuilder::from_snapshot(&replaced)
        .remove_provider(b2)
        .build()
        .unwrap();
    assert_eq!(removed.new_binding_default(&dref("example/value")), None);

    let unimplemented = RegistryBuilder::from_snapshot(&old)
        .definition(definition("example/other"))
        .new_binding_default(dref("example/other"), b())
        .build();
    assert!(matches!(
        unimplemented,
        Err(RegistryError::DefaultNotProvided { .. })
    ));
}

#[test]
fn clearing_a_default_keeps_providers_and_locked_documents_unchanged() {
    let original = first();
    let doc = document();
    let selected = RegistryBuilder::from_snapshot(&original)
        .new_binding_default(dref("example/value"), b())
        .build()
        .unwrap();
    let cleared = RegistryBuilder::from_snapshot(&selected)
        .clear_new_binding_default(dref("example/value"))
        .build()
        .unwrap();
    assert!(cleared.generation() > selected.generation());
    assert_eq!(
        selected.new_binding_default(&dref("example/value")),
        Some(&b())
    );
    assert_eq!(cleared.new_binding_default(&dref("example/value")), None);
    assert!(cleared.provider(&a()).is_some());
    assert!(cleared.provider(&b()).is_some());
    assert!(cleared.definition(&dref("example/value")).is_some());
    assert_eq!(codes(&doc, &cleared), vec![]);
    assert_eq!(doc, document());

    // Clearing an already absent choice is valid; it does not remove a provider.
    let again = RegistryBuilder::from_snapshot(&cleared)
        .clear_new_binding_default(dref("example/value"))
        .build()
        .unwrap();
    assert_eq!(again.new_binding_default(&dref("example/value")), None);
    assert!(again.provider(&b()).is_some());
    assert_eq!(codes(&doc, &again), vec![]);
}

#[test]
fn removing_a_locked_provider_reports_it_unavailable_without_substitution() {
    let old = first();
    let doc = document();
    let next = RegistryBuilder::from_snapshot(&old)
        .new_binding_default(dref("example/value"), b())
        .remove_provider(a())
        .build()
        .unwrap();
    assert_eq!(codes(&doc, &old), vec![]);
    assert_eq!(
        codes(&doc, &next),
        vec![DiagnosticCode::ProviderUnavailable]
    );
    assert!(validate_nodes(&doc, &next).resolved.is_empty());
    assert_eq!(doc, document());
}
