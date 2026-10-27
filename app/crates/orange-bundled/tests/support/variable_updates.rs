use super::*;
use orange_model::Value;
use orange_runtime::{ControlOutcome, Run, RunError, RunOptions, RunState};
use std::sync::Arc;
fn canonical(ty: &str, value: Json) -> Json {
    if matches!(ty, "core/i64@1" | "core/u64@1" | "core/duration@1") && !value.is_string() {
        json!(value.to_string())
    } else {
        value
    }
}
fn typed(ty: &str, value: Json) -> Json {
    if matches!(ty, "core/f32@1" | "core/f64@1") {
        json!({"type":ty,"bits":value})
    } else {
        json!({"type":ty,"value":canonical(ty,value)})
    }
}
fn float_variable(ty: &str, bits: &str) -> Json {
    let mut v = variable(100, "run", ty, None);
    v["init"] = typed(ty, json!(bits));
    v
}
fn update(n: u32, var: u32, op: &str, ty: &str, value: Json, expected: Option<Json>) -> Json {
    let mut node = var_node(n, "update-variable", var);
    node["config"]["operation"] = json!(op);
    node["inputs"] = json!({"value":typed(ty,value)});
    if let Some(expected) = expected {
        node["inputs"]["expected"] = typed(ty, expected);
    }
    node
}
fn idle(run: &mut Run, budget: usize) {
    if run.state() == RunState::Stopped {
        return;
    }
    for _ in 0..20000 {
        if !run.pump(budget).unwrap().ready {
            return;
        }
    }
    panic!("fixture did not idle");
}
fn value(ty: &str, raw: &Json) -> Value {
    match ty {
        "core/bool@1" => Value::Bool(raw.as_bool().unwrap()),
        "core/i32@1" => Value::I32(raw.as_i64().unwrap().try_into().unwrap()),
        "core/i64@1" => Value::I64(raw.as_i64().unwrap()),
        "core/u32@1" => Value::U32(raw.as_u64().unwrap().try_into().unwrap()),
        "core/u64@1" => Value::U64(raw.as_u64().unwrap()),
        "core/string@1" => Value::Str(Arc::new(raw.as_str().unwrap().into())),
        "core/duration@1" => Value::Duration(raw.as_str().unwrap().parse().unwrap()),
        _ => panic!("unsupported oracle type"),
    }
}
#[test]
fn update_operations_publish_typed_old_new_and_exchange_results() {
    let cases = [
        (
            "append",
            "core/string@1",
            json!(""),
            json!("é中"),
            None,
            json!("é中"),
        ),
        ("add", "core/i32@1", json!(8), json!(-3), None, json!(5)),
        (
            "sub",
            "core/u64@1",
            json!(u64::MAX),
            json!(7),
            None,
            json!(u64::MAX - 7),
        ),
        (
            "min",
            "core/i64@1",
            json!(i64::MAX),
            json!(i64::MIN),
            None,
            json!(i64::MIN),
        ),
        (
            "max",
            "core/u32@1",
            json!(0),
            json!(u32::MAX),
            None,
            json!(u32::MAX),
        ),
        (
            "min",
            "core/bool@1",
            json!(true),
            json!(false),
            None,
            json!(false),
        ),
        (
            "max",
            "core/string@1",
            json!(""),
            json!("中"),
            None,
            json!("中"),
        ),
        (
            "min",
            "core/duration@1",
            json!("-2"),
            json!("-5"),
            None,
            json!("-5"),
        ),
        ("and", "core/i32@1", json!(-1), json!(5), None, json!(5)),
        (
            "or",
            "core/u64@1",
            json!(0),
            json!(u64::MAX),
            None,
            json!(u64::MAX),
        ),
        (
            "xor",
            "core/bool@1",
            json!(true),
            json!(true),
            None,
            json!(false),
        ),
        (
            "append",
            "core/string@1",
            json!("é"),
            json!("中🙂"),
            None,
            json!("é中🙂"),
        ),
        (
            "compare-and-set",
            "core/u64@1",
            json!(7),
            json!(9),
            Some(json!(7)),
            json!(9),
        ),
        (
            "compare-and-set",
            "core/u64@1",
            json!(7),
            json!(9),
            Some(json!(8)),
            json!(7),
        ),
        (
            "compare-and-set",
            "core/string@1",
            json!("same"),
            json!("same"),
            Some(json!("same")),
            json!("same"),
        ),
    ];
    for (op, ty, initial, operand, expected, result) in cases {
        for budget in [1, 13] {
            let compare = expected.is_some();
            let exchanged = expected.as_ref().map(|e| e == &initial);
            let mut nodes = vec![
                core(1, "on-start"),
                update(2, 100, op, ty, operand.clone(), expected.clone()),
                var_node(3, "set-variable", 101),
                var_node(4, "set-variable", 102),
            ];
            let mut edges = vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 2, "then", 3, "in"),
                edge(3, 3, "then", 4, "in"),
                edge(4, 2, "old", 3, "value"),
                edge(5, 2, "new", 4, "value"),
            ];
            let mut vars = vec![
                variable(100, "run", ty, Some(canonical(ty, initial.clone()))),
                variable(101, "run", ty, None),
                variable(102, "run", ty, None),
            ];
            if compare {
                nodes.push(var_node(5, "set-variable", 103));
                edges.extend([
                    edge(6, 4, "then", 5, "in"),
                    edge(7, 2, "exchanged", 5, "value"),
                ]);
                vars.push(variable(103, "run", "core/bool@1", None));
            }
            let (doc, plan) = state_plan(nodes, edges, vars);
            let mut run = Run::start(
                plan.clone(),
                &doc,
                plan.data().snapshot(),
                1,
                RunOptions::default(),
            )
            .unwrap();
            idle(&mut run, budget);
            assert_eq!(
                run.variable(id(100).parse().unwrap()),
                Some(&value(ty, &result)),
                "{op} {ty} budget{budget}"
            );
            assert_eq!(
                run.variable(id(101).parse().unwrap()),
                Some(&value(ty, &initial))
            );
            assert_eq!(
                run.variable(id(102).parse().unwrap()),
                Some(&value(ty, &result))
            );
            if let Some(exchanged) = exchanged {
                assert_eq!(
                    run.variable(id(103).parse().unwrap()),
                    Some(&Value::Bool(exchanged))
                );
            }
            assert!(
                run.trace()
                    .iter()
                    .any(|e| e.node == id(2).parse().unwrap() && e.outcome == ControlOutcome::Then)
            );
        }
    }
}

