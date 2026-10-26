use orange_bundled::{MathPackage, Operation, math_definitions};
use orange_compiler::{
    CompileError, CompileOptions, FoldedValue, KernelBinding, KernelFault, PureKernel,
    PureKernelResolver, ResolveError, compile_pure,
};
use orange_compiler::{EvaluationError, EvaluationOptions, PlanType};
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

#[path = "support/pure_functions.rs"]
mod pure_functions;

fn select_node(n: u32, condition: bool, ty: &str, if_true: Json, if_false: Json) -> Json {
    json!({"id":id(n),"definition_id":"core/select","definition_version":1,"config_version":1,
        "config":{},"type_pins":{"T":ty},"inputs":{"condition":{"type":"core/bool@1","value":condition},
            "if_true":if_true,"if_false":if_false}})
}
fn integer_literal(value: i64) -> Json {
    json!({"type":"core/i64@1","value":value.to_string()})
}

fn typed_literal(snapshot: &RegistrySnapshot, ty: &str, value: &Value) -> Json {
    serde_json::to_value(
        orange_model::value::literal_codec::encode(
            &ty.parse().unwrap(),
            value,
            snapshot.types().as_ref(),
        )
        .unwrap(),
    )
    .unwrap()
}
fn coalesce_node(n: u32, ty: &str, value: Json, fallback: Json) -> Json {
    json!({"id":id(n),"definition_id":"core/coalesce","definition_version":1,"config_version":1,
        "config":{},"type_pins":{"T":ty},"inputs":{"value":value,"fallback":fallback}})
}

fn try_node(n: u32, ty: &str, value: Option<Json>) -> Json {
    json!({"id":id(n),"definition_id":"core/try","definition_version":1,"config_version":1,
        "config":{},"type_pins":{"T":ty},"inputs":value.map(|value|json!({"value":value})).unwrap_or(json!({}))})
}
fn captured_error(code: &str, source: u32) -> Value {
    Value::Err(Arc::new(orange_model::value::NodeError {
        code: code.into(),
        node: Some(id(source).parse().unwrap()),
    }))
}

#[test]
fn try_captures_graph_faults_without_changing_separately_exported_faults() {
    let (package, snapshot) = catalog();
    for fails in [false, true] {
        let doc = graph(
            &package,
            vec![
                node(1, "divide", 42, if fails { 0 } else { 2 }),
                try_node(2, "core/i64@1", None),
                try_node(3, "core/result@1<core/i64@1>", None),
            ],
            vec![
                edge(1, 1, "quotient", 2, "value"),
                edge(2, 2, "result", 3, "value"),
            ],
            vec![
                export(3, "result"),
                export(2, "result"),
                export(1, "quotient"),
            ],
        );
        let resolver = Counting::new(package.clone());
        let plan = compile(&doc, snapshot.clone(), &resolver);
        assert_eq!(resolver.folds.load(Ordering::Relaxed), 1);
        assert_eq!(plan.diagnostics().len(), usize::from(fails));
        let inner = if fails {
            captured_error("core/DivideByZero", 1)
        } else {
            Value::I64(21)
        };
        assert_eq!(
            plan.constants()[plan.exports()[0].slot.index()],
            FoldedValue::Value(Value::Ok(Arc::new(inner.clone())))
        );
        assert_eq!(
            plan.constants()[plan.exports()[1].slot.index()],
            FoldedValue::Value(inner)
        );
        if fails {
            let FoldedValue::Fault(fault) = &plan.constants()[plan.exports()[2].slot.index()]
            else {
                panic!("direct fault was removed")
            };
            assert_eq!(fault.source, id(1).parse().unwrap());
        }
        let analysis = analyze(&doc, &snapshot);
        let expected = reference::Reference::new(&doc, &snapshot, &analysis).exports();
        for (actual, expected) in plan
            .evaluate(&doc, &snapshot, EvaluationOptions::default())
            .unwrap()
            .exports()
            .zip(expected)
        {
            match (actual.unwrap().outcome, expected) {
                (Ok(actual), Ok(expected)) => assert_eq!(actual, &expected),
                (Err(actual), Err(expected)) => {
                    assert_eq!(actual.source, expected.source);
                    assert_eq!(actual.fault.code, expected.code);
                }
                _ => panic!("try oracle mismatch"),
            }
        }
    }
}

#[test]
fn try_preserves_successful_none_and_result_option_composition() {
    let (package, snapshot) = catalog();
    for inner in [
        "core/i64@1",
        "core/result@1<core/i64@1>",
        "core/option@1<core/result@1<core/i64@1>>",
    ] {
        let option = format!("core/option@1<{inner}>");
        let none = typed_literal(&snapshot, &option, &Value::None);
        let doc = graph(
            &package,
            vec![try_node(1, &option, Some(none))],
            vec![],
            vec![export(1, "result")],
        );
        let plan = compile(&doc, snapshot.clone(), &package);
        let expected = if inner.starts_with("core/result") {
            Value::Ok(Arc::new(Value::None))
        } else {
            Value::None
        };
        assert_eq!(
            plan.constants()[plan.exports()[0].slot.index()],
            FoldedValue::Value(expected.clone())
        );
        let analysis = analyze(&doc, &snapshot);
        assert_eq!(
            reference::Reference::new(&doc, &snapshot, &analysis).exports(),
            [Ok(expected)]
        );
    }
}

