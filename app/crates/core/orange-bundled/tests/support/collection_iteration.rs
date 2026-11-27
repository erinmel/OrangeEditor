use super::*;
use orange_model::{
    Value,
    value::{ListRepr, MapRepr},
};
use orange_runtime::{ControlOutcome, Run, RunError, RunOptions, RunState};
use std::sync::Arc;
fn each(n: u32, mode: &str, key: &str, ty: &str, items: Json) -> Json {
    let mut node = core(n, "for-each");
    node["config"] = json!({"mode":mode});
    let collection = if mode == "list" {
        node["type_pins"] = json!({"T":ty});
        format!("core/list@1<{ty}>")
    } else {
        node["type_pins"] = json!({"K":key,"V":ty});
        format!("core/map@1<{key},{ty}>")
    };
    node["inputs"] = json!({"items":{"type":collection,"value":items}});
    node
}
fn update(n: u32, var: u32, op: &str, ty: &str, value: Json) -> Json {
    let mut node = var_node(n, "update-variable", var);
    node["config"]["operation"] = json!(op);
    node["inputs"] = json!({"value":{"type":ty,"value":value}});
    node
}
fn drain(run: &mut Run, budget: usize) {
    for _ in 0..100000 {
        if run.state() == RunState::Stopped {
            return;
        }
        let p = run.pump(budget).unwrap();
        if !p.ready {
            if let Some(at) = p.next_wakeup {
                run.advance_to(at).unwrap();
            } else {
                return;
            }
        }
    }
    panic!("fixture did not drain")
}
#[test]
fn for_each_list_and_map_follow_source_order_and_publish_indices() {
    for mode in ["list", "map"] {
        for budget in [1, 17] {
            let items = if mode == "list" {
                json!(["7", "2", "9"])
            } else {
                json!([["z", "7"], ["a", "2"], ["中", "9"]])
            };
            let mut nodes = vec![
                core(1, "on-start"),
                each(2, mode, "core/string@1", "core/u64@1", items),
                sequence(3, &[0, 1, 2]),
                update(4, 100, "add", "core/u64@1", json!("0")),
                update(5, 101, "add", "core/i64@1", json!("0")),
                update(6, 102, "append", "core/string@1", json!("L")),
            ];
            let mut edges = vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 2, "body", 3, "in"),
                edge(3, 3, "then#0", 4, "in"),
                edge(4, 3, "then#1", 5, "in"),
                edge(5, 3, "then#2", 6, "in"),
                edge(
                    6,
                    2,
                    if mode == "list" { "element" } else { "value" },
                    4,
                    "value",
                ),
                edge(7, 2, "index", 5, "value"),
            ];
            if mode == "map" {
                edges.push(edge(8, 2, "key", 6, "value"));
            }
            let (doc, plan) = state_plan(
                std::mem::take(&mut nodes),
                edges,
                vec![
                    variable(100, "run", "core/u64@1", Some(json!("0"))),
                    variable(101, "run", "core/i64@1", Some(json!("0"))),
                    variable(102, "run", "core/string@1", Some(json!(""))),
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
            drain(&mut run, budget);
            assert_eq!(
                run.variable(id(100).parse().unwrap()),
                Some(&Value::U64(18))
            );
            assert_eq!(run.variable(id(101).parse().unwrap()), Some(&Value::I64(3)));
            assert_eq!(
                run.variable(id(102).parse().unwrap()),
                Some(&Value::Str(Arc::new(
                    if mode == "list" { "LLL" } else { "za中" }.into()
                )))
            );
            assert_eq!(
                run.trace()
                    .iter()
                    .filter(
                        |e| e.node == id(2).parse().unwrap() && e.outcome == ControlOutcome::Body
                    )
                    .count(),
                3
            );
        }
    }
}
#[test]
fn empty_collection_completes_without_demanding_faulting_body() {
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            each(2, "list", "", "core/u64@1", json!([])),
            update(3, 100, "add", "core/u64@1", json!("0")),
            var_node(4, "get-variable", 101),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 3, "in"),
            edge(3, 4, "value", 3, "value"),
        ],
        vec![
            variable(100, "run", "core/u64@1", Some(json!("0"))),
            variable(101, "run", "core/u64@1", None),
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
    drain(&mut run, 1);
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(0)));
    assert!(
        run.trace()
            .iter()
            .any(|e| e.node == id(2).parse().unwrap() && e.outcome == ControlOutcome::Completed)
    );
    assert!(run.trace().iter().all(|e| e.fault.is_none()));
}
#[test]
fn collection_snapshot_survives_replacement_and_waits_between_items() {
    let mut clear = var_node(4, "set-variable", 200);
    clear["inputs"] = json!({"value":{"type":"core/list@1<core/u64@1>","value":[]}});
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            each(2, "list", "", "core/u64@1", json!([])),
            sequence(3, &[0, 1, 2]),
            clear,
            update(5, 100, "add", "core/u64@1", json!("0")),
            wait(6, 1),
            var_node(7, "get-variable", 200),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 3, "in"),
            edge(3, 3, "then#0", 4, "in"),
            edge(4, 3, "then#1", 5, "in"),
            edge(5, 3, "then#2", 6, "in"),
            edge(6, 2, "element", 5, "value"),
            edge(7, 7, "value", 2, "items"),
        ],
        vec![
            variable(100, "run", "core/u64@1", Some(json!("0"))),
            variable(
                200,
                "run",
                "core/list@1<core/u64@1>",
                Some(json!(["1", "2", "3"])),
            ),
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
    drain(&mut run, 1);
    assert_eq!(run.now(), 3);
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(6)));
    assert_eq!(
        run.variable(id(200).parse().unwrap()),
        Some(&Value::List(Arc::new(ListRepr::U64(Box::new([])))))
    );
}
#[test]
fn break_and_continue_target_the_collection_loop_and_release_before_reentry() {
    let mut switch = core(3, "switch");
    switch["type_pins"] = json!({"K":"core/i64@1"});
    switch["variadic"] = json!({"case":{"order":[7,2],"next":8}});
    switch["config"] = json!({"cases":{"7":0,"2":1}});
    switch["inputs"] = json!({"key":{"type":"core/i64@1","value":"0"}});
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            count(10, 50),
            each(2, "list", "", "core/u64@1", json!(["10", "20", "30", "40"])),
            switch,
            core(4, "continue"),
            update(5, 100, "add", "core/u64@1", json!("0")),
            core(6, "break"),
        ],
        vec![
            edge(1, 1, "then", 10, "in"),
            edge(2, 10, "body", 2, "in"),
            edge(3, 2, "body", 3, "in"),
            edge(4, 2, "index", 3, "key"),
            edge(5, 3, "case#7", 4, "in"),
            edge(6, 3, "case#2", 5, "in"),
            edge(7, 3, "default", 6, "in"),
            edge(8, 2, "element", 5, "value"),
        ],
        vec![variable(100, "run", "core/u64@1", Some(json!("0")))],
    );
    let mut options = RunOptions::default();
    options.limits.max_state_bytes = 128;
    let mut run = Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options).unwrap();
    drain(&mut run, 1);
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&Value::U64(1000))
    );
}

