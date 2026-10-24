//! Type-only generic inference for graph-local function calls.

mod common;

use common::generic::{add, data_edge, registry, typed_sink, typed_source};
use common::{content, function_id, minimal_document, nid, node_id};
use orange_model::document::{FunctionDecl, NodeInstance};
use orange_model::inference::{InferenceIssue, apply_inferred, plan_edit};
use orange_model::{BodyRef, DocCommand, GraphDocument, from_json};
use serde_json::{Value as Json, json};

fn generic_function() -> Json {
    json!({
        "id": function_id().to_string(), "name": "identity", "kind": "pure",
        "type_params": [{"name":"T"}],
        "inputs": [{"port_id":"value", "direction":"input", "kind":"data", "type":"$T"}],
        "outputs": [{"port_id":"result", "direction":"output", "kind":"data", "type":"$T", "source":"computed"}],
        "nodes": [], "edges": []
    })
}

fn numeric_function() -> Json {
    let mut function = generic_function();
    function["type_params"][0]["constraints"] = json!(["num"]);
    function
}

fn concrete_function(ty: &str) -> Json {
    json!({
        "id": function_id().to_string(), "name": "identity", "kind": "pure",
        "type_params": [],
        "inputs": [{"port_id":"value", "direction":"input", "kind":"data", "type":ty}],
        "outputs": [{"port_id":"result", "direction":"output", "kind":"data", "type":ty, "source":"computed"}],
        "nodes": [], "edges": []
    })
}

fn call(n: u8, extra: Json) -> Json {
    let mut node = json!({
        "id": node_id(n), "definition_id":"core/call", "definition_version":1,
        "config_version":1, "config":{"function":function_id().to_string()}
    });
    node.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    node
}

fn document(nodes: Vec<Json>, edges: Vec<Json>, function: Json) -> GraphDocument {
    let mut signatures = serde_json::Map::new();
    signatures.insert(
        "stale-call".into(),
        json!({"kind":"pure","ports":[
            {"port_id":"value","direction":"input","kind":"data","type":"core/i64@1"},
            {"port_id":"result","direction":"output","kind":"data","type":"core/i64@1"}
        ]}),
    );
    for ty in ["i32", "i64", "f64", "string"] {
        signatures.insert(
            format!("s-out-{ty}"),
            json!({"kind":"pure","ports":[
                {"port_id":"value","direction":"output","kind":"data","type":format!("core/{ty}@1")}
            ]}),
        );
        signatures.insert(
            format!("s-in-{ty}"),
            json!({"kind":"pure","ports":[
                {"port_id":"value","direction":"input","kind":"data","type":format!("core/{ty}@1")}
            ]}),
        );
    }
    from_json(&minimal_document(json!({
        "functions":[function],
        "signatures":signatures,
        "nodes":nodes,
        "edges":edges
    })))
    .unwrap()
}

fn add_node(value: Json) -> DocCommand {
    let n: NodeInstance = serde_json::from_value(value).unwrap();
    DocCommand::AddNode {
        body: BodyRef::Root,
        node: n,
    }
}

fn connect(value: Json) -> DocCommand {
    DocCommand::Connect {
        edge: serde_json::from_value(value).unwrap(),
    }
}

fn type_arg(document: &GraphDocument, node: u8) -> Option<String> {
    document
        .node(nid(node))?
        .type_args
        .get("T")
        .map(ToString::to_string)
}

fn signature_types(document: &GraphDocument, node: u8) -> Vec<String> {
    let key = document
        .node(nid(node))
        .unwrap()
        .signature
        .as_ref()
        .unwrap();
    document.signatures()[key]
        .ports
        .iter()
        .filter_map(|port| port.ty.as_ref().map(ToString::to_string))
        .collect()
}

fn codes(issues: &[InferenceIssue]) -> Vec<&'static str> {
    issues.iter().map(InferenceIssue::code).collect()
}

#[test]
fn calls_infer_independent_instantiations_and_share_arguments_within_each_call() {
    let mut doc = document(
        vec![call(3, json!({})), call(4, json!({}))],
        vec![],
        generic_function(),
    );
    let committed = apply_inferred(
        &mut doc,
        [
            add_node(typed_source(1, "i64")),
            add_node(typed_source(2, "string")),
            connect(data_edge(1, (1, "value"), (3, "value"))),
            connect(data_edge(2, (2, "value"), (4, "value"))),
            add_node(typed_sink(5, "i64", json!({}))),
            add_node(typed_sink(6, "string", json!({}))),
            connect(data_edge(3, (3, "result"), (5, "value"))),
            connect(data_edge(4, (4, "result"), (6, "value"))),
        ],
        &registry(None),
    )
    .unwrap();
    assert!(committed.issues.is_empty(), "{:?}", committed.issues);
    assert_eq!(type_arg(&doc, 3).as_deref(), Some("core/i64@1"));
    assert_eq!(type_arg(&doc, 4).as_deref(), Some("core/string@1"));
    assert!(doc.node(nid(3)).unwrap().signature.is_none());
    assert!(
        doc.signatures()
            .get(&"stale-call".parse().unwrap())
            .is_some()
    );
}

