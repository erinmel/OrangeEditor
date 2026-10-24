mod common;

use common::{minimal_document, node_id};
use orange_model::catalog::{
    NodeDefinition, Ports, ProviderDescriptor, ProviderKey, RegistryBuilder, RegistrySnapshot,
};
use orange_model::document::{DefinitionRef, NodeKind};
use orange_model::from_json;
use orange_model::validation::{
    CONFIG_SCHEMA_DIALECT, DiagnosticCode, HostOrigin, ValidationCheck, analyze,
    config_schema_is_supported,
};
use serde_json::{Value, json};

const DIGEST: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

fn origin() -> HostOrigin {
    HostOrigin::new("0.1.0", format!("sha256:{}", "a".repeat(64))).unwrap()
}

fn registry(schema: Option<Value>) -> std::sync::Arc<RegistrySnapshot> {
    let definition = DefinitionRef {
        id: "example/configured".parse().unwrap(),
        version: 1,
    };
    let key = ProviderKey {
        package: "example.pkg".into(),
        version: "1.0.0".into(),
        content_digest: DIGEST.into(),
        id: "example/configured".into(),
    };
    let mut provider = ProviderDescriptor {
        key,
        implements: Default::default(),
        specializations: Default::default(),
    };
    provider.implements.insert((definition.clone(), 1));
    RegistryBuilder::core(origin())
        .definition(NodeDefinition {
            definition,
            config_version: 1,
            kind: NodeKind::Pure,
            volatility: None,
            determinism: orange_model::catalog::Determinism::Deterministic,
            suspends: None,
            type_params: vec![],
            ports: Ports::Declared(vec![]),
            references: vec![],
            specializations: None,
            config_schema: schema,
        })
        .provider(provider)
        .build()
        .unwrap()
}

fn codes(schema: Option<Value>, config: Value) -> Vec<DiagnosticCode> {
    let document = from_json(&minimal_document(json!({
        "package_lock":[{"id":"example.pkg","version":"1.0.0","content_digest":DIGEST}],
        "signatures":{"sig":{"kind":"pure","ports":[]}},
        "nodes":[{"id":node_id(1),"definition_id":"example/configured","definition_version":1,
            "signature":"sig","provider":{"package":"example.pkg","version":"1.0.0",
            "id":"example/configured"},"config_version":1,"config":config}]
    })))
    .unwrap();
    analyze(&document, &registry(schema))
        .report()
        .diagnostics
        .iter()
        .map(|d| d.code)
        .collect()
}

fn schema() -> Value {
    json!({
        "$schema": CONFIG_SCHEMA_DIALECT,
        "type": "object",
        "properties": {
            "mode": {"enum": ["a", "b"], "default": "a"},
            "n": {"type": "integer", "minimum": 1, "maximum": 5}
        },
        "required": ["mode"],
        "additionalProperties": false
    })
}

#[test]
fn a_conforming_config_and_an_absent_schema_report_nothing() {
    assert!(codes(Some(schema()), json!({"mode":"a","n":3})).is_empty());
    assert!(codes(Some(schema()), json!({"mode":"b"})).is_empty());
    assert!(codes(None, json!({"anything":[1,2]})).is_empty());
}

#[test]
fn each_supported_keyword_rejects_a_violation() {
    for config in [
        json!({}),                        // required
        json!({"mode":"c"}),              // enum
        json!({"mode":"a","n":0}),        // minimum
        json!({"mode":"a","n":6}),        // maximum
        json!({"mode":"a","n":1.5}),      // integer
        json!({"mode":"a","n":"3"}),      // type
        json!({"mode":"a","extra":true}), // additionalProperties
    ] {
        assert_eq!(
            codes(Some(schema()), config.clone()),
            [DiagnosticCode::ConfigInvalid],
            "{config}"
        );
    }
}

#[test]
fn constructs_outside_the_dialect_are_reported_never_ignored() {
    for unsupported in [
        json!({"patternProperties": {}}),
        json!({"$ref": "#/definitions/x"}),
        json!({"oneOf": []}),
        json!({"$schema": "http://json-schema.org/draft-07/schema#"}),
        json!({"type": ["string", "null"]}),
        json!({"additionalProperties": {"type": "string"}}),
        json!({"properties": {"x": {"items": {}}}}),
        json!({"minimum": "1"}),
        json!({"enum": []}),
    ] {
        assert!(!config_schema_is_supported(&unsupported), "{unsupported}");
        assert_eq!(
            codes(Some(unsupported.clone()), json!({"x": 1})),
            [DiagnosticCode::ConfigSchemaUnsupported],
            "{unsupported}"
        );
    }
}

#[test]
fn definition_and_config_is_covered() {
    let document = from_json(&minimal_document(json!({}))).unwrap();
    let analysis = analyze(&document, &registry(None));
    assert!(
        analysis
            .covered_checks()
            .any(|check| check == ValidationCheck::DefinitionAndConfig)
    );
}

#[test]
fn numeric_bounds_preserve_integer_precision_and_mixed_number_order() {
    for (value, keyword, bound, valid) in [
        (
            json!(9_007_199_254_740_993u64),
            "maximum",
            json!(9_007_199_254_740_992u64),
            false,
        ),
        (
            json!(9_007_199_254_740_992u64),
            "minimum",
            json!(9_007_199_254_740_993u64),
            false,
        ),
        (json!(u64::MAX), "maximum", json!(u64::MAX - 1), false),
        (json!(i64::MIN), "minimum", json!(i64::MIN + 1), false),
        (
            json!(u64::MAX),
            "maximum",
            json!(18_446_744_073_709_551_616.0),
            true,
        ),
        (
            json!(u64::MAX),
            "minimum",
            json!(18_446_744_073_709_551_616.0),
            false,
        ),
        (
            json!(18_446_744_073_709_551_616.0),
            "maximum",
            json!(u64::MAX),
            false,
        ),
        (
            json!(18_446_744_073_709_551_616.0),
            "minimum",
            json!(u64::MAX),
            true,
        ),
        (json!(-1), "minimum", json!(-0.5), false),
        (json!(0), "maximum", json!(-0.5), false),
        (json!(0.5), "maximum", json!(0), false),
        (json!(-0.5), "minimum", json!(-1), true),
        (
            json!(i64::MIN),
            "maximum",
            json!(-9_223_372_036_854_775_808.0),
            true,
        ),
        (json!(u64::MAX), "minimum", json!(-1), true),
    ] {
        let schema = json!({"properties":{"n":{keyword: bound}}});
        let diagnostics = codes(Some(schema), json!({"n": value}));
        assert_eq!(
            diagnostics.is_empty(),
            valid,
            "{value} {keyword} {bound}: {diagnostics:?}"
        );
        if !valid {
            assert_eq!(diagnostics, [DiagnosticCode::ConfigInvalid]);
        }
    }
}
