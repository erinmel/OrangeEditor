//! Journal-owned change identities and stale-index recovery.
mod common;

use common::{empty_document, node};
use orange_model::document::{PackageLockEntry, SignatureSnapshot};
use orange_model::{BodyRef, ChangeSet, DocCommand, DocIndex, GraphDocument, LayoutCommand};
use serde_json::json;

fn identities(document: &GraphDocument) -> (u64, u64, u64) {
    (
        document.revision(),
        document.state_id(),
        document.semantic_state_id(),
    )
}

fn assert_transition(change: &ChangeSet, before: (u64, u64, u64), after: &GraphDocument) {
    assert!(change.is_committed());
    assert_eq!(
        (
            change.from_revision(),
            change.from_state(),
            change.from_semantic_state()
        ),
        before
    );
    assert_eq!(
        (
            change.to_revision(),
            change.to_state(),
            change.to_semantic_state()
        ),
        identities(after)
    );
    assert_eq!(
        change.layout_only(),
        change.from_semantic_state() == change.to_semantic_state()
    );
}

fn add(n: u8) -> DocCommand {
    DocCommand::AddNode {
        body: BodyRef::Root,
        node: node(n),
    }
}

#[test]
fn edit_layout_undo_and_redo_bind_exact_content_and_semantics() {
    let mut document = empty_document();
    let initial = identities(&document);
    let semantic = document.apply([add(1)]).unwrap();
    assert_transition(&semantic.change_set, initial, &document);
    assert!(!semantic.change_set.layout_only());
    let after_semantic = identities(&document);

    let layout = document
        .apply([DocCommand::Layout(LayoutCommand::Set {
            path: vec!["position".into()],
            value: Some(json!([2, 4])),
        })])
        .unwrap();
    assert_transition(&layout.change_set, after_semantic, &document);
    assert!(layout.change_set.layout_only());
    assert_ne!(document.state_id(), after_semantic.1);
    assert_eq!(document.semantic_state_id(), after_semantic.2);
    let after_layout = identities(&document);

    let undo_layout = document.revert(layout.inverse).unwrap();
    assert_transition(&undo_layout.change_set, after_layout, &document);
    assert_eq!(
        (document.state_id(), document.semantic_state_id()),
        (after_semantic.1, after_semantic.2)
    );
    let before_undo = identities(&document);
    let undo_semantic = document.revert(semantic.inverse).unwrap();
    assert_transition(&undo_semantic.change_set, before_undo, &document);
    assert_eq!(
        (document.state_id(), document.semantic_state_id()),
        (initial.1, initial.2)
    );

    let before_redo = identities(&document);
    let redo = document.revert(undo_semantic.inverse).unwrap();
    assert_transition(&redo.change_set, before_redo, &document);
    let redo_layout = document.revert(undo_layout.inverse).unwrap();
    assert!(redo_layout.change_set.layout_only());
    assert_eq!(
        (document.state_id(), document.semantic_state_id()),
        (after_layout.1, after_layout.2)
    );
}

#[test]
fn same_revision_divergent_documents_and_skipped_edits_rebuild_indexes() {
    let mut left = empty_document();
    let mut right = left.clone();
    let mut index = DocIndex::build(&left);
    let left_edit = left.apply([add(1)]).unwrap();
    let _right_edit = right.apply([add(2)]).unwrap();
    assert_eq!(left.revision(), right.revision());
    index.update(&right, &left_edit.change_set);
    assert_eq!(index, DocIndex::build(&right));

    let _missed = right.apply([add(3)]).unwrap();
    let latest = right.apply([add(4)]).unwrap();
    index.update(&right, &latest.change_set);
    assert_eq!(index, DocIndex::build(&right));
    // Replaying an old receipt must never mark its old deltas current for this document.
    index.update(&right, &left_edit.change_set);
    assert_eq!(index, DocIndex::build(&right));
}

#[test]
fn shared_signature_and_lock_changes_report_global_invalidation_including_undo() {
    let signature: SignatureSnapshot =
        serde_json::from_value(json!({"kind":"pure", "ports":[]})).unwrap();
    let lock: PackageLockEntry = serde_json::from_value(json!({
        "id":"org.test.provenance", "version":"1.0.0", "content_digest":format!("sha256:{}", "0".repeat(64))
    })).unwrap();
    let mut document = empty_document();
    for command in [
        DocCommand::PutSignature {
            key: "provenance".parse().unwrap(),
            snapshot: signature,
        },
        DocCommand::PutPackageLock(lock),
    ] {
        let before = identities(&document);
        let edit = document.apply([command]).unwrap();
        assert_transition(&edit.change_set, before, &document);
        assert!(edit.change_set.global_dirty());
        assert!(!edit.change_set.layout_only());
        assert!(edit.change_set.nodes().is_empty());
        assert!(edit.change_set.edges().is_empty());
        assert!(edit.change_set.decls().is_empty());
        let undo = document.revert(edit.inverse).unwrap();
        assert!(undo.change_set.global_dirty());
        assert_eq!(document.state_id(), before.1);
    }
}
