//! Zone pairing, purity, closure and nesting (11 §8.4, `E-ZONE-STRUCTURE`).

use std::collections::BTreeMap;

use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{
    GraphDocument, NodeKind, OutputSource, PortDescriptor, SignatureSnapshot,
};
use orange_model::from_json;
use orange_model::id::NodeId;
use orange_model::validation::{
    ConnectionValidation, DiagnosticCode, HostOrigin, RegionValidation, validate_regions,
};
use serde_json::{Value, json};

const SIGNATURE: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn id(n: u8) -> String {
    format!("10000000-0000-4000-8000-{n:012}")
}

fn nid(n: u8) -> NodeId {
    id(n).parse().unwrap()
}

struct N {
    json: Value,
    id: NodeId,
    signature: SignatureSnapshot,
}

fn int() -> orange_model::id::TypeExpr {
    "core/i64@1".parse().unwrap()
}

fn node(n: u8, definition: &str, kind: NodeKind, config: Value, ports: Vec<PortDescriptor>) -> N {
    let mut json = json!({
        "id": id(n), "definition_id": definition, "definition_version": 1,
        "config_version": 1, "config": config
    });
    if !definition.starts_with("core/") {
        json["signature"] = json!(SIGNATURE);
    }
    N {
        json,
        id: nid(n),
        signature: SignatureSnapshot {
            kind,
            suspends: None,
            ports,
            extensions: Default::default(),
        },
    }
}

fn begin(n: u8, zone: &str) -> N {
    node(
        n,
        "core/map-begin",
        NodeKind::Pure,
        json!({"zone": zone}),
        vec![PortDescriptor::data_out(
            "element",
            int(),
            OutputSource::Iteration,
        )],
    )
}

fn end(n: u8, zone: &str) -> N {
    node(
        n,
        "core/map-end",
        NodeKind::Pure,
        json!({"zone": zone}),
        vec![
            PortDescriptor::data_in("item", int()),
            PortDescriptor::data_out("result", int(), OutputSource::Computed),
        ],
    )
}

fn step(n: u8, kind: NodeKind) -> N {
    node(
        n,
        "example/step",
        kind,
        json!({}),
        vec![
            PortDescriptor::data_in("x", int()),
            PortDescriptor::data_out("y", int(), OutputSource::Computed),
        ],
    )
}

fn wire(n: u8, from: u8, from_port: &str, to: u8, to_port: &str) -> Value {
    json!({
        "id": format!("30000000-0000-4000-8000-{n:012}"),
        "from": {"node": id(from), "port": from_port},
        "to": {"node": id(to), "port": to_port}
    })
}

fn run(nodes: Vec<N>, edges: Vec<Value>) -> RegionValidation {
    let doc: GraphDocument = from_json(
        &json!({
            "format":"orange.graph","format_version":1,
            "graph_id":"40000000-0000-4000-8000-000000000001","revision":0,
            "nodes": nodes.iter().map(|n| n.json.clone()).collect::<Vec<_>>(),
            "edges": edges,
            "signatures":{SIGNATURE:{"kind":"pure","ports":[]}}
        })
        .to_string(),
    )
    .unwrap();
    let resolved: BTreeMap<_, _> = nodes.into_iter().map(|n| (n.id, n.signature)).collect();
    let origin = HostOrigin::new("0.1.0", format!("sha256:{}", "b".repeat(64))).unwrap();
    validate_regions(
        &doc,
        &resolved,
        &ConnectionValidation::default(),
        &RegistrySnapshot::core(origin),
    )
}

fn zone_faults(result: &RegionValidation) -> Vec<NodeId> {
    result
        .diagnostics
        .iter()
        .filter(|d| d.code == DiagnosticCode::ZoneStructure)
        .map(|d| d.subject.node.unwrap())
        .collect()
}

/// begin(1) -> step(2) -> end(3), plus a capture step(4) feeding step(2).
fn closed_zone() -> (Vec<N>, Vec<Value>) {
    (
        vec![begin(1, "z"), step(2, NodeKind::Pure), end(3, "z")],
        vec![wire(1, 1, "element", 2, "x"), wire(2, 2, "y", 3, "item")],
    )
}

