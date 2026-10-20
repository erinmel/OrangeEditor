//! G-39 (presentation never changes semantic signatures) and G-29 (missing package and
//! snapshot drift lose no data) vectors, plus implicit kind ports.

mod common;

use std::collections::BTreeMap;

use common::{edge_id, node_id, read_fixture, saved};
use orange_model::document::{NodeKind, ResolvedPort, SignatureSnapshot};
use orange_model::id::{EdgeId, NodeId, PortId, TypeExpr};
use orange_model::signature::{SignatureDrift, compare, project, semantic_key};
use orange_model::{DocIndex, from_json};
use serde_json::{Value as Json, json};

const ADD_KEY: &str = include_str!("fixtures/s-add-i64.sha256");

/// The 12 §4 `org.orange.math/add` ports with the given captions (`None` = no presentation).
fn add_ports(captions: Option<[&str; 3]>) -> Vec<ResolvedPort> {
    let ids = ["a", "b", "sum"];
    let mut ports = vec![
        json!({"port_id": "a", "direction": "input", "kind": "data", "type": "$T"}),
        json!({"port_id": "b", "direction": "input", "kind": "data", "type": "$T"}),
        json!({"port_id": "sum", "direction": "output", "kind": "data", "type": "$T", "source": "computed"}),
    ];
    if let Some(captions) = captions {
        for ((port, id), caption) in ports.iter_mut().zip(ids).zip(captions) {
            port["label"] =
                json!({"key": format!("org.orange.math/add-port-{id}"), "fallback": caption});
            port["description"] = json!({"key": "k", "fallback": caption});
            port["group"] = json!("operands");
            port["editor"] = json!({"widget": "number"});
        }
    }
    ports
        .into_iter()
        .map(|port| serde_json::from_value(port).unwrap())
        .collect()
}

fn i64_args() -> BTreeMap<String, TypeExpr> {
    BTreeMap::from([("T".to_string(), "core/i64@1".parse().unwrap())])
}

#[test]
fn g39_presentation_changes_never_change_the_key() {
    let english = project(
        NodeKind::Pure,
        add_ports(Some(["A", "B", "Sum"])),
        &i64_args(),
    );
    let spanish = project(
        NodeKind::Pure,
        add_ports(Some(["A", "B", "Suma"])),
        &i64_args(),
    );
    let rtl = project(
        NodeKind::Pure,
        add_ports(Some([
            "\u{202B}أ\u{202C}",
            "\u{202B}ب\u{202C}",
            "\u{202B}مجموع\u{202C}",
        ])),
        &i64_args(),
    );
    let bare = project(NodeKind::Pure, add_ports(None), &i64_args());

    assert_eq!(english, spanish);
    assert_eq!(english, rtl);
    assert_eq!(
        english, bare,
        "missing presentation keeps the same snapshot"
    );
    assert_eq!(
        semantic_key(&english).as_str(),
        ADD_KEY.trim(),
        "matches the independent vector"
    );
}

#[test]
fn g39_projection_equals_the_stored_snapshot_and_writes_nothing() {
    let source = read_fixture("threshold-v1.json");
    let document = from_json(&source).unwrap();
    let before = (saved(&document), document.revision());

    let stored = &document.signatures()[&"s-add-i64".parse().unwrap()];
    let projected = project(
        NodeKind::Pure,
        add_ports(Some(["Suma", "B", "Σ"])),
        &i64_args(),
    );
    assert_eq!(&projected, stored);
    assert!(
        compare(stored, &projected).is_none(),
        "no drift from presentation"
    );
    assert_eq!((saved(&document), document.revision()), before);
}

fn nid(n: u8) -> NodeId {
    node_id(n).parse().unwrap()
}

fn eid(n: u8) -> EdgeId {
    edge_id(n).parse().unwrap()
}

fn snapshot(value: Json) -> SignatureSnapshot {
    serde_json::from_value(value).unwrap()
}

#[test]
fn g29_missing_package_keeps_snapshot_and_edges_and_drift_finds_orphans() {
    let source = read_fixture("threshold-v1.json");
    let document = from_json(&source).unwrap();
    let index = DocIndex::build(&document);
    let log = nid(9);

    // The diagnostics package is absent: nothing resolves, yet the node keeps its ports.
    let stored = &document.signatures()[document.node(log).unwrap().signature.as_ref().unwrap()];
    assert!(stored.declares_port(&"value".parse().unwrap()));
    assert!(
        index.orphan_edges(log, stored).is_empty(),
        "`in` is implicit for actions"
    );
    let edges: Vec<EdgeId> = index.incident(log).collect();
    assert_eq!(edges.len(), 2);

    // A newer package renames `value` to `message`.
    let resolved = snapshot(json!({"kind": "action", "ports": [
        {"port_id": "message", "direction": "input", "kind": "data", "type": "core/i64@1"}
    ]}));
    let drift = compare(stored, &resolved).unwrap();
    assert_eq!(
        drift,
        SignatureDrift {
            kind_changed: false,
            removed: vec!["value".parse().unwrap()],
            added: vec!["message".parse().unwrap()],
            changed: vec![],
        }
    );
    assert_eq!(
        index.orphan_edges(log, &resolved),
        [eid(11)],
        "edge to `value` is orphaned"
    );
    assert_eq!(
        saved(&document),
        {
            let reloaded = from_json(&source).unwrap();
            saved(&reloaded)
        },
        "no data loss: detection never edits the document"
    );
}

#[test]
fn drift_reports_changed_types_and_kinds() {
    let stored = snapshot(json!({"kind": "pure", "ports": [
        {"port_id": "a", "direction": "input", "kind": "data", "type": "core/i64@1"}
    ]}));
    let resolved = snapshot(json!({"kind": "action", "ports": [
        {"port_id": "a", "direction": "input", "kind": "data", "type": "core/f64@1"}
    ]}));
    let drift = compare(&stored, &resolved).unwrap();
    assert!(drift.kind_changed);
    assert_eq!(drift.changed, ["a".parse::<PortId>().unwrap()]);
}

#[test]
fn implicit_ports_follow_the_kind_table() {
    let implicit = |kind: &str, ports: Json| {
        let mut ports = snapshot(json!({"kind": kind, "ports": ports})).implicit_ports();
        ports.sort_unstable();
        ports
    };
    assert!(implicit("pure", json!([])).is_empty());
    assert!(implicit("volatile", json!([])).is_empty());
    assert_eq!(implicit("entry", json!([])), ["then"]);
    assert_eq!(
        implicit("action", json!([])),
        ["error", "failed", "in", "then"]
    );
    assert_eq!(
        implicit("async", json!([])),
        ["cancelled", "error", "failed", "in", "then"]
    );
    assert_eq!(
        implicit("iterator", json!([])),
        ["body", "cancelled", "completed", "error", "failed", "in"]
    );
    let outcomes = json!([{"port_id": "acquired", "direction": "output", "kind": "exec"}]);
    assert_eq!(
        implicit("action", outcomes),
        ["error", "failed", "in"],
        "declared outcomes replace `then`"
    );
}
