use super::*;
use orange_compiler::HostSource;

fn start() -> Json {
    let mut node = core(1, "on-start");
    node["activation"] = json!({"mode":"queue","capacity":4,"overflow":"drop-newest","on_fault":"continue","order":0});
    node
}
fn state_call(n: u32, f: u32) -> Json {
    let mut node = core(n, "call");
    node["config"] = json!({"function":id(f)});
    node
}
fn reader(f: u32, variable: u32, nested: Option<u32>) -> Json {
    let producer = nested.map_or_else(|| get(f + 3, variable), |target| state_call(f + 3, target));
    json!({"id":id(f),"name":"read","kind":"volatile","inputs":[],
        "outputs":[port("result","output","core/u64@1")],
        "nodes":[core(f+2,"function-outputs"),producer],
        "edges":[edge(f+1,f+3,if nested.is_some(){"result"}else{"value"},f+2,"result")]})
}
fn initialized(n: u32, scope: &str, value: u64) -> Json {
    let mut v = variable(n, "core/u64@1");
    v["scope"] = json!(scope);
    v["init"] = literal(value);
    v
}
fn set(n: u32, variable: u32, value: u64) -> Json {
    let mut node = core(n, "set-variable");
    node["config"] = json!({"variable":id(variable)});
    node["inputs"] = json!({"value":literal(value)});
    node
}
fn control(
    doc: &GraphDocument,
    snapshot: Arc<orange_model::catalog::RegistrySnapshot>,
    resolver: &Counting,
) -> Arc<orange_compiler::ControlPlan> {
    let analysis = analyze(doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    Arc::new(
        compile_control(
            doc,
            snapshot,
            &analysis,
            resolver,
            CompileOptions::default(),
        )
        .unwrap(),
    )
}
fn finish(run: &mut Run, budget: usize) {
    for _ in 0..100_000 {
        let state = run.pump(budget).unwrap();
        if !state.ready {
            return;
        }
    }
    panic!("control did not finish");
}

#[test]
fn nested_state_calls_follow_an_independent_recurrence_across_consumption_epochs() {
    let (package, snapshot) = catalog();
    for budget in [1, 3, 31] {
        let mut count = core(2, "count");
        count["inputs"] = json!({"count":{"type":"core/i64@1","value":"24"}});
        let mut add = math(6, "add", 0, 0);
        add["inputs"] = json!({});
        let doc = document(
            &package,
            vec![
                start(),
                count,
                set(3, 10, 0),
                state_call(4, 100),
                state_call(5, 100),
                add,
            ],
            vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 2, "body", 3, "in"),
                edge(3, 4, "result", 6, "a"),
                edge(4, 5, "result", 6, "b"),
                edge(5, 6, "sum", 3, "value"),
            ],
            vec![],
            vec![reader(100, 10, Some(200)), reader(200, 10, None)],
            vec![initialized(10, "run", 3)],
        );
        let resolver = Counting::new(package.clone());
        let plan = control(&doc, snapshot.clone(), &resolver);
        assert_eq!(plan.data().host_inputs().len(), 2);
        assert!(
            plan.host_sources()
                .iter()
                .all(|s| matches!(s, HostSource::Variable(_)))
        );
        for _ in 0..2 {
            let mut run =
                Run::start(plan.clone(), &doc, &snapshot, 1, RunOptions::default()).unwrap();
            finish(&mut run, budget);
            let expected = (0..24).fold(3u64, |value, _| value + value);
            assert_eq!(
                run.variable(id(10).parse().unwrap()),
                Some(&Value::U64(expected))
            );
            assert!(!run.trace().iter().any(|e| e.fault.is_some()));
        }
        assert_eq!(resolver.entropy.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn function_locals_use_initial_values_while_root_locals_keep_committed_writes() {
    let (package, snapshot) = catalog();
    for budget in [1, 19] {
        let doc = document(
            &package,
            vec![
                start(),
                set(2, 10, 99),
                set(3, 11, 0),
                set(4, 12, 0),
                get(5, 10),
                state_call(6, 100),
            ],
            vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 2, "then", 3, "in"),
                edge(3, 3, "then", 4, "in"),
                edge(4, 6, "result", 3, "value"),
                edge(5, 5, "value", 4, "value"),
            ],
            vec![],
            vec![reader(100, 10, Some(200)), reader(200, 10, None)],
            vec![
                initialized(10, "local", 7),
                initialized(11, "run", 0),
                initialized(12, "run", 0),
            ],
        );
        let resolver = Counting::new(package.clone());
        let plan = control(&doc, snapshot.clone(), &resolver);
        assert!(
            plan.host_sources()
                .iter()
                .any(|s| matches!(s, HostSource::CallLocal(_)))
        );
        let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
        finish(&mut run, budget);
        assert_eq!(run.variable(id(11).parse().unwrap()), Some(&Value::U64(7)));
        assert_eq!(run.variable(id(12).parse().unwrap()), Some(&Value::U64(99)));
    }
}

