use super::*;
use orange_runtime::{EntropyKey, EntropyStream, FrameId};
fn volatile_call(n: u32, f: u32) -> Json {
    let mut c = core(n, "call");
    c["config"] = json!({"function":id(f)});
    c
}
fn entropy_function(f: u32, nested: Option<u32>) -> Json {
    let producer = if let Some(target) = nested {
        volatile_call(f + 3, target)
    } else {
        random(f + 3)
    };
    json!({"id":id(f),"name":"seeded","kind":"volatile","inputs":[],"outputs":[port("result","output","core/u64@1")],
        "nodes":[core(f+2,"function-outputs"),producer],"edges":[edge(f+1,f+3,if nested.is_some(){"result"}else{"value"},f+2,"result")]})
}
fn expected(path: &[u32], node: u32, invocation: u64) -> Value {
    let frame = path.iter().fold(FrameId::ROOT, |frame, n| {
        frame.call(id(*n).parse().unwrap())
    });
    Value::U64(
        EntropyStream::new(EntropyKey {
            seed: 42,
            frame,
            node: id(node).parse().unwrap(),
            activation_sequence: 7,
            invocation,
        })
        .next_u64()
        .unwrap(),
    )
}

#[test]
fn flat_and_nested_calls_match_independent_sha_vectors_and_keep_root_replay() {
    let (package, snapshot) = catalog();
    let doc = document(
        &package,
        vec![
            volatile_call(1, 100),
            volatile_call(2, 100),
            volatile_call(3, 200),
            random(4),
        ],
        vec![],
        vec![
            export(1, "result"),
            export(2, "result"),
            export(3, "result"),
            export(4, "value"),
        ],
        vec![
            entropy_function(100, None),
            entropy_function(200, Some(100)),
        ],
        vec![],
    );
    let before = to_json(&doc).unwrap();
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    assert_eq!(plan.resources().entropy_nodes, 4);
    assert_eq!(resolver.draws.load(Ordering::Relaxed), 0);
    // Independent Python hashlib/UUID bytes/LE-u64 vectors, not runtime output.
    let vectors = [
        [
            7127809647224613403,
            16439378587780866589,
            5407170047477787437,
            13342201968877971713,
        ],
        [
            5096851514245061343,
            17718613777076706527,
            12337760647887203092,
            1729434771204628782,
        ],
    ];
    let mut frames = (0..2)
        .map(|_| {
            DataActivation::start(plan.clone(), &doc, &snapshot, 42, 7, DataOptions::default())
                .unwrap()
        })
        .collect::<Vec<_>>();
    for (epoch, words) in vectors.into_iter().enumerate() {
        for frame in &mut frames {
            frame.begin_epoch().unwrap();
            for (i, word) in words.into_iter().enumerate() {
                let actual = value(frame, i, 1);
                assert_eq!(actual, Value::U64(word));
                assert_eq!(value(frame, i, 2), actual);
            }
        }
        assert_eq!(resolver.draws.load(Ordering::Relaxed), (epoch + 1) * 8);
    }
    assert_eq!(before, to_json(&doc).unwrap());
}

#[test]
fn skipped_epochs_and_unused_body_nodes_do_not_consume_invocations() {
    let (package, snapshot) = catalog();
    let mut f = entropy_function(100, None);
    f["nodes"].as_array_mut().unwrap().push(random(104));
    let mut select = core(4, "select");
    select["type_pins"] = json!({"T":"core/u64@1"});
    select["inputs"] = json!({"if_false":literal(7)});
    let doc = document(
        &package,
        vec![
            volatile_call(1, 100),
            volatile_call(2, 100),
            select,
            get(5, 10),
        ],
        vec![
            edge(1, 2, "result", 4, "if_true"),
            edge(2, 5, "value", 4, "condition"),
        ],
        vec![export(4, "result"), export(1, "result")],
        vec![f],
        vec![variable(10, "core/bool@1")],
    );
    let mut resolver = Counting::new(package);
    resolver.reject = Some(id(104).parse().unwrap());
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    assert_eq!(plan.resources().entropy_nodes, 2);
    let mut frame =
        DataActivation::start(plan.clone(), &doc, &snapshot, 42, 7, DataOptions::default())
            .unwrap();
    let mut invocation = 0;
    for (epoch, flag) in [false, false, true, false, true].into_iter().enumerate() {
        frame.begin_epoch().unwrap();
        supplied(&mut frame, &plan, 5, FoldedValue::Value(Value::Bool(flag)));
        let actual = value(&mut frame, 0, 1);
        if flag {
            assert_eq!(actual, expected(&[2], 103, invocation));
            invocation += 1;
        } else {
            assert_eq!(actual, Value::U64(7));
        }
        assert_eq!(value(&mut frame, 1, 1), expected(&[1], 103, epoch as u64));
        assert_eq!(
            resolver.draws.load(Ordering::Relaxed),
            invocation as usize + epoch + 1
        );
    }
}

