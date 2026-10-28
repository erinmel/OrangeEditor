//! Structural transaction rules: atomicity, ID uniqueness, body membership, ordered undo,
//! stale inverses and change-set deltas.

mod common;

use common::{
    add, body, chain, connect, content, eid, empty_document, function, function_id, nid, node,
    port, saved, typed_edge,
};
use indexmap::IndexMap;
use orange_model::document::{
    Edge, FunctionDecl, GraphBody, ProviderBinding, VarScope, VariableDecl,
};
use orange_model::id::VariableId;
use orange_model::{BodyRef, DeclId, DocCommand, GraphDocument, TxError, from_json};

fn vid() -> VariableId {
    "30000000-0000-4000-8000-000000000001".parse().unwrap()
}

/// Function `f` whose body holds test nodes `nodes` and the given edges.
fn function_with(nodes: &[u8], edges: Vec<Edge>) -> FunctionDecl {
    function(body(nodes.iter().map(|n| node(*n)), edges))
}

fn variable(name: &str) -> VariableDecl {
    VariableDecl {
        id: vid(),
        name: name.into(),
        ty: "core/i64@1".parse().unwrap(),
        scope: VarScope::Run,
        init: None,
        persistent: false,
        conflict: Default::default(),
        extensions: Default::default(),
    }
}

/// Asserts the command fails with `expected` and leaves the document byte-identical.
fn assert_rejected(document: &mut GraphDocument, commands: Vec<DocCommand>, expected: TxError) {
    let before = saved(document);
    assert_eq!(document.apply(commands).unwrap_err(), expected);
    assert_eq!(saved(document), before, "document and revision unchanged");
}

#[test]
fn empty_transaction_is_rejected() {
    assert_rejected(&mut empty_document(), vec![], TxError::Empty);
}

#[test]
fn failure_rolls_back_earlier_commands() {
    let mut document = chain();
    // The first two commands succeed before the third fails.
    assert_rejected(
        &mut document,
        vec![add(4), connect(3, 3, 4), connect(4, 4, 9)],
        TxError::EdgeEndpointMissing {
            edge: eid(4),
            node: nid(9),
        },
    );
}

#[test]
fn ids_must_be_unique_and_targets_must_exist() {
    let mut document = chain();
    assert_rejected(&mut document, vec![add(1)], TxError::NodeExists(nid(1)));
    assert_rejected(
        &mut document,
        vec![connect(1, 1, 3)],
        TxError::EdgeExists(eid(1)),
    );
    assert_rejected(
        &mut document,
        vec![DocCommand::Disconnect { edge: eid(9) }],
        TxError::EdgeNotFound(eid(9)),
    );
    assert_rejected(
        &mut document,
        vec![DocCommand::RemoveNodes { ids: vec![nid(9)] }],
        TxError::NodeNotFound(nid(9)),
    );
    assert_rejected(
        &mut document,
        vec![DocCommand::RemoveVariable(vid())],
        TxError::VariableNotFound(vid()),
    );
    assert_rejected(
        &mut document,
        vec![DocCommand::RemoveFunction(function_id())],
        TxError::FunctionNotFound(function_id()),
    );
    assert_rejected(
        &mut document,
        vec![DocCommand::AddNode {
            body: BodyRef::Function(function_id()),
            node: node(7),
        }],
        TxError::FunctionNotFound(function_id()),
    );
}

#[test]
fn edges_stay_inside_one_body() {
    let mut document = chain();
    let _ = document
        .apply([DocCommand::UpsertFunction(function_with(&[5, 6], vec![]))])
        .unwrap();
    assert_eq!(
        document.locate_node(nid(5)),
        Some(BodyRef::Function(function_id()))
    );
    assert_rejected(
        &mut document,
        vec![connect(3, 1, 5)],
        TxError::CrossBodyEdge(eid(3)),
    );

    let _ = document.apply([connect(3, 5, 6)]).unwrap();
    assert_eq!(
        document.locate_edge(eid(3)),
        Some(BodyRef::Function(function_id()))
    );
}

