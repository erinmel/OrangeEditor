//! 00c.6b: applying a connection plan in one transaction, with undo and redo.

mod common;

use common::generic::{
    add, data_edge, document, no_overlays, origin, package_converters, package_digest,
    package_registry, ready_plan, registry_with, typed_sink, typed_source,
};
use common::{body, content, eid, function, function_id, id_source, nid, port, saved, typed_edge};
use orange_model::catalog::{NodeContract, RegistrySnapshot};
use orange_model::conversion::{
    ConnectionError, ConnectionRequest, PlanApplyError, apply_connection, plan_connection,
};
use orange_model::document::{NodeKind, PackageLockEntry};
use orange_model::inference::NodeOverlays;
use orange_model::{BodyRef, DocCommand, GraphDocument, TxError, from_json};
use serde_json::json;

/// i32 source 1 → i64 sink 3, whose literal is shadowed once connected.
fn convertible() -> GraphDocument {
    let literal = json!({"inputs": {"value": {"type": "core/i64@1", "value": "7"}}});
    document(
        vec![typed_source(1, "i32"), typed_sink(3, "i64", literal)],
        vec![],
    )
}

#[test]
fn insertion_is_one_undo_step_and_redo_restores_identities_and_provider() {
    let (registry, mut doc) = (package_registry(), convertible());
    let (before, revision) = (content(&doc), doc.revision());
    let plan = ready_plan(&doc, &registry, (1, "value"), (3, "value"));
    let (node, edges) = (plan.converter.as_ref().unwrap().node, plan.edges.clone());

    let done = apply_connection(&mut doc, &registry, &no_overlays(), plan).unwrap();
    assert_eq!(doc.revision(), revision + 1);
    let provider = doc.node(node).unwrap().provider.clone().unwrap();
    assert_eq!(provider.package, "org.test");
    assert!(
        provider.extensions.is_empty(),
        "the digest lives in the lock"
    );
    let lock = doc.package_lock();
    assert_eq!(lock.len(), 1);
    assert_eq!(
        (lock[0].id.as_str(), lock[0].version.as_str()),
        ("org.test", "1.0.0")
    );
    assert_eq!(lock[0].content_digest, package_digest());
    assert!(edges.iter().all(|&edge| doc.edge(edge).is_some()));
    assert_eq!(doc.node(nid(3)).unwrap().inputs.len(), 1, "literal stays");
    let after = content(&doc);

    let redo = doc.revert(done.inverse).unwrap().inverse;
    assert_eq!(content(&doc), before);
    assert!(doc.node(node).is_none() && edges.iter().all(|&e| doc.edge(e).is_none()));

    let _ = doc.revert(redo).unwrap();
    assert_eq!(content(&doc), after);
    assert_eq!(doc.node(node).unwrap().provider, Some(provider));
    assert_eq!(content(&from_json(&saved(&doc)).unwrap()), after);
}

#[test]
fn replacement_removes_the_old_edge_in_the_same_step() {
    let registry = package_registry();
    let mut doc = document(
        vec![
            typed_source(1, "i32"),
            typed_source(2, "i64"),
            typed_sink(3, "i64", json!({})),
        ],
        vec![data_edge(1, (2, "value"), (3, "value"))],
    );
    let (before, revision) = (content(&doc), doc.revision());
    let plan = ready_plan(&doc, &registry, (1, "value"), (3, "value"));
    let done = apply_connection(&mut doc, &registry, &no_overlays(), plan).unwrap();
    assert!(doc.edge(eid(1)).is_none());
    assert_eq!(doc.revision(), revision + 1);

    let _ = doc.revert(done.inverse).unwrap();
    assert_eq!(content(&doc), before);
}

