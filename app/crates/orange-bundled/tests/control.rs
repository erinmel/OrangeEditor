#[path = "support/control_reference.rs"]
mod control_reference;
#[path = "support/control_regressions.rs"]
mod control_regressions;
#[path = "support/reference.rs"]
#[allow(dead_code)]
mod reference;
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
        let mut run = Run::start(plan.clone(), &doc, &snapshot, 42, RunOptions::default()).unwrap();
        let replacement = from_json(&orange_model::to_json(&doc).unwrap()).unwrap();
        assert!(matches!(
            Run::start(
                plan.clone(),
                &replacement,
                &snapshot,
                42,
                RunOptions::default()
            ),
            Err(orange_runtime::RunError::Data(
                orange_runtime::DataError::StaleDocument
            ))
        ));
        let (_, new_snapshot) = catalog();
        assert!(matches!(
            Run::start(plan, &doc, &new_snapshot, 42, RunOptions::default()),
            Err(orange_runtime::RunError::Data(
                orange_runtime::DataError::StaleRegistry
            ))
        ));
        // The already active run continues against its original retained snapshot.
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
    assert_eq!(run.state(), RunState::Stopped);
    assert!(run.transitions().contains(&RunState::Faulted));
    assert_eq!(run.last_error(), Some(RunError::ResourceLimit));
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

fn count(n: u32, times: i64) -> Json {
    let mut node = core(n, "count");
    node["inputs"] = json!({"count":{"type":"core/i64@1","value":times.to_string()}});
    node
}

#[test]
fn count_captures_count_and_publishes_fresh_indices_with_zero_and_negative_cases() {
    use orange_runtime::{ControlOutcome, Run, RunOptions};
    for times in [-1, 0, 1, 3] {
        let (doc, plan) = state_plan(
            vec![
                core(1, "on-start"),
                count(2, times),
                var_node(3, "set-variable", 100),
                branch(4, true),
            ],
            vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 2, "body", 3, "in"),
                edge(3, 2, "index", 3, "value"),
                edge(4, 2, "completed", 4, "in"),
            ],
            vec![variable(100, "run", "core/i64@1", None)],
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
            run.variable(id(100).parse().unwrap()).cloned(),
            if times > 0 {
                Some(orange_model::Value::I64(times - 1))
            } else {
                None
            }
        );
        if times < 0 {
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
        } else {
            assert_eq!(run.trace().back().unwrap().outcome, ControlOutcome::True);
        }
    }
}

#[test]
fn break_and_continue_retire_remaining_sequence_steps_inside_loop_body() {
    use orange_runtime::{ControlOutcome, Run, RunOptions};
    for control in ["break", "continue"] {
        let (doc, plan) = state_plan(
            vec![
                core(1, "on-start"),
                count(2, 3),
                sequence(3, &[0, 1]),
                core(4, control),
                set_bool(5, 100, false),
                branch(6, true),
            ],
            vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 2, "body", 3, "in"),
                edge(3, 3, "then#0", 4, "in"),
                edge(4, 3, "then#1", 5, "in"),
                edge(5, 2, "completed", 6, "in"),
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
        run.pump(300).unwrap();
        assert_eq!(
            run.variable(id(100).parse().unwrap()),
            Some(&orange_model::Value::Bool(true))
        );
        assert_eq!(
            run.trace()
                .iter()
                .filter(|e| e.node.to_string() == id(2) && e.outcome == ControlOutcome::Body)
                .count(),
            if control == "break" { 1 } else { 3 }
        );
        assert_eq!(run.trace().back().unwrap().node.to_string(), id(6));
    }
}

#[test]
fn count_body_waits_before_advancing_iteration_and_accumulates_three() {
    use orange_runtime::{Run, RunOptions};
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            count(2, 3),
            wait(3, 5),
            var_node(4, "get-variable", 100),
            math(5, "add", 0, 1),
            var_node(6, "set-variable", 100),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 3, "in"),
            edge(3, 3, "then", 6, "in"),
            edge(4, 4, "value", 5, "a"),
            edge(5, 5, "sum", 6, "value"),
        ],
        vec![variable(100, "run", "core/u64@1", Some(json!("0")))],
    );
    let mut run = Run::start(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        1,
        RunOptions::default(),
    )
    .unwrap();
    for iteration in 0..3 {
        let progress = run.pump(300).unwrap();
        assert_eq!(progress.next_wakeup, Some((iteration + 1) * 5));
        assert_eq!(
            run.variable(id(100).parse().unwrap()),
            Some(&orange_model::Value::U64(iteration))
        );
        run.advance_to((iteration + 1) * 5).unwrap();
        // Use one item to deliver the completion; body execution remains scheduled.
        run.pump(1).unwrap();
    }
    run.pump(300).unwrap();
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&orange_model::Value::U64(3))
    );
}

