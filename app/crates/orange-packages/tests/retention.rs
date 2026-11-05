//! Bounded rollback history and journal compaction: retirement frees only the store's own
//! references and never identity state (04 "Hot update/removal").

mod common;

use std::sync::{Arc, Weak};

use common::*;
use orange_model::catalog::RegistrySnapshot;
use orange_packages::{MemoryJournal, PackageStore, Recovery, RetentionPolicy, TransactionError};

fn bounded(window: usize) -> PackageStore {
    let mut store =
        PackageStore::new(base(), HOST, MemoryJournal::new()).with_retention(RetentionPolicy {
            max_rollback_generations: window,
        });
    store.install(device("1.0.0", HEX_1)).unwrap();
    store.install(device("2.0.0", HEX_2)).unwrap();
    store
}

/// Alternates the default between the two versions, so every call really changes it.
fn flip(store: &mut PackageStore, times: usize) {
    for turn in 0..times {
        let chosen = if turn % 2 == 0 {
            provider("1.0.0", HEX_1)
        } else {
            provider("2.0.0", HEX_2)
        };
        store.select_default(dref(NODE), chosen).unwrap();
    }
}

#[test]
fn evicted_generations_drop_while_an_external_snapshot_stays_usable() {
    let mut store = bounded(4);
    let pinned = Arc::clone(store.current());
    let mut seen: Vec<Weak<RegistrySnapshot>> = Vec::new();
    for turn in 0..64 {
        seen.push(Arc::downgrade(store.current()));
        flip(&mut store, 1 + turn % 2);
    }
    let alive = seen.iter().filter(|weak| weak.upgrade().is_some()).count();
    // The window plus the pinned snapshot; never the whole history.
    assert!(alive <= 4 + 1, "{alive} generations are still held");
    assert!(pinned.provider(&provider("2.0.0", HEX_2)).is_some());
    assert!(pinned.provides(&dref(NODE), 1, &provider("1.0.0", HEX_1)));
    drop(store);
    drop(pinned);
    assert!(seen.iter().all(|weak| weak.upgrade().is_none()));
}

#[test]
fn rollback_works_inside_the_window_and_is_refused_outside_it() {
    let mut store = bounded(3);
    // Two installs, then five real default changes; only the last three are reachable.
    flip(&mut store, 5);
    for _ in 0..3 {
        store.rollback().unwrap();
    }
    assert!(!store.can_roll_back());
    assert_eq!(
        store.rollback().unwrap_err(),
        TransactionError::RollbackWindowExceeded
    );
    // A store that never had history is a different, plain refusal.
    let mut fresh = PackageStore::new(base(), HOST, MemoryJournal::new());
    assert_eq!(
        fresh.rollback().unwrap_err(),
        TransactionError::NothingToRollBack
    );
    // No rollback at all is a valid policy.
    let mut none = bounded(0);
    assert!(!none.can_roll_back());
    flip(&mut none, 2);
    assert_eq!(
        none.rollback().unwrap_err(),
        TransactionError::RollbackWindowExceeded
    );
}

#[test]
fn reselecting_the_current_default_creates_no_generation() {
    let mut store = bounded(8);
    let chosen = provider("1.0.0", HEX_1);
    let first = store.select_default(dref(NODE), chosen.clone()).unwrap();
    let appended = store.journal().entries().len();
    for _ in 0..20 {
        let again = store.select_default(dref(NODE), chosen.clone()).unwrap();
        assert!(Arc::ptr_eq(&first, &again));
    }
    assert_eq!(store.journal().entries().len(), appended);
    // One rollback undoes the only real selection, not a no-op copy of it.
    let back = store.rollback().unwrap();
    assert_eq!(back.new_binding_default(&dref(NODE)), None);
}

#[test]
fn shrinking_the_policy_drops_history_immediately() {
    let mut store = bounded(8);
    flip(&mut store, 4);
    store.set_retention(RetentionPolicy {
        max_rollback_generations: 1,
    });
    store.rollback().unwrap();
    assert_eq!(
        store.rollback().unwrap_err(),
        TransactionError::RollbackWindowExceeded
    );
}

#[test]
fn identity_and_revocations_survive_retirement_and_compaction() {
    let mut store = bounded(1);
    let doomed = app("org.example.doomed", &"4".repeat(64), &[]);
    store.install(doomed.clone()).unwrap();
    store.revoke(doomed.manifest().key()).unwrap();
    flip(&mut store, 10);
    store
        .select_default(dref(NODE), provider("2.0.0", HEX_2))
        .unwrap();
    store.remove(&pkg(PACKAGE, "1.0.0")).unwrap();

    let check = |store: &mut PackageStore| {
        assert_eq!(
            store.install(device("1.0.0", HEX_2)).unwrap_err(),
            TransactionError::DigestConflict(pkg(PACKAGE, "1.0.0"))
        );
        assert_eq!(
            store.install(doomed.clone()).unwrap_err(),
            TransactionError::Revoked(doomed.manifest().key().clone())
        );
    };
    check(&mut store);

    let before = Recovery::from_entries(store.journal().entries());
    let entries = store.journal().entries().len();
    store.compact_journal().unwrap();
    assert_eq!(store.journal().entries().len(), 1);
    assert!(entries > 1);
    let after = Recovery::from_entries(store.journal().entries());
    assert_eq!(after.committed, before.committed);
    assert_eq!(after.published, before.published);
    assert_eq!(after.revoked, before.revoked);
    assert_eq!(after.removed, before.removed);
    assert_eq!(after.defaults, before.defaults);

    // Restart from the compacted journal keeps the same state and the same refusals.
    let mut restored = PackageStore::restore(
        base(),
        HOST,
        MemoryJournal::new(),
        &after,
        vec![device("2.0.0", HEX_2)],
    )
    .unwrap();
    assert_eq!(
        restored.current().new_binding_default(&dref(NODE)),
        Some(&provider("2.0.0", HEX_2))
    );
    check(&mut restored);
}
