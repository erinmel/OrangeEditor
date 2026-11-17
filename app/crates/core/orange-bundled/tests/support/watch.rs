use super::*;

fn watch(n: u32, mode: &str, ty: &str) -> Json {
    let mut node = core(n, "on-watch");
    node["config"] = json!({"mode":mode});
    node["type_pins"] = json!({"T":ty});
    node["activation"] = activation(0, 8, "fault");
    node
}
fn set(n: u32, v: u32, value: u64) -> Json {
    let mut node = variable_node(n, "set-variable", v);
    node["inputs"] = json!({"value":{"type":"core/u64@1","value":value.to_string()}});
    node
}
fn seen(run: &Run, n: u32) -> Option<u64> {
    match run.variable(id(n).parse().unwrap()) {
        Some(Value::U64(v)) => Some(*v),
        None => None,
        other => panic!("{other:?}"),
    }
}
#[test]
fn diamond_observes_both_final_writes_once_across_pump_slices() {
    let nodes = vec![
        core(1, "on-start"),
        set(2, 100, 10),
        set(3, 101, 20),
        variable_node(4, "get-variable", 100),
        variable_node(5, "get-variable", 101),
        math(6, "add", 0, 0),
        watch(10, "changed", "core/u64@1"),
        variable_node(11, "set-variable", 102),
        update_local(12, 103),
    ];
    let edges = vec![
        edge(1, 1, "then", 2, "in"),
        edge(2, 2, "then", 3, "in"),
        edge(3, 4, "value", 6, "a"),
        edge(4, 5, "value", 6, "b"),
        edge(5, 6, "sum", 10, "watch"),
        edge(6, 10, "then", 11, "in"),
        edge(7, 10, "value", 11, "value"),
        edge(8, 11, "then", 12, "in"),
    ];
    let variables = vec![
        variable(100, "run", Some(0)),
        variable(101, "run", Some(0)),
        variable(102, "run", None),
        variable(103, "run", Some(0)),
    ];
    let (doc, plan) = compile(nodes, edges, variables);
    for slice in [1, 2, 1000] {
        let mut run = run(&doc, plan.clone());
        for _ in 0..1000 {
            if !run.pump(slice).unwrap().ready {
                break;
            }
        }
        assert_eq!(seen(&run, 102), Some(30));
        assert_eq!(seen(&run, 103), Some(1));
        assert_eq!(
            run.last_event().unwrap().target,
            orange_runtime::EventTarget::Watch(id(10).parse().unwrap())
        );
        run.stop().unwrap();
        pump_until_idle(&mut run);
        run.dispose().unwrap();
        assert_eq!(run.retention().reserved_bytes, 0);
    }
}
#[test]
fn equal_result_is_silent_and_rising_fires_only_false_to_true() {
    let mut set_true = variable_node(2, "set-variable", 100);
    set_true["inputs"] = json!({"value":{"type":"core/bool@1","value":true}});
    let mut set_true_again = set_true.clone();
    set_true_again["id"] = json!(id(4));
    let nodes = vec![
        core(1, "on-start"),
        set_true,
        wait_node(3, 1),
        set_true_again,
        variable_node(5, "get-variable", 100),
        watch(10, "rising", "core/bool@1"),
        update_local(11, 101),
    ];
    let edges = vec![
        edge(1, 1, "then", 2, "in"),
        edge(2, 2, "then", 3, "in"),
        edge(3, 3, "then", 4, "in"),
        edge(4, 5, "value", 10, "watch"),
        edge(5, 10, "then", 11, "in"),
    ];
    let vars = vec![
        json!({"id":id(100),"name":"flag","type":"core/bool@1","scope":"run","init":{"type":"core/bool@1","value":false}}),
        variable(101, "run", Some(0)),
    ];
    let (doc, plan) = compile(nodes, edges, vars);
    let mut run = run(&doc, plan);
    pump_until_idle(&mut run);
    assert_eq!(seen(&run, 101), Some(1));
    run.advance_to(1).unwrap();
    pump_until_idle(&mut run);
    assert_eq!(seen(&run, 101), Some(1));
}
#[test]
fn watch_feedback_stops_at_causal_depth_without_faulting_run() {
    let nodes = vec![
        core(1, "on-start"),
        set(2, 100, 1),
        variable_node(3, "get-variable", 100),
        watch(10, "changed", "core/u64@1"),
        update_local(11, 100),
    ];
    let edges = vec![
        edge(1, 1, "then", 2, "in"),
        edge(2, 3, "value", 10, "watch"),
        edge(3, 10, "then", 11, "in"),
    ];
    let (doc, plan) = compile(nodes, edges, vec![variable(100, "run", Some(0))]);
    let mut run = run_with_options(
        &doc,
        plan,
        RunOptions {
            max_cause_depth: 3,
            ..Default::default()
        },
    );
    pump_until_idle(&mut run);
    assert_eq!(seen(&run, 100), Some(4));
    assert_eq!(run.event_stats().overflows, 1);
    assert_eq!(run.state(), RunState::Running);
}