#[test]
fn a_closed_pure_zone_is_valid_and_captures_are_allowed() {
    let (nodes, edges) = closed_zone();
    assert!(zone_faults(&run(nodes, edges)).is_empty());
    let (mut nodes, _) = closed_zone();
    nodes.push(step(4, NodeKind::Pure));
    nodes.push(node(
        5,
        "example/two",
        NodeKind::Pure,
        json!({}),
        vec![
            PortDescriptor::data_in("a", int()),
            PortDescriptor::data_in("b", int()),
            PortDescriptor::data_out("y", int(), OutputSource::Computed),
        ],
    ));
    // Rewire: begin and the capture both feed node 5, which feeds the end.
    let edges = vec![
        wire(1, 1, "element", 5, "a"),
        wire(2, 4, "y", 5, "b"),
        wire(3, 5, "y", 3, "item"),
    ];
    nodes.retain(|n| n.id != nid(2));
    assert!(zone_faults(&run(nodes, edges)).is_empty());
}

#[test]
fn a_zone_without_an_id_or_partner_is_rejected() {
    let mut anonymous = begin(1, "z");
    anonymous.json["config"] = json!({});
    assert_eq!(zone_faults(&run(vec![anonymous], vec![])), [nid(1)]);
    assert_eq!(zone_faults(&run(vec![begin(1, "z")], vec![])), [nid(1)]);
    let twice = run(vec![begin(1, "z"), begin(2, "z"), end(3, "z")], vec![]);
    assert_eq!(zone_faults(&twice), [nid(1), nid(2), nid(3)]);
}

#[test]
fn an_impure_node_inside_a_zone_is_rejected() {
    let (_, edges) = closed_zone();
    let nodes = vec![begin(1, "z"), step(2, NodeKind::Action), end(3, "z")];
    assert_eq!(zone_faults(&run(nodes, edges)), [nid(2)]);
}

#[test]
fn a_value_leaving_a_zone_other_than_through_the_end_is_rejected() {
    let (mut nodes, mut edges) = closed_zone();
    nodes.push(step(4, NodeKind::Pure));
    edges.push(wire(3, 2, "y", 4, "x"));
    let result = run(nodes, edges);
    assert_eq!(zone_faults(&result), [nid(2)]);
    assert!(result.diagnostics[0].subject.edge.is_some());
}

fn switch(cases: Value) -> RegionValidation {
    let mut switch = node(
        1,
        "core/switch",
        NodeKind::Action,
        json!({"cases": cases}),
        vec![
            PortDescriptor::exec_in("in"),
            PortDescriptor::data_in("key", int()),
            PortDescriptor::exec_out("case").variadic(1, None),
            PortDescriptor::exec_out("default"),
        ],
    );
    switch.json["variadic"] = json!({"case": {"order": [0, 1], "next": 2}});
    run(vec![switch], vec![])
}

#[test]
fn switch_cases_must_be_present_distinct_and_of_the_key_kind() {
    let switch_faults = |cases| {
        switch(cases)
            .diagnostics
            .iter()
            .filter(|d| d.code == DiagnosticCode::SwitchCase)
            .count()
    };
    assert_eq!(switch_faults(json!({"0": 1, "1": 2})), 0);
    assert_eq!(switch_faults(json!({"0": 1, "1": 1})), 1);
    assert_eq!(switch_faults(json!({"0": 1, "1": "two"})), 1);
    assert_eq!(switch_faults(json!({"0": 1})), 1);
    assert_eq!(switch_faults(json!({"0": 1, "1": 2, "7": 3})), 1);
    assert_eq!(switch_faults(json!({})), 1);
}

#[test]
fn zones_must_nest_properly() {
    // Zone b's end sits inside zone a's body while its begin is outside it.
    let nodes = vec![
        begin(1, "a"),
        step(2, NodeKind::Pure),
        end(3, "a"),
        begin(4, "b"),
        end(5, "b"),
    ];
    let edges = vec![
        wire(1, 1, "element", 2, "x"),
        wire(2, 2, "y", 5, "item"),
        wire(3, 5, "result", 3, "item"),
    ];
    let result = run(nodes, edges);
    assert_eq!(zone_faults(&result), [nid(4)]);
}
