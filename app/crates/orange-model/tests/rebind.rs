//! Explicit provider rebinds: preview, a checked copy, and commit (04 "Hot update/removal",
//! ADR-07, 12 §8).

use std::sync::Arc;

use orange_model::catalog::{
    Determinism, NodeDefinition, Ports, ProviderDescriptor, ProviderKey, RegistryBuilder,
    RegistrySnapshot,
};
use orange_model::document::{DefinitionRef, NodeKind, ProviderBinding};
use orange_model::edit::{
    PlanApplyError, RebindError, RebindOutcome, RebindPlan, RebindRequest, RebindSelection,
    apply_rebind, plan_rebind,
};
use orange_model::id::NodeId;
use orange_model::validation::{DiagnosticCode, HostOrigin};
use orange_model::{DocCommand, GraphDocument, validate};
use serde_json::json;

const A1: &str = "10000000-0000-4000-8000-000000000001";
const A2: &str = "10000000-0000-4000-8000-000000000002";
const B1: &str = "10000000-0000-4000-8000-000000000003";
const OTHER: &str = "10000000-0000-4000-8000-000000000004";
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

fn node_id(id: &str) -> NodeId {
    id.parse().unwrap()
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

fn provider(key: ProviderKey, implements: &str) -> ProviderDescriptor {
    ProviderDescriptor {
        key,
        implements: [(dref(implements), 1)].into(),
        specializations: Default::default(),
    }
}

fn a() -> ProviderKey {
    key("1.0.0", DIGEST, "impl-a")
}

fn b() -> ProviderKey {
    key("1.0.0", DIGEST, "impl-b")
}

/// Implements only `example/other`.
fn c() -> ProviderKey {
    key("1.0.0", DIGEST, "impl-c")
}

/// Provider B of the next package version.
fn b2() -> ProviderKey {
    key("2.0.0", DIGEST_2, "impl-b")
}

fn registry() -> Arc<RegistrySnapshot> {
    RegistryBuilder::core(origin())
        .definition(definition("example/value"))
        .definition(definition("example/other"))
        .provider(provider(a(), "example/value"))
        .provider(provider(b(), "example/value"))
        .provider(provider(c(), "example/other"))
        .build()
        .unwrap()
}

/// The next generation adds B of package version 2.0.0.
fn updated(old: &RegistrySnapshot) -> Arc<RegistrySnapshot> {
    RegistryBuilder::from_snapshot(old)
        .provider(provider(b2(), "example/value"))
        .build()
        .unwrap()
}

fn node(id: &str, definition: &str, provider: &str) -> serde_json::Value {
    json!({
        "id": id, "definition_id": definition, "definition_version": 1,
        "config_version": 1, "config": {}, "signature": SIG,
        "provider": {"package":"example.pkg", "version":"1.0.0", "id":provider}
    })
}

/// Two nodes on A, one on B, and one `example/other` node on C, all at `example.pkg@1.0.0`.
fn document() -> GraphDocument {
    serde_json::from_value(json!({
        "format_version": 1,
        "graph_id": "20000000-0000-4000-8000-000000000001",
        "revision": 1,
        "nodes": [
            node(A1, "example/value", "impl-a"),
            node(B1, "example/value", "impl-b"),
            node(A2, "example/value", "impl-a"),
            node(OTHER, "example/other", "impl-c"),
        ],
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

fn binding(document: &GraphDocument, id: &str) -> Option<ProviderBinding> {
    document.node(node_id(id)).unwrap().provider.clone()
}

fn ready(outcome: RebindOutcome) -> RebindPlan {
    match outcome {
        RebindOutcome::Ready(plan) => plan,
        RebindOutcome::Rejected { errors, .. } => panic!("rebind rejected: {errors:?}"),
    }
}

fn request(from: RebindSelection, to: ProviderKey) -> RebindRequest {
    RebindRequest { from, to }
}

#[test]
fn preview_lists_exactly_the_selected_bindings() {
    let old = registry();
    let next = updated(&old);
    let doc = document();

    let by_provider =
        ready(plan_rebind(&doc, &next, request(RebindSelection::Provider(a()), b2())).unwrap());
    assert_eq!(by_provider.affected, vec![node_id(A1), node_id(A2)]);
    assert_eq!(
        by_provider.commands()[1],
        DocCommand::Rebind {
            nodes: vec![node_id(A1), node_id(A2)],
            provider: ProviderBinding {
                package: "example.pkg".into(),
                version: "2.0.0".into(),
                id: "impl-b".into(),
                extensions: Default::default(),
            },
        }
    );
    assert!(matches!(
        &by_provider.commands()[0],
        DocCommand::PutPackageLock(lock)
            if lock.version == "2.0.0" && lock.content_digest == DIGEST_2
    ));

    let by_definition = ready(
        plan_rebind(
            &doc,
            &next,
            request(RebindSelection::Definition(dref("example/value")), b2()),
        )
        .unwrap(),
    );
    assert_eq!(
        by_definition.affected,
        vec![node_id(A1), node_id(B1), node_id(A2)]
    );

    // Nodes already bound to the target are not rebound.
    let onto_b = ready(
        plan_rebind(
            &doc,
            &old,
            request(RebindSelection::Definition(dref("example/value")), b()),
        )
        .unwrap(),
    );
    assert_eq!(onto_b.affected, vec![node_id(A1), node_id(A2)]);
    assert_eq!(doc, document());
}

#[test]
fn a_committed_rebind_updates_locks_and_only_the_selected_bindings() {
    let old = registry();
    let next = updated(&old);
    let mut doc = document();
    let plan =
        ready(plan_rebind(&doc, &next, request(RebindSelection::Provider(a()), b2())).unwrap());

    let committed = apply_rebind(&mut doc, &next, plan).unwrap();
    assert_eq!(doc.revision(), 2);
    for id in [A1, A2] {
        let bound = binding(&doc, id).unwrap();
        assert_eq!(
            (bound.version.as_str(), bound.id.as_str()),
            ("2.0.0", "impl-b")
        );
    }
    let original = document();
    for id in [B1, OTHER] {
        assert_eq!(binding(&doc, id), binding(&original, id));
    }
    let locks: Vec<_> = doc
        .package_lock()
        .iter()
        .map(|lock| (lock.version.as_str(), lock.content_digest.as_str()))
        .collect();
    assert_eq!(locks, vec![("1.0.0", DIGEST), ("2.0.0", DIGEST_2)]);
    assert!(validate(&doc, &next).is_valid());

    let _ = doc.revert(committed.inverse).unwrap();
    assert_eq!(binding(&doc, A1), binding(&original, A1));
    assert_eq!(doc.package_lock(), original.package_lock());
}

#[test]
fn a_rebind_that_fails_analysis_is_rejected_and_changes_nothing() {
    let registry = registry();
    let doc = document();
    let outcome = plan_rebind(
        &doc,
        &registry,
        request(RebindSelection::Provider(a()), c()),
    )
    .unwrap();
    let RebindOutcome::Rejected { affected, errors } = outcome else {
        panic!("C does not implement example/value");
    };
    assert_eq!(affected, vec![node_id(A1), node_id(A2)]);
    let reported: Vec<_> = errors
        .iter()
        .map(|item| (item.code, item.subject.node))
        .collect();
    assert_eq!(
        reported,
        vec![
            (DiagnosticCode::ProviderUnavailable, Some(node_id(A1))),
            (DiagnosticCode::ProviderUnavailable, Some(node_id(A2))),
        ]
    );
    assert_eq!(doc, document());
}

#[test]
fn errors_the_document_already_had_do_not_block_a_repairing_rebind() {
    let old = registry();
    let removed = RegistryBuilder::from_snapshot(&old)
        .remove_provider(a())
        .build()
        .unwrap();
    let mut doc = document();
    assert!(!validate(&doc, &removed).is_valid());

    let plan =
        ready(plan_rebind(&doc, &removed, request(RebindSelection::Provider(a()), b())).unwrap());
    let _ = apply_rebind(&mut doc, &removed, plan).unwrap();
    assert!(validate(&doc, &removed).is_valid());
    // The package version was already locked to the same digest.
    assert_eq!(doc.package_lock(), document().package_lock());
}

#[test]
fn unknown_targets_empty_selections_and_stale_plans_are_rejected() {
    let old = registry();
    let next = updated(&old);
    let mut doc = document();
    let missing = key("1.0.0", DIGEST, "impl-z");
    assert_eq!(
        plan_rebind(
            &doc,
            &old,
            request(RebindSelection::Provider(a()), missing.clone())
        ),
        Err(RebindError::UnknownProvider(Box::new(missing)))
    );
    // B2 exists only in the next generation.
    assert_eq!(
        plan_rebind(&doc, &old, request(RebindSelection::Provider(a()), b2())),
        Err(RebindError::UnknownProvider(Box::new(b2())))
    );
    assert_eq!(
        plan_rebind(&doc, &next, request(RebindSelection::Provider(b2()), b())),
        Err(RebindError::NothingToRebind)
    );
    assert_eq!(doc, document());

    let plan =
        ready(plan_rebind(&doc, &old, request(RebindSelection::Provider(a()), b())).unwrap());
    assert_eq!(
        apply_rebind(&mut doc, &next, plan),
        Err(PlanApplyError::StaleRegistry)
    );
    assert_eq!(doc, document());
}