#[test]
fn call_pins_ambiguity_and_constraints_resolve_the_call_scope() {
    let mut doc = document(vec![call(3, json!({}))], vec![], generic_function());
    let pinned = apply_inferred(
        &mut doc,
        [DocCommand::SetTypePins {
            node: nid(3),
            pins: [("T".into(), "core/i32@1".parse().unwrap())].into(),
        }],
        &registry(None),
    )
    .unwrap();
    assert!(pinned.issues.is_empty(), "{:?}", pinned.issues);
    assert_eq!(type_arg(&doc, 3).as_deref(), Some("core/i32@1"));

    let unpinned = apply_inferred(
        &mut doc,
        [DocCommand::SetTypePins {
            node: nid(3),
            pins: Default::default(),
        }],
        &registry(None),
    )
    .unwrap();
    assert_eq!(codes(&unpinned.issues), ["E-TYPE-AMBIGUOUS"]);
    assert_eq!(type_arg(&doc, 3), None);

    // A typed literal is evidence too, then the function's declared `num` constraint is
    // checked against the call-site specialization.
    let mut literal_doc = document(
        vec![call(
            3,
            json!({"inputs":{"value":{"type":"core/string@1", "value":"x"}}}),
        )],
        vec![],
        numeric_function(),
    );
    let result = apply_inferred(
        &mut literal_doc,
        [DocCommand::SetTypePins {
            node: nid(3),
            pins: [("T".into(), "core/string@1".parse().unwrap())].into(),
        }],
        &registry(None),
    )
    .unwrap();
    assert_eq!(codes(&result.issues), ["E-CONSTRAINT-UNSATISFIED"]);
}

#[test]
fn incompatible_call_input_and_output_evidence_is_reported() {
    let mut doc = document(
        vec![call(3, json!({}))],
        vec![
            serde_json::from_value(data_edge(1, (1, "value"), (3, "value"))).unwrap(),
            serde_json::from_value(data_edge(2, (3, "result"), (2, "value"))).unwrap(),
        ],
        numeric_function(),
    );
    let result = apply_inferred(
        &mut doc,
        [
            add_node(typed_source(1, "i64")),
            add_node(typed_sink(2, "f64", json!({}))),
        ],
        &registry(None),
    )
    .unwrap();
    assert!(
        codes(&result.issues).contains(&"E-TYPE-MISMATCH"),
        "{:?}",
        result.issues
    );
    assert_eq!(type_arg(&doc, 3), None);
}

#[test]
fn function_interface_edits_clear_stale_call_arguments_and_undo_atomically() {
    let mut doc = document(
        vec![call(3, json!({"type_args":{"T":"core/i64@1"}}))],
        vec![],
        generic_function(),
    );
    let _ = apply_inferred(
        &mut doc,
        [
            add_node(typed_source(1, "i64")),
            connect(data_edge(1, (1, "value"), (3, "value"))),
        ],
        &registry(None),
    )
    .unwrap();
    assert_eq!(type_arg(&doc, 3).as_deref(), Some("core/i64@1"));
    let before = content(&doc);
    let changed = apply_inferred(
        &mut doc,
        [DocCommand::UpsertFunction(
            serde_json::from_value::<FunctionDecl>(concrete_function("core/i64@1")).unwrap(),
        )],
        &registry(None),
    )
    .unwrap();
    assert_eq!(type_arg(&doc, 3), None);
    let after = content(&doc);
    let redo = doc.revert(changed.committed.inverse).unwrap().inverse;
    assert_eq!(content(&doc), before);
    let _ = doc.revert(redo).unwrap();
    assert_eq!(content(&doc), after);
}

fn caller_function() -> Json {
    json!({
        "id":"50000000-0000-4000-8000-000000000002", "name":"caller", "kind":"action",
        "type_params":[], "inputs":[], "outputs":[],
        "nodes":[call(4, json!({})), typed_source(5, "i64"), add(7, json!({}))],
        "edges":[
            data_edge(4, (5, "value"), (4, "value")),
            data_edge(5, (4, "result"), (7, "a"))
        ]
    })
}

