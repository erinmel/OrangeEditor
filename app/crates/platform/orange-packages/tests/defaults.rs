//! New-binding provider defaults are part of the recovery checkpoint (12 §8): a restart
//! reproduces the exact selection, never an install-order guess, and never rebinds saved
//! document locks.

use crate::common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::*;
use orange_model::GraphDocument;
use orange_model::catalog::{ProviderDescriptor, ProviderKey, RegistryBuilder, RegistrySnapshot};
use orange_model::validation::validate_nodes;
use orange_packages::{MemoryJournal, PackageContents, PackageStore, Recovery, TransactionError};
use serde_json::json;

const OTHER: &str = "org.base/other";

fn restart(
    store: &PackageStore,
    base: Arc<RegistrySnapshot>,
    live: Vec<PackageContents>,
) -> Result<PackageStore, TransactionError> {
    let recovery = Recovery::from_entries(store.journal().entries());
    PackageStore::restore(base, HOST, MemoryJournal::new(), &recovery, live)
}

fn default_of(store: &PackageStore) -> Option<ProviderKey> {
    store.current().new_binding_default(&dref(NODE)).cloned()
}

fn two_versions(base: &Arc<RegistrySnapshot>) -> PackageStore {
    let mut store = PackageStore::new(Arc::clone(base), HOST, MemoryJournal::new());
    store.install(device("1.0.0", HEX_1)).unwrap();
    store.install(device("2.0.0", HEX_2)).unwrap();
    store
}

fn both() -> Vec<PackageContents> {
    vec![device("2.0.0", HEX_2), device("1.0.0", HEX_1)]
}

fn base_with_default() -> (Arc<RegistrySnapshot>, ProviderKey) {
    let base_provider = ProviderKey {
        package: "org.base".into(),
        version: "1.0.0".into(),
        content_digest: format!("sha256:{}", "b".repeat(64)),
        id: "impl".into(),
    };
    let base = RegistryBuilder::from_snapshot(&RegistrySnapshot::core(origin()))
        .definition(definition(NODE))
        .definition(definition(OTHER))
        .provider(ProviderDescriptor {
            key: base_provider.clone(),
            implements: BTreeSet::from([(dref(NODE), 1), (dref(OTHER), 1)]),
            specializations: Default::default(),
        })
        .new_binding_default(dref(NODE), base_provider.clone())
        .new_binding_default(dref(OTHER), base_provider.clone())
        .build()
        .unwrap();
    (base, base_provider)
}

#[test]
fn removing_an_override_keeps_the_default_cleared_across_restarts() {
    let (base, base_provider) = base_with_default();
    let mut store = PackageStore::new(Arc::clone(&base), HOST, MemoryJournal::new());
    store.install(device("1.0.0", HEX_1)).unwrap();
    store
        .select_default(dref(NODE), provider("1.0.0", HEX_1))
        .unwrap();
    store.remove(&pkg(PACKAGE, "1.0.0")).unwrap();
    assert_eq!(default_of(&store), None);
    let restored = restart(&store, Arc::clone(&base), vec![]).unwrap();
    assert_eq!(default_of(&restored), None);
    let again = restart(&restored, base, vec![]).unwrap();
    assert_eq!(default_of(&again), None);
    assert_eq!(
        again.current().new_binding_default(&dref(OTHER)),
        Some(&base_provider),
        "recovering the controlled definition must not clear unrelated base policy"
    );
}

#[test]
fn cleared_defaults_survive_compaction_retirement_and_failed_restore() {
    let (base, base_provider) = base_with_default();
    let mut store = PackageStore::new(Arc::clone(&base), HOST, MemoryJournal::new());
    store.install(device("1.0.0", HEX_1)).unwrap();
    store
        .select_default(dref(NODE), provider("1.0.0", HEX_1))
        .unwrap();
    store.remove(&pkg(PACKAGE, "1.0.0")).unwrap();
    store.set_retention(orange_packages::RetentionPolicy {
        max_rollback_generations: 0,
    });
    let before = Recovery::from_entries(store.journal().entries());
    store.compact_journal().unwrap();
    assert_eq!(store.journal().entries().len(), 1);
    let compacted = Recovery::from_entries(store.journal().entries());
    assert_eq!(compacted.defaults, before.defaults);
    assert_eq!(compacted.defaults.get(&dref(NODE)), Some(&None));

    // The failed final append cannot replace even a compacted last-good checkpoint.
    let entries = store.journal().entries();
    let (journal, durable) = Flaky::over(entries, Some(0));
    assert!(matches!(
        PackageStore::restore(Arc::clone(&base), HOST, journal, &compacted, vec![]),
        Err(TransactionError::Journal(_))
    ));
    assert_eq!(*durable.borrow(), entries);
    assert_eq!(Recovery::from_entries(&durable.borrow()), compacted);
    let mut restored = restart(&store, Arc::clone(&base), vec![]).unwrap();
    assert_eq!(default_of(&restored), None);
    assert_eq!(
        restored.current().new_binding_default(&dref(OTHER)),
        Some(&base_provider)
    );
    restored.compact_journal().unwrap();
    let again = restart(&restored, base, vec![]).unwrap();
    assert_eq!(default_of(&again), None);
    assert_eq!(
        Recovery::from_entries(again.journal().entries()).defaults,
        compacted.defaults
    );
}

