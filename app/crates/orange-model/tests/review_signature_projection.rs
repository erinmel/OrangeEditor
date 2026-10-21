use std::collections::BTreeMap;

use orange_model::document::{NodeKind, ResolvedPort, SignatureSnapshot};
use orange_model::signature::{project, semantic_key};
use serde_json::{Value, json};

fn port(default: Value) -> ResolvedPort {
    serde_json::from_value(json!({
        "port_id": "value",
        "direction": "input",
        "kind": "data",
        "type": "core/f64@1",
        "default": default,
    }))
    .unwrap()
}

fn snapshot(default: Value) -> SignatureSnapshot {
    project(NodeKind::Pure, [port(default)], &BTreeMap::new())
}

#[test]
fn float_default_display_does_not_change_semantic_identity() {
    let short = snapshot(json!({
        "type": "core/f64@1",
        "bits": "3ff0000000000000",
        "display": "1.0",
    }));
    let long = snapshot(json!({
        "type": "core/f64@1",
        "bits": "3ff0000000000000",
        "display": "1.000",
    }));
    let absent = snapshot(json!({
        "type": "core/f64@1",
        "bits": "3ff0000000000000",
    }));

    assert_eq!(short, long);
    assert_eq!(short, absent);
    assert_eq!(semantic_key(&short), semantic_key(&long));
    assert_eq!(semantic_key(&short), semantic_key(&absent));
}

#[test]
fn changes_to_float_default_bits_change_semantic_identity() {
    let one = snapshot(json!({
        "type": "core/f64@1",
        "bits": "3ff0000000000000",
        "display": "1.0",
    }));
    let two = snapshot(json!({
        "type": "core/f64@1",
        "bits": "4000000000000000",
        "display": "2.0",
    }));

    assert_ne!(one, two);
    assert_ne!(semantic_key(&one), semantic_key(&two));
}

#[test]
fn default_type_remains_part_of_semantic_identity() {
    let f64_default = snapshot(json!({
        "type": "core/f64@1",
        "bits": "3ff0000000000000",
    }));
    let f32_default = snapshot(json!({
        "type": "core/f32@1",
        "bits": "3f800000",
    }));

    assert_ne!(semantic_key(&f64_default), semantic_key(&f32_default));
}
