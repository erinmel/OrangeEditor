#[path = "support/data_graph.rs"]
mod support;
use orange_compiler::{
    CompileError, CompileOptions, DataOperation, DataStability, FoldedValue, compile_data,
    compile_pure,
};
use orange_model::{Value, analyze, from_json, to_json};
use serde_json::{Value as Json, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use support::*;

#[test]
fn plan_and_export_debug_are_bounded_metadata_while_values_remain_explicitly_readable() {
    let (package, snapshot) = catalog();
    let payload = "PRIVATE_GRAPH_VALUE_".repeat(4096);
    let selected = json!({"id":id(1),"definition_id":"core/select","definition_version":1,"config_version":1,
        "config":{},"type_pins":{"T":"core/json@1"},"inputs":{
            "condition":{"type":"core/bool@1","value":true},
            "if_true":{"type":"core/json@1","value":serde_json::to_string(&payload).unwrap()},
            "if_false":{"type":"core/json@1","value":"null"}}});
    let doc = graph(&package, vec![selected], vec![], vec![export(1, "result")]);
    let resolver = Counting::new(package);
    let data = compile(&doc, snapshot.clone(), &resolver);
    let analysis = analyze(&doc, &snapshot);
    let pure = compile_pure(
        &doc,
        snapshot.clone(),
        &analysis,
        &resolver,
        CompileOptions::default(),
    )
    .unwrap();
    let evaluation = pure
        .evaluate(
            &doc,
            &snapshot,
            orange_compiler::EvaluationOptions::default(),
        )
        .unwrap();
    let export = evaluation.export(0).unwrap().unwrap();
    let Value::Json(value) = export.outcome.unwrap() else {
        panic!("expected JSON")
    };
    assert!(value.as_str().contains("PRIVATE_GRAPH_VALUE_"));
    for debug in [
        format!("{data:#?}"),
        format!("{pure:#?}"),
        format!("{evaluation:#?}"),
        format!("{export:#?}"),
    ] {
        assert!(!debug.contains("PRIVATE_GRAPH_VALUE_"));
        assert!(
            debug.len() < 1024,
            "metadata summary expanded with value size"
        );
    }
}
#[test]
fn entropy_is_bound_not_executed_and_constant_islands_fold_once_across_exports() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![random(1), math(2, "add", 20, 22), math(3, "add", 0, 0)],
        vec![edge(1, 1, "value", 3, "a"), edge(2, 2, "sum", 3, "b")],
        vec![export(3, "sum"), export(1, "value"), export(2, "sum")],
    );
    let resolver = Counting::new(package);
    let plan = compile(&doc, snapshot.clone(), &resolver);
    assert_eq!(resolver.pure.load(Ordering::Relaxed), 2);
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.entropy.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.draws.load(Ordering::Relaxed), 0);
    assert_eq!(plan.nodes().len(), 3);
    assert_eq!(plan.nodes()[0].stability(), DataStability::Epoch);
    assert_eq!(plan.nodes()[1].stability(), DataStability::Const);
    assert_eq!(plan.nodes()[2].stability(), DataStability::Epoch);
    assert!(matches!(
        plan.nodes()[0].operation(),
        DataOperation::Entropy(_)
    ));
    assert_eq!(
        plan.initial_values()[plan.exports()[2].slot.index()],
        Some(FoldedValue::Value(Value::U64(42)))
    );
    assert_eq!(plan.initial_values()[plan.exports()[0].slot.index()], None);
    assert_eq!(plan.resources().slots, 5);
    assert_eq!(plan.resources().entropy_nodes, 1);
    assert!(plan.is_current(&doc, &snapshot));
    assert!(Arc::ptr_eq(plan.snapshot(), &snapshot));
    for node in plan.nodes() {
        for output in node.outputs() {
            assert_eq!(plan.slot_sources()[output.index()].node, node.node());
        }
    }
    assert_eq!(
        plan.value_charge(plan.exports()[0].slot, &Value::I64(0)),
        None
    );
    assert_eq!(
        plan.value_charge(plan.exports()[0].slot, &Value::U64(0)),
        Some(16)
    );
}

