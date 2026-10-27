use super::*;
fn start() -> Json {
    let mut n = core(1, "on-start");
    n["activation"] = json!({"mode":"queue","capacity":4,"overflow":"drop-newest","on_fault":"continue","order":0});
    n
}
fn action_call(n: u32, f: u32, value: u64) -> Json {
    let mut n = core(n, "call");
    n["config"] = json!({"function":id(f)});
    n["inputs"] = json!({"value":literal(value)});
    n
}
fn set(n: u32, v: u32, value: u64) -> Json {
    let mut n = core(n, "set-variable");
    n["config"] = json!({"variable":id(v)});
    n["inputs"] = json!({"value":literal(value)});
    n
}
fn initialized(n: u32, scope: &str, value: u64) -> Json {
    let mut v = variable(n, "core/u64@1");
    v["scope"] = json!(scope);
    v["init"] = literal(value);
    v
}
fn local_function(f: u32, v: u32) -> Json {
    json!({"id":id(f),"name":"local","kind":"action","inputs":[port("value","input","core/u64@1")],"outputs":[port("result","output","core/u64@1")],
        "nodes":[core(f+1,"function-entry"),set(f+3,v,0),get(f+4,v),core(f+5,"return")],
        "edges":[edge(f+1,f+1,"then",f+3,"in"),edge(f+2,f+1,"value",f+3,"value"),edge(f+3,f+3,"then",f+5,"in"),edge(f+4,f+4,"value",f+5,"result")]})
}
fn compile_plan(
    doc: &GraphDocument,
    snapshot: Arc<orange_model::catalog::RegistrySnapshot>,
    package: &orange_bundled::MathPackage,
) -> Arc<orange_compiler::ControlPlan> {
    let analysis = analyze(doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    Arc::new(compile_control(doc, snapshot, &analysis, package, CompileOptions::default()).unwrap())
}
fn drain(run: &mut Run, budget: usize) {
    for _ in 0..10000 {
        if !run.pump(budget).unwrap().ready {
            return;
        }
    }
    panic!("did not drain");
}

#[test]
fn action_call_arguments_locals_and_results_preserve_caller_scope() {
    let (package, snapshot) = catalog();
    let doc = document(
        &package,
        vec![
            start(),
            set(2, 10, 99),
            action_call(3, 100, 42),
            set(4, 11, 0),
            get(5, 10),
            set(6, 12, 0),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 3, "then", 4, "in"),
            edge(4, 3, "result", 4, "value"),
            edge(5, 4, "then", 6, "in"),
            edge(6, 5, "value", 6, "value"),
        ],
        vec![],
        vec![local_function(100, 10)],
        vec![
            initialized(10, "local", 7),
            initialized(11, "run", 0),
            initialized(12, "run", 0),
        ],
    );
    let plan = compile_plan(&doc, snapshot.clone(), &package);
    for budget in [1, 31] {
        let mut run = Run::start(plan.clone(), &doc, &snapshot, 1, RunOptions::default()).unwrap();
        drain(&mut run, budget);
        assert_eq!(run.variable(id(11).parse().unwrap()), Some(&Value::U64(42)));
        assert_eq!(run.variable(id(12).parse().unwrap()), Some(&Value::U64(99)));
    }
}

#[test]
fn repeated_and_nested_calls_get_fresh_local_cells_and_release_their_quota() {
    let (package, snapshot) = catalog();
    let mut f = local_function(100, 10);
    let mut update = core(103, "update-variable");
    update["config"] = json!({"variable":id(10),"operation":"add"});
    update["inputs"] = json!({"value":literal(1)});
    f["nodes"][1] = update;
    f["edges"] = json!([
        edge(101, 101, "then", 103, "in"),
        edge(103, 103, "then", 105, "in"),
        edge(104, 104, "value", 105, "result")
    ]);
    let mut count = core(2, "count");
    count["inputs"] = json!({"count":{"type":"core/i64@1","value":"50"}});
    let doc = document(
        &package,
        vec![start(), count, action_call(3, 100, 42), set(4, 11, 0)],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 3, "in"),
            edge(3, 3, "then", 4, "in"),
            edge(4, 3, "result", 4, "value"),
        ],
        vec![],
        vec![f],
        vec![initialized(10, "local", 7), initialized(11, "run", 0)],
    );
    let plan = compile_plan(&doc, snapshot.clone(), &package);
    for budget in [1, 31] {
        let mut options = RunOptions::default();
        options.limits.max_state_bytes = 96;
        let mut run = Run::start(plan.clone(), &doc, &snapshot, 1, options).unwrap();
        drain(&mut run, budget);
        assert_eq!(run.variable(id(11).parse().unwrap()), Some(&Value::U64(8)));
        assert_eq!(
            run.trace()
                .iter()
                .filter(|e| e.node.to_string() == id(3)
                    && e.outcome == orange_runtime::ControlOutcome::Then)
                .count(),
            50
        );
    }

    let mut outer = local_function(200, 10);
    outer["nodes"]
        .as_array_mut()
        .unwrap()
        .push(action_call(206, 100, 99));
    outer["edges"] = json!([
        edge(201, 201, "then", 203, "in"),
        edge(202, 201, "value", 203, "value"),
        edge(203, 203, "then", 206, "in"),
        edge(204, 206, "then", 205, "in"),
        edge(205, 204, "value", 205, "result")
    ]);
    let doc = document(
        &package,
        vec![start(), action_call(2, 200, 42), set(3, 11, 0)],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 2, "result", 3, "value"),
        ],
        vec![],
        vec![local_function(100, 10), outer],
        vec![initialized(10, "local", 7), initialized(11, "run", 0)],
    );
    let plan = compile_plan(&doc, snapshot.clone(), &package);
    let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
    drain(&mut run, 1);
    assert_eq!(run.variable(id(11).parse().unwrap()), Some(&Value::U64(42)));
}

