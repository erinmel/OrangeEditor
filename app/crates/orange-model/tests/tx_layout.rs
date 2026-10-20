//! G-36: layout-only edits are flagged so no validation, compile or preview work runs.

mod common;

use common::{content, empty_document, i64_literal, node, node_id, port};
use orange_model::{BodyRef, DocCommand, LayoutCommand, TxError};
use serde_json::{Value as Json, json};

fn set(path: &[&str], value: Option<Json>) -> DocCommand {
    DocCommand::Layout(LayoutCommand::Set {
        path: path.iter().map(ToString::to_string).collect(),
        value,
    })
}

#[test]
fn layout_only_transaction_is_flagged_and_undo_removes_created_paths() {
    let mut document = empty_document();
    let _ = document
        .apply([DocCommand::AddNode {
            body: BodyRef::Root,
            node: node(1),
        }])
        .unwrap();
    let before = content(&document);
    let revision = document.revision();

    let n1 = node_id(1);
    let moved = document
        .apply([set(&["nodes", &n1], Some(json!({"x": 10, "y": 20})))])
        .unwrap();
    let change = &moved.change_set;
    assert!(change.layout_only, "G-36");
    assert!(change.nodes.is_empty() && change.edges.is_empty() && change.decls.is_empty());
    assert_eq!(
        (change.from_revision, change.to_revision),
        (revision, revision + 1)
    );
    assert_eq!(document.editor()["nodes"][&n1], json!({"x": 10, "y": 20}));

    let _ = document.revert(moved.inverse).unwrap();
    assert_eq!(
        content(&document),
        before,
        "created `nodes` object removed again"
    );
}

#[test]
fn mixed_transaction_is_not_layout_only() {
    let mut document = empty_document();
    let _ = document
        .apply([DocCommand::AddNode {
            body: BodyRef::Root,
            node: node(1),
        }])
        .unwrap();
    let committed = document
        .apply([
            set(&["viewport"], Some(json!({"zoom": 2}))),
            DocCommand::SetLiteral {
                port: port(1, "b"),
                literal: Some(i64_literal(1)),
            },
        ])
        .unwrap();
    assert!(!committed.change_set.layout_only);
}

#[test]
fn replacing_keeps_key_order_and_removal_is_undoable() {
    let mut document = empty_document();
    let _ = document
        .apply([
            set(&["nodes", "a"], Some(json!(1))),
            set(&["nodes", "b"], Some(json!(2))),
        ])
        .unwrap();
    let _ = document
        .apply([set(&["nodes", "a"], Some(json!(3)))])
        .unwrap();
    let keys: Vec<&String> = document.editor()["nodes"]
        .as_object()
        .unwrap()
        .keys()
        .collect();
    assert_eq!(keys, ["a", "b"]);

    let before = content(&document);
    let removal = document.apply([set(&["nodes", "a"], None)]).unwrap();
    assert!(
        !document.editor()["nodes"]
            .as_object()
            .unwrap()
            .contains_key("a")
    );
    let _ = document.revert(removal.inverse).unwrap();
    assert_eq!(content(&document), before, "`a` restored in first position");
}

#[test]
fn invalid_paths_are_rejected() {
    let mut document = empty_document();
    let _ = document
        .apply([set(&["viewport"], Some(json!(1)))])
        .unwrap();
    assert!(matches!(
        document.apply([set(&[], Some(json!(1)))]),
        Err(TxError::EmptyLayoutPath)
    ));
    assert!(matches!(
        document.apply([set(&["viewport", "zoom"], Some(json!(2)))]),
        Err(TxError::LayoutPathConflict { .. })
    ));
}
