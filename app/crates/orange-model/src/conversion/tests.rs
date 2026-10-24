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
            "org.example",
            &ty("org.example/label@1"),
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
fn converter_definition_uses_normative_converter_id_wire_field() {
    let definition = definition(
        "org.example/point-to-string",
        "org.example/point@1",
        "core/string@1",
        ConversionClass::LosslessTotal,
        Vec::new(),
        &[],
    );
    let wire = serde_json::to_value(definition).unwrap();
    assert_eq!(
        wire.get("converter_id").and_then(serde_json::Value::as_str),
        Some("org.example/point-to-string")
    );
    assert!(wire.get("id").is_none());
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

#[test]
fn unique_lossless_entry_is_automatic_alongside_partial_entry() {
    let types = type_registry();
    let mut builder = ConverterRegistryBuilder::new(types);
    let lossless = builder
        .register_package_entry(
            "org.example",
            definition(
                "org.example/point-to-text-exact",
                "org.example/point@1",
                "core/string@1",
                ConversionClass::LosslessTotal,
                Vec::new(),
                &[],
            ),
        )
        .unwrap();
    let partial = builder
        .register_package_entry(
            "org.example",
            definition(
                "org.example/point-to-text-checked",
                "org.example/point@1",
                "core/string@1",
                ConversionClass::CheckedPartial,
                Vec::new(),
                &["org.example/Format"],
            ),
        )
        .unwrap();
    let lossless_provider = package_provider("org.example", "exact-formatter");
    let partial_provider = package_provider("org.example", "checked-formatter");
    builder
        .register_package_provider("org.example", &lossless, lossless_provider.clone())
        .unwrap();
    builder
        .register_package_provider("org.example", &partial, partial_provider)
        .unwrap();
    let registry = builder.build();

    let outcome = registry.select_for_connection(
        &ty("org.example/point@1"),
        &ty("core/string@1"),
        &BTreeMap::new(),
    );
    assert_eq!(
        outcome,
        SelectionOutcome::Automatic(ResolvedConverter {
            entry: lossless,
            provider: ConverterProvider::Package(lossless_provider),
            options: BTreeMap::new(),
        })
    );
}

#[test]
fn ambiguous_entries_and_provider_alternatives_are_stable_choices() {
    let mut builder = ConverterRegistryBuilder::new(type_registry());
    let first = builder
        .register_package_entry(
            "org.example",
            definition(
                "org.example/point-to-text-a",
                "org.example/point@1",
                "core/string@1",
                ConversionClass::LosslessTotal,
                Vec::new(),
                &[],
            ),
        )
        .unwrap();
    let second = builder
        .register_package_entry(
            "org.example",
            definition(
                "org.example/point-to-text-b",
                "org.example/point@1",
                "core/string@1",
                ConversionClass::LosslessTotal,
                Vec::new(),
                &[],
            ),
        )
        .unwrap();
    for (key, provider_id) in [(&first, "formatter-a"), (&second, "formatter-b")] {
        builder
            .register_package_provider(
                "org.example",
                key,
                package_provider("org.example", provider_id),
            )
            .unwrap();
    }
    let registry = builder.build();
    let outcome = registry.select_for_connection(
        &ty("org.example/point@1"),
        &ty("core/string@1"),
        &BTreeMap::new(),
    );
    let SelectionOutcome::ChoiceRequired { candidates, reason } = outcome else {
        panic!("expected a semantic-entry choice");
    };
    assert_eq!(reason, ChoiceReason::MultipleLosslessEntries);
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].entry, first);
    assert_eq!(candidates[1].entry, second);

    let mut builder = ConverterRegistryBuilder::new(type_registry());
    let one_entry = builder
        .register_package_entry(
            "org.example",
            definition(
                "org.example/point-to-text",
                "org.example/point@1",
                "core/string@1",
                ConversionClass::LosslessTotal,
                Vec::new(),
                &[],
            ),
        )
        .unwrap();
    builder
        .register_package_provider(
            "org.example",
            &one_entry,
            package_provider("org.example", "formatter-a"),
        )
        .unwrap();
    builder
        .register_package_provider(
            "org.example",
            &one_entry,
            package_provider("org.example", "formatter-b"),
        )
        .unwrap();
    let registry = builder.build();
    let outcome = registry.select_for_connection(
        &ty("org.example/point@1"),
        &ty("core/string@1"),
        &BTreeMap::new(),
    );
    let SelectionOutcome::ChoiceRequired { candidates, reason } = outcome else {
        panic!("expected a provider choice");
    };
    assert_eq!(reason, ChoiceReason::ProviderAmbiguity);
    assert_eq!(candidates.len(), 2);
    assert!(candidates.iter().all(|candidate| {
        candidate
            .reasons
            .contains(&CandidateReason::MultipleProviders)
    }));

    let mut builder = ConverterRegistryBuilder::new(type_registry());
    let one_entry = builder
        .register_package_entry(
            "org.example",
            definition(
                "org.example/point-to-text",
                "org.example/point@1",
                "core/string@1",
                ConversionClass::LosslessTotal,
                Vec::new(),
                &[],
            ),
        )
        .unwrap();
    let default_provider = package_provider("org.example", "formatter-b");
    builder
        .register_package_provider(
            "org.example",
            &one_entry,
            package_provider("org.example", "formatter-a"),
        )
        .unwrap();
    builder
        .register_package_provider("org.example", &one_entry, default_provider.clone())
        .unwrap();
    builder
        .set_new_binding_default(
            &one_entry,
            ConverterProvider::Package(default_provider.clone()),
        )
        .unwrap();
    let registry = builder.build();
    assert_eq!(
        registry.select_for_connection(
            &ty("org.example/point@1"),
            &ty("core/string@1"),
            &BTreeMap::new(),
        ),
        SelectionOutcome::Automatic(ResolvedConverter {
            entry: one_entry,
            provider: ConverterProvider::Package(default_provider),
            options: BTreeMap::new(),
        })
    );
}