fn waiting_function(f: u32) -> Json {
    let mut function = local_function(f, 10);
    function["kind"] = json!("async");
    let mut wait = core(f + 6, "wait");
    wait["inputs"] = json!({"duration":{"type":"core/duration@1","value":"5"}});
    function["nodes"].as_array_mut().unwrap().push(wait);
    function["edges"] = json!([
        edge(f + 1, f + 1, "then", f + 3, "in"),
        edge(f + 2, f + 1, "value", f + 3, "value"),
        edge(f + 3, f + 3, "then", f + 6, "in"),
        edge(f + 4, f + 6, "then", f + 5, "in"),
        edge(f + 5, f + 4, "value", f + 5, "result")
    ]);
    function
}

#[test]
fn async_calls_suspend_and_publish_only_after_their_return() {
    let (package, snapshot) = catalog();
    let doc = document(
        &package,
        vec![start(), action_call(2, 100, 42), set(3, 11, 0)],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 2, "result", 3, "value"),
        ],
        vec![],
        vec![waiting_function(100)],
        vec![initialized(10, "local", 7), initialized(11, "run", 0)],
    );
    let plan = compile_plan(&doc, snapshot.clone(), &package);
    for budget in [1, 31] {
        let mut run = Run::start(plan.clone(), &doc, &snapshot, 1, RunOptions::default()).unwrap();
        drain(&mut run, budget);
        assert_eq!(run.variable(id(11).parse().unwrap()), Some(&Value::U64(0)));
        run.advance_to(4).unwrap();
        drain(&mut run, budget);
        assert_eq!(run.variable(id(11).parse().unwrap()), Some(&Value::U64(0)));
        run.advance_to(5).unwrap();
        drain(&mut run, budget);
        assert_eq!(run.variable(id(11).parse().unwrap()), Some(&Value::U64(42)));
    }
}

fn detached_function() -> Json {
    let mut f = local_function(100, 10);
    f["kind"] = json!("async");
    let mut fork = core(106, "fork");
    fork["config"] = json!({"join":"detach"});
    fork["variadic"] = json!({"branch":{"order":[0],"next":1}});
    let mut wait = core(107, "wait");
    wait["inputs"] = json!({"duration":{"type":"core/duration@1","value":"100"}});
    f["nodes"]
        .as_array_mut()
        .unwrap()
        .extend([fork, wait, set(108, 12, 0)]);
    f["edges"] = json!([
        edge(101, 101, "then", 103, "in"),
        edge(102, 101, "value", 103, "value"),
        edge(103, 103, "then", 106, "in"),
        edge(104, 106, "branch#0", 107, "in"),
        edge(105, 106, "completed", 105, "in"),
        edge(106, 104, "value", 105, "result"),
        edge(107, 107, "cancelled", 108, "in"),
        edge(108, 104, "value", 108, "value")
    ]);
    f
}