#[test]
fn integer_update_policies_and_float_rounding_match_language_rules() {
    for budget in [1, 17] {
        for (ty, initial, operand, op, policy, result) in [
            (
                "core/i32@1",
                json!(i32::MAX),
                json!(1),
                "add",
                "checked",
                None,
            ),
            (
                "core/i32@1",
                json!(i32::MAX),
                json!(1),
                "add",
                "wrapping",
                Some(Value::I32(i32::MIN)),
            ),
            (
                "core/i32@1",
                json!(i32::MAX),
                json!(1),
                "add",
                "saturating",
                Some(Value::I32(i32::MAX)),
            ),
            (
                "core/i64@1",
                json!(i64::MIN),
                json!(1),
                "sub",
                "checked",
                None,
            ),
            (
                "core/i64@1",
                json!(i64::MIN),
                json!(1),
                "sub",
                "wrapping",
                Some(Value::I64(i64::MAX)),
            ),
            (
                "core/u32@1",
                json!(0),
                json!(1),
                "sub",
                "saturating",
                Some(Value::U32(0)),
            ),
            (
                "core/u64@1",
                json!(u64::MAX),
                json!(1),
                "add",
                "wrapping",
                Some(Value::U64(0)),
            ),
        ] {
            let mut node = update(2, 100, op, ty, operand, None);
            node["config"]["int_overflow"] = json!(policy);
            let (doc, plan) = state_plan(
                vec![core(1, "on-start"), node],
                vec![edge(1, 1, "then", 2, "in")],
                vec![variable(
                    100,
                    "run",
                    ty,
                    Some(canonical(ty, initial.clone())),
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
            idle(&mut run, budget);
            match result {
                Some(expected) => {
                    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&expected))
                }
                None => {
                    assert_eq!(
                        run.variable(id(100).parse().unwrap()),
                        Some(&value(ty, &initial))
                    );
                    assert!(run.trace().iter().any(|e| {
                        e.fault
                            .as_ref()
                            .is_some_and(|f| f.fault.code == "core/NumericOverflow")
                    }));
                }
            }
        }
    }
    for (ty, initial, operand, expected) in [
        (
            "core/f32@1",
            "4b800000",
            "3f800000",
            Value::F32(16_777_216.0),
        ),
        (
            "core/f64@1",
            "3ff0000000000000",
            "4000000000000000",
            Value::F64(3.0),
        ),
    ] {
        let (doc, plan) = state_plan(
            vec![
                core(1, "on-start"),
                update(2, 100, "add", ty, json!(operand), None),
            ],
            vec![edge(1, 1, "then", 2, "in")],
            vec![float_variable(ty, initial)],
        );
        let mut run = Run::start(
            plan.clone(),
            &doc,
            plan.data().snapshot(),
            1,
            RunOptions::default(),
        )
        .unwrap();
        idle(&mut run, 1);
        assert_eq!(run.variable(id(100).parse().unwrap()), Some(&expected));
    }
    for finite in [true, false] {
        let mut node = update(2, 100, "add", "core/f32@1", json!("7f7fffff"), None);
        node["config"]["float_policy"] = json!(if finite { "finite" } else { "ieee" });
        let (doc, plan) = state_plan(
            vec![core(1, "on-start"), node],
            vec![edge(1, 1, "then", 2, "in")],
            vec![float_variable("core/f32@1", "7f7fffff")],
        );
        let mut run = Run::start(
            plan.clone(),
            &doc,
            plan.data().snapshot(),
            1,
            RunOptions::default(),
        )
        .unwrap();
        idle(&mut run, 1);
        assert_eq!(
            run.variable(id(100).parse().unwrap()),
            Some(&Value::F32(if finite { f32::MAX } else { f32::INFINITY }))
        );
        assert_eq!(
            run.trace().iter().any(|e| e
                .fault
                .as_ref()
                .is_some_and(|f| f.fault.code == "core/NonFinite")),
            finite
        );
    }
}

#[test]
fn compare_operands_share_one_epoch_and_still_demand_value_on_mismatch() {
    for shared in [true, false] {
        for budget in [1, 19] {
            let (package, snapshot) = catalog();
            let nodes = vec![
                core(1, "on-start"),
                update(
                    2,
                    100,
                    "compare-and-set",
                    "core/u64@1",
                    json!(99),
                    Some(json!(0)),
                ),
                random(3),
                random(4),
            ];
            let source = graph(
                &package,
                nodes,
                vec![
                    edge(1, 1, "then", 2, "in"),
                    edge(2, 3, "value", 2, "expected"),
                    edge(3, if shared { 3 } else { 4 }, "value", 2, "value"),
                ],
                vec![],
            );
            let mut raw: Json =
                serde_json::from_str(&orange_model::to_json(&source).unwrap()).unwrap();
            raw["variables"] = json!([variable(100, "run", "core/u64@1", Some(json!("0")))]);
            let doc = from_json(&raw.to_string()).unwrap();
            let analysis = analyze(&doc, &snapshot);
            assert!(analysis.is_ready_for_lowering(&doc, &snapshot));
            let resolver = Counting::new(package);
            let plan = Arc::new(
                compile_control(
                    &doc,
                    snapshot.clone(),
                    &analysis,
                    &resolver,
                    CompileOptions::default(),
                )
                .unwrap(),
            );
            let mut run = Run::start(plan, &doc, &snapshot, 42, RunOptions::default()).unwrap();
            idle(&mut run, budget);
            assert_eq!(
                resolver.draws.load(std::sync::atomic::Ordering::Relaxed),
                if shared { 1 } else { 2 }
            );
            assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(0)));
            assert!(run.segment_writes().is_none_or(|(_, ids)| ids.is_empty()));
        }
    }
}

