//! Checked zone lowering and, as runtime support lands, source-level execution regressions.
#[path = "support/data_graph.rs"]
mod support;
use orange_compiler::{
    CompileError, CompileLimits, CompileOptions, DataOperation, compile_data, compile_pure,
};
use orange_model::{analyze, from_json, to_json};
use serde_json::{Value as Json, json};
use std::sync::atomic::Ordering;
use support::*;

fn zone(n: u32, family: &str, end: bool, label: &str, pins: Json, inputs: Json) -> Json {
    json!({"id": id(n), "definition_id":format!("core/{family}-{}", if end { "end" } else { "begin" }),
        "definition_version":1, "config_version":1, "config":{"zone":label}, "type_pins":pins, "inputs":inputs})
}
fn repeat(n: u32, label: &str, count: i64, initial: u64) -> Json {
    zone(
        n,
        "repeat",
        false,
        label,
        json!({"A":"core/u64@1"}),
        json!({
        "count":{"type":"core/i64@1","value":count.to_string()},
        "initial":{"type":"core/u64@1","value":initial.to_string()}}),
    )
}

#[test]
fn million_iteration_repeat_lowers_once_with_original_sources_and_no_body_folding() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![
            repeat(1, "z", 1_000_000, 7),
            math(2, "add", 0, 1),
            zone(3, "repeat", true, "z", json!({"A":"core/u64@1"}), json!({})),
        ],
        vec![edge(1, 1, "carried", 2, "a"), edge(2, 2, "sum", 3, "next")],
        vec![export(3, "result")],
    );
    let resolver = Counting::new(package);
    let plan = compile(&doc, snapshot, &resolver);
    assert_eq!(plan.zones().len(), 1);
    assert_eq!(plan.nodes().len(), 3);
    let parameters = plan.zones()[0].parameters().unwrap();
    assert!(matches!(
        plan.nodes()[parameters.index()].operation(),
        DataOperation::Iteration(_)
    ));
    assert_eq!(plan.nodes()[parameters.index()].inputs().len(), 0);
    let body = plan
        .nodes()
        .iter()
        .find(|n| n.node().to_string() == id(2))
        .unwrap();
    assert_eq!(body.zone(), plan.nodes()[parameters.index()].zone());
    let end = plan.nodes().last().unwrap();
    assert_eq!(end.zone(), None);
    assert!(matches!(
        end.operation(),
        DataOperation::Zone { prefix: 2, .. }
    ));
    assert_eq!(end.inputs().len(), 3);
    assert_eq!(resolver.pure.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 0);
}

#[test]
fn map_fold_and_filter_lower_their_declared_strict_ports_without_binding_intrinsics() {
    for family in ["map", "fold", "filter"] {
        let (package, snapshot) = catalog();
        let pins = if family == "map" {
            json!({"T":"core/u64@1"})
        } else if family == "fold" {
            json!({"T":"core/u64@1","A":"core/u64@1"})
        } else {
            json!({"T":"core/u64@1"})
        };
        let mut inputs = json!({"items":{"type":"core/list@1<core/u64@1>","value":["3","1","9"]}});
        if family == "fold" {
            inputs["initial"] = json!({"type":"core/u64@1","value":"0"});
        }
        let end_pins = if family == "map" {
            json!({"U":"core/u64@1"})
        } else if family == "fold" {
            json!({"A":"core/u64@1"})
        } else {
            json!({"T":"core/u64@1"})
        };
        let end_inputs = if family == "filter" {
            json!({"keep":{"type":"core/bool@1","value":false}})
        } else {
            json!({})
        };
        let doc = graph(
            &package,
            vec![
                zone(1, family, false, "z", pins, inputs),
                zone(2, family, true, "z", end_pins, end_inputs),
            ],
            vec![edge(
                1,
                1,
                "element",
                2,
                if family == "fold" { "next" } else { "item" },
            )],
            vec![export(2, "result")],
        );
        let resolver = Counting::new(package);
        let plan = compile(&doc, snapshot, &resolver);
        assert_eq!(plan.zones().len(), 1);
        assert_eq!(resolver.pure.load(Ordering::Relaxed), 0);
        let end = plan.nodes().last().unwrap();
        let prefix = if family == "fold" { 2 } else { 1 };
        assert!(matches!(end.operation(),DataOperation::Zone{prefix:p,..} if *p==prefix));
        assert_eq!(
            end.inputs()[prefix].source.port.to_string(),
            if family == "filter" {
                "keep"
            } else if family == "fold" {
                "next"
            } else {
                "item"
            }
        );
    }
}

#[test]
fn pure_borrowed_result_api_refuses_zones_without_binding_callbacks() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![
            repeat(1, "z", 100, 7),
            zone(2, "repeat", true, "z", json!({"A":"core/u64@1"}), json!({})),
        ],
        vec![edge(1, 1, "carried", 2, "next")],
        vec![export(2, "result")],
    );
    let resolver = Counting::new(package);
    let analysis = analyze(&doc, &snapshot);
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
    assert_eq!(resolver.pure.load(Ordering::Relaxed), 0);
}

