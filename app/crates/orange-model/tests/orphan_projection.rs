//! The authoring orphan projection and the validator agree on which edges have no port,
//! including variadic instances (11 §4.5).

mod common;

use std::collections::BTreeSet;

use common::{branch_node, edge_id, eid, minimal_document, nid, node_id};
use orange_model::catalog::RegistrySnapshot;
use orange_model::id::EdgeId;
use orange_model::validation::{DiagnosticCode, HostOrigin};
use orange_model::{DocCommand, DocIndex, GraphDocument, from_json, validate};
use serde_json::json;

fn registry() -> std::sync::Arc<RegistrySnapshot> {
    RegistrySnapshot::core(HostOrigin::new("1.0.0", format!("sha256:{}", "a".repeat(64))).unwrap())
}

fn wire(n: u8, from_port: &str, to: u8) -> serde_json::Value {
    json!({"id": edge_id(n), "from": {"node": node_id(1), "port": from_port},
        "to": {"node": node_id(to), "port": "in"}})
}

/// Edges the validator reports as `E-PORT-UNKNOWN`, and those the projection reports, at
/// the Fork node.
fn observations(document: &GraphDocument) -> (BTreeSet<EdgeId>, BTreeSet<EdgeId>) {
    let report = validate(document, &registry());
    let unknown = report
        .diagnostics
        .iter()
        .filter(|d| d.code == DiagnosticCode::PortUnknown && d.subject.node == Some(nid(1)))
        .filter_map(|d| d.subject.edge)
        .collect();
    let fork = document.node(nid(1)).unwrap();
    let orphans = DocIndex::build(document)
        .orphan_edges(fork, &report.resolved[&nid(1)])
        .into_iter()
        .collect();
    (unknown, orphans)
}

#[test]
fn stale_variadic_instances_are_orphans_for_the_projection_and_the_validator() {
    let fork = json!({"id": node_id(1), "definition_id": "core/fork", "definition_version": 1,
        "config_version": 1, "config": {"join": "all"},
        "variadic": {"branch": {"order": [0], "next": 1}}});
    let text = minimal_document(json!({
        "nodes": [fork, branch_node(2), branch_node(3), branch_node(4), branch_node(5)],
        "edges": [
            wire(1, "branch#0", 2), // active instance
            wire(2, "branch#9", 3), // instance missing from the variadic state
            wire(3, "branch", 4),   // bare variadic base port
            wire(4, "nope", 5),     // undeclared port
        ]
    }));
    let mut document = from_json(&text).unwrap();
    let stale: BTreeSet<EdgeId> = [eid(2), eid(3), eid(4)].into();
    let (unknown, orphans) = observations(&document);
    assert_eq!(unknown, stale);
    assert_eq!(orphans, stale);

    // Removing the active instance takes its edge with it; the stale edges stay for repair.
    // Both views keep agreeing, and undo restores the instance and its edge.
    let committed = document
        .apply([DocCommand::RemoveVariadic {
            node: nid(1),
            port: "branch#0".parse().unwrap(),
        }])
        .unwrap();
    assert!(!document.root().edges.contains_key(&eid(1)));
    assert_eq!(observations(&document), (stale.clone(), stale.clone()));
    let _ = document.revert(committed.inverse).unwrap();
    assert!(document.root().edges.contains_key(&eid(1)));
    assert_eq!(observations(&document), (stale.clone(), stale));
}