#[test]
fn try_cannot_capture_admission_cancellation_or_backend_failures() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![node(1, "divide", 1, 0), try_node(2, "core/i64@1", None)],
        vec![edge(1, 1, "quotient", 2, "value")],
        vec![export(2, "result")],
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
    let rejected = GuardResolver {
        package: package.clone(),
        trap: id(1).parse().unwrap(),
        reject: true,
        calls: AtomicUsize::new(0),
    };
    assert!(matches!(
        compile_pure(
            &doc,
            snapshot.clone(),
            &analysis,
            &rejected,
            CompileOptions::default()
        ),
        Err(CompileError::KernelUnavailable { .. })
    ));
    let cancel = AtomicBool::new(true);
    assert_eq!(
        compile_pure(
            &doc,
            snapshot.clone(),
            &analysis,
            &package,
            CompileOptions {
                cancel: Some(&cancel),
                ..Default::default()
            }
        )
        .unwrap_err(),
        CompileError::Cancelled
    );
    assert_eq!(
        compile_pure(
            &doc,
            snapshot,
            &analysis,
            &package,
            CompileOptions {
                limits: orange_compiler::CompileLimits {
                    max_constant_bytes: 100,
                    ..Default::default()
                },
                ..Default::default()
            }
        )
        .unwrap_err(),
        CompileError::ResourceLimit
    );
}

#[test]
fn try_in_an_inactive_arm_does_not_bind_or_warn() {
    let (package, snapshot) = catalog();
    let mut selection = select_node(4, true, "core/result@1<core/i64@1>", json!({}), json!({}));
    selection["inputs"]
        .as_object_mut()
        .unwrap()
        .remove("if_true");
    selection["inputs"]
        .as_object_mut()
        .unwrap()
        .remove("if_false");
    let doc = graph(
        &package,
        vec![
            node(1, "divide", 1, 0),
            try_node(2, "core/i64@1", None),
            try_node(3, "core/i64@1", Some(integer_literal(42))),
            selection,
        ],
        vec![
            edge(1, 1, "quotient", 2, "value"),
            edge(2, 2, "result", 4, "if_false"),
            edge(3, 3, "result", 4, "if_true"),
        ],
        vec![export(4, "result"), export(3, "result")],
    );
    let resolver = GuardResolver {
        package,
        trap: id(1).parse().unwrap(),
        reject: true,
        calls: AtomicUsize::new(0),
    };
    let plan = compile(&doc, snapshot, &resolver);
    assert_eq!(resolver.calls.load(Ordering::Relaxed), 0);
    assert_eq!(plan.steps().len(), 2);
    assert!(plan.diagnostics().is_empty());
    for export in plan.exports() {
        assert_eq!(
            plan.constants()[export.slot.index()],
            FoldedValue::Value(Value::I64(42))
        );
    }
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(128))]
    #[test]
    fn nested_try_chains_match_independent_reference(a in proptest::num::i64::ANY,b in proptest::num::i64::ANY,depth in 1u32..10) {
        let (package,snapshot)=catalog();
        let mut nodes=vec![node(1,"divide",a,b)]; let mut edges=vec![]; let mut exports=vec![export(1,"quotient")];
        let mut ty="core/i64@1".to_owned();
        for n in 2..=depth+1 {
            nodes.push(try_node(n,&ty,None));
            edges.push(edge(n,n-1,if n==2 {"quotient"} else {"result"},n,"value"));
            exports.push(export(n,"result"));
            ty=format!("core/result@1<{ty}>");
        }
        nodes.reverse();edges.reverse();exports.reverse();
        let doc=graph(&package,nodes,edges,exports);
        let doc=from_json(&to_json(&doc).unwrap()).unwrap();
        let analysis=analyze(&doc,&snapshot);
        let expected=reference::Reference::new(&doc,&snapshot,&analysis).exports();
        let resolver=Counting::new(package);let plan=compile(&doc,snapshot.clone(),&resolver);
        proptest::prop_assert_eq!(resolver.folds.load(Ordering::Relaxed),1);
        for (actual,expected) in plan.evaluate(&doc,&snapshot,EvaluationOptions::default()).unwrap().exports().zip(expected) {
            match (actual.unwrap().outcome,expected) {
                (Ok(actual),Ok(expected))=>proptest::prop_assert_eq!(actual,&expected),
                (Err(actual),Err(expected))=>{proptest::prop_assert_eq!(actual.source,expected.source);proptest::prop_assert_eq!(actual.fault.code,expected.code);}
                _=>proptest::prop_assert!(false,"try oracle mismatch"),
            }
        }
    }
}

