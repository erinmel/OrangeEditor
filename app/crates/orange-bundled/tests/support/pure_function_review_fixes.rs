use super::*;
use orange_model::catalog::Ports;
use orange_model::document::literal::Literal;
use orange_model::document::{OutputSource, PortDescriptor, TypeParam};

#[derive(Debug)]
struct CopyValue;
impl PureKernel for CopyValue {
    fn fold(&self, inputs: &[Value], outputs: &mut [Value]) -> Result<(), KernelFault> {
        outputs[0] = inputs[0].clone();
        Ok(())
    }
}
struct CheckDefaults<'a> {
    borrowed: Option<&'a Literal>,
    shared: AtomicUsize,
    calls: AtomicUsize,
}
impl PureKernelResolver for CheckDefaults<'_> {
    fn resolve(
        &self,
        _: &GraphDocument,
        _: &NodeInstance,
        signature: &SignatureSnapshot,
        _: Arc<RegistrySnapshot>,
    ) -> Result<KernelBinding, ResolveError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let default = signature
            .ports
            .iter()
            .find_map(|p| p.default.as_ref())
            .unwrap();
        if let Some(expected) = self.borrowed {
            assert!(
                std::ptr::eq(default, expected),
                "nongeneric defaults must be borrowed from fresh analysis"
            );
        } else {
            let address = std::ptr::from_ref(default).addr();
            if let Err(first) =
                self.shared
                    .compare_exchange(0, address, Ordering::Relaxed, Ordering::Relaxed)
            {
                assert_eq!(
                    first, address,
                    "one specialized body must be shared across invocations"
                );
            }
        }
        Ok(KernelBinding {
            inputs: signature
                .ports
                .iter()
                .filter(|p| p.direction == orange_model::document::PortDirection::Input)
                .map(|p| orange_model::id::PortKey::new(p.port_id.clone(), None))
                .collect(),
            outputs: vec!["result".parse().unwrap()].into(),
            kernel: Arc::new(CopyValue),
        })
    }
}
fn default_fixture(
    generic: bool,
    calls: u32,
) -> (GraphDocument, Arc<RegistrySnapshot>, MathPackage) {
    let (package, original) = catalog();
    let mut provider = original.provider(package.provider_key()).unwrap().clone();
    let mut builder = RegistryBuilder::core(original.origin().clone());
    for definition in math_definitions()
        .unwrap()
        .into_iter()
        .chain([orange_bundled::random_u64_definition().unwrap()])
    {
        builder = builder.definition(definition);
    }
    let mut definition = math_definitions().unwrap().pop().unwrap();
    definition.definition.id = "acme.review/default".parse().unwrap();
    definition.type_params = if generic {
        vec![TypeParam {
            name: "T".into(),
            constraints: vec![],
            extensions: Default::default(),
        }]
    } else {
        vec![]
    };
    definition.specializations = None;
    let default = serde_json::from_value(
        json!({"type":"core/json@1","value":format!("\"{}\"","x".repeat(65536))}),
    )
    .unwrap();
    let mut ports = Vec::new();
    if generic {
        ports.push(PortDescriptor::data_in("value", "$T".parse().unwrap()));
    }
    ports.push(
        PortDescriptor::data_in("fallback", "core/json@1".parse().unwrap()).with_default(default),
    );
    ports.push(PortDescriptor::data_out(
        "result",
        if generic { "$T" } else { "core/json@1" }.parse().unwrap(),
        OutputSource::Computed,
    ));
    definition.ports = Ports::Declared(ports.clone());
    provider
        .implements
        .insert((definition.definition.clone(), 1));
    provider.specializations.insert(
        definition.definition.clone(),
        vec![if generic {
            [("T".into(), "core/i64@1".parse().unwrap())].into()
        } else {
            Default::default()
        }],
    );
    let snapshot = builder
        .definition(definition)
        .provider(provider)
        .build()
        .unwrap();
    let base = graph(&package, vec![], vec![], vec![]);
    let mut wire: Json = serde_json::from_str(&to_json(&base).unwrap()).unwrap();
    wire["signatures"]["custom"] = serde_json::to_value(orange_model::signature::project(
        NodeKind::Pure,
        ports,
        &Default::default(),
    ))
    .unwrap();
    let mut function = identity(100, None);
    if !generic {
        function["type_params"] = json!([]);
        function["inputs"] = json!([]);
        function["outputs"][0]["type"] = json!("core/json@1");
    }
    let custom = json!({"id":id(103),"definition_id":"acme.review/default","definition_version":1,"config_version":1,"config":{},"signature":"custom",
        "provider":{"package":"org.orange.math","version":"0.0.0","id":"native-v1"}});
    function["nodes"] = if generic {
        json!([
            intrinsic(101, "function-entry"),
            custom,
            intrinsic(102, "function-outputs")
        ])
    } else {
        json!([custom, intrinsic(102, "function-outputs")])
    };
    let mut edges = vec![edge(103, 103, "result", 102, "result")];
    if generic {
        edges.push(edge(101, 101, "value", 103, "value"));
    }
    function["edges"] = json!(edges);
    wire["functions"] = json!([function]);
    wire["nodes"] = json!(
        (1..=calls)
            .map(|n| {
                if generic {
                    call(n, 100, "core/i64@1", integer_literal(n as i64))
                } else {
                    let mut c = intrinsic(n, "call");
                    c["config"] = json!({"function":id(100)});
                    c
                }
            })
            .collect::<Vec<_>>()
    );
    wire["exports"] = json!((1..=calls).map(|n| export(n, "result")).collect::<Vec<_>>());
    (from_json(&wire.to_string()).unwrap(), snapshot, package)
}

