//! 00c.4b: inference results persisted in the same transaction as the edit (11 §5.5).

mod common;

use common::generic::{add, data_edge, document, registry, source, typed_source};
use common::{body, content, eid, function, function_id, minimal_document, nid, node_id, saved};
use orange_model::document::{Edge, NodeInstance};
use orange_model::inference::{InferenceIssue, apply_inferred, apply_inferred_indexed, plan_edit};
use orange_model::{BodyRef, DocCommand, DocIndex, GraphDocument, TxError, from_json};
use serde_json::{Value as Json, json};

fn connect(edge: Json) -> DocCommand {
    DocCommand::Connect {
        edge: serde_json::from_value::<Edge>(edge).unwrap(),
    }
}

fn add_node(node: Json) -> DocCommand {
    DocCommand::AddNode {
        body: BodyRef::Root,
        node: serde_json::from_value::<NodeInstance>(node).unwrap(),
    }
}

fn commit(
    document: &mut GraphDocument,
    edit: Vec<DocCommand>,
) -> Result<Vec<InferenceIssue>, TxError> {
    apply_inferred(document, edit, &registry(None)).map(|done| done.issues)
}

fn type_arg(document: &GraphDocument, n: u8) -> Option<String> {
    let args = &document.node(nid(n))?.type_args;
    args.get("T").map(|ty| ty.to_string())
}

/// The types of the node's referenced snapshot, in port order.
fn signature_types(document: &GraphDocument, n: u8) -> Vec<String> {
    let key = document.node(nid(n)).unwrap().signature.as_ref().unwrap();
    document.signatures()[key]
        .ports
        .iter()
        .map(|port| port.ty.as_ref().unwrap().to_string())
        .collect()
}

fn codes(issues: &[InferenceIssue]) -> Vec<&'static str> {
    issues.iter().map(InferenceIssue::code).collect()
}

#[test]
fn reusing_an_edge_id_reinfers_old_and_new_endpoints_and_is_undoable() {
    for target in [3, 4] {
        let mut doc = document(
            vec![
                typed_source(1, "i64"),
                typed_source(2, "f64"),
                add(3, json!({})),
                add(4, json!({})),
                add(5, json!({"type_args": {"T": "core/i32@1"}})),
            ],
            vec![],
        );
        commit(
            &mut doc,
            vec![connect(data_edge(1, (1, "value"), (3, "a")))],
        )
        .unwrap();
        let before = content(&doc);
        let unrelated = doc.node(nid(5)).unwrap().clone();
        let revision = doc.revision();
        let done = apply_inferred(
            &mut doc,
            [
                DocCommand::Disconnect { edge: eid(1) },
                connect(data_edge(1, (2, "value"), (target, "a"))),
            ],
            &registry(None),
        )
        .unwrap();
        assert!(done.committed.change_set.edges.changed.contains(&eid(1)));
        assert_eq!(doc.revision(), revision + 1);
        assert_eq!(type_arg(&doc, target).as_deref(), Some("core/f64@1"));
        assert_eq!(signature_types(&doc, target), ["core/f64@1"; 3]);
        if target == 4 {
            assert_eq!(type_arg(&doc, 3), None);
            assert_eq!(signature_types(&doc, 3), ["$T"; 3]);
            assert_eq!(codes(&done.issues), ["E-TYPE-AMBIGUOUS"]);
        } else {
            assert!(done.issues.is_empty());
        }
        assert_eq!(doc.node(nid(5)).unwrap(), &unrelated);
        let after = content(&doc);
        let redo = doc.revert(done.committed.inverse).unwrap().inverse;
        assert_eq!(content(&doc), before);
        let _ = doc.revert(redo).unwrap();
        assert_eq!(content(&doc), after);
    }
}

/// An `i64` source (1) and an unresolved `add` (2), not yet connected.
fn unconnected() -> GraphDocument {
    document(vec![source(1), add(2, json!({}))], vec![])
}

#[test]
fn resolved_bindings_and_signature_survive_save_and_load() {
    let mut doc = unconnected();
    let issues = commit(
        &mut doc,
        vec![connect(data_edge(1, (1, "value"), (2, "a")))],
    )
    .unwrap();
    assert!(issues.is_empty());
    assert_eq!(type_arg(&doc, 2).as_deref(), Some("core/i64@1"));
    assert_eq!(signature_types(&doc, 2), ["core/i64@1"; 3]);

    let loaded = from_json(&saved(&doc)).unwrap();
    assert_eq!(loaded.node(nid(2)), doc.node(nid(2)));
    assert_eq!(signature_types(&loaded, 2), ["core/i64@1"; 3]);
    assert_eq!(content(&loaded), content(&doc));
}