#[test]
fn faulting_expected_operand_prevents_later_entropy_and_preserves_producer() {
    let (package, snapshot) = catalog();
    let source = graph(
        &package,
        vec![
            core(1, "on-start"),
            update(
                2,
                100,
                "compare-and-set",
                "core/u64@1",
                json!(0),
                Some(json!(0)),
            ),
            math(3, "divide", 1, 0),
            random(4),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 3, "quotient", 2, "expected"),
            edge(3, 4, "value", 2, "value"),
        ],
        vec![],
    );
    let mut raw: Json = serde_json::from_str(&orange_model::to_json(&source).unwrap()).unwrap();
    raw["variables"] = json!([variable(100, "run", "core/u64@1", Some(json!("7")))]);
    let doc = from_json(&raw.to_string()).unwrap();
    let resolver = Counting::new(package);
    let analysis = analyze(&doc, &snapshot);
    let plan = Arc::new(
        compile_control(
            &doc,
            snapshot.clone(),
            &analysis,
            &resolver,
            CompileOptions::default(),
        )
        .unwrap(),
    );
    let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
    idle(&mut run, 1);
    assert_eq!(resolver.draws.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(7)));
    assert!(
        run.trace()
            .iter()
            .any(|e| e.fault.as_ref().is_some_and(
                |f| f.source == id(3).parse().unwrap() && f.fault.code == "core/DivideByZero"
            ))
    );
}

