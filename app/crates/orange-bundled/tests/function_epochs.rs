#[path = "support/data_graph.rs"]
mod support;
use orange_compiler::{
    CompileError, CompileLimits, CompileOptions, ConstantFault, DataOperation, FoldedValue,
    KernelFault, compile_control, compile_data,
};
use orange_model::{GraphDocument, Value, analyze, from_json, to_json};
use orange_runtime::{
    CancelToken, DataActivation, DataError, DataOptions, DataPoll, Run, RunOptions,
};
use serde_json::{Value as Json, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use support::*;

fn core(n: u32, name: &str) -> Json {
    json!({"id":id(n),"definition_id":format!("core/{name}"),"definition_version":1,"config_version":1,"config":{}})
}
fn call(n: u32, f: u32, ty: &str) -> Json {
    let mut n = core(n, "call");
    n["config"] = json!({"function":id(f)});
    n["type_pins"] = json!({"T":ty});
    n
}
fn get(n: u32, variable: u32) -> Json {
    let mut n = core(n, "get-variable");
    n["config"] = json!({"variable":id(variable)});
    n
}
fn port(name: &str, direction: &str, ty: &str) -> Json {
    json!({"port_id":name,"direction":direction,"kind":"data","type":ty})
}
fn identity(f: u32, nested: Option<u32>) -> Json {
    let mut nodes = vec![
        core(f + 1, "function-entry"),
        core(f + 2, "function-outputs"),
    ];
    let edges = if let Some(target) = nested {
        let mut c = call(f + 3, target, "$T");
        c["type_pins"] = json!({});
        nodes.push(c);
        vec![
            edge(f + 1, f + 1, "value", f + 3, "value"),
            edge(f + 2, f + 3, "result", f + 2, "result"),
        ]
    } else {
        vec![edge(f + 1, f + 1, "value", f + 2, "result")]
    };
    json!({"id":id(f),"name":"identity","kind":"pure","type_params":[{"name":"T"}],"inputs":[port("value","input","$T")],"outputs":[port("result","output","$T")],"nodes":nodes,"edges":edges})
}
fn arithmetic(f: u32, op: &str) -> Json {
    let mut n = math(f + 3, op, 0, 0);
    n["inputs"] = json!({});
    n["signature"] = json!(format!("{op}-generic"));
    json!({"id":id(f),"name":"arithmetic","kind":"pure","type_params":[{"name":"T","constraints":["num"]}],
        "inputs":[port("a","input","$T"),port("b","input","$T")],"outputs":[port("result","output","$T")],
        "nodes":[core(f+1,"function-entry"),core(f+2,"function-outputs"),n],
        "edges":[edge(f+1,f+1,"a",f+3,"a"),edge(f+2,f+1,"b",f+3,"b"),edge(f+3,f+3,if op=="add" {"sum"} else {"quotient"},f+2,"result")]})
}
fn document(
    package: &orange_bundled::MathPackage,
    nodes: Vec<Json>,
    edges: Vec<Json>,
    exports: Vec<Json>,
    functions: Vec<Json>,
    variables: Vec<Json>,
) -> GraphDocument {
    let base = graph(package, nodes, edges, exports);
    let mut raw: Json = serde_json::from_str(&to_json(&base).unwrap()).unwrap();
    raw["functions"] = json!(functions);
    raw["variables"] = json!(variables);
    for op in ["add", "divide"] {
        let mut signature = raw["signatures"][op].clone();
        for p in signature["ports"].as_array_mut().unwrap() {
            p["type"] = json!("$T");
        }
        raw["signatures"][format!("{op}-generic")] = signature;
    }
    from_json(&raw.to_string()).unwrap()
}
fn variable(n: u32, ty: &str) -> Json {
    json!({"id":id(n),"name":format!("v{n}"),"type":ty,"scope":"run"})
}
fn supplied(
    activation: &mut DataActivation,
    plan: &orange_compiler::DataPlan,
    n: u32,
    value: FoldedValue,
) {
    let index = plan
        .host_inputs()
        .iter()
        .position(|i| plan.nodes()[i.index()].node().to_string() == id(n))
        .unwrap();
    activation.set_input_outcome(index, value).unwrap();
}
fn value(activation: &mut DataActivation, index: usize, budget: usize) -> Value {
    for _ in 0..10_000 {
        match activation.poll_export(index, &mut { budget }).unwrap() {
            DataPoll::Pending => {}
            DataPoll::Ready(Some(e)) => return e.outcome.unwrap().clone(),
            DataPoll::Ready(None) => panic!("missing export"),
        }
    }
    panic!("pull did not finish")
}

#[test]
fn generic_call_instances_match_arithmetic_oracle_and_isolate_epochs_and_activations() {
    let (package, snapshot) = catalog();
    let mut first = call(1, 100, "core/u64@1");
    first["inputs"] = json!({"b":literal(3)});
    let mut second = call(2, 100, "core/u64@1");
    second["inputs"] = json!({"b":literal(7)});
    let mut duplicate = export(1, "result");
    duplicate["id"] = json!("again");
    let doc = document(
        &package,
        vec![first, second, get(3, 10)],
        vec![edge(1, 3, "value", 1, "a"), edge(2, 3, "value", 2, "a")],
        vec![export(1, "result"), export(2, "result"), duplicate],
        vec![arithmetic(100, "add")],
        vec![variable(10, "core/u64@1")],
    );
    let before = to_json(&doc).unwrap();
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 0);
    let bodies: Vec<_> = plan
        .nodes()
        .iter()
        .filter(|n| n.node().to_string() == id(103))
        .collect();
    assert_eq!(bodies.len(), 2);
    assert_ne!(bodies[0].call_instance(), bodies[1].call_instance());
    assert_eq!(plan.call_sites().len(), 2);
    let mut a =
        DataActivation::start(plan.clone(), &doc, &snapshot, 9, 0, DataOptions::default()).unwrap();
    let mut b =
        DataActivation::start(plan.clone(), &doc, &snapshot, 9, 1, DataOptions::default()).unwrap();
    for epoch in 0..32u64 {
        a.begin_epoch().unwrap();
        b.begin_epoch().unwrap();
        let x = epoch.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        supplied(&mut a, &plan, 3, FoldedValue::Value(Value::U64(x)));
        supplied(&mut b, &plan, 3, FoldedValue::Value(Value::U64(!x)));
        for (frame, input) in [(&mut a, x), (&mut b, !x)] {
            assert_eq!(value(frame, 0, 1), Value::U64(input.wrapping_add(3)));
            assert_eq!(value(frame, 1, 3), Value::U64(input.wrapping_add(7)));
            assert_eq!(value(frame, 2, 1), Value::U64(input.wrapping_add(3)));
        }
        assert_eq!(
            resolver.folds.load(Ordering::Relaxed),
            (epoch as usize + 1) * 4
        );
    }
    assert_eq!(before, to_json(&doc).unwrap());
}

