//! Regressions for malformed saved state, transaction failure and plan freshness.
#[path = "support/data_graph.rs"]
#[allow(dead_code)]
mod support;
use support::*;

use orange_model::document::{FunctionDecl, NodeInstance};
use orange_model::edit::LayoutCommand;
use orange_model::validation::DiagnosticCode;
use orange_model::{BodyRef, DocCommand, GraphDocument, analyze};
use orange_runtime::{Run, RunOptions, RunState};
use serde_json::{Value as Json, json};
use std::sync::Arc;

fn core(n: u32, name: &str) -> Json {
    let mut node = json!({"id":id(n),"definition_id":format!("core/{name}"),
        "definition_version":1,"config_version":1,"config":{}});
    if name == "on-start" {
        node["activation"] = json!({"mode":"queue","capacity":4,"overflow":"drop-newest",
            "on_fault":"continue","order":0});
    }
    node
}

fn branch(n: u32) -> Json {
    let mut node = core(n, "branch");
    node["inputs"] = json!({"condition":{"type":"core/bool@1","value":true}});
    node
}

fn control(
    nodes: Vec<Json>,
    edges: Vec<Json>,
) -> (GraphDocument, Arc<orange_compiler::ControlPlan>) {
    let (package, snapshot) = catalog();
    let doc = graph(&package, nodes, edges, vec![]);
    let analysis = analyze(&doc, &snapshot);
    let plan =
        orange_compiler::compile_control(&doc, snapshot, &analysis, &package, Default::default())
            .unwrap();
    (doc, Arc::new(plan))
}

#[test]
fn iterator_unwind_restores_document_and_freshness() {
    let mut doc = GraphDocument::new("20000000-0000-4000-8000-000000000001".parse().unwrap());
    let node: NodeInstance = serde_json::from_value(core(1, "yield")).unwrap();
    let before = (
        orange_model::to_json(&doc).unwrap(),
        doc.state_id(),
        doc.semantic_state_id(),
    );
    let index = orange_model::DocIndex::build(&doc);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let commands = std::iter::once(DocCommand::AddNode {
            body: BodyRef::Root,
            node,
        })
        .chain(std::iter::from_fn(|| -> Option<DocCommand> {
            panic!("iterator failure")
        }));
        let _ = doc.apply(commands);
    }));
    assert!(result.is_err());
    assert_eq!(
        (
            orange_model::to_json(&doc).unwrap(),
            doc.state_id(),
            doc.semantic_state_id()
        ),
        before
    );
    assert_eq!(doc.revision(), 0);
    assert!(index.is_current(&doc));
}

#[test]
fn paste_rejects_duplicate_function_node_ids_and_existing_target_ids() {
    let function: FunctionDecl = serde_json::from_value(json!({"id":id(10),"name":"F",
        "kind":"action","nodes":[core(1,"yield"),core(2,"yield")],"edges":[]}))
    .unwrap();
    let fragment = orange_model::paste::Fragment {
        functions: vec![function],
        ..Default::default()
    };
    let mut target = GraphDocument::new("20000000-0000-4000-8000-000000000001".parse().unwrap());
    let collision: orange_model::id::NodeId = id(99).parse().unwrap();
    assert!(matches!(
        fragment.paste(&target, BodyRef::Root, || collision.as_uuid()),
        Err(orange_model::error::IdError::GeneratedCollision { .. })
    ));
    let node: NodeInstance = serde_json::from_value(core(99, "yield")).unwrap();
    let _ = target
        .apply([DocCommand::AddNode {
            body: BodyRef::Root,
            node,
        }])
        .unwrap();
    let fragment = orange_model::paste::Fragment {
        nodes: vec![serde_json::from_value(core(1, "yield")).unwrap()],
        ..Default::default()
    };
    let before = orange_model::to_json(&target).unwrap();
    assert!(
        fragment
            .paste(&target, BodyRef::Root, || collision.as_uuid())
            .is_err()
    );
    assert_eq!(orange_model::to_json(&target).unwrap(), before);
}

