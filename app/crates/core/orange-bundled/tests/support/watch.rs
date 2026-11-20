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

fn numeric_watch_run(config: Json) -> (GraphDocument, Arc<orange_compiler::ControlPlan>) {
    let mut trigger = watch(10, "above", "core/f64@1");
    trigger["config_version"] = json!(2);
    trigger["config"] = config;
    let initial = json!({"id":id(100), "name":"input", "type":"core/f64@1", "scope":"run",
        "init":{"type":"core/f64@1","bits":"0000000000000000"}});
    compile(
        vec![
            variable_node(1, "get-variable", 100),
            trigger,
            update_local(11, 101),
        ],
        vec![
            edge(1, 1, "value", 10, "watch"),
            edge(2, 10, "then", 11, "in"),
        ],
        vec![initial, variable(101, "run", Some(0))],
    )
}

#[test]
fn numeric_watch_releases_holds_and_observes_cooldown_in_virtual_time() {
    let (doc, plan) = numeric_watch_run(json!({
        "mode":"above", "threshold":10, "release":8, "hold":50, "cooldown":200
    }));
    let mut run = run(&doc, plan);
    let sample = |run: &mut Run, at_ms: u64, value| {
        run.advance_to(at_ms * 1_000_000).unwrap();
        run.set_variable(id(100).parse().unwrap(), Value::F64(value))
            .unwrap();
        pump_until_idle(run);
    };
    pump_until_idle(&mut run);
    sample(&mut run, 0, 11.0);
    sample(&mut run, 20, 9.0);
    assert_eq!(seen(&run, 101), Some(0));
    sample(&mut run, 25, 11.0);
    run.advance_to(74_000_000).unwrap();
    pump_until_idle(&mut run);
    assert_eq!(seen(&run, 101), Some(0));
    run.advance_to(75_000_000).unwrap();
    pump_until_idle(&mut run);
    assert_eq!(seen(&run, 101), Some(1));

    sample(&mut run, 85, 9.0);
    sample(&mut run, 100, 12.0);
    assert_eq!(run.pump(1).unwrap().next_wakeup, None);
    sample(&mut run, 110, 8.0);
    sample(&mut run, 120, 12.0);
    run.advance_to(274_000_000).unwrap();
    pump_until_idle(&mut run);
    assert_eq!(seen(&run, 101), Some(1));
    run.advance_to(275_000_000).unwrap();
    pump_until_idle(&mut run);
    assert_eq!(seen(&run, 101), Some(2));
}

#[test]
fn range_watches_use_closed_bounds_and_ignore_unordered_samples() {
    for (mode, samples) in [
        ("inside", [0.0, 2.0, f64::NAN, 3.0, 4.0, 3.0]),
        ("outside", [2.0, 4.0, f64::NAN, 0.0, 3.0, 0.0]),
        ("below", [0.0, -2.0, f64::NAN, -3.0, 1.0, -3.0]),
    ] {
        let config = if mode == "below" {
            json!({"mode":mode, "threshold":-1, "release":1})
        } else {
            json!({"mode":mode, "low":2, "high":3})
        };
        let (doc, plan) = numeric_watch_run(config);
        let mut run = run(&doc, plan);
        for (value, expected) in samples.into_iter().zip([0, 1, 1, 1, 1, 2]) {
            run.set_variable(id(100).parse().unwrap(), Value::F64(value))
                .unwrap();
            pump_until_idle(&mut run);
            assert_eq!(
                seen(&run, 101),
                Some(expected),
                "mode {mode}, sample {value}"
            );
        }
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
fn host_writes_before_a_pump_keep_their_own_change_payloads() {
    let mut first_change = variable_node(20, "on-variable-changed", 100);
    first_change["activation"] = activation(0, 8, "fault");
    let mut second_change = variable_node(21, "on-variable-changed", 101);
    second_change["activation"] = activation(0, 8, "fault");
    let (doc, plan) = compile(
        vec![
            variable_node(1, "get-variable", 100),
            watch(10, "changed", "core/u64@1"),
            first_change,
            second_change,
            variable_node(30, "set-variable", 102),
            variable_node(31, "set-variable", 103),
        ],
        vec![
            edge(1, 1, "value", 10, "watch"),
            edge(2, 20, "then", 30, "in"),
            edge(3, 20, "previous", 30, "value"),
            edge(4, 21, "then", 31, "in"),
            edge(5, 21, "value", 31, "value"),
        ],
        (100..=103).map(|id| variable(id, "run", Some(0))).collect(),
    );
    let mut run = run(&doc, plan);
    pump_until_idle(&mut run);
    for (variable, value) in [(100, 7), (101, 12), (100, 15)] {
        run.set_variable(id(variable).parse().unwrap(), Value::U64(value))
            .unwrap();
    }
    pump_until_idle(&mut run);
    assert_eq!(seen(&run, 102), Some(7));
    assert_eq!(seen(&run, 103), Some(12));
    assert_eq!(run.event_stats().accepted, 4);
}

#[test]
fn independent_host_writes_do_not_inherit_a_finished_segments_causal_depth() {
    for with_watch in [false, true] {
        let mut change = variable_node(1, "on-variable-changed", 100);
        change["activation"] = activation(0, 8, "fault");
        let mut nodes = vec![change, variable_node(2, "set-variable", 101)];
        let mut edges = vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 1, "previous", 2, "value"),
        ];
        if with_watch {
            nodes.extend([
                variable_node(3, "get-variable", 100),
                watch(4, "changed", "core/u64@1"),
            ]);
            edges.push(edge(3, 3, "value", 4, "watch"));
        }
        let (doc, plan) = compile(
            nodes,
            edges,
            vec![variable(100, "run", Some(0)), variable(101, "run", None)],
        );
        let mut run = run(&doc, plan);
        let source = id(100).parse().unwrap();
        pump_until_idle(&mut run);
        for value in 1..=96 {
            run.set_variable(source, Value::U64(value)).unwrap();
            pump_until_idle(&mut run);
            assert_eq!(seen(&run, 101), Some(value - 1), "host write {value}");
        }
        assert_eq!(
            run.event_stats().accepted,
            if with_watch { 192 } else { 96 }
        );
    }
}

