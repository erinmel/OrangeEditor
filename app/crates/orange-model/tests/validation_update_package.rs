mod common;

use std::sync::Arc;

use common::{minimal_document, node_id};
use orange_model::catalog::{
    NodeDefinition, Ports, ProviderDescriptor, ProviderKey, ReferenceField, ReferenceKind,
    RegistryBuilder, RegistrySnapshot, VariableAccess,
};
use orange_model::document::{
    DefinitionRef, OutputSource, PortDescriptor, PortDirection, ProviderBinding, SignatureSnapshot,
    Suspends,
};
use orange_model::types::{
    CapabilityEvidence, CapabilityRequest, Constraint, ConstraintId, ConstraintImplementationRef,
    NumericOperation, TypeRegistryBuilder,
};
use orange_model::validation::{
    DiagnosticCode, FunctionEvidence, HostOrigin, analyze_functions, validate_inputs,
    validate_nodes,
};
use orange_model::value::literal_codec::RecordSchema;
use orange_model::{GraphDocument, from_json};
use serde_json::{Value, json};

const PKG: &str = "org.test";
const VER: &str = "1.0.0";
const DIGEST: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const VARIABLE: &str = "30000000-0000-4000-8000-000000000001";

#[derive(Clone, Copy, Debug)]
enum CapabilityMode {
    Add,
    Num,
    Ord,
    Eq,
}

fn origin() -> HostOrigin {
    HostOrigin::new("0.1.0", format!("sha256:{}", "a".repeat(64))).unwrap()
}

fn implementation(id: &str) -> ConstraintImplementationRef {
    ConstraintImplementationRef {
        provider: ProviderBinding {
            package: PKG.to_owned(),
            version: VER.to_owned(),
            id: "types".to_owned(),
            extensions: Default::default(),
        },
        content_digest: DIGEST.to_owned(),
        implementation_id: id.to_owned(),
    }
}

fn type_registry(mode: CapabilityMode) -> Arc<orange_model::types::TypeRegistry> {
    let ty = "org.test/money@1".parse().unwrap();
    let term = orange_model::type_term::TypeTerm::parse(&ty).unwrap();
    let mut builder = TypeRegistryBuilder::core();
    builder
        .register_record(PKG, &ty, RecordSchema { fields: vec![] })
        .unwrap();
    match mode {
        CapabilityMode::Num => builder
            .register_constraint_implementation(
                PKG,
                &term,
                ConstraintId::Core(Constraint::Num),
                implementation("num-only"),
            )
            .unwrap(),
        CapabilityMode::Ord => builder
            .register_constraint_implementation(
                PKG,
                &term,
                ConstraintId::Core(Constraint::Ord),
                implementation("money-ord-v1"),
            )
            .unwrap(),
        CapabilityMode::Eq => builder
            .register_constraint_implementation(
                PKG,
                &term,
                ConstraintId::Core(Constraint::Eq),
                implementation("money-eq-v1"),
            )
            .unwrap(),
        CapabilityMode::Add => builder
            .register_numeric_operation_implementation(
                PKG,
                &term,
                NumericOperation::Add,
                implementation("money-add-v1"),
            )
            .unwrap(),
    }
    Arc::new(builder.build().unwrap())
}

fn registry(mode: CapabilityMode, package_shape: bool) -> Arc<RegistrySnapshot> {
    registry_with_access(mode, package_shape.then_some(VariableAccess::Update))
}

fn registry_with_access(
    mode: CapabilityMode,
    access: Option<VariableAccess>,
) -> Arc<RegistrySnapshot> {
    let mut builder = RegistryBuilder::core(origin()).types(type_registry(mode));
    if let Some(access) = access {
        let shared = matches!(
            access,
            VariableAccess::WriteShared | VariableAccess::UpdateShared
        );
        let definition = DefinitionRef {
            id: "org.test/update-money".parse().unwrap(),
            version: 1,
        };
        builder = builder
            .definition(NodeDefinition {
                definition: definition.clone(),
                config_version: 1,
                volatility: None,
                determinism: orange_model::catalog::Determinism::Deterministic,
                kind: if shared {
                    orange_model::document::NodeKind::Async
                } else {
                    orange_model::document::NodeKind::Action
                },
                suspends: Some(if shared {
                    Suspends::Maybe
                } else {
                    Suspends::Never
                }),
                type_params: vec![],
                ports: Ports::Variable(access),
                references: vec![ReferenceField {
                    pointer: "/variable".to_owned(),
                    kind: ReferenceKind::Variable,
                }],
                specializations: None,
                config_schema: None,
            })
            .provider(ProviderDescriptor {
                key: ProviderKey {
                    package: PKG.to_owned(),
                    version: VER.to_owned(),
                    content_digest: DIGEST.to_owned(),
                    id: "update-money".to_owned(),
                },
                implements: [(definition, 1)].into(),
                specializations: Default::default(),
            });
    }
    builder.build().unwrap()
}

