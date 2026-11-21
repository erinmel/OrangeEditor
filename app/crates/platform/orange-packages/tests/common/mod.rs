//! Shared package fixtures for the recovery tests.
#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use orange_model::catalog::{Determinism, NodeDefinition, Ports, ProviderKey, RegistrySnapshot};
use orange_model::document::{DefinitionRef, NodeKind};
use orange_model::validation::HostOrigin;
use orange_packages::{
    HOST_API_VERSION, HostApiVersion, Journal, JournalEntry, JournalError, PackageContents,
    PackageKey, PackageManifest,
};
use serde_json::{Value, json};

pub const PACKAGE: &str = "org.example.device";
pub const NODE: &str = "org.example.device/value";
pub const HEX_1: &str = "1111111111111111111111111111111111111111111111111111111111111111";
pub const HEX_2: &str = "2222222222222222222222222222222222222222222222222222222222222222";
pub const HEX_3: &str = "3333333333333333333333333333333333333333333333333333333333333333";
pub const HOST: HostApiVersion = HOST_API_VERSION;

pub fn origin() -> HostOrigin {
    HostOrigin::new("0.1.0", format!("sha256:{}", "c".repeat(64))).unwrap()
}

pub fn base() -> Arc<RegistrySnapshot> {
    RegistrySnapshot::core(origin())
}

pub fn dref(id: &str) -> DefinitionRef {
    DefinitionRef {
        id: id.parse().unwrap(),
        version: 1,
    }
}

pub fn definition(id: &str) -> NodeDefinition {
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

pub fn manifest(value: &Value) -> PackageManifest {
    PackageManifest::from_json(&serde_json::to_vec(value).unwrap()).unwrap()
}

pub fn device(version: &str, digest: &str) -> PackageContents {
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
            "integrity": {"algorithm": "sha256", "content_digest": digest},
            "license": "MIT"
        })),
        vec![definition(NODE)],
    )
}

/// A package without contributions; `needs` lists the exact `device` versions it depends on
/// with their digests (hex).
pub fn app(id: &str, digest: &str, needs: &[(&str, &str)]) -> PackageContents {
    let dependencies: Vec<Value> = needs
        .iter()
        .map(|(version, hex)| json!({"id": PACKAGE, "version": version, "content_digest": format!("sha256:{hex}")}))
        .collect();
    PackageContents::new(
        manifest(&json!({
            "format": "orange.plugin", "format_version": 1,
            "id": id, "version": "1.0.0", "publisher": "org.example",
            "host_api": {"major": 1, "min_minor": 0, "max_minor": 0},
            "dependencies": dependencies,
            "integrity": {"algorithm": "sha256", "content_digest": digest}
        })),
        vec![],
    )
}

pub fn pkg(id: &str, version: &str) -> PackageKey {
    PackageKey {
        id: id.parse().unwrap(),
        version: version.parse().unwrap(),
    }
}

pub fn provider(version: &str, hex: &str) -> ProviderKey {
    ProviderKey {
        package: PACKAGE.into(),
        version: version.into(),
        content_digest: format!("sha256:{hex}"),
        id: "impl".into(),
    }
}

/// A journal whose shared entries survive the store and that fails its `fail_at`th append.
#[derive(Debug)]
pub struct Flaky {
    pub entries: Rc<RefCell<Vec<JournalEntry>>>,
    fail_at: Option<usize>,
    appends: usize,
}

impl Flaky {
    pub fn over(
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

/// A fresh directory under the system temporary directory, deleted on drop.
#[derive(Debug)]
pub struct TempDir(std::path::PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "orange-packages-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