#[test]
fn large_nongeneric_defaults_stay_borrowed_across_rejected_and_successful_calls() {
    let (doc, snapshot, _) = default_fixture(false, 16);
    let analysis = analyze(&doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(&doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    let expected = analysis.report().resolved[&id(103).parse().unwrap()].ports[0]
        .default
        .as_ref()
        .unwrap();
    let resolver = CheckDefaults {
        borrowed: Some(expected),
        shared: AtomicUsize::new(0),
        calls: AtomicUsize::new(0),
    };
    let mut options = CompileOptions::default();
    options.limits.max_constant_bytes = 4096;
    assert!(matches!(
        compile_pure(&doc, snapshot.clone(), &analysis, &resolver, options),
        Err(CompileError::ResourceLimit)
    ));
    assert_eq!(resolver.calls.load(Ordering::Relaxed), 1);
    resolver.calls.store(0, Ordering::Relaxed);
    let plan = compile_pure(
        &doc,
        snapshot,
        &analysis,
        &resolver,
        CompileOptions::default(),
    )
    .unwrap();
    assert_eq!(resolver.calls.load(Ordering::Relaxed), 16);
    assert_eq!(plan.exports().len(), 16);
    assert!(
        outcomes(&plan)
            .iter()
            .all(|v| matches!(v, FoldedValue::Value(Value::Json(_))))
    );
}

#[test]
fn generic_signature_copies_are_charged_before_copying_and_shared_after_admission() {
    let (doc, snapshot, _) = default_fixture(true, 2);
    let analysis = analyze(&doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(&doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    let resolver = CheckDefaults {
        borrowed: None,
        shared: AtomicUsize::new(0),
        calls: AtomicUsize::new(0),
    };
    let mut options = CompileOptions::default();
    options.limits.max_constant_bytes = 4096;
    assert!(matches!(
        compile_pure(&doc, snapshot.clone(), &analysis, &resolver, options),
        Err(CompileError::ResourceLimit)
    ));
    assert_eq!(resolver.calls.load(Ordering::Relaxed), 0);
    let plan = compile_pure(
        &doc,
        snapshot,
        &analysis,
        &resolver,
        CompileOptions::default(),
    )
    .unwrap();
    assert_eq!(resolver.calls.load(Ordering::Relaxed), 2);
    assert_eq!(
        outcomes(&plan),
        [
            FoldedValue::Value(Value::I64(1)),
            FoldedValue::Value(Value::I64(2))
        ]
    );
}

#[test]
fn computed_source_admission_is_identical_at_root_and_inside_functions() {
    let (package, original) = catalog();
    let provider = original.provider(package.provider_key()).unwrap().clone();
    let mut builder = RegistryBuilder::core(original.origin().clone());
    for mut definition in math_definitions()
        .unwrap()
        .into_iter()
        .chain([orange_bundled::random_u64_definition().unwrap()])
    {
        if definition.definition.id.as_str() == "org.orange.math/add" {
            let Ports::Declared(ports) = &mut definition.ports else {
                panic!()
            };
            for port in ports
                .iter_mut()
                .filter(|p| p.direction == orange_model::document::PortDirection::Output)
            {
                port.source = Some(OutputSource::State);
            }
        }
        builder = builder.definition(definition);
    }
    let snapshot = builder.provider(provider).build().unwrap();
    let base = graph(
        &package,
        vec![node(10, "add", 1, 2)],
        vec![],
        vec![export(10, "sum")],
    );
    let mut wire: Json = serde_json::from_str(&to_json(&base).unwrap()).unwrap();
    for port in wire["signatures"]["add"]["ports"].as_array_mut().unwrap() {
        if port["direction"] == "output" {
            port["source"] = json!("state");
        }
    }
    for nested in [false, true] {
        let mut wire = wire.clone();
        if nested {
            let body = wire["nodes"][0].clone();
            let mut call = intrinsic(1, "call");
            call["config"] = json!({"function":id(100)});
            wire["nodes"] = json!([call]);
            wire["exports"] = json!([export(1, "result")]);
            wire["functions"] = json!([{"id":id(100),"name":"f","kind":"pure","inputs":[],"outputs":[{"port_id":"result","direction":"output","kind":"data","type":"core/i64@1"}],
                "nodes":[body,intrinsic(11,"function-outputs")],"edges":[edge(10,10,"sum",11,"result")]}]);
        }
        let doc = from_json(&wire.to_string()).unwrap();
        let analysis = analyze(&doc, &snapshot);
        assert!(analysis.is_ready_for_lowering(&doc, &snapshot));
        let resolver = Counting::new(package.clone());
        assert!(
            matches!(compile_pure(&doc,snapshot.clone(),&analysis,&resolver,CompileOptions::default()),Err(CompileError::UnsupportedPort{node,..}) if node==id(10).parse().unwrap())
        );
        assert_eq!(resolver.resolves.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn wide_declaration_order_matches_exact_keys_and_types_without_resorting_ports() {
    let (package, snapshot) = catalog();
    let (mut inputs, mut outputs, mut edges) = (Vec::new(), Vec::new(), Vec::new());
    let mut literals = serde_json::Map::new();
    for i in 0..256 {
        let name = format!("arg{}", i * 17 % 256);
        let result = format!("out{i}");
        let ty = if i % 2 == 0 {
            "core/i64@1"
        } else {
            "core/bool@1"
        };
        inputs.push(json!({"port_id":name,"direction":"input","kind":"data","type":ty}));
        outputs.push(json!({"port_id":result,"direction":"output","kind":"data","type":ty}));
        literals.insert(
            name.clone(),
            if i % 2 == 0 {
                integer_literal(i)
            } else {
                json!({"type":ty,"value":true})
            },
        );
        edges.push(json!({"id":format!("30000000-0000-4000-8000-{i:012x}"),"from":{"node":id(101),"port":name},"to":{"node":id(102),"port":result}}));
    }
    let function = json!({"id":id(100),"name":"wide","kind":"pure","inputs":inputs,"outputs":outputs,
        "nodes":[intrinsic(101,"function-entry"),intrinsic(102,"function-outputs")],"edges":edges});
    let mut call = intrinsic(1, "call");
    call["config"] = json!({"function":id(100)});
    call["inputs"] = json!(literals);
    let doc = graph_with_functions(
        &package,
        vec![call],
        vec![],
        vec![
            json!({"id":"first","node":id(1),"port":"out0"}),
            json!({"id":"last","node":id(1),"port":"out255"}),
        ],
        vec![function],
    );
    let plan = compile(&doc, snapshot, &package);
    assert_eq!(
        outcomes(&plan),
        [
            FoldedValue::Value(Value::I64(0)),
            FoldedValue::Value(Value::Bool(true))
        ]
    );
    assert_eq!(plan.constants().len(), 256);
}
