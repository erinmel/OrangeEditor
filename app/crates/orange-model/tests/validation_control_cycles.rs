use std::{collections::BTreeMap, sync::Arc};

use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{
    GraphDocument, NodeKind, PortDescriptor, SignatureSnapshot, Suspends,
};
use orange_model::from_json;
use orange_model::id::NodeId;
use orange_model::validation::{
    ConnectionValidation, ControlCycleValidation, DiagnosticCode, HostOrigin,
    validate_control_cycles,
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

struct N {
    json: Value,
    id: NodeId,
    signature: SignatureSnapshot,
}

fn node(n: u8, kind: NodeKind, suspends: Option<Suspends>) -> N {
    N {
        json: json!({
            "id": id(n), "definition_id": "example/node", "definition_version": 1,
            "config_version": 1, "config": {}, "signature": SIGNATURE
        }),
        id: nid(n),
        signature: SignatureSnapshot {
            kind,
            suspends,
            ports: vec![
                PortDescriptor::exec_in("in"),
                PortDescriptor::exec_out("then"),
            ],
            extensions: Default::default(),
        },
    }
}

fn action(n: u8) -> N {
    node(n, NodeKind::Action, None)
}

fn suspending(n: u8, suspends: Suspends) -> N {
    node(n, NodeKind::Async, Some(suspends))
}

fn wire(n: u8, from: u8, to: u8) -> Value {
    json!({
        "id": format!("30000000-0000-4000-8000-{n:012}"),
        "from": {"node": id(from), "port": "then"},
        "to": {"node": id(to), "port": "in"}
    })
}

fn run(nodes: Vec<N>, edges: Vec<Value>) -> ControlCycleValidation {
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
    validate_control_cycles(
        &doc,
        &resolved,
        &ConnectionValidation::default(),
        &registry(),
    )
}

#[test]
fn synchronous_cycle_reports_a_deterministic_closed_witness() {
    let result = run(
        vec![action(3), action(1), action(2)],
        vec![wire(1, 1, 2), wire(2, 2, 3), wire(3, 3, 1)],
    );
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].code, DiagnosticCode::CycleSyncControl);
    assert_eq!(result.cycle_paths, [vec![nid(1), nid(2), nid(3), nid(1)]]);
}

#[test]
fn self_loop_is_a_cycle() {
    let result = run(vec![action(1)], vec![wire(1, 1, 1)]);
    assert_eq!(result.cycle_paths, [vec![nid(1), nid(1)]]);
}

#[test]
fn always_suspending_node_certifies_the_cycle() {
    let result = run(
        vec![suspending(1, Suspends::Always), action(2)],
        vec![wire(1, 1, 2), wire(2, 2, 1)],
    );
    assert!(result.is_valid(), "{:?}", result.diagnostics);
}

#[test]
fn maybe_suspension_does_not_certify() {
    let result = run(
        vec![suspending(1, Suspends::Maybe), action(2)],
        vec![wire(1, 1, 2), wire(2, 2, 1)],
    );
    assert_eq!(result.cycle_paths.len(), 1);
}

#[test]
fn a_second_cycle_that_avoids_the_suspension_is_still_reported() {
    // 1 (always) -> 2 -> 1 is certified; 2 -> 3 -> 2 avoids node 1.
    let result = run(
        vec![suspending(1, Suspends::Always), action(2), action(3)],
        vec![wire(1, 1, 2), wire(2, 2, 1), wire(3, 2, 3), wire(4, 3, 2)],
    );
    assert_eq!(result.cycle_paths, [vec![nid(2), nid(3), nid(2)]]);
}

#[test]
fn acyclic_exec_chain_is_valid() {
    let result = run(vec![action(1), action(2)], vec![wire(1, 1, 2)]);
    assert!(result.is_valid());
    assert!(result.cycle_paths.is_empty());
}
