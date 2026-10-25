use std::{collections::BTreeMap, sync::Arc};

use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{GraphDocument, NodeKind, PortDescriptor, SignatureSnapshot};
use orange_model::from_json;
use orange_model::id::NodeId;
use orange_model::validation::{
    ConnectionValidation, DiagnosticCode, HostOrigin, RegionValidation, Severity, validate_regions,
};
use serde_json::{Value, json};

const SIGNATURE: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn id(n: u8) -> String {
    format!("10000000-0000-4000-8000-{n:012}")
}

fn nid(n: u8) -> NodeId {
    id(n).parse().unwrap()
}

fn registry() -> Arc<RegistrySnapshot> {
    RegistrySnapshot::core(HostOrigin::new("0.1.0", format!("sha256:{}", "b".repeat(64))).unwrap())
}

/// A node plus its resolved signature. Core definitions carry no signature key.
struct N {
    json: Value,
    id: NodeId,
    signature: SignatureSnapshot,
}

fn node(n: u8, definition: &str, kind: NodeKind, ports: Vec<PortDescriptor>) -> N {
    let mut json = json!({
        "id": id(n), "definition_id": definition, "definition_version": 1,
        "config_version": 1, "config": {}
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

fn entry(n: u8) -> N {
    node(
        n,
        "example/entry",
        NodeKind::Entry,
        vec![PortDescriptor::exec_out("then")],
    )
}

fn action(n: u8) -> N {
    node(
        n,
        "example/action",
        NodeKind::Action,
        vec![
            PortDescriptor::exec_in("in"),
            PortDescriptor::exec_out("then"),
        ],
    )
}

fn looped(n: u8, definition: &str) -> N {
    node(
        n,
        definition,
        NodeKind::Iterator,
        vec![
            PortDescriptor::exec_in("in"),
            PortDescriptor::exec_out("body"),
            PortDescriptor::exec_out("completed"),
        ],
    )
}

fn terminator(n: u8, definition: &str) -> N {
    node(
        n,
        definition,
        NodeKind::Action,
        vec![PortDescriptor::exec_in("in")],
    )
}

fn sequence(n: u8) -> N {
    let mut node = node(
        n,
        "core/sequence",
        NodeKind::Action,
        vec![
            PortDescriptor::exec_in("in"),
            PortDescriptor::exec_out("then").variadic(2, None),
        ],
    );
    node.json["variadic"] = json!({"then":{"order":[0,1],"next":2}});
    node
}

fn wire(n: u8, from: u8, from_port: &str, to: u8) -> Value {
    json!({
        "id": format!("30000000-0000-4000-8000-{n:012}"),
        "from": {"node": id(from), "port": from_port},
        "to": {"node": id(to), "port": "in"}
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
    validate_regions(
        &doc,
        &resolved,
        &ConnectionValidation::default(),
        &registry(),
    )
}

fn codes(result: &RegionValidation) -> Vec<DiagnosticCode> {
    result.diagnostics.iter().map(|d| d.code).collect()
}

#[test]
fn break_in_nested_sequence_inside_a_loop_is_valid_and_indexed() {
    let result = run(
        vec![
            entry(1),
            looped(2, "core/count"),
            sequence(3),
            terminator(4, "core/break"),
        ],
        vec![
            wire(1, 1, "then", 2),
            wire(2, 2, "body", 3),
            wire(3, 3, "then#0", 4),
        ],
    );
    assert!(result.is_valid(), "{:?}", result.diagnostics);
    let region = &result.index.owners[&nid(4)];
    assert_eq!(result.index.regions[region].loop_depth, 1);
    assert_eq!(result.index.loop_of(nid(4)).unwrap().node, nid(2));
    assert!(result.index.loop_of(nid(2)).is_none());
}

#[test]
fn break_outside_a_loop_is_rejected() {
    let result = run(
        vec![entry(1), terminator(2, "core/break")],
        vec![wire(1, 1, "then", 2)],
    );
    assert_eq!(codes(&result), [DiagnosticCode::BreakOutsideLoop]);
}

#[test]
fn node_reachable_from_two_entries_is_ambiguous_once() {
    let result = run(
        vec![entry(1), entry(2), action(3), action(4)],
        vec![
            wire(1, 1, "then", 3),
            wire(2, 2, "then", 3),
            wire(3, 3, "then", 4),
        ],
    );
    assert_eq!(codes(&result), [DiagnosticCode::RegionAmbiguous]);
    assert!(!result.index.owners.contains_key(&nid(3)));
}

#[test]
fn loop_body_edge_into_the_parent_region_escapes() {
    let result = run(
        vec![entry(1), looped(2, "core/count"), action(3), action(4)],
        vec![
            wire(1, 1, "then", 2),
            wire(2, 2, "body", 3),
            wire(3, 2, "completed", 4),
            wire(4, 3, "then", 4),
        ],
    );
    assert_eq!(codes(&result), [DiagnosticCode::RegionEscape]);
    assert_eq!(result.diagnostics[0].subject.node, Some(nid(3)));
}

#[test]
fn unreachable_exec_node_warns_without_an_owner() {
    let result = run(vec![entry(1), action(2)], vec![]);
    assert_eq!(codes(&result), [DiagnosticCode::Unreachable]);
    assert_eq!(result.diagnostics[0].severity, Severity::Warning);
    assert!(result.is_valid());
    assert!(result.index.unreachable.contains(&nid(2)));
}

#[test]
fn while_requires_a_positive_integer_bound() {
    let result = run(
        vec![entry(1), looped(2, "core/while")],
        vec![wire(1, 1, "then", 2)],
    );
    assert_eq!(codes(&result), [DiagnosticCode::LoopUnbounded]);
    let mut bounded = looped(2, "core/while");
    bounded.json["config"] = json!({"max_iterations": 10});
    let result = run(vec![entry(1), bounded], vec![wire(1, 1, "then", 2)]);
    assert!(result.is_valid(), "{:?}", result.diagnostics);
}