fn while_node(n: u32, condition: bool, limit: u64) -> Json {
    let mut node = core(n, "while");
    node["config"] = json!({"max_iterations":limit});
    node["inputs"] = json!({"condition":{"type":"core/bool@1","value":condition}});
    node
}

#[test]
fn while_rechecks_committed_state_and_enforces_limit_after_last_condition() {
    use orange_runtime::{ControlOutcome, Run, RunOptions};
    for initial in [false, true] {
        let (doc, plan) = state_plan(
            vec![
                core(1, "on-start"),
                while_node(2, true, 1),
                set_bool(3, 100, false),
                var_node(4, "get-variable", 100),
                branch(5, true),
            ],
            vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 2, "body", 3, "in"),
                edge(3, 4, "value", 2, "condition"),
                edge(4, 2, "completed", 5, "in"),
            ],
            vec![variable(100, "run", "core/bool@1", Some(json!(initial)))],
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
            run.trace()
                .iter()
                .filter(|e| e.node.to_string() == id(2) && e.outcome == ControlOutcome::Body)
                .count(),
            usize::from(initial)
        );
        assert_eq!(run.trace().back().unwrap().node.to_string(), id(5));
    }
    let (doc, plan) = compiled(
        vec![core(1, "on-start"), while_node(2, true, 3)],
        vec![edge(1, 1, "then", 2, "in")],
    );
    let mut run = Run::start(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        1,
        RunOptions::default(),
    )
    .unwrap();
    run.pump(200).unwrap();
    assert_eq!(
        run.trace()
            .iter()
            .filter(|e| e.outcome == ControlOutcome::Body)
            .count(),
        3
    );
    assert_eq!(
        run.trace()
            .back()
            .unwrap()
            .fault
            .as_ref()
            .unwrap()
            .fault
            .code,
        "core/LoopBudgetExceeded"
    );
}

#[test]
fn nested_break_exits_inner_while_and_outer_count_keeps_progressing() {
    use orange_runtime::{ControlOutcome, Run, RunOptions};
    let (doc, plan) = compiled(
        vec![
            core(1, "on-start"),
            count(2, 2),
            while_node(3, true, 10),
            core(4, "break"),
            branch(5, true),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 3, "in"),
            edge(3, 3, "body", 4, "in"),
            edge(4, 2, "completed", 5, "in"),
        ],
    );
    let mut run = Run::start(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        1,
        RunOptions::default(),
    )
    .unwrap();
    while run.pump(2).unwrap().ready {}
    assert_eq!(
        run.trace()
            .iter()
            .filter(|e| e.node.to_string() == id(4) && e.outcome == ControlOutcome::Break)
            .count(),
        2
    );
    assert_eq!(run.trace().back().unwrap().node.to_string(), id(5));
}

#[test]
fn graceful_stop_runs_on_stop_once_then_drains_waiting_work() {
    use orange_runtime::{Run, RunOptions, RunState, StopMode};
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            wait(2, 100),
            set_bool(3, 100, false),
            core(4, "on-stop"),
            set_bool(5, 100, true),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 4, "then", 5, "in"),
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
    run.pump(300).unwrap();
    run.request_stop(StopMode::Graceful).unwrap();
    assert!(run.request_stop(StopMode::Graceful).is_err());
    run.pump(300).unwrap();
    assert_eq!(run.state(), RunState::Draining);
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&orange_model::Value::Bool(true))
    );
    run.advance_to(100).unwrap();
    run.pump(300).unwrap();
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(run.pending_continuations(), 0);
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&orange_model::Value::Bool(false))
    );
    assert_eq!(
        run.trace()
            .iter()
            .filter(|e| e.node.to_string() == id(4))
            .count(),
        1
    );
    assert_eq!(
        run.transitions(),
        &[
            RunState::Created,
            RunState::Starting,
            RunState::Running,
            RunState::Draining,
            RunState::Stopped
        ]
    );
}

