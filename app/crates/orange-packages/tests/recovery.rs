//! Restart recovery: restoring a store publishes one complete checkpoint or nothing, and
//! keeps the recovered identity state (04 "Hot update/removal").

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use orange_model::catalog::{Determinism, NodeDefinition, Ports, ProviderKey, RegistrySnapshot};
use orange_model::document::{DefinitionRef, NodeKind};
use orange_model::validation::HostOrigin;
use orange_packages::{
    HOST_API_VERSION, HostApiVersion, Journal, JournalEntry, JournalError, MemoryJournal,
    PackageContents, PackageKey, PackageManifest, PackageStore, Recovery, TransactionError,
};
use serde_json::{Value, json};

const PACKAGE: &str = "org.example.device";
const NODE: &str = "org.example.device/value";
const HEX_1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const HEX_2: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const HEX_3: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const HOST: HostApiVersion = HOST_API_VERSION;

fn origin() -> HostOrigin {
    HostOrigin::new("0.1.0", format!("sha256:{}", "c".repeat(64))).unwrap()
}

fn base() -> Arc<RegistrySnapshot> {
    RegistrySnapshot::core(origin())
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

fn manifest(value: &Value) -> PackageManifest {
    PackageManifest::from_json(&serde_json::to_vec(value).unwrap()).unwrap()
}

fn device(version: &str, digest: &str) -> PackageContents {
    PackageContents::new(
        manifest(&json!({
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
        })),
        vec![definition(NODE)],
    )
}

/// A package without contributions; `needs` lists exact `device` versions it depends on.
fn app(id: &str, digest: &str, needs: &[&str]) -> PackageContents {
    let dependencies: Vec<Value> = needs
        .iter()
        .map(|version| json!({"id": PACKAGE, "version": version}))
        .collect();
    PackageContents::new(
        manifest(&json!({
            "format": "orange.plugin", "format_version": 1,
            "id": id, "version": "1.0.0", "publisher": "org.example",
            "host_api": {"major": 1, "min_minor": 0, "max_minor": 0},
            "dependencies": dependencies,
            "integrity": {"algorithm": "sha256", "payload_digest": digest}
        })),
        vec![],
    )
}

fn pkg(id: &str, version: &str) -> PackageKey {
    PackageKey {
        id: id.parse().unwrap(),
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

/// A journal whose shared entries survive the store and that fails its `fail_at`th append.
struct Flaky {
    entries: Rc<RefCell<Vec<JournalEntry>>>,
    fail_at: Option<usize>,
    appends: usize,
}

impl Flaky {
    fn over(
        entries: &[JournalEntry],
        fail_at: Option<usize>,
    ) -> (Self, Rc<RefCell<Vec<JournalEntry>>>) {
        let shared = Rc::new(RefCell::new(entries.to_vec()));
        let journal = Self {
            entries: Rc::clone(&shared),
            fail_at,
            appends: 0,
        };
        (journal, shared)
    }
}

impl Journal for Flaky {
    fn append(&mut self, entry: JournalEntry) -> Result<(), JournalError> {
        let index = self.appends;
        self.appends += 1;
        if self.fail_at == Some(index) {
            return Err(JournalError {
                reason: "injected".into(),
            });
        }
        self.entries.borrow_mut().push(entry);
        Ok(())
    }
}

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
}
