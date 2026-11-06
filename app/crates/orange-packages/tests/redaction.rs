//! Redaction (13 §2): untrusted manifest text never appears in `Display` or `Debug` of
//! package errors or objects; hosts read it through explicit accessors.

use std::fmt::{Debug, Display};

use orange_model::catalog::{Determinism, NodeDefinition, Ports, RegistrySnapshot};
use orange_model::document::{DefinitionRef, NodeKind};
use orange_model::validation::HostOrigin;
use orange_packages::{
    HOST_API_VERSION, ManifestError, MemoryJournal, PackageContents, PackageKey, PackageManifest,
    PackageStore, ProviderSlot, TransactionError,
};
use serde_json::{Value, json};

const CANARY: &str = "CANARY-7f3a";
const PACKAGE: &str = "org.example.device";
const NODE: &str = "org.example.device/value";
const HEX: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const LOCK: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";

fn manifest_json() -> Value {
    json!({
        "format": "orange.plugin", "format_version": 1,
        "id": PACKAGE, "version": "1.0.0", "publisher": "org.example",
        "host_api": {"major": 1, "min_minor": 0, "max_minor": 0},
        "execution": {"kind": "wasm-component", "entry": "component.wasm",
                      "world": "orange:expansion/plugin@1.0.0"},
        "dependencies": [],
        "contributions": {
            "nodes": [NODE],
            "providers": [{"slot": format!("node:{NODE}@1"), "id": "impl"}]
        },
        "permissions": [],
        "integrity": {"algorithm": "sha256", "content_digest": HEX},
        "license": "MIT"
    })
}

fn parse(value: &Value) -> Result<PackageManifest, ManifestError> {
    PackageManifest::from_json(&serde_json::to_vec(value).unwrap())
}

type Case = (&'static str, Box<dyn Fn(&mut Value)>, &'static str);

fn quiet(item: &(impl Debug + ?Sized)) {
    for text in [format!("{item:?}"), format!("{item:#?}")] {
        assert!(!text.contains(CANARY), "Debug leaked: {text}");
    }
}

fn quiet_error(error: &(impl Debug + Display)) {
    assert!(
        !error.to_string().contains(CANARY),
        "Display leaked: {error}"
    );
    quiet(error);
}

/// Applies `edit` to a valid manifest and expects a redacted rejection.
fn rejects(edit: impl FnOnce(&mut Value)) -> ManifestError {
    let mut value = manifest_json();
    edit(&mut value);
    let error = parse(&value).unwrap_err();
    quiet_error(&error);
    assert!(error.message_key().starts_with("packages/manifest-"));
    error
}

