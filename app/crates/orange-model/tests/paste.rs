//! FR-02: copy/paste keeps internal edges and remaps every ID and reference without
//! aliasing the originals.

mod common;

use std::collections::{BTreeSet, HashSet};

use common::strategies::document;
use common::{
    body, content, empty_document, function, function_id, id_source, nid, node, port, read_fixture,
    typed_edge,
};
use orange_model::id::{EdgeId, NodeId, VariableId};
use orange_model::paste::Fragment;
use orange_model::{BodyRef, DocCommand, GraphDocument, PasteError, from_json};
use proptest::prelude::*;
use serde_json::json;

fn fixture() -> GraphDocument {
    from_json(&read_fixture("threshold-v1.json")).unwrap()
}

fn total_variable() -> VariableId {
    "30000000-0000-4000-8000-000000000001".parse().unwrap()
}

fn all_ids(document: &GraphDocument) -> (HashSet<NodeId>, HashSet<EdgeId>) {
    let nodes = document
        .bodies()
        .flat_map(|body| body.nodes.keys().copied())
        .collect();
    let edges = document
        .bodies()
        .flat_map(|body| body.edges.keys().copied())
        .collect();
    (nodes, edges)
}

#[test]
fn copy_keeps_internal_edges_and_captures_references() {
    let document = fixture();
    // Get (n2) → Add (n3) → Set (n4); the trigger and the branch stay behind.
    let fragment = Fragment::copy(&document, &[nid(2), nid(3), nid(4)]).unwrap();
    assert_eq!(fragment.nodes.len(), 3);
    let edges: BTreeSet<String> = fragment
        .edges
        .iter()
        .map(|edge| edge.id.to_string())
        .collect();
    assert_eq!(
        edges,
        BTreeSet::from([common::edge_id(2), common::edge_id(4)])
    );
    assert_eq!(
        fragment.variables.len(),
        1,
        "`total` is referenced by Get and Set"
    );
    assert_eq!(
        fragment.signatures.len(),
        1,
        "only Add has a stored signature"
    );
}

#[test]
fn same_document_paste_reuses_declarations_and_never_aliases() {
    let mut document = fixture();
    let before = content(&document);
    let (old_nodes, old_edges) = all_ids(&document);
    let fragment = Fragment::copy(&document, &[nid(2), nid(3), nid(4)]).unwrap();
    let paste = fragment
        .paste(&document, BodyRef::Root, id_source(1))
        .unwrap();
    let committed = document.apply(paste.commands).unwrap();

    assert_eq!(document.variables().len(), 1, "same variable reused");
    let pasted_get = document.node(paste.nodes[&nid(2)]).unwrap();
    assert_eq!(
        pasted_get.config["variable"],
        json!(total_variable().to_string())
    );
    let (new_nodes, new_edges) = all_ids(&document);
    assert_eq!(new_nodes.len(), old_nodes.len() + 3);
    assert_eq!(new_edges.len(), old_edges.len() + 2);
    assert!(
        paste.nodes.values().all(|id| !old_nodes.contains(id)),
        "no aliasing"
    );

    // Pasted edges connect the pasted counterparts with the original ports.
    let pasted_add = paste.nodes[&nid(3)];
    let inbound: Vec<_> = document
        .root()
        .edges
        .values()
        .filter(|edge| edge.to.node == pasted_add)
        .collect();
    assert_eq!(inbound.len(), 1);
    assert_eq!(inbound[0].from.node, paste.nodes[&nid(2)]);
    assert_eq!(inbound[0].to.port.to_string(), "a");

    // The signature is re-keyed by content and the node points at it.
    let key = document
        .node(pasted_add)
        .unwrap()
        .signature
        .clone()
        .unwrap();
    assert_eq!(
        key.as_str(),
        include_str!("fixtures/s-add-i64.sha256").trim()
    );
    assert!(document.signatures().contains_key(&key));

    let _ = document.revert(committed.inverse).unwrap();
    assert_eq!(content(&document), before);
}

#[test]
fn paste_into_another_document_copies_declarations_under_new_ids() {
    let source = fixture();
    let fragment = Fragment::copy(&source, &[nid(2), nid(4)]).unwrap();
    let mut target = empty_document();
    let paste = fragment
        .paste(&target, BodyRef::Root, id_source(2))
        .unwrap();
    let _ = target.apply(paste.commands).unwrap();

    assert_eq!(target.variables().len(), 1);
    let (new_id, decl) = target.variables().iter().next().unwrap();
    assert_ne!(
        *new_id,
        total_variable(),
        "pasted declaration gets a new ID"
    );
    assert_eq!(decl.name, "total");
    for old in [nid(2), nid(4)] {
        let pasted = target.node(paste.nodes[&old]).unwrap();
        assert_eq!(
            pasted.config["variable"],
            json!(new_id.to_string()),
            "reference rewritten"
        );
    }
}

