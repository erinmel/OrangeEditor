use super::*;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::id::TypeExpr;
use crate::types::TypeRegistryBuilder;
use crate::types::{TypeOwner, TypeRegistry};
use crate::value::literal_codec::RecordSchema;

fn ty(value: &str) -> TypeExpr {
    value.parse().unwrap()
}

fn type_registry() -> Arc<TypeRegistry> {
    let mut builder = TypeRegistryBuilder::core();
    builder
        .register_record(
            "org.example",
            &ty("org.example/point@1"),
            RecordSchema { fields: Vec::new() },
        )
        .unwrap();
    builder
        .register_record(
            "org.other",
            &ty("org.other/point@1"),
            RecordSchema { fields: Vec::new() },
        )
        .unwrap();
    Arc::new(builder.build().unwrap())
}

fn definition(
    id: &str,
    from: &str,
    to: &str,
    class: ConversionClass,
    policies: Vec<NamedPolicy>,
    errors: &[&str],
) -> ConverterDefinition {
    ConverterDefinition {
        id: id.parse().unwrap(),
        contract_version: 1,
        from: ty(from),
        to: ty(to),
        class,
        policies,
        errors: errors
            .iter()
            .map(|code| ConverterErrorCode::new(*code).unwrap())
            .collect(),
    }
}

fn package_provider(package: &str, id: &str) -> PackageProvider {
    PackageProvider {
        package: package.to_owned(),
        version: "1.2.3".to_owned(),
        content_digest: format!("sha256:{}", "a".repeat(64)),
        id: id.to_owned(),
        contract_version: 1,
    }
}

#[test]
fn core_registry_publishes_v1_integer_entries_with_expected_classes() {
    let types = type_registry();
    let registry = ConverterRegistryBuilder::with_core_conversions(types)
        .unwrap()
        .build();

    let up = ty("core/i32@1");
    let wide = ty("core/i64@1");
    let widening: Vec<_> = registry.entries_for_pair(&up, &wide).collect();
    assert_eq!(widening.len(), 1);
    assert_eq!(widening[0].1.class, ConversionClass::LosslessTotal);
    assert_eq!(registry.owner(widening[0].0), Some(&TypeOwner::Core));
    assert_eq!(
        registry.new_binding_default(widening[0].0),
        Some(&ConverterProvider::Core)
    );

    let single = ty("core/f32@1");
    let double = ty("core/f64@1");
    let float_widening: Vec<_> = registry.entries_for_pair(&single, &double).collect();
    assert_eq!(float_widening.len(), 1);
    assert_eq!(float_widening[0].1.class, ConversionClass::LosslessTotal);
    assert_eq!(float_widening[0].1.id.as_str(), "core/f32-to-f64");

    let float_narrowing: Vec<_> = registry.entries_for_pair(&double, &single).collect();
    assert_eq!(float_narrowing.len(), 1);
    assert_eq!(float_narrowing[0].1.class, ConversionClass::Lossy);
    assert_eq!(
        float_narrowing[0].1.loss_output_type(),
        Some(ty("core/json@1"))
    );
    assert_eq!(float_narrowing[0].1.errors[0].as_str(), "core/OutOfRange");
    let policies = float_narrowing[0]
        .1
        .resolve_options(&BTreeMap::new())
        .unwrap();
    assert_eq!(policies["rounding"], "nearest-even");
    assert_eq!(policies["overflow"], "reject");

    let narrowing: Vec<_> = registry.entries_for_pair(&wide, &up).collect();
    assert_eq!(narrowing.len(), 1);
    assert_eq!(narrowing[0].1.class, ConversionClass::CheckedPartial);
    assert_eq!(narrowing[0].1.errors[0].as_str(), "core/OutOfRange");
    assert_eq!(narrowing[0].1.loss_output_type(), None);
}

