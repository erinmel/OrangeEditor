//! Zone pairing, purity, closure and nesting (11 §8.4, `E-ZONE-STRUCTURE`).

use std::collections::BTreeMap;

use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{
    GraphDocument, NodeKind, OutputSource, PortDescriptor, SignatureSnapshot,
};
use orange_model::from_json;
use orange_model::id::NodeId;
use orange_model::validation::{
    ConnectionValidation, DiagnosticCode, HostOrigin, RegionValidation, ZoneKind,
    validate_data_cycles, validate_regions,
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

fn fixture(
    nodes: Vec<N>,
    edges: Vec<Value>,
) -> (GraphDocument, BTreeMap<NodeId, SignatureSnapshot>) {
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
    (doc, resolved)
}

fn registry() -> std::sync::Arc<RegistrySnapshot> {
    let origin = HostOrigin::new("0.1.0", format!("sha256:{}", "b".repeat(64))).unwrap();
    RegistrySnapshot::core(origin)
}

fn run(nodes: Vec<N>, edges: Vec<Value>) -> RegionValidation {
    let (doc, resolved) = fixture(nodes, edges);
    validate_regions(
        &doc,
        &resolved,
        &ConnectionValidation::default(),
        &registry(),
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

#[test]
fn checked_zone_ownership_distinguishes_body_captures_and_result() {
    let (mut nodes, mut edges) = closed_zone();
    nodes[1]
        .signature
        .ports
        .push(PortDescriptor::data_in("capture", int()));
    nodes.push(step(4, NodeKind::Pure));
    edges.push(wire(3, 4, "y", 2, "capture"));
    let result = run(nodes, edges);
    assert!(result.is_valid());
    let zones = &result.index.zones;
    let pair = zones.get(nid(3)).unwrap();
    assert_eq!(pair.kind, ZoneKind::Map);
    assert_eq!(pair.begin, nid(1));
    assert_eq!(pair.function, None);
    assert_eq!(pair.parent, None);
    assert_eq!(zones.iter().count(), 1);
    assert_eq!(zones.owner(nid(1)), Some(pair));
    assert_eq!(zones.owner(nid(2)), Some(pair));
    assert_eq!(zones.owner(nid(3)), None);
    assert_eq!(zones.owner(nid(4)), None);
}

#[test]
fn nested_zone_begin_owns_parameters_but_its_end_belongs_to_parent() {
    let mut inner = begin(4, "inner");
    inner
        .signature
        .ports
        .push(PortDescriptor::data_in("items", int()));
    let result = run(
        vec![
            begin(1, "outer"),
            end(3, "outer"),
            inner,
            step(2, NodeKind::Pure),
            end(5, "inner"),
        ],
        vec![
            wire(1, 1, "element", 4, "items"),
            wire(2, 4, "element", 2, "x"),
            wire(3, 2, "y", 5, "item"),
            wire(4, 5, "result", 3, "item"),
        ],
    );
    assert!(result.is_valid(), "{:?}", result.diagnostics);
    let zones = &result.index.zones;
    assert_eq!(zones.get(nid(5)).unwrap().parent, Some(nid(3)));
    assert_eq!(zones.owner(nid(4)).unwrap().end, nid(5));
    assert_eq!(zones.owner(nid(2)).unwrap().end, nid(5));
    assert_eq!(zones.owner(nid(5)).unwrap().end, nid(3));
    assert_eq!(zones.owner(nid(3)), None);
}

#[test]
fn invalid_zone_structure_publishes_no_guessed_ownership() {
    let (mut nodes, mut edges) = closed_zone();
    nodes.push(step(4, NodeKind::Pure));
    edges.push(wire(3, 2, "y", 4, "x"));
    let result = run(nodes, edges);
    assert!(!result.is_valid());
    assert_eq!(result.index.zones.iter().count(), 0);
    assert_eq!(result.index.zones.owner(nid(2)), None);
}

#[test]
fn all_four_families_pair_independently_with_the_same_config_id() {
    let mut nodes = Vec::new();
    for (i, family) in ["repeat", "map", "fold", "filter"].into_iter().enumerate() {
        let mut a = begin((i * 2 + 1) as u8, "same");
        let mut b = end((i * 2 + 2) as u8, "same");
        a.json["definition_id"] = json!(format!("core/{family}-begin"));
        b.json["definition_id"] = json!(format!("core/{family}-end"));
        nodes.extend([a, b]);
    }
    let result = run(nodes, vec![]);
    assert!(result.is_valid());
    assert_eq!(
        result
            .index
            .zones
            .iter()
            .map(|p| p.kind)
            .collect::<Vec<_>>(),
        [
            ZoneKind::Repeat,
            ZoneKind::Map,
            ZoneKind::Fold,
            ZoneKind::Filter
        ]
    );
}

fn prefix_begin(n: u8, zone: &str) -> N {
    let mut node = begin(n, zone);
    node.signature
        .ports
        .push(PortDescriptor::data_in("items", int()));
    node
}

#[test]
fn zone_result_cannot_supply_its_own_strict_prefix() {
    let (doc, resolved) = fixture(
        vec![prefix_begin(1, "z"), end(2, "z")],
        vec![
            wire(1, 1, "element", 2, "item"),
            wire(2, 2, "result", 1, "items"),
        ],
    );
    let result = validate_data_cycles(
        &doc,
        &resolved,
        &ConnectionValidation::default(),
        &registry(),
    );
    assert_eq!(result.cycle_paths.len(), 1);
    let witness = &result.cycle_paths[0];
    assert_eq!(witness.first(), witness.last());
    assert!(
        witness
            .iter()
            .any(|p| p.node == nid(1) && p.port.to_string() == "items")
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.code == DiagnosticCode::CycleData)
    );
}

#[test]
fn mutually_dependent_zone_prefixes_are_cycles_even_without_body_edges() {
    let (doc, resolved) = fixture(
        vec![
            prefix_begin(1, "a"),
            end(2, "a"),
            prefix_begin(3, "b"),
            end(4, "b"),
        ],
        vec![
            wire(1, 2, "result", 3, "items"),
            wire(2, 4, "result", 1, "items"),
        ],
    );
    let result = validate_data_cycles(
        &doc,
        &resolved,
        &ConnectionValidation::default(),
        &registry(),
    );
    assert_eq!(result.cycle_paths.len(), 1);
}

#[test]
fn zone_prefix_feedback_through_state_remains_acyclic() {
    let mut state = step(3, NodeKind::Action);
    state.signature.ports[1].source = Some(OutputSource::State);
    let (doc, resolved) = fixture(
        vec![prefix_begin(1, "z"), end(2, "z"), state],
        vec![
            wire(1, 1, "element", 2, "item"),
            wire(2, 2, "result", 3, "x"),
            wire(3, 3, "y", 1, "items"),
        ],
    );
    let result = validate_data_cycles(
        &doc,
        &resolved,
        &ConnectionValidation::default(),
        &registry(),
    );
    assert!(result.is_valid(), "{:?}", result.diagnostics);
}

#[test]
fn zone_pair_ids_and_ownership_are_local_to_each_function_body() {
    let (root, mut resolved) = fixture(vec![begin(1, "same"), end(2, "same")], vec![]);
    let (inner, inner_resolved) = fixture(vec![begin(3, "same"), end(4, "same")], vec![]);
    resolved.extend(inner_resolved);
    let mut saved = serde_json::from_str::<Value>(&orange_model::to_json(&root).unwrap()).unwrap();
    let inner = serde_json::from_str::<Value>(&orange_model::to_json(&inner).unwrap()).unwrap();
    let function = "20000000-0000-4000-8000-000000000001";
    saved["functions"] = json!([{
        "id": function, "name": "nested", "kind": "pure", "inputs": [], "outputs": [],
        "nodes": inner["nodes"], "edges": []
    }]);
    let doc: GraphDocument = from_json(&saved.to_string()).unwrap();
    let result = validate_regions(
        &doc,
        &resolved,
        &ConnectionValidation::default(),
        &registry(),
    );
    assert!(result.is_valid(), "{:?}", result.diagnostics);
    assert_eq!(result.index.zones.iter().count(), 2);
    assert_eq!(result.index.zones.get(nid(2)).unwrap().function, None);
    assert_eq!(
        result.index.zones.get(nid(4)).unwrap().function,
        Some(function.parse().unwrap())
    );
    assert_eq!(result.index.zones.get(nid(4)).unwrap().parent, None);
}