#[test]
fn nested_identity_calls_specialize_independently_and_constant_calls_keep_folding() {
    let (package, snapshot) = catalog();
    let mut constant = call(5, 100, "core/u64@1");
    constant["inputs"] = json!({"value":literal(19)});
    let doc = document(
        &package,
        vec![
            call(1, 100, "core/u64@1"),
            get(2, 10),
            call(3, 100, "core/bool@1"),
            get(4, 11),
            constant,
        ],
        vec![
            edge(1, 2, "value", 1, "value"),
            edge(2, 4, "value", 3, "value"),
        ],
        vec![
            export(1, "result"),
            export(3, "result"),
            export(5, "result"),
        ],
        vec![identity(100, Some(200)), identity(200, None)],
        vec![variable(10, "core/u64@1"), variable(11, "core/bool@1")],
    );
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    assert!(plan.call_sites().iter().any(|c| c.parent.is_some()));
    assert!(plan.initial_values()[plan.exports()[2].slot.index()].is_some());
    let mut frame =
        DataActivation::start(plan.clone(), &doc, &snapshot, 1, 0, DataOptions::default()).unwrap();
    frame.begin_epoch().unwrap();
    supplied(&mut frame, &plan, 2, FoldedValue::Value(Value::U64(99)));
    supplied(&mut frame, &plan, 4, FoldedValue::Value(Value::Bool(true)));
    assert_eq!(value(&mut frame, 0, 1), Value::U64(99));
    assert_eq!(value(&mut frame, 1, 1), Value::Bool(true));
    assert_eq!(value(&mut frame, 2, 1), Value::U64(19));
    assert_eq!(resolver.pure.load(Ordering::Relaxed), 0);
}