#[test]
fn argument_fault_skips_entropy_and_preserves_the_first_live_invocation() {
    let (package, snapshot) = catalog();
    let mut f = entropy_function(100, None);
    f["inputs"] = json!([port("unused", "input", "core/u64@1")]);
    let doc = document(
        &package,
        vec![volatile_call(1, 100), get(2, 10)],
        vec![edge(1, 2, "value", 1, "unused")],
        vec![export(1, "result")],
        vec![f],
        vec![variable(10, "core/u64@1")],
    );
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let mut frame =
        DataActivation::start(plan.clone(), &doc, &snapshot, 42, 7, DataOptions::default())
            .unwrap();
    frame.begin_epoch().unwrap();
    let fault = ConstantFault {
        source: id(2).parse().unwrap(),
        fault: KernelFault {
            code: "core/TestFault",
            message_key: "core/test-fault",
            message: "test",
        },
    };
    supplied(&mut frame, &plan, 2, FoldedValue::Fault(fault.clone()));
    assert_eq!(
        frame.export(0).unwrap().unwrap().outcome.unwrap_err(),
        &fault
    );
    assert_eq!(resolver.draws.load(Ordering::Relaxed), 0);
    for invocation in 0..2 {
        frame.begin_epoch().unwrap();
        supplied(&mut frame, &plan, 2, FoldedValue::Value(Value::U64(4)));
        assert_eq!(value(&mut frame, 0, 1), expected(&[1], 103, invocation));
    }
}