#[test]
fn coalesce_distinguishes_none_present_scalars_and_json_null_after_roundtrip() {
    let (package, snapshot) = catalog();
    let leaves = [
        ("core/bool@1", Value::Bool(false)),
        ("core/i32@1", Value::I32(-1)),
        ("core/i64@1", Value::I64(i64::MIN)),
        ("core/u32@1", Value::U32(u32::MAX)),
        ("core/u64@1", Value::U64(u64::MAX)),
        ("core/f32@1", Value::F32(-0.0)),
        ("core/f64@1", Value::F64(f64::from_bits(0x7ff8000000000123))),
        (
            "core/json@1",
            Value::Json(Arc::new(
                orange_model::value::JsonText::new("null".into()).unwrap(),
            )),
        ),
    ];
    for (ty, value) in leaves {
        let option = format!("core/option@1<{ty}>");
        for present in [false, true] {
            let initial = if present { &value } else { &Value::None };
            let doc = graph(
                &package,
                vec![coalesce_node(
                    1,
                    ty,
                    typed_literal(&snapshot, &option, initial),
                    typed_literal(&snapshot, ty, &value),
                )],
                vec![],
                vec![export(1, "result")],
            );
            let doc = from_json(&to_json(&doc).unwrap()).unwrap();
            let analysis = analyze(&doc, &snapshot);
            let expected = reference::Reference::new(&doc, &snapshot, &analysis).exports();
            let plan = compile(&doc, snapshot.clone(), &package);
            let result = plan
                .evaluate(&doc, &snapshot, EvaluationOptions::default())
                .unwrap()
                .export(0)
                .unwrap()
                .unwrap();
            assert_eq!(result.outcome, Ok(&value));
            assert_eq!(expected, [Ok(value.clone())]);
            assert_eq!(plan.steps()[0].inputs().len(), if present { 1 } else { 2 });
            assert!(plan.diagnostics().is_empty());
        }
    }
}

#[test]
fn coalesce_skips_missing_backend_and_propagates_demanded_fallback_fault() {
    let (package, snapshot) = catalog();
    for present in [false, true] {
        let doc = graph(
            &package,
            vec![
                node(1, "divide", 1, 0),
                coalesce_node(
                    2,
                    "core/i64@1",
                    typed_literal(
                        &snapshot,
                        "core/option@1<core/i64@1>",
                        if present {
                            &Value::I64(42)
                        } else {
                            &Value::None
                        },
                    ),
                    integer_literal(-1),
                ),
            ],
            vec![edge(1, 1, "quotient", 2, "fallback")],
            vec![export(2, "result")],
        );
        let unavailable = GuardResolver {
            package: package.clone(),
            trap: id(1).parse().unwrap(),
            reject: true,
            calls: AtomicUsize::new(0),
        };
        let analysis = analyze(&doc, &snapshot);
        let result = compile_pure(
            &doc,
            snapshot.clone(),
            &analysis,
            &unavailable,
            CompileOptions::default(),
        );
        assert_eq!(
            unavailable.calls.load(Ordering::Relaxed),
            usize::from(!present)
        );
        assert_eq!(result.is_ok(), present);
        let plan = compile(&doc, snapshot.clone(), &package);
        let outcome = &plan.constants()[plan.exports()[0].slot.index()];
        if present {
            assert_eq!(outcome, &FoldedValue::Value(Value::I64(42)));
        } else {
            let FoldedValue::Fault(fault) = outcome else {
                panic!("missing fault")
            };
            assert_eq!(fault.source, id(1).parse().unwrap());
            assert_eq!(fault.fault.code, "core/DivideByZero");
        }
        assert_eq!(plan.diagnostics().len(), usize::from(!present));
    }
}

#[test]
fn coalesce_selector_fault_is_not_absence_and_shared_exports_fold_once() {
    let (package, snapshot) = catalog();
    // A faulting Select supplies an option. Coalesce must neither hide the fault nor
    // demand its unavailable fallback. Another export can demand that fallback.
    let option = "core/option@1<core/i64@1>";
    let none = typed_literal(&snapshot, option, &Value::None);
    for separate_export in [false, true] {
        let mut selected = select_node(2, true, option, none.clone(), none.clone());
        selected["inputs"]["condition"] = json!({"type":"core/bool@1","value":true});
        let mut exports = vec![export(3, "result"), export(2, "result")];
        if separate_export {
            exports.push(export(4, "sum"));
        }
        let doc = graph(
            &package,
            vec![
                not_node(1, true),
                selected,
                coalesce_node(3, "core/i64@1", none.clone(), integer_literal(0)),
                node(4, "add", 20, 22),
            ],
            vec![
                edge(1, 1, "result", 2, "condition"),
                edge(2, 2, "result", 3, "value"),
                edge(3, 4, "sum", 3, "fallback"),
            ],
            exports,
        );
        let resolver = GuardResolver {
            package: package.clone(),
            trap: id(1).parse().unwrap(),
            reject: false,
            calls: AtomicUsize::new(0),
        };
        let plan = compile(&doc, snapshot.clone(), &resolver);
        assert_eq!(
            resolver.calls.load(Ordering::Relaxed),
            if separate_export { 2 } else { 1 }
        );
        assert_eq!(plan.diagnostics().len(), 1);
        for export in &plan.exports()[..2] {
            let FoldedValue::Fault(fault) = &plan.constants()[export.slot.index()] else {
                panic!("fault lost")
            };
            assert_eq!(fault.source, id(1).parse().unwrap());
        }
    }
}

