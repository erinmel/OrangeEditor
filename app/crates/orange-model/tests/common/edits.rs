//! Random edit sequences for property tests: each `Op` is resolved against the current
//! document, so generated edits mostly succeed and occasionally exercise rollback.

use orange_model::document::literal::Literal;
use orange_model::document::{NodeInstance, PortRef};
use orange_model::id::{NodeId, PortKey};
use orange_model::{BodyRef, DocCommand, GraphDocument, LayoutCommand};
use proptest::prelude::*;
use serde_json::json;

use super::strategies::{edge_between, literal, node, port_key, uuid_v4};
use super::{body, function, function_id};

#[derive(Debug, Clone)]
pub enum Op {
    Add(Box<NodeInstance>),
    Remove(usize),
    Connect(uuid::Uuid, usize, usize, PortKey, PortKey),
    Disconnect(usize),
    Literal(usize, PortKey, Option<Literal>),
    InsertVariadic(usize, usize),
    Layout(usize, i64),
    /// Creates or replaces the function, its body nodes chained by one edge.
    Function(Vec<NodeInstance>, uuid::Uuid),
    RemoveFunction,
    /// Always invalid: exercises rollback.
    RemoveMissing(uuid::Uuid),
}

pub fn op() -> impl Strategy<Value = Op> {
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
        1 => (proptest::collection::vec(node(), 0..3), uuid_v4()).prop_map(|(nodes, edge)| Op::Function(nodes, edge)),
        1 => Just(Op::RemoveFunction),
        1 => uuid_v4().prop_map(Op::RemoveMissing),
    ]
}

/// The command for `op`, or `None` when the document has nothing for it to act on.
pub fn command(document: &GraphDocument, op: Op) -> Option<DocCommand> {
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
            let item = "item".parse().unwrap();
            let len = document
                .node(node)?
                .variadic
                .get(&item)
                .map_or(0, |s| s.order.len());
            DocCommand::InsertVariadic {
                node,
                port: item,
                at: at % (len + 1),
            }
        }
        Op::Layout(i, x) => DocCommand::Layout(LayoutCommand::Set {
            path: vec!["nodes".into(), pick(i)?.to_string()],
            value: Some(json!({"x": x})),
        }),
        Op::Function(body_nodes, edge) => {
            let ids: Vec<NodeId> = body_nodes.iter().map(|node| node.id).collect();
            let edges = (ids.len() >= 2).then(|| {
                edge_between(
                    &ids,
                    edge,
                    0,
                    1,
                    "then".parse().unwrap(),
                    "in".parse().unwrap(),
                )
            });
            DocCommand::UpsertFunction(function(body(body_nodes, edges)))
        }
        Op::RemoveFunction => DocCommand::RemoveFunction(function_id()),
        Op::RemoveMissing(id) => DocCommand::RemoveNodes {
            ids: vec![NodeId::from_uuid(id).ok()?],
        },
    })
}
