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
        vec![core(1, "on-start"), core(2, "do-once")],
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

#[test]
fn one_work_item_segments_preserve_outcomes_and_run_fuel_is_terminal() {
    use orange_runtime::{Run, RunError, RunOptions, RunState};
    let (doc, plan) = compiled(
        vec![core(1, "on-start"), branch(2, true), branch(3, false)],
        vec![edge(1, 1, "then", 2, "in"), edge(2, 2, "true", 3, "in")],
    );
    let mut traces = vec![];
    for budget in [1, 2, 1024] {
        let mut run = Run::start(
            plan.clone(),
            &doc,
            plan.data().snapshot(),
            99,
            RunOptions::default(),
        )
        .unwrap();
        while run.pump(budget).unwrap().ready {}
        traces.push(run.trace().clone());
    }
    assert_eq!(traces[0], traces[1]);
    assert_eq!(traces[1], traces[2]);
    let mut options = RunOptions::default();
    options.limits.max_operations = 1;
    let mut run = Run::start(plan.clone(), &doc, plan.data().snapshot(), 99, options).unwrap();
    assert_eq!(run.pump(100), Err(RunError::ResourceLimit));
    assert_eq!(run.state(), RunState::Faulted);
}

#[test]
fn suspended_data_pull_keeps_epoch_and_samples_once() {
    use orange_runtime::{DataActivation, DataOptions, DataPoll};
    let (package, snapshot) = catalog();
    let resolver = Counting::new(package.clone());
    let doc = graph(
        &package,
        vec![random(1), math(2, "add", 0, 1)],
        vec![edge(1, 1, "value", 2, "a")],
        vec![export(2, "sum")],
    );
    let plan = std::sync::Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let mut frame = DataActivation::start(
        plan.clone(),
        &doc,
        &snapshot,
        123,
        0,
        DataOptions::default(),
    )
    .unwrap();
    frame.begin_epoch().unwrap();
    assert!(matches!(
        frame.poll_export(0, &mut 0).unwrap(),
        DataPoll::Pending
    ));
    let got = loop {
        match frame.poll_export(0, &mut 1).unwrap() {
            DataPoll::Pending => {}
            DataPoll::Ready(Some(value)) => break value.outcome.unwrap().clone(),
            _ => panic!("missing export"),
        }
    };
    assert_eq!(resolver.draws.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(&got, frame.export(0).unwrap().unwrap().outcome.unwrap());
    assert_eq!(resolver.draws.load(std::sync::atomic::Ordering::Relaxed), 1);
    let mut direct =
        DataActivation::start(plan, &doc, &snapshot, 123, 0, DataOptions::default()).unwrap();
    direct.begin_epoch().unwrap();
    assert_eq!(&got, direct.export(0).unwrap().unwrap().outcome.unwrap());
}

fn state_plan(
    nodes: Vec<Json>,
    edges: Vec<Json>,
    variables: Vec<Json>,
) -> (
    orange_model::GraphDocument,
    std::sync::Arc<orange_compiler::ControlPlan>,
) {
    let (package, snapshot) = catalog();
    let source = graph(&package, nodes, edges, vec![]);
    let mut raw: Json = serde_json::from_str(&orange_model::to_json(&source).unwrap()).unwrap();
    raw["variables"] = json!(variables);
    let doc = from_json(&raw.to_string()).unwrap();
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
fn variable(n: u32, scope: &str, ty: &str, init: Option<Json>) -> Json {
    let mut v = json!({"id":id(n),"name":"sensitive display name","type":ty,"scope":scope});
    if let Some(value) = init {
        v["init"] = json!({"type":ty,"value":value});
    }
    v
}
fn var_node(n: u32, name: &str, var: u32) -> Json {
    let mut node = core(n, name);
    node["config"] = json!({"variable":id(var)});
    node
}
fn set_bool(n: u32, var: u32, value: bool) -> Json {
    let mut node = var_node(n, "set-variable", var);
    node["inputs"] = json!({"value":{"type":"core/bool@1","value":value}});
    node
}

#[test]
fn run_get_observes_committed_set_and_uninitialized_read_routes_original_fault() {
    use orange_runtime::{ControlOutcome, Run, RunOptions};
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            set_bool(2, 100, false),
            var_node(3, "get-variable", 100),
            branch(4, true),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 4, "in"),
            edge(3, 3, "value", 4, "condition"),
        ],
        vec![variable(100, "run", "core/bool@1", Some(json!(true)))],
    );
    let mut run = Run::start(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        1,
        RunOptions::default(),
    )
    .unwrap();
    while run.pump(1).unwrap().ready {}
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&orange_model::Value::Bool(false))
    );
    assert_eq!(run.trace().back().unwrap().outcome, ControlOutcome::False);
    assert!(!format!("{run:?}").contains("sensitive"));
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            branch(2, true),
            var_node(3, "get-variable", 100),
            branch(4, true),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 3, "value", 2, "condition"),
            edge(3, 2, "failed", 4, "in"),
        ],
        vec![variable(100, "run", "core/bool@1", None)],
    );
    let mut run = Run::start(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        1,
        RunOptions::default(),
    )
    .unwrap();
    run.pump(100).unwrap();
    let event = &run.trace()[1];
    assert_eq!(event.outcome, ControlOutcome::Failed);
    assert_eq!(event.fault.as_ref().unwrap().source, id(3).parse().unwrap());
    assert_eq!(
        event.fault.as_ref().unwrap().fault.code,
        "core/UninitializedVariable"
    );
    assert_eq!(run.trace().back().unwrap().node, id(4).parse().unwrap());
}