#[test]
fn data_host_inputs_keep_call_instance_identity_and_do_not_carry_between_epochs() {
    let (package, snapshot) = catalog();
    let doc = document(
        &package,
        vec![state_call(1, 100), state_call(2, 100)],
        vec![],
        vec![export(1, "result"), export(2, "result")],
        vec![reader(100, 10, None)],
        vec![variable(10, "core/u64@1")],
    );
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let host = plan.host_inputs();
    assert_eq!(host.len(), 2);
    assert_ne!(
        plan.nodes()[host[0].index()].call_instance(),
        plan.nodes()[host[1].index()].call_instance()
    );
    let mut data =
        DataActivation::start(plan, &doc, &snapshot, 1, 0, DataOptions::default()).unwrap();
    for epoch in 0..16 {
        data.begin_epoch().unwrap();
        data.set_input(0, Value::U64(epoch)).unwrap();
        data.set_input(1, Value::U64(epoch + 100)).unwrap();
        assert_eq!(value(&mut data, 0, 1), Value::U64(epoch));
        assert_eq!(value(&mut data, 1, 3), Value::U64(epoch + 100));
    }
    data.begin_epoch().unwrap();
    assert!(
        matches!(data.export(0),Err(DataError::MissingInput{node}) if node.to_string()==id(103))
    );
}

#[test]
fn uninitialized_called_reads_keep_original_fault_identity_and_try_catches_them() {
    let (package, snapshot) = catalog();
    for scope in ["local", "run"] {
        let mut v = variable(10, "core/u64@1");
        v["scope"] = json!(scope);
        let mut attempt = core(4, "try");
        attempt["type_pins"] = json!({"T":"core/u64@1"});
        let result_ty = "core/result@1<core/u64@1>";
        let mut out = variable(11, result_ty);
        out["scope"] = json!("run");
        let doc = document(
            &package,
            vec![start(), set(2, 11, 0), state_call(3, 100), attempt],
            vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 3, "result", 4, "value"),
                edge(3, 4, "result", 2, "value"),
            ],
            vec![],
            vec![reader(100, 10, None)],
            vec![v, out],
        );
        let resolver = Counting::new(package.clone());
        let plan = control(&doc, snapshot.clone(), &resolver);
        let mut run = Run::start(plan, &doc, &snapshot, 1, RunOptions::default()).unwrap();
        finish(&mut run, 1);
        let Some(Value::Err(error)) = run.variable(id(11).parse().unwrap()) else {
            panic!("missing caught fault")
        };
        assert_eq!(error.code, "core/UninitializedVariable");
        assert_eq!(error.node, Some(id(103).parse().unwrap()));
    }
}

#[test]
fn inactive_state_call_does_not_demand_missing_host_input() {
    let (package, snapshot) = catalog();
    let mut select = core(2, "select");
    select["type_pins"] = json!({"T":"core/u64@1"});
    select["inputs"] =
        json!({"condition":{"type":"core/bool@1","value":true},"if_true":literal(9)});
    let doc = document(
        &package,
        vec![state_call(1, 100), select],
        vec![edge(1, 1, "result", 2, "if_false")],
        vec![export(2, "result")],
        vec![reader(100, 10, None)],
        vec![variable(10, "core/u64@1")],
    );
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    assert!(plan.host_inputs().is_empty());
    let mut data =
        DataActivation::start(plan, &doc, &snapshot, 1, 0, DataOptions::default()).unwrap();
    data.begin_epoch().unwrap();
    assert_eq!(value(&mut data, 0, 1), Value::U64(9));
}

#[test]
fn strict_faulting_argument_skips_the_body_state_demand() {
    let (package, snapshot) = catalog();
    let mut function = reader(100, 10, None);
    function["inputs"] = json!([port("unused", "input", "core/u64@1")]);
    function["nodes"]
        .as_array_mut()
        .unwrap()
        .push(core(101, "function-entry"));
    let doc = document(
        &package,
        vec![state_call(1, 100), math(2, "divide", 1, 0)],
        vec![edge(1, 2, "quotient", 1, "unused")],
        vec![export(1, "result")],
        vec![function],
        vec![variable(10, "core/u64@1")],
    );
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    assert_eq!(plan.host_inputs().len(), 1);
    let mut data =
        DataActivation::start(plan, &doc, &snapshot, 1, 0, DataOptions::default()).unwrap();
    data.begin_epoch().unwrap();
    let fault = data.export(0).unwrap().unwrap().outcome.unwrap_err();
    assert_eq!(fault.source.to_string(), id(2));
    assert_eq!(fault.fault.code, "core/DivideByZero");
}

#[test]
fn workspace_state_in_potential_callees_is_refused_before_any_kernel_binding() {
    let (package, snapshot) = catalog();
    let mut f = reader(100, 10, None);
    f["nodes"]
        .as_array_mut()
        .unwrap()
        .push(math(104, "add", 1, 2));
    let mut v = initialized(10, "workspace", 7);
    v["persistent"] = json!(false);
    let doc = document(
        &package,
        vec![start(), set(2, 11, 0), state_call(3, 100)],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 3, "result", 2, "value"),
        ],
        vec![],
        vec![f],
        vec![v, initialized(11, "run", 0)],
    );
    let analysis = analyze(&doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(&doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    let resolver = Counting::new(package);
    assert!(
        matches!(compile_control(&doc,snapshot,&analysis,&resolver,CompileOptions::default()),Err(CompileError::UnsupportedNode{node}) if node.to_string()==id(103))
    );
    assert_eq!(resolver.pure.load(Ordering::Relaxed), 0);
    assert_eq!(resolver.entropy.load(Ordering::Relaxed), 0);
}