#[test]
fn required_policy_and_checked_or_lossy_candidates_require_explicit_exact_choice() {
    let mut builder = ConverterRegistryBuilder::new(type_registry());
    let lossless = builder
        .register_package_entry(
            "org.example",
            definition(
                "org.example/point-to-text",
                "org.example/point@1",
                "core/string@1",
                ConversionClass::LosslessTotal,
                vec![NamedPolicy {
                    name: "format".to_owned(),
                    choices: vec!["compact".to_owned(), "verbose".to_owned()],
                    default: None,
                }],
                &[],
            ),
        )
        .unwrap();
    let provider = package_provider("org.example", "formatter");
    let partial = builder
        .register_package_entry(
            "org.example",
            definition(
                "org.example/point-to-text-checked",
                "org.example/point@1",
                "core/string@1",
                ConversionClass::CheckedPartial,
                vec![NamedPolicy {
                    name: "failure_mode".to_owned(),
                    choices: vec!["stop".to_owned(), "fallback".to_owned()],
                    default: None,
                }],
                &["org.example/Format"],
            ),
        )
        .unwrap();
    let partial_provider = package_provider("org.example", "checked-formatter");
    builder
        .register_package_provider("org.example", &lossless, provider.clone())
        .unwrap();
    builder
        .register_package_provider("org.example", &partial, partial_provider)
        .unwrap();
    let registry = builder.build();
    let outcome = registry.select_for_connection(
        &ty("org.example/point@1"),
        &ty("core/string@1"),
        &BTreeMap::new(),
    );
    let SelectionOutcome::ChoiceRequired { candidates, reason } = outcome else {
        panic!("missing required policy must request a choice");
    };
    assert_eq!(reason, ChoiceReason::MissingRequiredPolicies);
    assert_eq!(candidates.len(), 2);
    assert!(candidates.iter().any(|candidate| {
        candidate.entry == lossless
            && candidate
                .reasons
                .contains(&CandidateReason::RequiredPolicy("format".to_owned()))
    }));
    assert!(candidates.iter().any(|candidate| {
        candidate.entry == partial
            && candidate
                .reasons
                .contains(&CandidateReason::RequiredPolicy("failure_mode".to_owned()))
    }));

    let explicit = |value: &str| ExplicitConverterChoice {
        entry: lossless.clone(),
        provider: ConverterProvider::Package(provider.clone()),
        options: BTreeMap::from([("format".to_owned(), value.to_owned())]),
    };
    let selected = registry
        .resolve_explicit_choice(
            &ty("org.example/point@1"),
            &ty("core/string@1"),
            explicit("verbose"),
        )
        .unwrap();
    assert_eq!(selected.entry, lossless);
    assert_eq!(
        selected.provider,
        ConverterProvider::Package(provider.clone())
    );
    assert_eq!(selected.options["format"], "verbose");
    assert!(matches!(
        registry.resolve_explicit_choice(
            &ty("org.example/point@1"),
            &ty("core/string@1"),
            explicit("unknown"),
        ),
        Err(SelectionRejection::InvalidOptions(
            ConverterRegistrationError::UnknownPolicyChoice { .. }
        ))
    ));
    assert!(matches!(
        registry.resolve_explicit_choice(
            &ty("org.other/point@1"),
            &ty("core/string@1"),
            explicit("verbose"),
        ),
        Err(SelectionRejection::StaleChoice)
    ));
}

