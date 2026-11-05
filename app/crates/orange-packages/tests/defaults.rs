//! New-binding provider defaults are part of the recovery checkpoint (12 §8): a restart
//! reproduces the exact selection, never an install-order guess, and never rebinds saved
//! document locks.

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use common::*;
use orange_model::GraphDocument;
use orange_model::catalog::{ProviderDescriptor, ProviderKey, RegistryBuilder, RegistrySnapshot};
use orange_model::validation::validate_nodes;
use orange_packages::{MemoryJournal, PackageContents, PackageStore, Recovery, TransactionError};
use serde_json::json;

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
    let base_provider = ProviderKey {
        package: "org.base".into(),
        version: "1.0.0".into(),
        content_digest: format!("sha256:{}", "b".repeat(64)),
        id: "impl".into(),
    };
    let base = RegistryBuilder::from_snapshot(&RegistrySnapshot::core(origin()))
        .definition(definition(NODE))
        .provider(ProviderDescriptor {
            key: base_provider.clone(),
            implements: BTreeSet::from([(dref(NODE), 1)]),
            specializations: Default::default(),
        })
        .new_binding_default(dref(NODE), base_provider.clone())
        .build()
        .unwrap();
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
    recovery
        .defaults
        .as_mut()
        .unwrap()
        .insert(dref(NODE), missing.clone());
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
fn legacy_checkpoints_without_defaults_apply_none() {
    let base = base();
    let mut store = two_versions(&base);
    store
        .select_default(dref(NODE), provider("2.0.0", HEX_2))
        .unwrap();
    let mut recovery = Recovery::from_entries(store.journal().entries());
    recovery.defaults = None;
    let restored =
        PackageStore::restore(base, HOST, MemoryJournal::new(), &recovery, both()).unwrap();
    assert_eq!(default_of(&restored), None);
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