#[test]
fn package_entry_must_own_an_endpoint_and_its_namespaced_identity() {
    let mut builder = ConverterRegistryBuilder::new(type_registry());
    let owned = definition(
        "org.example/point-to-string",
        "org.example/point@1",
        "core/string@1",
        ConversionClass::LosslessTotal,
        Vec::new(),
        &[],
    );
    let key = builder
        .register_package_entry("org.example", owned)
        .unwrap();
    assert_eq!(key.id.namespace(), "org.example");

    let foreign = definition(
        "org.example/foreign-pair",
        "org.other/point@1",
        "core/i64@1",
        ConversionClass::LosslessTotal,
        Vec::new(),
        &[],
    );
    assert!(matches!(
        builder.register_package_entry("org.example", foreign),
        Err(ConverterRegistrationError::ForeignToForeign { .. })
    ));

    let wrong_id_namespace = definition(
        "core/pretend-core",
        "org.example/point@1",
        "core/string@1",
        ConversionClass::LosslessTotal,
        Vec::new(),
        &[],
    );
    assert!(matches!(
        builder.register_package_entry("org.example", wrong_id_namespace),
        Err(ConverterRegistrationError::EntryNamespaceMismatch { .. })
    ));
}

#[test]
fn override_providers_must_match_the_entry_contract_and_have_explicit_defaults() {
    let mut builder = ConverterRegistryBuilder::new(type_registry());
    let key = builder
        .register_package_entry(
            "org.example",
            definition(
                "org.example/point-to-string",
                "org.example/point@1",
                "core/string@1",
                ConversionClass::LosslessTotal,
                Vec::new(),
                &[],
            ),
        )
        .unwrap();
    let provider = package_provider("org.alternative", "point-text-v2");

    assert!(matches!(
        builder.register_package_provider(
            "org.alternative",
            &key,
            PackageProvider {
                contract_version: 2,
                ..provider.clone()
            }
        ),
        Err(ConverterRegistrationError::ProviderContractMismatch { .. })
    ));
    builder
        .register_package_provider("org.alternative", &key, provider.clone())
        .unwrap();
    builder
        .set_new_binding_default(&key, ConverterProvider::Package(provider.clone()))
        .unwrap();
    let registry = builder.build();

    let alternatives: Vec<_> = registry.providers(&key).unwrap().collect();
    assert_eq!(
        alternatives,
        [&ConverterProvider::Package(provider.clone())]
    );
    assert_eq!(
        registry.new_binding_default(&key),
        Some(&ConverterProvider::Package(provider))
    );
}

#[test]
fn duplicate_entry_identity_rejects_conflicting_semantics() {
    let mut builder = ConverterRegistryBuilder::new(type_registry());
    let original = definition(
        "org.example/point-to-string",
        "org.example/point@1",
        "core/string@1",
        ConversionClass::LosslessTotal,
        Vec::new(),
        &[],
    );
    let key = builder
        .register_package_entry("org.example", original.clone())
        .unwrap();
    assert_eq!(
        builder
            .register_package_entry("org.example", original)
            .unwrap(),
        key
    );

    let conflicting = definition(
        "org.example/point-to-string",
        "org.example/point@1",
        "core/string@1",
        ConversionClass::Lossy,
        Vec::new(),
        &[],
    );
    assert!(matches!(
        builder.register_package_entry("org.example", conflicting),
        Err(ConverterRegistrationError::ConflictingEntry { .. })
    ));
}