#[test]
fn one_undo_restores_the_prior_state_and_redo_the_accepted_result() {
    let mut doc = unconnected();
    let (before, revision, snapshots) = (content(&doc), doc.revision(), doc.signatures().len());
    let done = apply_inferred(
        &mut doc,
        vec![connect(data_edge(1, (1, "value"), (2, "a")))],
        &registry(None),
    )
    .unwrap();
    assert_eq!(doc.revision(), revision + 1);
    let after = content(&doc);

    let redo = doc.revert(done.committed.inverse).unwrap().inverse;
    assert_eq!(content(&doc), before);
    assert_eq!(type_arg(&doc, 2), None);
    assert_eq!(doc.node(nid(2)).unwrap().signature, None);
    assert_eq!(
        doc.signatures().len(),
        snapshots,
        "the inferred snapshot is undone"
    );

    let _ = doc.revert(redo).unwrap();
    assert_eq!(content(&doc), after);
    assert_eq!(doc.revision(), revision + 3);
}

#[test]
fn a_failed_edit_leaves_content_revision_and_state_unchanged() {
    let mut doc = unconnected();
    let (before, revision, state) = (content(&doc), doc.revision(), doc.state_id());
    let result = commit(
        &mut doc,
        vec![
            connect(data_edge(1, (1, "value"), (2, "a"))),
            DocCommand::Disconnect { edge: eid(9) },
        ],
    );
    assert_eq!(result, Err(TxError::EdgeNotFound(eid(9))));
    assert_eq!(content(&doc), before);
    assert_eq!((doc.revision(), doc.state_id()), (revision, state));
}

#[test]
fn a_kept_index_matches_the_document_through_commit_failure_and_removal() {
    let registry = registry(None);
    let mut doc = unconnected();
    let mut index = DocIndex::build(&doc);
    let edit = vec![connect(data_edge(1, (1, "value"), (2, "a")))];
    let mut plain = doc.clone();
    let expected = apply_inferred(&mut plain, edit.clone(), &registry).unwrap();
    let done = apply_inferred_indexed(&mut doc, &mut index, edit, &registry).unwrap();
    assert_eq!(done.issues, expected.issues);
    assert_eq!(content(&doc), content(&plain));
    assert_eq!(index, DocIndex::build(&doc));

    let failed = apply_inferred_indexed(
        &mut doc,
        &mut index,
        [DocCommand::Disconnect { edge: eid(9) }],
        &registry,
    );
    assert_eq!(failed.unwrap_err(), TxError::EdgeNotFound(eid(9)));
    assert_eq!(index, DocIndex::build(&doc));

    let removal = vec![DocCommand::RemoveNodes { ids: vec![nid(1)] }];
    let _ = apply_inferred_indexed(&mut doc, &mut index, removal, &registry).unwrap();
    assert_eq!(index, DocIndex::build(&doc));
    assert_eq!(type_arg(&doc, 2), None);
}

#[test]
fn a_kept_index_left_stale_by_an_undo_is_rebuilt_before_the_next_edit() {
    let registry = registry(None);
    let mut doc = unconnected();
    let mut index = DocIndex::build(&doc);
    let edit = vec![connect(data_edge(1, (1, "value"), (2, "a")))];
    let done = apply_inferred_indexed(&mut doc, &mut index, edit, &registry).unwrap();
    // The undo bypasses the index, which still holds the edge.
    let _ = doc.revert(done.committed.inverse).unwrap();
    assert!(!index.is_current(&doc));

    let edit = vec![connect(data_edge(2, (1, "value"), (2, "b")))];
    let mut plain = doc.clone();
    let expected = apply_inferred(&mut plain, edit.clone(), &registry).unwrap();
    let done = apply_inferred_indexed(&mut doc, &mut index, edit, &registry).unwrap();
    assert_eq!(done.issues, expected.issues);
    assert_eq!(content(&doc), content(&plain));
    assert!(index.is_current(&doc));
    assert_eq!(index, DocIndex::build(&doc));
}