fn variable_node(operation: &str) -> Value {
    json!({
        "id":node_id(1),"definition_id":"core/update-variable",
        "definition_version":1,"config_version":1,
        "config":{"variable":VARIABLE,"operation":operation}
    })
}

fn document(nodes: Value, ty: &str) -> GraphDocument {
    let signature = package_signature();
    let signature_key = orange_model::signature::semantic_key(&signature);
    let mut extra = json!({
        "package_lock":[{"id":PKG,"version":VER,"content_digest":DIGEST}],
        "variables":[{"id":VARIABLE,"name":"balance","type":ty,"scope":"run"}],
        "nodes":nodes,
        "signatures":{}
    });
    extra["signatures"][signature_key.as_str()] = serde_json::to_value(signature).unwrap();
    from_json(&minimal_document(extra)).unwrap()
}

fn package_signature() -> SignatureSnapshot {
    let ty: orange_model::id::TypeExpr = "org.test/money@1".parse().unwrap();
    SignatureSnapshot {
        kind: orange_model::document::NodeKind::Action,
        suspends: Some(Suspends::Never),
        ports: vec![
            PortDescriptor::data_in("value", ty.clone()),
            PortDescriptor {
                available_on: Some(vec!["then".parse().unwrap()]),
                ..PortDescriptor::data_out("old", ty.clone(), OutputSource::Execution)
            },
            PortDescriptor {
                available_on: Some(vec!["then".parse().unwrap()]),
                ..PortDescriptor::data_out("new", ty, OutputSource::Execution)
            },
        ],
        extensions: Default::default(),
    }
}

fn package_node(operation: &str) -> Value {
    let mut node = variable_node(operation);
    let signature = package_signature();
    node["definition_id"] = json!("org.test/update-money");
    node["provider"] = json!({"package":PKG,"version":VER,"id":"update-money"});
    node["signature"] = json!(orange_model::signature::semantic_key(&signature).as_str());
    node
}

#[test]
fn core_update_requires_the_exact_package_operation_witness() {
    let registry = registry(CapabilityMode::Add, false);
    let doc = document(json!([variable_node("add")]), "org.test/money@1");
    let report = validate_nodes(&doc, &registry);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert!(report.resolved.contains_key(&node_id(1).parse().unwrap()));

    let witness = registry
        .types()
        .supports_term(
            &orange_model::type_term::TypeTerm::parse(&"org.test/money@1".parse().unwrap())
                .unwrap(),
            CapabilityRequest::NumericOperation(NumericOperation::Add),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        witness.evidence,
        CapabilityEvidence::PackageImplementation(implementation("money-add-v1"))
    );

    let subtract = document(json!([variable_node("sub")]), "org.test/money@1");
    let report = validate_nodes(&subtract, &registry);
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(
        report.diagnostics[0].code,
        DiagnosticCode::ConstraintUnsatisfied
    );

    let incompatible = document(json!([variable_node("append")]), "org.test/money@1");
    let report = validate_nodes(&incompatible, &registry);
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(
        report.diagnostics[0].code,
        DiagnosticCode::ConstraintUnsatisfied
    );
}

#[test]
fn update_min_max_require_and_accept_exact_package_ord_evidence() {
    let registry = registry(CapabilityMode::Ord, false);
    let ty =
        orange_model::type_term::TypeTerm::parse(&"org.test/money@1".parse().unwrap()).unwrap();
    let witness = registry
        .types()
        .supports_term(&ty, CapabilityRequest::Constraint(Constraint::Ord))
        .unwrap()
        .unwrap();
    assert_eq!(
        witness.evidence,
        CapabilityEvidence::PackageImplementation(implementation("money-ord-v1"))
    );
    assert_eq!(witness.term, ty);

    for operation in ["min", "max"] {
        let doc = document(json!([variable_node(operation)]), "org.test/money@1");
        let report = validate_nodes(&doc, &registry);
        assert!(
            report.diagnostics.is_empty(),
            "{operation}: {:?}",
            report.diagnostics
        );
        assert!(report.resolved.contains_key(&node_id(1).parse().unwrap()));
    }
}

#[test]
fn broad_or_unrelated_evidence_does_not_authorize_update_operations() {
    for (mode, operation) in [
        (CapabilityMode::Num, "add"),
        (CapabilityMode::Num, "min"),
        (CapabilityMode::Add, "min"),
    ] {
        let registry = registry(mode, false);
        let doc = document(json!([variable_node(operation)]), "org.test/money@1");
        let report = validate_nodes(&doc, &registry);
        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(
            report.diagnostics[0].code,
            DiagnosticCode::ConstraintUnsatisfied
        );
    }
}

