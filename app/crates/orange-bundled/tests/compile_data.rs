use orange_bundled::{
    MathPackage, Operation, RANDOM_U64_DEFINITION, math_definitions, random_u64_definition,
};
use orange_compiler::{
    CompileError, CompileOptions, DataOperation, DataStability, EntropyBinding, EntropyKernel,
    EntropyKernelResolver, EntropySource, FoldedValue, KernelBinding, KernelFault, PureKernel,
    PureKernelResolver, ResolveError, compile_data, compile_pure,
};
use orange_model::catalog::{RegistryBuilder, RegistrySnapshot};
use orange_model::document::{NodeInstance, SignatureSnapshot};
use orange_model::validation::HostOrigin;
use orange_model::{GraphDocument, Value, analyze, from_json, to_json};
use serde_json::{Value as Json, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
fn id(n: u32) -> String {
    format!("10000000-0000-4000-8000-{n:012x}")
}
fn catalog() -> (MathPackage, Arc<RegistrySnapshot>) {
    let digest = format!("sha256:{}", "a".repeat(64));
    let package = MathPackage::new(digest.clone()).unwrap();
    let snapshot = package
        .register(RegistryBuilder::core(
            HostOrigin::new("0.0.0", digest).unwrap(),
        ))
        .unwrap()
        .build()
        .unwrap();
    (package, snapshot)
}
fn provider() -> Json {
    json!({"package":"org.orange.math","version":"0.0.0","id":"native-v1"})
}
fn literal(value: u64) -> Json {
    json!({"type":"core/u64@1","value":value.to_string()})
}
fn math(n: u32, op: &str, a: u64, b: u64) -> Json {
    json!({"id":id(n),"definition_id":format!("org.orange.math/{op}"),"definition_version":1,"config_version":1,
        "config":{"int_overflow":"wrapping"},"signature":op,"provider":provider(),"inputs":{"a":literal(a),"b":literal(b)}})
}
fn random(n: u32) -> Json {
    json!({"id":id(n),"definition_id":RANDOM_U64_DEFINITION,"definition_version":1,"config_version":1,
        "config":{},"signature":"random","provider":provider()})
}
fn edge(n: u32, source: u32, output: &str, target: u32, input: &str) -> Json {
    json!({"id":format!("30000000-0000-4000-8000-{n:012x}"),"from":{"node":id(source),"port":output},
        "to":{"node":id(target),"port":input}})
}
fn export(n: u32, port: &str) -> Json {
    json!({"id":format!("out-{n}"),"node":id(n),"port":port})
}
fn graph(
    package: &MathPackage,
    nodes: Vec<Json>,
    edges: Vec<Json>,
    exports: Vec<Json>,
) -> GraphDocument {
    let mut signatures = serde_json::Map::new();
    for (operation, definition) in Operation::ALL.into_iter().zip(math_definitions().unwrap()) {
        let orange_model::catalog::Ports::Declared(ports) = definition.ports else {
            unreachable!()
        };
        let args = if operation == Operation::Not {
            Default::default()
        } else {
            [("T".into(), "core/u64@1".parse().unwrap())].into()
        };
        signatures.insert(
            operation.definition_id().rsplit('/').next().unwrap().into(),
            serde_json::to_value(orange_model::signature::project(
                definition.kind,
                ports,
                &args,
            ))
            .unwrap(),
        );
    }
    let definition = random_u64_definition().unwrap();
    let orange_model::catalog::Ports::Declared(ports) = definition.ports else {
        unreachable!()
    };
    signatures.insert(
        "random".into(),
        serde_json::to_value(orange_model::signature::project(
            definition.kind,
            ports,
            &Default::default(),
        ))
        .unwrap(),
    );
    let key = package.provider_key();
    from_json(&json!({"format":"orange.graph","format_version":1,"graph_id":"20000000-0000-4000-8000-000000000001",
        "revision":0,"package_lock":[{"id":key.package,"version":key.version,"content_digest":key.content_digest}],
        "signatures":signatures,"nodes":nodes,"edges":edges,"exports":exports}).to_string()).unwrap()
}
struct Counting {
    package: MathPackage,
    pure: AtomicUsize,
    entropy: AtomicUsize,
    folds: Arc<AtomicUsize>,
    draws: Arc<AtomicUsize>,
    reject: Option<orange_model::id::NodeId>,
    bad_entropy_abi: bool,
}
impl Counting {
    fn new(package: MathPackage) -> Self {
        Self {
            package,
            pure: AtomicUsize::new(0),
            entropy: AtomicUsize::new(0),
            folds: Arc::new(AtomicUsize::new(0)),
            draws: Arc::new(AtomicUsize::new(0)),
            reject: None,
            bad_entropy_abi: false,
        }
    }
}
#[derive(Debug)]
struct CountPure {
    inner: Arc<dyn PureKernel>,
    calls: Arc<AtomicUsize>,
}
impl PureKernel for CountPure {
    fn fold(&self, inputs: &[Value], outputs: &mut [Value]) -> Result<(), KernelFault> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.inner.fold(inputs, outputs)
    }
}
#[derive(Debug)]
struct CountEntropy {
    inner: Arc<dyn EntropyKernel>,
    calls: Arc<AtomicUsize>,
}
impl EntropyKernel for CountEntropy {
    fn evaluate(
        &self,
        inputs: &[Value],
        outputs: &mut [Value],
        source: &mut dyn EntropySource,
    ) -> Result<(), KernelFault> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.inner.evaluate(inputs, outputs, source)
    }
}
impl PureKernelResolver for Counting {
    fn resolve(
        &self,
        doc: &GraphDocument,
        node: &NodeInstance,
        sig: &SignatureSnapshot,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Result<KernelBinding, ResolveError> {
        self.pure.fetch_add(1, Ordering::Relaxed);
        if self.reject == Some(node.id) {
            return Err(ResolveError::Unavailable);
        }
        let mut binding = PureKernelResolver::resolve(&self.package, doc, node, sig, snapshot)?;
        binding.kernel = Arc::new(CountPure {
            inner: binding.kernel,
            calls: self.folds.clone(),
        });
        Ok(binding)
    }
}
impl EntropyKernelResolver for Counting {
    fn resolve_entropy(
        &self,
        doc: &GraphDocument,
        node: &NodeInstance,
        sig: &SignatureSnapshot,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Result<EntropyBinding, ResolveError> {
        self.entropy.fetch_add(1, Ordering::Relaxed);
        if self.reject == Some(node.id) {
            return Err(ResolveError::Unavailable);
        }
        let mut binding = self.package.resolve_entropy(doc, node, sig, snapshot)?;
        if self.bad_entropy_abi {
            binding.outputs = Box::new([]);
        }
        binding.kernel = Arc::new(CountEntropy {
            inner: binding.kernel,
            calls: self.draws.clone(),
        });
        Ok(binding)
    }
}
fn compile(
    doc: &GraphDocument,
    snapshot: Arc<RegistrySnapshot>,
    resolver: &Counting,
) -> orange_compiler::DataPlan {
    let analysis = analyze(doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    compile_data(
        doc,
        snapshot,
        &analysis,
        resolver,
        CompileOptions::default(),
    )
    .unwrap()
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
fn state_reads_and_unimplemented_function_calls_never_enter_generic_callbacks() {
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
        assert!(matches!(
            compile_data(
                &doc,
                snapshot.clone(),
                &analysis,
                &resolver,
                CompileOptions::default()
            ),
            Err(CompileError::UnsupportedNode { .. })
        ));
        assert!(matches!(
            compile_pure(
                &doc,
                snapshot.clone(),
                &analysis,
                &resolver,
                CompileOptions::default()
            ),
            Err(CompileError::UnsupportedNode { .. })
        ));
        assert_eq!(resolver.pure.load(Ordering::Relaxed), 0);
        assert_eq!(resolver.entropy.load(Ordering::Relaxed), 0);
    }
}