/// Root node 1 calls function f, whose body is nodes 5 → 6.
fn with_function() -> GraphDocument {
    let mut document = empty_document();
    let mut call = node(1);
    call.config
        .insert("function".into(), json!(function_id().to_string()));
    let edge = typed_edge(5, port(5, "then"), port(6, "in"));
    let function = function(body([node(5), node(6)], [edge]));
    let _ = document
        .apply([
            DocCommand::UpsertFunction(function),
            DocCommand::AddNode {
                body: BodyRef::Root,
                node: call,
            },
        ])
        .unwrap();
    document
}

#[test]
fn referenced_functions_travel_with_remapped_bodies() {
    let source = with_function();
    let fragment = Fragment::copy(&source, &[nid(1)]).unwrap();
    assert_eq!(fragment.functions.len(), 1);

    let mut target = empty_document();
    let paste = fragment
        .paste(&target, BodyRef::Root, id_source(3))
        .unwrap();
    let _ = target.apply(paste.commands).unwrap();
    let (new_fid, function) = target.functions().iter().next().unwrap();
    assert_ne!(*new_fid, function_id());
    assert!(
        function
            .body
            .nodes
            .keys()
            .all(|id| *id != nid(5) && *id != nid(6)),
        "body IDs remapped"
    );
    let edge = function.body.edges.values().next().unwrap();
    assert!(
        function.body.nodes.contains_key(&edge.from.node)
            && function.body.nodes.contains_key(&edge.to.node)
    );
    let call = target.node(paste.nodes[&nid(1)]).unwrap();
    assert_eq!(call.config["function"], json!(new_fid.to_string()));

    // Pasting into the source reuses the existing function.
    let mut same = source.clone();
    let paste = fragment.paste(&same, BodyRef::Root, id_source(4)).unwrap();
    let _ = same.apply(paste.commands).unwrap();
    assert_eq!(same.functions().len(), 1);
}

#[test]
fn invalid_selections_are_rejected() {
    let document = with_function();
    assert_eq!(
        Fragment::copy(&document, &[nid(1), nid(5)]),
        Err(PasteError::SelectionSpansBodies)
    );
    assert_eq!(
        Fragment::copy(&document, &[nid(9)]),
        Err(PasteError::NodeNotFound(nid(9)))
    );
    assert_eq!(Fragment::copy(&document, &[]), Ok(Fragment::default()));
}

proptest! {
    #[test]
    fn paste_never_aliases_and_preserves_internal_edges(
        mut document in document(),
        picks in proptest::collection::vec(any::<bool>(), 8),
    ) {
        let selection: Vec<NodeId> = document
            .root()
            .nodes
            .keys()
            .zip(picks)
            .filter_map(|(id, pick)| pick.then_some(*id))
            .collect();
        let before = content(&document);
        let (old_nodes, old_edges) = all_ids(&document);
        let fragment = Fragment::copy(&document, &selection).unwrap();
        let paste = fragment.paste(&document, BodyRef::Root, id_source(9)).unwrap();
        if paste.commands.is_empty() {
            return Ok(());
        }
        let committed = document.apply(paste.commands).unwrap();

        let (new_nodes, new_edges) = all_ids(&document);
        prop_assert_eq!(new_nodes.len(), old_nodes.len() + selection.len());
        prop_assert_eq!(new_edges.len(), old_edges.len() + fragment.edges.len());
        prop_assert!(paste.nodes.values().all(|id| !old_nodes.contains(id)));
        for edge in &fragment.edges {
            let (from, to) = (paste.nodes[&edge.from.node], paste.nodes[&edge.to.node]);
            let matched = document.root().edges.values().any(|pasted| {
                pasted.from.node == from && pasted.to.node == to
                    && pasted.from.port == edge.from.port && pasted.to.port == edge.to.port
                    && !old_edges.contains(&pasted.id)
            });
            prop_assert!(matched, "internal edge {} not reproduced", edge.id);
        }
        let _ = document.revert(committed.inverse).unwrap();
        prop_assert_eq!(content(&document), before);
    }
}
