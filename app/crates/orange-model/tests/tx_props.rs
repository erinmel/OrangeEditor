//! Property: for any sequence of edits, undoing all of them restores the original content
//! and redoing all of them reproduces the edited content (FR-03 document state). A
//! rejected edit leaves the document byte-identical.

mod common;

use common::strategies::{edge_between, literal, node, port_key, uuid_v4};
use common::{content, empty_document, saved};
use orange_model::document::{NodeInstance, PortRef};
use orange_model::id::{NodeId, PortKey};
use orange_model::literal::Literal;
use orange_model::{BodyRef, DocCommand, GraphDocument, LayoutCommand};
use proptest::prelude::*;
use serde_json::json;

/// An edit described by indices, resolved against the document when it runs.
#[derive(Debug, Clone)]
enum Op {
    Add(Box<NodeInstance>),
    Remove(usize),
    Connect(uuid::Uuid, usize, usize, PortKey, PortKey),
    Disconnect(usize),
    Literal(usize, PortKey, Option<Literal>),
    InsertVariadic(usize, usize),
    Layout(usize, i64),
    /// Always invalid: exercises rollback.
    RemoveMissing(uuid::Uuid),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => node().prop_map(|node| Op::Add(Box::new(node))),
        1 => any::<usize>().prop_map(Op::Remove),
        3 => (uuid_v4(), any::<usize>(), any::<usize>(), port_key(), port_key())
            .prop_map(|(id, from, to, a, b)| Op::Connect(id, from, to, a, b)),
        1 => any::<usize>().prop_map(Op::Disconnect),
        2 => (any::<usize>(), port_key(), proptest::option::of(literal()))
            .prop_map(|(i, key, literal)| Op::Literal(i, key, literal)),
        1 => (any::<usize>(), any::<usize>()).prop_map(|(i, at)| Op::InsertVariadic(i, at)),
        1 => (any::<usize>(), any::<i64>()).prop_map(|(i, x)| Op::Layout(i, x)),
        1 => uuid_v4().prop_map(Op::RemoveMissing),
    ]
}

/// The command for `op`, or `None` when the document has nothing for it to act on.
fn command(document: &GraphDocument, op: Op) -> Option<DocCommand> {
    let nodes: Vec<NodeId> = document.root().nodes.keys().copied().collect();
    let pick = |i: usize| (!nodes.is_empty()).then(|| nodes[i % nodes.len()]);
    Some(match op {
        Op::Add(node) => DocCommand::AddNode {
            body: BodyRef::Root,
            node: *node,
        },
        Op::Remove(i) => DocCommand::RemoveNodes {
            ids: vec![pick(i)?],
        },
        Op::Connect(id, from, to, a, b) => {
            pick(0)?;
            DocCommand::Connect {
                edge: edge_between(&nodes, id, from, to, a, b),
            }
        }
        Op::Disconnect(i) => {
            let edges: Vec<_> = document.root().edges.keys().copied().collect();
            DocCommand::Disconnect {
                edge: *edges.get(i % edges.len().max(1))?,
            }
        }
        Op::Literal(i, port, literal) => DocCommand::SetLiteral {
            port: PortRef {
                node: pick(i)?,
                port,
            },
            literal,
        },
        Op::InsertVariadic(i, at) => {
            let node = pick(i)?;
            let len = document
                .node(node)?
                .variadic
                .get(&"item".parse().unwrap())
                .map_or(0, |s| s.order.len());
            DocCommand::InsertVariadic {
                node,
                port: "item".parse().unwrap(),
                at: at % (len + 1),
            }
        }
        Op::Layout(i, x) => DocCommand::Layout(LayoutCommand::Set {
            path: vec!["nodes".into(), pick(i)?.to_string()],
            value: Some(json!({"x": x})),
        }),
        Op::RemoveMissing(id) => DocCommand::RemoveNodes {
            ids: vec![NodeId::from_uuid(id).ok()?],
        },
    })
}

proptest! {
    #[test]
    fn undo_all_then_redo_all(ops in proptest::collection::vec(op(), 1..24)) {
        let mut document = empty_document();
        let original = content(&document);
        let mut inverses = Vec::new();
        for op in ops {
            let Some(command) = command(&document, op) else { continue };
            let before = saved(&document);
            match document.apply([command]) {
                Ok(committed) => inverses.push(committed.inverse),
                Err(_) => prop_assert_eq!(saved(&document), before, "rejected edit changed the document"),
            }
        }
        let edited = content(&document);

        let mut redos = Vec::new();
        while let Some(inverse) = inverses.pop() {
            redos.push(document.revert(inverse).unwrap().inverse);
        }
        prop_assert_eq!(content(&document), original);

        while let Some(redo) = redos.pop() {
            let _ = document.revert(redo).unwrap();
        }
        prop_assert_eq!(content(&document), edited);
    }
}