#[test]
fn immediate_stop_routes_cancelled_to_bounded_cleanup_and_second_stop_aborts() {
    use orange_runtime::{ControlOutcome, Run, RunOptions, RunState};
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            wait(2, 100),
            set_bool(3, 100, false),
            core(4, "on-stop"),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "cancelled", 3, "in"),
        ],
        vec![variable(100, "run", "core/bool@1", Some(json!(true)))],
    );
    for abort in [false, true] {
        let mut run = Run::start(
            plan.clone(),
            &doc,
            plan.data().snapshot(),
            1,
            RunOptions::default(),
        )
        .unwrap();
        run.pump(300).unwrap();
        run.stop().unwrap();
        assert_eq!(run.state(), RunState::Cancelling);
        assert_eq!(run.pending_continuations(), 0);
        if abort {
            run.stop().unwrap();
        } else {
            while run.pump(1).unwrap().ready {}
        }
        assert_eq!(run.state(), RunState::Stopped);
        assert_eq!(
            run.variable(id(100).parse().unwrap()),
            Some(&orange_model::Value::Bool(abort))
        );
        assert_eq!(
            run.trace()
                .iter()
                .filter(|e| e.outcome == ControlOutcome::Cancelled)
                .count(),
            1
        );
        assert!(!run.trace().iter().any(|e| e.node.to_string() == id(4)));
        assert!(run.advance_to(100).is_err());
    }
}

#[test]
fn cleanup_rejects_new_waits_and_deadlines_retire_all_owners() {
    use orange_runtime::{Run, RunOptions, RunState, StopMode};
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            wait(2, 100),
            wait(3, 200),
            set_bool(4, 100, false),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "cancelled", 3, "in"),
            edge(3, 3, "failed", 4, "in"),
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
    run.pump(300).unwrap();
    run.stop().unwrap();
    run.pump(300).unwrap();
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(run.pending_continuations(), 0);
    assert!(run.trace().iter().any(|e| {
        e.fault
            .as_ref()
            .is_some_and(|f| f.fault.code == "core/Cancelled")
    }));
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&orange_model::Value::Bool(false))
    );
    let options = RunOptions {
        drain_timeout_ns: 5,
        ..RunOptions::default()
    };
    let mut run = Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options).unwrap();
    run.pump(300).unwrap();
    run.request_stop(StopMode::Graceful).unwrap();
    run.advance_to(5).unwrap();
    run.pump(300).unwrap();
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(run.pending_continuations(), 0);
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&orange_model::Value::Bool(true))
    );
    assert!(run.cleanup_truncated() > 0);
}

#[test]
fn cancellation_flag_is_observed_once_and_cleanup_has_a_separate_budget() {
    use orange_runtime::{CancelToken, Run, RunOptions, RunState};
    let (doc, plan) = state_plan(
        vec![core(1, "on-start"), wait(2, 100), set_bool(3, 100, false)],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "cancelled", 3, "in"),
        ],
        vec![variable(100, "run", "core/bool@1", Some(json!(true)))],
    );
    for fuel in [0, 20] {
        let token = CancelToken::default();
        let mut options = RunOptions::default();
        options.data.cancel = Some(token.clone());
        options.max_cleanup_operations = fuel;
        let mut run = Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options).unwrap();
        run.pump(300).unwrap();
        token.cancel();
        while run.pump(1).unwrap().ready {}
        assert_eq!(run.state(), RunState::Stopped);
        assert_eq!(
            run.variable(id(100).parse().unwrap()),
            Some(&orange_model::Value::Bool(fuel == 0))
        );
        if fuel == 0 {
            assert!(run.cleanup_truncated() > 0);
        }
    }
}

