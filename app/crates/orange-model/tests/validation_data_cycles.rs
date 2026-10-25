use std::{collections::BTreeMap, sync::Arc};

use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{
    GraphDocument, NodeKind, OutputSource, PortDescriptor, PortRef, SignatureSnapshot,
};
use orange_model::from_json;
use orange_model::id::{EdgeId, NodeId, PortId, PortKey};
use orange_model::validation::{
    ConnectionValidation, DataCycleValidation, Diagnostic, DiagnosticCode, HostOrigin, Subject,
    validate_data_cycles,
};
use serde_json::{Value, json};

const A: &str = "10000000-0000-4000-8000-000000000001";
const B: &str = "10000000-0000-4000-8000-000000000002";
const FN: &str = "20000000-0000-4000-8000-000000000001";
const SIGNATURE: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn origin() -> HostOrigin {
    HostOrigin::new("0.1.0", format!("sha256:{}", "b".repeat(64))).unwrap()
}

fn registry() -> Arc<RegistrySnapshot> {
    RegistrySnapshot::core(origin())
}

fn node(id: &str) -> Value {
    json!({
        "id":id,
        "definition_id":"example/node",
        "definition_version":1,
        "config_version":1,
        "config":{},
        "signature":SIGNATURE
    })
}

fn edge(id: u8, from_node: &str, from_port: &str, to_node: &str, to_port: &str) -> Value {
    json!({
        "id":format!("30000000-0000-4000-8000-{id:012}"),
        "from":{"node":from_node,"port":from_port},
        "to":{"node":to_node,"port":to_port}
    })
}

fn document(nodes: Vec<Value>, edges: Vec<Value>, functions: Vec<Value>) -> GraphDocument {
    from_json(
        &json!({
            "format":"orange.graph",
            "format_version":1,
            "graph_id":"40000000-0000-4000-8000-000000000001",
            "revision":0,
            "nodes":nodes,
            "edges":edges,
            "functions":functions,
            "signatures":{SIGNATURE:{"kind":"pure","ports":[]}}
        })
        .to_string(),
    )
    .unwrap()
}

fn signature(ports: Vec<PortDescriptor>) -> SignatureSnapshot {
    SignatureSnapshot {
        kind: NodeKind::Pure,
        suspends: None,
        ports,
        extensions: Default::default(),
    }
}

fn port(id: &str, instance: Option<u16>) -> PortKey {
    PortKey::new(id.parse::<PortId>().unwrap(), instance)
}

fn resolved(entries: Vec<(&str, SignatureSnapshot)>) -> BTreeMap<NodeId, SignatureSnapshot> {
    entries
        .into_iter()
        .map(|(id, signature)| (id.parse().unwrap(), signature))
        .collect()
}

fn check(
    doc: &GraphDocument,
    resolved: &BTreeMap<NodeId, SignatureSnapshot>,
    connections: &ConnectionValidation,
) -> DataCycleValidation {
    validate_data_cycles(doc, resolved, connections, &registry())
}

fn input_output(source: OutputSource) -> SignatureSnapshot {
    signature(vec![
        PortDescriptor::data_in("input", "core/i64@1".parse().unwrap()),
        PortDescriptor::data_out("output", "core/i64@1".parse().unwrap(), source),
    ])
}

#[test]
fn wide_computation_returns_a_closed_witness_using_only_real_ports() {
    let ports: Vec<_> = (0..512)
        .flat_map(|i| {
            [
                PortDescriptor::data_in(
                    format!("input{i}").as_str(),
                    "core/i64@1".parse().unwrap(),
                ),
                PortDescriptor::data_out(
                    format!("output{i}").as_str(),
                    "core/i64@1".parse().unwrap(),
                    OutputSource::Computed,
                ),
            ]
        })
        .collect();
    let doc = document(
        vec![node(A)],
        vec![edge(1, A, "output511", A, "input511")],
        vec![],
    );
    let signatures = resolved(vec![(A, signature(ports))]);
    let result = check(&doc, &signatures, &ConnectionValidation::default());
    assert_eq!(result.cycle_paths.len(), 1);
    assert_eq!(result.cycle_paths[0].len(), 3);
    let path = &result.cycle_paths[0];
    assert_eq!(path.first(), path.last());
    assert!(path.iter().all(|p| {
        p.node == A.parse().unwrap()
            && ["input511", "output511"]
                .iter()
                .any(|id| p.port == port(id, None))
    }));
}