#[test]
fn compare_and_set_requires_exact_package_eq_witness() {
    let registry = registry(CapabilityMode::Eq, false);
    let ty =
        orange_model::type_term::TypeTerm::parse(&"org.test/money@1".parse().unwrap()).unwrap();
    let witness = registry
        .types()
        .supports_term(&ty, CapabilityRequest::Constraint(Constraint::Eq))
        .unwrap()
        .unwrap();
    assert_eq!(
        witness.evidence,
        CapabilityEvidence::PackageImplementation(implementation("money-eq-v1"))
    );
    assert_eq!(witness.term, ty);

    let doc = document(
        json!([variable_node("compare-and-set")]),
        "org.test/money@1",
    );
    let report = validate_nodes(&doc, &registry);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let signature = &report.resolved[&node_id(1).parse().unwrap()];
    let inputs: Vec<_> = signature
        .ports
        .iter()
        .filter(|port| port.direction == PortDirection::Input)
        .map(|port| (port.port_id.as_str(), port.ty.as_ref().unwrap().as_str()))
        .collect();
    assert_eq!(
        inputs,
        [
            ("expected", "org.test/money@1"),
            ("value", "org.test/money@1")
        ]
    );
    assert!(signature.ports.iter().any(|port| {
        port.direction == PortDirection::Output
            && port.port_id.as_str() == "exchanged"
            && port
                .ty
                .as_ref()
                .is_some_and(|ty| ty.as_str() == "core/bool@1")
            && port.source == Some(OutputSource::Execution)
            && port
                .available_on
                .as_ref()
                .is_some_and(|triggers| triggers.len() == 1 && triggers[0].as_str() == "then")
    }));
}

#[test]
fn compare_and_set_rejects_non_eq_package_evidence() {
    for mode in [
        CapabilityMode::Num,
        CapabilityMode::Add,
        CapabilityMode::Ord,
    ] {
        let registry = registry(mode, false);
        let doc = document(
            json!([variable_node("compare-and-set")]),
            "org.test/money@1",
        );
        let report = validate_nodes(&doc, &registry);
        assert_eq!(report.diagnostics.len(), 1, "{mode:?}");
        assert_eq!(
            report.diagnostics[0].code,
            DiagnosticCode::ConstraintUnsatisfied,
            "{mode:?}"
        );
    }
}

#[test]
fn package_constraints_do_not_declare_logical_or_bitwise_kernels() {
    for mode in [
        CapabilityMode::Num,
        CapabilityMode::Add,
        CapabilityMode::Ord,
        CapabilityMode::Eq,
    ] {
        let registry = registry(mode, false);
        for operation in ["and", "or", "xor"] {
            let doc = document(json!([variable_node(operation)]), "org.test/money@1");
            let report = validate_nodes(&doc, &registry);
            assert_eq!(report.diagnostics.len(), 1, "{mode:?}/{operation}");
            assert_eq!(
                report.diagnostics[0].code,
                DiagnosticCode::ConstraintUnsatisfied,
                "{mode:?}/{operation}"
            );
            assert!(report.resolved.is_empty());
        }
    }
}

#[test]
fn append_accepts_package_elements_without_inventing_nominal_concat_kernels() {
    for mode in [
        CapabilityMode::Add,
        CapabilityMode::Num,
        CapabilityMode::Ord,
        CapabilityMode::Eq,
    ] {
        let registry = registry(mode, false);
        let list = document(
            json!([variable_node("append")]),
            "core/list@1<org.test/money@1>",
        );
        let report = validate_nodes(&list, &registry);
        assert!(report.is_valid(), "{mode:?}: {:?}", report.diagnostics);
        assert_eq!(
            report.resolved[&node_id(1).parse().unwrap()].ports[0]
                .ty
                .as_ref()
                .unwrap()
                .as_str(),
            "core/list@1<org.test/money@1>"
        );
        let nominal = document(json!([variable_node("append")]), "org.test/money@1");
        let report = validate_nodes(&nominal, &registry);
        assert_eq!(
            report.diagnostics[0].code,
            DiagnosticCode::ConstraintUnsatisfied
        );
        assert!(report.resolved.is_empty());
    }
}

