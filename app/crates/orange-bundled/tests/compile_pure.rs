use orange_bundled::{MathPackage, Operation, math_definitions};
use orange_compiler::{
    CompileError, CompileOptions, FoldedValue, KernelBinding, KernelFault, PureKernel,
    PureKernelResolver, ResolveError, compile_pure,
};
use orange_model::catalog::{RegistryBuilder, RegistrySnapshot};
use orange_model::document::{NodeInstance, NodeKind, SignatureSnapshot};
use orange_model::validation::{DiagnosticCode, HostOrigin, Severity};
use orange_model::{GraphDocument, Value, analyze, from_json, to_json};
use serde_json::{Value as Json, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[path = "support/reference.rs"]
mod reference;

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
fn node(n: u32, op: &str, a: i64, b: i64) -> Json {
    json!({"id":id(n),"definition_id":format!("org.orange.math/{op}"),"definition_version":1,
        "config_version":1,"config":{},"provider":{"package":"org.orange.math","version":"0.0.0","id":"native-v1"},
        "signature":op,"inputs":{"a":{"type":"core/i64@1","value":a.to_string()},
        "b":{"type":"core/i64@1","value":b.to_string()}}})
}
fn edge(n: u32, source: u32, output: &str, target: u32, input: &str) -> Json {
    json!({"id":format!("30000000-0000-4000-8000-{n:012x}"),
        "from":{"node":id(source),"port":output},"to":{"node":id(target),"port":input}})
}
fn graph(
    package: &MathPackage,
    nodes: Vec<Json>,
    edges: Vec<Json>,
    exports: Vec<Json>,
) -> GraphDocument {
    let mut signatures = serde_json::Map::new();
    for (op, def) in Operation::ALL.into_iter().zip(math_definitions().unwrap()) {
        let orange_model::catalog::Ports::Declared(ports) = def.ports else {
            panic!()
        };
        let args = if op == Operation::Not {
            Default::default()
        } else {
            [("T".into(), "core/i64@1".parse().unwrap())].into()
        };
        signatures.insert(
            op.definition_id().rsplit('/').next().unwrap().into(),
            serde_json::to_value(orange_model::signature::project(
                NodeKind::Pure,
                ports,
                &args,
            ))
            .unwrap(),
        );
    }
    let key = package.provider_key();
    from_json(&json!({"format":"orange.graph","format_version":1,"graph_id":"20000000-0000-4000-8000-000000000001",
        "revision":0,"package_lock":[{"id":key.package,"version":key.version,"content_digest":key.content_digest}],
        "nodes":nodes,"edges":edges,"signatures":signatures,"exports":exports}).to_string()).unwrap()
}
fn export(n: u32, port: &str) -> Json {
    json!({"id":format!("out-{n}"),"node":id(n),"port":port})
}
fn compile(
    doc: &GraphDocument,
    snapshot: Arc<RegistrySnapshot>,
    resolver: &impl PureKernelResolver,
) -> orange_compiler::PurePlan {
    let analysis = analyze(doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    compile_pure(
        doc,
        snapshot,
        &analysis,
        resolver,
        CompileOptions::default(),
    )
    .unwrap()
}

struct Counting {
    package: MathPackage,
    resolves: AtomicUsize,
    folds: Arc<AtomicUsize>,
}
impl Counting {
    fn new(package: MathPackage) -> Self {
        Self {
            package,
            resolves: AtomicUsize::new(0),
            folds: Arc::new(AtomicUsize::new(0)),
        }
    }
}
#[derive(Debug)]
struct CountKernel {
    inner: Arc<dyn PureKernel>,
    count: Arc<AtomicUsize>,
}
impl PureKernel for CountKernel {
    fn fold(&self, inputs: &[Value], outputs: &mut [Value]) -> Result<(), KernelFault> {
        self.count.fetch_add(1, Ordering::Relaxed);
        self.inner.fold(inputs, outputs)
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
        self.resolves.fetch_add(1, Ordering::Relaxed);
        let mut binding = PureKernelResolver::resolve(&self.package, doc, node, sig, snapshot)?;
        binding.kernel = Arc::new(CountKernel {
            inner: binding.kernel,
            count: self.folds.clone(),
        });
        Ok(binding)
    }
}

#[test]
fn diamond_folds_shared_producer_once_and_keeps_dense_source_maps() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![
            node(4, "add", 0, 0),
            node(3, "multiply", 0, 3),
            node(2, "multiply", 0, 2),
            node(1, "add", 3, 4),
            node(5, "divide", 1, 0),
        ],
        vec![
            edge(1, 1, "sum", 2, "a"),
            edge(2, 1, "sum", 3, "a"),
            edge(3, 2, "product", 4, "a"),
            edge(4, 3, "product", 4, "b"),
        ],
        vec![export(4, "sum")],
    );
    let resolver = Counting::new(package);
    let before = to_json(&doc).unwrap();
    let plan = compile(&doc, snapshot.clone(), &resolver);
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 4);
    assert_eq!(plan.steps().len(), 4);
    assert_eq!(
        plan.constants()[plan.exports()[0].slot.index()],
        FoldedValue::Value(Value::I64(35))
    );
    assert_eq!(plan.slot_types().len(), plan.constants().len());
    assert_eq!(plan.slot_sources().len(), plan.constants().len());
    assert_eq!(plan.steps()[0].node(), id(1).parse().unwrap());
    assert!(
        plan.steps()[3]
            .inputs()
            .iter()
            .all(|input| input.edge.is_some())
    );
    for step in plan.steps() {
        for slot in step.outputs() {
            assert_eq!(plan.slot_sources()[slot.index()].node, step.node());
        }
    }
    assert!(Arc::ptr_eq(plan.snapshot(), &snapshot));
    assert!(plan.is_current(&doc, &snapshot));
    assert_eq!(to_json(&doc).unwrap(), before);
    assert!(plan.diagnostics().is_empty()); // dead divide was not folded
}