#[test]
fn disconnect_clears_the_stale_side_and_keeps_the_pinned_side() {
    // 2 is pinned to f64 and feeds 3, whose cached binding came from that edge.
    let cached = json!({"type_args": {"T": "core/f64@1"}});
    let mut doc = document(
        vec![
            add(
                2,
                json!({"type_args": {"T": "core/f64@1"}, "type_pins": {"T": "core/f64@1"}}),
            ),
            add(3, cached),
        ],
        vec![data_edge(1, (2, "sum"), (3, "a"))],
    );
    let issues = commit(&mut doc, vec![DocCommand::Disconnect { edge: eid(1) }]).unwrap();

    assert_eq!(codes(&issues), ["E-TYPE-AMBIGUOUS"]);
    assert_eq!(type_arg(&doc, 3), None);
    assert_eq!(signature_types(&doc, 3), ["$T"; 3]);
    let pinned = doc.node(nid(2)).unwrap();
    assert_eq!(pinned.type_pins["T"].as_str(), "core/f64@1");
    assert_eq!(type_arg(&doc, 2).as_deref(), Some("core/f64@1"));
    assert_eq!(signature_types(&doc, 2), ["core/f64@1"; 3]);
}

#[test]
fn unrelated_nodes_and_provider_locks_are_untouched() {
    let provider = json!({"package": "org.test", "version": "1.0.0", "id": "add"});
    // Node 4 is another generic component with a stale cache; the edit never touches it.
    let stale = json!({"provider": provider, "type_args": {"T": "core/i32@1"}});
    let mut doc = document(
        vec![
            source(1),
            add(2, json!({"provider": provider})),
            add(4, stale),
        ],
        vec![],
    );
    let (provider_2, node_4) = (
        doc.node(nid(2)).unwrap().provider.clone(),
        doc.node(nid(4)).unwrap().clone(),
    );
    let done = apply_inferred(
        &mut doc,
        vec![connect(data_edge(1, (1, "value"), (2, "a")))],
        &registry(None),
    )
    .unwrap();

    assert_eq!(doc.node(nid(2)).unwrap().provider, provider_2);
    assert_eq!(doc.node(nid(4)), Some(&node_4));
    assert!(!done.committed.change_set.nodes.changed.contains(&nid(4)));
}

#[test]
fn planning_reports_a_conflict_without_changing_the_document() {
    let mut doc = document(
        vec![
            add(2, json!({"type_pins": {"T": "core/i64@1"}})),
            add(3, json!({"type_pins": {"T": "core/f64@1"}})),
        ],
        vec![],
    );
    let edit = vec![connect(data_edge(1, (2, "sum"), (3, "a")))];
    let (before, revision) = (content(&doc), doc.revision());
    let plan = plan_edit(&doc, edit.clone(), &registry(None)).unwrap();
    assert_eq!(codes(&plan.issues), ["E-TYPE-MISMATCH"]);
    assert_eq!((content(&doc), doc.revision()), (before, revision));

    // Committing the plan is still one representable transaction: no bindings, `$T` kept.
    let issues = commit(&mut doc, edit).unwrap();
    assert_eq!(issues, plan.issues);
    assert_eq!(type_arg(&doc, 2), None);
    assert_eq!(signature_types(&doc, 3), ["$T"; 3]);
    assert_eq!(doc.revision(), revision + 1);
}

#[test]
fn adding_a_generic_node_with_a_pin_resolves_it_in_the_same_transaction() {
    let mut doc = document(vec![], vec![]);
    let node = add(2, json!({"type_pins": {"T": "core/u32@1"}}));
    let issues = commit(&mut doc, vec![add_node(node)]).unwrap();
    assert!(issues.is_empty());
    assert_eq!(type_arg(&doc, 2).as_deref(), Some("core/u32@1"));
    assert_eq!(signature_types(&doc, 2), ["core/u32@1"; 3]);
}

fn pins(ty: Option<&str>) -> DocCommand {
    DocCommand::SetTypePins {
        node: nid(2),
        pins: ty
            .map(|ty| [("T".to_owned(), ty.parse().unwrap())].into())
            .unwrap_or_default(),
    }
}

#[test]
fn pinning_resolves_unpinning_makes_ambiguous_and_one_undo_reverts_each() {
    let mut doc = document(vec![add(2, json!({}))], vec![]);
    let issues = commit(&mut doc, vec![pins(Some("core/f32@1"))]).unwrap();
    assert!(issues.is_empty());
    assert_eq!(type_arg(&doc, 2).as_deref(), Some("core/f32@1"));
    let pinned = content(&doc);

    let done = apply_inferred(&mut doc, vec![pins(None)], &registry(None)).unwrap();
    assert_eq!(codes(&done.issues), ["E-TYPE-AMBIGUOUS"]);
    assert_eq!(type_arg(&doc, 2), None);
    assert_eq!(signature_types(&doc, 2), ["$T"; 3]);

    let _ = doc.revert(done.committed.inverse).unwrap();
    assert_eq!(content(&doc), pinned);
}

