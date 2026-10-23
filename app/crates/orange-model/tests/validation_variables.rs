mod common;

use common::{minimal_document, nid, node_id};
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{NodeKind, OutputSource, PortDirection, Suspends};
use orange_model::validation::{
    DiagnosticCode, HostOrigin, NodeValidation, validate_inputs, validate_nodes,
};
use orange_model::{GraphDocument, from_json, to_json};
use serde_json::{Value, json};

fn origin() -> HostOrigin {
    HostOrigin::new("0.1.0", format!("sha256:{}", "a".repeat(64))).unwrap()
}

fn registry() -> std::sync::Arc<RegistrySnapshot> {
    RegistrySnapshot::core(origin())
}
fn declaration(n: u8, ty: &str, scope: &str) -> Value {
    json!({"id":node_id(n), "name":"same display name", "type":ty,"scope":scope})
}
fn variable_node(n: u8, operation: &str, variable: Value) -> Value {
    json!({"id":node_id(n),"definition_id":format!("core/{operation}-variable"),
        "definition_version":1,"config_version":1,"config":{"variable":variable},
        "type_args":{"T":"core/string@1"}})
}
fn update_node(n: u8, variable: Value, operation: Value) -> Value {
    json!({"id":node_id(n),"definition_id":"core/update-variable",
        "definition_version":1,"config_version":1,
        "config":{"variable":variable,"operation":operation}})
}
fn document(nodes: Value, variables: Value) -> GraphDocument {
    from_json(&minimal_document(
        json!({"nodes":nodes,"variables":variables}),
    ))
    .unwrap()
}
fn validate(doc: &GraphDocument) -> NodeValidation {
    let before = to_json(doc).unwrap();
    let revision = doc.revision();
    let state = doc.state_id();
    let report = validate_nodes(doc, &registry());
    assert_eq!(to_json(doc).unwrap(), before);
    assert_eq!(doc.revision(), revision);
    assert_eq!(doc.state_id(), state);
    report
}
#[test]
fn get_nodes_resolve_distinct_declaration_types() {
    let doc = document(
        json!([
            variable_node(1, "get", json!(node_id(10))),
            variable_node(2, "get", json!(node_id(11)))
        ]),
        json!([
            declaration(10, "core/i64@1", "run"),
            declaration(11, "core/bool@1", "workspace")
        ]),
    );
    let report = validate(&doc);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    for (n, ty) in [(1, "core/i64@1"), (2, "core/bool@1")] {
        let signature = &report.resolved[&nid(n)];
        assert_eq!(signature.kind, NodeKind::Volatile);
        assert_eq!(signature.suspends, Some(Suspends::Never));
        assert_eq!(signature.ports.len(), 1);
        let port = &signature.ports[0];
        assert_eq!(port.port_id.as_str(), "value");
        assert_eq!(port.direction, PortDirection::Output);
        assert_eq!(port.source, Some(OutputSource::State));
        assert_eq!(port.ty.as_ref().unwrap().as_str(), ty);
    }
}
#[test]
fn get_reference_errors_are_incomplete() {
    for (operation, reference) in [
        ("get", None),
        ("get", Some(Value::Null)),
        ("get", Some(json!(42))),
        ("get", Some(json!("bad"))),
        ("get", Some(json!(node_id(99)))),
        ("set", None),
        ("set", Some(json!(node_id(99)))),
    ] {
        let mut node = variable_node(1, operation, reference.unwrap_or(Value::Null));
        if node["config"]["variable"].is_null() {
            node["config"].as_object_mut().unwrap().remove("variable");
        }
        let doc = document(json!([node]), json!([]));
        let report = validate(&doc);
        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(report.diagnostics[0].code, DiagnosticCode::VariableUnknown);
        assert_eq!(report.diagnostics[0].subject.node, Some(nid(1)));
        assert!(report.resolved.is_empty());
        assert!(report.incomplete_nodes.contains(&nid(1)));
    }
}
#[test]
fn set_uses_declaration_type_and_requires_value() {
    for scope in ["local", "run"] {
        for (literal, expected) in [
            (None, Some(DiagnosticCode::InputRequired)),
            (Some(json!({"type":"core/i64@1","value":"7"})), None),
            (
                Some(json!({"type":"core/bool@1","value":true})),
                Some(DiagnosticCode::LiteralType),
            ),
        ] {
            let mut node = variable_node(1, "set", json!(node_id(10)));
            if let Some(literal) = literal {
                node["inputs"] = json!({"value":literal});
            }
            let doc = document(json!([node]), json!([declaration(10, "core/i64@1", scope)]));
            let report = validate(&doc);
            assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
            let signature = &report.resolved[&nid(1)];
            assert_eq!(signature.kind, NodeKind::Action);
            assert_eq!(signature.suspends, Some(Suspends::Never));
            assert_eq!(signature.ports.len(), 1);
            assert_eq!(signature.ports[0].direction, PortDirection::Input);
            assert_eq!(signature.ports[0].source, None);
            assert_eq!(signature.ports[0].default, None);
            assert_eq!(
                signature.ports[0].ty.as_ref().unwrap().as_str(),
                "core/i64@1"
            );
            let inputs = validate_inputs(&doc, &report.resolved, &registry());
            assert_eq!(
                inputs.iter().map(|d| d.code).collect::<Vec<_>>(),
                expected.into_iter().collect::<Vec<_>>()
            );
        }
    }
}
#[test]
fn set_workspace_requires_shared_operation() {
    let doc = document(
        json!([variable_node(1, "set", json!(node_id(10)))]),
        json!([declaration(10, "core/i64@1", "workspace")]),
    );
    let report = validate(&doc);
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].code.as_str(), "E-VARIABLE-SCOPE");
    assert_eq!(
        report.diagnostics[0].message_key,
        "core/graph-variable-scope"
    );
    assert_eq!(report.diagnostics[0].message_origin, origin());
    assert!(report.diagnostics[0].message_args.is_empty());
    assert_eq!(
        report.diagnostics[0].message,
        "Workspace variables require an asynchronous shared write."
    );
    assert_eq!(report.diagnostics[0].subject.node, Some(nid(1)));
    assert!(report.resolved.is_empty());
    assert!(report.incomplete_nodes.contains(&nid(1)));
}