#[test]
fn nested_some_none_is_present_and_export_type_is_borrowed() {
    let (package, snapshot) = catalog();
    let ty = "core/option@1<core/i64@1>";
    let outer = "core/option@1<core/option@1<core/i64@1>>";
    for present in [false, true] {
        let initial = if present {
            Value::Some(Arc::new(Value::None))
        } else {
            Value::None
        };
        let expected = if present { Value::None } else { Value::I64(99) };
        let doc = graph(
            &package,
            vec![coalesce_node(
                1,
                ty,
                typed_literal(&snapshot, outer, &initial),
                typed_literal(&snapshot, ty, &Value::I64(99)),
            )],
            vec![],
            vec![export(1, "result")],
        );
        let plan = compile(&doc, snapshot.clone(), &package);
        let result = plan
            .evaluate(&doc, &snapshot, EvaluationOptions::default())
            .unwrap()
            .export(0)
            .unwrap()
            .unwrap();
        assert_eq!(result.outcome, Ok(&expected));
        assert_eq!(result.ty, &PlanType::Option(Box::new(PlanType::I64)));
        assert!(std::ptr::eq(
            result.ty,
            &plan.slot_types()[plan.exports()[0].slot.index()]
        ));
        let analysis = analyze(&doc, &snapshot);
        assert_eq!(
            reference::Reference::new(&doc, &snapshot, &analysis).exports(),
            [Ok(expected)]
        );
    }
}

#[test]
fn option_json_payload_budget_is_checked_and_inactive_fallback_is_not_decoded() {
    let (package, snapshot) = catalog();
    let small = Value::Json(Arc::new(
        orange_model::value::JsonText::new("null".into()).unwrap(),
    ));
    let large = Value::Json(Arc::new(
        orange_model::value::JsonText::new(format!("\"{}\"", "x".repeat(100_000))).unwrap(),
    ));
    for large_selector in [false, true] {
        let doc = graph(
            &package,
            vec![coalesce_node(
                1,
                "core/json@1",
                typed_literal(
                    &snapshot,
                    "core/option@1<core/json@1>",
                    if large_selector { &large } else { &small },
                ),
                typed_literal(&snapshot, "core/json@1", &large),
            )],
            vec![],
            vec![export(1, "result")],
        );
        let analysis = analyze(&doc, &snapshot);
        let options = CompileOptions {
            limits: orange_compiler::CompileLimits {
                max_constant_bytes: 96,
                ..Default::default()
            },
            ..Default::default()
        };
        let result = compile_pure(&doc, snapshot.clone(), &analysis, &package, options);
        if large_selector {
            assert!(matches!(result, Err(CompileError::ResourceLimit)));
        } else {
            assert!(result.is_ok());
        }
    }
}

#[test]
fn nested_option_boxes_are_charged_to_retained_constant_budget() {
    let (package, snapshot) = catalog();
    let ty = "core/option@1<core/option@1<core/i64@1>>";
    let outer = format!("core/option@1<{ty}>");
    let present = Value::Some(Arc::new(Value::I64(42)));
    let selector = Value::Some(Arc::new(present.clone()));
    let doc = graph(
        &package,
        vec![coalesce_node(
            1,
            ty,
            typed_literal(&snapshot, &outer, &selector),
            typed_literal(&snapshot, ty, &Value::None),
        )],
        vec![],
        vec![export(1, "result")],
    );
    let analysis = analyze(&doc, &snapshot);
    // Selector = 48; result = 32; persisted export ID "out-1" = 5.
    for budget in [84, 85] {
        let options = CompileOptions {
            limits: orange_compiler::CompileLimits {
                max_constant_bytes: budget,
                ..Default::default()
            },
            ..Default::default()
        };
        let result = compile_pure(&doc, snapshot.clone(), &analysis, &package, options);
        if budget == 84 {
            assert!(matches!(result, Err(CompileError::ResourceLimit)));
        } else {
            assert_eq!(
                result.unwrap().constants().last(),
                Some(&FoldedValue::Value(present.clone()))
            );
        }
    }
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(128))]
    #[test]
    fn nested_options_coalesce_with_independent_oracle(depth in 1usize..8, present in proptest::bool::ANY,
        inner_absent in proptest::bool::ANY, number in proptest::num::i64::ANY) {
        let (package,snapshot)=catalog();
        let mut ty="core/i64@1".to_owned();
        let mut value=Value::I64(number);
        for layer in 0..depth {
            if layer==0 && inner_absent { value=Value::None; }
            else if layer>0 {value=Value::Some(Arc::new(value));}
            ty=format!("core/option@1<{ty}>");
        }
        let outer=format!("core/option@1<{ty}>");
        let initial=if present {Value::Some(Arc::new(value.clone()))} else {Value::None};
        let mut fallback=Value::I64(number.wrapping_add(1));
        for _ in 1..depth {fallback=Value::Some(Arc::new(fallback));}
        let doc=graph(&package,vec![coalesce_node(1,&ty,typed_literal(&snapshot,&outer,&initial),
            typed_literal(&snapshot,&ty,&fallback))],vec![],vec![export(1,"result")]);
        let doc=from_json(&to_json(&doc).unwrap()).unwrap();
        let analysis=analyze(&doc,&snapshot);
        let expected=reference::Reference::new(&doc,&snapshot,&analysis).exports();
        let plan=compile(&doc,snapshot.clone(),&package);
        let result=plan.evaluate(&doc,&snapshot,EvaluationOptions::default()).unwrap().export(0).unwrap().unwrap();
        let answer=if present {value} else {fallback};
        proptest::prop_assert_eq!(result.outcome,Ok(&answer));
        proptest::prop_assert_eq!(expected,vec![Ok(answer)]);
    }
}