#[test]
fn removing_the_typed_source_clears_the_neighbours_binding() {
    let mut doc = unconnected();
    let _ = commit(
        &mut doc,
        vec![connect(data_edge(1, (1, "value"), (2, "a")))],
    )
    .unwrap();
    assert_eq!(type_arg(&doc, 2).as_deref(), Some("core/i64@1"));

    let issues = commit(
        &mut doc,
        vec![DocCommand::RemoveNodes { ids: vec![nid(1)] }],
    )
    .unwrap();
    assert_eq!(codes(&issues), ["E-TYPE-AMBIGUOUS"]);
    assert_eq!(type_arg(&doc, 2), None);
    assert_eq!(signature_types(&doc, 2), ["$T"; 3]);
}

/// Function `f` whose body holds the given nodes and edges.
fn function_with(nodes: Vec<Json>, edges: Vec<Json>) -> DocCommand {
    let nodes = nodes
        .into_iter()
        .map(|node| serde_json::from_value(node).unwrap());
    let edges = edges
        .into_iter()
        .map(|edge| serde_json::from_value(edge).unwrap());
    DocCommand::UpsertFunction(function(body(nodes, edges)))
}

#[test]
fn a_replaced_function_body_is_inferred_and_one_undo_removes_it() {
    let mut doc = document(vec![], vec![]);
    let before = content(&doc);
    let upsert = function_with(
        vec![source(1), add(2, json!({}))],
        vec![data_edge(1, (1, "value"), (2, "a"))],
    );
    let done = apply_inferred(&mut doc, [upsert], &registry(None)).unwrap();
    assert!(done.issues.is_empty());
    assert_eq!(
        doc.locate_node(nid(2)),
        Some(BodyRef::Function(function_id()))
    );
    assert_eq!(type_arg(&doc, 2).as_deref(), Some("core/i64@1"));
    assert_eq!(signature_types(&doc, 2), ["core/i64@1"; 3]);

    let redo = doc.revert(done.committed.inverse).unwrap().inverse;
    assert_eq!(content(&doc), before);
    let _ = doc.revert(redo).unwrap();

    // Replacing the body again without the edge leaves the generic node ambiguous.
    let issues = commit(
        &mut doc,
        vec![function_with(vec![source(1), add(2, json!({}))], vec![])],
    );
    assert_eq!(codes(&issues.unwrap()), ["E-TYPE-AMBIGUOUS"]);
    assert_eq!(type_arg(&doc, 2), None);
}

#[test]
fn edits_inside_a_function_body_are_inferred_like_root_edits() {
    let mut doc = document(vec![], vec![]);
    let _ = commit(
        &mut doc,
        vec![function_with(vec![source(1), add(2, json!({}))], vec![])],
    );
    let issues = commit(
        &mut doc,
        vec![connect(data_edge(1, (1, "value"), (2, "a")))],
    )
    .unwrap();
    assert!(issues.is_empty());
    assert_eq!(type_arg(&doc, 2).as_deref(), Some("core/i64@1"));
}

fn variable_get(n: u8, variable: u8) -> Json {
    json!({"id":node_id(n),"definition_id":"core/get-variable","definition_version":1,
        "config_version":1,"config":{"variable":node_id(variable)}})
}

fn variable_update(n: u8, variable: u8) -> Json {
    json!({"id":node_id(n),"definition_id":"core/update-variable","definition_version":1,
        "config_version":1,"config":{"variable":node_id(variable),"operation":"add"}})
}

fn variable_decl(id: u8, ty: &str) -> orange_model::document::VariableDecl {
    serde_json::from_value(json!({"id":node_id(id),"name":"v","type":ty,"scope":"run"})).unwrap()
}

fn declaration_driven_document() -> GraphDocument {
    let provider = json!({"package":"org.test","version":"1.0.0","id":"native"});
    from_json(&minimal_document(json!({
        "package_lock":[{"id":"org.test","version":"1.0.0","content_digest":format!("sha256:{}", "c".repeat(64))}],
        "signatures":{"unrelated":{"kind":"pure","ports":[
            {"port_id":"a","direction":"input","kind":"data","type":"core/i32@1"},
            {"port_id":"b","direction":"input","kind":"data","type":"core/i32@1"},
            {"port_id":"sum","direction":"output","kind":"data","type":"core/i32@1","source":"computed"}
        ]}},
        "nodes":[variable_get(1,10),add(2,json!({"provider":provider})),add(4,json!({"provider":provider,"type_args":{"T":"core/i32@1"},"type_pins":{"T":"core/i32@1"},"signature":"unrelated"}))],
        "edges":[data_edge(1,(1,"value"),(2,"a"))]
    })))
    .unwrap()
}