#[test]
fn generic_calls_with_constant_arguments_keep_separate_zone_programs() {
    let (package, snapshot) = catalog();
    let call = |n, ty, value: Json| {
        json!({"id":id(n),"definition_id":"core/call","definition_version":1,
        "config_version":1,"config":{"function":id(100)},"type_pins":{"T":ty},
        "inputs":{"value":{"type":ty,"value":value}}})
    };
    let base = graph(
        &package,
        vec![
            call(1, "core/u64@1", json!("7")),
            call(2, "core/string@1", json!("hello")),
        ],
        vec![],
        vec![export(1, "result"), export(2, "result")],
    );
    let mut saved: Json = serde_json::from_str(&to_json(&base).unwrap()).unwrap();
    let marker = |n, name| json!({"id":id(n),"definition_id":format!("core/{name}"),"definition_version":1,"config_version":1,"config":{}});
    saved["functions"] = json!([{"id":id(100),"name":"repeat_identity","kind":"pure","type_params":[{"name":"T"}],
        "inputs":[{"port_id":"value","direction":"input","kind":"data","type":"$T"}],
        "outputs":[{"port_id":"result","direction":"output","kind":"data","type":"$T"}],
        "nodes":[marker(101,"function-entry"),zone(102,"repeat",false,"z",json!({}),json!({"count":{"type":"core/i64@1","value":"3"}})),
            zone(103,"repeat",true,"z",json!({}),json!({})),marker(104,"function-outputs")],
        "edges":[edge(101,101,"value",102,"initial"),edge(102,102,"carried",103,"next"),edge(103,103,"result",104,"result")]}]);
    let doc = from_json(&saved.to_string()).unwrap();
    let resolver = Counting::new(package);
    let plan = compile(&doc, snapshot.clone(), &resolver);
    assert_eq!(plan.zones().len(), 2);
    assert_ne!(
        plan.zones()[0].call_instance(),
        plan.zones()[1].call_instance()
    );
    assert_eq!(plan.zones()[0].begin(), plan.zones()[1].begin());
    assert!(
        plan.nodes()
            .iter()
            .filter(|n| matches!(n.operation(), DataOperation::Call { .. }))
            .count()
            == 2
    );
    assert_eq!(resolver.pure.load(Ordering::Relaxed), 0);
    let mut active = activation(
        &doc,
        &snapshot,
        std::sync::Arc::new(plan),
        Default::default(),
    );
    assert!(matches!(
        result_at(&mut active, 0, 1),
        Ok(orange_model::Value::U64(7))
    ));
    assert!(
        matches!(result_at(&mut active,1,1),Ok(orange_model::Value::Str(s)) if s.as_str()=="hello")
    );
}

#[test]
fn dense_zone_metadata_is_admitted_before_any_provider_binding() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![
            repeat(1, "z", 9, 7),
            math(2, "add", 0, 1),
            zone(3, "repeat", true, "z", json!({"A":"core/u64@1"}), json!({})),
        ],
        vec![edge(1, 1, "carried", 2, "a"), edge(2, 2, "sum", 3, "next")],
        vec![export(3, "result")],
    );
    let analysis = analyze(&doc, &snapshot);
    let resolver = Counting::new(package);
    let options = CompileOptions {
        limits: CompileLimits {
            max_constant_bytes: 128,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(matches!(
        compile_data(&doc, snapshot, &analysis, &resolver, options),
        Err(CompileError::ResourceLimit)
    ));
    assert_eq!(resolver.pure.load(Ordering::Relaxed), 0);
}

#[test]
fn entropy_capture_keeps_the_outer_domain_while_body_is_iteration_owned() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![
            repeat(1, "z", 20, 0),
            random(2),
            math(3, "add", 0, 0),
            zone(4, "repeat", true, "z", json!({"A":"core/u64@1"}), json!({})),
        ],
        vec![
            edge(1, 1, "carried", 3, "a"),
            edge(2, 2, "value", 3, "b"),
            edge(3, 3, "sum", 4, "next"),
        ],
        vec![export(4, "result")],
    );
    let resolver = Counting::new(package);
    let plan = compile(&doc, snapshot.clone(), &resolver);
    let capture = plan
        .nodes()
        .iter()
        .find(|n| matches!(n.operation(), DataOperation::Entropy(_)))
        .unwrap();
    assert_eq!(capture.zone(), None);
    let body = plan
        .nodes()
        .iter()
        .find(|n| n.node().to_string() == id(3))
        .unwrap();
    assert!(body.zone().is_some());
    assert_eq!(resolver.draws.load(Ordering::Relaxed), 0);
    let mut active = activation(
        &doc,
        &snapshot,
        std::sync::Arc::new(plan),
        Default::default(),
    );
    for invocation in 0..2 {
        let sample = orange_runtime::EntropyStream::new(orange_runtime::EntropyKey {
            seed: 17,
            frame: orange_runtime::FrameId::ROOT,
            node: id(2).parse().unwrap(),
            activation_sequence: 1,
            invocation,
        })
        .next_u64()
        .unwrap();
        assert!(
            matches!(result(&mut active,1),Ok(orange_model::Value::U64(value)) if value==sample.wrapping_mul(20))
        );
        active.begin_epoch().unwrap();
    }
    assert_eq!(resolver.draws.load(Ordering::Relaxed), 2);
}

