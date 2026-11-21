//! Property: any structurally valid document survives save→load unchanged and saves to
//! the same bytes again.

use crate::behaviour::common;

use crate::{from_json, to_json};
use common::strategies::document;
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
