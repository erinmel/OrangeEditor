//! Package lifecycle: manifest validation and install/update/remove/rollback as registry
//! transactions (04 "Installation, activation and override", "Hot update/removal"; ADR-07).

use std::sync::Arc;

use orange_model::GraphDocument;
use orange_model::catalog::{Determinism, NodeDefinition, Ports, ProviderKey, RegistrySnapshot};
use orange_model::document::{DefinitionRef, NodeKind};
use orange_model::validation::{DiagnosticCode, HostOrigin, validate_nodes};
use orange_packages::{
    HOST_API_VERSION, HostApiVersion, Journal, JournalEntry, JournalError, ManifestError,
    MemoryJournal, PackageContents, PackageKey, PackageManifest, PackageState, PackageStore,
    ProviderSlot, Recovery, SlotKind, TransactionError,
};
use serde_json::{Value, json};

const PACKAGE: &str = "org.example.device";
const NODE: &str = "org.example.device/value";
const SIG: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HEX_1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const HEX_2: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const HOST: HostApiVersion = HOST_API_VERSION;

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

/// A manifest close to the 04 sample, with one node provider.
fn manifest_json(version: &str, digest: &str) -> Value {
    json!({
        "format": "orange.plugin", "format_version": 1,
        "id": PACKAGE, "version": version, "publisher": "org.example",
        "host_api": {"major": 1, "min_minor": 0, "max_minor": 0},
        "execution": {"kind": "wasm-component", "entry": "component.wasm",
                      "world": "orange:expansion/plugin@1.0.0"},
        "dependencies": [],
        "contributions": {
            "nodes": [NODE],
            "providers": [{"slot": format!("node:{NODE}@1"), "id": "impl"}]
        },
        "permissions": [],
        "integrity": {"algorithm": "sha256", "payload_digest": digest},
        "license": "MIT"
    })
}

fn parse(value: &Value) -> Result<PackageManifest, ManifestError> {
    PackageManifest::from_json(&serde_json::to_vec(value).unwrap())
}

fn contents(version: &str, digest: &str) -> PackageContents {
    PackageContents::new(
        parse(&manifest_json(version, digest)).unwrap(),
        vec![definition(NODE)],
    )
}

fn pkg(version: &str) -> PackageKey {
    PackageKey {
        id: PACKAGE.parse().unwrap(),
        version: version.parse().unwrap(),
    }
}

fn provider(version: &str, hex: &str) -> ProviderKey {
    ProviderKey {
        package: PACKAGE.into(),
        version: version.into(),
        content_digest: format!("sha256:{hex}"),
        id: "impl".into(),
    }
}

fn store() -> PackageStore {
    PackageStore::new(RegistrySnapshot::core(origin()), HOST, MemoryJournal::new())
}