#[test]
fn computed_self_dependency_reports_closed_port_path() {
    let doc = document(
        vec![node(A)],
        vec![edge(1, A, "output", A, "input")],
        vec![],
    );
    let signatures = resolved(vec![(A, input_output(OutputSource::Computed))]);
    let result = check(&doc, &signatures, &ConnectionValidation::default());
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].code, DiagnosticCode::CycleData);
    assert_eq!(result.cycle_paths.len(), 1);
    assert_eq!(result.cycle_paths[0].len(), 3);
    assert_eq!(
        result.cycle_paths[0][0],
        PortRef {
            node: A.parse().unwrap(),
            port: port("input", None)
        }
    );
    assert_eq!(
        result.cycle_paths[0][1],
        PortRef {
            node: A.parse().unwrap(),
            port: port("output", None)
        }
    );
    assert_eq!(
        result.cycle_paths[0][0],
        *result.cycle_paths[0].last().unwrap()
    );
    assert!(!result.is_valid());
}

#[test]
fn state_execution_payload_and_iteration_outputs_break_data_dependencies() {
    for source in [
        OutputSource::State,
        OutputSource::Execution,
        OutputSource::Payload,
        OutputSource::Iteration,
    ] {
        let doc = document(
            vec![node(A)],
            vec![edge(1, A, "output", A, "input")],
            vec![],
        );
        let signatures = resolved(vec![(A, input_output(source))]);
        let result = check(&doc, &signatures, &ConnectionValidation::default());
        assert!(result.cycle_paths.is_empty(), "{source:?}");
        assert!(
            result.diagnostics.is_empty(),
            "{source:?}: {:?}",
            result.diagnostics
        );
    }
}

#[test]
fn implicit_exec_and_event_edges_are_resolved_but_excluded_from_data_graph() {
    let mut source = node(A);
    source["definition_id"] = json!("core/set-variable");
    let mut target = node(B);
    target["definition_id"] = json!("core/set-variable");
    let doc = document(
        vec![source.clone(), target],
        vec![edge(1, A, "then", B, "in")],
        vec![],
    );
    let set_signature = SignatureSnapshot {
        kind: NodeKind::Action,
        ..signature(vec![PortDescriptor::data_in(
            "value",
            "core/i64@1".parse().unwrap(),
        )])
    };
    let exec_result = check(
        &doc,
        &resolved(vec![(A, set_signature.clone()), (B, set_signature)]),
        &ConnectionValidation::default(),
    );
    assert!(exec_result.cycle_paths.is_empty());
    assert!(exec_result.incomplete_nodes.is_empty());

    let mut event_source = node(A);
    event_source["definition_id"] = json!("example/event-source");
    let mut action_target = node(B);
    action_target["definition_id"] = json!("core/set-variable");
    let event_doc = document(
        vec![event_source, action_target],
        vec![edge(2, A, "event", B, "in")],
        vec![],
    );
    let event_signature = signature(vec![PortDescriptor {
        direction: orange_model::document::PortDirection::Output,
        kind: orange_model::document::PortKind::Event,
        ty: Some("core/i64@1".parse().unwrap()),
        source: Some(OutputSource::Payload),
        ..PortDescriptor::data_in("event", "core/i64@1".parse().unwrap())
    }]);
    let target_signature = SignatureSnapshot {
        kind: NodeKind::Action,
        ..signature(vec![PortDescriptor::data_in(
            "value",
            "core/i64@1".parse().unwrap(),
        )])
    };
    let event_result = check(
        &event_doc,
        &resolved(vec![(A, event_signature), (B, target_signature)]),
        &ConnectionValidation::default(),
    );
    assert!(event_result.cycle_paths.is_empty());
    assert!(event_result.incomplete_nodes.is_empty());
}