fn fork_node(n: u32, mode: &str) -> Json {
    let mut node = core(n, "fork");
    node["config"] = json!({"join":mode});
    node["variadic"] = json!({"branch":{"order":[0,1],"next":2}});
    node
}
#[test]
fn break_cleanup_reads_its_own_item_before_the_next_loop_invocation() {
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            count(10, 20),
            each(2, "list", "", "core/u64@1", json!(["11", "22"])),
            fork_node(3, "all"),
            wait(4, 100),
            core(5, "break"),
            update(6, 100, "add", "core/u64@1", json!("0")),
        ],
        vec![
            edge(1, 1, "then", 10, "in"),
            edge(2, 10, "body", 2, "in"),
            edge(3, 2, "body", 3, "in"),
            edge(4, 3, "branch#0", 4, "in"),
            edge(5, 3, "branch#1", 5, "in"),
            edge(6, 4, "cancelled", 6, "in"),
            edge(7, 2, "element", 6, "value"),
        ],
        vec![variable(100, "run", "core/u64@1", Some(json!("0")))],
    );
    let mut options = RunOptions::default();
    options.limits.max_state_bytes = 128;
    let mut run = Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options).unwrap();
    drain(&mut run, 1);
    assert_eq!(run.now(), 0);
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&Value::U64(220))
    );
    assert!(run.trace().iter().all(|e| e.fault.is_none()));
}
#[test]
fn race_and_run_stop_preserve_cancelled_iteration_context() {
    for stop in [false, true] {
        let (doc, plan) = state_plan(
            vec![
                core(1, "on-start"),
                fork_node(2, "race"),
                each(3, "list", "", "core/u64@1", json!(["3", "4"])),
                wait(4, 10),
                wait(5, 1),
                update(6, 100, "add", "core/u64@1", json!("0")),
            ],
            vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 2, "branch#0", 3, "in"),
                edge(3, 2, "branch#1", 5, "in"),
                edge(4, 3, "body", 4, "in"),
                edge(5, 4, "cancelled", 6, "in"),
                edge(6, 3, "element", 6, "value"),
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
        if stop {
            for _ in 0..1000 {
                if !run.pump(1).unwrap().ready {
                    break;
                }
            }
            run.stop().unwrap();
        }
        drain(&mut run, 1);
        assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(3)));
        assert!(run.trace().iter().all(|e| e.fault.is_none()));
        if stop {
            assert_eq!(run.state(), RunState::Stopped);
            run.dispose().unwrap();
        }
    }
}
#[test]
fn nested_lists_keep_outer_and_inner_indices_separate() {
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            each(
                2,
                "list",
                "",
                "core/list@1<core/u64@1>",
                json!([["1", "2"], ["3"]]),
            ),
            each(3, "list", "", "core/u64@1", json!([])),
            sequence(4, &[0, 1, 2]),
            update(5, 100, "add", "core/u64@1", json!("0")),
            update(6, 101, "add", "core/i64@1", json!("0")),
            update(7, 102, "add", "core/i64@1", json!("0")),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 3, "in"),
            edge(3, 2, "element", 3, "items"),
            edge(4, 3, "body", 4, "in"),
            edge(5, 4, "then#0", 5, "in"),
            edge(6, 4, "then#1", 6, "in"),
            edge(7, 4, "then#2", 7, "in"),
            edge(8, 3, "element", 5, "value"),
            edge(9, 2, "index", 6, "value"),
            edge(10, 3, "index", 7, "value"),
        ],
        vec![
            variable(100, "run", "core/u64@1", Some(json!("0"))),
            variable(101, "run", "core/i64@1", Some(json!("0"))),
            variable(102, "run", "core/i64@1", Some(json!("0"))),
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
    drain(&mut run, 1);
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(6)));
    assert_eq!(run.variable(id(101).parse().unwrap()), Some(&Value::I64(1)));
    assert_eq!(run.variable(id(102).parse().unwrap()), Some(&Value::I64(1)));
}