#[test]
fn function_bodies_obey_the_same_rules() {
    let mut document = chain();
    assert_rejected(
        &mut document,
        vec![DocCommand::UpsertFunction(function_with(&[1], vec![]))],
        TxError::NodeExists(nid(1)),
    );
    let outward = typed_edge(9, port(5, "then"), port(1, "in"));
    assert_rejected(
        &mut document,
        vec![DocCommand::UpsertFunction(function_with(
            &[5],
            vec![outward],
        ))],
        TxError::EdgeEndpointMissing {
            edge: eid(9),
            node: nid(1),
        },
    );
}

#[test]
fn remove_nodes_takes_incident_edges_and_undo_restores_order() {
    let mut document = chain();
    let before = content(&document);

    let removal = document
        .apply([DocCommand::RemoveNodes { ids: vec![nid(2)] }])
        .unwrap();
    assert_eq!(document.root().edges.len(), 0, "both edges touched node 2");
    let change = &removal.change_set;
    assert_eq!(
        change.nodes().removed.iter().copied().collect::<Vec<_>>(),
        [nid(2)]
    );
    assert_eq!(change.edges().removed.len(), 2);
    assert!(!change.layout_only());

    let _ = document.revert(removal.inverse).unwrap();
    assert_eq!(
        content(&document),
        before,
        "node 2 back between 1 and 3, edges in order"
    );
}

#[test]
fn revision_counts_every_committed_transaction_and_redo_works() {
    let mut document = chain();
    let start = document.revision();
    let edited = document
        .apply([DocCommand::Disconnect { edge: eid(1) }])
        .unwrap();
    let after_edit = content(&document);
    assert_eq!(edited.change_set.to_revision(), start + 1);

    let undo = document.revert(edited.inverse).unwrap();
    assert_eq!(document.revision(), start + 2, "undo is a transaction too");
    let redo = document.revert(undo.inverse).unwrap();
    assert_eq!(content(&document), after_edit);
    assert_eq!(
        redo.change_set
            .edges()
            .removed
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [eid(1)]
    );
}

#[test]
fn redo_stays_valid_after_several_undos() {
    // Regression: binding inverses to the revision broke the second redo, because each
    // undo and redo advances the revision.
    let mut document = chain();
    let first = document
        .apply([DocCommand::Disconnect { edge: eid(1) }])
        .unwrap();
    let second = document
        .apply([DocCommand::Disconnect { edge: eid(2) }])
        .unwrap();
    let edited = content(&document);

    let redo_second = document.revert(second.inverse).unwrap().inverse;
    let redo_first = document.revert(first.inverse).unwrap().inverse;
    let _ = document.revert(redo_first).unwrap();
    let _ = document.revert(redo_second).unwrap();
    assert_eq!(content(&document), edited);
}

#[test]
fn stale_inverse_is_rejected() {
    let mut document = chain();
    let first = document
        .apply([DocCommand::Disconnect { edge: eid(1) }])
        .unwrap();
    let _ = document
        .apply([DocCommand::Disconnect { edge: eid(2) }])
        .unwrap();
    let before = saved(&document);
    assert_eq!(
        document.revert(first.inverse.clone()).unwrap_err(),
        TxError::StaleInverse {
            expected: first.inverse.applies_to_state(),
            actual: document.state_id()
        }
    );
    assert_eq!(saved(&document), before);
}

#[test]
fn upsert_replaces_in_place_and_reports_decl_deltas() {
    let mut document = empty_document();
    let created = document
        .apply([DocCommand::UpsertVariable(variable("total"))])
        .unwrap();
    assert!(
        created
            .change_set
            .decls()
            .added
            .contains(&DeclId::Variable(vid()))
    );

    let renamed = document
        .apply([DocCommand::UpsertVariable(variable("sum"))])
        .unwrap();
    assert!(
        renamed
            .change_set
            .decls()
            .changed
            .contains(&DeclId::Variable(vid()))
    );
    assert_eq!(document.variables()[&vid()].name, "sum");
}

#[test]
fn net_deltas_ignore_ids_added_and_removed_together() {
    let mut document = chain();
    let committed = document
        .apply([
            add(4),
            connect(3, 3, 4),
            DocCommand::RemoveNodes { ids: vec![nid(4)] },
        ])
        .unwrap();
    assert!(committed.change_set.nodes().is_empty());
    assert!(committed.change_set.edges().is_empty());
}

