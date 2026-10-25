use orange_model::catalog::{NodeDefinition, Ports, RegistryBuilder, RegistryError};
use orange_model::document::literal::Literal;
use orange_model::document::{
    DefinitionRef, NodeKind, OutputSource, PortDescriptor, PortDirection, PortKind, TypeParam,
};
use orange_model::id::TypeExpr;
use orange_model::type_term::TypeTerm;
use orange_model::types::{TypeRegistry, TypeRegistryBuilder};
use orange_model::validation::HostOrigin;
use orange_model::value::literal_codec::{EnumSchema, FieldSchema, RecordSchema, VariantSchema};
use serde_json::json;

fn origin() -> HostOrigin {
    HostOrigin::new("1.0.0", format!("sha256:{}", "a".repeat(64))).unwrap()
}

fn definition(ports: Vec<PortDescriptor>, params: &[&str]) -> NodeDefinition {
    NodeDefinition {
        definition: DefinitionRef {
            id: "acme/input-metadata".parse().unwrap(),
            version: 1,
        },
        config_version: 1,
        kind: NodeKind::Action,
        volatility: None,
        determinism: orange_model::catalog::Determinism::Deterministic,
        suspends: None,
        type_params: params
            .iter()
            .map(|name| serde_json::from_value::<TypeParam>(json!({"name":name})).unwrap())
            .collect(),
        ports: Ports::Declared(ports),
        references: Vec::new(),
        specializations: None,
        config_schema: None,
    }
}

fn build(ports: Vec<PortDescriptor>, params: &[&str]) -> Result<(), RegistryError> {
    RegistryBuilder::core(origin())
        .definition(definition(ports, params))
        .build()
        .map(|_| ())
}

fn build_with_types(
    ports: Vec<PortDescriptor>,
    params: &[&str],
    types: std::sync::Arc<TypeRegistry>,
) -> Result<(), RegistryError> {
    RegistryBuilder::core(origin())
        .types(types)
        .definition(definition(ports, params))
        .build()
        .map(|_| ())
}

fn option_type() -> TypeExpr {
    "core/option@1<core/i64@1>".parse().unwrap()
}

#[test]
fn defaults_and_optional_flags_are_confined_to_data_inputs() {
    let default = Literal::value("core/i64@1".parse().unwrap(), json!("1"));
    let mut valid_input = PortDescriptor::data_in("value", "core/i64@1".parse().unwrap());
    valid_input.default = Some(default.clone());
    assert!(build(vec![valid_input], &[]).is_ok());

    let mut exec_input = PortDescriptor::exec_in("exec_value");
    exec_input.default = Some(default.clone());
    assert!(matches!(
        build(vec![exec_input], &[]),
        Err(RegistryError::DefaultPortPosition { definition, port })
            if definition.as_str() == "acme/input-metadata" && port.as_str() == "exec_value"
    ));

    let mut output = PortDescriptor::data_out(
        "output_value",
        "core/i64@1".parse().unwrap(),
        OutputSource::Computed,
    );
    output.default = Some(default);
    assert!(matches!(
        build(vec![output], &[]),
        Err(RegistryError::DefaultPortPosition { .. })
    ));

    let mut event_output = PortDescriptor {
        direction: PortDirection::Output,
        kind: PortKind::Event,
        ty: Some("core/i64@1".parse().unwrap()),
        ..PortDescriptor::data_in("event", "core/i64@1".parse().unwrap())
    };
    event_output.optional = Some(true);
    assert!(matches!(
        build(vec![event_output], &[]),
        Err(RegistryError::OptionalPortPosition { .. })
    ));

    let mut data_output =
        PortDescriptor::data_out("optional_output", option_type(), OutputSource::Computed);
    data_output.optional = Some(true);
    assert!(matches!(
        build(vec![data_output], &[]),
        Err(RegistryError::OptionalPortPosition { .. })
    ));
}