#[test]
fn lazy_select_folds_only_chosen_cone_with_original_fault_or_value() {
    let (package, snapshot) = catalog();
    for condition in [true, false] {
        let select = select_node(
            3,
            condition,
            "core/i64@1",
            integer_literal(-999),
            integer_literal(-999),
        );
        let doc = graph(
            &package,
            vec![select, node(1, "divide", 1, 0), node(2, "add", 20, 22)],
            vec![
                edge(1, 2, "sum", 3, "if_true"),
                edge(2, 1, "quotient", 3, "if_false"),
            ],
            vec![export(3, "result")],
        );
        let resolver = Counting::new(package.clone());
        let plan = compile(&doc, snapshot.clone(), &resolver);
        let actual = plan
            .evaluate(&doc, &snapshot, EvaluationOptions::default())
            .unwrap()
            .export(0)
            .unwrap()
            .unwrap();
        assert_eq!(resolver.resolves.load(Ordering::Relaxed), 1);
        assert_eq!(resolver.folds.load(Ordering::Relaxed), 1);
        assert_eq!(plan.steps().len(), 2);
        assert_eq!(plan.steps()[1].inputs().len(), 2);
        assert_eq!(
            plan.steps()[1].inputs()[1].port,
            if condition { "if_true" } else { "if_false" }
                .parse()
                .unwrap()
        );
        if condition {
            assert_eq!(actual.outcome, Ok(&Value::I64(42)));
            assert!(plan.diagnostics().is_empty());
        } else {
            assert_eq!(actual.outcome.unwrap_err().source, id(1).parse().unwrap());
            assert_eq!(actual.outcome.unwrap_err().fault.code, "core/DivideByZero");
            assert_eq!(plan.diagnostics().len(), 1);
        }
        let analysis = analyze(&doc, &snapshot);
        let expected = reference::Reference::new(&doc, &snapshot, &analysis).exports();
        match (&expected[0], actual.outcome) {
            (Ok(value), Ok(actual)) => assert_eq!(value, actual),
            (Err(fault), Err(actual)) => assert_eq!(fault.source, actual.source),
            _ => panic!("oracle selection mismatch"),
        }
    }
}

#[derive(Debug)]
struct DeliberateFault;
impl PureKernel for DeliberateFault {
    fn fold(&self, _: &[Value], _: &mut [Value]) -> Result<(), KernelFault> {
        Err(KernelFault {
            code: "core/DeliberateFault",
            message_key: "core/test-fault",
            message: "Test kernel failed.",
        })
    }
}
struct GuardResolver {
    package: MathPackage,
    trap: orange_model::id::NodeId,
    reject: bool,
    calls: AtomicUsize,
}
impl PureKernelResolver for GuardResolver {
    fn resolve(
        &self,
        doc: &GraphDocument,
        node: &NodeInstance,
        sig: &SignatureSnapshot,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Result<KernelBinding, ResolveError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if node.id == self.trap && self.reject {
            return Err(ResolveError::Unavailable);
        }
        let mut binding = PureKernelResolver::resolve(&self.package, doc, node, sig, snapshot)?;
        if node.id == self.trap {
            binding.kernel = Arc::new(DeliberateFault);
        }
        Ok(binding)
    }
}
fn not_node(n: u32, value: bool) -> Json {
    json!({"id":id(n),"definition_id":"org.orange.math/not","definition_version":1,"config_version":1,
        "config":{},"signature":"not","provider":{"package":"org.orange.math","version":"0.0.0","id":"native-v1"},
        "inputs":{"value":{"type":"core/bool@1","value":value}}})
}

#[test]
fn unselected_backend_is_not_resolved_and_selector_fault_skips_both_arms() {
    let (package, snapshot) = catalog();
    for condition in [true, false] {
        let doc = graph(
            &package,
            vec![
                node(1, "divide", 1, 0),
                node(2, "add", 20, 22),
                select_node(
                    3,
                    condition,
                    "core/i64@1",
                    integer_literal(0),
                    integer_literal(0),
                ),
            ],
            vec![
                edge(1, 2, "sum", 3, "if_true"),
                edge(2, 1, "quotient", 3, "if_false"),
            ],
            vec![export(3, "result")],
        );
        let resolver = GuardResolver {
            package: package.clone(),
            trap: id(1).parse().unwrap(),
            reject: true,
            calls: AtomicUsize::new(0),
        };
        let analysis = analyze(&doc, &snapshot);
        let result = compile_pure(
            &doc,
            snapshot.clone(),
            &analysis,
            &resolver,
            CompileOptions::default(),
        );
        if condition {
            assert!(result.is_ok());
        } else {
            assert!(matches!(
                result,
                Err(CompileError::KernelUnavailable { .. })
            ));
        }
        assert_eq!(resolver.calls.load(Ordering::Relaxed), 1);
    }
    let doc = graph(
        &package,
        vec![
            not_node(1, true),
            node(2, "add", 20, 22),
            node(3, "divide", 1, 0),
            select_node(
                4,
                true,
                "core/i64@1",
                integer_literal(0),
                integer_literal(0),
            ),
        ],
        vec![
            edge(1, 1, "result", 4, "condition"),
            edge(2, 2, "sum", 4, "if_true"),
            edge(3, 3, "quotient", 4, "if_false"),
        ],
        vec![export(4, "result")],
    );
    let resolver = GuardResolver {
        package,
        trap: id(1).parse().unwrap(),
        reject: false,
        calls: AtomicUsize::new(0),
    };
    let plan = compile(&doc, snapshot.clone(), &resolver);
    assert_eq!(resolver.calls.load(Ordering::Relaxed), 1);
    assert_eq!(plan.steps().len(), 2);
    assert_eq!(plan.steps()[1].inputs().len(), 1);
    let evaluation = plan
        .evaluate(&doc, &snapshot, EvaluationOptions::default())
        .unwrap();
    let fault = evaluation.export(0).unwrap().unwrap().outcome.unwrap_err();
    assert_eq!(fault.source, id(1).parse().unwrap());
    assert_eq!(fault.fault.code, "core/DeliberateFault");
    assert_eq!(plan.diagnostics().len(), 1);
}