#[test]
fn faults_remain_observable_with_declared_input_order_and_original_producer() {
    let (package, snapshot) = catalog();
    for reverse_storage in [false, true] {
        let mut nodes = vec![
            node(1, "divide", 1, 0),
            node(9, "divide", 2, 0),
            node(3, "add", 0, 0),
        ];
        if reverse_storage {
            nodes.reverse();
        }
        let doc = graph(
            &package,
            nodes,
            vec![
                edge(1, 9, "quotient", 3, "a"),
                edge(2, 1, "quotient", 3, "b"),
            ],
            vec![export(3, "sum")],
        );
        let resolver = Counting::new(package.clone());
        let plan = compile(&doc, snapshot.clone(), &resolver);
        assert_eq!(resolver.folds.load(Ordering::Relaxed), 2); // add never executes
        let FoldedValue::Fault(fault) = &plan.constants()[plan.exports()[0].slot.index()] else {
            panic!()
        };
        assert_eq!(fault.source, id(9).parse().unwrap()); // a, despite higher UUID
        assert_eq!(fault.fault.code, "core/DivideByZero");
        assert_eq!(plan.diagnostics().len(), 2);
        assert!(
            plan.diagnostics()
                .iter()
                .all(|d| d.code == DiagnosticCode::ConstError && d.severity == Severity::Warning)
        );
    }
}

#[test]
fn stale_inputs_and_quotas_cancel_before_executable_callbacks() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![node(1, "add", 20, 22)],
        vec![],
        vec![export(1, "sum")],
    );
    let analysis = analyze(&doc, &snapshot);
    let other_doc = graph(
        &package,
        vec![node(1, "add", 21, 22)],
        vec![],
        vec![export(1, "sum")],
    );
    let resolver = Counting::new(package);
    let (_, other_snapshot) = catalog();
    assert!(matches!(
        compile_pure(
            &other_doc,
            snapshot.clone(),
            &analysis,
            &resolver,
            CompileOptions::default()
        ),
        Err(CompileError::StaleDocument)
    ));
    assert!(matches!(
        compile_pure(
            &doc,
            other_snapshot,
            &analysis,
            &resolver,
            CompileOptions::default()
        ),
        Err(CompileError::StaleRegistry)
    ));
    let cancelled = AtomicBool::new(true);
    let mut options = CompileOptions {
        cancel: Some(&cancelled),
        ..Default::default()
    };
    assert!(matches!(
        compile_pure(&doc, snapshot.clone(), &analysis, &resolver, options),
        Err(CompileError::Cancelled)
    ));
    options.cancel = None;
    options.limits.max_graph_items = 0;
    assert!(matches!(
        compile_pure(&doc, snapshot.clone(), &analysis, &resolver, options),
        Err(CompileError::ResourceLimit)
    ));
    options = CompileOptions::default();
    options.limits.max_fold_operations = 0;
    assert!(matches!(
        compile_pure(&doc, snapshot.clone(), &analysis, &resolver, options),
        Err(CompileError::ResourceLimit)
    ));
    for (slots, bytes) in [(2, 1024), (100, 0)] {
        options = CompileOptions::default();
        options.limits.max_slots = slots;
        options.limits.max_constant_bytes = bytes;
        assert!(matches!(
            compile_pure(&doc, snapshot.clone(), &analysis, &resolver, options),
            Err(CompileError::ResourceLimit)
        ));
    }
    assert_eq!(resolver.resolves.load(Ordering::Relaxed), 0);
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 0);
}

