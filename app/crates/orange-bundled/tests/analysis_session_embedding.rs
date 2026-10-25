//! A headless editor controller uses the public session, compiler and runtime APIs.
#[path = "support/data_graph.rs"]
#[allow(dead_code)]
mod support;

use std::collections::BTreeMap;
use std::sync::Arc;

use orange_compiler::{CompileError, compile_control};
use orange_model::document::{NodeInstance, PortRef, VariableDecl};
use orange_model::validation::{AnalysisSession, SessionWork};
use orange_model::{BodyRef, DocCommand, LayoutCommand, TxError, Value};
use orange_runtime::{DataError, Run, RunError};
use serde_json::{Value as Json, json};
use support::*;

fn core(n: u32, name: &str) -> Json {
    let mut node = json!({"id":id(n), "definition_id":format!("core/{name}"),
        "definition_version":1, "config_version":1, "config":{}});
    if name == "on-start" {
        node["activation"] = json!({"mode":"queue", "capacity":4,
            "overflow":"drop-newest", "on_fault":"continue", "order":0});
    }
    node
}

fn drive_to_idle(run: &mut Run) {
    for _ in 0..32 {
        let progress = run.pump(64).unwrap();
        if !progress.ready {
            if let Some(at) = progress.next_wakeup {
                run.advance_to(at).unwrap();
            } else {
                return;
            }
        }
    }
    panic!("the finite wait/set graph did not become idle");
}

#[test]
fn editing_preserves_a_live_run_and_rejects_late_compiler_input() {
    let (package, registry) = catalog();
    let mut wait = core(2, "wait");
    wait["inputs"] = json!({"duration":{"type":"core/duration@1", "value":"10"}});
    let mut set = core(4, "set-variable");
    set["config"] = json!({"variable":id(100)});
    let mut document = graph(
        &package,
        vec![core(1, "on-start"), wait, math(3, "add", 2, 3), set],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 4, "in"),
            edge(3, 3, "sum", 4, "value"),
        ],
        vec![],
    );
    let variable: VariableDecl = serde_json::from_value(json!({"id":id(100),
        "name":"result", "type":"core/u64@1", "scope":"run", "init":literal(0)}))
    .unwrap();
    let _ = document
        .apply([DocCommand::UpsertVariable(variable)])
        .unwrap();
    let mut session =
        AnalysisSession::new(&document, &registry, BTreeMap::new(), Default::default());
    let background_result = session.materialize();
    let old_plan = Arc::new(
        compile_control(
            &document,
            registry.clone(),
            &background_result,
            &package,
            Default::default(),
        )
        .unwrap(),
    );
    let mut old_run = Run::start(
        old_plan.clone(),
        &document,
        &registry,
        42,
        Default::default(),
    )
    .unwrap();

    let layout = document
        .apply([DocCommand::Layout(LayoutCommand::Set {
            path: vec!["viewport".into()],
            value: Some(json!({"zoom":2})),
        })])
        .unwrap();
    assert_eq!(
        session
            .update(&document, &registry, &layout.change_set, Default::default())
            .work,
        SessionWork::None
    );
    assert!(session.is_current(&document, &registry));
    assert!(background_result.is_ready_for_lowering(&document, &registry));
    assert!(old_plan.is_current(&document, &registry));

    let edit = document
        .apply([DocCommand::SetLiteral {
            port: serde_json::from_value::<PortRef>(json!({"node":id(3), "port":"a"})).unwrap(),
            literal: Some(serde_json::from_value(literal(20)).unwrap()),
        }])
        .unwrap();
    session.update(&document, &registry, &edit.change_set, Default::default());
    assert!(session.is_current(&document, &registry));
    assert!(!old_plan.is_current(&document, &registry));
    assert!(matches!(
        Run::start(old_plan, &document, &registry, 42, Default::default()),
        Err(RunError::Data(DataError::StaleDocument))
    ));
    assert!(matches!(
        compile_control(
            &document,
            registry.clone(),
            &background_result,
            &package,
            Default::default()
        ),
        Err(CompileError::StaleDocument)
    ));

    // This run was started before the edit. Its checked, pinned plan remains usable.
    drive_to_idle(&mut old_run);
    assert_eq!(
        old_run.variable(id(100).parse().unwrap()),
        Some(&Value::U64(5))
    );
    assert_eq!(old_run.now(), 10);
    let new_plan = Arc::new(
        compile_control(
            &document,
            registry.clone(),
            &session.materialize(),
            &package,
            Default::default(),
        )
        .unwrap(),
    );
    let mut new_run = Run::start(new_plan, &document, &registry, 42, Default::default()).unwrap();
    drive_to_idle(&mut new_run);
    assert_eq!(
        new_run.variable(id(100).parse().unwrap()),
        Some(&Value::U64(23))
    );
    assert_eq!(new_run.now(), 10);
}

#[test]
fn failed_editor_trial_keeps_document_analysis_and_retained_index() {
    let (package, registry) = catalog();
    let mut document = graph(
        &package,
        vec![math(1, "add", 2, 3)],
        vec![],
        vec![export(1, "sum")],
    );
    let mut session =
        AnalysisSession::new(&document, &registry, BTreeMap::new(), Default::default());
    let before = (
        orange_model::to_json(&document).unwrap(),
        document.state_id(),
        document.semantic_state_id(),
        document.revision(),
    );
    let analysis = session.materialize();
    let index = session.index().unwrap().clone();
    let temporary: NodeInstance = serde_json::from_value(core(99, "yield")).unwrap();
    let duplicate = document.node(id(1).parse().unwrap()).unwrap().clone();
    assert_eq!(
        session.plan_edit(
            &mut document,
            [
                DocCommand::AddNode {
                    body: BodyRef::Root,
                    node: temporary
                },
                DocCommand::AddNode {
                    body: BodyRef::Root,
                    node: duplicate
                },
            ],
            &registry
        ),
        Err(TxError::NodeExists(id(1).parse().unwrap()))
    );
    assert_eq!(
        (
            orange_model::to_json(&document).unwrap(),
            document.state_id(),
            document.semantic_state_id(),
            document.revision()
        ),
        before
    );
    assert_eq!(session.analysis(), &analysis);
    assert_eq!(session.index(), Some(&index));
    assert!(session.is_current(&document, &registry));
}
