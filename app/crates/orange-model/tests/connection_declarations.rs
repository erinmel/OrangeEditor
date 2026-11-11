//! Connection declarations are document content like variables and events: saved,
//! edited atomically, undone exactly, and referenced by nodes through config.
use orange_model::catalog::{NodeDefinition, ReferenceKind};
use orange_model::document::{ConnectionDecl, ConnectionScope};
use orange_model::id::ConnectionId;
use orange_model::{DeclId, DocCommand, GraphDocument, TxError, from_json, to_json};
use serde_json::json;

fn connection() -> ConnectionDecl {
    serde_json::from_value(json!({
        "id": "40000000-0000-4000-8000-000000000001", "name": "obs",
        "adapter": "org.orange.net/websocket", "scope": "graph",
        "settings": {"url": "ws://127.0.0.1:4455"}
    }))
    .unwrap()
}
fn empty() -> GraphDocument {
    GraphDocument::new("20000000-0000-4000-8000-000000000001".parse().unwrap())
}

#[test]
fn a_connection_is_saved_loaded_and_undone_exactly() {
    let mut document = empty();
    let before = to_json(&document).unwrap();
    // A document without connections keeps its previous saved form.
    assert!(!before.contains("connections"));

    let decl = connection();
    let committed = document
        .apply([DocCommand::UpsertConnection(decl.clone())])
        .unwrap();
    assert!(
        committed
            .change_set
            .decls()
            .added
            .contains(&DeclId::Connection(decl.id))
    );
    let saved = to_json(&document).unwrap();
    let loaded = from_json(&saved).unwrap();
    assert_eq!(loaded.connections().get(&decl.id), Some(&decl));
    assert_eq!(to_json(&loaded).unwrap(), saved);

    let _ = document.revert(committed.inverse).unwrap();
    assert!(document.connections().is_empty());
}

#[test]
fn a_shared_connection_carries_no_settings_and_removal_needs_a_target() {
    let mut document = empty();
    let shared: ConnectionDecl = serde_json::from_value(json!({
        "id": "40000000-0000-4000-8000-000000000002", "name": "chat",
        "adapter": "org.orange.net/websocket", "scope": "shared"
    }))
    .unwrap();
    assert_eq!(shared.scope, ConnectionScope::Shared);
    assert!(shared.settings.is_empty());
    let _ = document
        .apply([DocCommand::UpsertConnection(shared.clone())])
        .unwrap();
    assert!(!to_json(&document).unwrap().contains("settings"));
    let _ = document
        .apply([DocCommand::RemoveConnection(shared.id)])
        .unwrap();
    let missing: ConnectionId = "40000000-0000-4000-8000-000000000009".parse().unwrap();
    assert_eq!(
        document.apply([DocCommand::RemoveConnection(missing)]),
        Err(TxError::ConnectionNotFound(missing))
    );
}

#[test]
fn a_package_definition_can_declare_that_it_uses_a_connection() {
    let definition = |uses: bool| {
        NodeDefinition::from_json(
            &json!({"definition_id": "org.example.net/send", "definition_version": 1,
                "config_version": 1, "kind": "async", "suspends": "always",
                "determinism": "external", "ports": [], "connection": uses})
            .to_string(),
        )
        .unwrap()
    };
    let references = definition(true).references;
    assert_eq!(references.len(), 1);
    assert_eq!(references[0].pointer, "/connection");
    assert_eq!(references[0].kind, ReferenceKind::Connection);
    assert!(definition(false).references.is_empty());
}
