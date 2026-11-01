//! Derived index queries, and the property that an incrementally updated index always
//! equals a fresh build (through edits, rollbacks, function body rescans and undo).

mod common;

use std::collections::BTreeSet;

use common::edits::{command, op};
use common::{edge_id, empty_document, nid, port, read_fixture};
use orange_model::id::EdgeId;
use orange_model::{BodyRef, DocIndex, from_json};
use proptest::prelude::*;

fn test_uuid(n: u128) -> uuid::Uuid {
    format!("00000000-0000-4000-8000-{n:012x}").parse().unwrap()
}

fn edge_with_id(
    id: EdgeId,
    from: orange_model::document::PortRef,
    to: orange_model::document::PortRef,
) -> orange_model::document::Edge {
    orange_model::document::Edge {
        id,
        from,
        to,
        extensions: Default::default(),
    }
}

fn eids(ns: &[u8]) -> BTreeSet<EdgeId> {
    ns.iter().map(|n| edge_id(*n).parse().unwrap()).collect()
}

#[test]
fn queries_on_the_saved_graph_example() {
    let document = from_json(&read_fixture("threshold-v1.json")).unwrap();
    let index = DocIndex::build(&document);
    assert_eq!(index.body_of(nid(3)), Some(BodyRef::Root));
    assert_eq!(
        index.incoming(&port(3, "a")).collect::<BTreeSet<_>>(),
        eids(&[2])
    );
    assert_eq!(
        index.outgoing(&port(2, "value")).collect::<BTreeSet<_>>(),
        eids(&[2, 6, 9])
    );
    assert_eq!(
        index.incident(nid(1)).collect::<BTreeSet<_>>(),
        eids(&[1, 3])
    );
    assert_eq!(index.incoming(&port(1, "in")).count(), 0);
    assert_eq!(index.body_of(nid(99)), None);
}

#[test]
fn function_membership_index_tracks_empty_replaced_removed_and_edited_bodies() {
    use common::{body, function, node};
    use orange_model::DocCommand;
    use orange_model::document::PortRef;
    use orange_model::id::{FunctionId, NodeId};

    let make_function = |id: FunctionId, nodes: Vec<_>, edges: Vec<_>| {
        let mut declaration = function(body(nodes, edges));
        declaration.id = id;
        declaration
    };
    let first = NodeId::from_uuid(test_uuid(10_001)).unwrap();
    let second = NodeId::from_uuid(test_uuid(10_002)).unwrap();
    let edge = EdgeId::from_uuid(test_uuid(10_003)).unwrap();
    let body_nodes = || {
        let mut a = node(1);
        a.id = first;
        let mut b = node(2);
        b.id = second;
        (a, b)
    };
    let body_edge = || {
        edge_with_id(
            edge,
            PortRef {
                node: first,
                port: "then".parse().unwrap(),
            },
            PortRef {
                node: second,
                port: "in".parse().unwrap(),
            },
        )
    };
    let assert_synced = |document: &orange_model::GraphDocument, index: &DocIndex| {
        assert!(index.is_current(document));
        assert_eq!(index, &DocIndex::build(document));
    };

    let mut document = empty_document();
    let mut index = DocIndex::build(&document);
    let (a, b) = body_nodes();
    let source_id = FunctionId::from_uuid(test_uuid(20_000)).unwrap();
    let target_id = FunctionId::from_uuid(test_uuid(30_000)).unwrap();
    let created = document
        .apply([
            DocCommand::UpsertFunction(make_function(
                source_id,
                vec![a.clone(), b.clone()],
                vec![body_edge()],
            )),
            DocCommand::UpsertFunction(make_function(target_id, vec![], vec![])),
        ])
        .unwrap();
    index.update(&document, &created.change_set);
    assert_synced(&document, &index);

    // An edit confined to a function body updates only that function's membership.
    let mut local = a.clone();
    local.id = NodeId::from_uuid(test_uuid(10_004)).unwrap();
    local.config_version += 1;
    let local_id = local.id;
    let edited = document
        .apply([DocCommand::AddNode {
            body: BodyRef::Function(target_id),
            node: local,
        }])
        .unwrap();
    index.update(&document, &edited.change_set);
    assert_synced(&document, &index);
    let removed_local = document
        .apply([DocCommand::RemoveNodes {
            ids: vec![local_id],
        }])
        .unwrap();
    index.update(&document, &removed_local.change_set);
    assert_synced(&document, &index);

    // Replacing and removing bodies, including an empty body, keeps all private
    // membership state identical to a fresh full build.
    let replacement = document
        .apply([DocCommand::UpsertFunction(make_function(
            target_id,
            vec![],
            vec![],
        ))])
        .unwrap();
    index.update(&document, &replacement.change_set);
    assert_synced(&document, &index);
    let removed = document
        .apply([DocCommand::RemoveFunction(source_id)])
        .unwrap();
    index.update(&document, &removed.change_set);
    assert_synced(&document, &index);
}