fn declaration_registry() -> std::sync::Arc<orange_model::catalog::RegistrySnapshot> {
    common::generic::registry_with_provider(orange_model::catalog::ProviderKey {
        package: "org.test".into(),
        version: "1.0.0".into(),
        content_digest: format!("sha256:{}", "c".repeat(64)),
        id: "native".into(),
    })
}

#[test]
fn variable_changes_reinfer_referenced_generic_components_and_undo_redo_together() {
    let registry = declaration_registry();
    let mut doc = declaration_driven_document();
    let setup = apply_inferred(
        &mut doc,
        [DocCommand::UpsertVariable(variable_decl(10, "core/i64@1"))],
        &registry,
    )
    .unwrap();
    assert!(setup.issues.is_empty());
    assert_eq!(type_arg(&doc, 2).as_deref(), Some("core/i64@1"));
    assert_eq!(signature_types(&doc, 2), ["core/i64@1"; 3]);
    let before = content(&doc);
    let unrelated = doc.node(nid(4)).unwrap().clone();
    let revision = doc.revision();

    let changed = apply_inferred(
        &mut doc,
        [DocCommand::UpsertVariable(variable_decl(10, "core/f64@1"))],
        &registry,
    )
    .unwrap();
    assert!(changed.issues.is_empty(), "{:?}", changed.issues);
    assert_eq!(doc.revision(), revision + 1);
    assert_eq!(type_arg(&doc, 2).as_deref(), Some("core/f64@1"));
    assert_eq!(signature_types(&doc, 2), ["core/f64@1"; 3]);
    assert_eq!(doc.node(nid(4)).unwrap(), &unrelated);
    let resolved = orange_model::validation::validate_nodes(&doc, &registry);
    assert!(resolved.is_valid(), "{:?}", resolved.diagnostics);
    assert_eq!(
        resolved.resolved[&nid(2)]
            .ports
            .iter()
            .map(|port| port.ty.as_ref().unwrap().to_string())
            .collect::<Vec<_>>(),
        signature_types(&doc, 2)
    );
    let after = content(&doc);

    let redo = doc.revert(changed.committed.inverse).unwrap().inverse;
    assert_eq!(content(&doc), before);
    assert_eq!(type_arg(&doc, 2).as_deref(), Some("core/i64@1"));
    assert_eq!(signature_types(&doc, 2), ["core/i64@1"; 3]);
    let _ = doc.revert(redo).unwrap();
    assert_eq!(content(&doc), after);
    assert_eq!(type_arg(&doc, 2).as_deref(), Some("core/f64@1"));
}

#[test]
fn deleting_and_readding_a_referenced_variable_does_not_reuse_stale_type_args() {
    let registry = declaration_registry();
    let mut doc = declaration_driven_document();
    let _ = apply_inferred(
        &mut doc,
        [DocCommand::UpsertVariable(variable_decl(10, "core/i64@1"))],
        &registry,
    )
    .unwrap();
    assert_eq!(type_arg(&doc, 2).as_deref(), Some("core/i64@1"));

    let removed = apply_inferred(
        &mut doc,
        [DocCommand::RemoveVariable(node_id(10).parse().unwrap())],
        &registry,
    )
    .unwrap();
    assert_eq!(codes(&removed.issues), ["E-TYPE-AMBIGUOUS"]);
    assert_eq!(type_arg(&doc, 2), None);
    assert_eq!(signature_types(&doc, 2), ["$T"; 3]);

    let restored = apply_inferred(
        &mut doc,
        [DocCommand::UpsertVariable(variable_decl(10, "core/f64@1"))],
        &registry,
    )
    .unwrap();
    assert!(restored.issues.is_empty());
    assert_eq!(type_arg(&doc, 2).as_deref(), Some("core/f64@1"));
    assert_eq!(signature_types(&doc, 2), ["core/f64@1"; 3]);
}

