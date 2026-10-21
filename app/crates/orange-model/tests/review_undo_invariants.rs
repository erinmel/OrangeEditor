//! Regressions for transaction content identity and keyed function-body integrity.

mod common;

use common::{content, empty_document, nid, node, saved, typed_edge};
use indexmap::IndexMap;
use orange_model::document::{Edge, FunctionDecl, GraphBody, NodeKind};
use orange_model::id::{FunctionId, NodeId};
use orange_model::{BodyRef, DocCommand, GraphDocument, TxError, from_json};

fn function_id() -> FunctionId {
    "50000000-0000-4000-8000-000000000001".parse().unwrap()
}

fn function(body: GraphBody) -> FunctionDecl {
    FunctionDecl {
        id: function_id(),
        name: "f".into(),
        kind: NodeKind::Pure,
        type_params: vec![],
        inputs: vec![],
        outputs: vec![],
        body,
        extensions: Default::default(),
    }
}

fn node_key(n: u8) -> NodeId {
    format!("10000000-0000-4000-8000-{n:012}").parse().unwrap()
}

fn chain() -> GraphDocument {
    let mut document = empty_document();
    let _ = document
        .apply([
            DocCommand::AddNode {
                body: BodyRef::Root,
                node: node(1),
            },
            DocCommand::AddNode {
                body: BodyRef::Root,
                node: node(2),
            },
            DocCommand::AddNode {
                body: BodyRef::Root,
                node: node(3),
            },
            DocCommand::Connect {
                edge: typed_edge(1, common::port(1, "then"), common::port(2, "in")),
            },
            DocCommand::Connect {
                edge: typed_edge(2, common::port(2, "then"), common::port(3, "in")),
            },
        ])
        .unwrap();
    document
}

#[test]
fn divergent_clones_and_reloads_reject_each_others_inverses() {
    let original = chain();
    let mut left = original.clone();
    let mut right = original;
    let left_edit = left
        .apply([DocCommand::Disconnect {
            edge: common::eid(1),
        }])
        .unwrap();
    let _right_edit = right
        .apply([DocCommand::Disconnect {
            edge: common::eid(2),
        }])
        .unwrap();
    let before = saved(&right);
    assert!(matches!(
        right.revert(left_edit.inverse.clone()),
        Err(TxError::StaleInverse { .. })
    ));
    assert_eq!(saved(&right), before);

    let mut reloaded = from_json(&saved(&left)).unwrap();
    let before = saved(&reloaded);
    assert!(matches!(
        reloaded.revert(left_edit.inverse),
        Err(TxError::StaleInverse { .. })
    ));
    assert_eq!(saved(&reloaded), before);
}

#[test]
fn equal_state_clones_can_share_undo_and_undo_all_redo_all_preserves_content() {
    let mut document = chain();
    let baseline = content(&document);
    let first = document
        .apply([DocCommand::Disconnect {
            edge: common::eid(1),
        }])
        .unwrap();
    let mut identical_clone = document.clone();
    let _ = identical_clone.revert(first.inverse.clone()).unwrap();
    assert_eq!(content(&identical_clone), baseline);
    let second = document
        .apply([DocCommand::Disconnect {
            edge: common::eid(2),
        }])
        .unwrap();
    let edited = content(&document);

    let redo_second = document.revert(second.inverse).unwrap().inverse;
    let redo_first = document.revert(first.inverse).unwrap().inverse;
    assert_eq!(content(&document), baseline);
    let _undo_first_again = document.revert(redo_first).unwrap().inverse;
    let _redo_second = document.revert(redo_second).unwrap();
    assert_eq!(content(&document), edited);
}

#[test]
fn mismatched_function_node_key_is_typed_and_atomic() {
    let mut document = empty_document();
    let mut nodes = IndexMap::new();
    nodes.insert(node_key(9), node(1));
    let malformed = function(GraphBody {
        nodes,
        edges: IndexMap::new(),
    });
    let before = saved(&document);
    let error = document
        .apply([
            DocCommand::AddNode {
                body: BodyRef::Root,
                node: node(2),
            },
            DocCommand::UpsertFunction(malformed),
        ])
        .unwrap_err();
    assert_eq!(
        error,
        TxError::FunctionNodeKeyMismatch {
            key: node_key(9),
            item: nid(1)
        }
    );
    assert_eq!(saved(&document), before);
}

#[test]
fn mismatched_function_edge_key_is_typed_and_atomic() {
    let mut document = empty_document();
    let mut nodes = IndexMap::new();
    nodes.insert(nid(1), node(1));
    nodes.insert(nid(2), node(2));
    let edge = typed_edge(1, common::port(1, "then"), common::port(2, "in"));
    let mut edges = IndexMap::<_, Edge>::new();
    edges.insert(common::eid(9), edge);
    let malformed = function(GraphBody { nodes, edges });
    let before = saved(&document);
    let error = document
        .apply([
            DocCommand::AddNode {
                body: BodyRef::Root,
                node: node(3),
            },
            DocCommand::UpsertFunction(malformed),
        ])
        .unwrap_err();
    assert_eq!(
        error,
        TxError::FunctionEdgeKeyMismatch {
            key: common::eid(9),
            item: common::eid(1)
        }
    );
    assert_eq!(saved(&document), before);
}
