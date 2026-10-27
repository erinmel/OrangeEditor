//! Regressions from the pre-merge control/runtime review.
use super::*;

#[test]
fn initializers_and_data_constants_share_one_retained_budget() {
    let (package, snapshot) = catalog();
    let mut set = var_node(2, "set-variable", 100);
    set["inputs"] = json!({"value":{"type":"core/json@1","value":serde_json::to_string(&"y".repeat(160)).unwrap()}});
    let source = graph(
        &package,
        vec![core(1, "on-start"), set],
        vec![edge(1, 1, "then", 2, "in")],
        vec![],
    );
    let mut raw: Json = serde_json::from_str(&orange_model::to_json(&source).unwrap()).unwrap();
    raw["variables"] = json!([variable(
        100,
        "run",
        "core/json@1",
        Some(json!(serde_json::to_string(&"x".repeat(160)).unwrap()))
    )]);
    let doc = from_json(&raw.to_string()).unwrap();
    let analysis = analyze(&doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(&doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    let mut options = CompileOptions::default();
    options.limits.max_constant_bytes = 256;
    assert!(matches!(
        compile_control(&doc, snapshot.clone(), &analysis, &package, options),
        Err(CompileError::ResourceLimit)
    ));
    options.limits.max_constant_bytes = 512;
    let plan = compile_control(&doc, snapshot, &analysis, &package, options).unwrap();
    let initial: usize = plan
        .variables()
        .iter()
        .filter_map(|v| v.initial().and_then(|i| v.charge(i)))
        .sum();
    assert_eq!(initial + plan.data().initial_charge().unwrap(), 356);
}

use orange_model::Value;
use orange_runtime::{Run, RunOptions, RunState, StopMode};

#[test]
fn review_inactive_large_input_must_not_fault_active_literal_condition() {
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            branch(2, false),
            var_node(3, "set-variable", 101),
            var_node(4, "get-variable", 100),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "true", 3, "in"),
            edge(3, 4, "value", 3, "value"),
        ],
        vec![
            variable(
                100,
                "run",
                "core/json@1",
                Some(json!(serde_json::to_string(&"x".repeat(512)).unwrap())),
            ),
            variable(101, "run", "core/json@1", None),
        ],
    );
    let mut options = RunOptions::default();
    options.data.limits.max_retained_bytes = 128;
    let mut run = Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options).unwrap();
    let result = run.pump(1000);
    assert!(
        result.is_ok(),
        "inactive Get was published into an unrelated condition: {result:?}; state={:?}",
        run.state()
    );
    assert!(run.variable(id(101).parse().unwrap()).is_none());
}

#[test]
fn review_parallel_read_modify_write_must_not_lose_an_increment() {
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            var_node(2, "get-variable", 100),
            math(3, "add", 0, 1),
            var_node(4, "set-variable", 100),
            core(5, "on-start"),
            math(6, "add", 0, 1),
            var_node(7, "set-variable", 100),
        ],
        vec![
            edge(1, 1, "then", 4, "in"),
            edge(2, 2, "value", 3, "a"),
            edge(3, 3, "sum", 4, "value"),
            edge(4, 5, "then", 7, "in"),
            edge(5, 2, "value", 6, "a"),
            edge(6, 6, "sum", 7, "value"),
        ],
        vec![variable(100, "run", "core/u64@1", Some(json!("0")))],
    );
    let mut totals = vec![];
    for budget in [1, 1024] {
        let mut run = Run::start(
            plan.clone(),
            &doc,
            plan.data().snapshot(),
            1,
            RunOptions::default(),
        )
        .unwrap();
        while run.pump(budget).unwrap().ready {}
        totals.push(run.variable(id(100).parse().unwrap()).cloned());
    }
    assert_eq!(
        totals,
        vec![Some(Value::U64(2)), Some(Value::U64(2))],
        "pump quantum changed a Get/Add/Set result without an explicit graph suspension"
    );
}

#[test]
fn review_cancelled_control_only_cycle_observes_stop_before_fuel() {
    use std::time::{Duration, Instant};
    let (doc, plan) = compiled(
        vec![core(1, "on-start"), core(2, "yield"), core(3, "yield")],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 3, "then", 2, "in"),
        ],
    );
    let token = orange_runtime::CancelToken::default();
    let mut options = RunOptions::default();
    options.data.cancel = Some(token.clone());
    options.limits.max_operations = 2_000_000;
    options.limits.max_trace = 0;
    let mut run = Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options).unwrap();
    let sender = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(2));
        token.cancel();
    });
    let start = Instant::now();
    let result = run.pump(2_000_000);
    sender.join().unwrap();
    eprintln!(
        "control-only cancellation elapsed={:?}, result={result:?}, dropped={}",
        start.elapsed(),
        run.trace_dropped()
    );
    assert!(
        result.is_ok(),
        "cancellation was ignored until fuel exhaustion: {result:?}"
    );
    assert_eq!(run.state(), RunState::Stopped);
}