#[test]
fn strict_unused_argument_fault_prevents_body_work_and_keeps_original_source() {
    let (package, snapshot) = catalog();
    let mut function = arithmetic(100, "add");
    function["inputs"]
        .as_array_mut()
        .unwrap()
        .push(port("unused", "input", "$T"));
    let mut c = call(1, 100, "core/u64@1");
    c["inputs"] = json!({"b":literal(1)});
    let doc = document(
        &package,
        vec![c, get(2, 10), get(3, 11)],
        vec![
            edge(1, 2, "value", 1, "a"),
            edge(2, 3, "value", 1, "unused"),
        ],
        vec![export(1, "result")],
        vec![function],
        vec![variable(10, "core/u64@1"), variable(11, "core/u64@1")],
    );
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let mut frame =
        DataActivation::start(plan.clone(), &doc, &snapshot, 1, 0, DataOptions::default()).unwrap();
    frame.begin_epoch().unwrap();
    supplied(&mut frame, &plan, 2, FoldedValue::Value(Value::U64(8)));
    let fault = ConstantFault {
        source: id(3).parse().unwrap(),
        fault: KernelFault {
            code: "core/TestFault",
            message_key: "core/test-fault",
            message: "test",
        },
    };
    supplied(&mut frame, &plan, 3, FoldedValue::Fault(fault.clone()));
    let error = frame.export(0).unwrap().unwrap().outcome.unwrap_err();
    assert_eq!(error, &fault);
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 0);
}

#[test]
fn result_collection_fault_is_atomic_and_uses_declared_order() {
    let (package, snapshot) = catalog();
    let mut f = arithmetic(100, "divide");
    f["outputs"][0]["port_id"] = json!("z_bad");
    f["outputs"]
        .as_array_mut()
        .unwrap()
        .push(port("a_second", "output", "$T"));
    f["edges"][2]["to"]["port"] = json!("z_bad");
    f["edges"].as_array_mut().unwrap().extend([
        edge(104, 101, "a", 104, "a"),
        edge(105, 101, "b", 104, "b"),
        edge(106, 104, "quotient", 102, "a_second"),
    ]);
    let mut second = math(104, "divide", 0, 0);
    second["inputs"] = json!({});
    f["nodes"].as_array_mut().unwrap().push(second);
    f["type_params"] = json!([]);
    for field in ["inputs", "outputs"] {
        for p in f[field].as_array_mut().unwrap() {
            p["type"] = json!("core/u64@1");
        }
    }
    f["nodes"][2]["signature"] = json!("divide");
    let mut c = call(1, 100, "core/u64@1");
    c["type_pins"] = json!({});
    c["inputs"] = json!({"b":literal(0)});
    let doc = document(
        &package,
        vec![c, get(2, 10)],
        vec![edge(1, 2, "value", 1, "a")],
        vec![export(1, "a_second")],
        vec![f],
        vec![variable(10, "core/u64@1")],
    );
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let mut frame =
        DataActivation::start(plan.clone(), &doc, &snapshot, 1, 0, DataOptions::default()).unwrap();
    frame.begin_epoch().unwrap();
    supplied(&mut frame, &plan, 2, FoldedValue::Value(Value::U64(9)));
    let e = frame.export(0).unwrap().unwrap();
    assert_eq!(e.outcome.unwrap_err().source, id(103).parse().unwrap());
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 1);
    assert_eq!(
        plan.nodes()
            .iter()
            .find(|n| n.node().to_string() == id(103))
            .unwrap()
            .call_instance(),
        Some(0)
    );
}

