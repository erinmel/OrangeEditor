use super::*;

fn intrinsic(n: u32, name: &str) -> Json {
    json!({"id":id(n),"definition_id":format!("core/{name}"),"definition_version":1,"config_version":1,"config":{}})
}
fn call(n: u32, f: u32, ty: &str, value: Json) -> Json {
    let mut node = intrinsic(n, "call");
    node["config"] = json!({"function":id(f)});
    node["type_pins"] = json!({"T":ty});
    node["inputs"] = json!({"value":value});
    node
}
fn identity(f: u32, nested: Option<u32>) -> Json {
    let mut nodes = vec![
        intrinsic(f + 1, "function-entry"),
        intrinsic(f + 2, "function-outputs"),
    ];
    let edges = if let Some(target) = nested {
        let mut c = call(
            f + 3,
            target,
            "$T",
            json!({"type":"core/i64@1","value":"999"}),
        );
        c["inputs"] = json!({});
        c["type_pins"] = json!({});
        nodes.push(c);
        vec![
            edge(f + 1, f + 1, "value", f + 3, "value"),
            edge(f + 2, f + 3, "result", f + 2, "result"),
        ]
    } else {
        vec![edge(f + 1, f + 1, "value", f + 2, "result")]
    };
    json!({"id":id(f),"name":"identity","kind":"pure","type_params":[{"name":"T"}],
        "inputs":[{"port_id":"value","direction":"input","kind":"data","type":"$T"}],
        "outputs":[{"port_id":"result","direction":"output","kind":"data","type":"$T","source":"computed"}],
        "nodes":nodes,"edges":edges})
}
fn graph_with_functions(
    package: &MathPackage,
    nodes: Vec<Json>,
    edges: Vec<Json>,
    exports: Vec<Json>,
    functions: Vec<Json>,
) -> GraphDocument {
    let base = graph(package, nodes, edges, exports);
    let mut value: Json = serde_json::from_str(&to_json(&base).unwrap()).unwrap();
    value["functions"] = json!(functions);
    let mut generic = value["signatures"]["add"].clone();
    for port in generic["ports"].as_array_mut().unwrap() {
        port["type"] = json!("$T");
    }
    value["signatures"]["add-generic"] = generic;
    from_json(&value.to_string()).unwrap()
}
fn outcomes(plan: &orange_compiler::PurePlan) -> Vec<FoldedValue> {
    plan.exports()
        .iter()
        .map(|e| plan.constants()[e.slot.index()].clone())
        .collect()
}

#[test]
fn multiple_results_alias_typed_arguments_without_copying_values() {
    let (package, snapshot) = catalog();
    let mut f = identity(100, None);
    f["type_params"] = json!([]);
    f["inputs"][0]["type"] = json!("core/i64@1");
    f["outputs"][0]["type"] = json!("core/i64@1");
    f["outputs"]
        .as_array_mut()
        .unwrap()
        .push(json!({"port_id":"again","direction":"output","kind":"data","type":"core/i64@1"}));
    f["edges"]
        .as_array_mut()
        .unwrap()
        .push(edge(102, 101, "value", 102, "again"));
    let mut extra = export(1, "again");
    extra["id"] = json!("again");
    let mut c = call(1, 100, "core/i64@1", integer_literal(7));
    c["type_pins"] = json!({});
    let doc = graph_with_functions(
        &package,
        vec![c],
        vec![],
        vec![export(1, "result"), extra],
        vec![f],
    );
    let plan = compile(&doc, snapshot.clone(), &package);
    assert_oracle(&doc, &snapshot, &plan);
    assert_eq!(plan.exports()[0].slot, plan.exports()[1].slot);
    assert_eq!(plan.constants().len(), 1);
    assert_eq!(
        outcomes(&plan),
        [
            FoldedValue::Value(Value::I64(7)),
            FoldedValue::Value(Value::I64(7))
        ]
    );
}