#[test]
fn declaration_references_inside_function_bodies_are_invalidated() {
    let registry = declaration_registry();
    let mut doc = from_json(&minimal_document(json!({
        "package_lock":[{"id":"org.test","version":"1.0.0","content_digest":format!("sha256:{}", "c".repeat(64))}],
        "variables":[{"id":node_id(10),"name":"v","type":"core/i64@1","scope":"run"}]
    })))
    .unwrap();
    let _ = apply_inferred(
        &mut doc,
        [function_with(
            vec![
                variable_get(11, 10),
                add(
                    12,
                    json!({"provider":{"package":"org.test","version":"1.0.0","id":"native"}}),
                ),
            ],
            vec![data_edge(11, (11, "value"), (12, "a"))],
        )],
        &registry,
    )
    .unwrap();
    assert_eq!(type_arg(&doc, 12).as_deref(), Some("core/i64@1"));

    let changed = apply_inferred(
        &mut doc,
        [DocCommand::UpsertVariable(variable_decl(10, "core/f64@1"))],
        &registry,
    )
    .unwrap();
    assert!(changed.issues.is_empty(), "{:?}", changed.issues);
    assert_eq!(type_arg(&doc, 12).as_deref(), Some("core/f64@1"));
    assert_eq!(signature_types(&doc, 12), ["core/f64@1"; 3]);
}

#[test]
fn declaration_change_preserves_an_incompatible_explicit_pin_and_reports_the_conflict() {
    let registry = declaration_registry();
    let provider = json!({"package":"org.test","version":"1.0.0","id":"native"});
    let mut doc = from_json(&minimal_document(json!({
        "package_lock":[{"id":"org.test","version":"1.0.0","content_digest":format!("sha256:{}", "c".repeat(64))}],
        "variables":[{"id":node_id(10),"name":"v","type":"core/i64@1","scope":"run"}],
        "nodes":[variable_get(1,10),add(2,json!({"provider":provider,"type_pins":{"T":"core/i64@1"}}))],
        "edges":[data_edge(1,(1,"value"),(2,"a"))]
    })))
    .unwrap();
    let changed = apply_inferred(
        &mut doc,
        [DocCommand::UpsertVariable(variable_decl(10, "core/f64@1"))],
        &registry,
    )
    .unwrap();
    assert_eq!(codes(&changed.issues), ["E-TYPE-MISMATCH"]);
    assert_eq!(
        doc.node(nid(2)).unwrap().type_pins["T"].as_str(),
        "core/i64@1"
    );
    assert_eq!(type_arg(&doc, 2), None);
    assert_eq!(signature_types(&doc, 2), ["$T"; 3]);
}

