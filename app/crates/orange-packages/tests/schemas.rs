//! Enforced manifest sections (K01): execution, panels, localization (13 §3), strict JSON and
//! exact dependency digest locks. One test per rule family.

use orange_packages::{
    ArgumentType, Execution, ManifestError, MemoryJournal, PackageManifest, PackageStore, PathRule,
    TargetArch, TargetOs, TransactionError,
};
use serde_json::{Value, json};

use crate::common::{HEX_1, HEX_2, HEX_3, HOST, PACKAGE, app, base, device, pkg};

fn manifest() -> Value {
    json!({
        "format": "orange.plugin", "format_version": 1,
        "id": PACKAGE, "version": "1.0.0", "publisher": "org.example",
        "host_api": {"major": 1, "min_minor": 0, "max_minor": 0},
        "integrity": {"algorithm": "sha256", "content_digest": HEX_1}
    })
}

fn parse(value: &Value) -> Result<PackageManifest, ManifestError> {
    PackageManifest::from_json(&serde_json::to_vec(value).unwrap())
}

fn with(edit: impl FnOnce(&mut Value)) -> Result<PackageManifest, ManifestError> {
    let mut value = manifest();
    edit(&mut value);
    parse(&value)
}

fn rejected(edit: impl FnOnce(&mut Value)) -> ManifestError {
    with(edit).unwrap_err()
}

fn localization() -> Value {
    json!({
        "format_version": 1, "format": "fluent", "default_locale": "en",
        "catalogs": {"en": ["locales/en.ftl"], "es": ["locales/es.ftl"]},
        "arguments": {"selection-count": {"count": "u64"}}
    })
}

#[test]
fn a_data_only_package_has_no_sections() {
    let manifest = parse(&manifest()).unwrap();
    assert!(manifest.execution().is_none());
    assert!(manifest.localization().is_none());
    assert!(manifest.panels().is_empty());
    assert_eq!(manifest.referenced_files().count(), 0);
}

#[test]
fn component_execution_names_an_archive_entry_and_an_orange_world() {
    let manifest = with(|v| {
        v["execution"] = json!({"kind": "wasm-component", "entry": "bin/component.wasm",
                                "world": "orange:expansion/plugin@1.0.0"});
    })
    .unwrap();
    let Some(Execution::WasmComponent { entry, world }) = manifest.execution() else {
        panic!("expected a component");
    };
    assert_eq!(entry.as_str(), "bin/component.wasm");
    assert_eq!(world.as_str(), "orange:expansion/plugin@1.0.0");
    assert_eq!(manifest.referenced_files().count(), 1);

    let section = |edit: &dyn Fn(&mut Value)| {
        rejected(|v| {
            let mut execution = json!({"kind": "wasm-component", "entry": "c.wasm",
                                       "world": "orange:expansion/plugin@1.0.0"});
            edit(&mut execution);
            v["execution"] = execution;
        })
    };
    // Native libraries and unknown kinds are refused, not ignored.
    for kind in ["native-library", "dylib", ""] {
        assert!(matches!(
            section(&|e| e["kind"] = json!(kind)),
            ManifestError::ExecutionKind(_)
        ));
    }
    for world in [
        "wasi:cli/command@0.2.0",
        "orange:expansion/plugin",
        "orange:expansion/Plugin@1.0.0",
        "orange:expansion/plugin@1.0",
        "orange:expansion/plugin@01.0.0",
    ] {
        assert!(
            matches!(
                section(&|e| e["world"] = json!(world)),
                ManifestError::ComponentWorld(_)
            ),
            "{world}"
        );
    }
    let ManifestError::Path(error) = section(&|e| e["entry"] = json!("../c.wasm")) else {
        panic!("expected a path error");
    };
    assert_eq!(error.reason, PathRule::DotComponent);
    assert!(matches!(
        section(&|e| e["fuel"] = json!(1)),
        ManifestError::Decode { .. }
    ));
}