#[test]
fn a_host_write_during_a_sliced_watch_is_observed() {
    let (doc, plan) = compile(
        vec![
            variable_node(1, "get-variable", 100),
            variable_node(2, "get-variable", 101),
            math(3, "add", 0, 0),
            watch(10, "changed", "core/u64@1"),
            variable_node(11, "set-variable", 102),
        ],
        vec![
            edge(1, 1, "value", 3, "a"),
            edge(2, 2, "value", 3, "b"),
            edge(3, 3, "sum", 10, "watch"),
            edge(4, 10, "then", 11, "in"),
            edge(5, 10, "value", 11, "value"),
        ],
        vec![
            variable(100, "run", Some(0)),
            variable(101, "run", Some(10)),
            variable(102, "run", Some(0)),
        ],
    );
    for slice in 0..16 {
        let mut run = run(&doc, plan.clone());
        pump_until_idle(&mut run);
        run.set_variable(id(100).parse().unwrap(), Value::U64(1))
            .unwrap();
        for _ in 0..slice {
            run.pump(1).unwrap();
        }
        run.set_variable(id(101).parse().unwrap(), Value::U64(20))
            .unwrap();
        pump_until_idle(&mut run);
        assert_eq!(
            seen(&run, 102),
            Some(21),
            "after {slice} single-operation pumps"
        );
    }
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

/// Two watches, each on its own variable, whose `then` both lead to node 11.
fn two_watches_into_one_chain(chain: Vec<Json>, extra: Vec<Json>) -> GraphDocument {
    let (package, _) = catalog();
    let mut nodes = vec![
        variable_node(1, "get-variable", 100),
        variable_node(2, "get-variable", 102),
        watch(10, "changed", "core/u64@1"),
        watch(20, "changed", "core/u64@1"),
    ];
    nodes.extend(chain);
    let mut edges = vec![
        edge(1, 1, "value", 10, "watch"),
        edge(2, 2, "value", 20, "watch"),
        edge(3, 10, "then", 11, "in"),
        edge(4, 20, "then", 11, "in"),
    ];
    edges.extend(extra);
    let variables = vec![
        variable(100, "run", Some(0)),
        variable(101, "run", Some(0)),
        variable(102, "run", Some(0)),
    ];
    let mut saved: Json =
        serde_json::from_str(&to_json(&graph(&package, nodes, edges, vec![])).unwrap()).unwrap();
    saved["variables"] = json!(variables);
    from_json(&saved.to_string()).unwrap()
}

#[test]
fn different_triggers_run_one_shared_chain() {
    let (doc, plan) = compile(
        vec![
            variable_node(1, "get-variable", 100),
            variable_node(2, "get-variable", 102),
            watch(10, "changed", "core/u64@1"),
            watch(20, "changed", "core/u64@1"),
            update_local(11, 101),
        ],
        vec![
            edge(1, 1, "value", 10, "watch"),
            edge(2, 2, "value", 20, "watch"),
            edge(3, 10, "then", 11, "in"),
            edge(4, 20, "then", 11, "in"),
        ],
        vec![
            variable(100, "run", Some(0)),
            variable(101, "run", Some(0)),
            variable(102, "run", Some(0)),
        ],
    );
    let mut run = run(&doc, plan);
    pump_until_idle(&mut run);
    assert_eq!(seen(&run, 101), Some(0));
    run.set_variable(id(100).parse().unwrap(), Value::U64(2))
        .unwrap();
    pump_until_idle(&mut run);
    assert_eq!(seen(&run, 101), Some(1), "the first trigger runs the chain");
    run.set_variable(id(102).parse().unwrap(), Value::U64(3))
        .unwrap();
    pump_until_idle(&mut run);
    assert_eq!(seen(&run, 101), Some(2), "the second trigger runs it too");
}

#[test]
fn a_shared_chain_reads_only_what_every_trigger_provides() {
    let (_, snapshot) = catalog();
    let codes = |doc: &GraphDocument| {
        analyze(doc, &snapshot)
            .report()
            .diagnostics
            .iter()
            .map(|d| d.code)
            .collect::<Vec<_>>()
    };
    // A variable is there whichever trigger fired.
    let store = variable_node(11, "set-variable", 101);
    let reads_variable =
        two_watches_into_one_chain(vec![store.clone()], vec![edge(5, 2, "value", 11, "value")]);
    assert_eq!(codes(&reads_variable), []);
    // The first watch's payload is not, when the second one fired.
    let reads_payload =
        two_watches_into_one_chain(vec![store], vec![edge(5, 10, "value", 11, "value")]);
    assert_eq!(
        codes(&reads_payload),
        [orange_model::validation::DiagnosticCode::CrossActivation]
    );
}