#[test]
fn local_cells_reset_per_activation_and_run_cells_do_not_leak_across_runs() {
    use orange_runtime::{ControlOutcome, Run, RunOptions};
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            set_bool(2, 100, false),
            branch(3, true),
            var_node(4, "get-variable", 100),
            core(5, "on-start"),
            branch(6, true),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 4, "value", 3, "condition"),
            edge(4, 5, "then", 6, "in"),
            edge(5, 4, "value", 6, "condition"),
        ],
        vec![variable(100, "local", "core/bool@1", Some(json!(true)))],
    );
    for _ in 0..2 {
        let mut run = Run::start(
            plan.clone(),
            &doc,
            plan.data().snapshot(),
            1,
            RunOptions::default(),
        )
        .unwrap();
        run.pump(200).unwrap();
        let outcomes: Vec<_> = run
            .trace()
            .iter()
            .filter(|e| [id(3), id(6)].contains(&e.node.to_string()))
            .map(|e| e.outcome)
            .collect();
        assert_eq!(outcomes, vec![ControlOutcome::False, ControlOutcome::True]);
        assert!(run.variable(id(100).parse().unwrap()).is_none());
    }
}

fn sequence(n: u32, order: &[u16]) -> Json {
    let mut node = core(n, "sequence");
    node["variadic"] =
        json!({"then":{"order":order,"next":order.iter().copied().max().unwrap_or(0)+1}});
    node
}

