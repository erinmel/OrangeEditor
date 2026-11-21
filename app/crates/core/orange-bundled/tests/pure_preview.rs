//! Effect-free authoring publications through public compiler/model/runtime APIs.
use crate::data_graph::{catalog, edge, export, graph, id, literal, math, random};
use orange_bundled::MathPackage;
use orange_compiler::{
    BindingRequest, CompileError, CompileOptions, EvaluationError, EvaluationOptions, FoldedValue,
    KernelBinding, KernelFault, PreviewLimits, PreviewReset, PreviewSnapshot, PureKernel,
    PureKernelResolver, PurePreview, ResolveError, compile_pure,
};
use orange_model::catalog::{RegistryBuilder, RegistrySnapshot};
use orange_model::id::NodeId;
use orange_model::{GraphDocument, Value, analyze, from_json, to_json};
use serde_json::{Value as Json, json};
use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
struct Probe {
    calls: Mutex<BTreeMap<NodeId, usize>>,
    panic: AtomicBool,
    bad: AtomicBool,
}
impl Probe {
    fn count(&self, n: u32) -> usize {
        *self
            .calls
            .lock()
            .unwrap()
            .get(&id(n).parse().unwrap())
            .unwrap_or(&0)
    }
}
struct Resolver {
    package: MathPackage,
    probe: Arc<Probe>,
}
struct Count {
    inner: Arc<dyn PureKernel>,
    node: NodeId,
    probe: Arc<Probe>,
}
impl std::fmt::Debug for Count {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Count(..)")
    }
}
impl PureKernel for Count {
    fn fold(&self, inputs: &[Value], outputs: &mut [Value]) -> Result<(), KernelFault> {
        *self
            .probe
            .calls
            .lock()
            .unwrap()
            .entry(self.node)
            .or_default() += 1;
        assert!(!self.probe.panic.load(Ordering::Relaxed), "provider panic");
        if self.probe.bad.load(Ordering::Relaxed) {
            outputs[0] = Value::Bool(true);
            return Ok(());
        }
        self.inner.fold(inputs, outputs)
    }
}
impl PureKernelResolver for Resolver {
    fn resolve_binding(&self, r: &BindingRequest<'_>) -> Result<KernelBinding, ResolveError> {
        let mut binding = self.package.resolve_binding(r)?;
        binding.kernel = Arc::new(Count {
            inner: binding.kernel,
            node: r.node(),
            probe: self.probe.clone(),
        });
        Ok(binding)
    }
}
fn preview(package: MathPackage, limits: PreviewLimits) -> (PurePreview<Resolver>, Arc<Probe>) {
    let probe = Arc::new(Probe::default());
    (
        PurePreview::new(
            Resolver {
                package,
                probe: probe.clone(),
            },
            limits,
        ),
        probe,
    )
}
fn change(doc: &GraphDocument, edit: impl FnOnce(&mut Json)) -> GraphDocument {
    let mut wire: Json = serde_json::from_str(&to_json(doc).unwrap()).unwrap();
    edit(&mut wire);
    from_json(&wire.to_string()).unwrap()
}
fn update(
    session: &mut PurePreview<Resolver>,
    doc: &GraphDocument,
    snapshot: &Arc<RegistrySnapshot>,
) -> Arc<PreviewSnapshot> {
    let analysis = analyze(doc, snapshot);
    assert!(
        analysis.is_ready_for_lowering(doc, snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    session
        .update(doc, snapshot.clone(), &analysis, CompileOptions::default())
        .unwrap()
}
fn values(
    snapshot: &PreviewSnapshot,
    doc: &GraphDocument,
    registry: &RegistrySnapshot,
) -> Vec<FoldedValue> {
    snapshot
        .evaluate(doc, registry, EvaluationOptions::default())
        .unwrap()
        .exports()
        .map(|e| match e.unwrap().outcome {
            Ok(v) => FoldedValue::Value(v.clone()),
            Err(f) => FoldedValue::Fault(f.clone()),
        })
        .collect()
}
fn oracle(
    doc: &GraphDocument,
    snapshot: &Arc<RegistrySnapshot>,
    package: &MathPackage,
) -> Vec<FoldedValue> {
    let plan = compile_pure(
        doc,
        snapshot.clone(),
        &analyze(doc, snapshot),
        package,
        CompileOptions::default(),
    )
    .unwrap();
    plan.exports()
        .iter()
        .map(|e| plan.constants()[e.slot.index()].clone())
        .collect()
}
fn cones(package: &MathPackage) -> GraphDocument {
    graph(
        package,
        vec![
            math(1, "add", 1, 2),
            math(2, "multiply", 0, 5),
            math(3, "add", 20, 30),
        ],
        vec![edge(1, 1, "sum", 2, "a")],
        vec![export(2, "product"), export(3, "sum")],
    )
}
#[test]
fn literal_edits_recompute_the_affected_cone_and_match_full_compilation() {
    let (package, snapshot) = catalog();
    let doc = cones(&package);
    let (mut session, probe) = preview(package.clone(), PreviewLimits::default());
    let first = update(&mut session, &doc, &snapshot);
    assert_eq!(
        values(&first, &doc, &snapshot),
        vec![
            FoldedValue::Value(Value::U64(15)),
            FoldedValue::Value(Value::U64(50))
        ]
    );
    assert_eq!(session.stats().folded, 3);
    assert_eq!(session.stats().reset, PreviewReset::First);
    let changed = change(&doc, |w| w["nodes"][0]["inputs"]["a"] = literal(8));
    let next = update(&mut session, &changed, &snapshot);
    assert_eq!(
        values(&next, &changed, &snapshot),
        oracle(&changed, &snapshot, &package)
    );
    assert_eq!(session.stats().folded, 2);
    assert_eq!(session.stats().reused, 1);
    assert_eq!((probe.count(1), probe.count(2), probe.count(3)), (2, 2, 1));
    assert!(!first.is_current(&changed, &snapshot));
    assert!(first.is_current(&doc, &snapshot));
    let again = update(&mut session, &doc, &snapshot);
    assert_eq!(
        values(&again, &doc, &snapshot),
        oracle(&doc, &snapshot, &package)
    );
    assert_eq!((probe.count(1), probe.count(2), probe.count(3)), (3, 3, 1));
}
#[test]
fn layout_only_updates_reuse_all_pure_values_without_changing_semantics() {
    let (package, snapshot) = catalog();
    let doc = cones(&package);
    let (mut session, probe) = preview(package.clone(), PreviewLimits::default());
    let first = update(&mut session, &doc, &snapshot);
    let mut changed = doc.clone();
    let _layout = changed
        .apply([orange_model::DocCommand::Layout(
            orange_model::LayoutCommand::Set {
                path: vec!["nodes".into(), id(1)],
                value: Some(json!({"x":100.0,"y":200.0})),
            },
        )])
        .unwrap();
    assert!(first.is_current(&changed, &snapshot));
    let next = update(&mut session, &changed, &snapshot);
    assert_eq!(
        values(&next, &changed, &snapshot),
        values(&first, &doc, &snapshot)
    );
    assert_eq!(session.stats().folded, 0);
    assert_eq!(session.stats().reused, 3);
    assert_eq!(probe.count(1), 1);
}
#[test]
fn configuration_and_type_changes_do_not_reuse_a_different_kernel_contract() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![math(1, "add", u64::MAX, 1)],
        vec![],
        vec![export(1, "sum")],
    );
    let (mut session, probe) = preview(package.clone(), PreviewLimits::default());
    update(&mut session, &doc, &snapshot);
    let changed = change(&doc, |w| {
        w["nodes"][0]["config"]["int_overflow"] = json!("saturating")
    });
    let next = update(&mut session, &changed, &snapshot);
    assert_eq!(
        values(&next, &changed, &snapshot),
        vec![FoldedValue::Value(Value::U64(u64::MAX))]
    );
    assert_eq!(probe.count(1), 2);
    let typed = change(&changed, |w| {
        w["nodes"][0]["type_pins"] = json!({"T":"core/u32@1"});
        w["nodes"][0]["type_args"] = json!({"T":"core/u32@1"});
        for port in w["signatures"]["add"]["ports"].as_array_mut().unwrap() {
            port["type"] = json!("core/u32@1");
        }
        w["nodes"][0]["inputs"] =
            json!({"a":{"type":"core/u32@1","value":10},"b":{"type":"core/u32@1","value":2}});
    });
    let next = update(&mut session, &typed, &snapshot);
    assert_eq!(
        values(&next, &typed, &snapshot),
        oracle(&typed, &snapshot, &package)
    );
    assert_eq!(probe.count(1), 3);
}
#[test]
fn rewire_to_an_equal_value_can_reuse_the_consumer_and_removal_prunes_unused_entries() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![
            math(1, "add", 1, 2),
            math(2, "add", 1, 2),
            math(3, "multiply", 0, 5),
        ],
        vec![edge(1, 1, "sum", 3, "a")],
        vec![export(3, "product"), export(2, "sum")],
    );
    let (mut session, probe) = preview(package.clone(), PreviewLimits::default());
    update(&mut session, &doc, &snapshot);
    let changed = change(&doc, |w| {
        w["edges"][0]["from"]["node"] = json!(id(2));
        w["exports"] = json!([export(3, "product")]);
    });
    let next = update(&mut session, &changed, &snapshot);
    assert_eq!(
        values(&next, &changed, &snapshot),
        oracle(&changed, &snapshot, &package)
    );
    assert_eq!(session.stats().folded, 0);
    assert_eq!(session.stats().entries, 2);
    assert_eq!(probe.count(3), 1);
    let empty = change(&changed, |w| w["exports"] = json!([]));
    update(&mut session, &empty, &snapshot);
    assert_eq!(session.stats().entries, 0);
}
#[test]
fn lazy_unselected_faults_stay_skipped_and_cached_faults_keep_producer_identity() {
    let (package, snapshot) = catalog();
    let select = json!({"id":id(3),"definition_id":"core/select","definition_version":1,"config_version":1,"config":{},"type_pins":{"T":"core/u64@1"},"inputs":{"condition":{"type":"core/bool@1","value":true},"if_true":literal(7)}});
    let doc = graph(
        &package,
        vec![math(1, "divide", 9, 0), select],
        vec![edge(1, 1, "quotient", 3, "if_false")],
        vec![export(3, "result")],
    );
    let (mut session, probe) = preview(package.clone(), PreviewLimits::default());
    let first = update(&mut session, &doc, &snapshot);
    assert_eq!(probe.count(1), 0);
    assert_eq!(
        values(&first, &doc, &snapshot),
        vec![FoldedValue::Value(Value::U64(7))]
    );
    let changed = change(&doc, |w| {
        w["nodes"][1]["inputs"]["condition"]["value"] = json!(false)
    });
    let first = update(&mut session, &changed, &snapshot);
    let expected = oracle(&changed, &snapshot, &package);
    assert_eq!(values(&first, &changed, &snapshot), expected);
    assert_eq!(probe.count(1), 1);
    let second = update(&mut session, &changed, &snapshot);
    assert_eq!(values(&second, &changed, &snapshot), expected);
    assert_eq!(first.diagnostics(), second.diagnostics());
    assert_eq!(probe.count(1), 1);
    assert!(matches!(&expected[0],FoldedValue::Fault(f) if f.source==id(1).parse().unwrap()));
}
#[test]
fn unselected_volatile_dependency_is_refused_before_any_kernel_runs() {
    let (package, snapshot) = catalog();
    let select = json!({"id":id(3),"definition_id":"core/select","definition_version":1,"config_version":1,"config":{},"type_pins":{"T":"core/u64@1"},"inputs":{"condition":{"type":"core/bool@1","value":true},"if_true":literal(7)}});
    let doc = graph(
        &package,
        vec![math(1, "add", 1, 2), random(2), select],
        vec![edge(1, 2, "value", 3, "if_false")],
        vec![export(1, "sum"), export(3, "result")],
    );
    let (mut session, probe) = preview(package, PreviewLimits::default());
    let error = session
        .update(
            &doc,
            snapshot.clone(),
            &analyze(&doc, &snapshot),
            CompileOptions::default(),
        )
        .unwrap_err();
    assert!(matches!(error, CompileError::UnsupportedNode { .. }));
    assert_eq!(probe.count(1), 0);
    assert!(session.snapshot().is_none());
}
#[test]
fn stale_cancelled_and_over_quota_requests_publish_nothing_and_do_not_bypass_admission() {
    let (package, snapshot) = catalog();
    let doc = cones(&package);
    let analysis = analyze(&doc, &snapshot);
    let (mut session, probe) = preview(package.clone(), PreviewLimits::default());
    let first = update(&mut session, &doc, &snapshot);
    let changed = change(&doc, |w| w["nodes"][0]["inputs"]["a"] = literal(9));
    assert_eq!(
        session
            .update(
                &changed,
                snapshot.clone(),
                &analysis,
                CompileOptions::default()
            )
            .unwrap_err(),
        CompileError::StaleDocument
    );
    assert!(session.snapshot().is_none());
    assert_eq!(probe.count(1), 1);
    assert!(first.is_current(&doc, &snapshot));
    let cancel = AtomicBool::new(true);
    let options = CompileOptions {
        cancel: Some(&cancel),
        ..Default::default()
    };
    assert_eq!(
        session
            .update(&doc, snapshot.clone(), &analysis, options)
            .unwrap_err(),
        CompileError::Cancelled
    );
    assert_eq!(session.stats().folded, 0);
    update(&mut session, &doc, &snapshot);
    let mut options = CompileOptions::default();
    options.limits.max_fold_operations = 0;
    assert_eq!(
        session
            .update(&doc, snapshot.clone(), &analysis, options)
            .unwrap_err(),
        CompileError::ResourceLimit
    );
    assert_eq!(session.stats().reused, 0);
    assert!(matches!(
        first.evaluate(&changed, &snapshot, EvaluationOptions::default()),
        Err(EvaluationError::StaleDocument)
    ));
}
#[test]
fn disabled_and_small_caches_change_work_without_changing_outputs() {
    let (package, snapshot) = catalog();
    let doc = cones(&package);
    for limits in [
        PreviewLimits {
            max_entries: 0,
            max_bytes: 0,
        },
        PreviewLimits {
            max_entries: 1,
            max_bytes: 4096,
        },
        PreviewLimits {
            max_entries: 100,
            max_bytes: 1,
        },
    ] {
        let (mut session, _) = preview(package.clone(), limits);
        for _ in 0..3 {
            let result = update(&mut session, &doc, &snapshot);
            assert_eq!(
                values(&result, &doc, &snapshot),
                oracle(&doc, &snapshot, &package)
            );
            assert!(session.stats().entries <= limits.max_entries);
            assert!(session.stats().retained_bytes <= limits.max_bytes);
        }
    }
}
#[test]
fn invalid_output_and_panic_never_poison_later_publications() {
    let (package, snapshot) = catalog();
    let doc = cones(&package);
    let (mut session, probe) = preview(package.clone(), PreviewLimits::default());
    probe.bad.store(true, Ordering::Relaxed);
    assert!(matches!(
        session.update(
            &doc,
            snapshot.clone(),
            &analyze(&doc, &snapshot),
            CompileOptions::default()
        ),
        Err(CompileError::KernelContract { .. })
    ));
    assert_eq!(session.stats().entries, 0);
    probe.bad.store(false, Ordering::Relaxed);
    probe.panic.store(true, Ordering::Relaxed);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| session.update(
            &doc,
            snapshot.clone(),
            &analyze(&doc, &snapshot),
            CompileOptions::default()
        )))
        .is_err()
    );
    assert!(session.snapshot().is_none());
    probe.panic.store(false, Ordering::Relaxed);
    let next = update(&mut session, &doc, &snapshot);
    assert_eq!(session.stats().reset, PreviewReset::Interrupted);
    assert_eq!(
        values(&next, &doc, &snapshot),
        oracle(&doc, &snapshot, &package)
    );
}
#[test]
fn registry_and_graph_identity_changes_reset_cache_without_retargeting_old_snapshots() {
    let (package, snapshot) = catalog();
    let doc = cones(&package);
    let (mut session, probe) = preview(package.clone(), PreviewLimits::default());
    let old = update(&mut session, &doc, &snapshot);
    let registry = RegistryBuilder::from_snapshot(&snapshot).build().unwrap();
    update(&mut session, &doc, &registry);
    assert_eq!(session.stats().reset, PreviewReset::RegistryChanged);
    assert_eq!(probe.count(1), 2);
    assert!(old.is_current(&doc, &snapshot));
    assert!(!old.is_current(&doc, &registry));
    let changed = change(&doc, |w| {
        w["graph_id"] = json!("20000000-0000-4000-8000-000000000002")
    });
    update(&mut session, &changed, &registry);
    assert_eq!(session.stats().reset, PreviewReset::GraphChanged);
    assert_eq!(probe.count(1), 3);
}