#[test]
fn moving_function_body_members_is_order_independent_and_undoable() {
    use common::{body, function, node};
    use orange_model::DocCommand;
    use orange_model::document::PortRef;
    use orange_model::id::{FunctionId, NodeId};

    let id = |value| FunctionId::from_uuid(test_uuid(value)).unwrap();
    let make_function = |function_id, nodes, edges| {
        let mut declaration = function(body(nodes, edges));
        declaration.id = function_id;
        declaration
    };
    let first = NodeId::from_uuid(test_uuid(40_001)).unwrap();
    let second = NodeId::from_uuid(test_uuid(40_002)).unwrap();
    let edge_id = EdgeId::from_uuid(test_uuid(40_003)).unwrap();
    let mut first_node = node(1);
    first_node.id = first;
    let mut second_node = node(2);
    second_node.id = second;
    let nodes = vec![first_node, second_node];
    let edge = edge_with_id(
        edge_id,
        PortRef {
            node: first,
            port: "then".parse().unwrap(),
        },
        PortRef {
            node: second,
            port: "in".parse().unwrap(),
        },
    );

    // Exercise both update iteration orders. In the reverse case, adding the
    // destination body can precede removal of the source body's recorded IDs.
    for (source, target) in [(id(50_000), id(60_000)), (id(60_000), id(50_000))] {
        let mut document = empty_document();
        let mut index = DocIndex::build(&document);
        let added = document
            .apply([
                DocCommand::UpsertFunction(make_function(
                    source,
                    nodes.clone(),
                    vec![edge.clone()],
                )),
                DocCommand::UpsertFunction(make_function(target, vec![], vec![])),
            ])
            .unwrap();
        index.update(&document, &added.change_set);
        assert_eq!(index, DocIndex::build(&document));

        let moved = document
            .apply([
                DocCommand::UpsertFunction(make_function(source, vec![], vec![])),
                DocCommand::UpsertFunction(make_function(
                    target,
                    nodes.clone(),
                    vec![edge.clone()],
                )),
            ])
            .unwrap();
        index.update(&document, &moved.change_set);
        assert_eq!(index, DocIndex::build(&document));
        assert_eq!(index.body_of(first), Some(BodyRef::Function(target)));
        assert_eq!(index.body_of(second), Some(BodyRef::Function(target)));
        assert_eq!(
            index.incident(first).collect::<BTreeSet<_>>(),
            [edge_id].into()
        );

        let undone = document.revert(moved.inverse).unwrap();
        index.update(&document, &undone.change_set);
        assert_eq!(index, DocIndex::build(&document));
        assert_eq!(index.body_of(first), Some(BodyRef::Function(source)));
        let redone = document.revert(undone.inverse).unwrap();
        index.update(&document, &redone.change_set);
        assert_eq!(index, DocIndex::build(&document));
        assert_eq!(index.body_of(first), Some(BodyRef::Function(target)));
    }
}

proptest! {
    #[test]
    fn incremental_update_equals_full_build(ops in proptest::collection::vec(op(), 1..24)) {
        let mut document = empty_document();
        let mut index = DocIndex::build(&document);
        let mut inverses = Vec::new();
        for op in ops {
            let Some(command) = command(&document, op) else { continue };
            if let Ok(committed) = document.apply([command]) {
                index.update(&document, &committed.change_set);
                prop_assert_eq!(&index, &DocIndex::build(&document));
                inverses.push(committed.inverse);
            }
        }
        while let Some(inverse) = inverses.pop() {
            let committed = document.revert(inverse).unwrap();
            index.update(&document, &committed.change_set);
            prop_assert_eq!(&index, &DocIndex::build(&document));
        }
    }
}

/// Every node ever seen is located in exactly the body that now holds it, or nowhere.
fn assert_lookup_current(
    document: &orange_model::GraphDocument,
    seen: &mut BTreeSet<orange_model::id::NodeId>,
) -> Result<(), TestCaseError> {
    let mut held = std::collections::BTreeMap::new();
    for (body, nodes) in document.bodies_with_refs() {
        for id in nodes.nodes.keys() {
            held.insert(*id, body);
            seen.insert(*id);
        }
    }
    for id in seen.iter() {
        prop_assert_eq!(document.locate_node(*id), held.get(id).copied());
        prop_assert_eq!(document.node(*id).is_some(), held.contains_key(id));
    }
    Ok(())
}

proptest! {
    #[test]
    fn node_lookup_follows_edits_and_undo(ops in proptest::collection::vec(op(), 1..24)) {
        let mut document = empty_document();
        let mut seen = BTreeSet::new();
        assert_lookup_current(&document, &mut seen)?;
        let mut inverses = Vec::new();
        for op in ops {
            let Some(command) = command(&document, op) else { continue };
            if let Ok(committed) = document.apply([command]) {
                assert_lookup_current(&document, &mut seen)?;
                inverses.push(committed.inverse);
            }
        }
        while let Some(inverse) = inverses.pop() {
            let _ = document.revert(inverse).unwrap();
            assert_lookup_current(&document, &mut seen)?;
        }
    }
}