#[test]
fn optional_true_requires_the_exact_option_constructor_but_false_is_inert() {
    let mut option = PortDescriptor::data_in("value", option_type());
    option.optional = Some(true);
    assert!(build(vec![option], &[]).is_ok());

    let mut generic_option =
        PortDescriptor::data_in("generic_value", "core/option@1<$T>".parse().unwrap());
    generic_option.optional = Some(true);
    assert!(build(vec![generic_option], &["T"]).is_ok());

    for ty in ["core/i64@1", "core/option@1", "$T"] {
        let mut invalid = PortDescriptor::data_in("value", ty.parse().unwrap());
        invalid.optional = Some(true);
        assert!(
            matches!(
                build(vec![invalid], &["T"]),
                Err(RegistryError::OptionalType { .. })
            ),
            "{ty}"
        );
    }

    let mut false_flag = PortDescriptor::data_in("value", "core/i64@1".parse().unwrap());
    false_flag.optional = Some(false);
    assert!(build(vec![false_flag], &[]).is_ok());
    let mut false_output = PortDescriptor::data_out(
        "value",
        "core/i64@1".parse().unwrap(),
        OutputSource::Computed,
    );
    false_output.optional = Some(false);
    assert!(build(vec![false_output], &[]).is_ok());
    let mut unknown_inner =
        PortDescriptor::data_in("value", "core/option@1<missing/type@1>".parse().unwrap());
    unknown_inner.optional = Some(true);
    assert!(build(vec![unknown_inner], &[]).is_ok());
}

#[test]
fn malformed_optional_type_syntax_remains_deferred_to_inference() {
    let mut malformed = PortDescriptor::data_in("value", "not-a-type".parse().unwrap());
    malformed.optional = Some(true);
    assert!(build(vec![malformed], &[]).is_ok());
}

#[test]
fn descriptor_and_snapshot_optional_metadata_round_trip_without_normalization() {
    let descriptor = PortDescriptor {
        optional: Some(false),
        default: Some(Literal::value("core/i64@1".parse().unwrap(), json!("1"))),
        // Invalid for new publication; historic passive metadata remains repairable.
        ..PortDescriptor::data_out(
            "value",
            "core/i64@1".parse().unwrap(),
            OutputSource::Computed,
        )
    };
    let snapshot = orange_model::document::SignatureSnapshot {
        kind: NodeKind::Action,
        suspends: None,
        ports: vec![descriptor.clone()],
        extensions: Default::default(),
    };

    let descriptor_wire = serde_json::to_value(&descriptor).unwrap();
    let snapshot_wire = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(
        serde_json::from_value::<PortDescriptor>(descriptor_wire).unwrap(),
        descriptor
    );
    assert_eq!(
        serde_json::from_value::<orange_model::document::SignatureSnapshot>(snapshot_wire).unwrap(),
        snapshot
    );
}

#[test]
fn published_defaults_must_match_and_decode_for_registered_types() {
    let mut matching = PortDescriptor::data_in("value", "core/i64@1".parse().unwrap());
    matching.default = Some(Literal::value("core/i64@1".parse().unwrap(), json!("7")));
    assert!(build(vec![matching], &[]).is_ok());

    let mut mismatch = PortDescriptor::data_in("value", "core/i64@1".parse().unwrap());
    mismatch.default = Some(Literal::value("core/i32@1".parse().unwrap(), json!("7")));
    assert!(matches!(
        build(vec![mismatch], &[]),
        Err(RegistryError::DefaultType { definition, port })
            if definition.as_str() == "acme/input-metadata" && port.as_str() == "value"
    ));

    let mut mismatch_and_bad_payload =
        PortDescriptor::data_in("value", "core/i64@1".parse().unwrap());
    mismatch_and_bad_payload.default = Some(Literal::value(
        "core/i32@1".parse().unwrap(),
        json!({"sentinel":"private-payload"}),
    ));
    let error = build(vec![mismatch_and_bad_payload], &[]).unwrap_err();
    assert!(matches!(error, RegistryError::DefaultType { .. }));
    assert!(!error.to_string().contains("private-payload"));

    let mut invalid_payload = PortDescriptor::data_in("value", "core/i64@1".parse().unwrap());
    invalid_payload.default = Some(Literal::value("core/i64@1".parse().unwrap(), json!(7)));
    assert!(matches!(
        build(vec![invalid_payload], &[]),
        Err(RegistryError::DefaultLiteral { .. })
    ));
}