#[test]
fn rebind_changes_only_the_provider() {
    let mut document = chain();
    let provider = ProviderBinding {
        package: "org.orange.math".into(),
        version: "0.2.0".into(),
        id: "native-checked".into(),
        extensions: Default::default(),
    };
    let committed = document
        .apply([DocCommand::Rebind {
            nodes: vec![nid(1), nid(3)],
            provider: provider.clone(),
        }])
        .unwrap();
    assert_eq!(
        document.node(nid(1)).unwrap().provider.as_ref(),
        Some(&provider)
    );
    assert_eq!(committed.change_set.nodes().changed.len(), 2);
    assert!(committed.change_set.edges().is_empty());
}

#[test]
fn put_signature_is_idempotent_rejects_conflicts_and_undoes() {
    use orange_model::document::SignatureSnapshot;
    let snapshot = |ty: &str| -> SignatureSnapshot {
        serde_json::from_value(serde_json::json!({"kind": "pure", "ports": [
            {"port_id": "a", "direction": "input", "kind": "data", "type": ty}
        ]}))
        .unwrap()
    };
    let key: orange_model::id::SignatureKey = "s-a".parse().unwrap();
    let put = |ty: &str| DocCommand::PutSignature {
        key: key.clone(),
        snapshot: snapshot(ty),
    };

    let mut document = empty_document();
    let before = content(&document);
    let first = document.apply([put("core/i64@1")]).unwrap();
    let again = document.apply([put("core/i64@1")]).unwrap();
    assert!(again.change_set.nodes().is_empty() && document.signatures().len() == 1);
    assert_rejected(
        &mut document,
        vec![put("core/f64@1")],
        TxError::SignatureConflict(key.clone()),
    );

    let _ = document.revert(again.inverse).unwrap();
    let _ = document.revert(first.inverse).unwrap();
    assert_eq!(content(&document), before);
}

#[test]
fn divergent_clones_and_reloads_reject_each_others_inverses() {
    let original = chain();
    let mut left = original.clone();
    let mut right = original;
    let left_edit = left
        .apply([DocCommand::Disconnect { edge: eid(1) }])
        .unwrap();
    let _right_edit = right
        .apply([DocCommand::Disconnect { edge: eid(2) }])
        .unwrap();
    let before = saved(&right);
    assert!(matches!(
        right.revert(left_edit.inverse.clone()),
        Err(TxError::StaleInverse { .. })
    ));
    assert_eq!(saved(&right), before);

    let mut reloaded = from_json(&saved(&left)).unwrap();
    let before = saved(&reloaded);
    assert!(matches!(
        reloaded.revert(left_edit.inverse),
        Err(TxError::StaleInverse { .. })
    ));
    assert_eq!(saved(&reloaded), before);
}

#[test]
fn equal_state_clones_can_share_undo() {
    let mut document = chain();
    let baseline = content(&document);
    let first = document
        .apply([DocCommand::Disconnect { edge: eid(1) }])
        .unwrap();
    let mut identical_clone = document.clone();
    let _ = identical_clone.revert(first.inverse.clone()).unwrap();
    assert_eq!(content(&identical_clone), baseline);
    let second = document
        .apply([DocCommand::Disconnect { edge: eid(2) }])
        .unwrap();
    let edited = content(&document);

    let redo_second = document.revert(second.inverse).unwrap().inverse;
    let redo_first = document.revert(first.inverse).unwrap().inverse;
    assert_eq!(content(&document), baseline);
    let _ = document.revert(redo_first).unwrap();
    let _ = document.revert(redo_second).unwrap();
    assert_eq!(content(&document), edited);
}

#[test]
fn mismatched_function_node_key_is_typed_and_atomic() {
    let mut nodes = IndexMap::new();
    nodes.insert(nid(9), node(1));
    let malformed = function(GraphBody {
        nodes,
        edges: IndexMap::new(),
    });
    assert_rejected(
        &mut empty_document(),
        vec![add(2), DocCommand::UpsertFunction(malformed)],
        TxError::FunctionNodeKeyMismatch {
            key: nid(9),
            item: nid(1),
        },
    );
}