#[test]
fn return_cancels_detached_waits_and_cleanup_keeps_function_local_context() {
    let (package, snapshot) = catalog();
    let doc = document(
        &package,
        vec![
            start(),
            action_call(2, 100, 42),
            set(3, 11, 0),
            get(4, 12),
            set(5, 13, 0),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 4, "value", 3, "value"),
            edge(4, 3, "then", 5, "in"),
            edge(5, 2, "result", 5, "value"),
        ],
        vec![],
        vec![detached_function()],
        vec![
            initialized(10, "local", 7),
            initialized(11, "run", 0),
            initialized(12, "run", 0),
            initialized(13, "run", 0),
        ],
    );
    let plan = compile_plan(&doc, snapshot.clone(), &package);
    for budget in [1, 31] {
        let mut run = Run::start(plan.clone(), &doc, &snapshot, 1, RunOptions::default()).unwrap();
        drain(&mut run, budget);
        for v in [11, 12, 13] {
            assert_eq!(run.variable(id(v).parse().unwrap()), Some(&Value::U64(42)));
        }
        assert_eq!(run.pump(1).unwrap().next_wakeup, None);
    }
}

#[test]
fn external_stop_keeps_cancelled_frame_locals_but_does_not_resume_the_caller() {
    let (package, snapshot) = catalog();
    let mut f = waiting_function(100);
    f["nodes"].as_array_mut().unwrap().push(set(107, 12, 0));
    f["edges"].as_array_mut().unwrap().extend([
        edge(106, 106, "cancelled", 107, "in"),
        edge(107, 104, "value", 107, "value"),
        edge(108, 107, "then", 105, "in"),
    ]);
    let doc = document(
        &package,
        vec![start(), action_call(2, 100, 42), set(3, 11, 99)],
        vec![edge(1, 1, "then", 2, "in"), edge(2, 2, "then", 3, "in")],
        vec![],
        vec![f],
        vec![
            initialized(10, "local", 7),
            initialized(11, "run", 0),
            initialized(12, "run", 0),
        ],
    );
    let plan = compile_plan(&doc, snapshot.clone(), &package);
    let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
    drain(&mut run, 1);
    let run = std::thread::spawn(move || {
        run.stop().unwrap();
        for _ in 0..1000 {
            if run.state() == orange_runtime::RunState::Stopped {
                break;
            }
            run.pump(1).unwrap();
        }
        run
    })
    .join()
    .unwrap();
    assert_eq!(run.variable(id(11).parse().unwrap()), Some(&Value::U64(0)));
    assert_eq!(run.variable(id(12).parse().unwrap()), Some(&Value::U64(42)));
    assert_eq!(run.state(), orange_runtime::RunState::Stopped);
    assert!(
        run.trace().iter().any(|e| e.node.to_string() == id(105)
            && e.outcome == orange_runtime::ControlOutcome::Cancelled)
    );
}

#[test]
fn concurrent_async_call_sites_have_independent_frames_and_scoped_source_ids() {
    let (package, snapshot) = catalog();
    let mut fork = core(2, "fork");
    fork["config"] = json!({"join":"all"});
    fork["variadic"] = json!({"branch":{"order":[0,1],"next":2}});
    let doc = document(
        &package,
        vec![
            start(),
            fork,
            action_call(3, 100, 42),
            action_call(4, 100, 99),
            set(5, 11, 0),
            set(6, 12, 0),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "branch#0", 3, "in"),
            edge(3, 2, "branch#1", 4, "in"),
            edge(4, 2, "completed", 5, "in"),
            edge(5, 3, "result", 5, "value"),
            edge(6, 5, "then", 6, "in"),
            edge(7, 4, "result", 6, "value"),
        ],
        vec![],
        vec![waiting_function(100)],
        vec![
            initialized(10, "local", 7),
            initialized(11, "run", 0),
            initialized(12, "run", 0),
        ],
    );
    let plan = compile_plan(&doc, snapshot.clone(), &package);
    let copies: Vec<_> = plan
        .nodes()
        .iter()
        .filter(|n| n.node().to_string() == id(103))
        .collect();
    assert_eq!(copies.len(), 2);
    assert_ne!(copies[0].scope(), copies[1].scope());
    for budget in [1, 31] {
        let mut run = Run::start(plan.clone(), &doc, &snapshot, 1, RunOptions::default()).unwrap();
        drain(&mut run, budget);
        run.advance_to(5).unwrap();
        drain(&mut run, budget);
        assert_eq!(run.variable(id(11).parse().unwrap()), Some(&Value::U64(42)));
        assert_eq!(run.variable(id(12).parse().unwrap()), Some(&Value::U64(99)));
    }
}