#[test]
fn pending_update_holds_the_read_compute_commit_segment() {
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            update(2, 100, "add", "core/u64@1", json!(0), None),
            math(3, "add", 0, 1),
            var_node(4, "get-variable", 200),
            core(10, "on-start"),
            {
                let mut n = var_node(11, "set-variable", 200);
                n["inputs"] = json!({"value":literal(99)});
                n
            },
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 3, "sum", 2, "value"),
            edge(3, 4, "value", 3, "a"),
            edge(4, 10, "then", 11, "in"),
        ],
        vec![
            variable(100, "run", "core/u64@1", Some(json!("0"))),
            variable(200, "run", "core/u64@1", Some(json!("0"))),
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
    idle(&mut run, 1);
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(1)));
    let order: Vec<_> = run
        .trace()
        .iter()
        .filter(|e| {
            [id(2).parse().unwrap(), id(11).parse().unwrap()].contains(&e.node)
                && e.outcome == ControlOutcome::Then
        })
        .map(|e| e.node)
        .collect();
    assert_eq!(order, vec![id(2).parse().unwrap(), id(11).parse().unwrap()]);
}

#[test]
fn update_projection_quota_rejection_preserves_variable_and_write_intents() {
    let (doc, plan) = state_plan(
        vec![
            core(1, "on-start"),
            update(2, 100, "append", "core/string@1", json!("X"), None),
            var_node(3, "set-variable", 101),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 2, "new", 3, "value"),
        ],
        vec![
            variable(100, "run", "core/string@1", Some(json!("old!"))),
            variable(101, "run", "core/string@1", None),
        ],
    );
    let mut options = RunOptions::default();
    options.limits.max_state_bytes = 40;
    let mut run = Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options).unwrap();
    let mut result = None;
    for _ in 0..100 {
        if let Err(error) = run.pump(1) {
            result = Some(error);
            break;
        }
    }
    assert_eq!(result, Some(RunError::ResourceLimit));
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&Value::Str(Arc::new("old!".into())))
    );
    assert!(run.segment_writes().is_none_or(|(_, ids)| ids.is_empty()));
    assert!(
        !run.trace()
            .iter()
            .any(|e| e.node == id(2).parse().unwrap() && e.outcome == ControlOutcome::Then)
    );
}

#[test]
fn update_uninitialized_and_stopped_pending_paths_never_commit() {
    for initialized in [false, true] {
        let (doc, plan) = state_plan(
            vec![
                core(1, "on-start"),
                update(2, 100, "add", "core/u64@1", json!(1), None),
            ],
            vec![edge(1, 1, "then", 2, "in")],
            vec![variable(
                100,
                "run",
                "core/u64@1",
                initialized.then(|| json!("7")),
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
        if initialized {
            run.pump(1).unwrap();
            run.pump(1).unwrap();
            run.stop().unwrap();
            idle(&mut run, 1);
            assert_eq!(run.state(), RunState::Stopped);
            assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(7)));
        } else {
            idle(&mut run, 1);
            assert!(run.trace().iter().any(|e| {
                e.fault.as_ref().is_some_and(|f| {
                    f.source == id(2).parse().unwrap()
                        && f.fault.code == "core/UninitializedVariable"
                })
            }));
        }
        assert!(run.segment_writes().is_none_or(|(_, ids)| ids.is_empty()));
        if run.state() != RunState::Stopped {
            run.stop().unwrap();
            idle(&mut run, 1);
        }
        run.dispose().unwrap();
        assert!(run.variable(id(100).parse().unwrap()).is_none());
    }
}

#[test]
fn repeated_update_outputs_replace_retained_charges_instead_of_accumulating() {
    for budget in [1, 19] {
        let payload = "🙂".repeat(32);
        let (doc, plan) = state_plan(
            vec![
                core(1, "on-start"),
                count(2, 64),
                update(3, 100, "append", "core/string@1", json!(""), None),
                var_node(4, "set-variable", 101),
            ],
            vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 2, "body", 3, "in"),
                edge(3, 3, "then", 4, "in"),
                edge(4, 3, "new", 4, "value"),
            ],
            vec![
                variable(100, "run", "core/string@1", Some(json!(payload))),
                variable(101, "run", "core/string@1", None),
            ],
        );
        let mut options = RunOptions::default();
        options.limits.max_state_bytes = 400;
        let mut run = Run::start(plan.clone(), &doc, plan.data().snapshot(), 1, options).unwrap();
        idle(&mut run, budget);
        assert_eq!(
            run.variable(id(101).parse().unwrap()),
            Some(&Value::Str(Arc::new(payload.clone())))
        );
        assert_eq!(
            run.trace()
                .iter()
                .filter(|e| e.node == id(3).parse().unwrap() && e.outcome == ControlOutcome::Then)
                .count(),
            64
        );
    }
}

