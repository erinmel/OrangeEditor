#[path = "support/seeded_provider.rs"]
mod seeded_provider;
use seeded_provider::*;
#[path = "support/data_graph.rs"]
mod support;
use orange_compiler::{
    CompileOptions, DataOperation, DataPlan, DataStability, EntropyBinding, EntropyKernel,
    EntropyKernelResolver, EntropySource, KernelBinding, KernelFault, PureKernelResolver,
    ResolveError, compile_data,
};
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{NodeInstance, SignatureSnapshot};
use orange_model::value::JsonText;
use orange_model::{GraphDocument, Value, analyze, from_json, to_json};
use orange_runtime::{
    CancelToken, DataActivation, DataError, DataLimits, DataOptions, EntropyKey, EntropyStream,
};
use serde_json::json;
use std::sync::{Arc, OnceLock, Weak, atomic::Ordering};
use support::*;

#[test]
#[ignore = "manual release-only local timing baseline; no NFR threshold"]
fn root_data_epoch_timing_baseline() {
    use std::hint::black_box;
    use std::time::{Duration, Instant};
    assert!(
        !black_box(cfg!(debug_assertions)),
        "run this timing probe with --release"
    );
    fn samples(label: &str, count: usize, mut operation: impl FnMut()) {
        let mut nanos = Vec::with_capacity(30);
        for _ in 0..30 {
            let started = Instant::now();
            for _ in 0..count {
                operation();
            }
            nanos.push(started.elapsed().as_nanos() as f64 / count as f64);
        }
        nanos.sort_by(f64::total_cmp);
        println!(
            "{label}: samples=30 batch={count} ns/op median={:.1} p95={:.1} p99={:.1} max={:.1}",
            (nanos[14] + nanos[15]) / 2.0,
            nanos[28],
            nanos[29],
            nanos[29],
        );
        println!("{label}: sorted batch-average ns/op={nanos:?}");
    }
    for size in [1_u32, 64, 512] {
        let (package, snapshot) = catalog();
        let mut nodes = vec![random(1)];
        nodes.extend((2..=size).map(|n| math(n, "add", 0, 1)));
        let edges = (2..=size)
            .map(|n| edge(n, n - 1, if n == 2 { "value" } else { "sum" }, n, "a"))
            .collect();
        let doc = graph(
            &package,
            nodes,
            edges,
            vec![
                export(size, if size == 1 { "value" } else { "sum" }),
                // Distinct export identity for the shared leaf when size == 1.
                json!({"id":"leaf","node":id(1),"port":"value"}),
            ],
        );
        let analysis = analyze(&doc, &snapshot);
        let plan = Arc::new(
            compile_data(
                &doc,
                snapshot.clone(),
                &analysis,
                &package,
                CompileOptions::default(),
            )
            .unwrap(),
        );
        let mut frame = start(plan.clone(), &doc, &snapshot, 17, 9, DataOptions::default());
        // Ten seconds of actual graph execution per size, separate from samples.
        let warm = Instant::now();
        while warm.elapsed() < Duration::from_secs(10) {
            frame.begin_epoch().unwrap();
            black_box(frame.export(0).unwrap().unwrap().outcome.unwrap());
        }
        samples(&format!("nodes={size} cached-export"), 10_000, || {
            black_box(frame.export(0).unwrap().unwrap().outcome.unwrap());
        });
        samples(&format!("nodes={size} sparse-epoch-leaf"), 1_000, || {
            frame.begin_epoch().unwrap();
            black_box(frame.export(1).unwrap().unwrap().outcome.unwrap());
        });
        samples(&format!("nodes={size} full-epoch"), 100, || {
            frame.begin_epoch().unwrap();
            black_box(frame.export(0).unwrap().unwrap().outcome.unwrap());
        });
        samples(&format!("nodes={size} frame-entry-and-drop"), 100, || {
            black_box(start(
                plan.clone(),
                &doc,
                &snapshot,
                17,
                9,
                DataOptions::default(),
            ));
        });
    }
}

fn sample(node: u32, seed: u64, sequence: u64, invocation: u64) -> u64 {
    EntropyStream::new(EntropyKey {
        seed,
        frame: orange_runtime::FrameId::ROOT,
        node: id(node).parse().unwrap(),
        activation_sequence: sequence,
        invocation,
    })
    .next_u64()
    .unwrap()
}
fn start(
    plan: Arc<DataPlan>,
    doc: &GraphDocument,
    snapshot: &RegistrySnapshot,
    seed: u64,
    sequence: u64,
    options: DataOptions,
) -> DataActivation {
    DataActivation::start(plan, doc, snapshot, seed, sequence, options).unwrap()
}