#[test]
fn static_selection_eliminates_an_unavailable_entropy_arm_before_binding() {
    let (package, snapshot) = catalog();
    let selection = json!({"id":id(3),"definition_id":"core/select","definition_version":1,"config_version":1,
        "config":{},"type_pins":{"T":"core/u64@1"},"inputs":{"condition":{"type":"core/bool@1","value":true}}});
    let doc = graph(
        &package,
        vec![random(1), math(2, "add", 20, 22), selection],
        vec![
            edge(1, 2, "sum", 3, "if_true"),
            edge(2, 1, "value", 3, "if_false"),
        ],
        vec![export(3, "result")],
    );
    let mut resolver = Counting::new(package);
    resolver.reject = Some(id(1).parse().unwrap());
    let plan = compile(&doc, snapshot, &resolver);
    assert_eq!(resolver.entropy.load(Ordering::Relaxed), 0);
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 1);
    assert_eq!(plan.resources().entropy_nodes, 0);
    assert_eq!(plan.nodes().len(), 2);
    assert_eq!(
        plan.initial_values()[plan.exports()[0].slot.index()],
        Some(FoldedValue::Value(Value::U64(42)))
    );
}

#[test]
fn constant_try_and_direct_exports_preserve_original_faults_and_warnings() {
    let (package, snapshot) = catalog();
    let attempt = json!({"id":id(2),"definition_id":"core/try","definition_version":1,"config_version":1,"config":{},
        "type_pins":{"T":"core/u64@1"}});
    let doc = graph(
        &package,
        vec![math(1, "divide", 1, 0), attempt],
        vec![edge(1, 1, "quotient", 2, "value")],
        vec![export(2, "result"), export(1, "quotient")],
    );
    let resolver = Counting::new(package);
    let plan = compile(&doc, snapshot, &resolver);
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 1);
    assert_eq!(plan.diagnostics().len(), 1);
    let Some(FoldedValue::Value(Value::Err(error))) =
        &plan.initial_values()[plan.exports()[0].slot.index()]
    else {
        panic!("capture absent")
    };
    assert_eq!(error.node, Some(id(1).parse().unwrap()));
    assert_eq!(error.code, "core/DivideByZero");
    let Some(FoldedValue::Fault(fault)) = &plan.initial_values()[plan.exports()[1].slot.index()]
    else {
        panic!("direct fault absent")
    };
    assert_eq!(fault.source, id(1).parse().unwrap());
    assert_eq!(plan.fault_charge(fault), Some(64));
}

#[test]
fn static_false_selection_keeps_only_its_epoch_arm_and_skips_constant_callbacks() {
    let (package, snapshot) = catalog();
    let selection = json!({"id":id(3),"definition_id":"core/select","definition_version":1,"config_version":1,
        "config":{},"type_pins":{"T":"core/u64@1"},"inputs":{"condition":{"type":"core/bool@1","value":false}}});
    let doc = graph(
        &package,
        vec![random(1), math(2, "add", 20, 22), selection],
        vec![
            edge(1, 2, "sum", 3, "if_true"),
            edge(2, 1, "value", 3, "if_false"),
        ],
        vec![export(3, "result")],
    );
    let resolver = Counting::new(package);
    let plan = compile(&doc, snapshot, &resolver);
    assert_eq!(resolver.pure.load(Ordering::Relaxed), 0);
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 0);
    assert_eq!(resolver.entropy.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.draws.load(Ordering::Relaxed), 0);
    assert_eq!(plan.nodes().len(), 2);
    assert!(matches!(
        plan.nodes()[1].operation(),
        DataOperation::Select {
            when_true: 1,
            when_false: 1
        }
    ));
    assert_eq!(
        plan.nodes()[1].inputs()[1].source.port,
        "if_false".parse().unwrap()
    );
    assert_eq!(plan.initial_values()[plan.exports()[0].slot.index()], None);
}

#[test]
fn deep_epoch_graph_lowers_iteratively_and_shared_exports_do_not_rebind() {
    let (package, snapshot) = catalog();
    let mut nodes = vec![random(1)];
    for n in 2..=512 {
        nodes.push(math(n, "add", 0, 1));
    }
    let edges = (2..=512)
        .map(|n| edge(n, n - 1, if n == 2 { "value" } else { "sum" }, n, "a"))
        .collect();
    nodes.reverse();
    let doc = graph(
        &package,
        nodes,
        edges,
        vec![export(512, "sum"), export(256, "sum")],
    );
    let resolver = Counting::new(package);
    let plan = compile(&doc, snapshot, &resolver);
    assert_eq!(plan.nodes().len(), 512);
    assert_eq!(resolver.pure.load(Ordering::Relaxed), 511);
    assert_eq!(resolver.entropy.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 0);
    assert_eq!(resolver.draws.load(Ordering::Relaxed), 0);
    assert_eq!(plan.resources().slots, 1023);
    for node in plan.nodes() {
        for input in node.inputs() {
            if let Some(producer) = input.producer {
                assert!(producer.index() < node.index().index());
            }
        }
    }
}