#[test]
fn mismatched_function_edge_key_is_typed_and_atomic() {
    let mut edges = IndexMap::new();
    edges.insert(eid(9), typed_edge(1, port(1, "then"), port(2, "in")));
    let malformed = function(GraphBody {
        nodes: body([node(1), node(2)], []).nodes,
        edges,
    });
    assert_rejected(
        &mut empty_document(),
        vec![add(3), DocCommand::UpsertFunction(malformed)],
        TxError::FunctionEdgeKeyMismatch {
            key: eid(9),
            item: eid(1),
        },
    );
}

#[test]
fn put_package_lock_appends_is_idempotent_rejects_another_digest_and_undoes() {
    use orange_model::document::PackageLockEntry;
    let lock = |package: &str, digest: &str| PackageLockEntry {
        id: package.to_owned(),
        version: "1.0.0".to_owned(),
        content_digest: format!("sha256:{}", digest.repeat(64)),
        extensions: Default::default(),
    };

    let mut document = empty_document();
    let before = content(&document);
    let first = document
        .apply([
            DocCommand::PutPackageLock(lock("org.b", "b")),
            DocCommand::PutPackageLock(lock("org.a", "a")),
        ])
        .unwrap();
    let ids: Vec<&str> = document
        .package_lock()
        .iter()
        .map(|e| e.id.as_str())
        .collect();
    assert_eq!(ids, ["org.b", "org.a"], "appended in command order");
    let again = document
        .apply([DocCommand::PutPackageLock(lock("org.a", "a"))])
        .unwrap();
    assert_eq!(document.package_lock().len(), 2);
    assert_rejected(
        &mut document,
        vec![DocCommand::PutPackageLock(lock("org.a", "c"))],
        TxError::PackageLockConflict {
            package: "org.a".to_owned(),
            version: "1.0.0".to_owned(),
        },
    );

    let _ = document.revert(again.inverse).unwrap();
    let _ = document.revert(first.inverse).unwrap();
    assert_eq!(content(&document), before);
}

#[test]
fn type_locks_append_coalesce_reject_another_codec_and_undo() {
    use orange_model::document::TypeLockEntry;
    let lock = |ty: &str, codec: &str| TypeLockEntry {
        ty: ty.parse().unwrap(),
        codec: codec.to_owned(),
        extensions: Default::default(),
    };

    let mut document = empty_document();
    let before = content(&document);
    let first = document
        .apply([
            DocCommand::PutTypeLock(lock("core/u64@1", "orange.value.v1")),
            DocCommand::PutTypeLock(lock("core/i64@1", "orange.value.v1")),
        ])
        .unwrap();
    let again = document
        .apply([DocCommand::PutTypeLock(lock(
            "core/i64@1",
            "orange.value.v1",
        ))])
        .unwrap();
    assert_eq!(document.type_lock().len(), 2, "a matching lock is reused");
    assert_rejected(
        &mut document,
        vec![DocCommand::PutTypeLock(lock(
            "core/i64@1",
            "org.example.other-v1",
        ))],
        TxError::TypeLockConflict("core/i64@1".parse().unwrap()),
    );
    assert_rejected(
        &mut document,
        vec![DocCommand::RemoveTypeLock("core/bool@1".parse().unwrap())],
        TxError::TypeLockNotFound("core/bool@1".parse().unwrap()),
    );

    // Changing a codec is an explicit removal and re-lock; undo restores the original slot.
    let locked = content(&document);
    let migrated = document
        .apply([
            DocCommand::RemoveTypeLock("core/u64@1".parse().unwrap()),
            DocCommand::PutTypeLock(lock("core/u64@1", "org.example.other-v1")),
        ])
        .unwrap();
    assert_eq!(
        document.type_lock()[1],
        lock("core/u64@1", "org.example.other-v1")
    );
    let _ = document.revert(migrated.inverse).unwrap();
    assert_eq!(content(&document), locked);

    let _ = document.revert(again.inverse).unwrap();
    let _ = document.revert(first.inverse).unwrap();
    assert_eq!(content(&document), before);
}

