//! 00c.6b: applying a connection plan in one transaction, with undo and redo.

mod common;

use common::generic::{
    add, catalog, data_edge, document, package_converters, package_digest, ready_plan, typed_sink,
    typed_source,
};
use common::{body, content, eid, function, function_id, nid, port, saved, typed_edge};
use orange_model::conversion::{PlanApplyError, apply_connection};
use orange_model::inference::TemplateCatalog;
use orange_model::{BodyRef, DocCommand, GraphDocument, from_json};
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
    let (registry, mut doc) = (package_converters(), convertible());
    let (before, revision) = (content(&doc), doc.revision());
    let plan = ready_plan(&doc, &registry, (1, "value"), (3, "value"));
    let (node, edges) = (plan.converter.as_ref().unwrap().node, plan.edges.clone());

    let done = apply_connection(&mut doc, &registry, &catalog(None), plan).unwrap();
    assert_eq!(doc.revision(), revision + 1);
    let provider = doc.node(node).unwrap().provider.clone().unwrap();
    assert_eq!(provider.package, "org.test");
    assert_eq!(
        provider.extensions["content_digest"],
        json!(package_digest())
    );
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
    let registry = package_converters();
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
    let done = apply_connection(&mut doc, &registry, &catalog(None), plan).unwrap();
    assert!(doc.edge(eid(1)).is_none());
    assert_eq!(doc.revision(), revision + 1);

    let _ = doc.revert(done.inverse).unwrap();
    assert_eq!(content(&doc), before);
}

#[test]
fn a_direct_plan_applies_its_inferred_updates_and_plain_connect_still_works() {
    let registry = package_converters();
    let mut doc = document(vec![typed_source(1, "i32"), add(2, json!({}))], vec![]);
    let plan = ready_plan(&doc, &registry, (1, "value"), (2, "a"));
    let _ = apply_connection(&mut doc, &registry, &catalog(None), plan).unwrap();
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
    let registry = package_converters();
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
        let result = apply_connection(target, &registry, &catalog(None), stale.clone());
        assert!(matches!(result, Err(PlanApplyError::StaleDocument { .. })));
        assert_eq!((content(target), target.revision()), (before, revision));
    }

    // A replaced registry snapshot invalidates it too.
    let (before, revision) = (content(&doc), doc.revision());
    let result = apply_connection(
        &mut doc,
        &package_converters(),
        &catalog(None),
        stale.clone(),
    );
    assert_eq!(result, Err(PlanApplyError::StaleRegistry));
    assert_eq!((content(&doc), doc.revision()), (before.clone(), revision));

    // So does a changed template catalog.
    let result = apply_connection(&mut doc, &registry, &TemplateCatalog::default(), stale);
    assert_eq!(result, Err(PlanApplyError::StaleTemplates));
    assert_eq!((content(&doc), doc.revision()), (before, revision));
}

#[test]
fn a_converter_planned_inside_a_function_body_is_inserted_there() {
    let registry = package_converters();
    let mut doc = document(vec![], vec![]);
    let nodes = [typed_source(1, "i32"), typed_sink(3, "i64", json!({}))]
        .map(|node| serde_json::from_value(node).unwrap());
    let _ = doc
        .apply([DocCommand::UpsertFunction(function(body(nodes, [])))])
        .unwrap();
    let plan = ready_plan(&doc, &registry, (1, "value"), (3, "value"));
    let node = plan.converter.as_ref().unwrap().node;

    let _ = apply_connection(&mut doc, &registry, &catalog(None), plan).unwrap();
    let inside = Some(BodyRef::Function(function_id()));
    assert_eq!(doc.locate_node(node), inside);
    assert!(doc.root().nodes.is_empty() && doc.root().edges.is_empty());
}
