use orange_model::catalog::{NodeDefinition, Ports, RegistryBuilder, RegistryError};
use orange_model::document::literal::Literal;
use orange_model::document::{
    DefinitionRef, NodeKind, OutputSource, PortDescriptor, PortDirection, PortKind, TypeParam,
};
use orange_model::id::TypeExpr;
use orange_model::validation::HostOrigin;
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