#[test]
fn callee_faults_follow_the_callers_failed_route_and_keep_original_producers() {
    let (package, snapshot) = catalog();
    let mut f = local_function(100, 10);
    f["nodes"]
        .as_array_mut()
        .unwrap()
        .push(math(106, "divide", 1, 0));
    f["edges"] = json!([
        edge(101, 101, "then", 103, "in"),
        edge(102, 106, "quotient", 103, "value"),
        edge(103, 103, "then", 105, "in"),
        edge(104, 104, "value", 105, "result")
    ]);
    for handled in [false, true] {
        let mut entry = start();
        entry["activation"]["on_fault"] = json!("stop-run");
        let mut edges = vec![edge(1, 1, "then", 2, "in"), edge(2, 2, "then", 3, "in")];
        if handled {
            edges.push(edge(3, 2, "failed", 4, "in"));
        }
        let doc = document(
            &package,
            vec![
                entry,
                action_call(2, 100, 42),
                set(3, 11, 99),
                set(4, 11, 77),
            ],
            edges,
            vec![],
            vec![f.clone()],
            vec![initialized(10, "local", 7), initialized(11, "run", 0)],
        );
        let plan = compile_plan(&doc, snapshot.clone(), &package);
        let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
        drain(&mut run, 1);
        assert_eq!(
            run.variable(id(11).parse().unwrap()),
            Some(&Value::U64(if handled { 77 } else { 0 }))
        );
        let event = run
            .trace()
            .iter()
            .find(|e| {
                e.node.to_string() == id(2) && e.outcome == orange_runtime::ControlOutcome::Failed
            })
            .unwrap();
        assert_eq!(event.fault.as_ref().unwrap().source.to_string(), id(106));
        assert_eq!(
            run.state(),
            if handled {
                orange_runtime::RunState::Running
            } else {
                orange_runtime::RunState::Stopped
            }
        );
    }
}

#[test]
fn frame_and_return_quota_failures_leave_caller_outputs_unpublished() {
    let (package, snapshot) = catalog();
    let doc = document(
        &package,
        vec![start(), action_call(2, 100, 42), set(3, 11, 0)],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 2, "result", 3, "value"),
        ],
        vec![],
        vec![local_function(100, 10)],
        vec![initialized(10, "local", 7), initialized(11, "run", 0)],
    );
    let plan = compile_plan(&doc, snapshot.clone(), &package);
    for limit in [32, 56] {
        let mut options = RunOptions::default();
        options.limits.max_state_bytes = limit;
        let mut run = Run::start(plan.clone(), &doc, &snapshot, 1, options).unwrap();
        let mut refusal = false;
        for _ in 0..1000 {
            match run.pump(1) {
                Ok(_) => {}
                Err(orange_runtime::RunError::ResourceLimit) => {
                    refusal = true;
                    break;
                }
                Err(error) => panic!("unexpected {error}"),
            }
        }
        assert!(refusal);
        assert_eq!(run.variable(id(11).parse().unwrap()), Some(&Value::U64(0)));
        assert_eq!(run.state(), orange_runtime::RunState::Stopped);
        run.dispose().unwrap();
    }
}

#[test]
fn outputless_functions_fall_through_and_fork_join_paths_still_require_return() {
    let (package, snapshot) = catalog();
    for entry in [false, true] {
        let nodes = if entry {
            vec![core(101, "function-entry"), set(103, 10, 8)]
        } else {
            vec![set(103, 10, 8)]
        };
        let f = json!({"id":id(100),"name":"outputless","kind":"action","inputs":[],"outputs":[],"nodes":nodes,"edges":if entry {vec![edge(101,101,"then",103,"in")]}else{vec![]}});
        let mut call = action_call(2, 100, 0);
        call["inputs"] = json!({});
        let doc = document(
            &package,
            vec![start(), call, set(3, 11, 9)],
            vec![edge(1, 1, "then", 2, "in"), edge(2, 2, "then", 3, "in")],
            vec![],
            vec![f],
            vec![initialized(10, "run", 7), initialized(11, "run", 0)],
        );
        let plan = compile_plan(&doc, snapshot.clone(), &package);
        let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
        drain(&mut run, 1);
        assert_eq!(
            run.variable(id(10).parse().unwrap()),
            Some(&Value::U64(if entry { 8 } else { 7 }))
        );
        assert_eq!(run.variable(id(11).parse().unwrap()), Some(&Value::U64(9)));
    }
    let mut f = detached_function();
    f["edges"]
        .as_array_mut()
        .unwrap()
        .retain(|e| e["id"] != edge(105, 106, "completed", 105, "in")["id"]);
    let doc = document(
        &package,
        vec![start(), action_call(2, 100, 42)],
        vec![edge(1, 1, "then", 2, "in")],
        vec![],
        vec![f],
        vec![initialized(10, "local", 7), initialized(12, "run", 0)],
    );
    let analysis = analyze(&doc, &snapshot);
    assert!(analysis.report().diagnostics.iter().any(|d| d.code
        == orange_model::validation::DiagnosticCode::MissingReturn
        && d.subject.node == Some(id(106).parse().unwrap())));
}