#[test]
fn failed_output_is_typed_data_with_original_producer_and_dispose_releases_plan() {
    use orange_runtime::{Run, RunOptions};
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            branch(2, true),
            var_node(3, "get-variable", 100),
            var_node(4, "set-variable", 101),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 3, "value", 2, "condition"),
            edge(3, 2, "failed", 4, "in"),
            edge(4, 2, "error", 4, "value"),
        ],
        vec![
            variable(100, "run", "core/bool@1", None),
            variable(101, "run", "core/error@1", None),
        ],
    );
    let weak = std::sync::Arc::downgrade(&plan);
    let mut run = Run::start(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        1,
        RunOptions::default(),
    )
    .unwrap();
    run.pump(300).unwrap();
    let Some(orange_model::Value::Err(error)) = run.variable(id(101).parse().unwrap()) else {
        panic!("missing typed error");
    };
    assert_eq!(error.code, "core/UninitializedVariable");
    assert_eq!(error.node, Some(id(3).parse().unwrap()));
    drop(plan);
    assert!(weak.upgrade().is_some());
    run.stop().unwrap();
    run.dispose().unwrap();
    assert!(weak.upgrade().is_none());
}

fn compare_source_control(
    doc: &orange_model::GraphDocument,
    plan: std::sync::Arc<orange_compiler::ControlPlan>,
    seed: u64,
    budget: usize,
) {
    use orange_runtime::{ControlOutcome, Run, RunOptions};
    let analysis = analyze(doc, plan.data().snapshot());
    let mut oracle =
        control_reference::ControlReference::new(doc, plan.data().snapshot(), &analysis, seed);
    oracle.run(seed);
    let mut run = Run::start(
        plan.clone(),
        doc,
        plan.data().snapshot(),
        seed,
        RunOptions::default(),
    )
    .unwrap();
    for step in 0..20_000 {
        assert!(step < 19_999, "bounded test driver");
        let progress = run.pump(budget).unwrap();
        assert!(progress.operations <= budget);
        if !progress.ready {
            if let Some(at) = progress.next_wakeup {
                run.advance_to(at).unwrap();
            } else {
                break;
            }
        }
    }
    for (id, value) in &oracle.variables {
        if doc.variables()[id].scope == orange_model::document::VarScope::Run {
            assert_eq!(run.variable(*id), value.as_ref());
        }
    }
    let observations: Vec<_> = run
        .trace()
        .iter()
        .filter_map(|e| {
            let definition = doc.root().nodes[&e.node].definition.id.as_str();
            if matches!(
                definition,
                "core/on-start" | "core/on-stop" | "core/sequence" | "core/count" | "core/while"
            ) && e.outcome != ControlOutcome::Failed
            {
                return None;
            }
            let label = match e.outcome {
                ControlOutcome::Then => "then",
                ControlOutcome::True => "true",
                ControlOutcome::False => "false",
                ControlOutcome::Failed => "failed",
                ControlOutcome::Suspended => "suspended",
                ControlOutcome::Yielded => "yielded",
                ControlOutcome::Break => "break",
                ControlOutcome::Continue => "continue",
                _ => return None,
            };
            Some((e.node, label))
        })
        .collect();
    assert_eq!(observations, oracle.observations);
    let faults: Vec<_> = run
        .trace()
        .iter()
        .filter_map(|e| e.fault.as_ref().map(|f| (e.node, f.source, f.fault.code)))
        .collect();
    assert_eq!(faults, oracle.faults);
    assert_eq!(run.now(), oracle.now);
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config { cases:96, rng_seed:proptest::test_runner::RngSeed::Fixed(20261024), ..proptest::test_runner::Config::default() })]
    #[test]
    fn nested_seeded_control_matches_independent_source_interpreter(
        outer in 0i64..5, inner in 0i64..5, selected in proptest::bool::ANY,
        continue_body in proptest::bool::ANY, delay in 0i64..4, seed in proptest::num::u64::ANY,
        reverse_storage in proptest::bool::ANY,
    ) {
        let mut nodes=vec![core(1,"on-start"),sequence(2,&[7,2]),count(3,outer),sequence(4,&[0,1,2]),count(5,inner),math(6,"add",0,0),var_node(7,"get-variable",100),random(8),branch(9,selected),var_node(10,"set-variable",100),core(11,if continue_body {"continue"} else {"break"}),while_node(12,true,1),set_bool(13,101,true),var_node(14,"get-variable",101),sequence(15,&[1,0]),wait(16,delay),set_bool(17,101,false),branch(18,true)];
        let mut edges=vec![edge(1,1,"then",2,"in"),edge(2,2,"then#7",3,"in"),edge(3,2,"then#2",18,"in"),edge(4,3,"body",4,"in"),edge(5,4,"then#0",13,"in"),edge(6,4,"then#1",5,"in"),edge(7,4,"then#2",12,"in"),edge(8,5,"body",9,"in"),edge(9,9,"true",10,"in"),edge(10,9,"false",11,"in"),edge(11,7,"value",6,"a"),edge(12,8,"value",6,"b"),edge(13,6,"sum",10,"value"),edge(14,14,"value",12,"condition"),edge(15,12,"body",15,"in"),edge(16,15,"then#1",16,"in"),edge(17,15,"then#0",17,"in")];
        if reverse_storage {nodes.reverse();edges.reverse();}
        let (doc,plan)=state_plan(nodes,edges,vec![variable(100,"run","core/u64@1",Some(json!("0"))),variable(101,"run","core/bool@1",Some(json!(false)))]);
        for budget in [1,3,64] {compare_source_control(&doc,plan.clone(),seed,budget);}
    }
}