#[test]
fn pair_can_have_distinct_semantics_and_defaults_are_per_entry() {
    let mut builder = ConverterRegistryBuilder::new(type_registry());
    let lossless = builder
        .register_package_entry(
            "org.example",
            definition(
                "org.example/point-to-string",
                "org.example/point@1",
                "core/string@1",
                ConversionClass::LosslessTotal,
                Vec::new(),
                &[],
            ),
        )
        .unwrap();
    let lossy = builder
        .register_package_entry(
            "org.example",
            definition(
                "org.example/point-to-string-lossy",
                "org.example/point@1",
                "core/string@1",
                ConversionClass::Lossy,
                Vec::new(),
                &[],
            ),
        )
        .unwrap();
    let provider_a = package_provider("org.example", "formatter-a");
    let provider_b = package_provider("org.example", "formatter-b");
    builder
        .register_package_provider("org.example", &lossless, provider_a.clone())
        .unwrap();
    builder
        .register_package_provider("org.example", &lossy, provider_b.clone())
        .unwrap();
    builder
        .set_new_binding_default(&lossless, ConverterProvider::Package(provider_a.clone()))
        .unwrap();
    builder
        .set_new_binding_default(&lossy, ConverterProvider::Package(provider_b.clone()))
        .unwrap();

    let registry = builder.build();
    let entries: Vec<_> = registry
        .entries_for_pair(&ty("org.example/point@1"), &ty("core/string@1"))
        .collect();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries
            .iter()
            .find(|(key, _)| **key == lossless)
            .unwrap()
            .1
            .loss_output_type(),
        None
    );
    assert_eq!(
        entries
            .iter()
            .find(|(key, _)| **key == lossy)
            .unwrap()
            .1
            .loss_output_type(),
        Some(ty("core/json@1"))
    );
    assert_eq!(
        registry.new_binding_default(&lossless),
        Some(&ConverterProvider::Package(provider_a))
    );
    assert_eq!(
        registry.new_binding_default(&lossy),
        Some(&ConverterProvider::Package(provider_b))
    );
}

#[test]
fn finite_named_policies_validate_defaults_required_values_and_unknown_options() {
    let policy = NamedPolicy {
        name: "rounding_mode".to_owned(),
        choices: vec!["truncate".to_owned(), "round-even".to_owned()],
        default: None,
    };
    let mut builder = ConverterRegistryBuilder::new(type_registry());
    let key = builder
        .register_package_entry(
            "org.example",
            definition(
                "org.example/parse-point",
                "core/string@1",
                "org.example/point@1",
                ConversionClass::CheckedPartial,
                vec![policy],
                &["core/Parse"],
            ),
        )
        .unwrap();
    let registry = builder.build();
    let entry = registry.entry(&key).unwrap();

    assert!(matches!(
        entry.resolve_options(&BTreeMap::new()),
        Err(ConverterRegistrationError::MissingPolicy { .. })
    ));
    let supplied = BTreeMap::from([("rounding_mode".to_owned(), "round-even".to_owned())]);
    assert_eq!(entry.resolve_options(&supplied).unwrap(), supplied);
    let unsupported = BTreeMap::from([("rounding_mode".to_owned(), "nearest".to_owned())]);
    assert!(matches!(
        entry.resolve_options(&unsupported),
        Err(ConverterRegistrationError::UnknownPolicyChoice { .. })
    ));
    let unknown = BTreeMap::from([("encoding".to_owned(), "replace".to_owned())]);
    assert!(matches!(
        entry.resolve_options(&unknown),
        Err(ConverterRegistrationError::UnknownPolicy { .. })
    ));
}

#[test]
fn policy_defaults_fill_missing_values_and_checked_partial_requires_errors() {
    let policy = NamedPolicy {
        name: "text_errors".to_owned(),
        choices: vec!["strict".to_owned(), "replace".to_owned()],
        default: Some("strict".to_owned()),
    };
    let lossy_definition = definition(
        "org.example/to-bytes",
        "org.example/point@1",
        "core/bytes@1",
        ConversionClass::Lossy,
        vec![policy],
        &[],
    );
    let mut builder = ConverterRegistryBuilder::new(type_registry());
    let key = builder
        .register_package_entry("org.example", lossy_definition)
        .unwrap();
    let entry = builder.build();
    assert_eq!(
        entry
            .entry(&key)
            .unwrap()
            .resolve_options(&BTreeMap::new())
            .unwrap(),
        BTreeMap::from([("text_errors".to_owned(), "strict".to_owned())])
    );

    let mut builder = ConverterRegistryBuilder::new(type_registry());
    assert!(matches!(
        builder.register_package_entry(
            "org.example",
            definition(
                "org.example/missing-error",
                "core/i64@1",
                "org.example/point@1",
                ConversionClass::CheckedPartial,
                Vec::new(),
                &[],
            )
        ),
        Err(ConverterRegistrationError::PartialNeedsErrors { .. })
    ));
}

