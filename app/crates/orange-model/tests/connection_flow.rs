//! 00c.7a: resolve → infer → select → insert → save/load → undo/redo through the public API.

mod common;

use common::generic::{
    add, document, package_digest, package_registry, ready_plan, typed_sink, typed_source,
};
use common::{content, nid, saved};
use orange_model::conversion::apply_connection;
use orange_model::inference::apply_inferred;
use orange_model::type_term::TypeTerm;
use orange_model::{DocCommand, from_json};
use serde_json::json;

#[test]
fn pinned_generic_fed_by_a_narrower_source_gets_a_persisted_converter() {
    let registry = package_registry();
    let mut doc = document(
        vec![
            typed_source(1, "i32"),
            add(2, json!({})),
            typed_sink(3, "i64", json!({})),
        ],
        vec![],
    );

    // Infer: pinning `T` resolves the generic node and projects its signature.
    let pin = DocCommand::SetTypePins {
        node: nid(2),
        pins: [("T".to_owned(), "core/i64@1".parse().unwrap())].into(),
    };
    let pinned = apply_inferred(&mut doc, [pin], &registry).unwrap();
    assert!(pinned.issues.is_empty());
    assert_eq!(
        doc.node(nid(2)).unwrap().type_args["T"].as_str(),
        "core/i64@1"
    );

    // Select + insert: i32 → i64 is a unique lossless entry, so a visible converter is planned
    // against the resolved `a: i64` port and applied as one step.
    let plan = ready_plan(&doc, &registry, (1, "value"), (2, "a"));
    let converter = plan
        .converter
        .clone()
        .expect("a converter node is inserted");
    assert_eq!(converter.resolved.entry.to_string(), "core/i32-to-i64@1");
    let before_insert = content(&doc);
    let inserted = apply_connection(&mut doc, &registry, plan).unwrap();
    let after_insert = content(&doc);

    // A matching wire out of the generic node needs no converter.
    let plan = ready_plan(&doc, &registry, (2, "sum"), (3, "value"));
    assert!(plan.converter.is_none());
    let wired = apply_connection(&mut doc, &registry, plan).unwrap();
    let complete = content(&doc);

    // Save/load keeps canonical types, the converter binding and the inferred arguments.
    let loaded = from_json(&saved(&doc)).unwrap();
    assert_eq!(content(&loaded), complete);
    let node = loaded.node(converter.node).unwrap();
    assert_eq!(node.config["converter"], "core/i32-to-i64");
    let provider = node.provider.as_ref().unwrap();
    let locked = loaded
        .package_lock()
        .iter()
        .find(|entry| entry.id == provider.package && entry.version == provider.version);
    assert_eq!(locked.unwrap().content_digest, package_digest());
    for snapshot in loaded.signatures().values() {
        for ty in snapshot.ports.iter().filter_map(|port| port.ty.as_ref()) {
            let term = TypeTerm::parse(ty).unwrap();
            assert!(
                term.contains_param() || registry.types().validate(&term).is_ok(),
                "{ty}"
            );
        }
    }

    // Undo walks back one step per edit; redo returns to the same document.
    let redo_wire = doc.revert(wired.inverse).unwrap();
    assert_eq!(content(&doc), after_insert);
    let redo_insert = doc.revert(inserted.inverse).unwrap();
    assert_eq!(content(&doc), before_insert);
    let _ = doc.revert(redo_insert.inverse).unwrap();
    assert_eq!(content(&doc), after_insert);
    let _ = doc.revert(redo_wire.inverse).unwrap();
    assert_eq!(content(&doc), complete);
}