#[test]
fn mixed_watch_and_variable_change_notifications_follow_saved_order() {
    for watch_first in [false, true] {
        let mut watched = watch(10, "changed", "core/u64@1");
        watched["activation"]["order"] = json!(if watch_first { 0 } else { 1 });
        let mut change = variable_node(20, "on-variable-changed", 100);
        change["activation"] = activation(if watch_first { 1 } else { 0 }, 8, "fault");
        let nodes = vec![
            core(1, "on-start"),
            set(2, 100, 1),
            variable_node(3, "get-variable", 100),
            watched,
            change,
            set(11, 101, 1),
            set(21, 101, 2),
        ];
        let edges = vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 3, "value", 10, "watch"),
            edge(3, 10, "then", 11, "in"),
            edge(4, 20, "then", 21, "in"),
        ];
        let (doc, plan) = compile(
            nodes,
            edges,
            vec![variable(100, "run", Some(0)), variable(101, "run", Some(0))],
        );
        let mut run = run(&doc, plan);
        pump_until_idle(&mut run);
        assert_eq!(seen(&run, 101), Some(if watch_first { 2 } else { 1 }));
        assert_eq!(run.event_stats().accepted, 2);
    }
}

#[test]
fn unrelated_variable_writes_do_not_open_watch_epochs() {
    let nodes = vec![
        core(1, "on-start"),
        set(2, 100, 1),
        wait_node(3, 1),
        set(4, 101, 9),
        variable_node(5, "get-variable", 100),
        watch(10, "changed", "core/u64@1"),
    ];
    let edges = vec![
        edge(1, 1, "then", 2, "in"),
        edge(2, 2, "then", 3, "in"),
        edge(3, 3, "then", 4, "in"),
        edge(4, 5, "value", 10, "watch"),
    ];
    let (doc, plan) = compile(
        nodes,
        edges,
        vec![variable(100, "run", Some(0)), variable(101, "run", Some(0))],
    );
    let mut run = run(&doc, plan);
    pump_until_idle(&mut run);
    assert_eq!(run.watch_evaluations(), 2);
    run.advance_to(1).unwrap();
    pump_until_idle(&mut run);
    assert_eq!(run.watch_evaluations(), 2);
    run.stop().unwrap();
    pump_until_idle(&mut run);
    assert_eq!(run.watch_evaluations(), 2);
}

#[test]
fn watch_fault_preserves_original_producer_and_success_clears_it() {
    let nodes = vec![
        core(1, "on-start"),
        wait_node(2, 1),
        set(3, 100, 1),
        variable_node(4, "get-variable", 100),
        math(6, "divide", 1, 1),
        watch(10, "changed", "core/u64@1"),
    ];
    let edges = vec![
        edge(1, 1, "then", 2, "in"),
        edge(2, 2, "then", 3, "in"),
        edge(3, 4, "value", 6, "b"),
        edge(4, 6, "quotient", 10, "watch"),
    ];
    let (doc, plan) = compile(nodes, edges, vec![variable(100, "run", Some(0))]);
    let mut run = run(&doc, plan);
    pump_until_idle(&mut run);
    assert_eq!(
        run.watch_fault(id(10).parse().unwrap()).unwrap().source,
        id(6).parse().unwrap()
    );
    run.advance_to(1).unwrap();
    pump_until_idle(&mut run);
    assert!(run.watch_fault(id(10).parse().unwrap()).is_none());
    assert_eq!(run.event_stats().accepted, 0);
}

#[test]
fn rising_requires_bool_and_entropy_is_outside_the_executable_watch_profile() {
    let (package, snapshot) = catalog();
    let mut bad = watch(10, "rising", "core/u64@1");
    bad["inputs"] = json!({"watch":{"type":"core/u64@1","value":"1"}});
    let doc = graph(&package, vec![bad], vec![], vec![]);
    let report = analyze(&doc, &snapshot);
    assert!(
        report
            .report()
            .diagnostics
            .iter()
            .any(|d| d.code == orange_model::validation::DiagnosticCode::TypeMismatch)
    );
    let doc = graph(
        &package,
        vec![
            crate::data_graph::random(1),
            watch(10, "changed", "core/u64@1"),
        ],
        vec![edge(1, 1, "value", 10, "watch")],
        vec![],
    );
    let analysis = analyze(&doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(&doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    assert!(matches!(
        compile_control(
            &doc,
            snapshot,
            &analysis,
            &package,
            CompileOptions::default()
        ),
        Err(orange_compiler::CompileError::UnsupportedNode { .. })
    ));
}