#[test]
fn fault_and_loop_limit_known_answers_match_source_oracle() {
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            branch(2, true),
            var_node(3, "get-variable", 100),
            var_node(4, "set-variable", 101),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 3, "value", 2, "condition"),
            edge(3, 2, "failed", 4, "in"),
            edge(4, 2, "error", 4, "value"),
        ],
        vec![
            variable(100, "run", "core/bool@1", None),
            variable(101, "run", "core/error@1", None),
        ],
    );
    for budget in [1, 64] {
        compare_source_control(&doc, plan.clone(), 5, budget);
    }
    let (doc, plan) = compiled(
        vec![core(1, "on-start"), while_node(2, true, 3)],
        vec![edge(1, 1, "then", 2, "in")],
    );
    compare_source_control(&doc, plan, 7, 1);
}

#[test]
fn state_snapshot_survives_another_activation_writing_between_data_safe_points() {
    use orange_runtime::{ControlOutcome, Run, RunOptions};
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            branch(2, true),
            var_node(3, "get-variable", 100),
            core(4, "on-start"),
            set_bool(5, 100, false),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 3, "value", 2, "condition"),
            edge(3, 4, "then", 5, "in"),
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
    run.pump(1).unwrap();
    run.pump(1).unwrap();
    run.pump(1).unwrap();
    run.pump(200).unwrap();
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&orange_model::Value::Bool(false))
    );
    assert_eq!(
        run.trace()
            .iter()
            .find(|e| e.node.to_string() == id(2))
            .unwrap()
            .outcome,
        ControlOutcome::True
    );
}

#[test]
fn stop_run_fault_policy_cancels_suspended_sibling_then_cleans_it_up() {
    use orange_runtime::{Run, RunOptions, RunState};
    let mut entry = core(3, "on-start");
    entry["activation"]["on_fault"] = json!("stop-run");
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            wait(2, 100),
            entry,
            branch(4, true),
            var_node(5, "get-variable", 100),
            set_bool(6, 101, false),
            core(7, "on-stop"),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 3, "then", 4, "in"),
            edge(3, 5, "value", 4, "condition"),
            edge(4, 2, "cancelled", 6, "in"),
        ],
        vec![
            variable(100, "run", "core/bool@1", None),
            variable(101, "run", "core/bool@1", Some(json!(true))),
        ],
    );
    let mut run = Run::start(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        1,
        RunOptions::default(),
    )
    .unwrap();
    run.pump(300).unwrap();
    assert_eq!(run.state(), RunState::Cancelling);
    run.pump(300).unwrap();
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(
        run.variable(id(101).parse().unwrap()),
        Some(&orange_model::Value::Bool(false))
    );
    assert_eq!(run.pending_continuations(), 0);
    assert!(run.transitions().contains(&RunState::Faulted));
    assert!(!run.trace().iter().any(|e| e.node.to_string() == id(7)));
}

#[test]
fn segment_write_ids_are_unique_and_wait_cycles_cannot_bypass_run_fuel() {
    use orange_runtime::{Run, RunError, RunOptions, RunState};
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            sequence(2, &[0, 1]),
            set_bool(3, 100, true),
            set_bool(4, 100, false),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then#0", 3, "in"),
            edge(3, 2, "then#1", 4, "in"),
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
    run.pump(300).unwrap();
    assert_eq!(
        run.segment_writes(),
        Some((0, [id(100).parse().unwrap()].as_slice()))
    );
    let (doc, plan) = compiled(
        vec![core(1, "on-start"), wait(2, 0), branch(3, true)],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 3, "true", 2, "in"),
        ],
    );
    let mut options = RunOptions::default();
    options.limits.max_operations = 30;
    let mut run = Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options).unwrap();
    assert_eq!(run.pump(300), Err(RunError::ResourceLimit));
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(run.pending_continuations(), 0);
}

