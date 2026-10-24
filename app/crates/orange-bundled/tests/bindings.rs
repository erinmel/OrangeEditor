use std::sync::Arc;

use orange_bundled::{BindingError, MathPackage, Operation, math_definitions};
use orange_model::catalog::{ProviderKey, RegistryBuilder, RegistrySnapshot, TypeArgs};
use orange_model::document::{DefinitionRef, NodeKind};
use orange_model::validation::{DiagnosticCode, HostOrigin};
use orange_model::{Value, analyze, from_json, to_json};
use serde_json::json;

fn catalog(digit: &str) -> (MathPackage, Arc<RegistrySnapshot>) {
    let digest = format!("sha256:{}", digit.repeat(64));
    let package = MathPackage::new(digest.clone()).unwrap();
    let snapshot = package
        .register(RegistryBuilder::core(
            HostOrigin::new("0.0.0", digest).unwrap(),
        ))
        .unwrap()
        .build()
        .unwrap();
    (package, snapshot)
}

fn definition() -> DefinitionRef {
    DefinitionRef {
        id: Operation::Add.definition_id().parse().unwrap(),
        version: 1,
    }
}

fn args() -> TypeArgs {
    [("T".into(), "core/i64@1".parse().unwrap())].into()
}

#[test]
fn a_saved_product_graph_validates_and_resolves_without_test_only_definitions() {
    let (package, snapshot) = catalog("a");
    let definition = math_definitions().unwrap().remove(0);
    let orange_model::catalog::Ports::Declared(ports) = definition.ports else {
        panic!("expected declared ports");
    };
    let signature = orange_model::signature::project(NodeKind::Pure, ports, &args());
    let key = package.provider_key();
    let id = "10000000-0000-4000-8000-000000000001";
    let doc = from_json(&json!({"format":"orange.graph","format_version":1,"graph_id":"20000000-0000-4000-8000-000000000001",
        "revision":0,"package_lock":[{"id":key.package,"version":key.version,"content_digest":key.content_digest}],
        "nodes":[{"id":id,"definition_id":"org.orange.math/add","definition_version":1,
            "config_version":1,"config":{},"provider":{"package":key.package,"version":key.version,"id":key.id},
            "signature":"add-i64","inputs":{"a":{"type":"core/i64@1","value":"20"},"b":{"type":"core/i64@1","value":"22"}}}],
        "signatures":{"add-i64":signature},"exports":[{"id":"answer","node":id,"port":"sum"}]
    }).to_string()).unwrap();
    let before = (to_json(&doc).unwrap(), doc.state_id(), doc.revision());
    let analysis = analyze(&doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(&doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    assert!(!analysis.is_ready_for_compilation(&doc, &snapshot));
    assert_eq!(analysis.report().resolved[&id.parse().unwrap()], signature);
    let node = &doc.root().nodes[&id.parse::<orange_model::id::NodeId>().unwrap()];
    let bound = ProviderKey::of(&doc, node.provider.as_ref().unwrap()).unwrap();
    let input_type = analysis.report().resolved[&node.id]
        .ports
        .iter()
        .find(|port| port.port_id.as_str() == "a")
        .unwrap()
        .ty
        .clone()
        .unwrap();
    let fresh_args = [("T".into(), input_type)].into();
    let kernel = package
        .resolve(
            Arc::clone(&snapshot),
            &bound,
            &node.definition,
            node.config_version,
            &fresh_args,
            &node.config,
        )
        .unwrap();
    assert_eq!(
        kernel.evaluate(&[Value::I64(20), Value::I64(22)]),
        Ok(Value::I64(42))
    );
    assert_eq!(
        (to_json(&doc).unwrap(), doc.state_id(), doc.revision()),
        before
    );

    // Unlocked authoring data remains loadable, but cannot supply an executable binding.
    let mut unlocked: serde_json::Value = serde_json::from_str(&before.0).unwrap();
    unlocked["package_lock"] = json!([]);
    let unlocked = from_json(&unlocked.to_string()).unwrap();
    let report = analyze(&unlocked, &snapshot);
    assert!(
        report
            .report()
            .diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::PackageLockMissing)
    );
    assert!(!report.is_ready_for_lowering(&unlocked, &snapshot));
    assert!(ProviderKey::of(&unlocked, node.provider.as_ref().unwrap()).is_none());
}

#[test]
fn updates_retain_old_pins_and_never_fall_back_to_another_provider() {
    let (old, old_snapshot) = catalog("a");
    let old_kernel = old
        .resolve(
            Arc::clone(&old_snapshot),
            old.provider_key(),
            &definition(),
            1,
            &args(),
            &Default::default(),
        )
        .unwrap();
    let (new, new_snapshot) = catalog("b");
    assert_ne!(old_snapshot.generation(), new_snapshot.generation());
    assert_eq!(
        old.resolve(
            Arc::clone(&new_snapshot),
            old.provider_key(),
            &definition(),
            1,
            &args(),
            &Default::default()
        )
        .unwrap_err(),
        BindingError::Unavailable
    );
    assert_eq!(
        old.resolve(
            Arc::clone(&old_snapshot),
            new.provider_key(),
            &definition(),
            1,
            &args(),
            &Default::default()
        )
        .unwrap_err(),
        BindingError::Unavailable
    );
    let mut guest = old.provider_key().clone();
    guest.id = "selected-guest".into();
    assert_eq!(
        old.resolve(
            Arc::clone(&old_snapshot),
            &guest,
            &definition(),
            1,
            &args(),
            &Default::default()
        )
        .unwrap_err(),
        BindingError::Unavailable
    );
    let new_kernel = new
        .resolve(
            Arc::clone(&new_snapshot),
            new.provider_key(),
            &definition(),
            1,
            &args(),
            &Default::default(),
        )
        .unwrap();
    drop(old_snapshot);
    assert_eq!(
        old_kernel.provider().content_digest,
        format!("sha256:{}", "a".repeat(64))
    );
    assert_ne!(
        old_kernel.snapshot().generation(),
        new_kernel.snapshot().generation()
    );
    assert_eq!(
        old_kernel.evaluate(&[Value::I64(20), Value::I64(22)]),
        Ok(Value::I64(42))
    );
}

#[test]
fn invalid_config_versions_and_specializations_fail_before_execution() {
    let (package, snapshot) = catalog("a");
    for config in [
        json!({"int_overflow":"unknown"}),
        json!({"float_policy":true}),
        json!({"additional":"private-value"}),
        json!({"int_overflow":null}),
    ] {
        let error = package
            .resolve(
                Arc::clone(&snapshot),
                package.provider_key(),
                &definition(),
                1,
                &args(),
                config.as_object().unwrap(),
            )
            .unwrap_err();
        assert_eq!(error, BindingError::InvalidConfig);
        assert!(!error.to_string().contains("private-value"));
    }
    for ty in [
        "core/bool@1",
        "core/duration@1",
        "core/list@1<core/i64@1>",
        "$T",
        "org.example/numeric@1",
    ] {
        let args = [("T".into(), ty.parse().unwrap())].into();
        assert_eq!(
            package
                .resolve(
                    Arc::clone(&snapshot),
                    package.provider_key(),
                    &definition(),
                    1,
                    &args,
                    &Default::default()
                )
                .unwrap_err(),
            BindingError::UnsupportedType
        );
    }
    for args in [
        TypeArgs::new(),
        [
            ("T".into(), "core/i64@1".parse().unwrap()),
            ("U".into(), "core/i64@1".parse().unwrap()),
        ]
        .into(),
    ] {
        assert_eq!(
            package
                .resolve(
                    Arc::clone(&snapshot),
                    package.provider_key(),
                    &definition(),
                    1,
                    &args,
                    &Default::default()
                )
                .unwrap_err(),
            BindingError::UnsupportedType
        );
    }
    for (version, config_version) in [(2, 1), (1, 2)] {
        let definition = DefinitionRef {
            version,
            ..definition()
        };
        assert_eq!(
            package
                .resolve(
                    Arc::clone(&snapshot),
                    package.provider_key(),
                    &definition,
                    config_version,
                    &args(),
                    &Default::default()
                )
                .unwrap_err(),
            BindingError::Unavailable
        );
    }
    let not = DefinitionRef {
        id: Operation::Not.definition_id().parse().unwrap(),
        version: 1,
    };
    assert_eq!(
        package
            .resolve(
                snapshot,
                package.provider_key(),
                &not,
                1,
                &Default::default(),
                json!({"float_policy":"finite"}).as_object().unwrap()
            )
            .unwrap_err(),
        BindingError::InvalidConfig
    );
}

#[test]
fn build_identity_is_canonical_and_unavailable_metadata_cannot_bind() {
    for digest in [
        "",
        "sha256:short",
        &format!("sha256:{}", "A".repeat(64)),
        "private-build-value",
    ] {
        let error = MathPackage::new(digest).unwrap_err();
        assert!(!error.to_string().contains(digest) || digest.is_empty());
        assert!(error.message_key().starts_with("core/"));
    }
    let (package, _) = catalog("a");
    let empty = RegistrySnapshot::core(
        HostOrigin::new("0.0.0", format!("sha256:{}", "b".repeat(64))).unwrap(),
    );
    assert_eq!(
        package
            .resolve(
                empty,
                package.provider_key(),
                &definition(),
                1,
                &args(),
                &Default::default()
            )
            .unwrap_err(),
        BindingError::Unavailable
    );
}

#[test]
fn published_config_schemas_and_native_resolution_agree() {
    let (package, snapshot) = catalog("a");
    for definition in math_definitions().unwrap() {
        let args = if definition.type_params.is_empty() {
            TypeArgs::new()
        } else {
            args()
        };
        for config in [
            json!({}),
            json!({"int_overflow":"checked"}),
            json!({"int_overflow":"wrapping","float_policy":"ieee"}),
            json!({"int_overflow":"saturating","float_policy":"finite"}),
            json!({"float_policy":"unknown"}),
            json!({"int_overflow":1}),
            json!({"float_policy":null}),
            json!({"extra":"private"}),
        ] {
            let schema_accepts = orange_model::validation::config_satisfies_schema(
                &config,
                definition.config_schema.as_ref().unwrap(),
            );
            let result = package.resolve(
                Arc::clone(&snapshot),
                package.provider_key(),
                &definition.definition,
                1,
                &args,
                config.as_object().unwrap(),
            );
            assert_eq!(
                result.is_ok(),
                schema_accepts,
                "{}: {config}",
                definition.definition.id
            );
        }
    }
}
