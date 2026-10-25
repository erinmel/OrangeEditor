use orange_model::catalog::{
    Determinism, NodeDefinition, Ports, RegistryBuilder, RegistryError, RegistrySnapshot,
    Volatility,
};
use orange_model::document::{DefinitionRef, NodeKind, OutputSource, PortDescriptor};
use orange_model::validation::HostOrigin;

fn origin() -> HostOrigin {
    HostOrigin::new("0.0.0", format!("sha256:{}", "a".repeat(64))).unwrap()
}
fn definition() -> NodeDefinition {
    NodeDefinition {
        definition: DefinitionRef {
            id: "acme/entropy".parse().unwrap(),
            version: 1,
        },
        config_version: 1,
        kind: NodeKind::Volatile,
        volatility: Some(Volatility::Entropy),
        determinism: Determinism::Seeded,
        suspends: None,
        type_params: vec![],
        ports: Ports::Declared(vec![
            PortDescriptor::data_in("limit", "core/u64@1".parse().unwrap()),
            PortDescriptor::data_out(
                "value",
                "core/u64@1".parse().unwrap(),
                OutputSource::Computed,
            ),
        ]),
        references: vec![],
        specializations: None,
        config_schema: None,
    }
}
fn build(definition: NodeDefinition) -> Result<std::sync::Arc<RegistrySnapshot>, RegistryError> {
    RegistryBuilder::core(origin())
        .definition(definition)
        .build()
}

#[test]
fn seeded_entropy_with_inputs_and_core_state_read_facts_publish() {
    let def = definition();
    let id = def.definition.clone();
    let snapshot = build(def).unwrap();
    let fresh = snapshot.definition(&id).unwrap();
    assert_eq!(fresh.volatility, Some(Volatility::Entropy));
    assert_eq!(fresh.determinism, Determinism::Seeded);
    let get = DefinitionRef {
        id: "core/get-variable".parse().unwrap(),
        version: 1,
    };
    assert_eq!(
        snapshot.definition(&get).unwrap().volatility,
        Some(Volatility::StateRead)
    );
    assert_eq!(
        snapshot.definition(&get).unwrap().determinism,
        Determinism::Deterministic
    );
}

#[test]
fn volatile_class_is_required_and_other_kinds_cannot_claim_it() {
    let mut missing = definition();
    missing.volatility = None;
    assert!(matches!(
        build(missing),
        Err(RegistryError::VolatilityKind { .. })
    ));
    let mut misplaced = definition();
    misplaced.kind = NodeKind::Pure;
    misplaced.determinism = Determinism::Deterministic;
    assert!(matches!(
        build(misplaced),
        Err(RegistryError::VolatilityKind { .. })
    ));
}

#[test]
fn volatile_definitions_cannot_publish_control_execution_ports() {
    for port in [
        PortDescriptor::exec_in("enter"),
        PortDescriptor::exec_out("done"),
    ] {
        let mut def = definition();
        let Ports::Declared(ports) = &mut def.ports else {
            unreachable!()
        };
        ports.push(port);
        assert!(matches!(
            build(def),
            Err(RegistryError::VolatileExecutionPort { .. })
        ));
    }
}

#[test]
fn packages_cannot_claim_host_state_or_clock_volatility() {
    for class in [Volatility::StateRead, Volatility::Clock] {
        let mut def = definition();
        def.volatility = Some(class);
        def.determinism = Determinism::Deterministic;
        assert!(matches!(
            build(def),
            Err(RegistryError::VolatilityAuthority { .. })
        ));
    }
}

#[test]
fn pure_and_entropy_determinism_are_checked_before_publication() {
    for determinism in [
        Determinism::Deterministic,
        Determinism::External,
        Determinism::Nondeterministic,
    ] {
        let mut def = definition();
        def.determinism = determinism;
        assert!(matches!(
            build(def),
            Err(RegistryError::DeterminismKind { .. })
        ));
    }
    for determinism in [
        Determinism::Seeded,
        Determinism::External,
        Determinism::Nondeterministic,
    ] {
        let mut def = definition();
        def.kind = NodeKind::Pure;
        def.volatility = None;
        def.determinism = determinism;
        assert!(matches!(
            build(def),
            Err(RegistryError::DeterminismKind { .. })
        ));
    }
}

#[test]
fn replacement_snapshots_retain_their_original_execution_declarations() {
    let mut old = definition();
    old.kind = NodeKind::Action;
    old.volatility = None;
    old.determinism = Determinism::External;
    let id = old.definition.clone();
    let mut replacement = old.clone();
    replacement.determinism = Determinism::Deterministic;
    let old = build(old).unwrap();
    let replacement = build(replacement).unwrap();
    assert_ne!(old.generation(), replacement.generation());
    assert_eq!(
        old.definition(&id).unwrap().determinism,
        Determinism::External
    );
    assert_eq!(
        replacement.definition(&id).unwrap().determinism,
        Determinism::Deterministic
    );
}