#[test]
fn worker_execution_lists_distinct_targets() {
    let worker = |targets: Value| {
        with(|v| v["execution"] = json!({"kind": "worker", "wire_version": 1, "targets": targets}))
    };
    let manifest = worker(json!([
        {"os": "linux", "arch": "x86_64", "entry": "bin/linux/worker"},
        {"os": "windows", "arch": "aarch64", "entry": "bin/win/worker.exe"}
    ]))
    .unwrap();
    let Some(Execution::Worker { targets, .. }) = manifest.execution() else {
        panic!("expected a worker");
    };
    // Sorted by target, so the order written does not matter.
    assert_eq!(
        targets.iter().map(|t| (t.os, t.arch)).collect::<Vec<_>>(),
        [
            (TargetOs::Windows, TargetArch::Aarch64),
            (TargetOs::Linux, TargetArch::X86_64)
        ]
    );

    assert_eq!(worker(json!([])).unwrap_err(), ManifestError::WorkerTargets);
    assert_eq!(
        worker(json!([
            {"os": "macos", "arch": "aarch64", "entry": "a"},
            {"os": "macos", "arch": "aarch64", "entry": "b"}
        ]))
        .unwrap_err(),
        ManifestError::DuplicateWorkerTarget {
            os: TargetOs::Macos,
            arch: TargetArch::Aarch64
        }
    );
    assert!(matches!(
        worker(json!([{"os": "freebsd", "arch": "x86_64", "entry": "a"}])).unwrap_err(),
        ManifestError::Decode { .. }
    ));
    assert_eq!(
        rejected(|v| v["execution"] = json!({"kind": "worker", "wire_version": 2, "targets": []})),
        ManifestError::WorkerWireVersion(2)
    );
}

#[test]
fn panels_are_unique_ids_in_the_package_namespace() {
    let manifest = with(|v| {
        v["contributions"] =
            json!({"panels": ["org.example.device/session", "org.example.device/log"]});
    })
    .unwrap();
    assert_eq!(manifest.panels()[1].as_str(), "org.example.device/log");

    let panels = |panels: Value| rejected(|v| v["contributions"] = json!({"panels": panels}));
    assert_eq!(
        panels(json!(["org.other/session"])),
        ManifestError::PanelNamespace {
            panel: "org.other/session".into(),
            package: PACKAGE.into()
        }
    );
    for bad in [
        "session",
        "org.example.device/Session",
        "org.example.device/a/b",
        "/x",
    ] {
        assert!(
            matches!(panels(json!([bad])), ManifestError::PanelId(_)),
            "{bad}"
        );
    }
    assert_eq!(
        panels(json!(["org.example.device/a", "org.example.device/a"])),
        ManifestError::DuplicatePanel("org.example.device/a".into())
    );
    let many: Vec<String> = (0..=orange_packages::MAX_PANELS)
        .map(|index| format!("org.example.device/p{index}"))
        .collect();
    assert!(matches!(
        panels(json!(many)),
        ManifestError::SectionLimit {
            section: "contributions.panels",
            ..
        }
    ));
    // Panels are IDs, not inline widget objects.
    assert!(matches!(
        panels(json!([{"id": "org.example.device/a"}])),
        ManifestError::Decode { .. }
    ));
}