#[test]
fn get_drives_generic_math_and_set_checks_result() {
    use common::generic;
    use orange_model::catalog::ProviderKey;
    let signature = format!("sha256:{}", "b".repeat(64));
    let digest = format!("sha256:{}", "c".repeat(64));
    let binding = json!({"package":"org.test","version":"1.0.0","id":"native"});
    let registry = generic::registry_with_provider(ProviderKey {
        package: "org.test".into(),
        version: "1.0.0".into(),
        content_digest: digest.clone(),
        id: "native".into(),
    });
    for (target, valid) in [("core/i64@1", true), ("core/bool@1", false)] {
        let doc = from_json(&minimal_document(json!({
            "variables":[declaration(10,"core/i64@1","run"),declaration(11,target,"run")],
            "package_lock":[{"id":"org.test","version":"1.0.0","content_digest":digest}],
            "nodes":[variable_node(1,"get",json!(node_id(10))),
                generic::add(2,json!({"provider":binding,"signature":signature,"inputs":{"b":{"type":"core/i64@1","value":"1"}}})),
                variable_node(3,"set",json!(node_id(11)))],
            "edges":[generic::data_edge(1,(1,"value"),(2,"a")),generic::data_edge(2,(2,"sum"),(3,"value"))],
            "signatures":{(signature.clone()):{"kind":"pure","ports":[
                {"port_id":"a","direction":"input","kind":"data","type":"core/i64@1"},
                {"port_id":"b","direction":"input","kind":"data","type":"core/i64@1"},
                {"port_id":"sum","direction":"output","kind":"data","type":"core/i64@1","source":"computed"}]}}
        }))).unwrap();
        let before = to_json(&doc).unwrap();
        let report = validate_nodes(&doc, &registry);
        if valid {
            assert!(report.is_valid(), "{:?}", report.diagnostics);
            assert!(validate_inputs(&doc, &report.resolved, &registry).is_empty());
            for port in &report.resolved[&nid(2)].ports {
                assert_eq!(port.ty.as_ref().unwrap().as_str(), "core/i64@1");
            }
        } else {
            assert!(
                report
                    .diagnostics
                    .iter()
                    .any(|d| d.code == DiagnosticCode::TypeMismatch)
            );
            assert!(!report.incomplete_nodes.is_empty());
        }
        assert_eq!(to_json(&doc).unwrap(), before);
    }
}