#[test]
fn deep_control_regions_are_iterative_and_depth_limits_fail_closed() {
    use orange_runtime::{Run, RunError, RunOptions, RunState};
    let mut nodes = vec![core(1, "on-start")];
    let mut edges = vec![edge(1, 1, "then", 2, "in")];
    for n in 2..514 {
        nodes.push(sequence(n, &[0, 1]));
        if n < 513 {
            edges.push(edge(n, n, "then#0", n + 1, "in"));
        }
    }
    let (doc, plan) = compiled(nodes, edges);
    for limit in [128, 1024] {
        let mut options = RunOptions::default();
        options.limits.max_region_depth = limit;
        let mut run = Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options).unwrap();
        if limit == 128 {
            assert_eq!(run.pump(10_000), Err(RunError::ResourceLimit));
            assert_eq!(run.state(), RunState::Stopped);
        } else {
            while run.pump(17).unwrap().ready {}
            assert_eq!(run.state(), RunState::Running);
        }
    }
}

#[test]
fn storage_and_presentation_changes_preserve_control_results_after_round_trip() {
    let (doc, plan) = state_plan(
        vec![core(1, "on-start"), count(2, 3), set_bool(3, 100, false)],
        vec![edge(1, 1, "then", 2, "in"), edge(2, 2, "body", 3, "in")],
        vec![variable(100, "run", "core/bool@1", Some(json!(true)))],
    );
    compare_source_control(&doc, plan, 1, 1);
    let mut raw: Json = serde_json::from_str(&orange_model::to_json(&doc).unwrap()).unwrap();
    raw["nodes"].as_array_mut().unwrap().reverse();
    raw["edges"].as_array_mut().unwrap().reverse();
    raw["variables"][0]["name"] = json!("متغير 日本語");
    raw["presentation_locale"] = json!("ar");
    let restored = from_json(&raw.to_string()).unwrap();
    let (package, snapshot) = catalog();
    let analysis = analyze(&restored, &snapshot);
    let plan = std::sync::Arc::new(
        compile_control(
            &restored,
            snapshot,
            &analysis,
            &package,
            CompileOptions::default(),
        )
        .unwrap(),
    );
    compare_source_control(&restored, plan, 1, 64);
}

#[test]
fn error_as_data_inside_successful_try_is_not_an_outer_failure() {
    use orange_runtime::{Run, RunOptions};
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            branch(2, true),
            var_node(3, "get-variable", 100),
            var_node(4, "set-variable", 101),
            core(5, "try"),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 3, "value", 2, "condition"),
            edge(3, 2, "failed", 4, "in"),
            edge(4, 2, "error", 5, "value"),
            edge(5, 5, "result", 4, "value"),
        ],
        vec![
            variable(100, "run", "core/bool@1", None),
            variable(101, "run", "core/result@1<core/error@1>", None),
        ],
    );
    compare_source_control(&doc, plan.clone(), 1, 1);
    let mut run = Run::start(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        1,
        RunOptions::default(),
    )
    .unwrap();
    run.pump(300).unwrap();
    let Some(orange_model::Value::Ok(inner)) = run.variable(id(101).parse().unwrap()) else {
        panic!("expected a boxed success");
    };
    let orange_model::Value::Err(error) = inner.as_ref() else {
        panic!("expected an error value as data");
    };
    assert_eq!(error.code, "core/UninitializedVariable");
    assert_eq!(error.node, Some(id(3).parse().unwrap()));
}