#[test]
fn review_stop_at_activation_capacity_can_start_on_stop() {
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            wait(2, 100),
            core(3, "on-stop"),
            set_bool(4, 100, false),
        ],
        vec![edge(1, 1, "then", 2, "in"), edge(2, 3, "then", 4, "in")],
        vec![variable(100, "run", "core/bool@1", Some(json!(true)))],
    );
    let mut options = RunOptions::default();
    options.limits.max_activations = 1;
    let mut run = Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options).unwrap();
    run.pump(1000).unwrap();
    assert!(
        run.request_stop(StopMode::Graceful).is_ok(),
        "accepted run cannot gracefully stop at its activation capacity"
    );
    run.pump(1000).unwrap();
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&Value::Bool(false))
    );
    run.advance_to(100).unwrap();
    run.pump(1000).unwrap();
    assert_eq!(run.state(), RunState::Stopped);
}

#[test]
fn stop_frames_are_reserved_at_start_and_never_executed_by_immediate_stop() {
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            core(2, "on-stop"),
            set_bool(3, 100, false),
        ],
        vec![edge(1, 2, "then", 3, "in")],
        vec![variable(100, "run", "core/bool@1", Some(json!(true)))],
    );
    let mut options = RunOptions::default();
    options.limits.max_stop_activations = 0;
    assert!(matches!(
        Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options),
        Err(orange_runtime::RunError::ResourceLimit)
    ));
    let mut run = Run::start(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        1,
        RunOptions::default(),
    )
    .unwrap();
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&Value::Bool(true))
    );
    run.request_stop(StopMode::Immediate).unwrap();
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&Value::Bool(true))
    );
    run.dispose().unwrap();
}

#[test]
fn aggregate_frame_storage_is_admitted_before_any_frame_is_built() {
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            core(2, "on-stop"),
            set_bool(3, 100, false),
        ],
        vec![edge(1, 2, "then", 3, "in")],
        vec![variable(100, "run", "core/bool@1", Some(json!(true)))],
    );
    let start = |limit: usize| {
        let mut options = RunOptions::default();
        options.limits.max_frame_storage_bytes = limit;
        Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options)
    };
    assert!(start(orange_runtime::RunLimits::default().max_frame_storage_bytes).is_ok());
    assert!(matches!(
        start(0),
        Err(orange_runtime::RunError::ResourceLimit)
    ));
    // Find the smallest admitted aggregate; one byte less is refused.
    let (mut refused, mut admitted) = (0usize, usize::MAX);
    while admitted - refused > 1 {
        let mid = refused + (admitted - refused) / 2;
        if start(mid).is_ok() {
            admitted = mid;
        } else {
            refused = mid;
        }
    }
    assert!(matches!(
        start(admitted - 1),
        Err(orange_runtime::RunError::ResourceLimit)
    ));
    // The admitted boundary still runs the reserved On Stop frame.
    let mut run = start(admitted).unwrap();
    run.request_stop(StopMode::Graceful).unwrap();
    run.pump(100).unwrap();
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&Value::Bool(false))
    );
}

#[test]
fn dynamic_lazy_arm_does_not_publish_unused_large_snapshot() {
    let select = json!({"id":id(5),"definition_id":"core/select","definition_version":1,
        "config_version":1,"config":{},"type_pins":{"T":"core/json@1"},
        "inputs":{"if_false":{"type":"core/json@1","value":"null"}}});
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            var_node(2, "set-variable", 102),
            var_node(3, "get-variable", 100),
            var_node(4, "get-variable", 101),
            select,
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 3, "value", 5, "condition"),
            edge(3, 4, "value", 5, "if_true"),
            edge(4, 5, "result", 2, "value"),
        ],
        vec![
            variable(100, "run", "core/bool@1", Some(json!(false))),
            variable(
                101,
                "run",
                "core/json@1",
                Some(json!(serde_json::to_string(&"x".repeat(512)).unwrap())),
            ),
            variable(102, "run", "core/json@1", None),
        ],
    );
    for budget in [1, 1000] {
        let mut options = RunOptions::default();
        options.data.limits.max_retained_bytes = 128;
        let mut run = Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options).unwrap();
        while run.pump(budget).unwrap().ready {}
        assert!(run.variable(id(102).parse().unwrap()).is_some());
        assert_eq!(run.state(), RunState::Running);
    }
}