#[test]
fn native_entropy_diamond_shares_per_epoch_advances_and_replays_with_constant_islands() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![random(1), math(2, "add", 0, 0), math(3, "add", 20, 22)],
        vec![edge(1, 1, "value", 2, "a"), edge(2, 1, "value", 2, "b")],
        vec![export(2, "sum"), export(1, "value"), export(3, "sum")],
    );
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 1);
    let mut frame = start(plan.clone(), &doc, &snapshot, 0, 0, DataOptions::default());
    assert_eq!(
        frame.entropy_metadata().algorithm,
        orange_runtime::ENTROPY_ALGORITHM
    );
    assert_eq!(frame.export(0), Err(DataError::NoEpoch));
    for epoch in 0..3 {
        frame.begin_epoch().unwrap();
        let word = sample(1, 0, 0, epoch);
        if epoch == 0 {
            assert_eq!(word, 0x217b7f7d1090c139);
        }
        assert_eq!(
            frame.export(0).unwrap().unwrap().outcome,
            Ok(&Value::U64(word.wrapping_add(word)))
        );
        assert_eq!(
            frame.export(1).unwrap().unwrap().outcome,
            Ok(&Value::U64(word))
        );
        assert_eq!(
            frame.export(0).unwrap().unwrap().outcome,
            Ok(&Value::U64(word.wrapping_add(word)))
        );
        assert_eq!(
            frame.export(2).unwrap().unwrap().outcome,
            Ok(&Value::U64(42))
        );
        assert_eq!(resolver.draws.load(Ordering::Relaxed), epoch as usize + 1);
        assert_eq!(resolver.folds.load(Ordering::Relaxed), epoch as usize + 2);
    }
    let mut replay = start(plan.clone(), &doc, &snapshot, 0, 0, DataOptions::default());
    replay.begin_epoch().unwrap();
    assert_eq!(
        replay.export(1).unwrap().unwrap().outcome,
        Ok(&Value::U64(sample(1, 0, 0, 0)))
    );
    let mut next = start(plan, &doc, &snapshot, 0, 1, DataOptions::default());
    next.begin_epoch().unwrap();
    assert_eq!(
        next.export(1).unwrap().unwrap().outcome,
        Ok(&Value::U64(sample(1, 0, 1, 0)))
    );
}

#[test]
fn first_fault_stops_later_entropy_and_nested_try_preserves_ok_err() {
    let (package, snapshot) = catalog();
    let attempt = |n, ty| json!({"id":id(n),"definition_id":"core/try","definition_version":1,"config_version":1,"config":{},"type_pins":{"T":ty}});
    let doc = graph(
        &package,
        vec![
            random(1),
            math(2, "divide", 0, 0),
            random(3),
            math(4, "add", 0, 0),
            attempt(5, "core/u64@1"),
            attempt(6, "core/result@1<core/u64@1>"),
        ],
        vec![
            edge(1, 1, "value", 2, "a"),
            edge(2, 2, "quotient", 4, "a"),
            edge(3, 3, "value", 4, "b"),
            edge(4, 4, "sum", 5, "value"),
            edge(5, 5, "result", 6, "value"),
        ],
        vec![export(6, "result"), export(4, "sum"), export(3, "value")],
    );
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    assert!(plan.diagnostics().is_empty());
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 0);
    let mut frame = start(plan, &doc, &snapshot, 7, 8, DataOptions::default());
    frame.begin_epoch().unwrap();
    let Value::Ok(inner) = frame.export(0).unwrap().unwrap().outcome.unwrap() else {
        panic!("outer success missing")
    };
    let Value::Err(error) = inner.as_ref() else {
        panic!("inner graph fault missing")
    };
    assert_eq!(error.code, "core/DivideByZero");
    assert_eq!(error.node, Some(id(2).parse().unwrap()));
    assert_eq!(resolver.draws.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 1);
    assert_eq!(
        frame
            .export(1)
            .unwrap()
            .unwrap()
            .outcome
            .unwrap_err()
            .source,
        id(2).parse().unwrap()
    );
    assert_eq!(resolver.draws.load(Ordering::Relaxed), 1);
    assert_eq!(
        frame.export(2).unwrap().unwrap().outcome,
        Ok(&Value::U64(sample(3, 7, 8, 0)))
    );
    assert_eq!(resolver.draws.load(Ordering::Relaxed), 2);
}

