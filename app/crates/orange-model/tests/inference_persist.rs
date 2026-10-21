//! 00c.4b: inference results persisted in the same transaction as the edit (11 §5.5).

mod common;

use common::generic::{add, catalog, data_edge, document, source};
use common::{content, eid, nid, saved};
use orange_model::document::{Edge, NodeInstance};
use orange_model::inference::{InferenceIssue, apply_inferred, plan_edit};
use orange_model::types::TypeRegistry;
use orange_model::{BodyRef, DocCommand, GraphDocument, TxError, from_json};
use serde_json::{Value as Json, json};

fn connect(edge: Json) -> DocCommand {
    DocCommand::Connect {
        edge: serde_json::from_value::<Edge>(edge).unwrap(),
    }
}

fn add_node(node: Json) -> DocCommand {
    DocCommand::AddNode {
        body: BodyRef::Root,
        node: serde_json::from_value::<NodeInstance>(node).unwrap(),
    }
}

fn commit(
    document: &mut GraphDocument,
    edit: Vec<DocCommand>,
) -> Result<Vec<InferenceIssue>, TxError> {
    apply_inferred(document, edit, &catalog(None), &TypeRegistry::core()).map(|done| done.issues)
}

fn type_arg(document: &GraphDocument, n: u8) -> Option<String> {
    let args = &document.node(nid(n))?.type_args;
    args.get("T").map(|ty| ty.to_string())
}

/// The types of the node's referenced snapshot, in port order.
fn signature_types(document: &GraphDocument, n: u8) -> Vec<String> {
    let key = document.node(nid(n)).unwrap().signature.as_ref().unwrap();
    document.signatures()[key]
        .ports
        .iter()
        .map(|port| port.ty.as_ref().unwrap().to_string())
        .collect()
}

fn codes(issues: &[InferenceIssue]) -> Vec<&'static str> {
    issues.iter().map(InferenceIssue::code).collect()
}

/// An `i64` source (1) and an unresolved `add` (2), not yet connected.
fn unconnected() -> GraphDocument {
    document(vec![source(1), add(2, json!({}))], vec![])
}

#[test]
fn resolved_bindings_and_signature_survive_save_and_load() {
    let mut doc = unconnected();
    let issues = commit(
        &mut doc,
        vec![connect(data_edge(1, (1, "value"), (2, "a")))],
    )
    .unwrap();
    assert!(issues.is_empty());
    assert_eq!(type_arg(&doc, 2).as_deref(), Some("core/i64@1"));
    assert_eq!(signature_types(&doc, 2), ["core/i64@1"; 3]);

    let loaded = from_json(&saved(&doc)).unwrap();
    assert_eq!(loaded.node(nid(2)), doc.node(nid(2)));
    assert_eq!(signature_types(&loaded, 2), ["core/i64@1"; 3]);
    assert_eq!(content(&loaded), content(&doc));
}

#[test]
fn one_undo_restores_the_prior_state_and_redo_the_accepted_result() {
    let mut doc = unconnected();
    let (before, revision) = (content(&doc), doc.revision());
    let done = apply_inferred(
        &mut doc,
        vec![connect(data_edge(1, (1, "value"), (2, "a")))],
        &catalog(None),
        &TypeRegistry::core(),
    )
    .unwrap();
    assert_eq!(doc.revision(), revision + 1);
    let after = content(&doc);

    let redo = doc.revert(done.committed.inverse).unwrap().inverse;
    assert_eq!(content(&doc), before);
    assert_eq!(type_arg(&doc, 2), None);
    assert_eq!(doc.node(nid(2)).unwrap().signature, None);
    assert!(doc.signatures().len() == 1, "only the source snapshot");

    let _ = doc.revert(redo).unwrap();
    assert_eq!(content(&doc), after);
    assert_eq!(doc.revision(), revision + 3);
}

#[test]
fn a_failed_edit_leaves_content_revision_and_state_unchanged() {
    let mut doc = unconnected();
    let (before, revision, state) = (content(&doc), doc.revision(), doc.state_id());
    let result = commit(
        &mut doc,
        vec![
            connect(data_edge(1, (1, "value"), (2, "a"))),
            DocCommand::Disconnect { edge: eid(9) },
        ],
    );
    assert_eq!(result, Err(TxError::EdgeNotFound(eid(9))));
    assert_eq!(content(&doc), before);
    assert_eq!((doc.revision(), doc.state_id()), (revision, state));
}