#[test]
fn independent_sibling_loops_do_not_capture_each_others_cleanup_frames() {
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            fork_node(2, "all"),
            each(3, "list", "", "core/u64@1", json!(["7"])),
            fork_node(4, "race"),
            wait(5, 10),
            wait(6, 1),
            update(7, 100, "add", "core/u64@1", json!("0")),
            each(10, "list", "", "core/u64@1", json!(["3", "4", "5"])),
            wait(11, 1),
            update(12, 101, "add", "core/u64@1", json!("0")),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "branch#0", 3, "in"),
            edge(3, 3, "body", 4, "in"),
            edge(4, 4, "branch#0", 5, "in"),
            edge(5, 4, "branch#1", 6, "in"),
            edge(6, 5, "cancelled", 7, "in"),
            edge(7, 3, "element", 7, "value"),
            edge(8, 2, "branch#1", 10, "in"),
            edge(9, 10, "body", 11, "in"),
            edge(10, 11, "then", 12, "in"),
            edge(11, 10, "element", 12, "value"),
        ],
        vec![
            variable(100, "run", "core/u64@1", Some(json!("0"))),
            variable(101, "run", "core/u64@1", Some(json!("0"))),
        ],
    );
    let mut options = RunOptions::default();
    options.limits.max_state_bytes = 160;
    let mut run = Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options).unwrap();
    drain(&mut run, 1);
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(7)));
    assert_eq!(
        run.variable(id(101).parse().unwrap()),
        Some(&Value::U64(12))
    );
    assert!(run.trace().iter().all(|e| e.fault.is_none()));
}
#[test]
fn iteration_quota_rejection_does_not_advance_or_publish_the_next_item() {
    let payload = "X".repeat(100);
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            each(2, "list", "", "core/string@1", json!(["a", payload])),
            var_node(3, "set-variable", 100),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 3, "in"),
            edge(3, 2, "element", 3, "value"),
        ],
        vec![variable(100, "run", "core/string@1", None)],
    );
    let mut options = RunOptions::default();
    options.limits.max_state_bytes = 200;
    let mut run = Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options).unwrap();
    let mut error = None;
    for _ in 0..1000 {
        if let Err(e) = run.pump(1) {
            error = Some(e);
            break;
        }
    }
    assert_eq!(error, Some(RunError::ResourceLimit));
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&Value::Str(Arc::new("a".into())))
    );
    assert_eq!(
        run.trace()
            .iter()
            .filter(|e| e.node == id(2).parse().unwrap() && e.outcome == ControlOutcome::Body)
            .count(),
        1
    );
    let mut fresh = Run::start(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        1,
        RunOptions::default(),
    )
    .unwrap();
    drain(&mut fresh, 1);
    assert_eq!(
        fresh.variable(id(100).parse().unwrap()),
        Some(&Value::Str(Arc::new(payload)))
    );
}
#[test]
fn local_writes_share_the_iterator_retained_budget() {
    let mut set = var_node(3, "set-variable", 100);
    set["inputs"] = json!({"value":{"type":"core/string@1","value":"X".repeat(20)}});
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            each(2, "list", "", "core/string@1", json!(["a"])),
            set,
        ],
        vec![edge(1, 1, "then", 2, "in"), edge(2, 2, "body", 3, "in")],
        vec![variable(100, "local", "core/string@1", Some(json!("old")))],
    );
    let mut options = RunOptions::default();
    options.limits.max_state_bytes = 96;
    let mut run = Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options).unwrap();
    let mut error = None;
    for _ in 0..1000 {
        if let Err(e) = run.pump(1) {
            error = Some(e);
            break;
        }
    }
    assert_eq!(error, Some(RunError::ResourceLimit));
    assert!(
        !run.trace()
            .iter()
            .any(|e| e.node == id(3).parse().unwrap() && e.outcome == ControlOutcome::Then)
    );
    assert!(run.segment_writes().is_none_or(|(_, ids)| ids.is_empty()));
}
#[test]
fn bytes_lists_and_timestamp_updates_use_the_existing_atomic_contract() {
    for (ty, op, initial, operand, expected) in [
        (
            "core/bytes@1",
            "append",
            json!("AAE="),
            json!("/w=="),
            Value::Bytes(Arc::new(vec![0, 1, 255])),
        ),
        (
            "core/list@1<core/u64@1>",
            "append",
            json!(["7", "2"]),
            json!(["9"]),
            Value::List(Arc::new(ListRepr::U64(vec![7, 2, 9].into()))),
        ),
        (
            "core/list@1<core/f64@1>",
            "append",
            json!(["3ff0000000000000"]),
            json!(["4000000000000000"]),
            Value::List(Arc::new(ListRepr::F64(vec![1.0, 2.0].into()))),
        ),
        (
            "core/timestamp@1",
            "max",
            json!("-1"),
            json!("7"),
            Value::Timestamp(7),
        ),
    ] {
        for budget in [1, 17] {
            let (doc, plan) = state_plan(
                vec![core(1, "on-start"), update(2, 100, op, ty, operand.clone())],
                vec![edge(1, 1, "then", 2, "in")],
                vec![variable(100, "run", ty, Some(initial.clone()))],
            );
            let mut run = Run::start(
                plan.clone(),
                &doc,
                plan.data().snapshot(),
                1,
                RunOptions::default(),
            )
            .unwrap();
            drain(&mut run, budget);
            assert_eq!(run.variable(id(100).parse().unwrap()), Some(&expected));
        }
    }
    let ty = "core/map@1<core/string@1,core/list@1<core/u64@1>>";
    let initial = json!([["z", ["1"]], ["a", ["2"]]]);
    let mut node = update(2, 100, "compare-and-set", ty, json!([]));
    node["inputs"]["expected"] = json!({"type":ty,"value":[["a",["2"]],["z",["1"]]]});
    let (doc, plan) = state_plan(
        vec![core(1, "on-start"), node],
        vec![edge(1, 1, "then", 2, "in")],
        vec![variable(100, "run", ty, Some(initial))],
    );
    let mut run = Run::start(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        1,
        RunOptions::default(),
    )
    .unwrap();
    drain(&mut run, 1);
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&Value::Map(Arc::default()))
    );
}
#[test]
fn oversized_collection_literal_is_refused_before_the_body_provider_is_bound() {
    let (package, snapshot) = catalog();
    let source = graph(
        &package,
        vec![
            core(1, "on-start"),
            each(2, "list", "", "core/u64@1", json!(vec!["1"; 4096])),
            update(3, 100, "add", "core/u64@1", json!("0")),
            math(90, "add", 1, 2),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 3, "in"),
            edge(3, 90, "sum", 3, "value"),
        ],
        vec![],
    );
    let mut raw: Json = serde_json::from_str(&orange_model::to_json(&source).unwrap()).unwrap();
    raw["variables"] = json!([variable(100, "run", "core/u64@1", Some(json!("0")))]);
    let doc = from_json(&raw.to_string()).unwrap();
    let analysis = analyze(&doc, &snapshot);
    assert!(analysis.is_ready_for_lowering(&doc, &snapshot));
    let resolver = Counting::new(package);
    let mut options = CompileOptions::default();
    options.limits.max_constant_bytes = 256;
    assert!(matches!(
        compile_control(&doc, snapshot, &analysis, &resolver, options),
        Err(CompileError::ResourceLimit)
    ));
    assert_eq!(resolver.pure.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[test]
fn pure_collection_exports_preserve_typed_values_and_map_order() {
    let (package, snapshot) = catalog();
    for (ty, payload, expected) in [
        (
            "core/bytes@1",
            json!("AAH/"),
            Value::Bytes(Arc::new(vec![0, 1, 255])),
        ),
        ("core/timestamp@1", json!("-7"), Value::Timestamp(-7)),
        (
            "core/list@1<core/option@1<core/u64@1>>",
            json!([{"none":true},{"some":"7"}]),
            Value::List(Arc::new(ListRepr::Values(
                vec![Value::None, Value::U64(7)].into(),
            ))),
        ),
        (
            "core/map@1<core/string@1,core/u64@1>",
            json!([["z", "7"], ["a", "2"]]),
            Value::Map(Arc::new(
                MapRepr::new(vec![
                    (Value::Str(Arc::new("z".into())), Value::U64(7)),
                    (Value::Str(Arc::new("a".into())), Value::U64(2)),
                ])
                .unwrap(),
            )),
        ),
    ] {
        let mut node = core(1, "select");
        node["type_pins"] = json!({"T":ty});
        node["inputs"] = json!({"condition":{"type":"core/bool@1","value":true},"if_true":{"type":ty,"value":payload},"if_false":{"type":ty,"value":payload}});
        let doc = graph(&package, vec![node], vec![], vec![export(1, "result")]);
        let analysis = analyze(&doc, &snapshot);
        let plan = orange_compiler::compile_pure(
            &doc,
            snapshot.clone(),
            &analysis,
            &package,
            CompileOptions::default(),
        )
        .unwrap();
        let values = plan
            .evaluate(
                &doc,
                &snapshot,
                orange_compiler::EvaluationOptions::default(),
            )
            .unwrap();
        assert_eq!(values.export(0).unwrap().unwrap().outcome, Ok(&expected));
    }
}
#[test]
fn malformed_or_oversized_host_maps_are_rejected_before_cache_publication() {
    use orange_runtime::{DataActivation, DataError, DataOptions};
    let ty = "core/map@1<core/string@1,core/u64@1>";
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            each(2, "map", "core/string@1", "core/u64@1", json!([])),
            var_node(3, "get-variable", 200),
        ],
        vec![edge(1, 1, "then", 2, "in"), edge(2, 3, "value", 2, "items")],
        vec![variable(200, "run", ty, Some(json!([])))],
    );
    let index = plan
        .host_sources()
        .iter()
        .position(|source| matches!(source, orange_compiler::HostSource::Variable(_)))
        .unwrap();
    let mut options = DataOptions::default();
    options.limits.max_retained_bytes = 1024;
    let mut data = DataActivation::start(
        plan.data().clone(),
        &doc,
        plan.data().snapshot(),
        1,
        0,
        options,
    )
    .unwrap();
    data.begin_epoch().unwrap();
    // A host cannot build a map with a repeated key; a wrongly typed one is refused.
    let wrong_key = Value::Map(Arc::new(
        MapRepr::new(vec![(Value::U64(1), Value::U64(1))]).unwrap(),
    ));
    assert_eq!(
        data.set_input(index, wrong_key),
        Err(DataError::InvalidInput { index })
    );
    let huge = Value::Map(Arc::new(
        MapRepr::new(vec![(
            Value::Str(Arc::new("X".repeat(4096))),
            Value::U64(1),
        )])
        .unwrap(),
    ));
    assert_eq!(data.set_input(index, huge), Err(DataError::ResourceLimit));
    assert_eq!(data.set_input(index, Value::Map(Arc::default())), Ok(()));
}
#[test]
fn suspended_iteration_can_move_to_another_host_thread_before_stop() {
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            each(2, "list", "", "core/u64@1", json!(["7"])),
            wait(3, 100),
            update(4, 100, "add", "core/u64@1", json!("0")),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 3, "in"),
            edge(3, 3, "cancelled", 4, "in"),
            edge(4, 2, "element", 4, "value"),
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
    for _ in 0..1000 {
        if !run.pump(1).unwrap().ready {
            break;
        }
    }
    let run = std::thread::spawn(move || {
        run.stop().unwrap();
        drain(&mut run, 1);
        run
    })
    .join()
    .unwrap();
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(7)));
}