#[test]
fn erased_type_arguments_are_refused_instead_of_reading_saved_inference() {
    let (package, snapshot) = catalog();
    let mut f = identity(100, None);
    f["type_params"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"Unused"}));
    let mut c = call(1, 100, "core/i64@1", integer_literal(1));
    c["type_pins"]["Unused"] = json!("core/bool@1");
    c["type_args"] = json!({"T":"core/bool@1","Unused":"core/i64@1"});
    let doc = graph_with_functions(
        &package,
        vec![c],
        vec![],
        vec![export(1, "result")],
        vec![f],
    );
    let analysis = analyze(&doc, &snapshot);
    assert!(analysis.is_ready_for_lowering(&doc, &snapshot));
    let resolver = Counting::new(package);
    assert!(matches!(
        compile_pure(
            &doc,
            snapshot,
            &analysis,
            &resolver,
            CompileOptions::default()
        ),
        Err(CompileError::UnsupportedNode { .. })
    ));
    assert_eq!(resolver.resolves.load(Ordering::Relaxed), 0);
}

fn assert_oracle(
    doc: &GraphDocument,
    snapshot: &RegistrySnapshot,
    plan: &orange_compiler::PurePlan,
) {
    let analysis = analyze(doc, snapshot);
    let expected = reference::Reference::new(doc, snapshot, &analysis).exports();
    for (actual, expected) in outcomes(plan).into_iter().zip(expected) {
        match (actual, expected) {
            (FoldedValue::Value(a), Ok(b)) => assert_eq!(a, b),
            (FoldedValue::Fault(a), Err(b)) => {
                assert_eq!(a.source, b.source);
                assert_eq!(a.fault.code, b.code);
            }
            pair => panic!("source function oracle mismatch: {pair:?}"),
        }
    }
}

#[test]
fn generic_nested_calls_are_independent_and_do_not_rewrite_the_document() {
    let (package, snapshot) = catalog();
    let doc = graph_with_functions(
        &package,
        vec![
            call(1, 100, "core/i64@1", integer_literal(17)),
            call(
                2,
                100,
                "core/bool@1",
                json!({"type":"core/bool@1","value":true}),
            ),
            call(
                3,
                100,
                "core/option@1<core/i64@1>",
                json!({"type":"core/option@1<core/i64@1>","value":{"none":true}}),
            ),
        ],
        vec![],
        vec![
            export(1, "result"),
            export(2, "result"),
            export(3, "result"),
        ],
        vec![identity(100, Some(200)), identity(200, None)],
    );
    let before = to_json(&doc).unwrap();
    let plan = compile(&doc, snapshot.clone(), &package);
    assert_oracle(&doc, &snapshot, &plan);
    assert_eq!(
        outcomes(&plan),
        [
            FoldedValue::Value(Value::I64(17)),
            FoldedValue::Value(Value::Bool(true)),
            FoldedValue::Value(Value::None)
        ]
    );
    assert_eq!(plan.call_sites().len(), 6);
    for site in &plan.call_sites()[3..] {
        assert!(site.parent.is_some());
    }
    assert_eq!(to_json(&doc).unwrap(), before);
    assert!(plan.is_current(&doc, &snapshot));
    assert_eq!(
        plan.evaluate(&doc, &snapshot, EvaluationOptions::default())
            .unwrap()
            .exports()
            .count(),
        3
    );
    let data = orange_compiler::compile_data(
        &doc,
        snapshot.clone(),
        &analyze(&doc, &snapshot),
        &package,
        CompileOptions::default(),
    )
    .unwrap();
    let actual: Vec<_> = data
        .exports()
        .iter()
        .map(|e| data.initial_values()[e.slot.index()].clone().unwrap())
        .collect();
    assert_eq!(actual, outcomes(&plan));
}