#[test]
fn update_requires_exact_intrinsic_identity_and_canonical_variable_id() {
    for (field, value) in [
        (
            "provider",
            json!({"package":"org.test","version":"1.0.0","id":"native"}),
        ),
        ("definition_version", json!(2)),
        ("config_version", json!(2)),
    ] {
        let mut node = update_node(1, json!(node_id(10)), json!("add"));
        node[field] = value;
        let report = validate(&document(
            json!([node]),
            json!([declaration(10, "core/i64@1", "run")]),
        ));
        assert_eq!(
            report.diagnostics[0].code,
            DiagnosticCode::ProviderUnavailable
        );
        assert!(report.resolved.is_empty());
    }

    let report = validate(&document(
        json!([update_node(1, json!("not-an-id"), json!("add"))]),
        json!([declaration(10, "core/i64@1", "run")]),
    ));
    assert_eq!(report.diagnostics[0].code, DiagnosticCode::VariableUnknown);
    assert!(report.resolved.is_empty());
}

#[test]
fn update_type_depth_limit_has_complexity_diagnostic() {
    let depth = 66;
    let ty = format!(
        "{}core/i64@1{}",
        "core/list@1<".repeat(depth),
        ">".repeat(depth)
    );
    let report = validate(&document(
        json!([update_node(1, json!(node_id(10)), json!("add"))]),
        json!([declaration(10, &ty, "run")]),
    ));
    assert_eq!(report.diagnostics[0].code, DiagnosticCode::TypeComplexity);
    assert!(report.resolved.is_empty());
}
#[test]
fn declaration_change_recomputes_the_contract() {
    use orange_model::DocCommand;
    let mut doc = document(
        json!([variable_node(1, "get", json!(node_id(10)))]),
        json!([
            declaration(10, "core/i64@1", "run"),
            declaration(11, "core/bool@1", "run")
        ]),
    );
    let value_type = |doc: &GraphDocument| {
        let report = validate(doc);
        assert!(report.is_valid());
        report.resolved[&nid(1)].ports[0]
            .ty
            .as_ref()
            .unwrap()
            .as_str()
            .to_owned()
    };
    assert_eq!(value_type(&doc), "core/i64@1");
    let unrelated = doc
        .apply([DocCommand::UpsertVariable(
            serde_json::from_value(declaration(11, "core/string@1", "run")).unwrap(),
        )])
        .unwrap();
    assert_eq!(value_type(&doc), "core/i64@1");
    let change = doc
        .apply([DocCommand::UpsertVariable(
            serde_json::from_value(declaration(10, "core/bool@1", "run")).unwrap(),
        )])
        .unwrap();
    assert_eq!(value_type(&doc), "core/bool@1");
    let redo = doc.revert(change.inverse).unwrap();
    assert_eq!(value_type(&doc), "core/i64@1");
    let undo = doc.revert(redo.inverse).unwrap();
    assert_eq!(value_type(&doc), "core/bool@1");
    let _ = doc.revert(undo.inverse).unwrap();
    let _ = doc.revert(unrelated.inverse).unwrap();
}
#[test]
fn contextual_intrinsic_identity_is_exact() {
    for (field, value) in [
        (
            "provider",
            json!({"package":"org.test","version":"1.0.0","id":"native"}),
        ),
        ("definition_version", json!(2)),
        ("config_version", json!(2)),
    ] {
        let mut node = variable_node(1, "get", json!(node_id(10)));
        node[field] = value;
        let report = validate(&document(
            json!([node]),
            json!([declaration(10, "core/i64@1", "run")]),
        ));
        assert_eq!(
            report.diagnostics[0].code,
            DiagnosticCode::ProviderUnavailable
        );
        assert!(report.resolved.is_empty());
    }
    for id in ["core/get-variable-like", "vendor/get-variable"] {
        let mut node = variable_node(1, "get", json!(node_id(10)));
        node["definition_id"] = json!(id);
        let report = validate(&document(
            json!([node]),
            json!([declaration(10, "core/i64@1", "run")]),
        ));
        assert_eq!(report.diagnostics[0].code, DiagnosticCode::SignatureMissing);
    }
    for id in ["core/update-variable", "core/set-shared"] {
        let mut node = variable_node(1, "get", json!(node_id(10)));
        node["definition_id"] = json!(id);
        assert!(
            !validate(&document(
                json!([node]),
                json!([declaration(10, "core/i64@1", "run")])
            ))
            .is_valid()
        );
    }
}
#[test]
fn declaration_types_and_traversal_order_are_authoritative() {
    let nodes = vec![
        variable_node(1, "get", json!(node_id(10))),
        variable_node(2, "get", json!(node_id(11))),
        variable_node(3, "set", json!(node_id(12))),
    ];
    let variables = vec![
        declaration(10, "core/i64@1", "run"),
        declaration(11, "missing/type@1", "run"),
        declaration(12, "core/bool@1", "workspace"),
    ];
    let report = validate(&document(json!(nodes), json!(variables)));
    assert!(report.resolved.contains_key(&nid(1)));
    assert!(!report.resolved.contains_key(&nid(2)));
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.subject.node == Some(nid(2)) && d.code == DiagnosticCode::TypeMismatch)
    );
    let shuffled = validate(&document(
        json!(nodes.into_iter().rev().collect::<Vec<_>>()),
        json!(variables.into_iter().rev().collect::<Vec<_>>()),
    ));
    assert_eq!(report, shuffled);
}
#[test]
fn registry_computes_variable_contracts_from_the_document() {
    use orange_model::DocCommand;
    use orange_model::catalog::ContractError;
    let mut doc = document(
        json!([
            variable_node(1, "get", json!(node_id(10))),
            variable_node(2, "set", json!(node_id(10))),
            variable_node(3, "set", json!(node_id(11))),
            variable_node(4, "get", json!(node_id(12)))
        ]),
        json!([
            declaration(10, "core/i64@1", "run"),
            declaration(11, "core/i64@1", "workspace")
        ]),
    );
    let registry = registry();
    let contract = |doc: &GraphDocument, n: u8| registry.contract(doc, doc.node(nid(n)).unwrap());
    let get = contract(&doc, 1).unwrap().unwrap();
    assert_eq!(
        (get.kind, get.suspends),
        (NodeKind::Volatile, Some(Suspends::Never))
    );
    assert_eq!(get.ports[0].direction, PortDirection::Output);
    assert_eq!(get.ports[0].source, Some(OutputSource::State));
    let set = contract(&doc, 2).unwrap().unwrap();
    assert_eq!(
        (set.kind, set.suspends),
        (NodeKind::Action, Some(Suspends::Never))
    );
    assert_eq!(set.ports[0].direction, PortDirection::Input);
    assert_eq!(contract(&doc, 3).unwrap_err(), ContractError::VariableScope);
    assert_eq!(
        contract(&doc, 4).unwrap_err(),
        ContractError::VariableUnknown
    );

    // Nothing is cached: the contract follows the declaration.
    let _ = doc
        .apply([DocCommand::UpsertVariable(
            serde_json::from_value(declaration(10, "core/bool@1", "run")).unwrap(),
        )])
        .unwrap();
    let get = contract(&doc, 1).unwrap().unwrap();
    assert_eq!(get.ports[0].ty.as_ref().unwrap().as_str(), "core/bool@1");

    let mut unknown = doc.node(nid(1)).unwrap().clone();
    unknown.definition.id = "acme/unregistered".parse().unwrap();
    assert!(registry.contract(&doc, &unknown).unwrap().is_none());
}