#[test]
fn closed_zone_parameters_and_body_values_cannot_be_exported_directly() {
    let (package, snapshot) = catalog();
    for target in [export(1, "carried"), export(2, "sum")] {
        let doc = graph(
            &package,
            vec![
                repeat(1, "z", 3, 0),
                math(2, "add", 0, 1),
                zone(3, "repeat", true, "z", json!({"A":"core/u64@1"}), json!({})),
            ],
            vec![edge(1, 1, "carried", 2, "a"), edge(2, 2, "sum", 3, "next")],
            vec![target],
        );
        let analysis = analyze(&doc, &snapshot);
        assert!(
            analysis
                .report()
                .diagnostics
                .iter()
                .any(|d| d.code == orange_model::validation::DiagnosticCode::ZoneStructure)
        );
        assert!(!analysis.is_ready_for_lowering(&doc, &snapshot));
    }
}

#[test]
fn filter_cannot_change_element_type_and_repeat_cannot_change_carry_type() {
    let (package, snapshot) = catalog();
    for family in ["filter", "repeat", "fold"] {
        let pins = if family == "repeat" {
            json!({"A":"core/u64@1"})
        } else if family == "fold" {
            json!({"T":"core/u64@1","A":"core/u64@1"})
        } else {
            json!({"T":"core/u64@1"})
        };
        let mut inputs = if family == "repeat" {
            json!({"count":{"type":"core/i64@1","value":"1"}})
        } else {
            json!({"items":{"type":"core/list@1<core/u64@1>","value":[]}})
        };
        if family != "filter" {
            inputs["initial"] = literal(7);
        }
        let end_pins = if family == "filter" {
            json!({"T":"core/bool@1"})
        } else {
            json!({"A":"core/bool@1"})
        };
        let end_inputs = if family == "filter" {
            json!({"keep":{"type":"core/bool@1","value":true},"item":{"type":"core/bool@1","value":false}})
        } else {
            json!({"next":{"type":"core/bool@1","value":false}})
        };
        let doc = graph(
            &package,
            vec![
                zone(1, family, false, "z", pins, inputs),
                zone(2, family, true, "z", end_pins, end_inputs),
            ],
            vec![],
            vec![export(2, "result")],
        );
        let analysis = analyze(&doc, &snapshot);
        assert!(
            analysis
                .report()
                .diagnostics
                .iter()
                .any(|d| d.code == orange_model::validation::DiagnosticCode::TypeMismatch),
            "{:?}",
            analysis.report().diagnostics
        );
        assert!(!analysis.is_ready_for_lowering(&doc, &snapshot));
    }
}

#[test]
fn nested_zone_layout_resolves_parents_independently_of_uuid_order() {
    let (package, snapshot) = catalog();
    let mut inner = repeat(2, "inner", 2, 0);
    inner["inputs"].as_object_mut().unwrap().remove("initial");
    let doc = graph(
        &package,
        vec![
            repeat(1, "outer", 3, 0),
            inner,
            math(3, "add", 0, 1),
            zone(
                4,
                "repeat",
                true,
                "inner",
                json!({"A":"core/u64@1"}),
                json!({}),
            ),
            zone(
                5,
                "repeat",
                true,
                "outer",
                json!({"A":"core/u64@1"}),
                json!({}),
            ),
        ],
        vec![
            edge(1, 1, "carried", 2, "initial"),
            edge(2, 2, "carried", 3, "a"),
            edge(3, 3, "sum", 4, "next"),
            edge(4, 4, "result", 5, "next"),
        ],
        vec![export(5, "result")],
    );
    let resolver = Counting::new(package);
    let plan = compile(&doc, snapshot.clone(), &resolver);
    let inner = plan
        .zones()
        .iter()
        .find(|z| z.end().to_string() == id(4))
        .unwrap();
    let parent = inner.parent().unwrap();
    assert_eq!(plan.zones()[parent.index()].end().to_string(), id(5));
    let inner_end = plan
        .nodes()
        .iter()
        .find(|n| n.node().to_string() == id(4))
        .unwrap();
    assert_eq!(inner_end.zone(), Some(parent));
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 0);
    let mut active = activation(
        &doc,
        &snapshot,
        std::sync::Arc::new(plan),
        Default::default(),
    );
    assert!(matches!(
        result(&mut active, 1),
        Ok(orange_model::Value::U64(6))
    ));
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 6);
}

fn activation(
    doc: &orange_model::GraphDocument,
    snapshot: &orange_model::catalog::RegistrySnapshot,
    plan: std::sync::Arc<orange_compiler::DataPlan>,
    options: orange_runtime::DataOptions,
) -> orange_runtime::DataActivation {
    let mut activation =
        orange_runtime::DataActivation::start(plan, doc, snapshot, 17, 1, options).unwrap();
    activation.begin_epoch().unwrap();
    activation
}
fn result(
    activation: &mut orange_runtime::DataActivation,
    budget: usize,
) -> Result<orange_model::Value, orange_compiler::ConstantFault> {
    result_at(activation, 0, budget)
}
fn result_at(
    activation: &mut orange_runtime::DataActivation,
    index: usize,
    budget: usize,
) -> Result<orange_model::Value, orange_compiler::ConstantFault> {
    for _ in 0..100000 {
        match activation.poll_export(index, &mut { budget }).unwrap() {
            orange_runtime::DataPoll::Pending => {}
            orange_runtime::DataPoll::Ready(Some(e)) => {
                return e.outcome.cloned().map_err(Clone::clone);
            }
            orange_runtime::DataPoll::Ready(None) => panic!("missing export"),
        }
    }
    panic!("zone did not complete")
}

