//! Port, kind and suspension contracts of the registered language intrinsics (11 §8.4).

use orange_model::GraphDocument;
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{NodeKind, OutputSource, PortDirection, PortKind, SignatureSnapshot};
use orange_model::validation::{HostOrigin, validate_nodes};
use serde_json::{Value, json};

const NODE: &str = "10000000-0000-4000-8000-000000000001";

/// Resolves one `definition` node (with optional type pins and variadic state) against the
/// core registry, asserting it is accepted without diagnostics.
fn resolve(definition: &str, pins: Value, variadic: Value) -> SignatureSnapshot {
    let doc: GraphDocument = serde_json::from_value(json!({
        "format_version": 1,
        "graph_id": "20000000-0000-4000-8000-000000000001",
        "revision": 0,
        "nodes": [{
            "id": NODE, "definition_id": definition, "definition_version": 1,
            "config_version": 1, "config": {}, "type_pins": pins, "variadic": variadic
        }],
        "edges": [],
        "signatures": {}
    }))
    .unwrap();
    let origin = HostOrigin::new("0.1.0", format!("sha256:{}", "b".repeat(64))).unwrap();
    let report = validate_nodes(&doc, &RegistrySnapshot::core(origin));
    assert!(report.diagnostics.is_empty(), "{definition}: {report:?}");
    report.resolved[&NODE.parse().unwrap()].clone()
}

fn ports(signature: &SignatureSnapshot, direction: PortDirection, kind: PortKind) -> Vec<String> {
    signature
        .ports
        .iter()
        .filter(|port| port.direction == direction && port.kind == kind)
        .map(|port| port.port_id.as_str().to_owned())
        .collect()
}

fn exec_in(signature: &SignatureSnapshot) -> Vec<String> {
    ports(signature, PortDirection::Input, PortKind::Exec)
}

fn exec_out(signature: &SignatureSnapshot) -> Vec<String> {
    ports(signature, PortDirection::Output, PortKind::Exec)
}

fn output(signature: &SignatureSnapshot, id: &str) -> (String, Option<OutputSource>) {
    let port = signature
        .ports
        .iter()
        .find(|port| port.port_id.as_str() == id)
        .unwrap_or_else(|| panic!("no port {id}"));
    (port.ty.as_ref().unwrap().as_str().to_owned(), port.source)
}

#[test]
fn switch_has_case_outcomes_and_a_default_in_the_parent_region() {
    let signature = resolve(
        "core/switch",
        json!({"K": "core/string@1"}),
        json!({"case": {"order": [0, 1], "next": 2}}),
    );
    assert_eq!(signature.kind, NodeKind::Action);
    assert_eq!(exec_in(&signature), ["in"]);
    assert_eq!(exec_out(&signature), ["case", "default"]);
    let case = &signature.ports[2];
    assert_eq!(case.variadic.map(|range| range.min), Some(1));
}

#[test]
fn register_feedback_reads_state_not_its_inputs() {
    let register = resolve("core/register", json!({"T": "core/i64@1"}), json!({}));
    assert_eq!(exec_in(&register), ["store", "reset"]);
    assert_eq!(
        output(&register, "current"),
        ("core/i64@1".into(), Some(OutputSource::State))
    );
    assert_eq!(
        register
            .ports
            .iter()
            .find(|port| port.port_id.as_str() == "next")
            .and_then(|port| port.ty.as_ref())
            .map(|ty| ty.as_str()),
        Some("core/i64@1")
    );
}

#[test]
fn state_controls_expose_their_state_outputs() {
    let gate = resolve("core/gate", json!({}), json!({}));
    assert_eq!(exec_in(&gate), ["enter", "open", "close", "toggle"]);
    assert_eq!(
        output(&gate, "is_open"),
        ("core/bool@1".into(), Some(OutputSource::State))
    );

    let once = resolve("core/do-once", json!({}), json!({}));
    assert_eq!(exec_in(&once), ["in", "reset"]);
    // `then` is the kind's implicit outcome, not a declared port.
    assert_eq!(once.kind, NodeKind::Action);
    assert!(exec_out(&once).is_empty());

    let flip = resolve("core/flip-flop", json!({}), json!({}));
    assert_eq!(exec_out(&flip), ["a", "b"]);
    assert_eq!(
        output(&flip, "is_a"),
        ("core/bool@1".into(), Some(OutputSource::State))
    );
}