fn intrinsic(n: u32, name: &str) -> Json {
    json!({"id":id(n),"definition_id":format!("core/{name}"),"definition_version":1,"config_version":1,"config":{}})
}
fn function_graph(package: &MathPackage) -> GraphDocument {
    let call = |n, a, b| {
        let mut c = intrinsic(n, "call");
        c["config"] = json!({"function":id(100)});
        c["type_pins"] = json!({"T":"core/u64@1"});
        c["inputs"] = json!({"a":literal(a),"b":literal(b)});
        c
    };
    let mut add = math(103, "add", 0, 0);
    add["inputs"] = json!({});
    add["signature"] = json!("add-generic");
    let doc = graph(
        package,
        vec![call(1, 1, 2), call(2, 10, 20)],
        vec![],
        vec![export(1, "result"), export(2, "result")],
    );
    change(&doc, |w| {
        let mut generic = w["signatures"]["add"].clone();
        for p in generic["ports"].as_array_mut().unwrap() {
            p["type"] = json!("$T");
        }
        w["signatures"]["add-generic"] = generic;
        w["functions"] = json!([{"id":id(100),"name":"add","kind":"pure","type_params":[{"name":"T","constraints":["num"]}],
        "inputs":[{"port_id":"a","direction":"input","kind":"data","type":"$T"},{"port_id":"b","direction":"input","kind":"data","type":"$T"}],
        "outputs":[{"port_id":"result","direction":"output","kind":"data","type":"$T","source":"computed"}],
        "nodes":[intrinsic(101,"function-entry"),intrinsic(102,"function-outputs"),add],
        "edges":[edge(101,101,"a",103,"a"),edge(102,101,"b",103,"b"),edge(103,103,"sum",102,"result")]}]);
    })
}
#[test]
fn function_instances_keep_distinct_inputs_and_report_changed_call_layout() {
    let (package, snapshot) = catalog();
    let doc = function_graph(&package);
    let (mut session, probe) = preview(package.clone(), PreviewLimits::default());
    let first = update(&mut session, &doc, &snapshot);
    assert_eq!(
        values(&first, &doc, &snapshot),
        vec![
            FoldedValue::Value(Value::U64(3)),
            FoldedValue::Value(Value::U64(30))
        ]
    );
    assert_eq!(probe.count(103), 2);
    let changed = change(&doc, |w| w["nodes"][0]["inputs"]["a"] = literal(4));
    let next = update(&mut session, &changed, &snapshot);
    assert_eq!(
        values(&next, &changed, &snapshot),
        oracle(&changed, &snapshot, &package)
    );
    assert_eq!(session.stats().folded, 1);
    assert_eq!(session.stats().reused, 1);
    assert_eq!(probe.count(103), 3);
    let body = change(&changed, |w| {
        w["functions"][0]["nodes"][2]["config"]["int_overflow"] = json!("checked")
    });
    let next = update(&mut session, &body, &snapshot);
    assert_eq!(
        values(&next, &body, &snapshot),
        oracle(&body, &snapshot, &package)
    );
    assert_eq!(session.stats().folded, 2);
    let smaller = change(&body, |w| w["exports"] = json!([export(2, "result")]));
    let next = update(&mut session, &smaller, &snapshot);
    assert_eq!(
        values(&next, &smaller, &snapshot),
        oracle(&smaller, &snapshot, &package)
    );
    assert_eq!(session.stats().reset, PreviewReset::CallLayoutChanged);
    assert_eq!(session.stats().entries, 1);
}
#[test]
fn unrelated_effects_remain_inert_while_pure_exports_are_previewed() {
    let (package, _) = catalog();
    let (doc, snapshot, run) = crate::effects::setup(true, orange_runtime::RunOptions::default());
    let doc = change(&doc, |w| {
        w["nodes"]
            .as_array_mut()
            .unwrap()
            .push(math(20, "add", 1, 2));
        w["exports"] = json!([export(20, "sum")]);
    });
    let (mut session, probe) = preview(package, PreviewLimits::default());
    let result = update(&mut session, &doc, &snapshot);
    assert_eq!(
        values(&result, &doc, &snapshot),
        vec![FoldedValue::Value(Value::U64(3))]
    );
    assert_eq!(probe.count(20), 1);
    assert_eq!(run.variable(id(100).parse().unwrap()), None);
}
#[test]
fn authoring_publications_do_not_change_a_suspended_runs_plan_or_provider_pin() {
    let (package, snapshot) = catalog();
    let doc = change(&cones(&package), |w| {
        let mut start = intrinsic(50, "on-start");
        start["activation"] =
            json!({"mode":"queue","capacity":4,"overflow":"fault","on_fault":"continue","order":0});
        let mut wait = intrinsic(51, "wait");
        wait["inputs"] = json!({"duration":{"type":"core/duration@1","value":"10"}});
        let mut set = intrinsic(52, "set-variable");
        set["config"] = json!({"variable":id(100)});
        w["nodes"]
            .as_array_mut()
            .unwrap()
            .extend([start, wait, set]);
        w["edges"].as_array_mut().unwrap().extend([
            edge(50, 50, "then", 51, "in"),
            edge(51, 51, "then", 52, "in"),
            edge(52, 2, "product", 52, "value"),
        ]);
        w["variables"] = json!([{"id":id(100),"name":"result","type":"core/u64@1","scope":"run"}]);
    });
    let plan = orange_compiler::compile_control(
        &doc,
        snapshot.clone(),
        &analyze(&doc, &snapshot),
        &package,
        CompileOptions::default(),
    )
    .unwrap();
    let mut run = orange_runtime::Run::start(
        Arc::new(plan),
        &doc,
        &snapshot,
        0,
        orange_runtime::RunOptions::default(),
    )
    .unwrap();
    for _ in 0..100 {
        if !run.pump(1).unwrap().ready {
            break;
        }
    }
    assert_eq!(run.variable(id(100).parse().unwrap()), None);
    let (mut session, _) = preview(package, PreviewLimits::default());
    let old = update(&mut session, &doc, &snapshot);
    let changed = change(&doc, |w| w["nodes"][0]["inputs"]["a"] = literal(8));
    let registry = RegistryBuilder::from_snapshot(&snapshot).build().unwrap();
    let new = update(&mut session, &changed, &registry);
    assert_eq!(
        values(&new, &changed, &registry)[0],
        FoldedValue::Value(Value::U64(50))
    );
    assert_eq!(
        values(&old, &doc, &snapshot)[0],
        FoldedValue::Value(Value::U64(15))
    );
    run.advance_to(10).unwrap();
    for _ in 0..100 {
        if !run.pump(1).unwrap().ready {
            break;
        }
    }
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&Value::U64(15))
    );
    run.stop().unwrap();
    run.dispose().unwrap();
}
#[test]
fn preview_debug_does_not_format_cached_strings_or_resolvers() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<PurePreview<Resolver>>();
    send_sync::<PreviewSnapshot>();
    let (package, snapshot) = catalog();
    let secret = "secret-never-in-preview-debug";
    let select = json!({"id":id(1),"definition_id":"core/select","definition_version":1,"config_version":1,"config":{},"type_pins":{"T":"core/string@1"},"inputs":{"condition":{"type":"core/bool@1","value":true},"if_true":{"type":"core/string@1","value":secret},"if_false":{"type":"core/string@1","value":"other"}}});
    let doc = graph(&package, vec![select], vec![], vec![export(1, "result")]);
    let (mut session, _) = preview(package, PreviewLimits::default());
    let result = update(&mut session, &doc, &snapshot);
    assert!(!format!("{session:?}").contains(secret));
    assert!(!format!("{result:?}").contains(secret));
    assert_eq!(
        values(&result, &doc, &snapshot),
        vec![FoldedValue::Value(Value::Str(Arc::new(secret.into())))]
    );
}
#[test]
fn bounded_edit_sequence_matches_fresh_compilation_and_keeps_an_unrelated_cone_warm() {
    let (package, snapshot) = catalog();
    let base = cones(&package);
    let (mut session, probe) = preview(package.clone(), PreviewLimits::default());
    for n in 0..32 {
        let a = (n * 17) % 13;
        let doc = change(&base, |w| w["nodes"][0]["inputs"]["a"] = literal(a));
        let result = update(&mut session, &doc, &snapshot);
        assert_eq!(
            values(&result, &doc, &snapshot),
            oracle(&doc, &snapshot, &package)
        );
        assert_eq!(
            values(&result, &doc, &snapshot)[0],
            FoldedValue::Value(Value::U64((a + 2) * 5))
        );
    }
    assert_eq!(probe.count(3), 1);
}

