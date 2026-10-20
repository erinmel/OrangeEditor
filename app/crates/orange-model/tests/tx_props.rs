//! Property: for any sequence of edits, undoing all of them restores the original content
//! and redoing all of them reproduces the edited content (FR-03 document state). A
//! rejected edit leaves the document byte-identical.

mod common;

use common::edits::{command, op};
use common::{content, empty_document, saved};
use proptest::prelude::*;

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