#[test]
fn registered_record_and_enum_defaults_are_checked_against_their_schemas() {
    let mut types = TypeRegistryBuilder::core();
    types
        .register_record(
            "acme",
            &"acme/point@1".parse().unwrap(),
            RecordSchema {
                fields: vec![FieldSchema {
                    field_id: "x".into(),
                    ty: TypeTerm::parse(&"core/i64@1".parse().unwrap()).unwrap(),
                    optional: false,
                }],
            },
        )
        .unwrap();
    types
        .register_enum(
            "acme",
            &"acme/state@1".parse().unwrap(),
            EnumSchema {
                variants: vec![VariantSchema {
                    variant_id: "ready".into(),
                    payload: Some(TypeTerm::parse(&"core/string@1".parse().unwrap()).unwrap()),
                }],
            },
        )
        .unwrap();
    let types = std::sync::Arc::new(types.build().unwrap());

    let mut point = PortDescriptor::data_in("point", "acme/point@1".parse().unwrap());
    point.default = Some(Literal::value(
        "acme/point@1".parse().unwrap(),
        json!({"x":"4"}),
    ));
    let mut state = PortDescriptor::data_in("state", "acme/state@1".parse().unwrap());
    state.default = Some(Literal::value(
        "acme/state@1".parse().unwrap(),
        json!({"variant":"ready","payload":"ok"}),
    ));
    assert!(build_with_types(vec![point.clone(), state], &[], types.clone()).is_ok());

    point.default = Some(Literal::value(
        "acme/point@1".parse().unwrap(),
        json!({"x":4}),
    ));
    assert!(matches!(
        build_with_types(vec![point], &[], types.clone()),
        Err(RegistryError::DefaultLiteral { .. })
    ));

    let mut malformed_variant = PortDescriptor::data_in("state", "acme/state@1".parse().unwrap());
    malformed_variant.default = Some(Literal::value(
        "acme/state@1".parse().unwrap(),
        json!({"variant":"unknown","payload":"ok"}),
    ));
    let mut malformed_payload = PortDescriptor::data_in("state", "acme/state@1".parse().unwrap());
    malformed_payload.default = Some(Literal::value(
        "acme/state@1".parse().unwrap(),
        json!({"variant":"ready","payload":7}),
    ));
    for invalid in [malformed_variant, malformed_payload] {
        assert!(matches!(
            build_with_types(vec![invalid], &[], types.clone()),
            Err(RegistryError::DefaultLiteral { .. })
        ));
    }
}

#[test]
fn unresolved_and_generic_defaults_remain_deferred_and_handles_are_not_literals() {
    let mut unknown = PortDescriptor::data_in("unknown", "missing/type@1".parse().unwrap());
    unknown.default = Some(Literal::value("missing/type@1".parse().unwrap(), json!({})));
    let mut generic = PortDescriptor::data_in("generic", "core/list@1<$T>".parse().unwrap());
    generic.default = Some(Literal::value(
        "core/list@1<$T>".parse().unwrap(),
        json!([]),
    ));
    assert!(build(vec![unknown, generic], &["T"]).is_ok());

    let mut types = TypeRegistryBuilder::core();
    types
        .register_handle_kind("acme", &"acme/resource@1".parse().unwrap())
        .unwrap();
    let registry = std::sync::Arc::new(types.build().unwrap());
    for text in [
        "core/stream@1<core/i64@1>",
        "core/result@1<core/i64@1>",
        "core/error@1",
        "core/handle@1<acme/resource@1>",
    ] {
        let ty: TypeExpr = text.parse().unwrap();
        let mut port = PortDescriptor::data_in("resource", ty.clone());
        port.default = Some(Literal::value(ty, json!({})));
        assert!(
            matches!(
                build_with_types(vec![port], &[], registry.clone()),
                Err(RegistryError::DefaultLiteral { .. })
            ),
            "{text}"
        );
    }
}

#[test]
fn repairable_invalid_default_metadata_still_round_trips_through_serde() {
    let mut port = PortDescriptor::data_in("value", "core/i64@1".parse().unwrap());
    port.default = Some(Literal::value(
        "core/i64@1".parse().unwrap(),
        json!({"repair":true}),
    ));
    let wire = serde_json::to_value(&port).unwrap();
    assert_eq!(
        serde_json::from_value::<PortDescriptor>(wire).unwrap(),
        port
    );
    assert!(matches!(
        build(vec![port], &[]),
        Err(RegistryError::DefaultLiteral { .. })
    ));
}

#[test]
fn existing_descriptor_errors_precede_default_literal_publication_errors() {
    let mut bad_default = PortDescriptor::data_in("default", "core/i64@1".parse().unwrap());
    bad_default.default = Some(Literal::value(
        "core/i32@1".parse().unwrap(),
        json!({"invalid":true}),
    ));
    let mut bad_optional = PortDescriptor::data_in("optional", "core/i64@1".parse().unwrap());
    bad_optional.optional = Some(true);
    assert!(matches!(
        build(vec![bad_default, bad_optional], &[]),
        Err(RegistryError::OptionalType { .. })
    ));
}
