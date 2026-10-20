//! Proptest strategies shared by the property tests.

use std::collections::BTreeMap;

use orange_model::document::{
    ActivationPolicy, DefinitionRef, Edge, FaultPolicy, GraphDocument, NodeInstance,
    OverflowPolicy, PortRef, ProviderBinding, ReentrancyMode,
};
use orange_model::id::{EdgeId, GraphId, NodeId, PortId, PortKey, TypeExpr};
use orange_model::literal::{Literal, LiteralPayload};
use orange_model::{BodyRef, DocCommand};
use proptest::prelude::*;
use serde_json::{Value as Json, json};

pub fn uuid_v4() -> impl Strategy<Value = uuid::Uuid> {
    any::<[u8; 16]>().prop_map(|bytes| uuid::Builder::from_random_bytes(bytes).into_uuid())
}

pub fn port_key() -> impl Strategy<Value = PortKey> {
    ("[a-z][a-z0-9_]{0,12}", proptest::option::of(any::<u16>()))
        .prop_map(|(port, k)| PortKey::new(port.parse::<PortId>().unwrap(), k))
}

pub fn type_expr() -> impl Strategy<Value = TypeExpr> {
    proptest::sample::select(vec![
        "core/i64@1",
        "core/bool@1",
        "core/string@1",
        "core/list@1<core/u32@1>",
    ])
    .prop_map(|t| t.parse().unwrap())
}

/// JSON payloads without floats (their text form is not this slice's concern) or null.
pub fn json_payload() -> impl Strategy<Value = Json> {
    prop_oneof![
        any::<bool>().prop_map(Json::from),
        any::<i64>().prop_map(|n| Json::from(n.to_string())),
        ".{0,16}".prop_map(Json::from),
        proptest::collection::vec(any::<u32>(), 0..4).prop_map(|items| json!(items)),
    ]
}

pub fn literal() -> impl Strategy<Value = Literal> {
    let payload = prop_oneof![
        json_payload().prop_map(LiteralPayload::Value),
        any::<u64>().prop_map(|bits| LiteralPayload::Bits(format!("{bits:016x}"))),
    ];
    (type_expr(), payload).prop_map(|(ty, payload)| Literal {
        ty,
        payload,
        display: None,
    })
}

pub fn activation() -> impl Strategy<Value = ActivationPolicy> {
    (any::<u32>(), any::<u32>()).prop_map(|(capacity, order)| ActivationPolicy {
        mode: ReentrancyMode::Queue,
        max_parallel: None,
        capacity,
        overflow: OverflowPolicy::DropOldest,
        on_fault: FaultPolicy::StopRun,
        order,
        extensions: Default::default(),
    })
}

pub fn node() -> impl Strategy<Value = NodeInstance> {
    (
        uuid_v4(),
        proptest::sample::select(vec![
            "core/branch",
            "org.orange.math/add",
            "org.example.x/y",
        ]),
        1..5u32,
        any::<u32>(),
        proptest::collection::btree_map("[A-Z]", type_expr(), 0..3),
        proptest::option::of("[a-z-]{1,12}"),
        proptest::collection::btree_map(port_key(), literal(), 0..4),
        proptest::option::of(activation()),
    )
        .prop_map(
            |(id, definition, version, config_version, type_args, provider, inputs, activation)| {
                NodeInstance {
                    id: NodeId::from_uuid(id).unwrap(),
                    definition: DefinitionRef {
                        id: definition.parse().unwrap(),
                        version,
                    },
                    config_version,
                    config: json!({"level": "info"}).as_object().unwrap().clone(),
                    type_args,
                    provider: provider.map(|id| ProviderBinding {
                        package: "org.orange.math".into(),
                        version: "0.1.0".into(),
                        id,
                        extensions: Default::default(),
                    }),
                    signature: None,
                    inputs,
                    variadic: BTreeMap::new(),
                    activation,
                    extensions: Default::default(),
                }
            },
        )
}

/// An edge between two of `nodes`, chosen by index.
pub fn edge_between(
    nodes: &[NodeId],
    id: uuid::Uuid,
    from: usize,
    to: usize,
    from_port: PortKey,
    to_port: PortKey,
) -> Edge {
    Edge {
        id: EdgeId::from_uuid(id).unwrap(),
        from: PortRef {
            node: nodes[from % nodes.len()],
            port: from_port,
        },
        to: PortRef {
            node: nodes[to % nodes.len()],
            port: to_port,
        },
        extensions: Default::default(),
    }
}

/// A document built through one transaction of `AddNode` and `Connect` commands.
pub fn document() -> impl Strategy<Value = GraphDocument> {
    let edge = (
        uuid_v4(),
        any::<usize>(),
        any::<usize>(),
        port_key(),
        port_key(),
    );
    (
        uuid_v4(),
        proptest::collection::vec(node(), 0..8),
        proptest::collection::vec(edge, 0..8),
    )
        .prop_filter_map("random UUIDs collided", |(graph, nodes, edges)| {
            let mut document = GraphDocument::new(GraphId::from_uuid(graph).unwrap());
            let ids: Vec<NodeId> = nodes.iter().map(|node| node.id).collect();
            let mut commands: Vec<DocCommand> = nodes
                .into_iter()
                .map(|node| DocCommand::AddNode {
                    body: BodyRef::Root,
                    node,
                })
                .collect();
            if !ids.is_empty() {
                commands.extend(edges.into_iter().map(|(id, from, to, from_port, to_port)| {
                    DocCommand::Connect {
                        edge: edge_between(&ids, id, from, to, from_port, to_port),
                    }
                }));
            }
            if !commands.is_empty() {
                let _committed = document.apply(commands).ok()?;
            }
            Some(document)
        })
}