#[test]
fn a_shared_body_folds_per_call_and_preserves_body_sources() {
    let (package, snapshot) = catalog();
    let mut f = identity(100, None);
    f["type_params"] = json!([{"name":"T","constraints":["num"]}]);
    f["inputs"] = json!([
        {"port_id":"a","direction":"input","kind":"data","type":"$T"},
        {"port_id":"b","direction":"input","kind":"data","type":"$T"}]);
    let mut add = node(103, "add", 0, 0);
    add["type_pins"] = json!({});
    add["inputs"] = json!({});
    add["signature"] = json!("add-generic");
    f["nodes"] = json!([
        intrinsic(101, "function-entry"),
        intrinsic(102, "function-outputs"),
        add
    ]);
    f["edges"] = json!([
        edge(101, 101, "a", 103, "a"),
        edge(102, 101, "b", 103, "b"),
        edge(103, 103, "sum", 102, "result")
    ]);
    let mut calls = Vec::new();
    for (n, a, b) in [(1, 7, 3), (2, 19, -4)] {
        let mut c = call(n, 100, "core/i64@1", integer_literal(0));
        c["inputs"] = json!({"a":integer_literal(a),"b":integer_literal(b)});
        calls.push(c);
    }
    let doc = graph_with_functions(
        &package,
        calls,
        vec![],
        vec![
            export(1, "result"),
            export(2, "result"),
            export(1, "result"),
        ],
        vec![f],
    );
    // Export IDs must remain unique.
    let mut wire: Json = serde_json::from_str(&to_json(&doc).unwrap()).unwrap();
    wire["exports"][2]["id"] = json!("shared-again");
    let doc = from_json(&wire.to_string()).unwrap();
    let resolver = Counting::new(package);
    let plan = compile(&doc, snapshot.clone(), &resolver);
    assert_oracle(&doc, &snapshot, &plan);
    assert_eq!(
        outcomes(&plan),
        [
            FoldedValue::Value(Value::I64(10)),
            FoldedValue::Value(Value::I64(15)),
            FoldedValue::Value(Value::I64(10))
        ]
    );
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 2);
    assert_eq!(plan.steps().len(), 2);
    assert_ne!(
        plan.steps()[0].call_instance(),
        plan.steps()[1].call_instance()
    );
    for step in plan.steps() {
        assert_eq!(step.node(), id(103).parse().unwrap());
    }
}

#[test]
fn input_faults_keep_original_producers_and_skip_callee_callbacks() {
    let (package, snapshot) = catalog();
    let mut c = call(2, 100, "core/i64@1", integer_literal(0));
    c["inputs"] = json!({});
    let doc = graph_with_functions(
        &package,
        vec![node(1, "divide", 1, 0), c, try_node(3, "core/i64@1", None)],
        vec![
            edge(1, 1, "quotient", 2, "value"),
            edge(2, 2, "result", 3, "value"),
        ],
        vec![export(2, "result"), export(3, "result")],
        vec![identity(100, None)],
    );
    let resolver = Counting::new(package);
    let plan = compile(&doc, snapshot.clone(), &resolver);
    assert_oracle(&doc, &snapshot, &plan);
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 1);
    let FoldedValue::Fault(fault) = &outcomes(&plan)[0] else {
        panic!("lost original fault")
    };
    assert_eq!(fault.source, id(1).parse().unwrap());
    assert_eq!(
        outcomes(&plan)[1],
        FoldedValue::Value(captured_error("core/DivideByZero", 1))
    );
}