#[test]
fn and_or_short_circuit_without_resolving_faulty_right_hand_side() {
    let (package, snapshot) = catalog();
    for (intrinsic, short_value) in [("core/and", false), ("core/or", true)] {
        for short in [true, false] {
            let left = if short { short_value } else { !short_value };
            let logic = json!({"id":id(2),"definition_id":intrinsic,"definition_version":1,"config_version":1,
                "config":{},"inputs":{"a":{"type":"core/bool@1","value":left}}});
            let doc = graph(
                &package,
                vec![logic, not_node(1, true)],
                vec![edge(1, 1, "result", 2, "b")],
                vec![export(2, "result")],
            );
            let resolver = GuardResolver {
                package: package.clone(),
                trap: id(1).parse().unwrap(),
                reject: false,
                calls: AtomicUsize::new(0),
            };
            let plan = compile(&doc, snapshot.clone(), &resolver);
            let evaluation = plan
                .evaluate(&doc, &snapshot, EvaluationOptions::default())
                .unwrap();
            let outcome = evaluation.export(0).unwrap().unwrap().outcome;
            if short {
                assert_eq!(outcome, Ok(&Value::Bool(short_value)));
                assert!(plan.diagnostics().is_empty());
            } else {
                assert_eq!(outcome.unwrap_err().source, id(1).parse().unwrap());
            }
            assert_eq!(resolver.calls.load(Ordering::Relaxed), usize::from(!short));
        }
    }
}

#[test]
fn unselected_json_literal_does_not_consume_decode_or_constant_budget() {
    let (package, snapshot) = catalog();
    for selected_small in [true, false] {
        let large = json!({"type":"core/json@1","value":json!({"padding":"x".repeat(100_000)}).to_string()});
        let small = json!({"type":"core/json@1","value":"{}"});
        let doc = graph(
            &package,
            vec![select_node(1, selected_small, "core/json@1", small, large)],
            vec![],
            vec![export(1, "result")],
        );
        let analysis = analyze(&doc, &snapshot);
        assert!(
            analysis.is_ready_for_lowering(&doc, &snapshot),
            "{:?}",
            analysis.report().diagnostics
        );
        let mut options = CompileOptions::default();
        options.limits.max_constant_bytes = 96;
        let result = compile_pure(&doc, snapshot.clone(), &analysis, &package, options);
        if selected_small {
            let plan = result.unwrap();
            let evaluation = plan
                .evaluate(&doc, &snapshot, EvaluationOptions::default())
                .unwrap();
            assert!(
                matches!(evaluation.export(0).unwrap().unwrap().outcome,Ok(Value::Json(text)) if text.as_str() == "{}")
            );
            assert_eq!(plan.constants().len(), 3); // selector, chosen literal, result
        } else {
            assert!(matches!(result, Err(CompileError::ResourceLimit)));
        }
    }
}

#[test]
fn nested_lazy_selections_share_values_and_remain_iterative_at_depth() {
    let (package, snapshot) = catalog();
    let mut nodes = vec![node(1, "add", 7, 8)];
    let mut edges = Vec::new();
    for n in 2..=513 {
        nodes.push(select_node(
            n,
            true,
            "core/i64@1",
            integer_literal(0),
            integer_literal(-1),
        ));
        edges.push(edge(
            n,
            n - 1,
            if n == 2 { "sum" } else { "result" },
            n,
            "if_true",
        ));
    }
    nodes.reverse();
    edges.reverse();
    let doc = graph(
        &package,
        nodes,
        edges,
        vec![export(513, "result"), export(256, "result")],
    );
    let resolver = Counting::new(package);
    let plan = compile(&doc, snapshot.clone(), &resolver);
    assert_eq!(plan.steps().len(), 513);
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 1);
    let evaluation = plan
        .evaluate(&doc, &snapshot, EvaluationOptions::default())
        .unwrap();
    for export in evaluation.exports() {
        assert_eq!(export.unwrap().outcome, Ok(&Value::I64(15)));
    }
}

