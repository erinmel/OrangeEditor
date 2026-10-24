use orange_model::catalog::{NodeDefinition, RegistryBuilder, RegistryError};
use orange_model::document::{DefinitionRef, NodeKind, PortDescriptor, Suspends};
use orange_model::validation::HostOrigin;

fn origin() -> HostOrigin {
    HostOrigin::new("1.0.0", format!("sha256:{}", "0".repeat(64))).unwrap()
}

fn port(value: serde_json::Value) -> PortDescriptor {
    serde_json::from_value(value).unwrap()
}

fn definition(kind: NodeKind, ports: Vec<PortDescriptor>) -> NodeDefinition {
    NodeDefinition {
        definition: DefinitionRef {
            id: "acme/test".parse().unwrap(),
            version: 1,
        },
        config_version: 1,
        kind,
        suspends: match kind {
            NodeKind::Async | NodeKind::Iterator => Some(Suspends::Maybe),
            _ => None,
        },
        type_params: vec![],
        ports: orange_model::catalog::Ports::Declared(ports),
        references: vec![],
        specializations: None,
        config_schema: None,
    }
}

fn build(definition: NodeDefinition) -> Result<(), RegistryError> {
    RegistryBuilder::core(origin())
        .definition(definition)
        .build()
        .map(|_| ())
}

fn exec_out(id: &str) -> PortDescriptor {
    port(serde_json::json!({"port_id":id,"direction":"output","kind":"exec"}))
}

fn execution_output(available_on: Option<serde_json::Value>) -> PortDescriptor {
    let mut value = serde_json::json!({"port_id":"value","direction":"output","kind":"data","type":"core/i64@1","source":"execution"});
    if let Some(available_on) = available_on {
        value["available_on"] = available_on;
    }
    port(value)
}

#[test]
fn omitted_availability_defaults_to_then_and_explicit_outcomes_resolve() {
    assert!(build(definition(NodeKind::Action, vec![execution_output(None)])).is_ok());
    assert!(
        build(definition(
            NodeKind::Action,
            vec![
                exec_out("done"),
                execution_output(Some(serde_json::json!(["done"])))
            ]
        ))
        .is_ok()
    );
    assert!(
        build(definition(
            NodeKind::Async,
            vec![execution_output(Some(serde_json::json!(["cancelled"])))]
        ))
        .is_ok()
    );
    assert!(build(definition(NodeKind::Entry, vec![execution_output(None)])).is_ok());
    for (kind, outcomes) in [
        (NodeKind::Action, serde_json::json!(["then", "failed"])),
        (NodeKind::Async, serde_json::json!(["failed", "cancelled"])),
        (NodeKind::Iterator, serde_json::json!(["body", "completed"])),
    ] {
        assert!(build(definition(kind, vec![execution_output(Some(outcomes))])).is_ok());
    }
    assert!(
        build(definition(
            NodeKind::Iterator,
            vec![execution_output(Some(serde_json::json!(["completed"])))]
        ))
        .is_ok()
    );
}

#[test]
fn availability_is_only_valid_on_execution_data_outputs() {
    for source in ["computed", "state", "payload", "iteration"] {
        let candidate = port(
            serde_json::json!({"port_id":"value","direction":"output","kind":"data","type":"core/i64@1","source":source,"available_on":["then"]}),
        );
        assert!(
            matches!(
                build(definition(NodeKind::Action, vec![candidate])),
                Err(RegistryError::AvailableOnPosition { .. })
            ),
            "{source}"
        );
    }
    for candidate in [
        port(
            serde_json::json!({"port_id":"value","direction":"output","kind":"data","type":"core/i64@1","source":"computed","available_on":["then"]}),
        ),
        port(
            serde_json::json!({"port_id":"input","direction":"input","kind":"data","type":"core/i64@1","available_on":["then"]}),
        ),
        port(
            serde_json::json!({"port_id":"event","direction":"output","kind":"event","type":"core/i64@1","available_on":["then"]}),
        ),
    ] {
        assert!(matches!(
            build(definition(NodeKind::Action, vec![candidate])),
            Err(RegistryError::AvailableOnPosition { .. })
        ));
    }
}