#[test]
fn unselected_calls_are_admitted_but_not_resolved_or_folded() {
    let (package, snapshot) = catalog();
    let mut f = identity(100, None);
    f["inputs"] = json!([]);
    f["type_params"] = json!([]);
    f["outputs"][0]["type"] = json!("core/i64@1");
    f["nodes"] = json!([
        intrinsic(102, "function-outputs"),
        node(103, "divide", 1, 0)
    ]);
    f["edges"] = json!([edge(103, 103, "quotient", 102, "result")]);
    let mut c = call(2, 100, "core/i64@1", integer_literal(0));
    c["inputs"] = json!({});
    c["type_pins"] = json!({});
    let doc = graph_with_functions(
        &package,
        vec![
            select_node(
                1,
                true,
                "core/i64@1",
                integer_literal(42),
                integer_literal(0),
            ),
            c,
        ],
        vec![edge(1, 2, "result", 1, "if_false")],
        vec![export(1, "result")],
        vec![f],
    );
    let resolver = Counting::new(package);
    let plan = compile(&doc, snapshot.clone(), &resolver);
    assert_oracle(&doc, &snapshot, &plan);
    assert_eq!(outcomes(&plan), [FoldedValue::Value(Value::I64(42))]);
    assert_eq!(resolver.resolves.load(Ordering::Relaxed), 0);
    assert!(plan.diagnostics().is_empty());
    let mut options = CompileOptions::default();
    options.limits.max_fold_operations = 3;
    assert!(matches!(
        compile_pure(
            &doc,
            snapshot.clone(),
            &analyze(&doc, &snapshot),
            &resolver,
            options
        ),
        Err(CompileError::ResourceLimit)
    ));
    assert_eq!(resolver.resolves.load(Ordering::Relaxed), 0);
}

#[test]
fn deep_calls_are_iterative_and_every_limit_fails_before_callbacks() {
    let (package, snapshot) = catalog();
    let functions = (0..128)
        .map(|i| identity(100 + i * 10, (i < 127).then_some(110 + i * 10)))
        .collect();
    let doc = graph_with_functions(
        &package,
        vec![call(1, 100, "core/i64@1", integer_literal(9))],
        vec![],
        vec![export(1, "result")],
        functions,
    );
    let resolver = Counting::new(package);
    let analysis = analyze(&doc, &snapshot);
    let plan = compile(&doc, snapshot.clone(), &resolver);
    assert_eq!(outcomes(&plan), [FoldedValue::Value(Value::I64(9))]);
    assert_eq!(plan.call_sites().len(), 128);
    for limit in 0..5 {
        let mut options = CompileOptions::default();
        match limit {
            0 => options.limits.max_call_depth = 127,
            1 => options.limits.max_graph_items = 50,
            2 => options.limits.max_fold_operations = 40,
            3 => options.limits.max_slots = 40,
            _ => options.limits.max_constant_bytes = 500,
        }
        assert!(
            matches!(
                compile_pure(&doc, snapshot.clone(), &analysis, &resolver, options),
                Err(CompileError::ResourceLimit)
            ),
            "limit {limit}"
        );
    }
    let cancel = AtomicBool::new(true);
    assert!(matches!(
        compile_pure(
            &doc,
            snapshot,
            &analysis,
            &resolver,
            CompileOptions {
                cancel: Some(&cancel),
                ..Default::default()
            }
        ),
        Err(CompileError::Cancelled)
    ));
    assert_eq!(resolver.resolves.load(Ordering::Relaxed), 0);
}

#[test]
fn result_collection_preserves_the_first_fault_and_call_instance_provenance() {
    let (package, snapshot) = catalog();
    let f = json!({"id":id(100),"name":"pair","kind":"pure","inputs":[],
        "outputs":[{"port_id":"bad","direction":"output","kind":"data","type":"core/i64@1"},
                   {"port_id":"good","direction":"output","kind":"data","type":"core/i64@1"}],
        "nodes":[intrinsic(102,"function-outputs"),node(103,"divide",1,0),node(104,"add",i64::MAX,1)],
        "edges":[edge(103,103,"quotient",102,"bad"),edge(104,104,"sum",102,"good")]});
    let mut c = intrinsic(1, "call");
    c["config"] = json!({"function":id(100)});
    let mut second = export(1, "good");
    second["id"] = json!("good");
    let doc = graph_with_functions(
        &package,
        vec![c],
        vec![],
        vec![export(1, "bad"), second],
        vec![f],
    );
    let resolver = Counting::new(package);
    let plan = compile(&doc, snapshot.clone(), &resolver);
    assert_oracle(&doc, &snapshot, &plan);
    let actual = outcomes(&plan);
    let FoldedValue::Fault(fault) = &actual[0] else {
        panic!("missing callee fault")
    };
    assert_eq!(fault.source, id(103).parse().unwrap());
    assert_eq!(actual[0], actual[1]);
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 2);
    for export in plan.exports() {
        assert_eq!(plan.slot_call_instance(export.slot), Some(0));
    }
}