#[test]
fn cleanup_entry_failure_retires_all_owned_work() {
    let mut wait = core(2, "wait");
    wait["inputs"] = json!({"duration":{"type":"core/duration@1","value":"100"}});
    let (doc, plan) = control(
        vec![core(1, "on-start"), wait, branch(3)],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "cancelled", 3, "in"),
        ],
    );
    let options = RunOptions {
        max_cleanup_operations: usize::MAX,
        ..Default::default()
    };
    let mut run = Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options).unwrap();
    run.pump(1000).unwrap();
    run.stop().unwrap();
    run.pump(1).unwrap();
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(run.cleanup_truncated(), 1);
    let owners = run.ownership();
    assert_eq!(
        (owners.activations, owners.strands, owners.regions),
        (0, 0, 0)
    );
    assert!(matches!(
        run.pump(1),
        Err(orange_runtime::RunError::InvalidState)
    ));
    assert_eq!(run.ownership(), owners);
}

#[test]
fn layout_preserves_analysis_and_plan_through_undo_redo() {
    let (mut doc, plan) = control(vec![core(1, "on-start")], vec![]);
    let registry = plan.data().snapshot();
    let analysis = analyze(&doc, registry);
    let content = doc.state_id();
    let semantic = doc.semantic_state_id();
    let layout = doc
        .apply([DocCommand::Layout(LayoutCommand::Set {
            path: vec!["zoom".into()],
            value: Some(json!(2)),
        })])
        .unwrap();
    assert!(layout.change_set.layout_only());
    assert_ne!(doc.state_id(), content);
    assert_eq!(doc.semantic_state_id(), semantic);
    assert!(analysis.is_ready_for_lowering(&doc, registry));
    assert!(plan.is_current(&doc, registry));
    Run::start(plan.clone(), &doc, registry, 1, Default::default()).unwrap();
    let undo = doc.revert(layout.inverse).unwrap();
    assert_eq!(doc.state_id(), content);
    let _ = doc.revert(undo.inverse).unwrap();
    assert!(plan.is_current(&doc, registry));
    let changed = doc
        .apply([DocCommand::AddNode {
            body: BodyRef::Root,
            node: serde_json::from_value(core(2, "yield")).unwrap(),
        }])
        .unwrap();
    assert!(!plan.is_current(&doc, registry));
    assert!(!analysis.is_ready_for_lowering(&doc, registry));
    let redo = doc.revert(changed.inverse).unwrap();
    assert!(plan.is_current(&doc, registry));
    let _ = doc.revert(redo.inverse).unwrap();
    assert!(!plan.is_current(&doc, registry));
    let loaded = orange_model::from_json(&orange_model::to_json(&doc).unwrap()).unwrap();
    assert_ne!(loaded.semantic_state_id(), doc.semantic_state_id());
}

#[test]
fn pure_evaluation_survives_layout_edits_but_rejects_semantic_edits() {
    let (package, snapshot) = catalog();
    let mut doc = graph(
        &package,
        vec![math(1, "add", 2, 3)],
        vec![],
        vec![export(1, "sum")],
    );
    let analysis = analyze(&doc, &snapshot);
    let plan = orange_compiler::compile_pure(
        &doc,
        snapshot.clone(),
        &analysis,
        &package,
        Default::default(),
    )
    .unwrap();
    let _ = doc
        .apply([DocCommand::Layout(LayoutCommand::Set {
            path: vec!["zoom".into()],
            value: Some(json!(2)),
        })])
        .unwrap();
    assert!(plan.is_current(&doc, &snapshot));
    assert!(plan.evaluate(&doc, &snapshot, Default::default()).is_ok());
    let _ = doc
        .apply([DocCommand::RemoveNodes {
            ids: vec![id(1).parse().unwrap()],
        }])
        .unwrap();
    assert!(!plan.is_current(&doc, &snapshot));
    assert!(matches!(
        plan.evaluate(&doc, &snapshot, Default::default()),
        Err(orange_compiler::EvaluationError::StaleDocument)
    ));
}