#[test]
fn a_direct_plan_applies_its_inferred_updates_and_plain_connect_still_works() {
    let registry = package_registry();
    let mut doc = document(vec![typed_source(1, "i32"), add(2, json!({}))], vec![]);
    let plan = ready_plan(&doc, &registry, (1, "value"), (2, "a"));
    let _ = apply_connection(&mut doc, &registry, &no_overlays(), plan).unwrap();
    assert_eq!(
        doc.node(nid(2)).unwrap().type_args["T"].as_str(),
        "core/i32@1"
    );

    let mut plain = convertible();
    let edge = typed_edge(9, port(1, "value"), port(3, "value"));
    let _ = plain.apply([DocCommand::Connect { edge }]).unwrap();
}

#[test]
fn stale_plans_are_rejected_without_changing_content_or_revision() {
    let registry = package_registry();
    let mut doc = convertible();
    let stale = ready_plan(&doc, &registry, (1, "value"), (3, "value"));

    // A changed document and a reloaded copy both differ in content state.
    let mut changed = doc.clone();
    let _ = changed
        .apply([DocCommand::SetLiteral {
            port: port(3, "value"),
            literal: None,
        }])
        .unwrap();
    let mut reloaded = from_json(&saved(&doc)).unwrap();
    for target in [&mut changed, &mut reloaded] {
        let (before, revision) = (content(target), target.revision());
        let result = apply_connection(target, &registry, &no_overlays(), stale.clone());
        assert!(matches!(result, Err(PlanApplyError::StaleDocument { .. })));
        assert_eq!((content(target), target.revision()), (before, revision));
    }

    // A replaced registry snapshot invalidates it too.
    let (before, revision) = (content(&doc), doc.revision());
    let result = apply_connection(
        &mut doc,
        &registry_with(package_converters(), None),
        &no_overlays(),
        stale.clone(),
    );
    assert_eq!(result, Err(PlanApplyError::StaleRegistry));
    assert_eq!((content(&doc), doc.revision()), (before.clone(), revision));

    // So do overlays installed after planning.
    let mut overlays = no_overlays();
    let node = doc.node(nid(3)).unwrap().clone();
    let contract = NodeContract {
        kind: NodeKind::Pure,
        suspends: None,
        type_params: vec![],
        ports: vec![],
    };
    overlays.insert_node(&doc, &node, contract).unwrap();
    let result = apply_connection(&mut doc, &registry, &overlays, stale.clone());
    assert_eq!(result, Err(PlanApplyError::StaleOverlays));
    assert_eq!((content(&doc), doc.revision()), (before.clone(), revision));

    // So does a snapshot with different node definitions.
    let result = apply_connection(
        &mut doc,
        &RegistrySnapshot::core(origin()),
        &no_overlays(),
        stale,
    );
    assert_eq!(result, Err(PlanApplyError::StaleRegistry));
    assert_eq!((content(&doc), doc.revision()), (before, revision));
}

#[test]
fn a_converter_planned_inside_a_function_body_is_inserted_there() {
    let registry = package_registry();
    let mut doc = document(vec![], vec![]);
    let nodes = [typed_source(1, "i32"), typed_sink(3, "i64", json!({}))]
        .map(|node| serde_json::from_value(node).unwrap());
    let _ = doc
        .apply([DocCommand::UpsertFunction(function(body(nodes, [])))])
        .unwrap();
    let plan = ready_plan(&doc, &registry, (1, "value"), (3, "value"));
    let node = plan.converter.as_ref().unwrap().node;

    let _ = apply_connection(&mut doc, &registry, &no_overlays(), plan).unwrap();
    let inside = Some(BodyRef::Function(function_id()));
    assert_eq!(doc.locate_node(node), inside);
    assert!(doc.root().nodes.is_empty() && doc.root().edges.is_empty());
}