#[test]
fn lazy_demand_in_a_callee_skips_faulting_nodes_and_old_views_stay_frozen() {
    let (package, snapshot) = catalog();
    let mut f = identity(100, None);
    f["type_params"] = json!([]);
    f["inputs"] = json!([]);
    f["outputs"][0]["type"] = json!("core/i64@1");
    f["nodes"] = json!([
        intrinsic(102, "function-outputs"),
        select_node(
            103,
            true,
            "core/i64@1",
            integer_literal(42),
            integer_literal(0)
        ),
        node(104, "divide", 1, 0)
    ]);
    f["edges"] = json!([
        edge(103, 103, "result", 102, "result"),
        edge(104, 104, "quotient", 103, "if_false")
    ]);
    let mut c = intrinsic(1, "call");
    c["config"] = json!({"function":id(100)});
    let mut doc = graph_with_functions(
        &package,
        vec![c],
        vec![],
        vec![export(1, "result")],
        vec![f],
    );
    let resolver = Counting::new(package);
    let analysis = analyze(&doc, &snapshot);
    let plan = compile(&doc, snapshot.clone(), &resolver);
    assert_oracle(&doc, &snapshot, &plan);
    assert!(plan.diagnostics().is_empty());
    assert_eq!(resolver.resolves.load(Ordering::Relaxed), 0);
    let view = plan
        .evaluate(&doc, &snapshot, EvaluationOptions::default())
        .unwrap();
    let _edit = doc
        .apply([orange_model::edit::DocCommand::SetLiteral {
            port: serde_json::from_value(json!({"node":id(103),"port":"condition"})).unwrap(),
            literal: Some(
                serde_json::from_value(json!({"type":"core/bool@1","value":false})).unwrap(),
            ),
        }])
        .unwrap();
    assert!(matches!(
        compile_pure(
            &doc,
            snapshot.clone(),
            &analysis,
            &resolver,
            CompileOptions::default()
        ),
        Err(CompileError::StaleDocument)
    ));
    assert!(matches!(
        plan.evaluate(&doc, &snapshot, EvaluationOptions::default()),
        Err(EvaluationError::StaleDocument)
    ));
    assert_eq!(
        view.exports().next().unwrap().unwrap().outcome.unwrap(),
        &Value::I64(42)
    );
    let next = compile(&doc, snapshot.clone(), &resolver);
    assert_oracle(&doc, &snapshot, &next);
    assert!(matches!(outcomes(&next)[0], FoldedValue::Fault(_)));
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 1);
}

#[test]
fn acyclic_call_expansion_is_bounded_before_any_provider_resolution() {
    let (package, snapshot) = catalog();
    let mut functions = Vec::new();
    for i in 0..10 {
        let f = 100 + i * 10;
        let mut function = identity(f, None);
        if i < 9 {
            let mut left = call(f + 3, f + 10, "$T", integer_literal(0));
            left["type_pins"] = json!({});
            left["inputs"] = json!({});
            let mut right = left.clone();
            right["id"] = json!(id(f + 4));
            function["nodes"] = json!([
                intrinsic(f + 1, "function-entry"),
                intrinsic(f + 2, "function-outputs"),
                left,
                right
            ]);
            function["edges"] = json!([
                edge(f + 1, f + 1, "value", f + 3, "value"),
                edge(f + 2, f + 3, "result", f + 4, "value"),
                edge(f + 3, f + 4, "result", f + 2, "result")
            ]);
        }
        functions.push(function);
    }
    let doc = graph_with_functions(
        &package,
        vec![call(1, 100, "core/i64@1", integer_literal(9))],
        vec![],
        vec![export(1, "result")],
        functions,
    );
    let analysis = analyze(&doc, &snapshot);
    assert!(analysis.is_ready_for_lowering(&doc, &snapshot));
    let resolver = Counting::new(package);
    let mut options = CompileOptions::default();
    options.limits.max_fold_operations = 100;
    assert!(matches!(
        compile_pure(&doc, snapshot, &analysis, &resolver, options),
        Err(CompileError::ResourceLimit)
    ));
    assert_eq!(resolver.resolves.load(Ordering::Relaxed), 0);
}