#[test]
fn fork_updates_share_local_cells_and_serialize_dynamic_operands() {
    for budget in [1, 17] {
        let mut fork = core(2, "fork");
        fork["config"] = json!({"join":"all"});
        fork["variadic"] = json!({"branch":{"order":[7,2],"next":8}});
        let (doc, plan) = state_plan(
            vec![
                core(1, "on-start"),
                fork,
                update(3, 100, "add", "core/u64@1", json!(0), None),
                update(4, 100, "add", "core/u64@1", json!(0), None),
                math(5, "add", 0, 1),
                var_node(6, "get-variable", 100),
                var_node(7, "set-variable", 101),
            ],
            vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 2, "branch#7", 3, "in"),
                edge(3, 2, "branch#2", 4, "in"),
                edge(4, 2, "completed", 7, "in"),
                edge(5, 6, "value", 5, "a"),
                edge(6, 5, "sum", 3, "value"),
                edge(7, 5, "sum", 4, "value"),
                edge(8, 6, "value", 7, "value"),
            ],
            vec![
                variable(100, "local", "core/u64@1", Some(json!("0"))),
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
        idle(&mut run, budget);
        assert_eq!(run.variable(id(101).parse().unwrap()), Some(&Value::U64(3)));
    }
}

#[test]
fn optional_compare_values_keep_none_some_and_nested_option_distinct() {
    for (ty, initial, replacement, expected) in [
        (
            "core/option@1<core/u64@1>",
            json!({"none":true}),
            json!({"some":"7"}),
            Value::U64(7),
        ),
        (
            "core/option@1<core/option@1<core/u64@1>>",
            json!({"none":true}),
            json!({"some":{"none":true}}),
            Value::Some(Arc::new(Value::None)),
        ),
    ] {
        let (doc, plan) = state_plan(
            vec![
                core(1, "on-start"),
                update(
                    2,
                    100,
                    "compare-and-set",
                    ty,
                    replacement,
                    Some(initial.clone()),
                ),
            ],
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
        idle(&mut run, 1);
        assert_eq!(run.variable(id(100).parse().unwrap()), Some(&expected));
    }
}

#[test]
fn malformed_numeric_update_policies_are_semantic_errors_before_binding() {
    for bad in [json!("unchecked"), json!(true)] {
        let (package, snapshot) = catalog();
        let mut node = update(2, 100, "add", "core/u64@1", json!(1), None);
        node["config"]["int_overflow"] = bad;
        let source = graph(
            &package,
            vec![core(1, "on-start"), node, math(3, "add", 1, 2)],
            vec![edge(1, 1, "then", 2, "in"), edge(2, 3, "sum", 2, "value")],
            vec![],
        );
        let mut raw: Json = serde_json::from_str(&orange_model::to_json(&source).unwrap()).unwrap();
        raw["variables"] = json!([variable(100, "run", "core/u64@1", Some(json!("0")))]);
        let doc = from_json(&raw.to_string()).unwrap();
        let analysis = analyze(&doc, &snapshot);
        assert!(!analysis.is_ready_for_lowering(&doc, &snapshot));
        let resolver = Counting::new(package);
        assert!(
            compile_control(
                &doc,
                snapshot,
                &analysis,
                &resolver,
                CompileOptions::default()
            )
            .is_err()
        );
        assert_eq!(resolver.pure.load(std::sync::atomic::Ordering::Relaxed), 0);
    }
}