#[test]
fn a_document_locking_the_provider_to_another_digest_cannot_plan_it() {
    let registry = package_registry();
    let mut doc = convertible();
    let other = PackageLockEntry {
        id: "org.test".to_owned(),
        version: "1.0.0".to_owned(),
        content_digest: format!("sha256:{}", "cd".repeat(32)),
        extensions: Default::default(),
    };
    let _ = doc.apply([DocCommand::PutPackageLock(other)]).unwrap();
    let request = ConnectionRequest {
        from: port(1, "value"),
        to: port(3, "value"),
        choice: None,
    };
    let result = plan_connection(
        &doc,
        &registry,
        &NodeOverlays::default(),
        request,
        id_source(5),
    );
    assert!(matches!(
        result,
        Err(ConnectionError::Edit(TxError::PackageLockConflict { .. }))
    ));
}

#[test]
fn replacing_the_only_type_evidence_reinfers_and_preserves_the_shadowed_literal() {
    let registry = package_registry();
    let mut doc = document(
        vec![
            typed_source(1, "i64"),
            typed_source(2, "f64"),
            add(
                3,
                json!({
                    "type_args": {"T": "core/i64@1"},
                    "inputs": {"a": {"type": "core/i32@1", "value": 7}}
                }),
            ),
        ],
        vec![data_edge(1, (1, "value"), (3, "a"))],
    );
    let before = content(&doc);
    let literal = doc.node(nid(3)).unwrap().inputs.clone();
    let plan = ready_plan(&doc, &registry, (2, "value"), (3, "a"));
    assert!(plan.converter.is_none() && plan.issues.is_empty());
    assert_eq!(content(&doc), before, "planning is read-only");
    let done = apply_connection(&mut doc, &registry, &no_overlays(), plan).unwrap();
    let node = doc.node(nid(3)).unwrap();
    assert_eq!(node.type_args["T"].as_str(), "core/f64@1");
    assert_eq!(node.inputs, literal);
    let snapshot = &doc.signatures()[node.signature.as_ref().unwrap()];
    assert!(
        snapshot
            .ports
            .iter()
            .all(|p| p.ty.as_ref().unwrap().as_str() == "core/f64@1")
    );
    let after = content(&doc);
    let redo = doc.revert(done.inverse).unwrap().inverse;
    assert_eq!(content(&doc), before);
    let _ = doc.revert(redo).unwrap();
    assert_eq!(content(&doc), after);
}

#[test]
fn replacement_conversion_uses_pins_or_remaining_type_evidence() {
    for pinned in [false, true] {
        let registry = package_registry();
        let extra = if pinned {
            json!({"type_args": {"T": "core/f64@1"}, "type_pins": {"T": "core/i64@1"}})
        } else {
            json!({"type_args": {"T": "core/f64@1"}})
        };
        let mut edges = vec![data_edge(1, (1, "value"), (3, "a"))];
        if !pinned {
            edges.push(data_edge(2, (1, "value"), (3, "b")));
        }
        let mut doc = document(
            vec![
                typed_source(1, "i64"),
                typed_source(2, "i32"),
                add(3, extra),
            ],
            edges,
        );
        let pins = doc.node(nid(3)).unwrap().type_pins.clone();
        let plan = ready_plan(&doc, &registry, (2, "value"), (3, "a"));
        assert_eq!(
            plan.converter.as_ref().unwrap().resolved.entry.to_string(),
            "core/i32-to-i64@1"
        );
        let _ = apply_connection(&mut doc, &registry, &no_overlays(), plan).unwrap();
        assert_eq!(
            doc.node(nid(3)).unwrap().type_args["T"].as_str(),
            "core/i64@1"
        );
        assert_eq!(doc.node(nid(3)).unwrap().type_pins, pins);
    }
}

#[test]
fn selection_reinfers_the_generic_source_instead_of_using_its_cache() {
    let registry = package_registry();
    let doc = document(
        vec![
            add(
                1,
                json!({"type_args": {"T": "core/i64@1"}, "type_pins": {"T": "core/f64@1"}}),
            ),
            typed_sink(3, "f64", json!({})),
        ],
        vec![],
    );
    let plan = ready_plan(&doc, &registry, (1, "sum"), (3, "value"));
    assert!(plan.converter.is_none() && plan.issues.is_empty());
}