#[test]
fn repeat_executes_fresh_carries_across_slices_and_epochs() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![
            repeat(1, "z", 17, 7),
            math(2, "add", 0, 1),
            zone(3, "repeat", true, "z", json!({"A":"core/u64@1"}), json!({})),
        ],
        vec![edge(1, 1, "carried", 2, "a"), edge(2, 2, "sum", 3, "next")],
        vec![export(3, "result")],
    );
    let resolver = Counting::new(package);
    let plan = std::sync::Arc::new(compile(&doc, snapshot.clone(), &resolver));
    for budget in [1, 37, 10000] {
        let mut activation = activation(&doc, &snapshot, plan.clone(), Default::default());
        for _ in 0..3 {
            assert!(matches!(
                result(&mut activation, budget),
                Ok(orange_model::Value::U64(24))
            ));
            activation.begin_epoch().unwrap();
        }
    }
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 17 * 3 * 3);
}

#[test]
fn map_fold_filter_execute_in_source_order_with_native_packed_results() {
    for family in ["map", "fold", "filter"] {
        let (package, snapshot) = catalog();
        let pins = if family == "fold" {
            json!({"T":"core/u64@1","A":"core/u64@1"})
        } else {
            json!({"T":"core/u64@1"})
        };
        let mut inputs = json!({"items":{"type":"core/list@1<core/u64@1>","value":["3","1","9"]}});
        if family == "fold" {
            inputs["initial"] = literal(7);
        }
        let endpins = if family == "fold" {
            json!({"A":"core/u64@1"})
        } else if family == "map" {
            json!({"U":"core/u64@1"})
        } else {
            json!({"T":"core/u64@1"})
        };
        let endinputs = if family == "filter" {
            json!({"keep":{"type":"core/bool@1","value":true}})
        } else {
            json!({})
        };
        let mut nodes = vec![
            zone(1, family, false, "z", pins, inputs),
            zone(3, family, true, "z", endpins, endinputs),
        ];
        let edges = if family == "filter" {
            vec![edge(1, 1, "element", 3, "item")]
        } else {
            nodes.push(math(2, "add", 0, 2));
            let mut edges = vec![
                edge(1, 1, "element", 2, "a"),
                edge(
                    2,
                    2,
                    "sum",
                    3,
                    if family == "fold" { "next" } else { "item" },
                ),
            ];
            if family == "fold" {
                edges.push(edge(3, 1, "accumulator", 2, "b"));
            }
            edges
        };
        let doc = graph(&package, nodes, edges, vec![export(3, "result")]);
        let resolver = Counting::new(package);
        let plan = std::sync::Arc::new(compile(&doc, snapshot.clone(), &resolver));
        let mut activation = activation(&doc, &snapshot, plan, Default::default());
        match result(&mut activation, 1).unwrap() {
            orange_model::Value::U64(value) if family == "fold" => assert_eq!(value, 20),
            orange_model::Value::List(items) => {
                let orange_model::value::ListRepr::U64(items) = items.as_ref() else {
                    panic!("expected packed u64")
                };
                assert_eq!(
                    &**items,
                    if family == "map" {
                        &[5, 3, 11]
                    } else {
                        &[3, 1, 9]
                    }
                );
            }
            value => panic!("unexpected {value:?}"),
        }
    }
}

#[test]
fn zero_repeat_skips_body_fault_but_negative_count_preserves_begin_identity() {
    for count in [0, -1, 1] {
        let (package, snapshot) = catalog();
        let doc = graph(
            &package,
            vec![
                repeat(1, "z", count, 7),
                math(2, "divide", 0, 0),
                zone(3, "repeat", true, "z", json!({"A":"core/u64@1"}), json!({})),
            ],
            vec![
                edge(1, 1, "carried", 2, "a"),
                edge(2, 2, "quotient", 3, "next"),
            ],
            vec![export(3, "result")],
        );
        let resolver = Counting::new(package);
        let plan = std::sync::Arc::new(compile(&doc, snapshot.clone(), &resolver));
        let mut activation = activation(&doc, &snapshot, plan, Default::default());
        let outcome = result(&mut activation, 1);
        if count == 0 {
            assert!(matches!(outcome, Ok(orange_model::Value::U64(7))));
        } else {
            let fault = outcome.unwrap_err();
            assert_eq!(fault.source.to_string(), id(if count < 0 { 1 } else { 2 }));
            assert_eq!(
                fault.fault.code,
                if count < 0 {
                    "core/OutOfRange"
                } else {
                    "core/DivideByZero"
                }
            );
        }
        assert_eq!(
            resolver.folds.load(Ordering::Relaxed),
            usize::from(count > 0)
        );
    }
}

#[test]
fn filter_false_still_demands_item_and_empty_input_skips_it() {
    for items in [json!([]), json!(["3"])] {
        let empty = items.as_array().unwrap().is_empty();
        let (package, snapshot) = catalog();
        let doc = graph(
            &package,
            vec![
                zone(
                    1,
                    "filter",
                    false,
                    "z",
                    json!({"T":"core/u64@1"}),
                    json!({"items":{"type":"core/list@1<core/u64@1>","value":items}}),
                ),
                math(2, "divide", 0, 0),
                zone(
                    3,
                    "filter",
                    true,
                    "z",
                    json!({"T":"core/u64@1"}),
                    json!({"keep":{"type":"core/bool@1","value":false}}),
                ),
            ],
            vec![
                edge(1, 1, "element", 2, "a"),
                edge(2, 2, "quotient", 3, "item"),
            ],
            vec![export(3, "result")],
        );
        let resolver = Counting::new(package);
        let plan = std::sync::Arc::new(compile(&doc, snapshot.clone(), &resolver));
        let mut activation = activation(&doc, &snapshot, plan, Default::default());
        let outcome = result(&mut activation, 1);
        if empty {
            let orange_model::Value::List(items) = outcome.unwrap() else {
                panic!()
            };
            assert!(items.is_empty());
        } else {
            assert_eq!(outcome.unwrap_err().source.to_string(), id(2));
        }
        assert_eq!(resolver.folds.load(Ordering::Relaxed), usize::from(!empty));
    }
}