#[test]
fn concrete_generic_provider_support_is_checked_before_any_binding() {
    let (package, snapshot) = catalog();
    let mut f = identity(100, None);
    f["type_params"] = json!([{"name":"T","constraints":["num"]}]);
    f["inputs"] = json!([
        {"port_id":"a","direction":"input","kind":"data","type":"$T"},
        {"port_id":"b","direction":"input","kind":"data","type":"$T"}]);
    let mut add = node(103, "add", 0, 0);
    add["inputs"] = json!({});
    add["signature"] = json!("add-generic");
    f["nodes"] = json!([
        intrinsic(101, "function-entry"),
        intrinsic(102, "function-outputs"),
        add
    ]);
    f["edges"] = json!([
        edge(101, 101, "a", 103, "a"),
        edge(102, 101, "b", 103, "b"),
        edge(103, 103, "sum", 102, "result")
    ]);
    // A restricted exact provider can implement only some concrete numeric types.
    let mut provider = snapshot.provider(package.provider_key()).unwrap().clone();
    for supported in provider.specializations.values_mut() {
        supported.retain(|args| args.get("T").is_none_or(|ty| ty.as_str() == "core/i64@1"));
    }
    let mut builder = RegistryBuilder::core(snapshot.origin().clone());
    for definition in math_definitions()
        .unwrap()
        .into_iter()
        .chain([orange_bundled::random_u64_definition().unwrap()])
    {
        builder = builder.definition(definition);
    }
    let snapshot = builder.provider(provider).build().unwrap();
    let mut c = call(1, 100, "core/i32@1", integer_literal(0));
    c["inputs"] = json!({"a":{"type":"core/i32@1","value":1},"b":{"type":"core/i32@1","value":2}});
    let doc = graph_with_functions(
        &package,
        vec![c],
        vec![],
        vec![export(1, "result")],
        vec![f],
    );
    let analysis = analyze(&doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(&doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    let resolver = Counting::new(package);
    assert!(
        compile_pure(
            &doc,
            snapshot,
            &analysis,
            &resolver,
            CompileOptions::default()
        )
        .is_err()
    );
    assert_eq!(resolver.resolves.load(Ordering::Relaxed), 0);
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(64))]
    #[test]
    fn nested_calls_match_independent_source_demand(
        depth in 1u32..12,
        values in proptest::collection::vec((-10000i64..10000, -3i64..4),1..7),
    ) {
        let (package,snapshot)=catalog();
        let functions=(0..depth).map(|i|identity(100+i*10,(i+1<depth).then_some(110+i*10))).collect();
        let (mut nodes,mut edges,mut exports)=(Vec::new(),Vec::new(),Vec::new());
        for (i,(a,b)) in values.into_iter().enumerate() {
            let n=i as u32*3+1;
            nodes.push(node(n,"divide",a,b));
            let mut c=call(n+1,100,"core/i64@1",integer_literal(0));c["inputs"]=json!({});nodes.push(c);
            nodes.push(try_node(n+2,"core/i64@1",None));
            edges.push(edge(n,n,"quotient",n+1,"value"));
            edges.push(edge(n+1,n+1,"result",n+2,"value"));
            exports.push(export(n+1,"result"));exports.push(export(n+2,"result"));
        }
        let doc=graph_with_functions(&package,nodes,edges,exports,functions);
        let plan=compile(&doc,snapshot.clone(),&package);
        assert_oracle(&doc,&snapshot,&plan);
    }
}

#[path = "pure_function_review_fixes.rs"]
mod review_fixes;
