//! Golden round trip of the 12 §6 saved-graph example (IDs replaced by UUIDs).
//! Regenerate the canonical file after an intended format change with
//! `ORANGE_BLESS=1 cargo test -p orange-model --test codec_golden`, then review its diff.

mod common;

use common::{edge_id, node_id, parse_json, read_fixture};
use orange_model::document::{FaultPolicy, OverflowPolicy, ReentrancyMode, VarScope};
use orange_model::id::{EdgeId, NodeId, PortKey};
use orange_model::literal::LiteralPayload;
use orange_model::{from_json, to_json};
use serde_json::json;

const SOURCE: &str = "threshold-v1.json";
const CANONICAL: &str = "threshold-v1.canonical.json";

#[test]
fn load_save_preserves_every_value() {
    let source = read_fixture(SOURCE);
    let saved = to_json(&from_json(&source).unwrap()).unwrap();
    // Object key order may change; values, array order and keys may not.
    assert_eq!(parse_json(&saved), parse_json(&source));
}

#[test]
fn save_matches_canonical_bytes_and_is_idempotent() {
    let saved = to_json(&from_json(&read_fixture(SOURCE)).unwrap()).unwrap();
    if std::env::var_os("ORANGE_BLESS").is_some() {
        std::fs::write(common::fixture_path(CANONICAL), &saved).unwrap();
    }
    let canonical = read_fixture(CANONICAL);
    assert_eq!(
        saved, canonical,
        "canonical output changed; see the module comment to bless"
    );
    assert_eq!(to_json(&from_json(&canonical).unwrap()).unwrap(), canonical);
}

#[test]
fn fields_are_reachable_through_the_model() {
    let document = from_json(&read_fixture(SOURCE)).unwrap();
    assert_eq!((document.format_version, document.revision), (1, 7));
    assert_eq!(document.package_lock.len(), 2);
    assert_eq!(document.root.nodes.len(), 9);
    assert_eq!(document.root.edges.len(), 11);

    // Creation order is the persisted array order.
    let order: Vec<String> = document
        .root
        .nodes
        .keys()
        .map(ToString::to_string)
        .collect();
    assert_eq!(order, (1..=9).map(node_id).collect::<Vec<_>>());

    let add = &document.root.nodes[&node_id(3).parse::<NodeId>().unwrap()];
    assert_eq!(add.definition.id.as_str(), "org.orange.math/add");
    assert!(!add.definition.id.is_core());
    assert_eq!(add.provider.as_ref().unwrap().id, "native-checked");
    assert_eq!(add.type_args["T"].as_str(), "core/i64@1");
    assert_eq!(add.config["int_overflow"], json!("checked"));
    assert_eq!(add.signature.as_ref().unwrap().as_str(), "s-add-i64");

    let greater = &document.root.nodes[&node_id(5).parse::<NodeId>().unwrap()];
    let b = &greater.inputs[&"b".parse::<PortKey>().unwrap()];
    assert_eq!(b.payload, LiteralPayload::Value(json!("100")));

    let manual = &document.root.nodes[&node_id(1).parse::<NodeId>().unwrap()];
    let activation = manual.activation.as_ref().unwrap();
    assert_eq!(
        (
            activation.mode,
            activation.capacity,
            activation.overflow,
            activation.on_fault
        ),
        (
            ReentrancyMode::Queue,
            256,
            OverflowPolicy::Fault,
            FaultPolicy::Continue
        )
    );

    let condition = &document.root.edges[&edge_id(7).parse::<EdgeId>().unwrap()];
    assert_eq!(condition.to.port.to_string(), "condition");

    let total = document.variables.values().next().unwrap();
    assert_eq!((total.name.as_str(), total.scope), ("total", VarScope::Run));
    assert_eq!(
        total.init.as_ref().unwrap().payload,
        LiteralPayload::Value(json!("0"))
    );

    assert_eq!(document.signatures.len(), 3);
    assert_eq!(document.exports[0].id, "total");
    assert!(document.functions.is_empty());
}