#[test]
fn disconnect_clears_the_stale_side_and_keeps_the_pinned_side() {
    // 2 is pinned to f64 and feeds 3, whose cached binding came from that edge.
    let cached = json!({"type_args": {"T": "core/f64@1"}});
    let mut doc = document(
        vec![
            add(
                2,
                json!({"type_args": {"T": "core/f64@1"}, "type_pins": {"T": "core/f64@1"}}),
            ),
            add(3, cached),
        ],
        vec![data_edge(1, (2, "sum"), (3, "a"))],
    );
    let issues = commit(&mut doc, vec![DocCommand::Disconnect { edge: eid(1) }]).unwrap();

    assert_eq!(codes(&issues), ["E-TYPE-AMBIGUOUS"]);
    assert_eq!(type_arg(&doc, 3), None);
    assert_eq!(signature_types(&doc, 3), ["$T"; 3]);
    let pinned = doc.node(nid(2)).unwrap();
    assert_eq!(pinned.type_pins["T"].as_str(), "core/f64@1");
    assert_eq!(type_arg(&doc, 2).as_deref(), Some("core/f64@1"));
    assert_eq!(signature_types(&doc, 2), ["core/f64@1"; 3]);
}

#[test]
fn unrelated_nodes_and_provider_locks_are_untouched() {
    let provider = json!({"package": "org.test", "version": "1.0.0", "id": "add"});
    // Node 4 is another generic component with a stale cache; the edit never touches it.
    let stale = json!({"provider": provider, "type_args": {"T": "core/i32@1"}});
    let mut doc = document(
        vec![
            source(1),
            add(2, json!({"provider": provider})),
            add(4, stale),
        ],
        vec![],
    );
    let (provider_2, node_4) = (
        doc.node(nid(2)).unwrap().provider.clone(),
        doc.node(nid(4)).unwrap().clone(),
    );
    let done = apply_inferred(
        &mut doc,
        vec![connect(data_edge(1, (1, "value"), (2, "a")))],
        &catalog(None),
        &TypeRegistry::core(),
    )
    .unwrap();

    assert_eq!(doc.node(nid(2)).unwrap().provider, provider_2);
    assert_eq!(doc.node(nid(4)), Some(&node_4));
    assert!(!done.committed.change_set.nodes.changed.contains(&nid(4)));
}

#[test]
fn planning_reports_a_conflict_without_changing_the_document() {
    let mut doc = document(
        vec![
            add(2, json!({"type_pins": {"T": "core/i64@1"}})),
            add(3, json!({"type_pins": {"T": "core/f64@1"}})),
        ],
        vec![],
    );
    let edit = vec![connect(data_edge(1, (2, "sum"), (3, "a")))];
    let (before, revision) = (content(&doc), doc.revision());
    let plan = plan_edit(&doc, edit.clone(), &catalog(None), &TypeRegistry::core()).unwrap();
    assert_eq!(codes(&plan.issues), ["E-TYPE-MISMATCH"]);
    assert_eq!((content(&doc), doc.revision()), (before, revision));

    // Committing the plan is still one representable transaction: no bindings, `$T` kept.
    let issues = commit(&mut doc, edit).unwrap();
    assert_eq!(issues, plan.issues);
    assert_eq!(type_arg(&doc, 2), None);
    assert_eq!(signature_types(&doc, 3), ["$T"; 3]);
    assert_eq!(doc.revision(), revision + 1);
}

#[test]
fn adding_a_generic_node_with_a_pin_resolves_it_in_the_same_transaction() {
    let mut doc = document(vec![], vec![]);
    let node = add(2, json!({"type_pins": {"T": "core/u32@1"}}));
    let issues = commit(&mut doc, vec![add_node(node)]).unwrap();
    assert!(issues.is_empty());
    assert_eq!(type_arg(&doc, 2).as_deref(), Some("core/u32@1"));
    assert_eq!(signature_types(&doc, 2), ["core/u32@1"; 3]);
}