#[test]
fn count_break_defers_index_retirement_until_cancelled_body_cleanup_finishes() {
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            count(2, 3),
            fork_node(3, "all"),
            wait(4, 100),
            core(5, "break"),
            var_node(6, "set-variable", 100),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 3, "in"),
            edge(3, 3, "branch#0", 4, "in"),
            edge(4, 3, "branch#1", 5, "in"),
            edge(5, 4, "cancelled", 6, "in"),
            edge(6, 2, "index", 6, "value"),
        ],
        vec![variable(100, "run", "core/i64@1", Some(json!("-1")))],
    );
    let mut run = Run::start(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        1,
        RunOptions::default(),
    )
    .unwrap();
    drain(&mut run, 1);
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::I64(0)));
    assert!(run.trace().iter().all(|e| e.fault.is_none()));
}
#[test]
fn deep_iteration_context_cancellation_releases_frames_iteratively() {
    let depth = 512u32;
    let mut nodes = vec![core(1, "on-start")];
    let mut edges = vec![edge(1, 1, "then", 2, "in")];
    for n in 2..depth + 2 {
        nodes.push(each(n, "list", "", "core/u64@1", json!(["7"])));
        edges.push(edge(n, n, "body", n + 1, "in"));
    }
    nodes.push(wait(depth + 2, 100));
    nodes.push(update(depth + 3, 100, "add", "core/u64@1", json!("0")));
    edges.push(edge(depth + 2, depth + 2, "cancelled", depth + 3, "in"));
    edges.push(edge(depth + 3, 2, "element", depth + 3, "value"));
    let (doc, plan) = state_plan(
        nodes,
        edges,
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
    for _ in 0..100000 {
        if !run.pump(1).unwrap().ready {
            break;
        }
    }
    assert_eq!(run.pending_continuations(), 1);
    run.stop().unwrap();
    drain(&mut run, 1);
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(7)));
    assert_eq!(run.ownership().strands, 0);
    run.dispose().unwrap();
}
#[test]
fn seeded_collection_orders_match_an_independent_source_fold() {
    let mut seed = 0x123456789abcdef0u64;
    for case in 0..48 {
        let count = case % 13;
        let mode = if case % 2 == 0 { "list" } else { "map" };
        let mut source = Vec::new();
        let mut items = Vec::new();
        for i in 0..count {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let value = format!("é{}🙂", seed % 97);
            source.push(value.clone());
            items.push(if mode == "list" {
                json!(value)
            } else {
                json!([format!("key-{}", count - i), value])
            });
        }
        let expected = source.concat();
        let (doc, plan) = state_plan(
            vec![
                core(1, "on-start"),
                each(2, mode, "core/string@1", "core/string@1", json!(items)),
                update(3, 100, "append", "core/string@1", json!("")),
            ],
            vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 2, "body", 3, "in"),
                edge(
                    3,
                    2,
                    if mode == "list" { "element" } else { "value" },
                    3,
                    "value",
                ),
            ],
            vec![variable(100, "run", "core/string@1", Some(json!("")))],
        );
        for budget in [1, 23] {
            let mut run = Run::start(
                plan.clone(),
                &doc,
                plan.data().snapshot(),
                seed,
                RunOptions::default(),
            )
            .unwrap();
            drain(&mut run, budget);
            assert_eq!(
                run.variable(id(100).parse().unwrap()),
                Some(&Value::Str(Arc::new(expected.clone())))
            );
            assert_eq!(
                run.trace()
                    .iter()
                    .filter(
                        |e| e.node == id(2).parse().unwrap() && e.outcome == ControlOutcome::Body
                    )
                    .count(),
                source.len()
            );
        }
    }
}