#[test]
fn exports_upsert_in_place_remove_and_undo() {
    use orange_model::document::GraphExport;
    let export = |id: &str, port: &str| GraphExport {
        id: id.to_owned(),
        node: nid(1),
        port: port.parse().unwrap(),
        extensions: Default::default(),
    };

    let mut document = empty_document();
    let _ = document.apply([add(1)]).unwrap();
    let before = content(&document);
    let first = document
        .apply([
            DocCommand::UpsertExport(export("a", "true")),
            DocCommand::UpsertExport(export("b", "true")),
        ])
        .unwrap();
    let replaced = document
        .apply([DocCommand::UpsertExport(export("a", "false"))])
        .unwrap();
    assert_eq!(
        document.exports(),
        [export("a", "false"), export("b", "true")]
    );
    assert!(!replaced.change_set.layout_only());

    let removed = document
        .apply([DocCommand::RemoveExport { id: "a".into() }])
        .unwrap();
    assert_eq!(document.exports(), [export("b", "true")]);
    assert_rejected(
        &mut document,
        vec![DocCommand::RemoveExport { id: "a".into() }],
        TxError::ExportNotFound("a".into()),
    );

    let _ = document.revert(removed.inverse).unwrap();
    assert_eq!(
        document.exports(),
        [export("a", "false"), export("b", "true")]
    );
    let _ = document.revert(replaced.inverse).unwrap();
    let _ = document.revert(first.inverse).unwrap();
    assert_eq!(content(&document), before);
}

#[test]
fn activation_policies_are_set_cleared_and_undone() {
    use orange_model::document::{ActivationPolicy, FaultPolicy, OverflowPolicy, ReentrancyMode};
    let policy = ActivationPolicy {
        mode: ReentrancyMode::Queue,
        max_parallel: None,
        capacity: 4,
        overflow: OverflowPolicy::DropOldest,
        on_fault: FaultPolicy::Continue,
        order: 0,
        extensions: Default::default(),
    };

    let mut document = empty_document();
    let _ = document.apply([add(1)]).unwrap();
    let before = content(&document);
    let set = document
        .apply([DocCommand::SetActivation {
            node: nid(1),
            policy: Some(policy.clone()),
        }])
        .unwrap();
    assert_eq!(document.node(nid(1)).unwrap().activation, Some(policy));
    let cleared = document
        .apply([DocCommand::SetActivation {
            node: nid(1),
            policy: None,
        }])
        .unwrap();
    assert_eq!(document.node(nid(1)).unwrap().activation, None);
    assert_rejected(
        &mut document,
        vec![DocCommand::SetActivation {
            node: nid(2),
            policy: None,
        }],
        TxError::NodeNotFound(nid(2)),
    );

    let _ = document.revert(cleared.inverse).unwrap();
    let _ = document.revert(set.inverse).unwrap();
    assert_eq!(content(&document), before);
}

/// A loaded document whose revision is `revision`.
fn at_revision(revision: u64) -> GraphDocument {
    let mut value: serde_json::Value = serde_json::from_str(&saved(&empty_document())).unwrap();
    value["revision"] = revision.into();
    from_json(&value.to_string()).unwrap()
}

#[test]
fn an_exhausted_revision_rejects_edits_and_undo_without_changing_anything() {
    let mut doc = at_revision(u64::MAX);
    let (before, state) = (content(&doc), doc.state_id());
    let add_node = || DocCommand::AddNode {
        body: BodyRef::Root,
        node: node(1),
    };
    assert_eq!(doc.apply([add_node()]), Err(TxError::RevisionExhausted));
    assert_eq!(
        (content(&doc), doc.revision(), doc.state_id()),
        (before, u64::MAX, state)
    );

    // The last revision is reachable, and undo then fails the same way.
    let mut doc = at_revision(u64::MAX - 1);
    let done = doc.apply([add_node()]).unwrap();
    let (after, state) = (content(&doc), doc.state_id());
    assert_eq!(doc.revision(), u64::MAX);
    assert_eq!(doc.revert(done.inverse), Err(TxError::RevisionExhausted));
    assert_eq!(
        (content(&doc), doc.revision(), doc.state_id()),
        (after, u64::MAX, state)
    );
}