#[test]
fn update_add_sub_projects_fresh_exact_action_ports_without_mutation() {
    let cases = [
        (1, 10, "core/i64@1", "add"),
        (2, 11, "core/f64@1", "sub"),
        (3, 12, "core/duration@1", "add"),
    ];
    let doc = document(
        json!(cases.map(|(node, variable, _, operation)| update_node(
            node,
            json!(node_id(variable)),
            json!(operation)
        ))),
        json!([
            declaration(10, "core/i64@1", "local"),
            declaration(11, "core/f64@1", "run"),
            declaration(12, "core/duration@1", "local")
        ]),
    );
    let before = to_json(&doc).unwrap();
    let revision = doc.revision();
    let state_id = doc.state_id();
    let registry = registry();
    let resolved = validate_nodes(&doc, &registry);
    assert!(resolved.is_valid(), "{:?}", resolved.diagnostics);
    assert_eq!(resolved.resolved.len(), cases.len());
    assert!(
        doc.signatures().is_empty(),
        "intrinsic projections are not stored"
    );
    for (node, variable, ty, _) in cases {
        let signature = &resolved.resolved[&nid(node)];
        assert_eq!(signature.kind, NodeKind::Action);
        assert_eq!(signature.suspends, Some(Suspends::Never));
        assert_eq!(
            signature
                .ports
                .iter()
                .map(|port| port.port_id.as_str())
                .collect::<Vec<_>>(),
            ["value", "old", "new"]
        );
        assert_eq!(signature.ports[0].direction, PortDirection::Input);
        assert_eq!(signature.ports[0].ty.as_ref().unwrap().as_str(), ty);
        for output in &signature.ports[1..] {
            assert_eq!(output.direction, PortDirection::Output);
            assert_eq!(output.ty.as_ref().unwrap().as_str(), ty);
            assert_eq!(output.source, Some(OutputSource::Execution));
            assert_eq!(output.available_on.as_ref().unwrap()[0].as_str(), "then");
        }
        assert_eq!(
            doc.node(nid(node)).unwrap().config["variable"],
            node_id(variable)
        );
    }
    assert_eq!(
        validate_inputs(&doc, &resolved.resolved, &registry)
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<Vec<_>>(),
        [DiagnosticCode::InputRequired; 3]
    );
    let mut supplied = update_node(1, json!(node_id(10)), json!("add"));
    supplied["inputs"] = json!({"value":{"type":"core/i64@1","value":"7"}});
    let supplied = document(
        json!([supplied]),
        json!([declaration(10, "core/i64@1", "run")]),
    );
    let supplied_resolved = validate_nodes(&supplied, &registry);
    assert!(validate_inputs(&supplied, &supplied_resolved.resolved, &registry).is_empty());
    assert_eq!(to_json(&doc).unwrap(), before);
    assert_eq!(doc.revision(), revision);
    assert_eq!(doc.state_id(), state_id);
}