#[derive(Debug)]
struct InvalidKernel {
    invalid_fault: bool,
}
impl PureKernel for InvalidKernel {
    fn fold(&self, _: &[Value], outputs: &mut [Value]) -> Result<(), KernelFault> {
        if self.invalid_fault {
            return Err(KernelFault {
                code: "/",
                message_key: "core/fault",
                message: "Safe fallback.",
            });
        }
        outputs.fill(Value::Bool(false));
        Ok(())
    }
}
struct Broken {
    package: MathPackage,
    mode: u8,
}
impl PureKernelResolver for Broken {
    fn resolve(
        &self,
        doc: &GraphDocument,
        node: &NodeInstance,
        sig: &SignatureSnapshot,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Result<KernelBinding, ResolveError> {
        if self.mode == 0 {
            return Err(ResolveError::Unavailable);
        }
        let mut binding = PureKernelResolver::resolve(&self.package, doc, node, sig, snapshot)?;
        if self.mode == 1 {
            binding.inputs.reverse();
        } else {
            binding.kernel = Arc::new(InvalidKernel {
                invalid_fault: self.mode == 3,
            });
        }
        Ok(binding)
    }
}

#[test]
fn unavailable_backends_abi_mismatch_and_wrong_output_types_publish_no_plan() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![node(1, "add", 20, 22)],
        vec![],
        vec![export(1, "sum")],
    );
    let analysis = analyze(&doc, &snapshot);
    for mode in 0..4 {
        let result = compile_pure(
            &doc,
            snapshot.clone(),
            &analysis,
            &Broken {
                package: package.clone(),
                mode,
            },
            CompileOptions::default(),
        );
        if mode == 0 {
            assert!(matches!(
                result,
                Err(CompileError::KernelUnavailable { .. })
            ));
        } else {
            assert!(matches!(result, Err(CompileError::KernelContract { .. })));
        }
    }
}

#[test]
fn semantic_errors_stop_compilation_and_undo_restores_only_matching_evidence() {
    let (package, snapshot) = catalog();
    let mut doc = graph(
        &package,
        vec![node(1, "add", 20, 22)],
        vec![],
        vec![export(1, "sum")],
    );
    let analysis = analyze(&doc, &snapshot);
    let port = orange_model::document::PortRef {
        node: id(1).parse().unwrap(),
        port: "a".parse().unwrap(),
    };
    let change = doc
        .apply([orange_model::DocCommand::SetLiteral {
            port,
            literal: Some(orange_model::document::literal::Literal::value(
                "core/i64@1".parse().unwrap(),
                json!("wrong"),
            )),
        }])
        .unwrap();
    let invalid = analyze(&doc, &snapshot);
    let resolver = Counting::new(package);
    assert!(matches!(
        compile_pure(
            &doc,
            snapshot.clone(),
            &invalid,
            &resolver,
            CompileOptions::default()
        ),
        Err(CompileError::ValidationIncomplete)
    ));
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
    let _ = doc.revert(change.inverse).unwrap();
    assert!(matches!(
        compile_pure(
            &doc,
            snapshot.clone(),
            &invalid,
            &resolver,
            CompileOptions::default()
        ),
        Err(CompileError::StaleDocument)
    ));
    assert_eq!(resolver.resolves.load(Ordering::Relaxed), 0);
    // Undo restores the original content identity, while its source revision advances.
    let plan = compile_pure(
        &doc,
        snapshot,
        &analysis,
        &resolver,
        CompileOptions::default(),
    )
    .unwrap();
    assert_eq!(plan.source_revision(), doc.revision());
}