#[test]
fn computed_output_does_not_inherit_a_state_sibling_outputs_inputs() {
    let doc = document(
        vec![node(A), node(B)],
        vec![
            edge(1, A, "state", B, "input"),
            edge(2, B, "output", A, "input"),
        ],
        vec![],
    );
    let a = signature(vec![
        PortDescriptor::data_in("input", "core/i64@1".parse().unwrap()),
        PortDescriptor::data_out("state", "core/i64@1".parse().unwrap(), OutputSource::State),
        PortDescriptor::data_out(
            "computed",
            "core/i64@1".parse().unwrap(),
            OutputSource::Computed,
        ),
    ]);
    let b = input_output(OutputSource::Computed);
    let signatures = resolved(vec![(A, a), (B, b)]);
    let result = check(&doc, &signatures, &ConnectionValidation::default());
    assert!(result.cycle_paths.is_empty(), "{:?}", result.cycle_paths);
    let mut fixture = serde_json::to_value(&doc).unwrap();
    fixture["edges"][0]["from"]["port"] = json!("computed");
    let cyclic: GraphDocument = serde_json::from_value(fixture).unwrap();
    assert_eq!(
        check(&cyclic, &signatures, &ConnectionValidation::default())
            .cycle_paths
            .len(),
        1
    );
}

#[test]
fn computed_two_node_cycle_is_deterministic_and_independent_per_function_body() {
    let nodes = vec![node(A), node(B)];
    let edges = vec![
        edge(1, A, "output", B, "input"),
        edge(2, B, "output", A, "input"),
    ];
    let function = json!({
        "id":FN,"name":"f","kind":"pure","inputs":[],"outputs":[],
        "nodes":nodes,"edges":edges
    });
    let doc = document(vec![], vec![], vec![function]);
    let signatures = resolved(vec![
        (A, input_output(OutputSource::Computed)),
        (B, input_output(OutputSource::Computed)),
    ]);
    let first = check(&doc, &signatures, &ConnectionValidation::default());
    let second = check(&doc, &signatures, &ConnectionValidation::default());
    assert_eq!(first, second);
    assert_eq!(first.cycle_paths.len(), 1);
    assert_eq!(first.diagnostics[0].code, DiagnosticCode::CycleData);
    assert_eq!(first.cycle_paths[0].first(), first.cycle_paths[0].last());
}

#[test]
fn disjoint_cyclic_components_each_produce_one_closed_witness() {
    let doc = document(
        vec![node(A), node(B)],
        vec![
            edge(1, A, "output", A, "input"),
            edge(2, B, "output", B, "input"),
        ],
        vec![],
    );
    let signatures = resolved(vec![
        (A, input_output(OutputSource::Computed)),
        (B, input_output(OutputSource::Computed)),
    ]);
    let result = check(&doc, &signatures, &ConnectionValidation::default());
    assert_eq!(result.cycle_paths.len(), 2);
    assert_eq!(result.diagnostics.len(), 2);
    for path in &result.cycle_paths {
        assert_eq!(path.first(), path.last());
    }
    assert_ne!(result.cycle_paths[0][0].node, result.cycle_paths[1][0].node);
    assert_eq!(
        result,
        check(&doc, &signatures, &ConnectionValidation::default())
    );
}

#[test]
fn sparse_variadic_instances_are_vertices_by_ordered_concrete_key() {
    let mut a = node(A);
    a["variadic"] = json!({
        "items":{"order":[8,2],"next":9},
        "result":{"order":[17],"next":18}
    });
    let mut b = node(B);
    b["variadic"] = json!({
        "items":{"order":[3],"next":4},
        "result":{"order":[29,11],"next":30}
    });
    let doc = document(
        vec![a, b],
        vec![
            edge(1, A, "result#17", B, "items#3"),
            edge(2, B, "result#29", A, "items#8"),
        ],
        vec![],
    );
    let variadic_signature = signature(vec![
        PortDescriptor::data_in("items", "core/i64@1".parse().unwrap()).variadic(0, None),
        PortDescriptor::data_out(
            "result",
            "core/i64@1".parse().unwrap(),
            OutputSource::Computed,
        )
        .variadic(0, None),
    ]);
    let signatures = resolved(vec![
        (A, variadic_signature.clone()),
        (B, variadic_signature),
    ]);
    let result = check(&doc, &signatures, &ConnectionValidation::default());
    assert_eq!(result.cycle_paths.len(), 1);
    assert!(
        result.cycle_paths[0]
            .iter()
            .any(|p| p.port.instance() == Some(17))
    );
    assert!(
        result.cycle_paths[0]
            .iter()
            .any(|p| p.port.instance() == Some(3))
    );
    assert!(
        result.cycle_paths[0]
            .iter()
            .any(|p| p.port.instance() == Some(29))
    );
    assert!(
        result.cycle_paths[0]
            .iter()
            .any(|p| p.port.instance() == Some(8))
    );
    let mut fixture = serde_json::to_value(&doc).unwrap();
    fixture["edges"][1]["to"]["port"] = json!("items#7");
    let orphan: GraphDocument = serde_json::from_value(fixture).unwrap();
    let connections =
        orange_model::validation::validate_connections(&orphan, &signatures, &registry());
    assert!(
        connections
            .diagnostics
            .iter()
            .any(|item| item.code == DiagnosticCode::PortUnknown)
    );
    let result = check(&orphan, &signatures, &connections);
    assert!(
        result.cycle_paths.is_empty(),
        "a nonexistent instance cannot close a cycle"
    );
}