#[test]
fn package_update_shape_projects_ports_without_claiming_core_effects() {
    let registry = registry(CapabilityMode::Add, true);
    let mut node = package_node("add");
    node["inputs"] = json!({"value":{"type":"org.test/money@1","value":{}}});
    let doc = document(json!([node]), "org.test/money@1");
    let before = orange_model::to_json(&doc).unwrap();
    let report = validate_nodes(&doc, &registry);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert!(validate_inputs(&doc, &report.resolved, &registry).is_empty());
    let signature = &report.resolved[&node_id(1).parse().unwrap()];
    assert_eq!(signature.kind, orange_model::document::NodeKind::Action);
    let value = signature
        .ports
        .iter()
        .find(|port| port.port_id.as_str() == "value")
        .unwrap();
    assert_eq!(value.direction, PortDirection::Input);
    assert_eq!(value.ty.as_ref().unwrap().as_str(), "org.test/money@1");
    assert!(signature.ports.iter().any(|port| {
        port.direction == PortDirection::Output
            && port.source == Some(OutputSource::Execution)
            && port.port_id.as_str() == "old"
            && port
                .available_on
                .as_ref()
                .is_some_and(|triggers| triggers.len() == 1 && triggers[0].as_str() == "then")
    }));
    assert_eq!(orange_model::to_json(&doc).unwrap(), before);
}

#[test]
fn package_shared_shape_still_requires_trusted_function_effects() {
    let registry = registry_with_access(CapabilityMode::Add, Some(VariableAccess::UpdateShared));
    let mut signature = package_signature();
    signature.kind = orange_model::document::NodeKind::Async;
    signature.suspends = Some(Suspends::Maybe);
    let signature_key = orange_model::signature::semantic_key(&signature);
    let mut node = package_node("add");
    node["signature"] = json!(signature_key.as_str());
    let function = "40000000-0000-4000-8000-000000000001";
    let mut fixture = json!({
        "package_lock":[{"id":PKG,"version":VER,"content_digest":DIGEST}],
        "variables":[{"id":VARIABLE,"name":"balance","type":"org.test/money@1","scope":"workspace"}],
        "functions":[{"id":function,"name":"adjust","kind":"async","nodes":[node],"edges":[]}],
        "signatures":{}
    });
    fixture["signatures"][signature_key.as_str()] = serde_json::to_value(signature).unwrap();
    let doc = from_json(&minimal_document(fixture)).unwrap();
    let nodes = validate_nodes(&doc, &registry);
    assert!(nodes.is_valid(), "{:?}", nodes.diagnostics);
    let missing = analyze_functions(
        &doc,
        &FunctionEvidence::from_nodes(&doc, &nodes, Default::default()),
        &registry,
    );
    assert!(
        missing
            .incomplete_functions
            .contains(&function.parse().unwrap())
    );
    let evidence = FunctionEvidence::from_nodes(
        &doc,
        &nodes,
        [(
            node_id(1).parse().unwrap(),
            orange_model::validation::LeafEffects::Known(vec![]),
        )]
        .into(),
    );
    let trusted = analyze_functions(&doc, &evidence, &registry);
    let summary = &trusted.summaries[&function.parse().unwrap()];
    assert_eq!(summary.kind, orange_model::document::NodeKind::Async);
    assert_eq!(summary.suspends, Suspends::Maybe);
    assert!(summary.run_writes.is_empty());
}

#[test]
fn package_update_shape_does_not_supply_function_leaf_effects() {
    let registry = registry(CapabilityMode::Add, true);
    let node = package_node("add");
    let signature = package_signature();
    let signature_key = orange_model::signature::semantic_key(&signature);
    let mut extra = json!({
        "package_lock":[{"id":PKG,"version":VER,"content_digest":DIGEST}],
        "variables":[{"id":VARIABLE,"name":"balance","type":"org.test/money@1","scope":"run"}],
        "functions":[{
            "id":"40000000-0000-4000-8000-000000000001",
            "name":"adjust","kind":"action","nodes":[node],"edges":[]
        }],
        "signatures":{}
    });
    extra["signatures"][signature_key.as_str()] = serde_json::to_value(signature).unwrap();
    let doc = from_json(&minimal_document(extra)).unwrap();
    let nodes = validate_nodes(&doc, &registry);
    assert!(nodes.diagnostics.is_empty(), "{:?}", nodes.diagnostics);
    let missing = FunctionEvidence::from_nodes(&doc, &nodes, Default::default());
    let functions = analyze_functions(&doc, &missing, &registry);
    assert!(
        functions
            .incomplete_functions
            .contains(&"40000000-0000-4000-8000-000000000001".parse().unwrap())
    );

    let node_id = node_id(1).parse().unwrap();
    let empty_effects = FunctionEvidence::from_nodes(
        &doc,
        &nodes,
        [(
            node_id,
            orange_model::validation::LeafEffects::Known(vec![]),
        )]
        .into_iter()
        .collect(),
    );
    let complete = analyze_functions(&doc, &empty_effects, &registry);
    let function_id = "40000000-0000-4000-8000-000000000001".parse().unwrap();
    assert!(!complete.incomplete_functions.contains(&function_id));
    assert!(complete.summaries[&function_id].run_writes.is_empty());
}
