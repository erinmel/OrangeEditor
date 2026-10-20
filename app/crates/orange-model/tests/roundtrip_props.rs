//! Property: any structurally valid document survives save→load unchanged and saves to
//! the same bytes again.

mod common;

use common::strategies::document;
use orange_model::{from_json, to_json};
use proptest::prelude::*;

proptest! {
    #[test]
    fn save_load_round_trip(document in document()) {
        let saved = to_json(&document).unwrap();
        let loaded = from_json(&saved).unwrap();
        prop_assert_eq!(&loaded, &document);
        prop_assert_eq!(to_json(&loaded).unwrap(), saved);
    }
}
