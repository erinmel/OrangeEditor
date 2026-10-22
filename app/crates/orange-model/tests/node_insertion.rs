//! Structural authoring primitive: splice one node into an established connection.
//! Type/port compatibility remains the planner/validator's responsibility.
mod common;

use common::{add, connect, eid, empty_document, nid};
use orange_model::{DocCommand, TxError};

#[test]
fn insert_between_connected_nodes_is_one_undoable_transaction() {
    let mut doc = empty_document();
    let _ = doc.apply([add(1), add(2), connect(1, 1, 2)]).unwrap();
    let original = doc.clone();
    let inserted = doc
        .apply([
            DocCommand::Disconnect { edge: eid(1) },
            add(3),
            connect(2, 1, 3),
            connect(3, 3, 2),
        ])
        .unwrap();
    assert!(doc.edge(eid(1)).is_none());
    assert_eq!(doc.edge(eid(2)).unwrap().to.node, nid(3));
    assert_eq!(doc.edge(eid(3)).unwrap().to.node, nid(2));
    assert_eq!(doc.revision(), original.revision() + 1);
    let redo = doc.revert(inserted.inverse).unwrap();
    assert!(doc.node(nid(3)).is_none());
    assert!(doc.edge(eid(1)).is_some());
    let _ = doc.revert(redo.inverse).unwrap();
    assert!(doc.node(nid(3)).is_some());
    assert!(doc.edge(eid(1)).is_none());
}

#[test]
fn invalid_splice_restores_original_wire_and_document_state() {
    let mut doc = empty_document();
    let _ = doc.apply([add(1), add(2), connect(1, 1, 2)]).unwrap();
    let original = doc.clone();
    let result = doc.apply([
        DocCommand::Disconnect { edge: eid(1) },
        add(3),
        connect(2, 1, 3),
        connect(3, 3, 99),
    ]);
    assert!(matches!(result, Err(TxError::EdgeEndpointMissing { .. })));
    assert_eq!(doc, original);
    assert_eq!(doc.state_id(), original.state_id());
}