#[test]
fn sequence_uses_persisted_order_and_does_not_advance_after_unhandled_fault() {
    use orange_runtime::{ControlOutcome, Run, RunOptions};
    for initialized in [true, false] {
        let first = if initialized {
            set_bool(3, 100, false)
        } else {
            branch(3, true)
        };
        let mut edges = vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then#7", 3, "in"),
            edge(3, 2, "then#2", 4, "in"),
            edge(4, 2, "then#5", 5, "in"),
            edge(5, 6, "value", 5, "condition"),
        ];
        if !initialized {
            edges.push(edge(6, 6, "value", 3, "condition"));
        }
        let (doc, plan) = state_plan(
            vec![
                core(1, "on-start"),
                sequence(2, &[7, 2, 5]),
                first,
                set_bool(4, 100, true),
                branch(5, false),
                var_node(6, "get-variable", 100),
            ],
            edges,
            vec![variable(
                100,
                "run",
                "core/bool@1",
                if initialized { Some(json!(true)) } else { None },
            )],
        );
        let mut run = Run::start(
            plan.clone(),
            &doc,
            plan.data().snapshot(),
            1,
            RunOptions::default(),
        )
        .unwrap();
        while run.pump(1).unwrap().ready {}
        if initialized {
            assert_eq!(run.trace().back().unwrap().outcome, ControlOutcome::True);
            let writes: Vec<_> = run
                .trace()
                .iter()
                .filter(|e| [id(3), id(4)].contains(&e.node.to_string()))
                .map(|e| e.node.to_string())
                .collect();
            assert_eq!(writes, vec![id(3), id(4)]);
        } else {
            assert_eq!(run.trace().back().unwrap().outcome, ControlOutcome::Failed);
            assert!(run.variable(id(100).parse().unwrap()).is_none());
        }
    }
}

fn wait(n: u32, duration: i64) -> Json {
    let mut node = core(n, "wait");
    node["inputs"] = json!({"duration":{"type":"core/duration@1","value":duration.to_string()}});
    node
}

#[test]
fn sequence_waits_for_suspended_child_and_virtual_clock_does_no_inline_work() {
    use orange_runtime::{ControlOutcome, Run, RunError, RunOptions};
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            sequence(2, &[1, 0]),
            wait(3, 10),
            set_bool(4, 100, false),
            set_bool(5, 100, true),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then#1", 3, "in"),
            edge(3, 3, "then", 4, "in"),
            edge(4, 2, "then#0", 5, "in"),
        ],
        vec![variable(100, "run", "core/bool@1", Some(json!(false)))],
    );
    let mut run = Run::start(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        1,
        RunOptions::default(),
    )
    .unwrap();
    let progress = run.pump(200).unwrap();
    assert!(!progress.ready);
    assert_eq!(progress.next_wakeup, Some(10));
    assert_eq!(
        run.trace().back().unwrap().outcome,
        ControlOutcome::Suspended
    );
    run.advance_to(9).unwrap();
    run.pump(200).unwrap();
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&orange_model::Value::Bool(false))
    );
    let before = run.trace().len();
    run.advance_to(10).unwrap();
    assert_eq!(run.trace().len(), before);
    assert_eq!(run.advance_to(9), Err(RunError::ClockWentBackwards));
    while run.pump(1).unwrap().ready {}
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&orange_model::Value::Bool(true))
    );
    assert_eq!(
        run.trace()
            .iter()
            .filter(|e| e.node.to_string() == id(3) && e.outcome == ControlOutcome::Then)
            .count(),
        1
    );
}

#[test]
fn wait_range_faults_and_stopped_timer_cannot_resume() {
    use orange_runtime::{ControlOutcome, Run, RunOptions};
    for duration in [-1, 0, 10] {
        let (doc, plan) = compiled(
            vec![core(1, "on-start"), wait(2, duration), branch(3, true)],
            vec![edge(1, 1, "then", 2, "in"), edge(2, 2, "then", 3, "in")],
        );
        let mut run = Run::start(
            plan.clone(),
            &doc,
            plan.data().snapshot(),
            1,
            RunOptions::default(),
        )
        .unwrap();
        let progress = run.pump(200).unwrap();
        if duration < 0 {
            assert_eq!(
                run.trace()
                    .back()
                    .unwrap()
                    .fault
                    .as_ref()
                    .unwrap()
                    .fault
                    .code,
                "core/OutOfRange"
            );
        } else if duration == 0 {
            assert_eq!(run.trace().back().unwrap().outcome, ControlOutcome::True);
        } else {
            assert_eq!(progress.next_wakeup, Some(10));
            run.stop().unwrap();
            assert!(run.advance_to(10).is_err());
            assert!(run.pump(200).is_err());
        }
    }
}