#[test]
#[ignore = "manual release control timing; no NFR or allocation threshold"]
fn headless_control_timing_baseline() {
    use orange_runtime::{Run, RunOptions};
    use std::hint::black_box;
    use std::time::{Duration, Instant};
    assert!(
        !black_box(cfg!(debug_assertions)),
        "run this probe with --release"
    );
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            count(2, 1000),
            var_node(3, "get-variable", 100),
            math(4, "add", 0, 1),
            var_node(5, "set-variable", 100),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 5, "in"),
            edge(3, 3, "value", 4, "a"),
            edge(4, 4, "sum", 5, "value"),
        ],
        vec![variable(100, "run", "core/u64@1", Some(json!("0")))],
    );
    let options = RunOptions {
        limits: orange_runtime::RunLimits {
            max_trace: 0,
            ..Default::default()
        },
        ..Default::default()
    };
    let execute = || {
        let mut run = Run::start(
            plan.clone(),
            &doc,
            plan.data().snapshot(),
            42,
            options.clone(),
        )
        .unwrap();
        let mut operations = 0;
        loop {
            let progress = run.pump(1024).unwrap();
            operations += progress.operations;
            if !progress.ready {
                break;
            }
        }
        assert_eq!(
            run.variable(id(100).parse().unwrap()),
            Some(&orange_model::Value::U64(1000))
        );
        run.stop().unwrap();
        run.dispose().unwrap();
        black_box(operations)
    };
    let warmup = Instant::now();
    while warmup.elapsed() < Duration::from_secs(10) {
        black_box(execute());
    }
    let mut samples = vec![];
    let mut operations = 0;
    for _ in 0..30 {
        let start = Instant::now();
        for _ in 0..8 {
            operations = execute();
        }
        samples.push(start.elapsed().as_nanos() as f64 / 8.0);
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "Count1000 release: warmup=10s samples=30 batch=8; start/run/stop/dispose included; trace disabled; operations/run={operations}; ns/run median={:.1} p95={:.1} max={:.1}",
        (samples[14] + samples[15]) / 2.0,
        samples[28],
        samples[29]
    );
    println!("sorted batch means ns/run={samples:?}");
}

#[test]
fn cleanup_preserves_entropy_invocations_instead_of_restarting_the_stream() {
    use orange_runtime::{EntropyKey, EntropyStream, FrameId, Run, RunOptions};
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            sequence(2, &[0, 1]),
            var_node(3, "set-variable", 100),
            wait(4, 100),
            var_node(5, "set-variable", 101),
            random(7),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then#0", 3, "in"),
            edge(3, 2, "then#1", 4, "in"),
            edge(4, 4, "cancelled", 5, "in"),
            edge(5, 7, "value", 3, "value"),
            edge(6, 7, "value", 5, "value"),
        ],
        vec![
            variable(100, "run", "core/u64@1", None),
            variable(101, "run", "core/u64@1", None),
        ],
    );
    let mut run = Run::start(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        42,
        RunOptions::default(),
    )
    .unwrap();
    run.pump(300).unwrap();
    run.stop().unwrap();
    run.pump(300).unwrap();
    for (invocation, var) in [(0, 100), (1, 101)] {
        let mut stream = EntropyStream::new(EntropyKey {
            seed: 42,
            frame: FrameId::ROOT,
            node: id(7).parse().unwrap(),
            activation_sequence: 0,
            invocation,
        });
        assert_eq!(
            run.variable(id(var).parse().unwrap()),
            Some(&orange_model::Value::U64(stream.next_u64().unwrap()))
        );
    }
}

#[test]
fn wait_completion_and_stop_choose_one_outcome_at_the_same_virtual_time() {
    use orange_runtime::{ControlOutcome, Run, RunOptions};
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            wait(2, 10),
            set_bool(3, 100, false),
            set_bool(4, 100, true),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "cancelled", 3, "in"),
            edge(3, 2, "then", 4, "in"),
        ],
        vec![variable(100, "run", "core/bool@1", Some(json!(true)))],
    );
    for completed in [false, true] {
        let mut run = Run::start(
            plan.clone(),
            &doc,
            plan.data().snapshot(),
            1,
            RunOptions::default(),
        )
        .unwrap();
        run.pump(300).unwrap();
        run.advance_to(10).unwrap();
        if completed {
            run.pump(1).unwrap();
        }
        run.stop().unwrap();
        if run.state() == orange_runtime::RunState::Cancelling {
            run.pump(300).unwrap();
        }
        assert_eq!(
            run.variable(id(100).parse().unwrap()),
            Some(&orange_model::Value::Bool(completed))
        );
        assert_eq!(
            run.trace()
                .iter()
                .filter(|e| e.node.to_string() == id(2)
                    && matches!(e.outcome, ControlOutcome::Then | ControlOutcome::Cancelled))
                .count(),
            1
        );
        assert_eq!(run.pending_continuations(), 0);
    }
}