/// A document whose only node is locked to provider `impl` of version 1.0.0.
fn document() -> GraphDocument {
    serde_json::from_value(json!({
        "format_version": 1,
        "graph_id": "20000000-0000-4000-8000-000000000001",
        "revision": 1,
        "nodes": [{
            "id": "10000000-0000-4000-8000-000000000001",
            "definition_id": NODE, "definition_version": 1,
            "config_version": 1, "config": {}, "signature": SIG,
            "provider": {"package": PACKAGE, "version": "1.0.0", "id": "impl"}
        }],
        "edges": [],
        "package_lock": [{"id": PACKAGE, "version": "1.0.0",
                          "content_digest": format!("sha256:{HEX_1}")}],
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
fn manifest_validation_accepts_the_sample_and_rejects_bad_inputs() {
    let manifest = parse(&manifest_json("1.0.0", HEX_1)).unwrap();
    assert_eq!(manifest.key(), &pkg("1.0.0"));
    assert_eq!(manifest.content_digest(), format!("sha256:{HEX_1}"));
    assert_eq!(manifest.nodes(), &[NODE.parse().unwrap()]);
    assert_eq!(manifest.providers()[0].slot, ProviderSlot::Node(dref(NODE)));
    assert_eq!(manifest.license(), Some("MIT"));

    let rejects = |edit: &dyn Fn(&mut Value), expected: ManifestError| {
        let mut value = manifest_json("1.0.0", HEX_1);
        edit(&mut value);
        assert_eq!(parse(&value), Err(expected), "{value}");
    };
    rejects(
        &|v| v["format"] = json!("other"),
        ManifestError::Format("other".into()),
    );
    rejects(
        &|v| v["format_version"] = json!(2),
        ManifestError::FormatVersion(2),
    );
    for version in ["1.0", "^1.0.0", "01.0.0", "1.0.0+build", "1.0.0-"] {
        rejects(
            &|v| v["version"] = json!(version),
            ManifestError::Version(version.into()),
        );
    }
    rejects(
        &|v| v["id"] = json!("Org.Example"),
        ManifestError::PackageId("Org.Example".into()),
    );
    rejects(
        &|v| v["publisher"] = json!("org.other"),
        ManifestError::PublisherNamespace {
            package: PACKAGE.into(),
            publisher: "org.other".into(),
        },
    );
    rejects(
        &|v| v["host_api"] = json!({"major": 1, "min_minor": 2, "max_minor": 1}),
        ManifestError::HostApiRange {
            major: 1,
            min_minor: 2,
            max_minor: 1,
        },
    );
    rejects(
        &|v| v["integrity"]["algorithm"] = json!("md5"),
        ManifestError::DigestAlgorithm("md5".into()),
    );
    rejects(
        &|v| v["integrity"]["payload_digest"] = json!("A".repeat(64)),
        ManifestError::Digest,
    );
    rejects(
        &|v| v["dependencies"] = json!([{"id": PACKAGE, "version": "1.0.0"}]),
        ManifestError::SelfDependency,
    );
    rejects(
        &|v| {
            v["dependencies"] = json!([
                {"id": "org.example.a", "version": "1.0.0"},
                {"id": "org.example.a", "version": "2.0.0"}
            ])
        },
        ManifestError::DuplicateDependency("org.example.a".into()),
    );
    rejects(
        &|v| v["contributions"]["nodes"] = json!(["org.other/value"]),
        ManifestError::NodeNamespace {
            node: "org.other/value".into(),
            package: PACKAGE.into(),
        },
    );
    rejects(
        &|v| v["contributions"]["nodes"] = json!([NODE, NODE]),
        ManifestError::DuplicateNode(NODE.into()),
    );
    for slot in ["widget:x", "node:org.example.device/value@01", "node:"] {
        rejects(
            &|v| v["contributions"]["providers"][0]["slot"] = json!(slot),
            ManifestError::Slot(slot.into()),
        );
    }
    rejects(
        &|v| v["contributions"]["providers"][0]["id"] = json!("Impl"),
        ManifestError::ProviderId("Impl".into()),
    );
    rejects(
        &|v| {
            let first = v["contributions"]["providers"][0].clone();
            v["contributions"]["providers"] = json!([first.clone(), first]);
        },
        ManifestError::DuplicateProvider {
            slot: format!("node:{NODE}@1").into(),
            id: "impl".into(),
        },
    );

    let mut misspelt = manifest_json("1.0.0", HEX_1);
    misspelt["dependancies"] = json!([]);
    assert!(matches!(
        parse(&misspelt),
        Err(ManifestError::Decode { .. })
    ));
    let oversized = vec![b' '; orange_packages::MAX_MANIFEST_BYTES + 1];
    assert!(matches!(
        PackageManifest::from_json(&oversized),
        Err(ManifestError::TooLarge { .. })
    ));
}

#[test]
fn install_publishes_a_new_generation_and_keeps_the_old_snapshot_readable() {
    let mut store = store();
    let base = Arc::clone(store.current());
    let next = store.install(contents("1.0.0", HEX_1)).unwrap();

    assert!(next.generation() > base.generation());
    assert!(Arc::ptr_eq(store.current(), &next));
    assert!(next.provides(&dref(NODE), 1, &provider("1.0.0", HEX_1)));
    assert!(base.definition(&dref(NODE)).is_none());
    assert!(base.provider(&provider("1.0.0", HEX_1)).is_none());
    assert!(base.definition(&dref("core/branch")).is_some());
    // No last-installed-wins: installing chooses no new-binding default.
    assert_eq!(next.new_binding_default(&dref(NODE)), None);

    let record = store.package(&pkg("1.0.0")).unwrap();
    assert_eq!(record.state(), PackageState::Active);
    assert_eq!(record.providers(), &[provider("1.0.0", HEX_1)]);
    assert_eq!(codes(&document(), &next), vec![]);
    assert_eq!(
        store.install(contents("1.0.0", HEX_1)).unwrap_err(),
        TransactionError::AlreadyActive(pkg("1.0.0"))
    );
    let entries = store.journal().entries();
    assert_eq!(
        Recovery::from_entries(entries).committed,
        Some(vec![pkg("1.0.0")])
    );
    // A crash before the commit record leaves a half-staged version that restart reports
    // instead of activating.
    let commit = entries
        .iter()
        .position(|entry| matches!(entry, JournalEntry::Committed { .. }))
        .unwrap();
    let crashed = Recovery::from_entries(&entries[..commit]);
    assert_eq!(crashed.committed, None);
    assert_eq!(crashed.abandoned, vec![pkg("1.0.0")]);
}

#[test]
fn the_published_host_api_version_gates_manifest_ranges() {
    let HostApiVersion { major, minor } = HOST_API_VERSION;
    let accepting = parse(&manifest_json("1.0.0", HEX_1)).unwrap().host_api();
    assert!(accepting.accepts(HOST_API_VERSION));

    let mut store = store();
    let mut value = manifest_json("1.0.0", HEX_1);
    value["host_api"] = json!({"major": major, "min_minor": minor + 1, "max_minor": minor + 1});
    let newer = parse(&value).unwrap();
    assert_eq!(
        store
            .install(PackageContents::new(newer.clone(), vec![definition(NODE)]))
            .unwrap_err(),
        TransactionError::HostApi {
            required: newer.host_api(),
            host: HOST_API_VERSION,
        }
    );
}

/// Fails every write of a committed generation.
#[derive(Debug, Default)]
struct FailingCommits(MemoryJournal);

impl Journal for FailingCommits {
    fn append(&mut self, entry: JournalEntry) -> Result<(), JournalError> {
        if matches!(entry, JournalEntry::Committed { .. }) {
            return Err(JournalError {
                reason: "disk full".into(),
            });
        }
        self.0.append(entry)
    }
}

#[test]
fn failed_install_quarantines_and_leaves_the_prior_registry_active() {
    let mut store = store();
    let before = store.install(contents("1.0.0", HEX_1)).unwrap();
    let key = pkg("2.0.0");
    let mut fail = |contents: PackageContents, expected: TransactionError| {
        assert_eq!(store.install(contents).unwrap_err(), expected.clone());
        assert!(Arc::ptr_eq(store.current(), &before));
        assert_eq!(store.quarantined(&key), Some(&expected));
        assert!(store.package(&key).is_none());
    };

    let mut value = manifest_json("2.0.0", HEX_2);
    value["host_api"] = json!({"major": 2, "min_minor": 0, "max_minor": 0});
    fail(
        PackageContents::new(parse(&value).unwrap(), vec![definition(NODE)]),
        TransactionError::HostApi {
            required: parse(&value).unwrap().host_api(),
            host: HOST,
        },
    );

    let mut value = manifest_json("2.0.0", HEX_2);
    value["dependencies"] = json!([{"id": "org.example.base", "version": "1.0.0"}]);
    fail(
        PackageContents::new(parse(&value).unwrap(), vec![definition(NODE)]),
        TransactionError::MissingDependency(PackageKey {
            id: "org.example.base".parse().unwrap(),
            version: "1.0.0".parse().unwrap(),
        }),
    );

    let mut changed = definition(NODE);
    changed.config_version = 2;
    fail(
        PackageContents::new(
            parse(&manifest_json("2.0.0", HEX_2)).unwrap(),
            vec![changed],
        ),
        TransactionError::DefinitionChanged(dref(NODE)),
    );

    let mut value = manifest_json("2.0.0", HEX_2);
    value["contributions"]["providers"] =
        json!([{"slot": "connection:org.example.device/session@1", "id": "impl"}]);
    fail(
        PackageContents::new(parse(&value).unwrap(), vec![definition(NODE)]),
        TransactionError::UnsupportedSlot {
            kind: SlotKind::Connection,
            descriptor: "org.example.device/session@1".into(),
        },
    );

    // Same version, different bytes: versions are immutable.
    assert_eq!(
        store.install(contents("1.0.0", HEX_2)).unwrap_err(),
        TransactionError::AlreadyActive(pkg("1.0.0"))
    );

    // A commit that cannot be journaled publishes nothing; restart treats the attempt as
    // abandoned and never as committed.
    let mut journaled = PackageStore::new(
        RegistrySnapshot::core(origin()),
        HOST,
        FailingCommits::default(),
    );
    let base = Arc::clone(journaled.current());
    assert!(matches!(
        journaled.install(contents("1.0.0", HEX_1)),
        Err(TransactionError::Journal(_))
    ));
    assert!(Arc::ptr_eq(journaled.current(), &base));
    let recovery = Recovery::from_entries(journaled.journal().0.entries());
    assert_eq!(recovery.committed, None);
    assert_eq!(recovery.abandoned, vec![]);
    let last = journaled.journal().0.entries().last().unwrap();
    assert_eq!(
        last,
        &JournalEntry::Transition {
            package: pkg("1.0.0"),
            state: PackageState::Quarantined
        }
    );
}

#[test]
fn update_keeps_existing_document_locks_on_the_old_version() {
    let mut store = store();
    store.install(contents("1.0.0", HEX_1)).unwrap();
    store
        .select_default(dref(NODE), provider("1.0.0", HEX_1))
        .unwrap();
    let doc = document();
    let old = Arc::clone(store.current());

    let next = store
        .update(&pkg("1.0.0"), contents("2.0.0", HEX_2))
        .unwrap();

    assert_eq!(codes(&doc, &next), vec![]);
    let resolved = validate_nodes(&doc, &next).resolved;
    assert_eq!(resolved.len(), 1);
    assert_eq!(doc, document());
    assert!(next.provides(&dref(NODE), 1, &provider("1.0.0", HEX_1)));
    assert!(next.provides(&dref(NODE), 1, &provider("2.0.0", HEX_2)));
    assert_eq!(
        next.new_binding_default(&dref(NODE)),
        Some(&provider("2.0.0", HEX_2))
    );
    assert_eq!(
        old.new_binding_default(&dref(NODE)),
        Some(&provider("1.0.0", HEX_1))
    );
    assert_eq!(
        store.package(&pkg("1.0.0")).unwrap().state(),
        PackageState::Active
    );
    assert_eq!(
        store
            .update(&pkg("2.0.0"), contents("2.0.0", HEX_2))
            .unwrap_err(),
        TransactionError::UpdateTarget {
            from: pkg("2.0.0"),
            to: pkg("2.0.0")
        }
    );
}

#[test]
fn remove_tombstones_and_locked_providers_resolve_unavailable() {
    let mut store = store();
    let installed = store.install(contents("1.0.0", HEX_1)).unwrap();
    store
        .select_default(dref(NODE), provider("1.0.0", HEX_1))
        .unwrap();
    let doc = document();

    let removed = store.remove(&pkg("1.0.0")).unwrap();

    let tombstone = store.package(&pkg("1.0.0")).unwrap();
    assert_eq!(tombstone.state(), PackageState::Removed);
    assert_eq!(tombstone.providers(), &[provider("1.0.0", HEX_1)]);
    assert_eq!(tombstone.generation(), removed.generation());
    assert!(removed.provider(&provider("1.0.0", HEX_1)).is_none());
    assert_eq!(removed.new_binding_default(&dref(NODE)), None);
    assert_eq!(
        codes(&doc, &removed),
        vec![DiagnosticCode::ProviderUnavailable]
    );
    assert!(validate_nodes(&doc, &removed).resolved.is_empty());
    assert_eq!(doc, document());
    assert_eq!(codes(&doc, &installed), vec![]);

    assert_eq!(
        store.remove(&pkg("1.0.0")).unwrap_err(),
        TransactionError::WrongState {
            key: pkg("1.0.0"),
            state: PackageState::Removed
        }
    );
    assert_eq!(
        store.install(contents("1.0.0", HEX_2)).unwrap_err(),
        TransactionError::DigestConflict(pkg("1.0.0"))
    );
    assert!(store.install(contents("1.0.0", HEX_1)).is_ok());
}

#[test]
fn remove_is_refused_while_an_active_package_depends_on_it() {
    let mut store = store();
    store.install(contents("1.0.0", HEX_1)).unwrap();
    let dependent = json!({
        "format": "orange.plugin", "format_version": 1,
        "id": "org.example.app", "version": "0.1.0", "publisher": "org.example",
        "host_api": {"major": 1, "min_minor": 0, "max_minor": 0},
        "dependencies": [{"id": PACKAGE, "version": "1.0.0"}],
        "integrity": {"algorithm": "sha256", "payload_digest": HEX_2}
    });
    store
        .install(PackageContents::new(parse(&dependent).unwrap(), vec![]))
        .unwrap();
    assert!(matches!(
        store.remove(&pkg("1.0.0")),
        Err(TransactionError::DependedOn { .. })
    ));
}

#[test]
fn rollback_restores_the_prior_generation() {
    let mut store = store();
    let base = Arc::clone(store.current());
    let installed = store.install(contents("1.0.0", HEX_1)).unwrap();
    store.remove(&pkg("1.0.0")).unwrap();

    let restored = store.rollback().unwrap();
    assert!(Arc::ptr_eq(&restored, &installed));
    assert!(Arc::ptr_eq(store.current(), &installed));
    assert_eq!(
        store.package(&pkg("1.0.0")).unwrap().state(),
        PackageState::Active
    );
    assert_eq!(codes(&document(), &restored), vec![]);

    assert!(Arc::ptr_eq(&store.rollback().unwrap(), &base));
    assert_eq!(
        store.package(&pkg("1.0.0")).unwrap().state(),
        PackageState::Removed
    );
    assert_eq!(
        store.rollback().unwrap_err(),
        TransactionError::NothingToRollBack
    );
    assert_eq!(
        Recovery::from_entries(store.journal().entries()).committed,
        Some(vec![])
    );
}

#[test]
fn rollback_keeps_version_identity_and_reconciles_quarantine() {
    let mut store = store();
    store.install(contents("1.0.0", HEX_1)).unwrap();
    store.rollback().unwrap();
    assert_eq!(
        store.published_digest(&pkg("1.0.0")),
        Some(&*format!("sha256:{HEX_1}"))
    );
    assert_eq!(
        store.install(contents("1.0.0", HEX_2)).unwrap_err(),
        TransactionError::DigestConflict(pkg("1.0.0"))
    );
    store.install(contents("1.0.0", HEX_1)).unwrap();

    // A failed reinstall over a tombstone is quarantined; rolling back the removal makes
    // the version active again and drops the stale quarantine entry.
    store.remove(&pkg("1.0.0")).unwrap();
    let mut incompatible = manifest_json("1.0.0", HEX_1);
    incompatible["host_api"] = json!({"major": 2, "min_minor": 0, "max_minor": 0});
    let attempt = PackageContents::new(parse(&incompatible).unwrap(), vec![definition(NODE)]);
    assert!(store.install(attempt).is_err());
    assert!(store.quarantined(&pkg("1.0.0")).is_some());
    store.rollback().unwrap();
    assert_eq!(
        store.package(&pkg("1.0.0")).unwrap().state(),
        PackageState::Active
    );
    assert_eq!(store.quarantined(&pkg("1.0.0")), None);
}

#[test]
fn recovery_reports_a_clean_rollback_as_removed_not_abandoned() {
    let mut store = store();
    store.install(contents("1.0.0", HEX_1)).unwrap();
    store.install(contents("2.0.0", HEX_2)).unwrap();
    store.rollback().unwrap();

    let recovery = Recovery::from_entries(store.journal().entries());
    assert_eq!(recovery.committed, Some(vec![pkg("1.0.0")]));
    assert_eq!(recovery.abandoned, vec![]);
    assert_eq!(recovery.removed, vec![pkg("2.0.0")]);
    assert_eq!(
        store.journal().entries().last(),
        Some(&JournalEntry::Committed {
            generation: store.current().generation(),
            active: vec![pkg("1.0.0")],
            published: [
                (pkg("1.0.0"), format!("sha256:{HEX_1}")),
                (pkg("2.0.0"), format!("sha256:{HEX_2}")),
            ]
            .into(),
            revoked: Default::default(),
        })
    );
}

/// A package with no nodes, used beside the main one.
fn other(digest: &str) -> PackageContents {
    let manifest = json!({
        "format": "orange.plugin", "format_version": 1,
        "id": "org.example.other", "version": "1.0.0", "publisher": "org.example",
        "host_api": {"major": 1, "min_minor": 0, "max_minor": 0},
        "integrity": {"algorithm": "sha256", "payload_digest": digest}
    });
    PackageContents::new(parse(&manifest).unwrap(), vec![])
}

#[test]
fn restart_restores_active_removed_and_revoked_versions() {
    const HEX_3: &str = "3333333333333333333333333333333333333333333333333333333333333333";
    let mut store = store();
    store.install(contents("1.0.0", HEX_1)).unwrap();
    store
        .update(&pkg("1.0.0"), contents("2.0.0", HEX_2))
        .unwrap();
    store.remove(&pkg("1.0.0")).unwrap();
    store.install(other(HEX_3)).unwrap();
    let other_key = other(HEX_3).manifest().key().clone();
    store.revoke(&other_key).unwrap();

    let recovery = Recovery::from_entries(store.journal().entries());
    assert_eq!(recovery.committed, Some(vec![pkg("2.0.0")]));
    assert_eq!(recovery.removed, vec![pkg("1.0.0")]);
    assert_eq!(recovery.revoked, [format!("sha256:{HEX_3}")].into());

    let restart = |contents| {
        PackageStore::restore(
            RegistrySnapshot::core(origin()),
            HOST,
            MemoryJournal::new(),
            &recovery,
            contents,
        )
    };
    assert_eq!(
        restart(vec![]).unwrap_err(),
        TransactionError::RecoveryMismatch(pkg("2.0.0"))
    );
    assert_eq!(
        restart(vec![contents("1.0.0", HEX_1)]).unwrap_err(),
        TransactionError::RecoveryMismatch(pkg("1.0.0"))
    );
    let mut restored = restart(vec![contents("2.0.0", HEX_2)]).unwrap();
    assert!(
        restored
            .current()
            .provides(&dref(NODE), 1, &provider("2.0.0", HEX_2))
    );
    assert!(
        restored
            .current()
            .provider(&provider("1.0.0", HEX_1))
            .is_none()
    );
    assert!(!restored.can_roll_back());
    assert_eq!(
        codes(&document(), restored.current()),
        vec![DiagnosticCode::ProviderUnavailable]
    );
    assert_eq!(
        restored.install(contents("1.0.0", HEX_2)).unwrap_err(),
        TransactionError::DigestConflict(pkg("1.0.0"))
    );
    assert_eq!(
        restored.install(other(HEX_3)).unwrap_err(),
        TransactionError::Revoked(other_key)
    );
    restored.install(contents("1.0.0", HEX_1)).unwrap();
}

#[test]
fn revoking_a_digest_deactivates_every_version_published_with_it() {
    let mut store = store();
    store.install(contents("1.0.0", HEX_1)).unwrap();
    store.install(contents("1.0.1", HEX_1)).unwrap();
    let revoked = store.revoke(&pkg("1.0.0")).unwrap();
    for version in ["1.0.0", "1.0.1"] {
        assert_eq!(
            store.package(&pkg(version)).unwrap().state(),
            PackageState::Revoked
        );
        assert!(revoked.provider(&provider(version, HEX_1)).is_none());
    }
    assert!(store.is_revoked(&format!("sha256:{HEX_1}")));
    assert_eq!(
        Recovery::from_entries(store.journal().entries()).committed,
        Some(vec![])
    );
}

#[test]
fn revocation_blocks_reinstall_and_rollback() {
    let mut store = store();
    store.install(contents("1.0.0", HEX_1)).unwrap();
    let revoked = store.revoke(&pkg("1.0.0")).unwrap();
    assert!(revoked.provider(&provider("1.0.0", HEX_1)).is_none());
    assert_eq!(
        store.package(&pkg("1.0.0")).unwrap().state(),
        PackageState::Revoked
    );
    assert!(!store.can_roll_back());
    assert_eq!(
        store.rollback().unwrap_err(),
        TransactionError::NothingToRollBack
    );
    assert_eq!(
        store.install(contents("1.0.0", HEX_1)).unwrap_err(),
        TransactionError::Revoked(pkg("1.0.0"))
    );
}

#[test]
fn lifecycle_states_follow_the_installation_order() {
    use PackageState::*;
    let order = [Discovered, Staged, Validated, Granted, Activating, Active];
    for pair in order.windows(2) {
        assert!(pair[0].can_become(pair[1]));
        assert!(!pair[1].can_become(pair[0]));
    }
    assert!(Activating.can_become(Quarantined));
    assert!(!Active.can_become(Quarantined));
    assert!(!Discovered.can_become(Active));
    assert!(Active.can_become(Removed) && Removed.can_become(Revoked));
    assert!(!Removed.can_become(Active) && !Revoked.can_become(Removed));
    assert!(!Quarantined.can_become(Staged));
}