#[test]
fn retained_budget_includes_untouched_old_epoch_slots_and_infrastructure_failure_aborts_epoch() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![random(1), random(2)],
        vec![],
        vec![export(1, "value"), export(2, "value")],
    );
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let options = DataOptions {
        limits: DataLimits {
            max_retained_bytes: 16,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut frame = start(plan, &doc, &snapshot, 0, 0, options);
    frame.begin_epoch().unwrap();
    assert!(frame.export(0).unwrap().unwrap().outcome.is_ok());
    frame.begin_epoch().unwrap();
    assert_eq!(frame.export(1), Err(DataError::ResourceLimit));
    assert_eq!(frame.export(0), Err(DataError::ResourceLimit));
    assert_eq!(resolver.draws.load(Ordering::Relaxed), 2);
}

#[test]
fn draw_limit_is_a_graph_fault_but_operation_and_cancel_limits_are_not_catchable() {
    let (package, snapshot) = catalog();
    let attempt = json!({"id":id(2),"definition_id":"core/try","definition_version":1,"config_version":1,"config":{},"type_pins":{"T":"core/u64@1"}});
    let doc = graph(
        &package,
        vec![random(1), attempt],
        vec![edge(1, 1, "value", 2, "value")],
        vec![export(2, "result")],
    );
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let mut frame = start(
        plan.clone(),
        &doc,
        &snapshot,
        0,
        0,
        DataOptions {
            limits: DataLimits {
                max_entropy_draws: 0,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    frame.begin_epoch().unwrap();
    let Value::Err(error) = frame.export(0).unwrap().unwrap().outcome.unwrap() else {
        panic!("draw limit wasn't captured")
    };
    assert_eq!(error.code, "core/LimitExceeded");
    assert_eq!(error.node, Some(id(1).parse().unwrap()));
    let mut limited = start(
        plan.clone(),
        &doc,
        &snapshot,
        0,
        0,
        DataOptions {
            limits: DataLimits {
                max_operations: 0,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    limited.begin_epoch().unwrap();
    assert_eq!(limited.export(0), Err(DataError::ResourceLimit));
    let cancel = CancelToken::new();
    let mut cancelled = start(
        plan,
        &doc,
        &snapshot,
        0,
        0,
        DataOptions {
            cancel: Some(cancel.clone()),
            ..Default::default()
        },
    );
    cancelled.begin_epoch().unwrap();
    cancel.cancel();
    assert_eq!(cancelled.export(0), Err(DataError::Cancelled));
    cancel.clear();
    assert_eq!(cancelled.export(0), Err(DataError::Cancelled));
}

#[test]
fn stale_entry_is_rejected_while_active_plan_keeps_its_old_snapshot() {
    let (package, snapshot) = catalog();
    let mut doc = graph(&package, vec![random(1)], vec![], vec![export(1, "value")]);
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let mut frame = start(plan.clone(), &doc, &snapshot, 0, 0, DataOptions::default());
    let reload = from_json(&to_json(&doc).unwrap()).unwrap();
    assert_eq!(
        DataActivation::start(
            plan.clone(),
            &reload,
            &snapshot,
            0,
            0,
            DataOptions::default()
        )
        .unwrap_err(),
        DataError::StaleDocument
    );
    let (_, replacement) = catalog();
    assert_eq!(
        DataActivation::start(
            plan.clone(),
            &doc,
            &replacement,
            0,
            0,
            DataOptions::default()
        )
        .unwrap_err(),
        DataError::StaleRegistry
    );
    let change = doc
        .apply([orange_model::DocCommand::SetConfig {
            node: id(1).parse().unwrap(),
            config_version: 1,
            config: [("invalid".into(), json!(true))].into_iter().collect(),
        }])
        .unwrap();
    assert_eq!(
        DataActivation::start(plan.clone(), &doc, &snapshot, 0, 0, DataOptions::default())
            .unwrap_err(),
        DataError::StaleDocument
    );
    frame.begin_epoch().unwrap();
    assert_eq!(
        frame.export(0).unwrap().unwrap().outcome,
        Ok(&Value::U64(sample(1, 0, 0, 0)))
    );
    assert!(Arc::ptr_eq(frame.plan().snapshot(), &snapshot));
    let _ = doc.revert(change.inverse).unwrap();
    let restored = start(plan, &doc, &snapshot, 0, 0, DataOptions::default());
    assert!(restored.document_revision() > restored.source_revision());
}

#[derive(Debug, Clone)]
enum Mutation {
    WrongType,
    PartialFault(Arc<OnceLock<Weak<JsonText>>>),
    Cancel(CancelToken, bool),
    IgnoreDrawFault { draws: usize, return_fault: bool },
}
#[derive(Debug)]
struct MutatedKernel(Mutation);
impl EntropyKernel for MutatedKernel {
    fn evaluate(
        &self,
        _: &[Value],
        outputs: &mut [Value],
        source: &mut dyn EntropySource,
    ) -> Result<(), KernelFault> {
        match &self.0 {
            Mutation::IgnoreDrawFault {
                draws,
                return_fault,
            } => {
                for _ in 0..*draws {
                    let _ = source.next_u64();
                }
                outputs[0] = Value::U64(42);
                if *return_fault {
                    Err(KernelFault {
                        code: "core/Domain",
                        message_key: "core/test-domain",
                        message: "The operation failed.",
                    })
                } else {
                    Ok(())
                }
            }
            Mutation::WrongType => {
                outputs[0] = Value::Bool(true);
                Ok(())
            }
            Mutation::PartialFault(observer) => {
                let payload = Arc::new(JsonText::new("\"unpublished-payload\"".into()).unwrap());
                observer.set(Arc::downgrade(&payload)).unwrap();
                outputs[0] = Value::Json(payload);
                Err(KernelFault {
                    code: "core/Domain",
                    message_key: "core/test-domain",
                    message: "The operation failed.",
                })
            }
            Mutation::Cancel(flag, ignore_fault) => {
                flag.cancel();
                let result = source.next_u64();
                flag.clear();
                if *ignore_fault {
                    outputs[0] = Value::U64(42);
                    Ok(())
                } else {
                    result.map(|value| outputs[0] = Value::U64(value))
                }
            }
        }
    }
}

#[test]
fn rejected_epoch_start_aborts_old_reads_and_only_a_successful_start_recovers() {
    let (package, snapshot) = catalog();
    let doc = graph(&package, vec![random(1)], vec![], vec![export(1, "value")]);
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let cancel = CancelToken::new();
    let mut frame = start(
        plan,
        &doc,
        &snapshot,
        0,
        0,
        DataOptions {
            cancel: Some(cancel.clone()),
            ..Default::default()
        },
    );
    // Rejected initial starts neither invent nor consume an epoch.
    cancel.cancel();
    assert_eq!(frame.begin_epoch(), Err(DataError::Cancelled));
    cancel.clear();
    assert_eq!(frame.export(0), Err(DataError::NoEpoch));
    assert_eq!(frame.begin_epoch(), Ok(0));
    assert_eq!(
        frame.export(0).unwrap().unwrap().outcome,
        Ok(&Value::U64(sample(1, 0, 0, 0)))
    );
    cancel.cancel();
    assert_eq!(frame.begin_epoch(), Err(DataError::Cancelled));
    cancel.clear();
    assert_eq!(frame.export(0), Err(DataError::Cancelled));
    assert_eq!(frame.export(usize::MAX), Err(DataError::Cancelled));
    assert_eq!(resolver.draws.load(Ordering::Relaxed), 1);
    assert_eq!(frame.begin_epoch(), Ok(1));
    assert_eq!(
        frame.export(0).unwrap().unwrap().outcome,
        Ok(&Value::U64(sample(1, 0, 0, 1)))
    );
    assert_eq!(resolver.draws.load(Ordering::Relaxed), 2);
}

#[test]
fn draw_budget_fault_cannot_be_hidden_by_a_downstream_callback() {
    for (budget, draws, exceeded) in [(0, 0, false), (0, 1, true), (1, 1, false), (1, 3, true)] {
        for return_fault in [false, true] {
            let (package, snapshot) = catalog();
            let attempt = json!({"id":id(2),"definition_id":"core/try","definition_version":1,"config_version":1,"config":{},"type_pins":{"T":"core/u64@1"}});
            let doc = graph(
                &package,
                vec![random(1), attempt],
                vec![edge(1, 1, "value", 2, "value")],
                vec![export(2, "result"), export(1, "value")],
            );
            let resolver = Mutated {
                package,
                mode: Mutation::IgnoreDrawFault {
                    draws,
                    return_fault,
                },
            };
            let analysis = analyze(&doc, &snapshot);
            let plan = Arc::new(
                compile_data(
                    &doc,
                    snapshot.clone(),
                    &analysis,
                    &resolver,
                    CompileOptions::default(),
                )
                .unwrap(),
            );
            let mut frame = start(
                plan,
                &doc,
                &snapshot,
                0,
                0,
                DataOptions {
                    limits: DataLimits {
                        max_entropy_draws: budget,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            );
            for _ in 0..2 {
                frame.begin_epoch().unwrap();
                let value = frame.export(0).unwrap().unwrap().outcome.unwrap().clone();
                if exceeded || return_fault {
                    let Value::Err(error) = value else {
                        panic!("expected captured fault")
                    };
                    assert_eq!(
                        error.code,
                        if exceeded {
                            "core/LimitExceeded"
                        } else {
                            "core/Domain"
                        }
                    );
                    assert_eq!(error.node, Some(id(1).parse().unwrap()));
                    assert_eq!(
                        frame
                            .export(1)
                            .unwrap()
                            .unwrap()
                            .outcome
                            .unwrap_err()
                            .fault
                            .code,
                        error.code
                    );
                } else {
                    assert_eq!(value, Value::U64(42));
                    assert_eq!(
                        frame.export(1).unwrap().unwrap().outcome,
                        Ok(&Value::U64(42))
                    );
                }
            }
        }
    }
}
struct Mutated {
    package: orange_bundled::MathPackage,
    mode: Mutation,
}
impl PureKernelResolver for Mutated {
    fn resolve(
        &self,
        doc: &GraphDocument,
        node: &NodeInstance,
        sig: &SignatureSnapshot,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Result<KernelBinding, ResolveError> {
        PureKernelResolver::resolve(&self.package, doc, node, sig, snapshot)
    }
}
impl EntropyKernelResolver for Mutated {
    fn resolve_entropy(
        &self,
        doc: &GraphDocument,
        node: &NodeInstance,
        sig: &SignatureSnapshot,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Result<EntropyBinding, ResolveError> {
        let mut binding = self.package.resolve_entropy(doc, node, sig, snapshot)?;
        binding.kernel = Arc::new(MutatedKernel(self.mode.clone()));
        Ok(binding)
    }
}
#[test]
fn wrong_output_is_infrastructure_error_and_faulting_callbacks_release_unpublished_payloads() {
    for wrong in [true, false] {
        let (package, snapshot) = catalog();
        let doc = graph(&package, vec![random(1)], vec![], vec![export(1, "value")]);
        let observer = Arc::new(OnceLock::new());
        let resolver = Mutated {
            package,
            mode: if wrong {
                Mutation::WrongType
            } else {
                Mutation::PartialFault(observer.clone())
            },
        };
        let analysis = analyze(&doc, &snapshot);
        let plan = Arc::new(
            compile_data(
                &doc,
                snapshot.clone(),
                &analysis,
                &resolver,
                CompileOptions::default(),
            )
            .unwrap(),
        );
        let mut frame = start(plan, &doc, &snapshot, 0, 0, DataOptions::default());
        frame.begin_epoch().unwrap();
        if wrong {
            assert!(matches!(
                frame.export(0),
                Err(DataError::KernelContract { .. })
            ));
        } else {
            assert_eq!(
                frame
                    .export(0)
                    .unwrap()
                    .unwrap()
                    .outcome
                    .unwrap_err()
                    .fault
                    .code,
                "core/Domain"
            );
            assert!(
                observer.get().unwrap().upgrade().is_none(),
                "unpublished callback payload was retained"
            );
        }
    }
}

#[test]
fn cancellation_observed_inside_stream_cannot_be_turned_into_try_data_even_if_flag_is_cleared() {
    for ignore_fault in [false, true] {
        let (package, snapshot) = catalog();
        let attempt = json!({"id":id(2),"definition_id":"core/try","definition_version":1,"config_version":1,"config":{},"type_pins":{"T":"core/u64@1"}});
        let doc = graph(
            &package,
            vec![random(1), attempt],
            vec![edge(1, 1, "value", 2, "value")],
            vec![export(2, "result")],
        );
        let cancel = CancelToken::new();
        let resolver = Mutated {
            package,
            mode: Mutation::Cancel(cancel.clone(), ignore_fault),
        };
        let analysis = analyze(&doc, &snapshot);
        let plan = Arc::new(
            compile_data(
                &doc,
                snapshot.clone(),
                &analysis,
                &resolver,
                CompileOptions::default(),
            )
            .unwrap(),
        );
        let mut frame = start(
            plan,
            &doc,
            &snapshot,
            0,
            0,
            DataOptions {
                cancel: Some(cancel.clone()),
                ..Default::default()
            },
        );
        frame.begin_epoch().unwrap();
        assert_eq!(frame.export(0), Err(DataError::Cancelled));
        assert!(!cancel.is_cancelled());
        assert_eq!(frame.export(0), Err(DataError::Cancelled));
    }
}

#[test]
fn epoch_select_pulls_only_selected_native_entropy_arm() {
    let selection = json!({"id":id(4),"definition_id":"core/select","definition_version":1,"config_version":1,"config":{},"type_pins":{"T":"core/u64@1"}});
    let (doc, snapshot, resolver) = fixture(
        vec![
            fixture_node(10, BOOL, false),
            random(1),
            random(2),
            selection,
        ],
        vec![
            edge(1, 10, "value", 4, "condition"),
            edge(2, 1, "value", 4, "if_true"),
            edge(3, 2, "value", 4, "if_false"),
        ],
        vec![export(4, "result")],
    );
    let plan = fixture_plan(&doc, &snapshot, &resolver);
    let mut seen = [false; 2];
    for seed in 0..16 {
        let flag = sample(10, seed, 0, 0) & 1 != 0;
        seen[usize::from(flag)] = true;
        let before = resolver.base.draws.load(Ordering::Relaxed);
        let mut frame = start(
            plan.clone(),
            &doc,
            &snapshot,
            seed,
            0,
            DataOptions::default(),
        );
        frame.begin_epoch().unwrap();
        assert_eq!(
            frame.export(0).unwrap().unwrap().outcome,
            Ok(&Value::U64(sample(if flag { 1 } else { 2 }, seed, 0, 0)))
        );
        assert_eq!(resolver.base.draws.load(Ordering::Relaxed), before + 1);
    }
    assert_eq!(seen, [true, true]);
}

#[test]
fn epoch_and_or_short_circuit_faulting_right_inputs_and_stop_on_selector_fault() {
    for operation in ["and", "or"] {
        let bool_op = json!({"id":id(2),"definition_id":format!("core/{operation}"),"definition_version":1,"config_version":1,"config":{}});
        let (doc, snapshot, resolver) = fixture(
            vec![
                fixture_node(10, BOOL, false),
                fixture_node(11, BOOL, true),
                bool_op.clone(),
            ],
            vec![edge(1, 10, "value", 2, "a"), edge(2, 11, "value", 2, "b")],
            vec![export(2, "result")],
        );
        let plan = fixture_plan(&doc, &snapshot, &resolver);
        for seed in 0..16 {
            let flag = sample(10, seed, 0, 0) & 1 != 0;
            let needed = if operation == "and" { flag } else { !flag };
            let before = resolver.calls.load(Ordering::Relaxed);
            let mut frame = start(
                plan.clone(),
                &doc,
                &snapshot,
                seed,
                0,
                DataOptions::default(),
            );
            frame.begin_epoch().unwrap();
            let result = frame.export(0).unwrap().unwrap();
            if needed {
                assert_eq!(result.outcome.unwrap_err().source, id(11).parse().unwrap());
            } else {
                assert_eq!(result.outcome, Ok(&Value::Bool(flag)));
            }
            assert_eq!(
                resolver.calls.load(Ordering::Relaxed),
                before + 1 + usize::from(needed)
            );
        }
        let (stopped, snapshot, resolver) = fixture(
            vec![
                fixture_node(10, BOOL, true),
                fixture_node(11, BOOL, false),
                bool_op,
            ],
            vec![edge(1, 10, "value", 2, "a"), edge(2, 11, "value", 2, "b")],
            vec![export(2, "result")],
        );
        let plan = fixture_plan(&stopped, &snapshot, &resolver);
        let mut frame = start(plan, &stopped, &snapshot, 0, 0, DataOptions::default());
        frame.begin_epoch().unwrap();
        assert_eq!(
            frame
                .export(0)
                .unwrap()
                .unwrap()
                .outcome
                .unwrap_err()
                .source,
            id(10).parse().unwrap()
        );
        assert_eq!(resolver.calls.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn deep_epoch_pull_is_iterative_and_live_debug_omits_value_payloads() {
    let (package, snapshot) = catalog();
    let mut nodes = vec![random(1)];
    for n in 2..=512 {
        nodes.push(math(n, "add", 0, 1));
    }
    let edges = (2..=512)
        .map(|n| edge(n, n - 1, if n == 2 { "value" } else { "sum" }, n, "a"))
        .collect();
    let doc = graph(
        &package,
        nodes,
        edges,
        vec![export(512, "sum"), export(256, "sum")],
    );
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let mut frame = start(plan, &doc, &snapshot, 0, 0, DataOptions::default());
    frame.begin_epoch().unwrap();
    assert_eq!(
        frame.export(0).unwrap().unwrap().outcome,
        Ok(&Value::U64(sample(1, 0, 0, 0).wrapping_add(511)))
    );
    assert_eq!(
        frame.export(1).unwrap().unwrap().outcome,
        Ok(&Value::U64(sample(1, 0, 0, 0).wrapping_add(255)))
    );
    assert_eq!(resolver.draws.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 511);
    let (package, snapshot) = catalog();
    let selected = json!({"id":id(1),"definition_id":"core/select","definition_version":1,"config_version":1,"config":{},
        "type_pins":{"T":"core/json@1"},"inputs":{"condition":{"type":"core/bool@1","value":true},
            "if_true":{"type":"core/json@1","value":"\"SENSITIVE_MARKER\""},
            "if_false":{"type":"core/json@1","value":"null"}}});
    let doc = graph(&package, vec![selected], vec![], vec![export(1, "result")]);
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let mut frame = start(plan, &doc, &snapshot, 0, 0, DataOptions::default());
    frame.begin_epoch().unwrap();
    assert!(frame.export(0).unwrap().unwrap().outcome.is_ok());
    assert!(!format!("{frame:?}").contains("SENSITIVE_MARKER"));
}

#[test]
fn entry_resource_limits_and_empty_or_missing_exports_have_explicit_results() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![random(1), math(2, "add", 0, 1)],
        vec![edge(1, 1, "value", 2, "a")],
        vec![export(2, "sum")],
    );
    let resolver = Counting::new(package.clone());
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    for kind in 0..5 {
        let mut limits = DataLimits::default();
        match kind {
            0 => limits.max_slots = 0,
            1 => limits.max_nodes = 0,
            2 => limits.max_exports = 0,
            3 => limits.max_work_items = 0,
            _ => limits.max_retained_bytes = 0,
        }
        assert_eq!(
            DataActivation::start(
                plan.clone(),
                &doc,
                &snapshot,
                0,
                0,
                DataOptions {
                    limits,
                    ..Default::default()
                }
            )
            .unwrap_err(),
            DataError::ResourceLimit
        );
    }
    let cancel = CancelToken::new();
    cancel.cancel();
    assert_eq!(
        DataActivation::start(
            plan,
            &doc,
            &snapshot,
            0,
            0,
            DataOptions {
                cancel: Some(cancel.clone()),
                ..Default::default()
            }
        )
        .unwrap_err(),
        DataError::Cancelled
    );
    assert_eq!(resolver.draws.load(Ordering::Relaxed), 0);
    let empty = graph(&package, vec![], vec![], vec![]);
    let plan = Arc::new(compile(&empty, snapshot.clone(), &resolver));
    let mut frame = start(
        plan,
        &empty,
        &snapshot,
        0,
        0,
        DataOptions {
            limits: DataLimits {
                max_operations: 0,
                max_exports: 0,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    assert_eq!(frame.export(0), Err(DataError::NoEpoch));
    frame.begin_epoch().unwrap();
    assert_eq!(frame.export(0).unwrap(), None);
}

#[test]
fn dynamic_converter_publishes_result_and_loss_together_or_preserves_one_fault_on_both() {
    let converter = json!({"id":id(2),"definition_id":"core/convert","definition_version":1,"config_version":1,
        "config":{"converter":"core/f64-to-f32","contract_version":1,"options":{"rounding":"nearest-even","overflow":"reject"}},"signature":"convert"});
    let (doc, snapshot, resolver) = fixture(
        vec![fixture_node(40, FLOAT, false), converter],
        vec![edge(1, 40, "value", 2, "value")],
        vec![
            export(2, "result"),
            json!({"id":"loss","node":id(2),"port":"loss"}),
        ],
    );
    let mut raw: serde_json::Value = serde_json::from_str(&to_json(&doc).unwrap()).unwrap();
    raw["signatures"]["convert"] = json!({"kind":"pure","ports":[
        {"port_id":"value","direction":"input","kind":"data","type":"core/f64@1"},
        {"port_id":"result","direction":"output","kind":"data","type":"core/f32@1","source":"computed"},
        {"port_id":"loss","direction":"output","kind":"data","type":"core/json@1","source":"computed"}]});
    let doc = from_json(&raw.to_string()).unwrap();
    let plan = fixture_plan(&doc, &snapshot, &resolver);
    let mut seen = [false; 2];
    for seed in 0..32 {
        let before = resolver.calls.load(Ordering::Relaxed);
        let mut frame = start(
            plan.clone(),
            &doc,
            &snapshot,
            seed,
            0,
            DataOptions::default(),
        );
        frame.begin_epoch().unwrap();
        let result = frame
            .export(0)
            .unwrap()
            .unwrap()
            .outcome
            .cloned()
            .map_err(Clone::clone);
        let loss = frame.export(1).unwrap().unwrap().outcome;
        match result {
            Ok(Value::F32(_)) => {
                assert!(matches!(loss, Ok(Value::Json(_))));
                seen[0] = true;
            }
            Err(error) => {
                assert_eq!(loss.unwrap_err(), &error);
                assert_eq!(error.source, id(2).parse().unwrap());
                seen[1] = true;
            }
            _ => panic!("wrong converter output"),
        }
        assert_eq!(resolver.calls.load(Ordering::Relaxed), before + 1);
    }
    assert_eq!(seen, [true, true]);
}

#[test]
fn epoch_coalesce_distinguishes_absent_present_and_nested_some_none() {
    for nested in [false, true] {
        let ty = if nested { MAYBE } else { "core/u64@1" };
        let fallback = if nested {
            json!({"type":MAYBE,"value":{"some":"99"}})
        } else {
            literal(99)
        };
        let coalesce = json!({"id":id(2),"definition_id":"core/coalesce","definition_version":1,"config_version":1,
            "config":{},"type_pins":{"T":ty},"inputs":{"fallback":fallback}});
        let (doc, snapshot, resolver) = fixture(
            vec![
                fixture_node(20, if nested { NESTED } else { MAYBE }, false),
                coalesce,
            ],
            vec![edge(1, 20, "value", 2, "value")],
            vec![export(2, "result")],
        );
        let plan = fixture_plan(&doc, &snapshot, &resolver);
        let mut saw_inner_none = false;
        for seed in 0..32 {
            let word = sample(20, seed, 0, 0);
            let expected = if nested {
                if word & 1 == 0 {
                    Value::U64(99)
                } else if word & 2 == 0 {
                    saw_inner_none = true;
                    Value::None
                } else {
                    Value::U64(word)
                }
            } else if word & 1 != 0 {
                Value::U64(99)
            } else {
                Value::U64(word)
            };
            let mut frame = start(
                plan.clone(),
                &doc,
                &snapshot,
                seed,
                0,
                DataOptions::default(),
            );
            frame.begin_epoch().unwrap();
            assert_eq!(frame.export(0).unwrap().unwrap().outcome, Ok(&expected));
        }
        if nested {
            assert!(saw_inner_none);
        }
    }
}

#[test]
fn activation_fuel_spans_epochs_and_exhaustion_is_sticky() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![random(1), math(2, "add", 0, 1)],
        vec![edge(1, 1, "value", 2, "a")],
        vec![export(2, "sum"), export(1, "value")],
    );
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let options = |fuel| DataOptions {
        limits: DataLimits {
            max_activation_operations: fuel,
            ..Default::default()
        },
        ..Default::default()
    };
    // One unit per epoch start and per evaluated node; memoized pulls are free.
    let mut frame = start(plan.clone(), &doc, &snapshot, 0, 0, options(7));
    for remaining in [4, 1] {
        frame.begin_epoch().unwrap();
        assert!(frame.export(0).unwrap().unwrap().outcome.is_ok());
        assert!(frame.export(1).unwrap().unwrap().outcome.is_ok());
        assert_eq!(frame.fuel_remaining(), remaining);
    }
    frame.begin_epoch().unwrap();
    assert_eq!(frame.export(0), Err(DataError::FuelExhausted));
    assert_eq!(frame.export(1), Err(DataError::FuelExhausted));
    assert_eq!(frame.begin_epoch(), Err(DataError::FuelExhausted));
    assert_eq!(frame.export(1), Err(DataError::FuelExhausted));
    assert_eq!(resolver.draws.load(Ordering::Relaxed), 2);
    // Empty epochs still burn fuel, so a loop of bare epoch starts ends too.
    let mut idle = start(plan, &doc, &snapshot, 0, 0, options(2));
    assert_eq!(idle.begin_epoch(), Ok(0));
    assert_eq!(idle.begin_epoch(), Ok(1));
    assert_eq!(idle.begin_epoch(), Err(DataError::FuelExhausted));
    assert_eq!(idle.export(1), Err(DataError::FuelExhausted));
    assert_eq!(
        DataError::FuelExhausted.message_key(),
        "core/data-fuel-exhausted"
    );
}

#[test]
fn state_reads_are_host_inputs_supplied_once_per_epoch_and_only_when_demanded() {
    let (package, snapshot) = catalog();
    let get = |n, variable| json!({"id":id(n),"definition_id":"core/get-variable","definition_version":1,"config_version":1,"config":{"variable":id(variable)}});
    let select = json!({"id":id(3),"definition_id":"core/select","definition_version":1,"config_version":1,"config":{},
        "type_pins":{"T":"core/u64@1"},"inputs":{"if_false":literal(7)}});
    let base = graph(
        &package,
        vec![get(1, 20), get(2, 21), select, math(4, "add", 0, 1)],
        vec![
            edge(1, 1, "value", 3, "condition"),
            edge(2, 2, "value", 3, "if_true"),
            edge(3, 3, "result", 4, "a"),
        ],
        vec![export(4, "sum"), export(2, "value")],
    );
    let mut raw: serde_json::Value = serde_json::from_str(&to_json(&base).unwrap()).unwrap();
    raw["variables"] = json!([
        {"id":id(20),"name":"flag","type":"core/bool@1","scope":"run"},
        {"id":id(21),"name":"state","type":"core/u64@1","scope":"run"}
    ]);
    let doc = from_json(&raw.to_string()).unwrap();
    let resolver = Counting::new(package);
    let plan = Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let input = |n: u32| {
        let node = id(n).parse().unwrap();
        plan.host_inputs()
            .iter()
            .position(|ix| plan.nodes()[ix.index()].node() == node)
            .unwrap()
    };
    let (flag, state) = (input(1), input(2));
    assert_eq!(plan.host_inputs().len(), 2);
    for ix in plan.host_inputs() {
        let node = &plan.nodes()[ix.index()];
        assert!(matches!(node.operation(), DataOperation::Input));
        assert_eq!(node.stability(), DataStability::Epoch);
    }
    let missing = |n: u32| DataError::MissingInput {
        node: id(n).parse().unwrap(),
    };
    let mut frame = start(plan.clone(), &doc, &snapshot, 0, 0, DataOptions::default());
    assert_eq!(
        frame.set_input(flag, Value::Bool(true)),
        Err(DataError::NoEpoch)
    );
    // A demanded input that was never supplied aborts the epoch.
    frame.begin_epoch().unwrap();
    assert_eq!(frame.export(0), Err(missing(1)));
    assert_eq!(frame.export(0), Err(missing(1)));
    assert_eq!(frame.set_input(flag, Value::Bool(true)), Err(missing(1)));
    // The inactive arm's input is not demanded, so it may stay unset.
    frame.begin_epoch().unwrap();
    frame.set_input(flag, Value::Bool(false)).unwrap();
    assert_eq!(
        frame.export(0).unwrap().unwrap().outcome,
        Ok(&Value::U64(8))
    );
    assert_eq!(frame.export(1), Err(missing(2)));
    // Rejected values change nothing and keep the epoch usable.
    frame.begin_epoch().unwrap();
    let invalid = |index| Err(DataError::InvalidInput { index });
    assert_eq!(frame.set_input(flag, Value::U64(1)), invalid(flag));
    assert_eq!(frame.set_input(99, Value::Bool(true)), invalid(99));
    frame.set_input(flag, Value::Bool(true)).unwrap();
    assert_eq!(frame.set_input(flag, Value::Bool(false)), invalid(flag));
    frame.set_input(state, Value::U64(41)).unwrap();
    assert_eq!(
        frame.export(0).unwrap().unwrap().outcome,
        Ok(&Value::U64(42))
    );
    assert_eq!(
        frame.export(1).unwrap().unwrap().outcome,
        Ok(&Value::U64(41))
    );
    // Supplied values never carry over into the next epoch.
    frame.begin_epoch().unwrap();
    frame.set_input(flag, Value::Bool(true)).unwrap();
    assert_eq!(frame.export(0), Err(missing(2)));
    assert_eq!(
        DataError::InvalidInput { index: 0 }.message_key(),
        "core/data-invalid-input"
    );
    assert_eq!(missing(1).message_key(), "core/data-missing-input");
}

#[test]
fn rejected_host_input_over_the_retained_budget_is_released() {
    let (package, snapshot) = catalog();
    let get = json!({"id":id(1),"definition_id":"core/get-variable","definition_version":1,"config_version":1,"config":{"variable":id(20)}});
    let base = graph(&package, vec![get], vec![], vec![export(1, "value")]);
    let mut raw: serde_json::Value = serde_json::from_str(&to_json(&base).unwrap()).unwrap();
    raw["variables"] = json!([{"id":id(20),"name":"blob","type":"core/json@1","scope":"run"}]);
    let doc = from_json(&raw.to_string()).unwrap();
    let plan = Arc::new(compile(&doc, snapshot.clone(), &Counting::new(package)));
    let options = DataOptions {
        limits: DataLimits {
            max_retained_bytes: 64,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut frame = start(plan, &doc, &snapshot, 0, 0, options);
    frame.begin_epoch().unwrap();
    let payload = Arc::new(JsonText::new(format!("\"{}\"", "x".repeat(100))).unwrap());
    let observer = Arc::downgrade(&payload);
    assert_eq!(
        frame.set_input(0, Value::Json(payload)),
        Err(DataError::ResourceLimit)
    );
    // No uncharged copy survives in activation scratch storage.
    assert!(observer.upgrade().is_none());
    let small = Value::Json(Arc::new(JsonText::new("1".into()).unwrap()));
    frame.set_input(0, small.clone()).unwrap();
    assert_eq!(frame.export(0).unwrap().unwrap().outcome, Ok(&small));
}