#[test]
fn checked_and_lossy_entries_offer_exact_candidates_and_validate_policy_values() {
    let mut builder = ConverterRegistryBuilder::new(type_registry());
    let checked = builder
        .register_package_entry(
            "org.example",
            definition(
                "org.example/point-to-text-checked",
                "org.example/point@1",
                "core/string@1",
                ConversionClass::CheckedPartial,
                vec![NamedPolicy {
                    name: "format".to_owned(),
                    choices: vec!["compact".to_owned(), "verbose".to_owned()],
                    default: None,
                }],
                &["org.example/Format"],
            ),
        )
        .unwrap();
    let lossy = builder
        .register_package_entry(
            "org.example",
            definition(
                "org.example/point-to-text-lossy",
                "org.example/point@1",
                "core/string@1",
                ConversionClass::Lossy,
                vec![NamedPolicy {
                    name: "loss_mode".to_owned(),
                    choices: vec!["replace".to_owned(), "drop".to_owned()],
                    default: Some("replace".to_owned()),
                }],
                &[],
            ),
        )
        .unwrap();
    let checked_provider = package_provider("org.example", "checked-formatter");
    let lossy_provider = package_provider("org.example", "lossy-formatter");
    builder
        .register_package_provider("org.example", &checked, checked_provider.clone())
        .unwrap();
    builder
        .register_package_provider("org.example", &lossy, lossy_provider.clone())
        .unwrap();
    let registry = builder.build();
    let outcome = registry.select_for_connection(
        &ty("org.example/point@1"),
        &ty("core/string@1"),
        &BTreeMap::new(),
    );
    let SelectionOutcome::ChoiceRequired { candidates, reason } = outcome else {
        panic!("partial and lossy candidates require an explicit selection");
    };
    assert_eq!(reason, ChoiceReason::OnlyPartialOrLossyEntries);
    assert!(candidates.iter().any(|candidate| {
        candidate.entry == checked && candidate.class == ConversionClass::CheckedPartial
    }));
    assert!(candidates.iter().any(|candidate| {
        candidate.entry == lossy && candidate.class == ConversionClass::Lossy
    }));

    let selected_checked = registry
        .resolve_explicit_choice(
            &ty("org.example/point@1"),
            &ty("core/string@1"),
            ExplicitConverterChoice {
                entry: checked.clone(),
                provider: ConverterProvider::Package(checked_provider.clone()),
                options: BTreeMap::from([("format".to_owned(), "verbose".to_owned())]),
            },
        )
        .unwrap();
    assert_eq!(selected_checked.entry, checked);
    assert_eq!(
        selected_checked.provider,
        ConverterProvider::Package(checked_provider.clone())
    );
    assert_eq!(selected_checked.options["format"], "verbose");

    let selected_lossy = registry
        .resolve_explicit_choice(
            &ty("org.example/point@1"),
            &ty("core/string@1"),
            ExplicitConverterChoice {
                entry: lossy.clone(),
                provider: ConverterProvider::Package(lossy_provider.clone()),
                options: BTreeMap::new(),
            },
        )
        .unwrap();
    assert_eq!(selected_lossy.entry, lossy);
    assert_eq!(
        selected_lossy.provider,
        ConverterProvider::Package(lossy_provider)
    );
    assert_eq!(selected_lossy.options["loss_mode"], "replace");

    assert!(matches!(
        registry.resolve_explicit_choice(
            &ty("org.example/point@1"),
            &ty("core/string@1"),
            ExplicitConverterChoice {
                entry: checked,
                provider: ConverterProvider::Package(checked_provider),
                options: BTreeMap::from([("format".to_owned(), "unknown".to_owned())]),
            },
        ),
        Err(SelectionRejection::InvalidOptions(
            ConverterRegistrationError::UnknownPolicyChoice { .. }
        ))
    ));
}