#[test]
fn revoking_an_override_keeps_the_default_cleared_and_old_snapshot_pinned() {
    let (base, _) = base_with_default();
    let mut store = PackageStore::new(Arc::clone(&base), HOST, MemoryJournal::new());
    store.install(device("1.0.0", HEX_1)).unwrap();
    store
        .select_default(dref(NODE), provider("1.0.0", HEX_1))
        .unwrap();
    let pinned = Arc::clone(store.current());
    store.revoke(&pkg(PACKAGE, "1.0.0")).unwrap();
    assert_eq!(default_of(&store), None);
    assert!(
        store
            .current()
            .provider(&provider("1.0.0", HEX_1))
            .is_none()
    );
    let mut restored = restart(&store, Arc::clone(&base), vec![]).unwrap();
    assert_eq!(default_of(&restored), None);
    assert_eq!(
        restored.install(device("1.0.0", HEX_1)).unwrap_err(),
        TransactionError::Revoked(pkg(PACKAGE, "1.0.0"))
    );
    let again = restart(&restored, base, vec![]).unwrap();
    assert_eq!(default_of(&again), None);
    assert!(again.is_revoked(&format!("sha256:{HEX_1}")));
    assert_eq!(
        pinned.new_binding_default(&dref(NODE)),
        Some(&provider("1.0.0", HEX_1))
    );
    assert!(pinned.provider(&provider("1.0.0", HEX_1)).is_some());
}

#[test]
fn rollback_restores_each_selection_without_mutating_a_pinned_cleared_snapshot() {
    let (base, base_provider) = base_with_default();
    let mut store = PackageStore::new(Arc::clone(&base), HOST, MemoryJournal::new());
    store.install(device("1.0.0", HEX_1)).unwrap();
    store
        .select_default(dref(NODE), provider("1.0.0", HEX_1))
        .unwrap();
    store.remove(&pkg(PACKAGE, "1.0.0")).unwrap();
    let cleared = Arc::clone(store.current());
    store.rollback().unwrap();
    assert_eq!(default_of(&store), Some(provider("1.0.0", HEX_1)));
    let restored = restart(&store, Arc::clone(&base), vec![device("1.0.0", HEX_1)]).unwrap();
    assert_eq!(default_of(&restored), Some(provider("1.0.0", HEX_1)));
    store.rollback().unwrap();
    assert_eq!(default_of(&store), Some(base_provider.clone()));
    let restored = restart(&store, base, vec![device("1.0.0", HEX_1)]).unwrap();
    assert_eq!(default_of(&restored), Some(base_provider));
    assert_eq!(cleared.new_binding_default(&dref(NODE)), None);
    assert!(cleared.provider(&provider("1.0.0", HEX_1)).is_none());
}

#[test]
fn a_failed_selection_does_not_make_unrelated_base_policy_controlled() {
    let (base, base_provider) = base_with_default();
    let alternative = ProviderKey {
        id: "alternative".into(),
        ..base_provider.clone()
    };
    let base = RegistryBuilder::from_snapshot(&base)
        .provider(ProviderDescriptor {
            key: alternative.clone(),
            implements: BTreeSet::from([(dref(NODE), 1)]),
            specializations: Default::default(),
        })
        .build()
        .unwrap();
    let (journal, durable) = Flaky::over(&[], Some(0));
    let mut store = PackageStore::new(base, HOST, journal);
    assert!(matches!(
        store.select_default(dref(NODE), alternative),
        Err(TransactionError::Journal(_))
    ));
    assert_eq!(
        store.current().new_binding_default(&dref(NODE)),
        Some(&base_provider)
    );
    assert!(durable.borrow().is_empty());

    // A later unrelated commit cannot publish policy left behind by the failed one.
    store
        .install(app("org.example.unrelated", HEX_3, &[]))
        .unwrap();
    let recovery = Recovery::from_entries(&durable.borrow());
    assert!(!recovery.defaults.contains_key(&dref(NODE)));
    assert!(!recovery.defaults.contains_key(&dref(OTHER)));
}

#[test]
fn restart_keeps_the_selected_default_not_install_order() {
    let base = base();
    let mut store = two_versions(&base);
    // Install order would favour 2.0.0; the host chose the older version.
    store
        .select_default(dref(NODE), provider("1.0.0", HEX_1))
        .unwrap();
    let restored = restart(&store, Arc::clone(&base), both()).unwrap();
    assert_eq!(default_of(&restored), Some(provider("1.0.0", HEX_1)));
    // A second restart from the restored journal keeps it.
    let again = restart(&restored, base, both()).unwrap();
    assert_eq!(default_of(&again), Some(provider("1.0.0", HEX_1)));
}

