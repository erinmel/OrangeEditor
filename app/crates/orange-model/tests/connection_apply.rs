//! 00c.6b: applying a connection plan in one transaction, with undo and redo.

mod common;

use std::sync::Arc;

use common::generic::{add, catalog, data_edge, document, typed_sink, typed_source};
use common::{content, eid, id_source, nid, port, saved, typed_edge};
use orange_model::conversion::{
    ConnectionOutcome, ConnectionPlan, ConnectionRequest, ConverterKey, ConverterProvider,
    ConverterRegistry, ConverterRegistryBuilder, PackageProvider, PlanApplyError, apply_connection,
    plan_connection,
};
use orange_model::types::TypeRegistry;
use orange_model::{DocCommand, GraphDocument, TxError, from_json};
use serde_json::json;

fn digest() -> String {
    format!("sha256:{}", "ab".repeat(32))
}

/// Core conversions, with a package implementation of `i32 → i64` as the new-binding default.
fn converters() -> Arc<ConverterRegistry> {
    let mut builder =
        ConverterRegistryBuilder::with_core_conversions(Arc::new(TypeRegistry::core())).unwrap();
    let key = ConverterKey {
        id: "core/i32-to-i64".parse().unwrap(),
        contract_version: 1,
    };
    let provider = PackageProvider {
        package: "org.test".to_owned(),
        version: "1.0.0".to_owned(),
        content_digest: digest(),
        id: "native".to_owned(),
        contract_version: 1,
    };
    builder
        .register_package_provider("org.test", &key, provider.clone())
        .unwrap();
    builder
        .set_new_binding_default(&key, ConverterProvider::Package(provider))
        .unwrap();
    builder.build()
}

fn plan(
    doc: &GraphDocument,
    converters: &ConverterRegistry,
    from: (u8, &str),
    to: (u8, &str),
) -> ConnectionPlan {
    let request = ConnectionRequest {
        from: port(from.0, from.1),
        to: port(to.0, to.1),
        choice: None,
    };
    match plan_connection(doc, converters, &catalog(None), request, id_source(5)).unwrap() {
        ConnectionOutcome::Ready(plan) => *plan,
        other => panic!("expected a plan, got {other:?}"),
    }
}

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
    let (registry, mut doc) = (converters(), convertible());
    let (before, revision) = (content(&doc), doc.revision());
    let plan = plan(&doc, &registry, (1, "value"), (3, "value"));
    let (node, edges) = (plan.converter.as_ref().unwrap().node, plan.edges.clone());

    let done = apply_connection(&mut doc, &registry, plan).unwrap();
    assert_eq!(doc.revision(), revision + 1);
    let provider = doc.node(node).unwrap().provider.clone().unwrap();
    assert_eq!(provider.package, "org.test");
    assert_eq!(provider.extensions["content_digest"], json!(digest()));
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
    let registry = converters();
    let mut doc = document(
        vec![
            typed_source(1, "i32"),
            typed_source(2, "i64"),
            typed_sink(3, "i64", json!({})),
        ],
        vec![data_edge(1, (2, "value"), (3, "value"))],
    );
    let (before, revision) = (content(&doc), doc.revision());
    let plan = plan(&doc, &registry, (1, "value"), (3, "value"));
    let done = apply_connection(&mut doc, &registry, plan).unwrap();
    assert!(doc.edge(eid(1)).is_none());
    assert_eq!(doc.revision(), revision + 1);

    let _ = doc.revert(done.inverse).unwrap();
    assert_eq!(content(&doc), before);
}

#[test]
fn a_direct_plan_applies_its_inferred_updates_and_plain_connect_still_works() {
    let registry = converters();
    let mut doc = document(vec![typed_source(1, "i32"), add(2, json!({}))], vec![]);
    let plan = plan(&doc, &registry, (1, "value"), (2, "a"));
    let _ = apply_connection(&mut doc, &registry, plan).unwrap();
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
    let registry = converters();
    let mut doc = convertible();
    let stale = plan(&doc, &registry, (1, "value"), (3, "value"));

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
        let result = apply_connection(target, &registry, stale.clone());
        assert!(matches!(result, Err(PlanApplyError::StaleDocument { .. })));
        assert_eq!((content(target), target.revision()), (before, revision));
    }

    // A replaced registry snapshot invalidates it too.
    let (before, revision) = (content(&doc), doc.revision());
    let result = apply_connection(&mut doc, &converters(), stale);
    assert_eq!(result, Err(PlanApplyError::StaleRegistry));
    assert_eq!((content(&doc), doc.revision()), (before, revision));
}

#[test]
fn an_injected_failure_rolls_back_every_command() {
    let registry = converters();
    let mut doc = convertible();
    let mut plan = plan(&doc, &registry, (1, "value"), (3, "value"));
    plan.commands.push(DocCommand::Disconnect { edge: eid(99) });
    let (before, revision, state) = (content(&doc), doc.revision(), doc.state_id());

    let result = apply_connection(&mut doc, &registry, plan);
    assert_eq!(
        result,
        Err(PlanApplyError::Edit(TxError::EdgeNotFound(eid(99))))
    );
    assert_eq!(content(&doc), before);
    assert_eq!((doc.revision(), doc.state_id()), (revision, state));
}