#[test]
fn update_errors_follow_reference_scope_operation_then_type_precedence() {
    let cases = [
        ("missing", "run", None, DiagnosticCode::UpdateOperation),
        (
            "42",
            "run",
            Some(json!(42)),
            DiagnosticCode::UpdateOperation,
        ),
        (
            "invented",
            "run",
            Some(json!("invented")),
            DiagnosticCode::UpdateOperation,
        ),
        (
            "set",
            "run",
            Some(json!("set")),
            DiagnosticCode::UpdateOperation,
        ),
        (
            "min",
            "run",
            Some(json!("min")),
            DiagnosticCode::UpdateUnsupported,
        ),
        (
            "add",
            "run",
            Some(json!("add")),
            DiagnosticCode::ConstraintUnsatisfied,
        ),
        (
            "add",
            "run",
            Some(json!("add")),
            DiagnosticCode::TypeMismatch,
        ),
        (
            "add",
            "workspace",
            Some(json!("min")),
            DiagnosticCode::VariableScope,
        ),
    ];
    for (name, scope, operation, expected) in cases {
        let ty = match expected {
            DiagnosticCode::ConstraintUnsatisfied => "core/bool@1",
            DiagnosticCode::TypeMismatch => "missing/type@1",
            _ => "core/i64@1",
        };
        let node = if let Some(operation) = operation {
            update_node(1, json!(node_id(10)), operation)
        } else {
            let mut node = update_node(1, json!(node_id(10)), Value::Null);
            node["config"].as_object_mut().unwrap().remove("operation");
            node
        };
        let doc = document(json!([node]), json!([declaration(10, ty, scope)]));
        let report = validate(&doc);
        assert_eq!(
            report.diagnostics.len(),
            1,
            "{name}: {:?}",
            report.diagnostics
        );
        assert_eq!(report.diagnostics[0].code, expected, "{name}");
        assert!(report.resolved.is_empty());
        assert!(report.incomplete_nodes.contains(&nid(1)));
    }
}