#[test]
fn no_selection_stays_unselected_after_restart() {
    let base = base();
    let store = two_versions(&base);
    assert_eq!(default_of(&store), None);
    assert_eq!(default_of(&restart(&store, base, both()).unwrap()), None);
}

#[test]
fn update_retargeting_rollback_and_remove_round_trip() {
    let base = base();
    let mut store = PackageStore::new(Arc::clone(&base), HOST, MemoryJournal::new());
    store.install(device("1.0.0", HEX_1)).unwrap();
    store
        .select_default(dref(NODE), provider("1.0.0", HEX_1))
        .unwrap();
    store
        .update(&pkg(PACKAGE, "1.0.0"), device("2.0.0", HEX_2))
        .unwrap();
    assert_eq!(default_of(&store), Some(provider("2.0.0", HEX_2)));
    let restored = restart(&store, Arc::clone(&base), both()).unwrap();
    assert_eq!(default_of(&restored), Some(provider("2.0.0", HEX_2)));

    store.rollback().unwrap();
    assert_eq!(default_of(&store), Some(provider("1.0.0", HEX_1)));
    let restored = restart(&store, Arc::clone(&base), vec![device("1.0.0", HEX_1)]).unwrap();
    assert_eq!(default_of(&restored), Some(provider("1.0.0", HEX_1)));

    store
        .update(&pkg(PACKAGE, "1.0.0"), device("2.0.0", HEX_2))
        .unwrap();
    store.remove(&pkg(PACKAGE, "2.0.0")).unwrap();
    assert_eq!(default_of(&store), None);
    let restored = restart(&store, base, vec![device("1.0.0", HEX_1)]).unwrap();
    assert_eq!(default_of(&restored), None);
}

#[test]
fn base_registry_defaults_survive_and_can_be_overridden() {
    let (base, base_provider) = base_with_default();
    let mut store = PackageStore::new(Arc::clone(&base), HOST, MemoryJournal::new());
    store.install(device("1.0.0", HEX_1)).unwrap();
    assert_eq!(default_of(&store), Some(base_provider.clone()));
    let restored = restart(&store, Arc::clone(&base), vec![device("1.0.0", HEX_1)]).unwrap();
    assert_eq!(default_of(&restored), Some(base_provider));

    store
        .select_default(dref(NODE), provider("1.0.0", HEX_1))
        .unwrap();
    let restored = restart(&store, base, vec![device("1.0.0", HEX_1)]).unwrap();
    assert_eq!(default_of(&restored), Some(provider("1.0.0", HEX_1)));
}

#[test]
fn an_unavailable_selected_provider_is_an_explicit_recovery_error() {
    let base = base();
    let mut store = two_versions(&base);
    store
        .select_default(dref(NODE), provider("2.0.0", HEX_2))
        .unwrap();
    let mut recovery = Recovery::from_entries(store.journal().entries());
    let missing = provider("3.0.0", HEX_3);
    recovery.defaults.insert(dref(NODE), Some(missing.clone()));
    let (journal, shared) = Flaky::over(store.journal().entries(), None);
    let before = shared.borrow().len();
    let error = PackageStore::restore(base, HOST, journal, &recovery, both()).unwrap_err();
    assert_eq!(
        error,
        TransactionError::RecoveryDefaultUnavailable {
            definition: dref(NODE),
            provider: Box::new(missing),
        }
    );
    assert_eq!(shared.borrow().len(), before, "nothing was journaled");
}

#[test]
fn saved_locks_do_not_follow_the_new_binding_default() {
    let base = base();
    let mut store = two_versions(&base);
    store
        .select_default(dref(NODE), provider("2.0.0", HEX_2))
        .unwrap();
    let restored = restart(&store, base, both()).unwrap();
    let sig = format!("sha256:{}", "a".repeat(64));
    let document: GraphDocument = serde_json::from_value(json!({
        "format_version": 1,
        "graph_id": "20000000-0000-4000-8000-000000000001",
        "revision": 1,
        "nodes": [{
            "id": "10000000-0000-4000-8000-000000000001",
            "definition_id": NODE, "definition_version": 1,
            "config_version": 1, "config": {}, "signature": sig,
            "provider": {"package": PACKAGE, "version": "1.0.0", "id": "impl"}
        }],
        "edges": [],
        "package_lock": [{"id": PACKAGE, "version": "1.0.0",
                          "content_digest": format!("sha256:{HEX_1}")}],
        "signatures": {sig: {
            "kind":"pure",
            "ports":[{"port_id":"value","direction":"output","kind":"data",
                      "type":"core/i64@1","source":"computed"}]
        }}
    }))
    .unwrap();
    assert!(
        validate_nodes(&document, restored.current())
            .diagnostics
            .is_empty()
    );
    assert_eq!(default_of(&restored), Some(provider("2.0.0", HEX_2)));
}