#[test]
fn an_unselected_cone_still_folds_when_requested_by_another_export() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![
            node(1, "divide", 1, 0),
            node(2, "add", 20, 22),
            select_node(
                3,
                true,
                "core/i64@1",
                integer_literal(0),
                integer_literal(0),
            ),
        ],
        vec![
            edge(1, 2, "sum", 3, "if_true"),
            edge(2, 1, "quotient", 3, "if_false"),
        ],
        vec![export(3, "result"), export(1, "quotient")],
    );
    let resolver = Counting::new(package);
    let plan = compile(&doc, snapshot.clone(), &resolver);
    let evaluation = plan
        .evaluate(&doc, &snapshot, EvaluationOptions::default())
        .unwrap();
    assert_eq!(
        evaluation.export(0).unwrap().unwrap().outcome,
        Ok(&Value::I64(42))
    );
    assert_eq!(
        evaluation
            .export(1)
            .unwrap()
            .unwrap()
            .outcome
            .unwrap_err()
            .source,
        id(1).parse().unwrap()
    );
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 2);
    assert_eq!(plan.diagnostics().len(), 1);
}

#[test]
fn lazy_preflight_and_unsupported_forms_fail_before_external_resolution() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![
            node(1, "divide", 1, 0),
            node(2, "add", 20, 22),
            select_node(
                3,
                true,
                "core/i64@1",
                integer_literal(0),
                integer_literal(0),
            ),
        ],
        vec![
            edge(1, 2, "sum", 3, "if_true"),
            edge(2, 1, "quotient", 3, "if_false"),
        ],
        vec![export(3, "result")],
    );
    let analysis = analyze(&doc, &snapshot);
    let resolver = Counting::new(package.clone());
    let mut options = CompileOptions::default();
    options.limits.max_fold_operations = 2;
    // Admission is conservative over the whole potential cone, not only chosen work.
    assert!(matches!(
        compile_pure(&doc, snapshot.clone(), &analysis, &resolver, options),
        Err(CompileError::ResourceLimit)
    ));
    let cancel = AtomicBool::new(true);
    assert!(matches!(
        compile_pure(
            &doc,
            snapshot.clone(),
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
    let unimplemented = json!({"id":id(1),"definition_id":"core/try","definition_version":1,"config_version":1,
        "config":{},"type_pins":{"T":"core/string@1"},"inputs":{"value":{"type":"core/string@1","value":"unsupported leaf"}}});
    let doc = graph(&package, vec![unimplemented], vec![], vec![]);
    let analysis = analyze(&doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(&doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    assert!(matches!(
        compile_pure(
            &doc,
            snapshot,
            &analysis,
            &resolver,
            CompileOptions::default()
        ),
        Err(CompileError::UnsupportedPort { .. })
    ));
    assert_eq!(resolver.resolves.load(Ordering::Relaxed), 0);
}

#[test]
fn cancellation_after_selector_stops_before_arm_resolution() {
    #[derive(Debug)]
    struct CancelAfter {
        inner: Arc<dyn PureKernel>,
        signal: Arc<AtomicBool>,
    }
    impl PureKernel for CancelAfter {
        fn fold(&self, inputs: &[Value], outputs: &mut [Value]) -> Result<(), KernelFault> {
            let result = self.inner.fold(inputs, outputs);
            self.signal.store(true, Ordering::Relaxed);
            result
        }
    }
    struct Resolver {
        package: MathPackage,
        signal: Arc<AtomicBool>,
        calls: AtomicUsize,
    }
    impl PureKernelResolver for Resolver {
        fn resolve(
            &self,
            doc: &GraphDocument,
            node: &NodeInstance,
            sig: &SignatureSnapshot,
            snapshot: Arc<RegistrySnapshot>,
        ) -> Result<KernelBinding, ResolveError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let mut binding = PureKernelResolver::resolve(&self.package, doc, node, sig, snapshot)?;
            if node.id == id(1).parse().unwrap() {
                binding.kernel = Arc::new(CancelAfter {
                    inner: binding.kernel,
                    signal: self.signal.clone(),
                });
            }
            Ok(binding)
        }
    }
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![
            not_node(1, true),
            node(2, "add", 20, 22),
            node(3, "divide", 1, 0),
            select_node(
                4,
                true,
                "core/i64@1",
                integer_literal(0),
                integer_literal(0),
            ),
        ],
        vec![
            edge(1, 1, "result", 4, "condition"),
            edge(2, 2, "sum", 4, "if_true"),
            edge(3, 3, "quotient", 4, "if_false"),
        ],
        vec![export(4, "result")],
    );
    let analysis = analyze(&doc, &snapshot);
    let signal = Arc::new(AtomicBool::new(false));
    let resolver = Resolver {
        package,
        signal: signal.clone(),
        calls: AtomicUsize::new(0),
    };
    assert!(matches!(
        compile_pure(
            &doc,
            snapshot,
            &analysis,
            &resolver,
            CompileOptions {
                cancel: Some(&signal),
                ..Default::default()
            }
        ),
        Err(CompileError::Cancelled)
    ));
    assert_eq!(resolver.calls.load(Ordering::Relaxed), 1);
}

#[test]
fn checked_evaluation_is_borrowed_pinned_and_does_not_reexecute_kernels() {
    let (package, snapshot) = catalog();
    let mut doc = graph(
        &package,
        vec![node(1, "add", 20, 22)],
        vec![],
        vec![export(1, "sum")],
    );
    let original = doc.clone();
    let resolver = Counting::new(package);
    let plan = compile(&doc, snapshot.clone(), &resolver);
    let evaluation = plan
        .evaluate(&doc, &snapshot, EvaluationOptions::default())
        .unwrap();
    let result = evaluation.export(0).unwrap().unwrap();
    assert_eq!(result.id, "out-1");
    assert_eq!(result.ty, &PlanType::I64);
    assert_eq!(result.outcome, Ok(&Value::I64(42)));
    let FoldedValue::Value(constant) = &plan.constants()[plan.exports()[0].slot.index()] else {
        panic!()
    };
    assert!(std::ptr::eq(result.outcome.unwrap(), constant));
    assert!(Arc::ptr_eq(evaluation.snapshot(), &snapshot));
    let change = doc
        .apply([orange_model::DocCommand::SetLiteral {
            port: orange_model::document::PortRef {
                node: id(1).parse().unwrap(),
                port: "a".parse().unwrap(),
            },
            literal: Some(orange_model::document::literal::Literal::value(
                "core/i64@1".parse().unwrap(),
                json!("21"),
            )),
        }])
        .unwrap();
    assert!(matches!(
        plan.evaluate(&doc, &snapshot, EvaluationOptions::default()),
        Err(EvaluationError::StaleDocument)
    ));
    let (_, new_snapshot) = catalog();
    assert!(matches!(
        plan.evaluate(&original, &new_snapshot, EvaluationOptions::default()),
        Err(EvaluationError::StaleRegistry)
    ));
    assert_eq!(
        evaluation.export(0).unwrap().unwrap().outcome,
        Ok(&Value::I64(42))
    );
    assert_eq!(evaluation.document_revision(), 0);
    let _ = doc.revert(change.inverse).unwrap();
    let restored = plan
        .evaluate(&doc, &snapshot, EvaluationOptions::default())
        .unwrap();
    assert_eq!(restored.document_revision(), doc.revision());
    assert_eq!(restored.source_revision(), 0);
    assert_eq!(resolver.folds.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.resolves.load(Ordering::Relaxed), 1);
}

#[test]
fn evaluation_keeps_graph_faults_distinct_from_entry_errors() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![node(1, "divide", 1, 0), node(2, "add", 0, 1)],
        vec![edge(1, 1, "quotient", 2, "a")],
        vec![export(2, "sum")],
    );
    let plan = compile(&doc, snapshot.clone(), &package);
    let evaluation = plan
        .evaluate(&doc, &snapshot, EvaluationOptions::default())
        .unwrap();
    let result = evaluation.exports().next().unwrap().unwrap();
    assert_eq!(result.source.node, id(2).parse().unwrap());
    assert_eq!(result.source.port, "sum".parse().unwrap());
    let fault = result.outcome.unwrap_err();
    assert_eq!(fault.source, id(1).parse().unwrap());
    assert_eq!(fault.fault.code, "core/DivideByZero");
}

