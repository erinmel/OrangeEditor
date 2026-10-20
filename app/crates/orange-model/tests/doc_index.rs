//! Derived index queries, and the property that an incrementally updated index always
//! equals a fresh build (through edits, rollbacks, function body rescans and undo).

mod common;

use std::collections::BTreeSet;

use common::edits::{command, op};
use common::{edge_id, empty_document, node_id, port, read_fixture};
use orange_model::id::{EdgeId, NodeId};
use orange_model::{BodyRef, DocIndex, from_json};
use proptest::prelude::*;

fn nid(n: u8) -> NodeId {
    node_id(n).parse().unwrap()
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