#[test]
fn generic_action_interfaces_specialize_independently_at_two_call_sites() {
    let (package, snapshot) = catalog();
    let mut gate = core(103, "gate");
    gate["config"] = json!({"start_open":true});
    let f = json!({"id":id(100),"name":"generic","kind":"action","type_params":[{"name":"T"}],"inputs":[port("value","input","$T")],"outputs":[port("result","output","$T")],"nodes":[core(101,"function-entry"),gate,core(105,"return")],"edges":[edge(101,101,"then",103,"in"),edge(102,103,"then",105,"in"),edge(103,101,"value",105,"result")]});
    let mut a = call(2, 100, "core/u64@1");
    a["inputs"] = json!({"value":literal(42)});
    let mut b = call(3, 100, "core/bool@1");
    b["inputs"] = json!({"value":{"type":"core/bool@1","value":true}});
    let doc = document(
        &package,
        vec![start(), a, b, set(4, 11, 0), core(5, "branch")],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 3, "then", 4, "in"),
            edge(4, 2, "result", 4, "value"),
            edge(5, 4, "then", 5, "in"),
            edge(6, 3, "result", 5, "condition"),
        ],
        vec![],
        vec![f],
        vec![initialized(11, "run", 0)],
    );
    let plan = compile_plan(&doc, snapshot.clone(), &package);
    let gates: Vec<_> = plan
        .nodes()
        .iter()
        .filter_map(|node| match node.operation() {
            orange_compiler::ControlOperation::Call { scope, .. } => {
                let entry = plan.function(*scope)?.entry()?;
                plan.nodes()[entry.index()]
                    .then()
                    .map(|target| target.node())
            }
            _ => None,
        })
        .collect();
    let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
    drain(&mut run, 1);
    assert_eq!(run.variable(id(11).parse().unwrap()), Some(&Value::U64(42)));
    assert!(
        run.trace()
            .iter()
            .any(|e| e.node.to_string() == id(5)
                && e.outcome == orange_runtime::ControlOutcome::True)
    );
    assert_eq!(run.node_state(id(103).parse().unwrap()), None);
    assert_eq!(gates.len(), 2);
    for gate in gates {
        assert_eq!(run.control_state(gate), Some(&Value::Bool(true)));
    }
}

#[test]
fn a_fault_in_the_return_collection_does_not_publish_earlier_results() {
    let (package, snapshot) = catalog();
    let mut f = local_function(100, 10);
    f["outputs"]
        .as_array_mut()
        .unwrap()
        .push(port("second", "output", "core/u64@1"));
    f["nodes"].as_array_mut().unwrap().push(get(106, 14));
    f["edges"]
        .as_array_mut()
        .unwrap()
        .push(edge(106, 106, "value", 105, "second"));
    let doc = document(
        &package,
        vec![
            start(),
            action_call(2, 100, 42),
            set(3, 11, 0),
            set(4, 12, 77),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 2, "result", 3, "value"),
            edge(4, 2, "failed", 4, "in"),
        ],
        vec![],
        vec![f],
        vec![
            initialized(10, "local", 7),
            initialized(11, "run", 0),
            initialized(12, "run", 0),
            variable(14, "core/u64@1"),
        ],
    );
    let plan = compile_plan(&doc, snapshot.clone(), &package);
    let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
    drain(&mut run, 1);
    assert_eq!(run.variable(id(11).parse().unwrap()), Some(&Value::U64(0)));
    assert_eq!(run.variable(id(12).parse().unwrap()), Some(&Value::U64(77)));
    let fault = run
        .trace()
        .iter()
        .find(|e| e.node.to_string() == id(2) && e.fault.is_some())
        .unwrap()
        .fault
        .as_ref()
        .unwrap();
    assert_eq!(fault.source.to_string(), id(106));
    assert_eq!(fault.fault.code, "core/UninitializedVariable");
}

