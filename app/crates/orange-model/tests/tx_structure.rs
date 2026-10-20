//! Structural transaction rules: atomicity, ID uniqueness, body membership, ordered undo,
//! stale inverses and change-set deltas.

mod common;

use common::{content, eid, empty_document, nid, node, port, saved, typed_edge};
use orange_model::document::{
    Edge, FunctionDecl, GraphBody, NodeKind, ProviderBinding, VarScope, VariableDecl,
};
use orange_model::id::{FunctionId, VariableId};
use orange_model::{BodyRef, DeclId, DocCommand, GraphDocument, TxError, keyed};

fn fid() -> FunctionId {
    "50000000-0000-4000-8000-000000000001".parse().unwrap()
}

fn vid() -> VariableId {
    "30000000-0000-4000-8000-000000000001".parse().unwrap()
}

fn add(n: u8) -> DocCommand {
    DocCommand::AddNode {
        body: BodyRef::Root,
        node: node(n),
    }
}

fn connect(n: u8, from: u8, to: u8) -> DocCommand {
    DocCommand::Connect {
        edge: typed_edge(n, port(from, "then"), port(to, "in")),
    }
}

fn function(nodes: Vec<u8>, edges: Vec<Edge>) -> FunctionDecl {
    FunctionDecl {
        id: fid(),
        name: "f".into(),
        kind: NodeKind::Pure,
        type_params: vec![],
        inputs: vec![],
        outputs: vec![],
        body: GraphBody {
            nodes: keyed::collect(nodes.into_iter().map(node)).unwrap(),
            edges: keyed::collect(edges).unwrap(),
        },
        extensions: Default::default(),
    }
}

fn variable(name: &str) -> VariableDecl {
    VariableDecl {
        id: vid(),
        name: name.into(),
        ty: "core/i64@1".parse().unwrap(),
        scope: VarScope::Run,
        init: None,
        persistent: false,
        conflict: Default::default(),
        extensions: Default::default(),
    }
}

/// Root nodes 1–3 chained by edges 1 (1→2) and 2 (2→3).
fn chain() -> GraphDocument {
    let mut document = empty_document();
    let _ = document
        .apply([add(1), add(2), add(3), connect(1, 1, 2), connect(2, 2, 3)])
        .unwrap();
    document
}

/// Asserts the command fails with `expected` and leaves the document byte-identical.
fn assert_rejected(document: &mut GraphDocument, commands: Vec<DocCommand>, expected: TxError) {
    let before = saved(document);
    assert_eq!(document.apply(commands).unwrap_err(), expected);
    assert_eq!(saved(document), before, "document and revision unchanged");
}

#[test]
fn empty_transaction_is_rejected() {
    assert_rejected(&mut empty_document(), vec![], TxError::Empty);
}

#[test]
fn failure_rolls_back_earlier_commands() {
    let mut document = chain();
    // The first two commands succeed before the third fails.
    assert_rejected(
        &mut document,
        vec![add(4), connect(3, 3, 4), connect(4, 4, 9)],
        TxError::EdgeEndpointMissing {
            edge: eid(4),
            node: nid(9),
        },
    );
}

#[test]
fn ids_must_be_unique_and_targets_must_exist() {
    let mut document = chain();
    assert_rejected(&mut document, vec![add(1)], TxError::NodeExists(nid(1)));
    assert_rejected(
        &mut document,
        vec![connect(1, 1, 3)],
        TxError::EdgeExists(eid(1)),
    );
    assert_rejected(
        &mut document,
        vec![DocCommand::Disconnect { edge: eid(9) }],
        TxError::EdgeNotFound(eid(9)),
    );
    assert_rejected(
        &mut document,
        vec![DocCommand::RemoveNodes { ids: vec![nid(9)] }],
        TxError::NodeNotFound(nid(9)),
    );
    assert_rejected(
        &mut document,
        vec![DocCommand::RemoveVariable(vid())],
        TxError::VariableNotFound(vid()),
    );
    assert_rejected(
        &mut document,
        vec![DocCommand::RemoveFunction(fid())],
        TxError::FunctionNotFound(fid()),
    );
    assert_rejected(
        &mut document,
        vec![DocCommand::AddNode {
            body: BodyRef::Function(fid()),
            node: node(7),
        }],
        TxError::FunctionNotFound(fid()),
    );
}

#[test]
fn edges_stay_inside_one_body() {
    let mut document = chain();
    let _ = document
        .apply([DocCommand::UpsertFunction(function(vec![5, 6], vec![]))])
        .unwrap();
    assert_eq!(document.locate_node(nid(5)), Some(BodyRef::Function(fid())));
    assert_rejected(
        &mut document,
        vec![connect(3, 1, 5)],
        TxError::CrossBodyEdge(eid(3)),
    );

    let _ = document.apply([connect(3, 5, 6)]).unwrap();
    assert_eq!(document.locate_edge(eid(3)), Some(BodyRef::Function(fid())));
}

#[test]
fn function_bodies_obey_the_same_rules() {
    let mut document = chain();
    assert_rejected(
        &mut document,
        vec![DocCommand::UpsertFunction(function(vec![1], vec![]))],
        TxError::NodeExists(nid(1)),
    );
    let outward = typed_edge(9, port(5, "then"), port(1, "in"));
    assert_rejected(
        &mut document,
        vec![DocCommand::UpsertFunction(function(vec![5], vec![outward]))],
        TxError::EdgeEndpointMissing {
            edge: eid(9),
            node: nid(1),
        },
    );
}