#[test]
fn pure_profile_and_state_read_bodies_fail_before_executable_binding() {
    let (package, snapshot) = catalog();
    for state in [false, true] {
        let mut f = entropy_function(100, None);
        if state {
            f["nodes"][1] = get(103, 10);
        }
        let doc = document(
            &package,
            vec![volatile_call(1, 100)],
            vec![],
            vec![export(1, "result")],
            vec![f],
            if state {
                vec![variable(10, "core/u64@1")]
            } else {
                vec![]
            },
        );
        let analysis = analyze(&doc, &snapshot);
        assert!(
            analysis.is_ready_for_lowering(&doc, &snapshot),
            "{:?}",
            analysis.report().diagnostics
        );
        let resolver = Counting::new(package.clone());
        assert!(matches!(
            orange_compiler::compile_pure(
                &doc,
                snapshot.clone(),
                &analysis,
                &resolver,
                CompileOptions::default()
            ),
            Err(CompileError::UnsupportedNode { .. })
        ));
        if state {
            assert!(
                compile_data(
                    &doc,
                    snapshot.clone(),
                    &analysis,
                    &resolver,
                    CompileOptions::default()
                )
                .is_err()
            );
        }
        assert_eq!(resolver.pure.load(Ordering::Relaxed), 0);
        assert_eq!(resolver.entropy.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn entropy_faults_in_a_callee_keep_producer_identity_and_cancellation_stays_infrastructure() {
    let (package, snapshot) = catalog();
    let mut attempt = core(4, "try");
    attempt["type_pins"] = json!({"T":"core/u64@1"});
    let doc = document(
        &package,
        vec![volatile_call(1, 100), attempt],
        vec![edge(1, 1, "result", 4, "value")],
        vec![export(4, "result")],
        vec![entropy_function(100, None)],
        vec![],
    );
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let mut frame = DataActivation::start(
        plan.clone(),
        &doc,
        &snapshot,
        42,
        7,
        DataOptions {
            limits: orange_runtime::DataLimits {
                max_entropy_draws: 0,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    frame.begin_epoch().unwrap();
    let Value::Err(error) = value(&mut frame, 0, 1) else {
        panic!("draw limit must be captured")
    };
    assert_eq!(error.node, Some(id(103).parse().unwrap()));
    assert_eq!(error.code, "core/LimitExceeded");
    let token = CancelToken::new();
    let mut frame = DataActivation::start(
        plan,
        &doc,
        &snapshot,
        42,
        7,
        DataOptions {
            cancel: Some(token.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    frame.begin_epoch().unwrap();
    assert!(matches!(
        frame.poll_export(0, &mut 1).unwrap(),
        DataPoll::Pending
    ));
    token.cancel();
    assert!(matches!(
        frame.poll_export(0, &mut 1),
        Err(DataError::Cancelled)
    ));
}

#[test]
fn volatile_body_can_call_a_specialized_pure_function_without_folding_entropy() {
    let (package, snapshot) = catalog();
    let mut f = entropy_function(100, None);
    let mut c = call(104, 200, "core/u64@1");
    c["inputs"] = json!({"b":literal(1)});
    f["nodes"].as_array_mut().unwrap().push(c);
    f["edges"] = json!([
        edge(101, 103, "value", 104, "a"),
        edge(102, 104, "result", 102, "result")
    ]);
    let doc = document(
        &package,
        vec![volatile_call(1, 100)],
        vec![],
        vec![export(1, "result")],
        vec![f, arithmetic(200, "add")],
        vec![],
    );
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    assert_eq!(resolver.draws.load(Ordering::Relaxed), 0);
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 0);
    let mut frame =
        DataActivation::start(plan, &doc, &snapshot, 42, 7, DataOptions::default()).unwrap();
    for invocation in 0..3 {
        frame.begin_epoch().unwrap();
        let Value::U64(sample) = expected(&[1], 103, invocation) else {
            unreachable!()
        };
        assert_eq!(value(&mut frame, 0, 1), Value::U64(sample.wrapping_add(1)));
    }
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 3);
}
#[test]
fn a_statically_skipped_volatile_call_does_not_bind_its_provider() {
    let (package, snapshot) = catalog();
    let mut select = core(4, "select");
    select["type_pins"] = json!({"T":"core/u64@1"});
    select["inputs"] =
        json!({"condition":{"type":"core/bool@1","value":false},"if_false":literal(7)});
    let doc = document(
        &package,
        vec![volatile_call(1, 100), select],
        vec![edge(1, 1, "result", 4, "if_true")],
        vec![export(4, "result")],
        vec![entropy_function(100, None)],
        vec![],
    );
    let mut resolver = Counting::new(package);
    resolver.reject = Some(id(103).parse().unwrap());
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    assert_eq!(resolver.entropy.load(Ordering::Relaxed), 0);
    assert_eq!(plan.resources().entropy_nodes, 0);
    let mut frame =
        DataActivation::start(plan, &doc, &snapshot, 42, 7, DataOptions::default()).unwrap();
    frame.begin_epoch().unwrap();
    assert_eq!(value(&mut frame, 0, 1), Value::U64(7));
}

#[test]
fn control_set_consumes_a_scoped_entropy_call_across_one_unit_pumps() {
    let (package, snapshot) = catalog();
    let mut start = core(1, "on-start");
    start["activation"] = json!({"mode":"queue","capacity":4,"overflow":"drop-newest","on_fault":"continue","order":0});
    let mut set = core(3, "set-variable");
    set["config"] = json!({"variable":id(10)});
    let doc = document(
        &package,
        vec![start, volatile_call(2, 100), set],
        vec![
            edge(1, 1, "then", 3, "in"),
            edge(2, 2, "result", 3, "value"),
        ],
        vec![],
        vec![entropy_function(100, None)],
        vec![variable(10, "core/u64@1")],
    );
    let analysis = analyze(&doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(&doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
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
    for _ in 0..32 {
        run.pump(1).unwrap();
    }
    let expected = EntropyStream::new(EntropyKey {
        seed: 42,
        frame: FrameId::ROOT.call(id(2).parse().unwrap()),
        node: id(103).parse().unwrap(),
        activation_sequence: 0,
        invocation: 0,
    })
    .next_u64()
    .unwrap();
    assert_eq!(
        run.variable(id(10).parse().unwrap()),
        Some(&Value::U64(expected))
    );
    assert_eq!(resolver.draws.load(Ordering::Relaxed), 1);
    run.stop().unwrap();
    run.dispose().unwrap();
}

#[test]
fn deep_volatile_paths_are_iterative_and_preflight_limits_precede_binding() {
    let (package, snapshot) = catalog();
    let functions = (0..128)
        .map(|i| {
            entropy_function(
                100 + i * 10,
                if i < 127 { Some(110 + i * 10) } else { None },
            )
        })
        .collect();
    let doc = document(
        &package,
        vec![volatile_call(1, 100)],
        vec![],
        vec![export(1, "result")],
        functions,
        vec![],
    );
    let analysis = analyze(&doc, &snapshot);
    assert!(analysis.is_ready_for_lowering(&doc, &snapshot));
    let resolver = Counting::new(package);
    for limits in [
        CompileLimits {
            max_call_depth: 127,
            ..Default::default()
        },
        CompileLimits {
            max_fold_operations: 16,
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
    assert_eq!(resolver.entropy.load(Ordering::Relaxed), 0);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    assert_eq!(plan.resources().entropy_nodes, 1);
    let mut frame =
        DataActivation::start(plan, &doc, &snapshot, 42, 7, DataOptions::default()).unwrap();
    frame.begin_epoch().unwrap();
    let mut path = vec![1];
    path.extend((0..127).map(|i| 103 + i * 10));
    assert_eq!(value(&mut frame, 0, 1), expected(&path, 1373, 0));
}
