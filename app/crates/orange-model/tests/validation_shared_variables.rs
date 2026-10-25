mod common;

use common::{minimal_document, nid, node_id};
use orange_model::catalog::{
    ContractError, NodeDefinition, Ports, ReferenceKind, RegistryBuilder, RegistryError,
    RegistrySnapshot, VariableAccess,
};
use orange_model::document::{DefinitionRef, NodeKind, OutputSource, PortDirection, Suspends};
use orange_model::validation::{DiagnosticCode, HostOrigin, validate_inputs, validate_nodes};
use orange_model::{DocCommand, GraphDocument, from_json};
use serde_json::{Value, json};

fn origin() -> HostOrigin {
    HostOrigin::new("0.1.0", format!("sha256:{}", "a".repeat(64))).unwrap()
}

fn registry() -> std::sync::Arc<RegistrySnapshot> {
    RegistrySnapshot::core(origin())
}

fn declaration(id: u8, ty: &str, scope: &str) -> Value {
    json!({"id":node_id(id),"name":"shared","type":ty,"scope":scope})
}

fn shared_node(id: u8, operation: &str, variable: Value) -> Value {
    let definition = if operation == "set" {
        "core/set-shared"
    } else {
        "core/update-shared"
    };
    let mut config = json!({"variable":variable});
    if operation != "set" {
        config["operation"] = json!(operation);
    }
    json!({"id":node_id(id),"definition_id":definition,"definition_version":1,
        "config_version":1,"config":config})
}

fn document(nodes: Value, variables: Value) -> GraphDocument {
    from_json(&minimal_document(
        json!({"nodes":nodes,"variables":variables}),
    ))
    .unwrap()
}