#[test]
fn remove_nodes_takes_incident_edges_and_undo_restores_order() {
    let mut document = chain();
    let before = content(&document);

    let removal = document
        .apply([DocCommand::RemoveNodes { ids: vec![nid(2)] }])
        .unwrap();
    assert_eq!(document.root().edges.len(), 0, "both edges touched node 2");
    let change = &removal.change_set;
    assert_eq!(
        change.nodes.removed.iter().copied().collect::<Vec<_>>(),
        [nid(2)]
    );
    assert_eq!(change.edges.removed.len(), 2);
    assert!(!change.layout_only);

    let _ = document.revert(removal.inverse).unwrap();
    assert_eq!(
        content(&document),
        before,
        "node 2 back between 1 and 3, edges in order"
    );
}

#[test]
fn revision_counts_every_committed_transaction_and_redo_works() {
    let mut document = chain();
    let start = document.revision();
    let edited = document
        .apply([DocCommand::Disconnect { edge: eid(1) }])
        .unwrap();
    let after_edit = content(&document);
    assert_eq!(edited.change_set.to_revision, start + 1);

    let undo = document.revert(edited.inverse).unwrap();
    assert_eq!(document.revision(), start + 2, "undo is a transaction too");
    let redo = document.revert(undo.inverse).unwrap();
    assert_eq!(content(&document), after_edit);
    assert_eq!(
        redo.change_set
            .edges
            .removed
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [eid(1)]
    );
}

#[test]
fn redo_stays_valid_after_several_undos() {
    // Regression: binding inverses to the revision broke the second redo, because each
    // undo and redo advances the revision.
    let mut document = chain();
    let first = document
        .apply([DocCommand::Disconnect { edge: eid(1) }])
        .unwrap();
    let second = document
        .apply([DocCommand::Disconnect { edge: eid(2) }])
        .unwrap();
    let edited = content(&document);

    let redo_second = document.revert(second.inverse).unwrap().inverse;
    let redo_first = document.revert(first.inverse).unwrap().inverse;
    let _ = document.revert(redo_first).unwrap();
    let _ = document.revert(redo_second).unwrap();
    assert_eq!(content(&document), edited);
}

#[test]
fn stale_inverse_is_rejected() {
    let mut document = chain();
    let first = document
        .apply([DocCommand::Disconnect { edge: eid(1) }])
        .unwrap();
    let _ = document
        .apply([DocCommand::Disconnect { edge: eid(2) }])
        .unwrap();
    let before = saved(&document);
    assert_eq!(
        document.revert(first.inverse.clone()).unwrap_err(),
        TxError::StaleInverse {
            expected: first.inverse.applies_to_state(),
            actual: document.state_id()
        }
    );
    assert_eq!(saved(&document), before);
}

#[test]
fn upsert_replaces_in_place_and_reports_decl_deltas() {
    let mut document = empty_document();
    let created = document
        .apply([DocCommand::UpsertVariable(variable("total"))])
        .unwrap();
    assert!(
        created
            .change_set
            .decls
            .added
            .contains(&DeclId::Variable(vid()))
    );

    let renamed = document
        .apply([DocCommand::UpsertVariable(variable("sum"))])
        .unwrap();
    assert!(
        renamed
            .change_set
            .decls
            .changed
            .contains(&DeclId::Variable(vid()))
    );
    assert_eq!(document.variables()[&vid()].name, "sum");
}

#[test]
fn net_deltas_ignore_ids_added_and_removed_together() {
    let mut document = chain();
    let committed = document
        .apply([
            add(4),
            connect(3, 3, 4),
            DocCommand::RemoveNodes { ids: vec![nid(4)] },
        ])
        .unwrap();
    assert!(committed.change_set.nodes.is_empty());
    assert!(committed.change_set.edges.is_empty());
}

#[test]
fn rebind_changes_only_the_provider() {
    let mut document = chain();
    let provider = ProviderBinding {
        package: "org.orange.math".into(),
        version: "0.2.0".into(),
        id: "native-checked".into(),
        extensions: Default::default(),
    };
    let committed = document
        .apply([DocCommand::Rebind {
            nodes: vec![nid(1), nid(3)],
            provider: provider.clone(),
        }])
        .unwrap();
    assert_eq!(
        document.node(nid(1)).unwrap().provider.as_ref(),
        Some(&provider)
    );
    assert_eq!(committed.change_set.nodes.changed.len(), 2);
    assert!(committed.change_set.edges.is_empty());
}

#[test]
fn put_signature_is_idempotent_rejects_conflicts_and_undoes() {
    use orange_model::document::SignatureSnapshot;
    let snapshot = |ty: &str| -> SignatureSnapshot {
        serde_json::from_value(serde_json::json!({"kind": "pure", "ports": [
            {"port_id": "a", "direction": "input", "kind": "data", "type": ty}
        ]}))
        .unwrap()
    };
    let key: orange_model::id::SignatureKey = "s-a".parse().unwrap();
    let put = |ty: &str| DocCommand::PutSignature {
        key: key.clone(),
        snapshot: snapshot(ty),
    };

    let mut document = empty_document();
    let before = content(&document);
    let first = document.apply([put("core/i64@1")]).unwrap();
    let again = document.apply([put("core/i64@1")]).unwrap();
    assert!(again.change_set.nodes.is_empty() && document.signatures().len() == 1);
    assert_rejected(
        &mut document,
        vec![put("core/f64@1")],
        TxError::SignatureConflict(key.clone()),
    );

    let _ = document.revert(again.inverse).unwrap();
    let _ = document.revert(first.inverse).unwrap();
    assert_eq!(content(&document), before);
}
