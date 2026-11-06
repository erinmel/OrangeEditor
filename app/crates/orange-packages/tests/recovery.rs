//! Restart recovery: restoring a store publishes one complete checkpoint or nothing, and
//! keeps the recovered identity state (04 "Hot update/removal").

use crate::common;

use common::*;
use orange_packages::{
    JournalEntry, MemoryJournal, PackageContents, PackageKey, PackageStore, Recovery,
    TransactionError,
};

/// A store with `device` 1.0.0 and 2.0.0, an `app` that needs 2.0.0, a revoked third
/// package and a removed fourth, as its journal ends up.
fn original() -> (Vec<JournalEntry>, Vec<PackageContents>) {
    let mut store = PackageStore::new(base(), HOST, MemoryJournal::new());
    store.install(device("1.0.0", HEX_1)).unwrap();
    store.install(device("2.0.0", HEX_2)).unwrap();
    store
        .install(app("org.example.app", HEX_3, &["2.0.0"]))
        .unwrap();
    let doomed = app("org.example.doomed", &"4".repeat(64), &[]);
    store.install(doomed.clone()).unwrap();
    store.revoke(doomed.manifest().key()).unwrap();
    store.remove(&pkg(PACKAGE, "1.0.0")).unwrap();
    let live = vec![
        device("2.0.0", HEX_2),
        app("org.example.app", HEX_3, &["2.0.0"]),
    ];
    (store.journal().entries().to_vec(), live)
}

#[test]
fn failed_restore_leaves_the_prior_commit_authoritative_at_every_append() {
    let (entries, live) = original();
    let before = Recovery::from_entries(&entries);
    assert_eq!(before.committed.as_ref().unwrap().len(), 2);

    // Dependents first: restore must still install the dependency before them.
    let reversed: Vec<PackageContents> = live.iter().rev().cloned().collect();

    // Restore performs exactly one durable append; fail it, and also let it succeed.
    for fail_at in [Some(0), None] {
        let (journal, shared) = Flaky::over(&entries, fail_at);
        let result = PackageStore::restore(base(), HOST, journal, &before, reversed.clone());
        let after = Recovery::from_entries(&shared.borrow());
        match fail_at {
            Some(_) => {
                assert!(matches!(result, Err(TransactionError::Journal(_))));
                assert_eq!(shared.borrow().len(), entries.len(), "nothing was appended");
            }
            None => {
                let restored = result.unwrap();
                assert_eq!(shared.borrow().len(), entries.len() + 1, "one checkpoint");
                assert!(matches!(
                    shared.borrow().last(),
                    Some(JournalEntry::Committed { .. })
                ));
                assert!(restored.package(&pkg("org.example.app", "1.0.0")).is_some());
            }
        }
        // Either way the next restart sees the same complete set, digests and revocations.
        assert_eq!(after.committed, before.committed);
        assert_eq!(after.published, before.published);
        assert_eq!(after.revoked, before.revoked);
        assert_eq!(after.removed, before.removed);
        assert_eq!(after.defaults, before.defaults);
    }
}

#[test]
fn restore_that_fails_validation_writes_nothing() {
    let (entries, live) = original();
    let recovery = Recovery::from_entries(&entries);

    // The dependent without its dependency, a digest mismatch, and a version never committed.
    let cases: Vec<Vec<PackageContents>> = vec![
        vec![live[1].clone(), device("2.0.0", HEX_3)],
        vec![live[0].clone(), live[1].clone(), device("1.0.0", HEX_1)],
        vec![live[1].clone()],
    ];
    for contents in cases {
        let (journal, shared) = Flaky::over(&entries, None);
        assert!(PackageStore::restore(base(), HOST, journal, &recovery, contents).is_err());
        assert_eq!(
            *shared.borrow(),
            entries,
            "no transition or checkpoint was written"
        );
        assert_eq!(Recovery::from_entries(&shared.borrow()), recovery);
    }
}

#[test]
fn successful_restore_publishes_the_complete_candidate_and_keeps_identity() {
    let (entries, live) = original();
    let recovery = Recovery::from_entries(&entries);
    let mut restored =
        PackageStore::restore(base(), HOST, MemoryJournal::new(), &recovery, live).unwrap();

    let active: Vec<PackageKey> = restored
        .packages()
        .filter(|record| record.state().is_active())
        .map(|record| record.manifest().key().clone())
        .collect();
    assert_eq!(active, recovery.committed.clone().unwrap());
    assert!(
        restored
            .current()
            .provides(&dref(NODE), 1, &provider("2.0.0", HEX_2))
    );
    assert_eq!(
        restored.published_digest(&pkg(PACKAGE, "1.0.0")),
        Some(format!("sha256:{HEX_1}").as_str())
    );
    assert!(restored.is_revoked(&format!("sha256:{}", "4".repeat(64))));
    assert_eq!(
        restored.install(device("1.0.0", HEX_2)).unwrap_err(),
        TransactionError::DigestConflict(pkg(PACKAGE, "1.0.0"))
    );
    // The checkpoint the restart wrote round-trips to the same recovery.
    let again = Recovery::from_entries(restored.journal().entries());
    assert_eq!(again.committed, recovery.committed);
    assert_eq!(again.published, recovery.published);
    assert_eq!(again.revoked, recovery.revoked);
    assert_eq!(again.defaults, recovery.defaults);
}