#[test]
fn negative_repeat_still_collects_initial_before_its_own_semantic_fault() {
    let (package, snapshot) = catalog();
    let mut begin = repeat(1, "z", -1, 0);
    begin["inputs"].as_object_mut().unwrap().remove("initial");
    let doc = graph(
        &package,
        vec![
            begin,
            math(2, "divide", 1, 0),
            zone(3, "repeat", true, "z", json!({"A":"core/u64@1"}), json!({})),
        ],
        vec![
            edge(1, 2, "quotient", 1, "initial"),
            edge(2, 1, "carried", 3, "next"),
        ],
        vec![export(3, "result")],
    );
    let resolver = Counting::new(package);
    let plan = std::sync::Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let mut active = activation(&doc, &snapshot, plan, Default::default());
    let fault = result(&mut active, 1).unwrap_err();
    assert_eq!(fault.source.to_string(), id(2));
    assert_eq!(fault.fault.code, "core/DivideByZero");
}

#[test]
fn canceled_partial_zone_restarts_with_fresh_carry_without_reviving_failed_epoch() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![
            repeat(1, "z", 100, 0),
            math(2, "add", 0, 1),
            zone(3, "repeat", true, "z", json!({"A":"core/u64@1"}), json!({})),
        ],
        vec![edge(1, 1, "carried", 2, "a"), edge(2, 2, "sum", 3, "next")],
        vec![export(3, "result")],
    );
    let resolver = Counting::new(package);
    let plan = std::sync::Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let token = orange_runtime::CancelToken::new();
    let mut active = activation(
        &doc,
        &snapshot,
        plan,
        orange_runtime::DataOptions {
            cancel: Some(token.clone()),
            ..Default::default()
        },
    );
    while resolver.folds.load(Ordering::Relaxed) < 5 {
        assert!(matches!(
            active.poll_export(0, &mut 1).unwrap(),
            orange_runtime::DataPoll::Pending
        ));
    }
    let fuel = active.fuel_remaining();
    token.cancel();
    assert!(matches!(
        active.poll_export(0, &mut 1),
        Err(orange_runtime::DataError::Cancelled)
    ));
    token.clear();
    assert!(matches!(
        active.poll_export(0, &mut 1),
        Err(orange_runtime::DataError::Cancelled)
    ));
    active.begin_epoch().unwrap();
    assert!(matches!(
        result(&mut active, 1),
        Ok(orange_model::Value::U64(100))
    ));
    assert!(active.fuel_remaining() < fuel);
}