#[test]
fn evaluation_export_limit_and_cancellation_are_checked_at_entry_and_reads() {
    let (package, snapshot) = catalog();
    let doc = graph(
        &package,
        vec![node(1, "add", 20, 22)],
        vec![],
        vec![
            export(1, "sum"),
            json!({"id":"second","node":id(1),"port":"sum"}),
        ],
    );
    let plan = compile(&doc, snapshot.clone(), &package);
    let cancel = AtomicBool::new(true);
    assert!(matches!(
        plan.evaluate(
            &doc,
            &snapshot,
            EvaluationOptions {
                cancel: Some(&cancel),
                ..Default::default()
            }
        ),
        Err(EvaluationError::Cancelled)
    ));
    assert!(matches!(
        plan.evaluate(
            &doc,
            &snapshot,
            EvaluationOptions {
                max_exports: 1,
                ..Default::default()
            }
        ),
        Err(EvaluationError::ResourceLimit)
    ));
    cancel.store(false, Ordering::Relaxed);
    let evaluation = plan
        .evaluate(
            &doc,
            &snapshot,
            EvaluationOptions {
                max_exports: 2,
                cancel: Some(&cancel),
            },
        )
        .unwrap();
    assert_eq!(evaluation.len(), 2);
    assert!(!evaluation.is_empty());
    let mut results = evaluation.exports();
    assert_eq!(results.len(), 2);
    assert_eq!(results.next().unwrap().unwrap().id, "out-1");
    cancel.store(true, Ordering::Relaxed);
    assert_eq!(results.next().unwrap(), Err(EvaluationError::Cancelled));
    assert_eq!(evaluation.export(1), Err(EvaluationError::Cancelled));
    assert_eq!(
        EvaluationError::Cancelled.message_key(),
        "core/evaluate-cancelled"
    );
}

#[test]
fn empty_pure_evaluation_and_missing_export_have_explicit_results() {
    let (package, snapshot) = catalog();
    let doc = graph(&package, vec![], vec![], vec![]);
    let plan = compile(&doc, snapshot.clone(), &package);
    let evaluation = plan
        .evaluate(
            &doc,
            &snapshot,
            EvaluationOptions {
                max_exports: 0,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(evaluation.is_empty());
    assert_eq!(evaluation.exports().len(), 0);
    assert_eq!(evaluation.export(0).unwrap(), None);
}

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
    builder = builder.definition(orange_bundled::random_u64_definition().unwrap());
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
