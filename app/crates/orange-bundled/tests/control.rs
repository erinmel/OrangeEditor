#[path = "support/data_graph.rs"]
#[allow(dead_code)]
mod support;
use orange_compiler::{CompileError, CompileOptions, ControlOperation, compile_control};
use orange_model::{analyze, from_json};
use serde_json::{Value as Json, json};
use support::*;

fn core(n: u32, name: &str) -> Json {
    let mut node = json!({"id":id(n),"definition_id":format!("core/{name}"),"definition_version":1,"config_version":1,"config":{}});
    if matches!(name, "on-start" | "on-stop") {
        node["activation"] = json!({"mode":"queue","capacity":4,"overflow":"drop-newest","on_fault":"continue","order":0});
    }
    node
}
fn branch(n: u32, condition: bool) -> Json {
    let mut node = core(n, "branch");
    node["inputs"] = json!({"condition":{"type":"core/bool@1","value":condition}});
    node
}

#[test]
fn checked_real_entry_branch_preserves_document_and_typed_condition() {
    let (package, snapshot) = catalog();
    for condition in [false, true] {
        let doc = graph(
            &package,
            vec![core(1, "on-start"), branch(2, condition)],
            vec![edge(1, 1, "then", 2, "in")],
            vec![],
        );
        let before = orange_model::to_json(&doc).unwrap();
        let analysis = analyze(&doc, &snapshot);
        assert!(
            analysis.is_ready_for_lowering(&doc, &snapshot),
            "{:?}",
            analysis.report().diagnostics
        );
        let plan = compile_control(
            &doc,
            snapshot.clone(),
            &analysis,
            &package,
            CompileOptions::default(),
        )
        .unwrap();
        assert_eq!(plan.starts().len(), 1);
        assert!(matches!(
            plan.nodes()[1].operation(),
            ControlOperation::Branch { .. }
        ));
        assert_eq!(plan.data().exports().len(), 1);
        assert_eq!(before, orange_model::to_json(&doc).unwrap());
        assert!(plan.is_current(&doc, &snapshot));
        assert!(!format!("{plan:?}").contains("condition"));
    }
}

#[test]
fn unsupported_control_and_stale_analysis_never_reach_resolvers() {
    let (package, snapshot) = catalog();
    let resolver = Counting::new(package.clone());
    let doc = graph(
        &package,
        vec![core(1, "on-start"), core(2, "yield")],
        vec![edge(1, 1, "then", 2, "in")],
        vec![],
    );
    let analysis = analyze(&doc, &snapshot);
    assert!(matches!(
        compile_control(
            &doc,
            snapshot.clone(),
            &analysis,
            &resolver,
            CompileOptions::default()
        ),
        Err(CompileError::UnsupportedNode { .. })
    ));
    assert_eq!(resolver.pure.load(std::sync::atomic::Ordering::Relaxed), 0);
    let other = from_json(&orange_model::to_json(&doc).unwrap()).unwrap();
    assert!(matches!(
        compile_control(
            &other,
            snapshot,
            &analysis,
            &resolver,
            CompileOptions::default()
        ),
        Err(CompileError::StaleDocument)
    ));
}

fn compiled(
    nodes: Vec<Json>,
    edges: Vec<Json>,
) -> (
    orange_model::GraphDocument,
    std::sync::Arc<orange_compiler::ControlPlan>,
) {
    let (package, snapshot) = catalog();
    let doc = graph(&package, nodes, edges, vec![]);
    let analysis = analyze(&doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(&doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    let plan = compile_control(
        &doc,
        snapshot,
        &analysis,
        &package,
        CompileOptions::default(),
    )
    .unwrap();
    (doc, std::sync::Arc::new(plan))
}

#[test]
fn branch_runtime_matches_source_condition_and_keeps_active_snapshot() {
    use orange_runtime::{ControlOutcome, Run, RunOptions, RunState};
    for condition in [true, false] {
        let (doc, plan) = compiled(
            vec![
                core(1, "on-start"),
                branch(2, condition),
                branch(3, true),
                branch(4, false),
            ],
            vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 2, "true", 3, "in"),
                edge(3, 2, "false", 4, "in"),
            ],
        );
        let snapshot = plan.data().snapshot().clone();
        let mut run = Run::start(plan, &doc, &snapshot, 42, RunOptions::default()).unwrap();
        run.pump(10).unwrap();
        let visited: Vec<_> = run.trace().iter().map(|e| e.node.to_string()).collect();
        assert_eq!(
            visited,
            vec![id(1), id(2), id(if condition { 3 } else { 4 })]
        );
        assert_eq!(
            run.trace()[1].outcome,
            if condition {
                ControlOutcome::True
            } else {
                ControlOutcome::False
            }
        );
        assert_eq!(run.state(), RunState::Running);
        run.stop().unwrap();
        run.dispose().unwrap();
        assert_eq!(run.state(), RunState::Disposed);
        assert!(run.pump(1).is_err());
    }
}