#[test]
fn localization_follows_the_13_3_contract() {
    let manifest = with(|v| v["localization"] = localization()).unwrap();
    let section = manifest.localization().unwrap();
    assert_eq!(section.default_locale(), "en");
    assert_eq!(section.catalogs().len(), 2);
    assert_eq!(
        section.arguments()["selection-count"]["count"],
        ArgumentType::U64
    );
    assert_eq!(manifest.referenced_files().count(), 2);

    let edit = |change: &dyn Fn(&mut Value)| {
        rejected(|v| {
            let mut section = localization();
            change(&mut section);
            v["localization"] = section;
        })
    };
    assert_eq!(
        edit(&|l| l["format_version"] = json!(2)),
        ManifestError::LocalizationFormatVersion(2)
    );
    assert!(matches!(
        edit(&|l| l["format"] = json!("icu")),
        ManifestError::LocalizationFormat(_)
    ));
    assert!(matches!(
        edit(&|l| l["default_locale"] = json!("es")),
        ManifestError::DefaultLocale(_)
    ));
    assert_eq!(
        edit(&|l| l["catalogs"] = json!({"es": ["es.ftl"]})),
        ManifestError::MissingDefaultCatalog
    );
    // Tags match ignoring case.
    assert!(matches!(
        edit(&|l| l["catalogs"]["EN"] = json!(["en2.ftl"])),
        ManifestError::DuplicateLocale(_)
    ));
    for tag in [
        "e",
        "englishlanguage",
        "en-toolongsubtag",
        "en_US",
        "en--US",
        "1en",
    ] {
        assert!(
            matches!(
                edit(&|l| l["catalogs"][tag] = json!(["x.ftl"])),
                ManifestError::LocaleTag(_)
            ),
            "{tag}"
        );
    }
    assert!(matches!(
        edit(&|l| l["catalogs"]["fr"] = json!([])),
        ManifestError::EmptyCatalog(_)
    ));
    assert!(matches!(
        edit(&|l| l["catalogs"]["fr"] = json!(["fr.po"])),
        ManifestError::CatalogFile(_)
    ));
    // One file cannot serve two locales, even under another case.
    assert!(matches!(
        edit(&|l| l["catalogs"]["fr"] = json!(["Locales/EN.ftl"])),
        ManifestError::DuplicateCatalogFile(_)
    ));
    assert!(matches!(
        edit(&|l| l["catalogs"]["fr"] = json!(["C:/fr.ftl"])),
        ManifestError::Path(_)
    ));
    assert!(matches!(
        edit(&|l| l["arguments"] = json!({"Bad": {}})),
        ManifestError::MessageId(_)
    ));
    assert!(matches!(
        edit(&|l| l["arguments"] = json!({"m": {"$count": "u64"}})),
        ManifestError::ArgumentName(_)
    ));
    assert!(matches!(
        edit(&|l| l["arguments"] = json!({"m": {"count": "u128"}})),
        ManifestError::ArgumentType(_)
    ));
    assert!(matches!(
        edit(&|l| l["bundles"] = json!({})),
        ManifestError::Decode { .. }
    ));
}

#[test]
fn manifests_are_strict_json() {
    let text = serde_json::to_string(&manifest()).unwrap();
    let repeated = text.replacen('{', r#"{"license":"MIT","license":"GPL-3.0","#, 1);
    assert!(matches!(
        PackageManifest::from_json(repeated.as_bytes()).unwrap_err(),
        ManifestError::DuplicateKey(_)
    ));
    // Opaque sections are strict too: readers must agree on every byte the digest covers.
    assert_eq!(
        rejected(|v| v["permissions"] = json!([{"quota": 1.5}])),
        ManifestError::Number
    );
    assert_eq!(
        rejected(|v| v["permissions"] = json!([{"quota": u64::MAX}])),
        ManifestError::Number
    );
}

#[test]
fn dependency_locks_carry_an_exact_digest() {
    let dependency = |lock: Value| rejected(|v| v["dependencies"] = json!([lock]));
    assert!(matches!(
        dependency(json!({"id": "org.example.base", "version": "1.0.0"})),
        ManifestError::Decode { .. }
    ));
    for digest in [
        HEX_1.to_owned(),
        format!("sha512:{HEX_1}"),
        format!("sha256:{}", "A".repeat(64)),
    ] {
        assert!(matches!(
            dependency(
                json!({"id": "org.example.base", "version": "1.0.0", "content_digest": digest})
            ),
            ManifestError::DependencyDigest(_)
        ));
    }
}

#[test]
fn install_refuses_a_dependency_with_another_digest() {
    let mut store = PackageStore::new(base(), HOST, MemoryJournal::new());
    store.install(device("2.0.0", HEX_2)).unwrap();
    let before = store.current().clone();
    // The lock names the 2.0.0 bytes as HEX_1, but HEX_2 is active.
    let error = store
        .install(app("org.example.app", HEX_3, &[("2.0.0", HEX_1)]))
        .unwrap_err();
    assert_eq!(
        error,
        TransactionError::DependencyDigest(pkg(PACKAGE, "2.0.0"))
    );
    assert_eq!(
        error.message_key(),
        "packages/transaction-dependency-digest"
    );
    assert!(std::sync::Arc::ptr_eq(store.current(), &before));
    store
        .install(app("org.example.app", HEX_3, &[("2.0.0", HEX_2)]))
        .unwrap();
}
