//! G-31: variadic instance ports keep stable `port#k` keys; counters are never reused.

mod common;

use common::{content, eid, empty_document, i64_literal, nid, node, port, saved, typed_edge};
use orange_model::document::VariadicState;
use orange_model::id::{PortId, PortKey};
use orange_model::{BodyRef, DocCommand, GraphDocument, TxError};

fn item() -> PortId {
    "item".parse().unwrap()
}

fn key(text: &str) -> PortKey {
    text.parse().unwrap()
}

fn insert(document: &mut GraphDocument, at: usize) -> Result<(), TxError> {
    document
        .apply([DocCommand::InsertVariadic {
            node: nid(1),
            port: item(),
            at,
        }])
        .map(|_| ())
}

fn remove(document: &mut GraphDocument, port: &str) -> Result<orange_model::Committed, TxError> {
    document.apply([DocCommand::RemoveVariadic {
        node: nid(1),
        port: key(port),
    }])
}

/// `item` instance order and next counter on node 1.
fn state(document: &GraphDocument) -> (Vec<u16>, u16) {
    let state = &document.node(nid(1)).unwrap().variadic[&item()];
    (state.order.clone(), state.next)
}

/// Node 1 with `item#0`, `item#1`, each fed by an edge from node 2.
fn two_items() -> GraphDocument {
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
            DocCommand::InsertVariadic {
                node: nid(1),
                port: item(),
                at: 0,
            },
            DocCommand::InsertVariadic {
                node: nid(1),
                port: item(),
                at: 1,
            },
            DocCommand::Connect {
                edge: typed_edge(1, port(2, "a"), port(1, "item#0")),
            },
            DocCommand::Connect {
                edge: typed_edge(2, port(2, "b"), port(1, "item#1")),
            },
        ])
        .unwrap();
    document
}

#[test]
fn insert_in_the_middle_keeps_existing_keys_and_edges() {
    let mut document = two_items();
    let edges_before = document.root().edges.clone();
    insert(&mut document, 1).unwrap();
    assert_eq!(
        state(&document),
        (vec![0, 2, 1], 3),
        "new item#2 placed second"
    );
    assert_eq!(document.root().edges, edges_before, "G-31: edges unchanged");
}

#[test]
fn counters_are_never_reused() {
    let mut document = two_items();
    let _ = remove(&mut document, "item#1").unwrap();
    assert_eq!(state(&document), (vec![0], 2));
    insert(&mut document, 1).unwrap();
    assert_eq!(state(&document), (vec![0, 2], 3), "k=1 is not reused");
}

#[test]
fn remove_takes_edges_and_literal_and_undo_restores_them() {
    let mut document = two_items();
    let _ = document
        .apply([DocCommand::SetLiteral {
            port: port(1, "item#0"),
            literal: Some(i64_literal(5)),
        }])
        .unwrap();
    let before = content(&document);

    let removal = remove(&mut document, "item#0").unwrap();
    assert_eq!(state(&document), (vec![1], 2));
    assert!(document.edge(eid(1)).is_none(), "edge to item#0 removed");
    assert!(document.edge(eid(2)).is_some());
    assert!(
        !document
            .node(nid(1))
            .unwrap()
            .inputs
            .contains_key(&key("item#0"))
    );
    assert!(removal.change_set.edges().removed.contains(&eid(1)));

    let _ = document.revert(removal.inverse).unwrap();
    assert_eq!(
        content(&document),
        before,
        "edge order, literal and variadic order restored"
    );
}

#[test]
fn invalid_variadic_edits_are_rejected() {
    let mut document = two_items();
    let before = saved(&document);
    assert!(matches!(
        insert(&mut document, 3),
        Err(TxError::VariadicPositionOutOfRange { at: 3, len: 2, .. })
    ));
    for port in ["item", "item#9", "other#0"] {
        assert!(
            matches!(
                remove(&mut document, port),
                Err(TxError::VariadicPortNotFound { .. })
            ),
            "{port}"
        );
    }
    assert_eq!(saved(&document), before, "rejected edits change nothing");
}

#[test]
fn exhausted_counter_fails_instead_of_wrapping() {
    let mut document = empty_document();
    let mut full = node(1);
    full.variadic.insert(
        item(),
        VariadicState {
            order: vec![],
            next: u16::MAX,
        },
    );
    let _ = document
        .apply([DocCommand::AddNode {
            body: BodyRef::Root,
            node: full,
        }])
        .unwrap();
    let before = saved(&document);
    assert!(matches!(
        insert(&mut document, 0),
        Err(TxError::VariadicExhausted { .. })
    ));
    assert_eq!(saved(&document), before);
}