#[test]
fn cached_values_still_obey_the_current_compiler_publication_budget() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![math(1, "add", 1, 2)],
        vec![],
        vec![export(1, "sum")],
    );
    let (mut session, probe) = preview(package.clone(), PreviewLimits::default());
    update(&mut session, &doc, &snapshot);
    let mut options = CompileOptions::default();
    options.limits.max_constant_bytes = 48;
    let analysis = analyze(&doc, &snapshot);
    assert_eq!(
        session
            .update(&doc, snapshot.clone(), &analysis, options)
            .unwrap_err(),
        CompileError::ResourceLimit
    );
    assert_eq!(session.stats().reused, 1);
    assert_eq!(session.stats().entries, 0);
    assert!(session.snapshot().is_none());
    assert_eq!(probe.count(1), 1);
    assert_eq!(
        compile_pure(&doc, snapshot, &analysis, &package, options).unwrap_err(),
        CompileError::ResourceLimit
    );
}
#[test]
fn committed_edits_undo_and_redo_compose_with_retained_analysis() {
    let (package, snapshot) = catalog();
    let mut doc = cones(&package);
    let (mut preview, probe) = preview(package.clone(), PreviewLimits::default());
    let mut analysis = orange_model::validation::AnalysisSession::new(
        &doc,
        &snapshot,
        BTreeMap::new(),
        Default::default(),
    );
    let original = preview
        .update(
            &doc,
            snapshot.clone(),
            &analysis.materialize(),
            CompileOptions::default(),
        )
        .unwrap();
    let committed = doc
        .apply([orange_model::DocCommand::SetLiteral {
            port: serde_json::from_value(json!({"node":id(1),"port":"a"})).unwrap(),
            literal: Some(serde_json::from_value(literal(8)).unwrap()),
        }])
        .unwrap();
    analysis.update(&doc, &snapshot, &committed.change_set, Default::default());
    let changed = preview
        .update(
            &doc,
            snapshot.clone(),
            &analysis.materialize(),
            CompileOptions::default(),
        )
        .unwrap();
    assert_eq!(
        values(&changed, &doc, &snapshot),
        oracle(&doc, &snapshot, &package)
    );
    let undone = doc.revert(committed.inverse).unwrap();
    analysis.update(&doc, &snapshot, &undone.change_set, Default::default());
    let again = preview
        .update(
            &doc,
            snapshot.clone(),
            &analysis.materialize(),
            CompileOptions::default(),
        )
        .unwrap();
    assert!(original.is_current(&doc, &snapshot));
    assert_eq!(
        values(&again, &doc, &snapshot),
        values(&original, &doc, &snapshot)
    );
    let redo = doc.revert(undone.inverse).unwrap();
    analysis.update(&doc, &snapshot, &redo.change_set, Default::default());
    let again = preview
        .update(
            &doc,
            snapshot.clone(),
            &analysis.materialize(),
            CompileOptions::default(),
        )
        .unwrap();
    assert_eq!(
        values(&again, &doc, &snapshot),
        oracle(&doc, &snapshot, &package)
    );
    assert_eq!((probe.count(1), probe.count(2), probe.count(3)), (4, 4, 1));
}