#[test]
fn nested_detached_call_frames_release_quota_after_return_cleanup() {
    let (package, snapshot) = catalog();
    let mut inner = waiting_function(100);
    inner["nodes"].as_array_mut().unwrap().push(set(107, 12, 0));
    inner["edges"].as_array_mut().unwrap().extend([
        edge(106, 106, "cancelled", 107, "in"),
        edge(107, 104, "value", 107, "value"),
    ]);
    let mut outer = local_function(200, 10);
    outer["kind"] = json!("async");
    let mut fork = core(206, "fork");
    fork["config"] = json!({"join":"detach"});
    fork["variadic"] = json!({"branch":{"order":[0],"next":1}});
    outer["nodes"]
        .as_array_mut()
        .unwrap()
        .extend([fork, action_call(207, 100, 99)]);
    outer["edges"] = json!([
        edge(201, 201, "then", 203, "in"),
        edge(202, 201, "value", 203, "value"),
        edge(203, 203, "then", 206, "in"),
        edge(204, 206, "branch#0", 207, "in"),
        edge(205, 206, "completed", 205, "in"),
        edge(206, 204, "value", 205, "result")
    ]);
    let mut count = core(2, "count");
    count["inputs"] = json!({"count":{"type":"core/i64@1","value":"50"}});
    let doc = document(
        &package,
        vec![start(), count, action_call(3, 200, 42), set(4, 11, 0)],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 3, "in"),
            edge(3, 3, "then", 4, "in"),
            edge(4, 3, "result", 4, "value"),
        ],
        vec![],
        vec![inner, outer],
        vec![
            initialized(10, "local", 7),
            initialized(11, "run", 0),
            initialized(12, "run", 0),
        ],
    );
    let plan = compile_plan(&doc, snapshot.clone(), &package);
    let mut options = RunOptions::default();
    options.limits.max_state_bytes = 128;
    let mut run = Run::start(plan, &doc, &snapshot, 1, options).unwrap();
    drain(&mut run, 1);
    assert_eq!(run.variable(id(11).parse().unwrap()), Some(&Value::U64(42)));
    assert_eq!(run.variable(id(12).parse().unwrap()), Some(&Value::U64(99)));
    assert_eq!(
        run.trace()
            .iter()
            .filter(|e| e.node.to_string() == id(3)
                && e.outcome == orange_runtime::ControlOutcome::Then)
            .count(),
        50
    );
}

#[test]
fn ordered_arguments_share_one_epoch_and_faults_precede_body_work() {
    let (package, snapshot) = catalog();
    let mut gate = core(103, "gate");
    gate["config"] = json!({"start_open":true});
    let f = json!({"id":id(100),"name":"pair","kind":"action","inputs":[port("a","input","core/u64@1"),port("b","input","core/u64@1")],"outputs":[port("first","output","core/u64@1"),port("second","output","core/u64@1")],"nodes":[core(101,"function-entry"),gate,core(105,"return")],"edges":[edge(101,101,"then",103,"in"),edge(102,103,"then",105,"in"),edge(103,101,"a",105,"first"),edge(104,101,"b",105,"second")]});
    for fault in [false, true] {
        let mut c = action_call(2, 100, 0);
        c["inputs"] = json!({});
        let mut nodes = vec![start(), c, set(3, 11, 0), set(4, 12, 0), set(5, 13, 77)];
        let mut edges = vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 2, "first", 3, "value"),
            edge(4, 3, "then", 4, "in"),
            edge(5, 2, "second", 4, "value"),
            edge(6, 2, "failed", 5, "in"),
        ];
        if fault {
            nodes.extend([math(90, "divide", 1, 0), math(80, "divide", 2, 0)]);
            edges.extend([
                edge(7, 90, "quotient", 2, "a"),
                edge(8, 80, "quotient", 2, "b"),
            ]);
        } else {
            nodes.push(random(90));
            edges.extend([edge(7, 90, "value", 2, "a"), edge(8, 90, "value", 2, "b")]);
        }
        let doc = document(
            &package,
            nodes,
            edges,
            vec![],
            vec![f.clone()],
            vec![
                initialized(11, "run", 0),
                initialized(12, "run", 0),
                initialized(13, "run", 0),
            ],
        );
        let analysis = analyze(&doc, &snapshot);
        assert!(
            analysis.is_ready_for_lowering(&doc, &snapshot),
            "{:?}",
            analysis.report().diagnostics
        );
        let resolver = Counting::new(package.clone());
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
        drain(&mut run, 1);
        if fault {
            let event = run
                .trace()
                .iter()
                .find(|e| e.node.to_string() == id(2) && e.fault.is_some())
                .unwrap();
            assert_eq!(event.fault.as_ref().unwrap().source.to_string(), id(90));
            assert!(!run.trace().iter().any(|e| e.node.to_string() == id(103)));
            assert_eq!(run.variable(id(13).parse().unwrap()), Some(&Value::U64(77)));
        } else {
            assert_eq!(
                run.variable(id(11).parse().unwrap()),
                run.variable(id(12).parse().unwrap())
            );
            assert_eq!(resolver.draws.load(Ordering::Relaxed), 1);
        }
    }
}