#[test]
fn stale_quota_cancellation_and_wrong_entropy_abi_publish_no_data_plan() {
    let (package, snapshot) = catalog();
    let doc = graph(&package, vec![random(1)], vec![], vec![export(1, "value")]);
    let analysis = analyze(&doc, &snapshot);
    assert!(analysis.is_ready_for_lowering(&doc, &snapshot));
    let mut resolver = Counting::new(package);
    let reload = from_json(&to_json(&doc).unwrap()).unwrap();
    assert_eq!(
        compile_data(
            &reload,
            snapshot.clone(),
            &analysis,
            &resolver,
            CompileOptions::default()
        )
        .unwrap_err(),
        CompileError::StaleDocument
    );
    let (_, replacement) = catalog();
    assert_eq!(
        compile_data(
            &doc,
            replacement,
            &analysis,
            &resolver,
            CompileOptions::default()
        )
        .unwrap_err(),
        CompileError::StaleRegistry
    );
    let cancel = AtomicBool::new(true);
    assert_eq!(
        compile_data(
            &doc,
            snapshot.clone(),
            &analysis,
            &resolver,
            CompileOptions {
                cancel: Some(&cancel),
                ..Default::default()
            }
        )
        .unwrap_err(),
        CompileError::Cancelled
    );
    for kind in 0..3 {
        let mut limits = orange_compiler::CompileLimits::default();
        match kind {
            0 => limits.max_fold_operations = 0,
            1 => limits.max_slots = 0,
            _ => limits.max_constant_bytes = 0,
        }
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
    resolver.bad_entropy_abi = true;
    assert!(matches!(
        compile_data(
            &doc,
            snapshot,
            &analysis,
            &resolver,
            CompileOptions::default()
        ),
        Err(CompileError::KernelContract { .. })
    ));
    assert_eq!(resolver.draws.load(Ordering::Relaxed), 0);
}

#[test]
fn state_reads_become_host_inputs_and_function_calls_never_enter_generic_callbacks() {
    let (package, snapshot) = catalog();
    let base = graph(&package, vec![], vec![], vec![]);
    for function_call in [false, true] {
        let mut raw: Json = serde_json::from_str(&to_json(&base).unwrap()).unwrap();
        if function_call {
            raw["nodes"] = json!([{"id":id(1),"definition_id":"core/call","definition_version":1,"config_version":1,
                "config":{"function":id(10)},"inputs":{"x":literal(1)}}]);
            raw["functions"] = json!([{"id":id(10),"name":"identity","kind":"pure",
                "inputs":[{"port_id":"x","direction":"input","kind":"data","type":"core/u64@1"}],
                "outputs":[{"port_id":"y","direction":"output","kind":"data","type":"core/u64@1"}],
                "nodes":[{"id":id(11),"definition_id":"core/function-entry","definition_version":1,"config_version":1,"config":{}},
                    {"id":id(12),"definition_id":"core/function-outputs","definition_version":1,"config_version":1,"config":{}}],
                "edges":[edge(10,11,"x",12,"y")]}]);
        } else {
            raw["variables"] =
                json!([{"id":id(10),"name":"state","type":"core/u64@1","scope":"run"}]);
            raw["nodes"] = json!([{"id":id(1),"definition_id":"core/get-variable","definition_version":1,"config_version":1,"config":{"variable":id(10)}}]);
        }
        let doc = from_json(&raw.to_string()).unwrap();
        let analysis = analyze(&doc, &snapshot);
        assert!(
            analysis.is_ready_for_lowering(&doc, &snapshot),
            "{:?}",
            analysis.report().diagnostics
        );
        let resolver = Counting::new(package.clone());
        let data = compile_data(
            &doc,
            snapshot.clone(),
            &analysis,
            &resolver,
            CompileOptions::default(),
        );
        if function_call {
            assert!(matches!(data, Err(CompileError::UnsupportedNode { .. })));
        } else {
            // Admitted, but nothing exports it, so no input slot is lowered.
            assert!(data.unwrap().host_inputs().is_empty());
        }
        let pure = compile_pure(
            &doc,
            snapshot.clone(),
            &analysis,
            &resolver,
            CompileOptions::default(),
        );
        if function_call {
            assert!(pure.unwrap().exports().is_empty());
        } else {
            assert!(matches!(pure, Err(CompileError::UnsupportedNode { .. })));
        }
        assert_eq!(resolver.pure.load(Ordering::Relaxed), 0);
        assert_eq!(resolver.entropy.load(Ordering::Relaxed), 0);
    }
}