#[test]
fn map_builder_quota_failure_releases_machine_and_allows_a_smaller_next_epoch() {
    let (package, snapshot) = catalog();
    let get = json!({"id":id(4),"definition_id":"core/get-variable","definition_version":1,"config_version":1,"config":{"variable":id(100)}});
    let base = graph(
        &package,
        vec![
            zone(1, "map", false, "z", json!({"T":"core/u64@1"}), json!({})),
            math(2, "add", 0, 1),
            zone(3, "map", true, "z", json!({"U":"core/u64@1"}), json!({})),
            get,
        ],
        vec![
            edge(1, 4, "value", 1, "items"),
            edge(2, 1, "element", 2, "a"),
            edge(3, 2, "sum", 3, "item"),
        ],
        vec![export(3, "result")],
    );
    let mut raw: Json = serde_json::from_str(&to_json(&base).unwrap()).unwrap();
    raw["variables"] =
        json!([{"id":id(100),"name":"items","type":"core/list@1<core/u64@1>","scope":"run"}]);
    let doc = from_json(&raw.to_string()).unwrap();
    let resolver = Counting::new(package);
    let plan = std::sync::Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let input = plan
        .host_inputs()
        .iter()
        .position(|n| plan.nodes()[n.index()].node().to_string() == id(4))
        .unwrap();
    let mut active = activation(
        &doc,
        &snapshot,
        plan,
        orange_runtime::DataOptions {
            limits: orange_runtime::DataLimits {
                max_retained_bytes: 3000,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let list = |values: Vec<u64>| {
        orange_model::Value::List(std::sync::Arc::new(orange_model::value::ListRepr::U64(
            values.into_boxed_slice(),
        )))
    };
    active.set_input(input, list((0..100).collect())).unwrap();
    let mut rejected = false;
    for _ in 0..10000 {
        match active.poll_export(0, &mut 1) {
            Err(orange_runtime::DataError::ResourceLimit) => {
                rejected = true;
                break;
            }
            Ok(orange_runtime::DataPoll::Pending) => {}
            _ => panic!("large builder should exceed its quota"),
        }
    }
    assert!(rejected);
    assert!(resolver.folds.load(Ordering::Relaxed) > 0);
    assert!(resolver.folds.load(Ordering::Relaxed) < 100);
    active.begin_epoch().unwrap();
    active.set_input(input, list(vec![7, 2])).unwrap();
    let orange_model::Value::List(values) = result(&mut active, 1).unwrap() else {
        panic!()
    };
    let orange_model::value::ListRepr::U64(values) = values.as_ref() else {
        panic!()
    };
    assert_eq!(&**values, &[8, 3]);
}

#[test]
fn fold_source_retained_by_its_slot_is_not_charged_again_by_the_zone() {
    let (package, snapshot) = catalog();
    let get = json!({"id":id(4),"definition_id":"core/get-variable","definition_version":1,"config_version":1,"config":{"variable":id(100)}});
    let base = graph(
        &package,
        vec![
            zone(
                1,
                "fold",
                false,
                "z",
                json!({"T":"core/u64@1","A":"core/u64@1"}),
                json!({"initial":literal(0)}),
            ),
            math(2, "add", 0, 0),
            zone(3, "fold", true, "z", json!({"A":"core/u64@1"}), json!({})),
            get,
        ],
        vec![
            edge(1, 4, "value", 1, "items"),
            edge(2, 1, "element", 2, "a"),
            edge(3, 1, "accumulator", 2, "b"),
            edge(4, 2, "sum", 3, "next"),
        ],
        vec![export(3, "result")],
    );
    let mut raw: Json = serde_json::from_str(&to_json(&base).unwrap()).unwrap();
    raw["variables"] =
        json!([{"id":id(100),"name":"items","type":"core/list@1<core/u64@1>","scope":"run"}]);
    let doc = from_json(&raw.to_string()).unwrap();
    let resolver = Counting::new(package);
    let plan = std::sync::Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let input = plan
        .host_inputs()
        .iter()
        .position(|n| plan.nodes()[n.index()].node().to_string() == id(4))
        .unwrap();
    // A 1,000-item list charges about 16,000 bytes: it fits once in 24,000, not twice.
    let mut active = activation(
        &doc,
        &snapshot,
        plan,
        orange_runtime::DataOptions {
            limits: orange_runtime::DataLimits {
                max_retained_bytes: 24_000,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let items: Box<[u64]> = (0..1000).collect();
    active
        .set_input(
            input,
            orange_model::Value::List(std::sync::Arc::new(orange_model::value::ListRepr::U64(
                items,
            ))),
        )
        .unwrap();
    assert!(matches!(
        result(&mut active, 64),
        Ok(orange_model::Value::U64(499_500))
    ));
}

#[test]
fn map_preserves_native_strings_and_generic_nested_lists() {
    for (ty, items) in [
        ("core/string@1", json!(["z", "a"])),
        ("core/list@1<core/u64@1>", json!([["3", "1"], ["9"]])),
    ] {
        let (package, snapshot) = catalog();
        let doc = graph(
            &package,
            vec![
                zone(
                    1,
                    "map",
                    false,
                    "z",
                    json!({"T":ty}),
                    json!({"items":{"type":format!("core/list@1<{ty}>"),"value":items}}),
                ),
                zone(2, "map", true, "z", json!({"U":ty}), json!({})),
            ],
            vec![edge(1, 1, "element", 2, "item")],
            vec![export(2, "result")],
        );
        let resolver = Counting::new(package);
        let plan = std::sync::Arc::new(compile(&doc, snapshot.clone(), &resolver));
        let mut active = activation(&doc, &snapshot, plan, Default::default());
        let orange_model::Value::List(values) = result(&mut active, 1).unwrap() else {
            panic!()
        };
        if ty == "core/string@1" {
            let orange_model::value::ListRepr::Str(values) = values.as_ref() else {
                panic!("strings should stay packed")
            };
            assert_eq!(
                values.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                ["z", "a"]
            );
        } else {
            let orange_model::value::ListRepr::Values(values) = values.as_ref() else {
                panic!("nested values need typed fallback")
            };
            let orange_model::Value::List(first) = &values[0] else {
                panic!()
            };
            assert!(matches!(first.get(0), Some(orange_model::Value::U64(3))));
            assert!(matches!(first.get(1), Some(orange_model::Value::U64(1))));
            assert_eq!(first.len(), 2);
            assert_eq!(values.len(), 2);
        }
    }
}

#[test]
fn body_call_entropy_uses_fresh_iteration_samples_and_original_call_stream_identity() {
    let (package, snapshot) = catalog();
    let call = json!({"id":id(2),"definition_id":"core/call","definition_version":1,"config_version":1,"config":{"function":id(100)}});
    let base = graph(
        &package,
        vec![
            zone(
                1,
                "map",
                false,
                "z",
                json!({"T":"core/u64@1"}),
                json!({"items":{"type":"core/list@1<core/u64@1>","value":["7","8","9","10"]}}),
            ),
            call,
            zone(3, "map", true, "z", json!({"U":"core/u64@1"}), json!({})),
        ],
        vec![
            edge(1, 1, "element", 2, "unused"),
            edge(2, 2, "result", 3, "item"),
        ],
        vec![export(3, "result")],
    );
    let mut raw: Json = serde_json::from_str(&to_json(&base).unwrap()).unwrap();
    let core = |n, name| json!({"id":id(n),"definition_id":format!("core/{name}"),"definition_version":1,"config_version":1,"config":{}});
    raw["functions"] = json!([{"id":id(100),"name":"draw","kind":"volatile",
        "inputs":[{"port_id":"unused","direction":"input","kind":"data","type":"core/u64@1"}],
        "outputs":[{"port_id":"result","direction":"output","kind":"data","type":"core/u64@1"}],
        "nodes":[core(101,"function-entry"),core(104,"function-outputs"),random(103)],
        "edges":[edge(101,103,"value",104,"result")]}]);
    let doc = from_json(&raw.to_string()).unwrap();
    let resolver = Counting::new(package);
    let plan = std::sync::Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let mut active = activation(&doc, &snapshot, plan, Default::default());
    for epoch in 0..2 {
        let expected: Vec<_> = (0..4)
            .map(|i| {
                orange_runtime::EntropyStream::new(orange_runtime::EntropyKey {
                    seed: 17,
                    frame: orange_runtime::FrameId::ROOT.call(id(2).parse().unwrap()),
                    node: id(103).parse().unwrap(),
                    activation_sequence: 1,
                    invocation: epoch * 4 + i,
                })
                .next_u64()
                .unwrap()
            })
            .collect();
        let orange_model::Value::List(values) = result(&mut active, 1).unwrap() else {
            panic!()
        };
        let orange_model::value::ListRepr::U64(values) = values.as_ref() else {
            panic!()
        };
        assert_eq!(&**values, &expected);
        assert_eq!(
            resolver.draws.load(Ordering::Relaxed),
            (epoch as usize + 1) * 4
        );
        result(&mut active, 1).unwrap();
        assert_eq!(
            resolver.draws.load(Ordering::Relaxed),
            (epoch as usize + 1) * 4
        );
        active.begin_epoch().unwrap();
    }
}

#[test]
fn zone_state_captures_are_outer_epoch_inputs_across_single_work_item_slices() {
    let (package, snapshot) = catalog();
    let get = json!({"id":id(4),"definition_id":"core/get-variable","definition_version":1,"config_version":1,"config":{"variable":id(100)}});
    let base = graph(
        &package,
        vec![
            repeat(1, "z", 4, 0),
            math(2, "add", 0, 0),
            zone(3, "repeat", true, "z", json!({"A":"core/u64@1"}), json!({})),
            get,
        ],
        vec![
            edge(1, 1, "carried", 2, "a"),
            edge(2, 4, "value", 2, "b"),
            edge(3, 2, "sum", 3, "next"),
        ],
        vec![export(3, "result")],
    );
    let mut raw: Json = serde_json::from_str(&to_json(&base).unwrap()).unwrap();
    raw["variables"] = json!([{"id":id(100),"name":"capture","type":"core/u64@1","scope":"run"}]);
    let doc = from_json(&raw.to_string()).unwrap();
    let resolver = Counting::new(package);
    let plan = std::sync::Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let input = plan
        .host_inputs()
        .iter()
        .position(|n| plan.nodes()[n.index()].node().to_string() == id(4))
        .unwrap();
    let mut active = activation(&doc, &snapshot, plan, Default::default());
    for capture in [11, 20] {
        active
            .set_input(input, orange_model::Value::U64(capture))
            .unwrap();
        assert!(matches!(
            active.poll_export(0, &mut 1).unwrap(),
            orange_runtime::DataPoll::Pending
        ));
        assert!(matches!(
            active.set_input(input, orange_model::Value::U64(99)),
            Err(orange_runtime::DataError::InvalidInput { .. })
        ));
        assert!(
            matches!(result(&mut active,1),Ok(orange_model::Value::U64(value)) if value==capture*4)
        );
        active.begin_epoch().unwrap();
    }
}

#[test]
fn run_pumps_zone_inputs_atomically_and_stop_discards_partial_results() {
    let (package, snapshot) = catalog();
    let start = json!({"id":id(10),"definition_id":"core/on-start","definition_version":1,"config_version":1,"config":{},
        "activation":{"mode":"queue","capacity":4,"overflow":"drop-newest","on_fault":"continue","order":0}});
    let set = json!({"id":id(20),"definition_id":"core/set-variable","definition_version":1,"config_version":1,"config":{"variable":id(100)}});
    let base = graph(
        &package,
        vec![
            start,
            set,
            repeat(1, "z", 20, 0),
            math(2, "add", 0, 1),
            zone(3, "repeat", true, "z", json!({"A":"core/u64@1"}), json!({})),
        ],
        vec![
            edge(1, 10, "then", 20, "in"),
            edge(2, 1, "carried", 2, "a"),
            edge(3, 2, "sum", 3, "next"),
            edge(4, 3, "result", 20, "value"),
        ],
        vec![],
    );
    let mut raw: Json = serde_json::from_str(&to_json(&base).unwrap()).unwrap();
    raw["variables"] =
        json!([{"id":id(100),"name":"output","type":"core/u64@1","scope":"run","init":literal(0)}]);
    let doc = from_json(&raw.to_string()).unwrap();
    let resolver = Counting::new(package);
    let analysis = analyze(&doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(&doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    let plan = std::sync::Arc::new(
        orange_compiler::compile_control(
            &doc,
            snapshot.clone(),
            &analysis,
            &resolver,
            Default::default(),
        )
        .unwrap(),
    );
    for stop in [false, true] {
        let before = resolver.folds.load(Ordering::Relaxed);
        let mut run =
            orange_runtime::Run::start(plan.clone(), &doc, &snapshot, 17, Default::default())
                .unwrap();
        while resolver.folds.load(Ordering::Relaxed) < before + 5 {
            run.pump(1).unwrap();
            assert!(matches!(
                run.variable(id(100).parse().unwrap()),
                Some(orange_model::Value::U64(0))
            ));
        }
        if stop {
            run.request_stop(orange_runtime::StopMode::Immediate)
                .unwrap();
        }
        for _ in 0..10000 {
            if run.state() == orange_runtime::RunState::Stopped {
                break;
            }
            let progress = run.pump(1).unwrap();
            if !progress.ready && progress.next_wakeup.is_none() {
                break;
            }
        }
        assert!(
            matches!(run.variable(id(100).parse().unwrap()),Some(orange_model::Value::U64(value)) if *value==if stop {0} else {20})
        );
        if stop {
            assert_eq!(run.state(), orange_runtime::RunState::Stopped);
        }
    }
}

#[test]
fn even_empty_body_repeat_spends_operation_and_activation_fuel() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![
            repeat(1, "z", i64::MAX, 7),
            zone(2, "repeat", true, "z", json!({"A":"core/u64@1"}), json!({})),
        ],
        vec![edge(1, 1, "carried", 2, "next")],
        vec![export(2, "result")],
    );
    let resolver = Counting::new(package);
    let plan = std::sync::Arc::new(compile(&doc, snapshot.clone(), &resolver));
    for fuel in [false, true] {
        let mut limits = orange_runtime::DataLimits::default();
        if fuel {
            limits.max_activation_operations = 4;
        } else {
            limits.max_operations = 3;
        }
        let mut active = activation(
            &doc,
            &snapshot,
            plan.clone(),
            orange_runtime::DataOptions {
                limits,
                ..Default::default()
            },
        );
        let expected = if fuel {
            orange_runtime::DataError::FuelExhausted
        } else {
            orange_runtime::DataError::ResourceLimit
        };
        let mut rejected = false;
        for _ in 0..100 {
            match active.poll_export(0, &mut 1) {
                Err(error) => {
                    assert_eq!(error, expected);
                    rejected = true;
                    break;
                }
                Ok(orange_runtime::DataPoll::Pending) => {}
                _ => panic!("unbounded repeat completed"),
            }
        }
        assert!(rejected);
        if fuel {
            assert_eq!(
                active.begin_epoch(),
                Err(orange_runtime::DataError::FuelExhausted)
            );
        }
    }
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 0);
}

#[test]
fn try_captures_zone_graph_faults_with_original_begin_identity() {
    let (package, snapshot) = catalog();
    let catch = json!({"id":id(3),"definition_id":"core/try","definition_version":1,"config_version":1,"config":{},"type_pins":{"T":"core/u64@1"}});
    let doc = graph(
        &package,
        vec![
            repeat(1, "z", -1, 7),
            zone(2, "repeat", true, "z", json!({"A":"core/u64@1"}), json!({})),
            catch,
        ],
        vec![
            edge(1, 1, "carried", 2, "next"),
            edge(2, 2, "result", 3, "value"),
        ],
        vec![export(3, "result")],
    );
    let resolver = Counting::new(package);
    let plan = std::sync::Arc::new(compile(&doc, snapshot.clone(), &resolver));
    let mut active = activation(&doc, &snapshot, plan, Default::default());
    let orange_model::Value::Err(error) = result(&mut active, 1).unwrap() else {
        panic!()
    };
    assert_eq!(error.code, "core/OutOfRange");
    assert_eq!(error.node, Some(id(1).parse().unwrap()));
}

#[test]
fn iteration_indices_are_i64_and_map_may_change_element_type() {
    for family in ["repeat", "map", "fold"] {
        let (package, snapshot) = catalog();
        let pins = match family {
            "repeat" => json!({"A":"core/i64@1"}),
            "fold" => json!({"T":"core/u64@1","A":"core/i64@1"}),
            _ => json!({"T":"core/u64@1"}),
        };
        let mut inputs = if family == "repeat" {
            json!({"count":{"type":"core/i64@1","value":"3"}})
        } else {
            json!({"items":{"type":"core/list@1<core/u64@1>","value":["7","2","9"]}})
        };
        if family != "map" {
            inputs["initial"] = json!({"type":"core/i64@1","value":"-7"});
        }
        let endpins = if family == "map" {
            json!({"U":"core/i64@1"})
        } else {
            json!({"A":"core/i64@1"})
        };
        let doc = graph(
            &package,
            vec![
                zone(1, family, false, "z", pins, inputs),
                zone(2, family, true, "z", endpins, json!({})),
            ],
            vec![edge(
                1,
                1,
                "index",
                2,
                if family == "map" { "item" } else { "next" },
            )],
            vec![export(2, "result")],
        );
        let resolver = Counting::new(package);
        let plan = std::sync::Arc::new(compile(&doc, snapshot.clone(), &resolver));
        let mut active = activation(&doc, &snapshot, plan, Default::default());
        match result(&mut active, 1).unwrap() {
            orange_model::Value::I64(value) => assert_eq!(value, 2),
            orange_model::Value::List(values) => {
                let orange_model::value::ListRepr::I64(values) = values.as_ref() else {
                    panic!()
                };
                assert_eq!(&**values, &[0, 1, 2]);
            }
            _ => panic!(),
        }
    }
}