#[test]
fn a_pending_action_call_keeps_its_actor_segment_across_one_unit_pumps() {
    let (package, snapshot) = catalog();
    let mut f = local_function(100, 12);
    f["nodes"].as_array_mut().unwrap().push(get(106, 10));
    f["edges"] = json!([
        edge(101, 101, "then", 103, "in"),
        edge(102, 106, "value", 103, "value"),
        edge(103, 103, "then", 105, "in"),
        edge(104, 104, "value", 105, "result")
    ]);
    let mut other = start();
    other["id"] = json!(id(20));
    other["activation"]["order"] = json!(1);
    let doc = document(
        &package,
        vec![
            start(),
            action_call(2, 100, 0),
            call(3, 200, "core/u64@1"),
            get(4, 10),
            other,
            set(21, 10, 99),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 3, "result", 2, "value"),
            edge(3, 4, "value", 3, "value"),
            edge(4, 20, "then", 21, "in"),
        ],
        vec![],
        vec![f, identity(200, Some(300)), identity(300, None)],
        vec![initialized(10, "run", 7), initialized(12, "run", 0)],
    );
    let plan = compile_plan(&doc, snapshot.clone(), &package);
    for budget in [1, 31] {
        let mut run = Run::start(plan.clone(), &doc, &snapshot, 1, RunOptions::default()).unwrap();
        drain(&mut run, budget);
        assert_eq!(run.variable(id(12).parse().unwrap()), Some(&Value::U64(7)));
        assert_eq!(run.variable(id(10).parse().unwrap()), Some(&Value::U64(99)));
    }
}

#[test]
fn suppressed_function_fallthrough_is_rejected_before_lowering() {
    let (package, snapshot) = catalog();
    let f = json!({"id":id(100),"name":"once","kind":"action","inputs":[port("value","input","core/u64@1")],"outputs":[port("result","output","core/u64@1")],"nodes":[core(101,"function-entry"),core(103,"do-once"),core(105,"return")],"edges":[edge(101,101,"then",103,"in"),edge(102,103,"then",105,"in"),edge(103,101,"value",105,"result")]});
    let mut count = core(2, "count");
    count["inputs"] = json!({"count":{"type":"core/i64@1","value":"2"}});
    let doc = document(
        &package,
        vec![
            start(),
            count,
            action_call(3, 100, 42),
            set(4, 11, 0),
            set(5, 12, 77),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "body", 3, "in"),
            edge(3, 3, "then", 4, "in"),
            edge(4, 3, "result", 4, "value"),
            edge(5, 3, "failed", 5, "in"),
        ],
        vec![],
        vec![f],
        vec![initialized(11, "run", 0), initialized(12, "run", 0)],
    );
    let analysis = analyze(&doc, &snapshot);
    assert!(!analysis.is_ready_for_lowering(&doc, &snapshot));
    assert!(analysis.report().diagnostics.iter().any(|d| d.code
        == orange_model::validation::DiagnosticCode::MissingReturn
        && d.subject.node == Some(id(103).parse().unwrap())));
    let resolver = Counting::new(package);
    assert!(matches!(
        compile_control(
            &doc,
            snapshot,
            &analysis,
            &resolver,
            CompileOptions::default()
        ),
        Err(CompileError::ValidationIncomplete)
    ));
    assert_eq!(resolver.pure.load(Ordering::Relaxed), 0);
}

#[test]
fn sparse_sequence_completion_and_early_return_follow_real_region_continuations() {
    let (package, snapshot) = catalog();
    for early in [false, true] {
        let mut sequence = core(106, "sequence");
        sequence["variadic"] = json!({"then":{"order":[5,1],"next":6}});
        let nodes = if early {
            vec![
                core(101, "function-entry"),
                core(105, "return"),
                sequence,
                set(103, 12, 99),
            ]
        } else {
            vec![
                core(101, "function-entry"),
                core(105, "return"),
                sequence,
                core(103, "do-once"),
            ]
        };
        let edges = if early {
            vec![
                edge(101, 101, "then", 106, "in"),
                edge(102, 106, "then#5", 105, "in"),
                edge(103, 106, "then#1", 103, "in"),
                edge(104, 101, "value", 105, "result"),
            ]
        } else {
            vec![
                edge(101, 101, "then", 106, "in"),
                edge(102, 106, "then#5", 103, "in"),
                edge(103, 106, "then#1", 105, "in"),
                edge(104, 101, "value", 105, "result"),
            ]
        };
        let f = json!({"id":id(100),"name":"sequence","kind":"action","inputs":[port("value","input","core/u64@1")],"outputs":[port("result","output","core/u64@1")],"nodes":nodes,"edges":edges});
        let mut count = core(2, "count");
        count["inputs"] = json!({"count":{"type":"core/i64@1","value":"2"}});
        let doc = document(
            &package,
            vec![start(), count, action_call(3, 100, 42), set(4, 11, 0)],
            vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 2, "body", 3, "in"),
                edge(3, 3, "then", 4, "in"),
                edge(4, 3, "result", 4, "value"),
            ],
            vec![],
            vec![f],
            vec![initialized(11, "run", 0), initialized(12, "run", 0)],
        );
        let plan = compile_plan(&doc, snapshot.clone(), &package);
        let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
        drain(&mut run, 1);
        assert_eq!(run.variable(id(11).parse().unwrap()), Some(&Value::U64(42)));
        if early {
            assert_eq!(run.variable(id(12).parse().unwrap()), Some(&Value::U64(0)));
        }
        assert_eq!(
            run.trace()
                .iter()
                .filter(|e| e.node.to_string() == id(3)
                    && e.outcome == orange_runtime::ControlOutcome::Then)
                .count(),
            2
        );
    }
}