#[test]
fn malformed_variadic_state_is_preserved_but_never_compiled_or_allocated() {
    let (package, snapshot) = catalog();
    for name in ["sequence", "fork"] {
        for state in [
            json!({"order":[0,0],"next":1}),
            json!({"order":[0,1],"next":0}),
        ] {
            let mut node = core(2, name);
            let port = if name == "sequence" { "then" } else { "branch" };
            node["variadic"] = json!({port:state});
            let mut doc = graph(
                &package,
                vec![core(1, "on-start"), node],
                vec![edge(1, 1, "then", 2, "in")],
                vec![],
            );
            let before = orange_model::to_json(&doc).unwrap();
            let analysis = analyze(&doc, &snapshot);
            assert!(
                analysis
                    .report()
                    .diagnostics
                    .iter()
                    .any(|d| d.code == DiagnosticCode::VariadicIdentity)
            );
            assert!(!analysis.is_ready_for_lowering(&doc, &snapshot));
            assert!(
                orange_compiler::compile_control(
                    &doc,
                    snapshot.clone(),
                    &analysis,
                    &package,
                    Default::default()
                )
                .is_err()
            );
            assert!(matches!(
                doc.apply([DocCommand::InsertVariadic {
                    node: id(2).parse().unwrap(),
                    port: port.parse().unwrap(),
                    at: 0
                }]),
                Err(orange_model::error::TxError::InvalidVariadicState { .. })
            ));
            assert_eq!(orange_model::to_json(&doc).unwrap(), before);
        }
    }
}

#[test]
fn explicit_codec_locks_gate_lowering_without_rewriting_saved_payloads() {
    let (package, snapshot) = catalog();
    let source = graph(
        &package,
        vec![core(1, "on-start"), branch(2)],
        vec![edge(1, 1, "then", 2, "in")],
        vec![],
    );
    let mut raw: Json = serde_json::from_str(&orange_model::to_json(&source).unwrap()).unwrap();
    for codec in ["unknown.future.codec.v999", "orange.value.v1"] {
        raw["type_lock"] = json!([{"type":"core/bool@1","codec":codec}]);
        let doc = orange_model::from_json(&raw.to_string()).unwrap();
        let analysis = analyze(&doc, &snapshot);
        let supported = codec == "orange.value.v1";
        assert_eq!(analysis.is_ready_for_lowering(&doc, &snapshot), supported);
        assert_eq!(
            analysis
                .report()
                .diagnostics
                .iter()
                .any(|d| d.code == DiagnosticCode::TypeCodec),
            !supported
        );
        assert_eq!(
            orange_compiler::compile_control(
                &doc,
                snapshot.clone(),
                &analysis,
                &package,
                Default::default()
            )
            .is_ok(),
            supported
        );
        let saved: Json = serde_json::from_str(&orange_model::to_json(&doc).unwrap()).unwrap();
        assert_eq!(saved["type_lock"], raw["type_lock"]);
        assert_eq!(saved["nodes"], raw["nodes"]);
    }
}

#[test]
fn deeply_nested_option_list_literals_round_trip() {
    use orange_model::document::literal::{Literal, LiteralPayload};
    use orange_model::value::literal_codec::{SchemaSet, decode, encode};
    let schemas = SchemaSet::default();
    for depth in [1, 32, 63] {
        let ty = format!(
            "core/list@1<{}core/i64@1{}>",
            "core/option@1<".repeat(depth),
            ">".repeat(depth)
        );
        let literal = Literal {
            ty: ty.parse().unwrap(),
            payload: LiteralPayload::Value(json!([{"none":true},{"none":true}])),
            display: None,
        };
        let value = decode(&literal, &schemas).unwrap();
        assert_eq!(encode(&literal.ty, &value, &schemas).unwrap(), literal);
    }
}

#[test]
#[ignore = "manual release timing probe; no wall-clock assertion"]
fn measure_nested_none_literal_decoding() {
    use orange_model::document::literal::{Literal, LiteralPayload};
    use orange_model::value::literal_codec::{SchemaSet, decode};
    let schemas = SchemaSet::default();
    for depth in [1, 32, 63] {
        let ty = format!(
            "core/list@1<{}core/i64@1{}>",
            "core/option@1<".repeat(depth),
            ">".repeat(depth)
        );
        let literal = Literal {
            ty: ty.parse().unwrap(),
            payload: LiteralPayload::Value(Json::Array(vec![json!({"none": true}); 10_000])),
            display: None,
        };
        for _ in 0..3 {
            std::hint::black_box(decode(&literal, &schemas).unwrap());
        }
        let mut samples = Vec::new();
        for _ in 0..5 {
            let start = std::time::Instant::now();
            std::hint::black_box(decode(&literal, &schemas).unwrap());
            samples.push(start.elapsed().as_micros());
        }
        samples.sort();
        eprintln!(
            "10,000 outer None literals: depth={depth}, median={}us, max={}us",
            samples[2], samples[4]
        );
    }
}