#[test]
fn availability_requires_nonempty_unique_known_output_outcomes() {
    assert!(matches!(
        build(definition(
            NodeKind::Action,
            vec![execution_output(Some(serde_json::json!([])))]
        )),
        Err(RegistryError::AvailableOnEmpty { .. })
    ));
    assert!(matches!(
        build(definition(
            NodeKind::Action,
            vec![execution_output(Some(serde_json::json!(["then", "then"])))]
        )),
        Err(RegistryError::AvailableOnDuplicate { .. })
    ));
    for outcome in ["missing", "in", "error"] {
        assert!(
            matches!(
                build(definition(
                    NodeKind::Action,
                    vec![execution_output(Some(serde_json::json!([outcome])))]
                )),
                Err(RegistryError::AvailableOnOutcome { .. })
            ),
            "{outcome}"
        );
    }
    assert!(matches!(
        build(definition(
            NodeKind::Action,
            vec![
                port(serde_json::json!({"port_id":"done","direction":"input","kind":"exec"})),
                execution_output(Some(serde_json::json!(["done"])))
            ]
        )),
        Err(RegistryError::AvailableOnOutcome { .. })
    ));
}

#[test]
fn replacing_implicit_then_requires_an_explicit_available_outcome() {
    assert!(matches!(
        build(definition(
            NodeKind::Action,
            vec![exec_out("done"), execution_output(None)]
        )),
        Err(RegistryError::AvailableOnOutcome { .. })
    ));
}

#[test]
fn generic_execution_outputs_are_checked_without_changing_their_type() {
    let value = serde_json::json!({"port_id":"value","direction":"output","kind":"data","type":"$T","source":"execution","available_on":["then"]});
    let mut definition = definition(NodeKind::Action, vec![port(value)]);
    definition.type_params = vec![serde_json::from_value(serde_json::json!({"name":"T"})).unwrap()];
    assert!(build(definition).is_ok());
}

#[test]
fn data_and_event_outputs_cannot_be_named_as_execution_outcomes() {
    for kind in ["data", "event"] {
        let candidate = port(
            serde_json::json!({"port_id":"done","direction":"output","kind":kind,"type":"core/i64@1","source":"computed"}),
        );
        assert!(
            matches!(
                build(definition(
                    NodeKind::Action,
                    vec![
                        candidate,
                        execution_output(Some(serde_json::json!(["done"])))
                    ]
                )),
                Err(RegistryError::AvailableOnOutcome { .. })
            ),
            "{kind}"
        );
    }
    assert!(matches!(
        build(definition(
            NodeKind::Action,
            vec![
                exec_out("error"),
                execution_output(Some(serde_json::json!(["error"])))
            ]
        )),
        Err(RegistryError::ReservedPort { .. })
    ));
}

#[test]
fn missing_default_outcome_does_not_create_control_ports() {
    for kind in [NodeKind::Pure, NodeKind::Volatile, NodeKind::Iterator] {
        assert!(
            matches!(build(definition(kind, vec![execution_output(None)])), Err(RegistryError::AvailableOnOutcome { outcome, .. }) if outcome.as_str() == "then"),
            "{kind:?}"
        );
    }
}

#[test]
fn invalid_availability_stays_repairable_and_publication_does_not_rewrite_it() {
    let invalid = execution_output(Some(serde_json::json!(["missing"])));
    let wire = serde_json::to_value(&invalid).unwrap();
    let decoded: PortDescriptor = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(decoded, invalid);
    let definition = definition(NodeKind::Action, vec![decoded]);
    let error = build(definition.clone()).unwrap_err();
    assert!(
        matches!(error, RegistryError::AvailableOnOutcome { definition, port, outcome } if definition.as_str() == "acme/test" && port.as_str() == "value" && outcome.as_str() == "missing")
    );
    let orange_model::catalog::Ports::Declared(ports) = definition.ports else {
        unreachable!()
    };
    assert_eq!(serde_json::to_value(&ports[0]).unwrap(), wire);
}