#[test]
fn scoped_state_inspection_and_source_write_intents_do_not_alias_call_sites() {
    let (package, snapshot) = catalog();
    let f = json!({"id":id(100),"name":"open","kind":"action","inputs":[port("value","input","core/u64@1")],"outputs":[],"nodes":[core(101,"function-entry"),core(103,"gate")],"edges":[edge(101,101,"then",103,"open")]});
    let doc = document(
        &package,
        vec![start(), action_call(2, 100, 42), action_call(3, 100, 99)],
        vec![edge(1, 1, "then", 2, "in"), edge(2, 2, "then", 3, "in")],
        vec![],
        vec![f],
        vec![],
    );
    let plan = compile_plan(&doc, snapshot.clone(), &package);
    let gates: Vec<_> = plan
        .nodes()
        .iter()
        .filter_map(|node| match node.operation() {
            orange_compiler::ControlOperation::Call { scope, .. } => {
                let entry = plan.function(*scope)?.entry()?;
                plan.nodes()[entry.index()]
                    .then()
                    .map(|target| target.node())
            }
            _ => None,
        })
        .collect();
    let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
    drain(&mut run, 1);
    assert_eq!(gates.len(), 2);
    for gate in gates {
        assert_eq!(run.control_state(gate), Some(&Value::Bool(true)));
    }
    assert_eq!(run.node_state(id(103).parse().unwrap()), None);
    assert_eq!(
        run.segment_state_writes().unwrap().1,
        &[id(103).parse().unwrap()]
    );
}

#[test]
fn returned_function_execution_projections_release_before_the_next_call() {
    let (package, snapshot) = catalog();
    let ty = "core/string@1";
    let mut update = core(103, "update-variable");
    update["config"] = json!({"variable":id(10),"operation":"append"});
    let f = json!({"id":id(100),"name":"append","kind":"action","inputs":[port("value","input",ty)],"outputs":[port("result","output",ty)],"nodes":[core(101,"function-entry"),update,core(105,"return")],"edges":[edge(101,101,"then",103,"in"),edge(102,101,"value",103,"value"),edge(103,103,"then",105,"in"),edge(104,103,"new",105,"result")]});
    let mut a = action_call(2, 100, 0);
    a["inputs"] = json!({"value":{"type":ty,"value":"a".repeat(40)}});
    let mut b = action_call(4, 100, 0);
    b["inputs"] = json!({"value":{"type":ty,"value":"b".repeat(40)}});
    let variables = [(10, "local"), (11, "run"), (12, "run")]
        .map(|(n, scope)| {
            let mut v = variable(n, ty);
            v["scope"] = json!(scope);
            v["init"] = json!({"type":ty,"value":""});
            v
        })
        .to_vec();
    let doc = document(
        &package,
        vec![start(), a, set(3, 11, 0), b, set(5, 12, 0)],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 2, "result", 3, "value"),
            edge(4, 3, "then", 4, "in"),
            edge(5, 4, "then", 5, "in"),
            edge(6, 4, "result", 5, "value"),
        ],
        vec![],
        vec![f],
        variables,
    );
    let plan = compile_plan(&doc, snapshot.clone(), &package);
    let mut options = RunOptions::default();
    options.limits.max_state_bytes = 320;
    let mut run = Run::start(plan, &doc, &snapshot, 1, options).unwrap();
    drain(&mut run, 1);
    assert_eq!(
        run.variable(id(11).parse().unwrap()),
        Some(&Value::Str(Arc::new("a".repeat(40))))
    );
    assert_eq!(
        run.variable(id(12).parse().unwrap()),
        Some(&Value::Str(Arc::new("b".repeat(40))))
    );
}