#[test]
fn every_rejection_path_hides_the_rejected_text() {
    let reveal = |error: &ManifestError| match error {
        ManifestError::Format(text)
        | ManifestError::PackageId(text)
        | ManifestError::Version(text)
        | ManifestError::Publisher(text)
        | ManifestError::DigestAlgorithm(text)
        | ManifestError::NodeId(text)
        | ManifestError::Slot(text)
        | ManifestError::ProviderId(text)
        | ManifestError::DuplicateKey(text)
        | ManifestError::DependencyDigest(text)
        | ManifestError::ExecutionKind(text)
        | ManifestError::ComponentWorld(text)
        | ManifestError::PanelId(text)
        | ManifestError::LocalizationFormat(text)
        | ManifestError::DefaultLocale(text)
        | ManifestError::LocaleTag(text)
        | ManifestError::DuplicateLocale(text)
        | ManifestError::EmptyCatalog(text)
        | ManifestError::CatalogFile(text)
        | ManifestError::DuplicateCatalogFile(text)
        | ManifestError::MessageId(text)
        | ManifestError::ArgumentName(text)
        | ManifestError::ArgumentType(text) => text.reveal().to_owned(),
        ManifestError::Path(error) => error.path.reveal().to_owned(),
        ManifestError::DuplicateProvider { slot, .. } => slot.reveal().to_owned(),
        other => panic!("no rejected text in {other:?}"),
    };

    let cases: Vec<Case> = vec![
        ("format", Box::new(|v| v["format"] = json!(CANARY)), CANARY),
        ("id", Box::new(|v| v["id"] = json!(CANARY)), CANARY),
        (
            "version",
            Box::new(|v| v["version"] = json!(CANARY)),
            CANARY,
        ),
        (
            "publisher",
            Box::new(|v| v["publisher"] = json!(CANARY)),
            CANARY,
        ),
        (
            "algorithm",
            Box::new(|v| v["integrity"]["algorithm"] = json!(CANARY)),
            CANARY,
        ),
        (
            "dependency id",
            Box::new(|v| {
                v["dependencies"] =
                    json!([{"id": CANARY, "version": "1.0.0", "content_digest": LOCK}])
            }),
            CANARY,
        ),
        (
            "dependency version",
            Box::new(|v| {
                v["dependencies"] =
                    json!([{"id": "org.example.a", "version": CANARY, "content_digest": LOCK}])
            }),
            CANARY,
        ),
        (
            "node id",
            Box::new(|v| v["contributions"]["nodes"] = json!([CANARY])),
            CANARY,
        ),
        (
            "slot without kind",
            Box::new(|v| v["contributions"]["providers"][0]["slot"] = json!(CANARY)),
            CANARY,
        ),
        (
            "unknown slot kind",
            Box::new(|v| {
                v["contributions"]["providers"][0]["slot"] = json!(format!("{CANARY}:x"));
            }),
            "CANARY-7f3a:x",
        ),
        (
            "malformed node slot",
            Box::new(|v| {
                v["contributions"]["providers"][0]["slot"] = json!(format!("node:{CANARY}@1"));
            }),
            "node:CANARY-7f3a@1",
        ),
        (
            "provider id",
            Box::new(|v| v["contributions"]["providers"][0]["id"] = json!(CANARY)),
            CANARY,
        ),
        (
            "duplicate provider",
            Box::new(|v| {
                let slot = json!(format!("convert:{CANARY}"));
                v["contributions"]["providers"] = json!([
                    {"slot": slot, "id": "impl"},
                    {"slot": slot, "id": "impl"}
                ]);
            }),
            "convert:CANARY-7f3a",
        ),
        (
            "dependency digest",
            Box::new(|v| {
                v["dependencies"] = json!([{
                    "id": "org.example.a", "version": "1.0.0", "content_digest": CANARY
                }]);
            }),
            CANARY,
        ),
        (
            "execution kind",
            Box::new(|v| v["execution"]["kind"] = json!(CANARY)),
            CANARY,
        ),
        (
            "component world",
            Box::new(|v| v["execution"]["world"] = json!(CANARY)),
            CANARY,
        ),
        (
            "execution entry",
            Box::new(|v| v["execution"]["entry"] = json!(format!("../{CANARY}"))),
            "../CANARY-7f3a",
        ),
        (
            "panel id",
            Box::new(|v| v["contributions"]["panels"] = json!([CANARY])),
            CANARY,
        ),
        (
            "localization format",
            Box::new(|v| v["localization"] = localization(|l| l["format"] = json!(CANARY))),
            CANARY,
        ),
        (
            "default locale",
            Box::new(|v| {
                v["localization"] = localization(|l| l["default_locale"] = json!(CANARY));
            }),
            CANARY,
        ),
        (
            "locale tag",
            Box::new(|v| {
                v["localization"] =
                    localization(|l| l["catalogs"][format!("{CANARY}_x")] = json!(["x.ftl"]));
            }),
            "CANARY-7f3a_x",
        ),
        (
            "duplicate locale",
            Box::new(|v| {
                v["localization"] = localization(|l| {
                    l["catalogs"][CANARY] = json!(["a.ftl"]);
                    l["catalogs"]["canary-7F3A"] = json!(["b.ftl"]);
                });
            }),
            "canary-7F3A",
        ),
        (
            "empty catalogue",
            Box::new(|v| {
                v["localization"] = localization(|l| l["catalogs"]["canary"] = json!([]));
            }),
            "canary",
        ),
        (
            "catalogue file",
            Box::new(|v| {
                v["localization"] =
                    localization(|l| l["catalogs"]["en"] = json!([format!("{CANARY}.txt")]));
            }),
            "CANARY-7f3a.txt",
        ),
        (
            "duplicate catalogue file",
            Box::new(|v| {
                v["localization"] = localization(|l| {
                    l["catalogs"]["en"] = json!([format!("{CANARY}.ftl"), "canary-7F3A.ftl"]);
                });
            }),
            "canary-7F3A.ftl",
        ),
        (
            "message id",
            Box::new(|v| {
                v["localization"] = localization(|l| l["arguments"] = json!({CANARY: {}}));
            }),
            CANARY,
        ),
        (
            "argument name",
            Box::new(|v| {
                v["localization"] =
                    localization(|l| l["arguments"] = json!({"m": {CANARY: "string"}}));
            }),
            CANARY,
        ),
        (
            "argument type",
            Box::new(|v| {
                v["localization"] = localization(|l| l["arguments"] = json!({"m": {"n": CANARY}}));
            }),
            CANARY,
        ),
    ];
    for (name, edit, expected) in cases {
        let error = rejects(edit);
        assert_eq!(reveal(&error), expected, "reveal for {name}");
    }

    let duplicate =
        format!(r#"{{"format":"orange.plugin","format_version":1,"{CANARY}":1,"{CANARY}":2}}"#);
    let error = PackageManifest::from_json(duplicate.as_bytes()).unwrap_err();
    quiet_error(&error);
    assert_eq!(reveal(&error), CANARY);
}

/// A valid 13 §3 localization section after `edit`.
fn localization(edit: impl FnOnce(&mut Value)) -> Value {
    let mut value = json!({
        "format_version": 1, "format": "fluent", "default_locale": "en",
        "catalogs": {"en": ["locales/en.ftl"]}
    });
    edit(&mut value);
    value
}

#[test]
fn decode_failures_keep_kind_and_position_but_not_decoder_text() {
    let unknown_top = rejects(|v| v[CANARY] = json!(1));
    let unknown_nested = rejects(|v| v["integrity"][CANARY] = json!(1));
    let wrong_shape = rejects(|v| v["host_api"] = json!(CANARY));
    let wrong_nested_shape = rejects(|v| v["contributions"]["nodes"] = json!({CANARY: 1}));
    assert!(matches!(wrong_nested_shape, ManifestError::Decode { .. }));
    for error in [&unknown_top, &unknown_nested, &wrong_shape] {
        let ManifestError::Decode { error } = error else {
            panic!("expected Decode, got {error:?}");
        };
        assert_eq!(error.kind, orange_model::error::JsonErrorKind::Data);
        // The decoder's own message, which names the field, stays available to the host.
        assert!(error.detail.reveal().contains(CANARY));
    }

    for bytes in [
        format!(r#"{{"format": "{CANARY}" "#),
        format!(r#"{{"{CANARY}": }}"#),
        format!(r#"[{CANARY}]"#),
    ] {
        let error = PackageManifest::from_json(bytes.as_bytes()).unwrap_err();
        assert!(matches!(error, ManifestError::Decode { .. }));
        quiet_error(&error);
        assert_eq!(error.message_key(), "packages/manifest-decode");
    }

    let oversized = vec![b' '; orange_packages::MAX_MANIFEST_BYTES + 1];
    let error = PackageManifest::from_json(&oversized).unwrap_err();
    assert_eq!(error.message_key(), "packages/manifest-too-large");
}

/// A valid manifest whose package-supplied paths, locale tags and uninterpreted text all
/// carry the canary.
fn canary_manifest() -> PackageManifest {
    let mut value = manifest_json();
    value["execution"]["entry"] = json!(format!("{CANARY}.wasm"));
    value["permissions"] = json!([CANARY]);
    value["localization"] = localization(|l| {
        l["catalogs"]["en"] = json!([format!("{CANARY}/en.ftl")]);
        l["catalogs"][CANARY] = json!([format!("{CANARY}.ftl")]);
    });
    value["license"] = json!(CANARY);
    let manifest = parse(&value).unwrap();
    quiet(&manifest);
    manifest
}

#[test]
fn package_text_stays_readable_through_accessors_only() {
    let manifest = canary_manifest();
    let entries = manifest.execution().unwrap().entries();
    assert_eq!(entries[0].as_str(), format!("{CANARY}.wasm"));
    assert_eq!(manifest.permissions().unwrap()[0], CANARY);
    assert!(
        manifest
            .localization()
            .unwrap()
            .catalogs()
            .contains_key(CANARY)
    );
    assert_eq!(manifest.license(), Some(CANARY));
    // Identity and section shapes stay readable in the summary.
    let summary = format!("{manifest:?}");
    assert!(summary.contains(PACKAGE) && summary.contains("wasm-component"));
    assert!(summary.contains("locales: 2") && summary.contains("array(1 items)"));
}

fn definition() -> NodeDefinition {
    NodeDefinition {
        definition: DefinitionRef {
            id: NODE.parse().unwrap(),
            version: 1,
        },
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
        config_schema: Some(json!({"description": CANARY})),
    }
}

fn store() -> PackageStore {
    let origin = HostOrigin::new("0.1.0", format!("sha256:{}", "c".repeat(64))).unwrap();
    PackageStore::new(
        RegistrySnapshot::core(origin),
        HOST_API_VERSION,
        MemoryJournal::new(),
    )
}

#[test]
fn contents_records_and_store_format_without_package_text() {
    let manifest = canary_manifest();
    let key: PackageKey = manifest.key().clone();
    let contents = PackageContents::new(manifest, vec![definition()]);
    quiet(&contents);
    assert_eq!(contents.manifest().license(), Some(CANARY));
    assert_eq!(contents.definitions().len(), 1);

    let mut store = store();
    store.install(contents).unwrap();
    quiet(&store);
    let record = store.package(&key).unwrap();
    quiet(record);
    assert_eq!(record.manifest().license(), Some(CANARY));
    assert_eq!(store.packages().count(), 1);
    assert_eq!(record.providers().len(), 1);
    // Summaries still identify what they describe.
    assert!(format!("{store:?}").contains("packages: 1"));
    assert!(format!("{record:?}").contains("Active"));
}

#[test]
fn quarantined_failures_and_transaction_errors_hide_package_text() {
    let canary_slot = |slot: &str| {
        let mut value = manifest_json();
        value["license"] = json!(CANARY);
        value["contributions"]["providers"] = json!([{"slot": slot, "id": "impl"}]);
        PackageContents::new(parse(&value).unwrap(), vec![definition()])
    };
    let mut store = store();
    let contents = canary_slot(&format!("connection:{CANARY}"));
    let key = contents.manifest().key().clone();
    let error = store.install(contents).unwrap_err();
    quiet_error(&error);
    assert_eq!(error.message_key(), "packages/transaction-unsupported-slot");
    let TransactionError::UnsupportedSlot { descriptor, .. } = &error else {
        panic!("expected UnsupportedSlot, got {error:?}");
    };
    assert_eq!(descriptor.reveal(), CANARY);

    // The quarantine entry and the store holding it are quiet too.
    quiet_error(store.quarantined(&key).unwrap());
    quiet(&store);
    assert!(format!("{store:?}").contains("packages/transaction-unsupported-slot"));

    let specialized = PackageContents::new(parse(&manifest_json()).unwrap(), vec![definition()])
        .with_specializations(
            CANARY,
            DefinitionRef {
                id: NODE.parse().unwrap(),
                version: 1,
            },
            vec![],
        );
    let error = store.install(specialized).unwrap_err();
    quiet_error(&error);
    assert_eq!(
        error.message_key(),
        "packages/transaction-undeclared-specialization"
    );
    quiet(&store);
}

#[test]
fn provider_slot_debug_hides_the_descriptor_but_fields_stay_readable() {
    let slot: ProviderSlot = format!("convert:{CANARY}").parse().unwrap();
    quiet(&slot);
    let ProviderSlot::Other { descriptor, .. } = &slot else {
        panic!("expected Other");
    };
    assert_eq!(descriptor, CANARY);
}