#[test]
fn lossy_entries_publish_fixed_json_loss_port_contract() {
    let mut builder = ConverterRegistryBuilder::new(type_registry());
    let key = builder
        .register_package_entry(
            "org.example",
            definition(
                "org.example/point-to-string-lossy",
                "org.example/point@1",
                "core/string@1",
                ConversionClass::Lossy,
                Vec::new(),
                &[],
            ),
        )
        .unwrap();
    let registry = builder.build();
    assert_eq!(
        registry.entry(&key).unwrap().loss_output_type(),
        Some(ty("core/json@1"))
    );
}

#[test]
fn package_provider_metadata_requires_exact_digest_and_matching_authority() {
    let mut builder = ConverterRegistryBuilder::new(type_registry());
    let key = builder
        .register_package_entry(
            "org.example",
            definition(
                "org.example/point-to-string",
                "org.example/point@1",
                "core/string@1",
                ConversionClass::LosslessTotal,
                Vec::new(),
                &[],
            ),
        )
        .unwrap();

    assert!(matches!(
        builder.register_package_provider("org.other", &key, package_provider("org.example", "p")),
        Err(ConverterRegistrationError::ProviderAuthorityMismatch { .. })
    ));
    let invalid_digest = PackageProvider {
        content_digest: "sha256:bad".to_owned(),
        ..package_provider("org.example", "p")
    };
    assert!(matches!(
        builder.register_package_provider("org.example", &key, invalid_digest),
        Err(ConverterRegistrationError::InvalidProviderDigest { .. })
    ));
}

#[test]
fn same_provider_binding_rejects_conflicting_digest_but_exact_repeat_is_idempotent() {
    let mut builder = ConverterRegistryBuilder::new(type_registry());
    let key = builder
        .register_package_entry(
            "org.example",
            definition(
                "org.example/point-to-string",
                "org.example/point@1",
                "core/string@1",
                ConversionClass::LosslessTotal,
                Vec::new(),
                &[],
            ),
        )
        .unwrap();
    let provider = package_provider("org.example", "formatter");
    builder
        .register_package_provider("org.example", &key, provider.clone())
        .unwrap();
    builder
        .register_package_provider("org.example", &key, provider.clone())
        .unwrap();
    assert!(matches!(
        builder.register_package_provider(
            "org.example",
            &key,
            PackageProvider {
                content_digest: format!("sha256:{}", "b".repeat(64)),
                ..provider.clone()
            }
        ),
        Err(ConverterRegistrationError::ConflictingProviderDigest { .. })
    ));
    builder
        .register_package_provider(
            "org.example",
            &key,
            PackageProvider {
                version: "1.2.4".to_owned(),
                ..provider.clone()
            },
        )
        .unwrap();
    builder
        .register_package_provider(
            "org.example",
            &key,
            PackageProvider {
                id: "formatter-next".to_owned(),
                ..provider.clone()
            },
        )
        .unwrap();
    let registry = builder.build();
    assert_eq!(registry.providers(&key).unwrap().count(), 3);
}

#[test]
fn provider_identity_rejects_padding() {
    let mut builder = ConverterRegistryBuilder::new(type_registry());
    let key = builder
        .register_package_entry(
            "org.example",
            definition(
                "org.example/point-to-string",
                "org.example/point@1",
                "core/string@1",
                ConversionClass::LosslessTotal,
                Vec::new(),
                &[],
            ),
        )
        .unwrap();
    assert!(matches!(
        builder.register_package_provider(
            "org.example",
            &key,
            PackageProvider {
                package: " org.example".to_owned(),
                ..package_provider("org.example", "formatter")
            },
        ),
        Err(ConverterRegistrationError::InvalidProviderIdentity)
    ));
}