#[test]
fn deep_chain_uses_iterative_lowering_with_once_only_sharing() {
    let (package, snapshot) = catalog();
    let nodes = (1..=512).map(|n| node(n, "add", 0, 1)).collect();
    let edges = (2..=512).map(|n| edge(n, n - 1, "sum", n, "a")).collect();
    let doc = graph(
        &package,
        nodes,
        edges,
        vec![export(512, "sum"), export(256, "sum")],
    );
    let resolver = Counting::new(package);
    let plan = compile(&doc, snapshot, &resolver);
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 512);
    assert_eq!(
        plan.constants()[plan.exports()[0].slot.index()],
        FoldedValue::Value(Value::I64(512))
    );
    assert_eq!(
        plan.constants()[plan.exports()[1].slot.index()],
        FoldedValue::Value(Value::I64(256))
    );
    assert!(
        plan.steps()
            .iter()
            .enumerate()
            .all(|(n, step)| step.index().index() == n)
    );
}

#[test]
fn fresh_descriptor_default_and_instance_override_precede_folding() {
    let (package, original) = catalog();
    let mut definitions = math_definitions().unwrap();
    let definition = definitions
        .iter_mut()
        .find(|d| d.definition.id.as_str() == Operation::Not.definition_id())
        .unwrap();
    let orange_model::catalog::Ports::Declared(ports) = &mut definition.ports else {
        panic!()
    };
    ports[0].default = Some(orange_model::document::literal::Literal::value(
        "core/bool@1".parse().unwrap(),
        json!(true),
    ));
    let signature =
        orange_model::signature::project(NodeKind::Pure, ports.clone(), &Default::default());
    let mut builder = RegistryBuilder::core(original.origin().clone());
    for definition in definitions {
        builder = builder.definition(definition);
    }
    let snapshot = builder
        .provider(original.provider(package.provider_key()).unwrap().clone())
        .build()
        .unwrap();
    for override_input in [false, true] {
        let mut raw: Json =
            serde_json::from_str(&to_json(&graph(&package, vec![], vec![], vec![])).unwrap())
                .unwrap();
        raw["nodes"] = json!([{"id":id(1),"definition_id":Operation::Not.definition_id(),"definition_version":1,
            "config_version":1,"config":{},"provider":{"package":"org.orange.math","version":"0.0.0","id":"native-v1"},
            "signature":"not","inputs":if override_input { json!({"value":{"type":"core/bool@1","value":false}}) } else { json!({}) }}]);
        raw["signatures"]["not"] = serde_json::to_value(&signature).unwrap();
        raw["exports"] = json!([export(1, "result")]);
        let doc = from_json(&raw.to_string()).unwrap();
        let analysis = analyze(&doc, &snapshot);
        let expected = reference::Reference::new(&doc, &snapshot, &analysis).exports();
        let plan = compile(&doc, snapshot.clone(), &package);
        assert_eq!(
            plan.constants()[plan.exports()[0].slot.index()],
            FoldedValue::Value(Value::Bool(override_input))
        );
        assert_eq!(expected, [Ok(Value::Bool(override_input))]);
    }
}