#[test]
fn unresolved_and_invalid_edges_are_not_guessed_into_cycles() {
    let doc = document(
        vec![node(A), node(B)],
        vec![
            edge(1, A, "output", B, "input"),
            edge(2, B, "output", A, "input"),
        ],
        vec![],
    );
    let only_b = resolved(vec![(B, input_output(OutputSource::Computed))]);
    let unresolved = check(&doc, &only_b, &ConnectionValidation::default());
    assert!(unresolved.cycle_paths.is_empty());
    assert!(unresolved.incomplete_nodes.contains(&A.parse().unwrap()));
    assert!(
        unresolved
            .diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::DependencyUnavailable)
    );

    let edge_id: EdgeId = "30000000-0000-4000-8000-000000000001".parse().unwrap();
    let mut connections = ConnectionValidation::default();
    connections.diagnostics.push(Diagnostic::new(
        DiagnosticCode::TypeMismatch,
        Subject::new(
            Some(B.parse().unwrap()),
            Some(port("input", None)),
            Some(edge_id),
        ),
        &origin(),
    ));
    let both = resolved(vec![
        (A, input_output(OutputSource::Computed)),
        (B, input_output(OutputSource::Computed)),
    ]);
    let invalid = check(&doc, &both, &connections);
    assert!(invalid.cycle_paths.is_empty());
}

#[test]
fn cross_body_edges_are_not_used_as_data_dependencies() {
    let mut function_node = node(B);
    function_node["id"] = json!(B);
    let function = json!({
        "id":FN,"name":"f","kind":"pure","inputs":[],"outputs":[],
        "nodes":[function_node],"edges":[]
    });
    let doc = document(
        vec![node(A)],
        vec![edge(1, A, "output", B, "input")],
        vec![function],
    );
    let signatures = resolved(vec![
        (A, input_output(OutputSource::Computed)),
        (B, input_output(OutputSource::Computed)),
    ]);
    let result = check(&doc, &signatures, &ConnectionValidation::default());
    assert!(result.cycle_paths.is_empty());
}

#[test]
fn connection_incompleteness_is_carried_forward() {
    let doc = document(vec![node(A)], vec![], vec![]);
    let mut connections = ConnectionValidation::default();
    connections.incomplete_nodes.insert(A.parse().unwrap());
    let result = check(
        &doc,
        &resolved(vec![(A, input_output(OutputSource::Computed))]),
        &connections,
    );
    assert!(result.incomplete_nodes.contains(&A.parse().unwrap()));
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::DependencyUnavailable)
    );
}

#[test]
fn absent_output_source_makes_dependency_metadata_incomplete() {
    let doc = document(vec![node(A)], vec![], vec![]);
    let malformed = signature(vec![
        PortDescriptor::data_in("input", "core/i64@1".parse().unwrap()),
        PortDescriptor {
            source: None,
            ..PortDescriptor::data_out(
                "output",
                "core/i64@1".parse().unwrap(),
                OutputSource::Computed,
            )
        },
    ]);
    let result = check(
        &doc,
        &resolved(vec![(A, malformed)]),
        &ConnectionValidation::default(),
    );
    assert!(result.incomplete_nodes.contains(&A.parse().unwrap()));
}