#[test]
fn consumer_projection_metadata_has_an_admission_bound() {
    let mut nodes = vec![core(1, "on-start"), var_node(2, "set-variable", 100)];
    let mut edges = vec![edge(1, 1, "then", 2, "in")];
    for i in 0..24 {
        nodes.push(var_node(10 + i, "get-variable", 100));
        if i > 0 {
            nodes.push(math(100 + i, "add", 0, 0));
            edges.push(edge(10 + 2 * i, 10 + i, "value", 100 + i, "a"));
            edges.push(edge(
                11 + 2 * i,
                if i == 1 { 10 } else { 99 + i },
                if i == 1 { "value" } else { "sum" },
                100 + i,
                "b",
            ));
        }
    }
    edges.push(edge(1000, 123, "sum", 2, "value"));
    let (doc, plan) = state_plan(
        nodes,
        edges,
        vec![variable(100, "run", "core/u64@1", Some(json!("1")))],
    );
    let (package, _) = catalog();
    let snapshot = plan.data().snapshot();
    let analysis = analyze(&doc, snapshot);
    let mut options = CompileOptions::default();
    options.limits.max_graph_items = 200;
    assert!(matches!(
        compile_control(&doc, snapshot.clone(), &analysis, &package, options),
        Err(CompileError::ResourceLimit)
    ));
    options.limits.max_graph_items = 1000;
    assert!(compile_control(&doc, snapshot.clone(), &analysis, &package, options).is_ok());
}

#[test]
#[ignore = "manual release-only scaling probe; no NFR claim"]
fn review_unrelated_host_input_scaling() {
    use std::hint::black_box;
    use std::time::{Duration, Instant};
    if cfg!(debug_assertions) {
        panic!("run this manual scaling probe with --release");
    }
    for unrelated in [0u32, 64, 512] {
        let mut nodes = vec![core(1, "on-start"), count(2, 1000), set_bool(3, 100, false)];
        let mut edges = vec![edge(1, 1, "then", 2, "in"), edge(2, 2, "body", 3, "in")];
        let mut variables = vec![variable(100, "run", "core/bool@1", Some(json!(true)))];
        for i in 0..unrelated {
            let get = 1000 + 2 * i;
            let consumer = get + 1;
            nodes.push(var_node(get, "get-variable", 10000 + i));
            nodes.push(branch(consumer, true));
            edges.push(edge(100 + i, get, "value", consumer, "condition"));
            variables.push(variable(10000 + i, "run", "core/bool@1", Some(json!(true))));
        }
        let (doc, plan) = state_plan(nodes, edges, variables);
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
                1,
                options.clone(),
            )
            .unwrap();
            let mut operations = 0;
            loop {
                let p = run.pump(1024).unwrap();
                operations += p.operations;
                if !p.ready {
                    break;
                }
            }
            assert_eq!(
                run.variable(id(100).parse().unwrap()),
                Some(&Value::Bool(false))
            );
            assert_eq!(operations, 3003, "unrelated inputs added execution work");
            black_box(operations)
        };
        let warmup = Instant::now();
        while warmup.elapsed() < Duration::from_secs(1) {
            black_box(execute());
        }
        let mut samples = vec![];
        let mut operations = 0;
        for _ in 0..30 {
            let now = Instant::now();
            operations = execute();
            samples.push(now.elapsed().as_nanos());
        }
        samples.sort();
        eprintln!(
            "unrelated={unrelated} host_inputs={} ops={operations} median_us={:.3} p95_us={:.3}",
            plan.host_sources().len(),
            (samples[14] + samples[15]) as f64 / 2000.0,
            samples[28] as f64 / 1000.0
        );
    }
}

#[test]
#[ignore = "manual activation preparation scaling; not NFR acceptance"]
fn activation_preparation_by_owner_count() {
    use std::hint::black_box;
    use std::time::{Duration, Instant};
    if cfg!(debug_assertions) {
        panic!("release only");
    }
    for owners in [1u32, 32, 128] {
        let mut nodes = (1..=owners)
            .map(|i| core(i, "on-start"))
            .collect::<Vec<_>>();
        let mut edges = vec![];
        for i in 0..512 {
            nodes.push(var_node(1000 + 2 * i, "get-variable", 100));
            nodes.push(branch(1001 + 2 * i, true));
            edges.push(edge(
                i + 1,
                1000 + 2 * i,
                "value",
                1001 + 2 * i,
                "condition",
            ));
        }
        let (doc, plan) = state_plan(
            nodes,
            edges,
            vec![variable(100, "run", "core/bool@1", Some(json!(true)))],
        );
        let start_drop = || {
            black_box(
                Run::start(
                    plan.clone(),
                    &doc,
                    plan.data().snapshot(),
                    1,
                    RunOptions::default(),
                )
                .unwrap(),
            );
        };
        let warmup = Instant::now();
        while warmup.elapsed() < Duration::from_millis(500) {
            start_drop();
        }
        let mut samples = vec![];
        for _ in 0..30 {
            let t = Instant::now();
            start_drop();
            samples.push(t.elapsed().as_nanos());
        }
        samples.sort();
        eprintln!(
            "owners={owners} data_nodes={} median_start_drop_us={:.3} p95_us={:.3}",
            plan.data().resources().nodes,
            (samples[14] + samples[15]) as f64 / 2000.,
            samples[28] as f64 / 1000.
        );
    }
}