#[test]
fn unused_volatile_root_node_is_rejected_before_resolver_calls() {
    let (package, snapshot) = catalog();
    let mut raw: Json =
        serde_json::from_str(&to_json(&graph(&package, vec![], vec![], vec![])).unwrap()).unwrap();
    raw["variables"] = json!([{"id":id(10),"name":"state","type":"core/i64@1","scope":"run"}]);
    raw["nodes"] = json!([{"id":id(1),"definition_id":"core/get-variable","definition_version":1,
        "config_version":1,"config":{"variable":id(10)}}]);
    let doc = from_json(&raw.to_string()).unwrap();
    let analysis = analyze(&doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(&doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
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

#[test]
fn connected_edge_shadows_literal_and_cached_type_arguments_are_not_binding_evidence() {
    let (package, snapshot) = catalog();
    let mut consumer = node(2, "add", 999, 1);
    consumer["type_args"] = json!({"T":"core/f64@1"});
    let doc = graph(
        &package,
        vec![node(1, "add", 20, 21), consumer],
        vec![edge(1, 1, "sum", 2, "a")],
        vec![export(2, "sum")],
    );
    let plan = compile(&doc, snapshot, &package);
    assert_eq!(
        plan.constants()[plan.exports()[0].slot.index()],
        FoldedValue::Value(Value::I64(42))
    );
    assert!(
        !plan
            .constants()
            .contains(&FoldedValue::Value(Value::I64(999)))
    );
}

#[test]
fn explicit_core_converter_is_resolved_by_the_same_checked_boundary() {
    let (package, snapshot) = catalog();
    let node = json!({"id":id(1),"definition_id":"core/convert","definition_version":1,"config_version":1,
        "config":{"converter":"core/i32-to-i64","contract_version":1,"options":{}},
        "signature":"convert","inputs":{"value":{"type":"core/i32@1","value":42}}});
    let mut raw: Json =
        serde_json::from_str(&to_json(&graph(&package, vec![], vec![], vec![])).unwrap()).unwrap();
    raw["nodes"] = json!([node]);
    raw["exports"] = json!([export(1, "result")]);
    raw["signatures"]["convert"] = json!({"kind":"pure","ports":[
        {"port_id":"value","direction":"input","kind":"data","type":"core/i32@1"},
        {"port_id":"result","direction":"output","kind":"data","type":"core/i64@1","source":"computed"}]});
    let doc = from_json(&raw.to_string()).unwrap();
    let plan = compile(&doc, snapshot, &package);
    assert_eq!(
        plan.constants()[plan.exports()[0].slot.index()],
        FoldedValue::Value(Value::I64(42))
    );
}

#[test]
fn lossy_conversion_has_typed_json_output_and_checked_failure_keeps_provenance() {
    let (package, snapshot) = catalog();
    for lossy in [false, true] {
        let (converter, input_type, output_type, payload, options) = if lossy {
            (
                "core/f64-to-f32",
                "core/f64@1",
                "core/f32@1",
                json!({"type":"core/f64@1","bits":"3ff199999999999a"}),
                json!({"rounding":"nearest-even","overflow":"reject"}),
            )
        } else {
            (
                "core/i64-to-i32",
                "core/i64@1",
                "core/i32@1",
                json!({"type":"core/i64@1","value":"2147483648"}),
                json!({}),
            )
        };
        let mut raw: Json =
            serde_json::from_str(&to_json(&graph(&package, vec![], vec![], vec![])).unwrap())
                .unwrap();
        raw["nodes"] = json!([{"id":id(1),"definition_id":"core/convert","definition_version":1,"config_version":1,
            "signature":"convert","config":{"converter":converter,"contract_version":1,"options":options},"inputs":{"value":payload}}]);
        let mut ports = vec![
            json!({"port_id":"value","direction":"input","kind":"data","type":input_type}),
            json!({"port_id":"result","direction":"output","kind":"data","type":output_type,"source":"computed"}),
        ];
        if lossy {
            ports.push(json!({"port_id":"loss","direction":"output","kind":"data","type":"core/json@1","source":"computed"}));
        }
        raw["signatures"]["convert"] = json!({"kind":"pure","ports":ports});
        raw["exports"] = json!([export(1, "result")]);
        if lossy {
            raw["exports"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":"loss","node":id(1),"port":"loss"}));
        }
        let doc = from_json(&raw.to_string()).unwrap();
        let plan = compile(&doc, snapshot.clone(), &package);
        if lossy {
            assert_eq!(
                plan.constants()[plan.exports()[0].slot.index()],
                FoldedValue::Value(Value::F32(1.1))
            );
            let FoldedValue::Value(Value::Json(text)) =
                &plan.constants()[plan.exports()[1].slot.index()]
            else {
                panic!()
            };
            assert_eq!(
                serde_json::from_str::<Json>(text.as_str()).unwrap(),
                json!({"lost":true})
            );
        } else {
            let FoldedValue::Fault(fault) = &plan.constants()[plan.exports()[0].slot.index()]
            else {
                panic!()
            };
            assert_eq!(fault.source, id(1).parse().unwrap());
            assert_eq!(fault.fault.code, "core/OutOfRange");
            assert_eq!(plan.diagnostics().len(), 1);
        }
    }
}
