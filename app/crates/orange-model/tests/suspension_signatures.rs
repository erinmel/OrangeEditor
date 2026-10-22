use std::collections::BTreeMap;

use orange_model::document::{NodeKind, SignatureSnapshot, Suspends};
use orange_model::id::TypeExpr;
use orange_model::signature::{
    canonical_json, compare, project, project_with_suspension, semantic_key,
};
use serde_json::{Value, json};

fn snapshot(value: Value) -> SignatureSnapshot {
    serde_json::from_value(value).unwrap()
}

#[test]
fn legacy_async_and_iterator_snapshots_keep_suspension_unknown() {
    for kind in ["async", "iterator"] {
        let legacy = json!({"kind":kind,"ports":[]});
        let decoded = snapshot(legacy);
        assert_eq!(decoded.suspends, None);
        assert_eq!(decoded.suspension(), None);

        let saved = serde_json::to_value(&decoded).unwrap();
        assert!(saved.get("suspends").is_none());
        assert_eq!(snapshot(saved), decoded);
    }

    let pure = snapshot(json!({"kind":"pure","ports":[]}));
    assert_eq!(pure.suspends, None);
    assert_eq!(pure.suspension(), Some(Suspends::Never));
    assert!(
        serde_json::to_value(pure)
            .unwrap()
            .get("suspends")
            .is_none()
    );
}

#[test]
fn suspension_is_semantic_signature_data() {
    let maybe = snapshot(json!({"kind":"async","suspends":"maybe","ports":[]}));
    let always = snapshot(json!({"kind":"async","suspends":"always","ports":[]}));
    assert_ne!(semantic_key(&maybe), semantic_key(&always));
    assert_eq!(maybe.suspension(), Some(Suspends::Maybe));
    assert_eq!(always.suspension(), Some(Suspends::Always));
    assert!(canonical_json(&always).contains("\"suspends\":\"always\""));
    let drift = compare(&maybe, &always).unwrap();
    assert!(drift.suspension_changed);
    assert!(!drift.kind_changed);
}

#[test]
fn projection_compatibility_keeps_unknown_and_explicit_api_accepts_declaration() {
    let args = BTreeMap::<String, TypeExpr>::new();
    let async_snapshot = project(NodeKind::Async, [], &args);
    assert_eq!(async_snapshot.suspends, None);
    assert_eq!(async_snapshot.suspension(), None);

    let always_snapshot =
        project_with_suspension(NodeKind::Async, Some(Suspends::Always), [], &args);
    assert_eq!(always_snapshot.suspends, Some(Suspends::Always));
    assert_ne!(
        semantic_key(&async_snapshot),
        semantic_key(&always_snapshot)
    );

    let action = project(NodeKind::Action, [], &args);
    assert_eq!(action.suspends, None);
    assert_eq!(action.suspension(), Some(Suspends::Never));
}

#[test]
fn explicit_never_on_sync_signature_has_same_key_as_omission() {
    let args = BTreeMap::<String, TypeExpr>::new();
    let omitted = project(NodeKind::Pure, [], &args);
    let explicit = project_with_suspension(NodeKind::Pure, Some(Suspends::Never), [], &args);
    assert_eq!(semantic_key(&omitted), semantic_key(&explicit));
    assert!(compare(&omitted, &explicit).is_none());
}