#[test]
fn selection_is_direct_only_and_equal_types_return_compatible_first() {
    let mut builder = ConverterRegistryBuilder::new(type_registry());
    for (id, from, to) in [
        (
            "org.example/point-to-label",
            "org.example/point@1",
            "org.example/label@1",
        ),
        (
            "org.example/label-to-string",
            "org.example/label@1",
            "core/string@1",
        ),
    ] {
        let key = builder
            .register_package_entry(
                "org.example",
                definition(
                    id,
                    from,
                    to,
                    ConversionClass::LosslessTotal,
                    Vec::new(),
                    &[],
                ),
            )
            .unwrap();
        builder
            .register_package_provider(
                "org.example",
                &key,
                package_provider("org.example", id.rsplit('/').next().unwrap()),
            )
            .unwrap();
    }
    let registry = builder.build();
    assert_eq!(
        registry.select_for_connection(
            &ty("org.example/point@1"),
            &ty("core/string@1"),
            &BTreeMap::new(),
        ),
        SelectionOutcome::Rejected(SelectionRejection::NoDirectConverter {
            from: ty("org.example/point@1"),
            to: ty("core/string@1"),
        })
    );
    assert_eq!(
        registry.select_for_connection(
            &ty("org.example/point@1"),
            &ty("org.example/point@1"),
            &BTreeMap::new(),
        ),
        SelectionOutcome::Compatible
    );
}

#[test]
fn available_partial_choice_survives_an_unavailable_unique_lossless_entry() {
    let mut builder = ConverterRegistryBuilder::new(type_registry());
    builder
        .register_package_entry(
            "org.example",
            definition(
                "org.example/point-to-text-unavailable",
                "org.example/point@1",
                "core/string@1",
                ConversionClass::LosslessTotal,
                Vec::new(),
                &[],
            ),
        )
        .unwrap();
    let partial = builder
        .register_package_entry(
            "org.example",
            definition(
                "org.example/point-to-text-checked",
                "org.example/point@1",
                "core/string@1",
                ConversionClass::CheckedPartial,
                Vec::new(),
                &["org.example/Format"],
            ),
        )
        .unwrap();
    let provider = package_provider("org.example", "checked-formatter");
    builder
        .register_package_provider("org.example", &partial, provider.clone())
        .unwrap();
    let registry = builder.build();
    let outcome = registry.select_for_connection(
        &ty("org.example/point@1"),
        &ty("core/string@1"),
        &BTreeMap::new(),
    );
    let SelectionOutcome::ChoiceRequired { candidates, reason } = outcome else {
        panic!("available partial converter must remain selectable");
    };
    assert_eq!(reason, ChoiceReason::UniqueLosslessProviderUnavailable);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].entry, partial);
    assert_eq!(candidates[0].provider, ConverterProvider::Package(provider));
    assert!(
        candidates[0]
            .reasons
            .contains(&CandidateReason::CheckedPartial)
    );
}

#[test]
fn converter_config_round_trips_and_names_its_failure() {
    let config = ConverterConfig {
        entry: ConverterKey {
            id: "core/i32-to-i64".parse().unwrap(),
            contract_version: 1,
        },
        options: BTreeMap::from([("rounding".to_owned(), "nearest".to_owned())]),
    };
    let written = config.to_config();
    assert_eq!(ConverterConfig::from_config(&written), Ok(config));

    let mut no_version = written.clone();
    no_version.insert("contract_version".to_owned(), serde_json::json!(0));
    assert_eq!(
        ConverterConfig::from_config(&no_version),
        Err(ConverterConfigError::Entry)
    );
    let mut bad_options = written;
    bad_options.insert("options".to_owned(), serde_json::json!({"rounding": 1}));
    assert_eq!(
        ConverterConfig::from_config(&bad_options),
        Err(ConverterConfigError::Options)
    );
}