#[test]
fn function_declaration_remove_and_readd_reinfers_root_and_function_callers() {
    let mut doc = document(vec![call(3, json!({}))], vec![], generic_function());
    let registry = registry(None);
    let _ = apply_inferred(
        &mut doc,
        [
            add_node(typed_source(1, "i64")),
            add_node(add(6, json!({}))),
            connect(data_edge(1, (1, "value"), (3, "value"))),
            connect(data_edge(3, (3, "result"), (6, "a"))),
            DocCommand::UpsertFunction(
                serde_json::from_value::<FunctionDecl>(caller_function()).unwrap(),
            ),
        ],
        &registry,
    )
    .unwrap();
    assert_eq!(type_arg(&doc, 3).as_deref(), Some("core/i64@1"));
    assert_eq!(type_arg(&doc, 4).as_deref(), Some("core/i64@1"));
    assert_eq!(type_arg(&doc, 6).as_deref(), Some("core/i64@1"));
    assert_eq!(type_arg(&doc, 7).as_deref(), Some("core/i64@1"));

    let before_remove = content(&doc);
    let removed = apply_inferred(
        &mut doc,
        [DocCommand::RemoveFunction(function_id())],
        &registry,
    )
    .unwrap();
    assert!(doc.node(nid(3)).unwrap().type_args.is_empty());
    assert!(doc.node(nid(4)).unwrap().type_args.is_empty());
    assert!(doc.node(nid(6)).unwrap().type_args.is_empty());
    assert!(doc.node(nid(7)).unwrap().type_args.is_empty());
    assert_eq!(doc.node(nid(3)).unwrap().signature, None);
    assert_eq!(doc.node(nid(4)).unwrap().signature, None);
    assert_eq!(signature_types(&doc, 6), ["$T"; 3]);
    assert_eq!(signature_types(&doc, 7), ["$T"; 3]);
    let after_remove = content(&doc);

    let restored = apply_inferred(
        &mut doc,
        [DocCommand::UpsertFunction(
            serde_json::from_value::<FunctionDecl>(generic_function()).unwrap(),
        )],
        &registry,
    )
    .unwrap();
    assert_eq!(type_arg(&doc, 3).as_deref(), Some("core/i64@1"));
    assert_eq!(type_arg(&doc, 4).as_deref(), Some("core/i64@1"));
    assert_eq!(type_arg(&doc, 6).as_deref(), Some("core/i64@1"));
    assert_eq!(type_arg(&doc, 7).as_deref(), Some("core/i64@1"));
    let _ = doc.revert(restored.committed.inverse).unwrap();
    assert_eq!(content(&doc), after_remove);
    let _ = doc.revert(removed.committed.inverse).unwrap();
    assert_eq!(content(&doc), before_remove);
}

#[test]
fn missing_target_never_falls_back_to_a_saved_call_signature() {
    let mut doc = document(
        vec![
            call(
                3,
                json!({"signature":"stale-call", "type_args":{"T":"core/i64@1"}}),
            ),
            add(6, json!({"type_args":{"T":"core/i64@1"}})),
        ],
        vec![data_edge(1, (3, "result"), (6, "a"))],
        generic_function(),
    );
    let plan = plan_edit(
        &doc,
        [DocCommand::SetConfig {
            node: nid(3),
            config_version: 1,
            config: json!({"function":"50000000-0000-4000-8000-000000000099"})
                .as_object()
                .unwrap()
                .clone(),
        }],
        &registry(None),
    )
    .unwrap();
    assert!(plan.commands.iter().any(|command| matches!(command,
        DocCommand::SetTypeArgs { node, args } if *node == nid(3) && args.is_empty()
    )));
    let _ = apply_inferred(
        &mut doc,
        [DocCommand::SetConfig {
            node: nid(3),
            config_version: 1,
            config: json!({"function":"50000000-0000-4000-8000-000000000099"})
                .as_object()
                .unwrap()
                .clone(),
        }],
        &registry(None),
    )
    .unwrap();
    assert!(
        doc.node(nid(3)).unwrap().signature.is_some(),
        "historical signature bytes are preserved"
    );
    assert!(doc.node(nid(3)).unwrap().type_args.is_empty());
    assert!(doc.node(nid(6)).unwrap().type_args.is_empty());
    assert_eq!(signature_types(&doc, 6), ["$T"; 3]);
}

#[test]
fn nested_call_parameters_infer_from_output_context_without_touching_other_calls() {
    let mut function = generic_function();
    function["inputs"][0]["type"] = json!("core/list@1<$T>");
    let mut doc = document(
        vec![
            call(3, json!({"type_args":{"T":"core/i64@1"}})),
            call(9, json!({"type_args":{"T":"core/f64@1"}})),
        ],
        vec![],
        function,
    );
    let untouched = doc.node(nid(9)).unwrap().clone();
    let before = (content(&doc), doc.state_id(), doc.revision());
    let commands = [
        add_node(typed_sink(1, "string", json!({}))),
        connect(data_edge(1, (3, "result"), (1, "value"))),
    ];
    let plan = plan_edit(&doc, commands.clone(), &registry(None)).unwrap();
    assert!(plan.issues.is_empty());
    assert_eq!((content(&doc), doc.state_id(), doc.revision()), before);
    let _ = apply_inferred(&mut doc, commands, &registry(None)).unwrap();
    assert_eq!(type_arg(&doc, 3).as_deref(), Some("core/string@1"));
    assert_eq!(doc.node(nid(9)), Some(&untouched));
    assert!(doc.node(nid(3)).unwrap().signature.is_none());

    let conflict = apply_inferred(
        &mut doc,
        [DocCommand::SetLiteral {
            port: common::port(3, "value"),
            literal: Some(orange_model::document::literal::Literal::value(
                "core/list@1<core/i64@1>".parse().unwrap(),
                json!(["1"]),
            )),
        }],
        &registry(None),
    )
    .unwrap();
    assert!(codes(&conflict.issues).contains(&"E-TYPE-MISMATCH"));
    assert_eq!(type_arg(&doc, 3), None);
    assert_eq!(doc.node(nid(9)), Some(&untouched));
}