#[test]
fn update_outputs_invalidate_generic_consumers_in_root_and_function_bodies() {
    use orange_model::validation::{validate_connections, validate_nodes};

    let registry = declaration_registry();
    for in_function in [false, true] {
        let mut fixture = serde_json::to_value(declaration_driven_document()).unwrap();
        let node = fixture["nodes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|node| node["id"] == node_id(1))
            .unwrap();
        *node = variable_update(1, 10);
        fixture["edges"] = json!([
            data_edge(1, (1, "new"), (2, "a")),
            data_edge(2, (1, "old"), (2, "b"))
        ]);
        if in_function {
            let nodes = fixture["nodes"].take();
            let edges = fixture["edges"].take();
            fixture["nodes"] = json!([]);
            fixture["edges"] = json!([]);
            fixture["functions"] = json!([{
                "id": function_id(), "name": "update", "kind": "action",
                "nodes": nodes, "edges": edges
            }]);
        }
        let mut doc: GraphDocument = serde_json::from_value(fixture).unwrap();
        let setup = apply_inferred(
            &mut doc,
            [DocCommand::UpsertVariable(variable_decl(10, "core/i64@1"))],
            &registry,
        )
        .unwrap();
        assert!(setup.issues.is_empty(), "{:?}", setup.issues);
        assert_eq!(type_arg(&doc, 2).as_deref(), Some("core/i64@1"));
        assert_eq!(signature_types(&doc, 2), ["core/i64@1"; 3]);
        let unrelated = doc.node(nid(4)).unwrap().clone();
        let before = content(&doc);
        let revision = doc.revision();

        let changed = apply_inferred(
            &mut doc,
            [DocCommand::UpsertVariable(variable_decl(10, "core/f64@1"))],
            &registry,
        )
        .unwrap();
        assert!(changed.issues.is_empty(), "{:?}", changed.issues);
        assert_eq!(doc.revision(), revision + 1);
        assert_eq!(type_arg(&doc, 2).as_deref(), Some("core/f64@1"));
        assert_eq!(signature_types(&doc, 2), ["core/f64@1"; 3]);
        assert_eq!(doc.node(nid(4)).unwrap(), &unrelated);
        assert!(doc.node(nid(1)).unwrap().signature.is_none());
        let nodes = validate_nodes(&doc, &registry);
        assert!(nodes.is_valid(), "{:?}", nodes.diagnostics);
        for port in &nodes.resolved[&nid(1)].ports {
            assert_eq!(port.ty.as_ref().unwrap().as_str(), "core/f64@1");
        }
        let connections = validate_connections(&doc, &nodes.resolved, &registry);
        assert!(
            connections.diagnostics.is_empty(),
            "{:?}",
            connections.diagnostics
        );
        // This tests type propagation, not the absent input value or execution/region rules.
        let after = content(&doc);
        let redo = doc.revert(changed.committed.inverse).unwrap().inverse;
        assert_eq!(content(&doc), before);
        assert_eq!(type_arg(&doc, 2).as_deref(), Some("core/i64@1"));
        let _ = doc.revert(redo).unwrap();
        assert_eq!(content(&doc), after);

        let removed = apply_inferred(
            &mut doc,
            [DocCommand::RemoveVariable(node_id(10).parse().unwrap())],
            &registry,
        )
        .unwrap();
        assert_eq!(codes(&removed.issues), ["E-TYPE-AMBIGUOUS"]);
        assert_eq!(type_arg(&doc, 2), None);
        assert_eq!(signature_types(&doc, 2), ["$T"; 3]);
        let restored = apply_inferred(
            &mut doc,
            [DocCommand::UpsertVariable(variable_decl(10, "core/i64@1"))],
            &registry,
        )
        .unwrap();
        assert!(restored.issues.is_empty());
        assert_eq!(type_arg(&doc, 2).as_deref(), Some("core/i64@1"));
        assert_eq!(doc.node(nid(4)).unwrap(), &unrelated);
        let before_scope_change = content(&doc);
        let mut workspace = variable_decl(10, "core/i64@1");
        workspace.scope = orange_model::document::VarScope::Workspace;
        let scope_change =
            apply_inferred(&mut doc, [DocCommand::UpsertVariable(workspace)], &registry).unwrap();
        assert_eq!(type_arg(&doc, 2), None);
        assert!(
            validate_nodes(&doc, &registry)
                .diagnostics
                .iter()
                .any(|diagnostic| {
                    diagnostic.code == orange_model::validation::DiagnosticCode::VariableScope
                        && diagnostic.subject.node == Some(nid(1))
                })
        );
        let _ = doc.revert(scope_change.committed.inverse).unwrap();
        assert_eq!(content(&doc), before_scope_change);
    }
}

#[test]
fn compare_and_set_outputs_reinfer_consumers_and_keep_the_boolean_output() {
    let registry = declaration_registry();
    for in_function in [false, true] {
        let provider = json!({"package":"org.test","version":"1.0.0","id":"native"});
        let mut fixture = serde_json::to_value(declaration_driven_document()).unwrap();
        fixture["nodes"] = json!([
            variable_get(1, 10),
            variable_get(2, 10),
            variable_update(3, 10),
            add(4, json!({"provider":provider})),
            add(5, json!({"provider":provider}))
        ]);
        fixture["nodes"][2]["config"]["operation"] = json!("compare-and-set");
        fixture["edges"] = json!([
            data_edge(1, (1, "value"), (3, "expected")),
            data_edge(2, (2, "value"), (3, "value")),
            data_edge(3, (3, "old"), (4, "a")),
            data_edge(4, (3, "new"), (5, "a"))
        ]);
        if in_function {
            let nodes = fixture["nodes"].take();
            let edges = fixture["edges"].take();
            fixture["nodes"] = json!([]);
            fixture["edges"] = json!([]);
            fixture["functions"] = json!([{
                "id": function_id(), "name": "cas", "kind": "action",
                "nodes": nodes, "edges": edges
            }]);
        }
        let mut doc: GraphDocument = serde_json::from_value(fixture).unwrap();
        let setup = apply_inferred(
            &mut doc,
            [DocCommand::UpsertVariable(variable_decl(10, "core/i64@1"))],
            &registry,
        )
        .unwrap();
        assert!(setup.issues.is_empty(), "{:?}", setup.issues);
        assert_eq!(type_arg(&doc, 4).as_deref(), Some("core/i64@1"));
        assert_eq!(type_arg(&doc, 5).as_deref(), Some("core/i64@1"));
        let before = content(&doc);
        let revision = doc.revision();

        let changed = apply_inferred(
            &mut doc,
            [DocCommand::UpsertVariable(variable_decl(10, "core/u32@1"))],
            &registry,
        )
        .unwrap();
        assert!(changed.issues.is_empty(), "{:?}", changed.issues);
        assert_eq!(doc.revision(), revision + 1);
        assert_eq!(type_arg(&doc, 4).as_deref(), Some("core/u32@1"));
        assert_eq!(type_arg(&doc, 5).as_deref(), Some("core/u32@1"));
        let cas = doc.node(nid(3)).unwrap();
        let contract = registry.contract(&doc, cas).unwrap().unwrap();
        let output_types: Vec<_> = contract
            .ports
            .iter()
            .filter(|port| port.direction == orange_model::document::PortDirection::Output)
            .map(|port| (port.port_id.as_str(), port.ty.as_ref().unwrap().to_string()))
            .collect();
        assert_eq!(
            output_types,
            [
                ("old", "core/u32@1".to_owned()),
                ("new", "core/u32@1".to_owned()),
                ("exchanged", "core/bool@1".to_owned())
            ]
        );
        let after = content(&doc);

        let redo = doc.revert(changed.committed.inverse).unwrap().inverse;
        assert_eq!(content(&doc), before);
        assert_eq!(type_arg(&doc, 4).as_deref(), Some("core/i64@1"));
        assert_eq!(type_arg(&doc, 5).as_deref(), Some("core/i64@1"));
        let _ = doc.revert(redo).unwrap();
        assert_eq!(content(&doc), after);
        assert_eq!(type_arg(&doc, 4).as_deref(), Some("core/u32@1"));
        assert!(doc.node(nid(3)).unwrap().signature.is_none());

        // Removing Eq support removes the entire contextual contract, including exchanged.
        // Existing inferred consumers must not retain the previously valid declaration type.
        let lost_eq = apply_inferred(
            &mut doc,
            [DocCommand::UpsertVariable(variable_decl(10, "core/f64@1"))],
            &registry,
        )
        .unwrap();
        assert_eq!(codes(&lost_eq.issues), ["E-TYPE-AMBIGUOUS"; 2]);
        for consumer in [4, 5] {
            assert_eq!(type_arg(&doc, consumer), None);
            assert_eq!(signature_types(&doc, consumer), ["$T"; 3]);
        }
        assert_eq!(
            registry
                .contract(&doc, doc.node(nid(3)).unwrap())
                .unwrap_err(),
            orange_model::catalog::ContractError::UpdateConstraint
        );
        let _ = doc.revert(lost_eq.committed.inverse).unwrap();
        assert_eq!(content(&doc), after);
    }
}

/// Two nodes of one definition in one component, solved to different arguments, must not
/// share a projected signature.
#[test]
fn same_definition_nodes_in_one_component_keep_distinct_signatures() {
    use common::generic::{core_converters, port, registry_builder};
    use orange_model::catalog::{Determinism, NodeDefinition, Ports};
    use orange_model::document::{DefinitionRef, NodeKind, OutputSource, PortDescriptor};
    let param = |name: &str| serde_json::from_value(json!({"name": name})).unwrap();
    let registry = registry_builder(core_converters(), None)
        .definition(NodeDefinition {
            definition: DefinitionRef {
                id: "org.test/pair".parse().unwrap(),
                version: 1,
            },
            config_version: 1,
            kind: NodeKind::Pure,
            volatility: None,
            determinism: Determinism::Deterministic,
            suspends: None,
            type_params: vec![param("T"), param("U")],
            ports: Ports::Declared(vec![
                port("x", "input", "$T"),
                PortDescriptor::data_out("y", "$U".parse().unwrap(), OutputSource::Computed),
            ]),
            references: Vec::new(),
            specializations: None,
            config_schema: None,
        })
        .build()
        .unwrap();
    let pair = |n: u8, out: &str| {
        json!({"id": node_id(n), "definition_id": "org.test/pair", "definition_version": 1,
            "config_version": 1, "config": {}, "type_pins": {"U": out}})
    };
    let mut doc = document(
        vec![
            typed_source(1, "i64"),
            pair(2, "core/f64@1"),
            pair(3, "core/i32@1"),
        ],
        vec![],
    );
    let done = apply_inferred(
        &mut doc,
        [
            connect(data_edge(1, (1, "value"), (2, "x"))),
            connect(data_edge(2, (2, "y"), (3, "x"))),
        ],
        &registry,
    )
    .unwrap();
    assert!(done.issues.is_empty(), "{:?}", done.issues);
    assert_eq!(signature_types(&doc, 2), ["core/i64@1", "core/f64@1"]);
    assert_eq!(signature_types(&doc, 3), ["core/f64@1", "core/i32@1"]);
}