#[test]
fn deep_dynamic_calls_yield_and_admission_fails_before_callbacks() {
    let (package, snapshot) = catalog();
    let functions = (0..128)
        .map(|i| {
            identity(
                100 + i * 10,
                if i < 127 { Some(110 + i * 10) } else { None },
            )
        })
        .collect();
    let doc = document(
        &package,
        vec![call(1, 100, "core/u64@1"), get(2, 10)],
        vec![edge(1, 2, "value", 1, "value")],
        vec![export(1, "result")],
        functions,
        vec![variable(10, "core/u64@1")],
    );
    let resolver = Counting::new(package);
    let analysis = analyze(&doc, &snapshot);
    assert!(analysis.is_ready_for_lowering(&doc, &snapshot));
    let flag = AtomicBool::new(true);
    assert_eq!(
        compile_data(
            &doc,
            snapshot.clone(),
            &analysis,
            &resolver,
            CompileOptions {
                cancel: Some(&flag),
                ..Default::default()
            }
        )
        .unwrap_err(),
        CompileError::Cancelled
    );
    for limits in [
        CompileLimits {
            max_call_depth: 127,
            ..Default::default()
        },
        CompileLimits {
            max_slots: 32,
            ..Default::default()
        },
        CompileLimits {
            max_fold_operations: 32,
            ..Default::default()
        },
    ] {
        assert_eq!(
            compile_data(
                &doc,
                snapshot.clone(),
                &analysis,
                &resolver,
                CompileOptions {
                    limits,
                    ..Default::default()
                }
            )
            .unwrap_err(),
            CompileError::ResourceLimit
        );
    }
    assert_eq!(resolver.pure.load(Ordering::Relaxed), 0);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let token = CancelToken::new();
    let mut frame = DataActivation::start(
        plan.clone(),
        &doc,
        &snapshot,
        1,
        0,
        DataOptions {
            cancel: Some(token.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    frame.begin_epoch().unwrap();
    supplied(&mut frame, &plan, 2, FoldedValue::Value(Value::U64(123)));
    assert!(matches!(
        frame.poll_export(0, &mut 1).unwrap(),
        DataPoll::Pending
    ));
    token.cancel();
    assert!(matches!(
        frame.poll_export(0, &mut 1),
        Err(DataError::Cancelled)
    ));
    let mut frame =
        DataActivation::start(plan.clone(), &doc, &snapshot, 1, 0, DataOptions::default()).unwrap();
    frame.begin_epoch().unwrap();
    supplied(&mut frame, &plan, 2, FoldedValue::Value(Value::U64(123)));
    assert_eq!(value(&mut frame, 0, 1), Value::U64(123));
    for activation_fuel in [false, true] {
        let mut limits = orange_runtime::DataLimits::default();
        if activation_fuel {
            limits.max_activation_operations = 3;
        } else {
            limits.max_operations = 3;
        }
        let mut bounded = DataActivation::start(
            plan.clone(),
            &doc,
            &snapshot,
            1,
            0,
            DataOptions {
                limits,
                ..Default::default()
            },
        )
        .unwrap();
        bounded.begin_epoch().unwrap();
        supplied(&mut bounded, &plan, 2, FoldedValue::Value(Value::U64(123)));
        assert_eq!(
            bounded.export(0).unwrap_err(),
            if activation_fuel {
                DataError::FuelExhausted
            } else {
                DataError::ResourceLimit
            }
        );
    }
}

#[test]
fn control_consumer_resumes_a_pure_call_across_host_pumps() {
    let (package, snapshot) = catalog();
    let mut init = variable(10, "core/bool@1");
    init["init"] = json!({"type":"core/bool@1","value":true});
    let mut branch = core(3, "branch");
    branch["inputs"] = json!({});
    let mut start = core(1, "on-start");
    start["activation"] = json!({"mode":"queue","capacity":4,"overflow":"drop-newest","on_fault":"continue","order":0});
    let doc = document(
        &package,
        vec![start, get(2, 10), branch, call(4, 100, "core/bool@1")],
        vec![
            edge(1, 1, "then", 3, "in"),
            edge(2, 2, "value", 4, "value"),
            edge(3, 4, "result", 3, "condition"),
        ],
        vec![],
        vec![identity(100, Some(200)), identity(200, None)],
        vec![init],
    );
    let analysis = analyze(&doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(&doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    let plan = Arc::new(
        compile_control(
            &doc,
            snapshot.clone(),
            &analysis,
            &package,
            CompileOptions::default(),
        )
        .unwrap(),
    );
    assert!(
        plan.data()
            .nodes()
            .iter()
            .any(|n| matches!(n.operation(), DataOperation::Call { .. }))
    );
    let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
    for _ in 0..100 {
        run.pump(1).unwrap();
    }
    assert_eq!(
        run.trace().back().unwrap().outcome,
        orange_runtime::ControlOutcome::True
    );
    run.stop().unwrap();
    run.dispose().unwrap();
}
#[test]
fn seeded_inputs_flow_through_calls_without_extra_draws_and_replay_across_epochs() {
    let (package, snapshot) = catalog();
    let doc = document(
        &package,
        vec![random(1), call(2, 100, "core/u64@1")],
        vec![edge(1, 1, "value", 2, "value")],
        vec![export(2, "result"), export(1, "value")],
        vec![identity(100, None)],
        vec![],
    );
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    assert_eq!(resolver.draws.load(Ordering::Relaxed), 0);
    let mut frames = (0..2)
        .map(|_| {
            DataActivation::start(plan.clone(), &doc, &snapshot, 45, 8, DataOptions::default())
                .unwrap()
        })
        .collect::<Vec<_>>();
    for epoch in 0..16 {
        let mut results = Vec::new();
        for frame in &mut frames {
            frame.begin_epoch().unwrap();
            let result = value(frame, 0, 1);
            assert_eq!(result, value(frame, 1, 1));
            results.push(result);
        }
        assert_eq!(results[0], results[1]);
        assert_eq!(resolver.draws.load(Ordering::Relaxed), (epoch + 1) * 2);
    }
}

#[test]
fn lazy_callee_chooses_only_one_arm_and_faults_do_not_poison_later_epochs() {
    for dynamic in [false, true] {
        let (package, snapshot) = catalog();
        let mut divide = math(103, "divide", 0, 0);
        divide["inputs"] = json!({"b":literal(0)});
        let mut select = core(104, "select");
        select["type_pins"] = json!({"T":"core/u64@1"});
        let f = json!({"id":id(100),"name":"lazy","kind":"pure","inputs":[port("flag","input","core/bool@1"),port("value","input","core/u64@1")],"outputs":[port("result","output","core/u64@1")],
            "nodes":[core(101,"function-entry"),core(102,"function-outputs"),divide,select],
            "edges":[edge(101,101,"value",103,"a"),edge(102,101,"flag",104,"condition"),edge(103,101,"value",104,"if_true"),edge(104,103,"quotient",104,"if_false"),edge(105,104,"result",102,"result")]});
        let mut c = call(1, 100, "core/u64@1");
        c["type_pins"] = json!({});
        c["inputs"] = json!({"value":literal(42)});
        let mut attempt = core(3, "try");
        attempt["type_pins"] = json!({"T":"core/u64@1"});
        let mut nodes = vec![c, get(2, 10), attempt];
        let mut edges = vec![
            edge(1, 2, "value", 1, "flag"),
            edge(2, 1, "result", 3, "value"),
        ];
        let mut variables = vec![variable(10, "core/bool@1")];
        if dynamic {
            nodes.push(get(4, 11));
            edges.push(edge(3, 4, "value", 1, "value"));
            variables.push(variable(11, "core/u64@1"));
        }
        let doc = document(
            &package,
            nodes,
            edges,
            vec![export(1, "result"), export(3, "result")],
            vec![f],
            variables,
        );
        let resolver = Counting::new(package);
        let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
        assert_eq!(
            resolver.folds.load(Ordering::Relaxed),
            usize::from(!dynamic)
        );
        assert_eq!(plan.diagnostics().len(), usize::from(!dynamic));
        let mut frame =
            DataActivation::start(plan.clone(), &doc, &snapshot, 1, 0, DataOptions::default())
                .unwrap();
        for flag in [true, false, true] {
            frame.begin_epoch().unwrap();
            supplied(&mut frame, &plan, 2, FoldedValue::Value(Value::Bool(flag)));
            if dynamic {
                supplied(&mut frame, &plan, 4, FoldedValue::Value(Value::U64(42)));
            }
            if flag {
                assert_eq!(value(&mut frame, 0, 1), Value::U64(42));
            } else {
                assert_eq!(
                    frame
                        .export(0)
                        .unwrap()
                        .unwrap()
                        .outcome
                        .unwrap_err()
                        .source,
                    id(103).parse().unwrap()
                );
                let Value::Err(error) = value(&mut frame, 1, 1) else {
                    panic!("Try must capture call fault")
                };
                assert_eq!(error.node, Some(id(103).parse().unwrap()));
            }
        }
        assert_eq!(resolver.folds.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn statically_unselected_call_has_no_callee_binding_or_runtime_work() {
    let (package, snapshot) = catalog();
    let mut c = call(1, 100, "core/u64@1");
    c["inputs"] = json!({"a":literal(1),"b":literal(2)});
    let mut select = core(2, "select");
    select["type_pins"] = json!({"T":"core/u64@1"});
    select["inputs"] =
        json!({"condition":{"type":"core/bool@1","value":true},"if_true":literal(7)});
    let doc = document(
        &package,
        vec![c, select],
        vec![edge(1, 1, "result", 2, "if_false")],
        vec![export(2, "result")],
        vec![arithmetic(100, "add")],
        vec![],
    );
    let mut resolver = Counting::new(package);
    resolver.reject = Some(id(103).parse().unwrap());
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    assert_eq!(resolver.pure.load(Ordering::Relaxed), 0);
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 0);
    let mut frame =
        DataActivation::start(plan, &doc, &snapshot, 1, 0, DataOptions::default()).unwrap();
    frame.begin_epoch().unwrap();
    assert_eq!(value(&mut frame, 0, 1), Value::U64(7));
}
