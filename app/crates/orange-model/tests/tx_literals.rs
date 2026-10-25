//! G-30: a literal shadowed by an edge stays in the document; disconnecting restores it.

mod common;

use common::{content, eid, empty_document, i64_literal, node, port, typed_edge};
use orange_model::{BodyRef, DocCommand, TxError};

#[test]
fn disconnect_restores_shadowed_literal() {
    let mut document = empty_document();
    let target = port(2, "b");
    let connect_edge = || DocCommand::Connect {
        edge: typed_edge(1, port(1, "sum"), target.clone()),
    };
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
            DocCommand::SetLiteral {
                port: target.clone(),
                literal: Some(i64_literal(100)),
            },
        ])
        .unwrap();
    assert_eq!(document.active_literal(&target), Some(&i64_literal(100)));
    let unconnected = content(&document);

    let connect = document.apply([connect_edge()]).unwrap();
    assert_eq!(
        document.active_literal(&target),
        None,
        "edge shadows the literal"
    );
    let stored = &document.node(target.node).unwrap().inputs[&target.port];
    assert_eq!(stored, &i64_literal(100), "shadowed literal is kept");
    assert!(
        connect.change_set.nodes().is_empty(),
        "connecting does not touch the node"
    );
    assert_eq!(
        connect
            .change_set
            .edges()
            .added
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [eid(1)]
    );
    let connected = content(&document);

    let disconnect = document
        .apply([DocCommand::Disconnect { edge: eid(1) }])
        .unwrap();
    assert_eq!(
        document.active_literal(&target),
        Some(&i64_literal(100)),
        "G-30"
    );
    assert_eq!(content(&document), unconnected);

    // History is linear: the connect's inverse is stale until the disconnect is undone.
    assert!(matches!(
        document.revert(connect.inverse.clone()),
        Err(TxError::StaleInverse { .. })
    ));
    let _ = document.revert(disconnect.inverse).unwrap();
    assert_eq!(content(&document), connected);
}

#[test]
fn clearing_a_literal_is_undoable() {
    let mut document = empty_document();
    let target = port(1, "b");
    let _ = document
        .apply([
            DocCommand::AddNode {
                body: BodyRef::Root,
                node: node(1),
            },
            DocCommand::SetLiteral {
                port: target.clone(),
                literal: Some(i64_literal(7)),
            },
        ])
        .unwrap();
    let before = content(&document);
    let clear = document
        .apply([DocCommand::SetLiteral {
            port: target.clone(),
            literal: None,
        }])
        .unwrap();
    assert_eq!(document.active_literal(&target), None);
    assert!(clear.change_set.nodes().changed.contains(&target.node));
    let _ = document.revert(clear.inverse).unwrap();
    assert_eq!(content(&document), before);
}