#[test]
fn set_shared_is_async_and_uses_only_a_required_declaration_typed_input() {
    let registry = registry();
    for ty in ["core/i64@1", "core/bool@1"] {
        let doc = document(
            json!([shared_node(1, "set", json!(node_id(10)))]),
            json!([declaration(10, ty, "workspace")]),
        );
        let nodes = validate_nodes(&doc, &registry);
        assert!(nodes.diagnostics.is_empty(), "{:?}", nodes.diagnostics);
        let contract = registry
            .contract(&doc, doc.node(nid(1)).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(contract.kind, NodeKind::Async);
        assert_eq!(contract.suspends, Some(Suspends::Maybe));
        assert_eq!(contract.ports.len(), 1);
        assert_eq!(contract.ports[0].port_id.as_str(), "value");
        assert_eq!(contract.ports[0].direction, PortDirection::Input);
        assert_eq!(contract.ports[0].ty.as_ref().unwrap().as_str(), ty);
        assert!(contract.ports[0].default.is_none());
        assert!(!contract.ports[0].optional.unwrap_or(false));
        assert_eq!(
            validate_inputs(&doc, &nodes.resolved, &registry)
                .iter()
                .map(|d| d.code)
                .collect::<Vec<_>>(),
            [DiagnosticCode::InputRequired]
        );
        assert!(
            contract
                .ports
                .iter()
                .all(|p| p.direction != PortDirection::Output)
        );
    }
}

#[test]
fn update_shared_reuses_all_update_operation_proofs_and_port_shapes() {
    let registry = registry();
    for (operation, ty) in [
        ("add", "core/i64@1"),
        ("sub", "core/i64@1"),
        ("min", "core/i64@1"),
        ("max", "core/i64@1"),
        ("and", "core/i64@1"),
        ("or", "core/i64@1"),
        ("xor", "core/i64@1"),
        ("append", "core/list@1<core/f64@1>"),
        ("compare-and-set", "core/i64@1"),
    ] {
        let doc = document(
            json!([shared_node(1, operation, json!(node_id(10)))]),
            json!([declaration(10, ty, "workspace")]),
        );
        let nodes = validate_nodes(&doc, &registry);
        assert!(
            nodes.diagnostics.is_empty(),
            "{operation}: {:?}",
            nodes.diagnostics
        );
        let contract = registry
            .contract(&doc, doc.node(nid(1)).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(contract.kind, NodeKind::Async);
        assert_eq!(contract.suspends, Some(Suspends::Maybe));
        let expected: &[&str] = if operation == "compare-and-set" {
            &["expected", "value", "old", "new", "exchanged"]
        } else {
            &["value", "old", "new"]
        };
        assert_eq!(
            contract
                .ports
                .iter()
                .map(|p| p.port_id.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        for port in &contract.ports {
            assert!(port.default.is_none());
            assert!(!port.optional.unwrap_or(false));
            if matches!(port.port_id.as_str(), "old" | "new" | "exchanged") {
                assert_eq!(port.direction, PortDirection::Output);
                assert_eq!(port.source, Some(OutputSource::Execution));
                assert_eq!(port.available_on.as_ref().unwrap()[0].as_str(), "then");
            } else {
                assert_eq!(port.direction, PortDirection::Input);
                assert_eq!(port.ty.as_ref().unwrap().as_str(), ty);
            }
        }
        if operation == "compare-and-set" {
            assert_eq!(
                contract.ports[4].ty.as_ref().unwrap().as_str(),
                "core/bool@1"
            );
        }
        let required = validate_inputs(&doc, &nodes.resolved, &registry);
        assert_eq!(
            required.iter().map(|d| d.code).collect::<Vec<_>>(),
            if operation == "compare-and-set" {
                vec![DiagnosticCode::InputRequired; 2]
            } else {
                vec![DiagnosticCode::InputRequired]
            }
        );
    }
}

#[test]
fn shared_scope_precedes_operation_and_sync_contracts_remain_separate() {
    let registry = registry();
    for scope in ["local", "run"] {
        let doc = document(
            json!([shared_node(1, "not-an-operation", json!(node_id(10)))]),
            json!([declaration(10, "core/i64@1", scope)]),
        );
        assert_eq!(
            registry
                .contract(&doc, doc.node(nid(1)).unwrap())
                .unwrap_err(),
            ContractError::SharedVariableScope
        );
    }
    let malformed = document(
        json!([shared_node(1, "not-an-operation", json!(node_id(10)))]),
        json!([declaration(10, "core/i64@1", "workspace")]),
    );
    assert_eq!(
        registry
            .contract(&malformed, malformed.node(nid(1)).unwrap())
            .unwrap_err(),
        ContractError::UpdateOperation
    );
    for definition in ["core/set-variable", "core/update-variable"] {
        let node = if definition.ends_with("set-variable") {
            json!({"id":node_id(1),"definition_id":definition,"definition_version":1,
                "config_version":1,"config":{"variable":node_id(10)}})
        } else {
            json!({"id":node_id(1),"definition_id":definition,"definition_version":1,
                "config_version":1,"config":{"variable":node_id(10),"operation":"add"}})
        };
        let doc = document(
            json!([node]),
            json!([declaration(10, "core/i64@1", "workspace")]),
        );
        assert_eq!(
            registry
                .contract(&doc, doc.node(nid(1)).unwrap())
                .unwrap_err(),
            ContractError::VariableScope
        );
    }
}

#[test]
fn shared_contracts_reject_bad_reference_identity_and_unresolved_types() {
    let registry = registry();
    for bad_reference in [Value::Null, json!(42), json!(node_id(99))] {
        let doc = document(
            json!([shared_node(1, "set", bad_reference)]),
            json!([declaration(10, "core/i64@1", "workspace")]),
        );
        assert_eq!(
            registry
                .contract(&doc, doc.node(nid(1)).unwrap())
                .unwrap_err(),
            ContractError::VariableUnknown
        );
    }
    let unknown = document(
        json!([shared_node(1, "add", json!(node_id(10)))]),
        json!([declaration(10, "missing/type@1", "workspace")]),
    );
    assert_eq!(
        registry
            .contract(&unknown, unknown.node(nid(1)).unwrap())
            .unwrap_err(),
        ContractError::TypeMismatch
    );

    for (field, value) in [
        (
            "provider",
            json!({"package":"acme.test","version":"1.0.0","id":"custom"}),
        ),
        ("definition_version", json!(2)),
        ("config_version", json!(2)),
    ] {
        let mut node = shared_node(1, "set", json!(node_id(10)));
        node[field] = value;
        let doc = document(
            json!([node]),
            json!([declaration(10, "core/i64@1", "workspace")]),
        );
        let report = validate_nodes(&doc, &registry);
        assert_eq!(
            report.diagnostics[0].code,
            DiagnosticCode::ProviderUnavailable
        );
        assert!(report.resolved.is_empty());
    }
}

#[test]
fn shared_contract_is_read_only_and_tracks_declaration_edits_and_undo() {
    let registry = registry();
    let mut doc = document(
        json!([shared_node(1, "add", json!(node_id(10)))]),
        json!([declaration(10, "core/i64@1", "workspace")]),
    );
    let before = serde_json::to_value(&doc).unwrap();
    let revision = doc.revision();
    let state = doc.state_id();
    let ty = |doc: &GraphDocument| {
        registry
            .contract(doc, doc.node(nid(1)).unwrap())
            .unwrap()
            .unwrap()
            .ports[0]
            .ty
            .as_ref()
            .unwrap()
            .as_str()
            .to_owned()
    };
    assert_eq!(ty(&doc), "core/i64@1");
    assert_eq!(serde_json::to_value(&doc).unwrap(), before);
    assert_eq!(doc.revision(), revision);
    assert_eq!(doc.state_id(), state);

    let changed = doc
        .apply([DocCommand::UpsertVariable(
            serde_json::from_value(declaration(10, "core/f64@1", "workspace")).unwrap(),
        )])
        .unwrap();
    assert_eq!(ty(&doc), "core/f64@1");
    let undone = doc.revert(changed.inverse).unwrap();
    assert_eq!(ty(&doc), "core/i64@1");
    let _ = doc.revert(undone.inverse).unwrap();
    assert_eq!(ty(&doc), "core/f64@1");
}

#[test]
fn package_definitions_can_reuse_async_shared_metadata_without_core_effect_authority() {
    let package_shared = NodeDefinition {
        definition: DefinitionRef {
            id: "acme.test/shared-write".parse().unwrap(),
            version: 1,
        },
        config_version: 1,
        kind: NodeKind::Async,
        volatility: None,
        determinism: orange_model::catalog::Determinism::Deterministic,
        suspends: Some(Suspends::Maybe),
        type_params: Vec::new(),
        ports: Ports::Variable(VariableAccess::WriteShared),
        references: vec![orange_model::catalog::ReferenceField {
            pointer: "/variable".into(),
            kind: ReferenceKind::Variable,
        }],
        specializations: None,
        config_schema: None,
    };
    let package_registry = RegistryBuilder::core(origin())
        .definition(package_shared.clone())
        .build()
        .unwrap();
    let doc = document(
        json!([{"id":node_id(1),"definition_id":"acme.test/shared-write",
            "definition_version":1,"config_version":1,
            "config":{"variable":node_id(10)}}]),
        json!([declaration(10, "core/i64@1", "workspace")]),
    );
    let contract = package_registry
        .contract(&doc, doc.node(nid(1)).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(contract.kind, NodeKind::Async);
    assert_eq!(contract.suspends, Some(Suspends::Maybe));
    assert_eq!(contract.ports.len(), 1);
    assert_eq!(contract.ports[0].port_id.as_str(), "value");
    assert_eq!(contract.ports[0].direction, PortDirection::Input);
    assert_eq!(
        contract.ports[0].ty.as_ref().unwrap().as_str(),
        "core/i64@1"
    );

    let mut wrong_kind = package_shared;
    wrong_kind.kind = NodeKind::Action;
    wrong_kind.suspends = Some(Suspends::Never);
    assert!(matches!(
        RegistryBuilder::core(origin())
            .definition(wrong_kind)
            .build(),
        Err(RegistryError::SharedVariableKind {
            kind: NodeKind::Action,
            ..
        })
    ));
}
