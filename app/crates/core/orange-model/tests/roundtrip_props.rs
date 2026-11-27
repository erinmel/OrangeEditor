//! Property: any structurally valid document survives save→load unchanged and saves to
//! the same bytes again.

use crate::behaviour::common;

use crate::{from_json, to_json};
use common::{branch_node, edge, minimal_document, strategies::document};
use proptest::prelude::*;
use serde_json::json;

proptest! {
    #[test]
    fn save_load_round_trip(document in document()) {
        let saved = to_json(&document).unwrap();
        let loaded = from_json(&saved).unwrap();
        prop_assert_eq!(&loaded, &document);
        prop_assert_eq!(to_json(&loaded).unwrap(), saved);
    }

    #[test]
    fn mutated_graph_json_is_rejected_or_canonical(
        edits in prop::collection::vec((any::<usize>(), 0..32usize, ".{0,32}"), 1..12),
    ) {
        let seed = minimal_document(json!({
            "nodes": [branch_node(1), branch_node(2)],
            "edges": [edge(1, 1, 2)]
        }));
        let mut input: Vec<char> = seed.chars().collect();
        for (offset, count, replacement) in edits {
            let start = offset % (input.len() + 1);
            let end = (start + count).min(input.len());
            input.splice(start..end, replacement.chars());
        }
        let input: String = input.into_iter().collect();
        if let Ok(decoded) = from_json(&input) {
            let saved = to_json(&decoded).unwrap();
            prop_assert_eq!(from_json(&saved).unwrap(), decoded);
        }
    }
}
