use super::support::*;
use orange_compiler::{
    CompileOptions, DataPlan, EntropyBinding, EntropyKernel, EntropyKernelResolver, EntropySource,
    KernelBinding, KernelFault, PureKernelResolver, ResolveError, compile_data,
};
use orange_model::catalog::RegistrySnapshot;
use orange_model::document::{NodeInstance, SignatureSnapshot};
use orange_model::{GraphDocument, Value, analyze, from_json, to_json};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
// Ordinary downstream host metadata exercises the open typed entropy ABI. It is
// a protocol fixture, not an additional shipped standard RNG/distribution family.
pub const FIXTURE: &str = "acme.frame/seeded-value";
pub const BOOL: &str = "core/bool@1";
pub const MAYBE: &str = "core/option@1<core/u64@1>";
pub const NESTED: &str = "core/option@1<core/option@1<core/u64@1>>";
pub const FLOAT: &str = "core/f64@1";
#[derive(Debug, Clone, Copy)]
enum Shape {
    Bool,
    Maybe,
    Nested,
    Float,
}
#[derive(Debug)]
struct FixtureKernel {
    shape: Shape,
    fault: bool,
    pub calls: Arc<AtomicUsize>,
}
impl EntropyKernel for FixtureKernel {
    fn evaluate(
        &self,
        _: &[Value],
        outputs: &mut [Value],
        source: &mut dyn EntropySource,
    ) -> Result<(), KernelFault> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.fault {
            return Err(KernelFault {
                code: "core/Domain",
                message_key: "core/test-domain",
                message: "The operation failed.",
            });
        }
        let word = source.next_u64()?;
        outputs[0] = match self.shape {
            Shape::Bool => Value::Bool(word & 1 != 0),
            Shape::Maybe => {
                if word & 1 != 0 {
                    Value::None
                } else {
                    Value::U64(word)
                }
            }
            Shape::Nested => {
                if word & 1 == 0 {
                    Value::None
                } else {
                    Value::Some(Arc::new(if word & 2 == 0 {
                        Value::None
                    } else {
                        Value::U64(word)
                    }))
                }
            }
            Shape::Float => Value::F64(f64::from_bits(word)),
        };
        Ok(())
    }
}
pub struct FixtureResolver {
    pub base: Counting,
    pub key: orange_model::catalog::ProviderKey,
    pub calls: Arc<AtomicUsize>,
}
impl PureKernelResolver for FixtureResolver {
    fn resolve(
        &self,
        doc: &GraphDocument,
        node: &NodeInstance,
        sig: &SignatureSnapshot,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Result<KernelBinding, ResolveError> {
        self.base.resolve(doc, node, sig, snapshot)
    }
}
impl EntropyKernelResolver for FixtureResolver {
    fn resolve_entropy(
        &self,
        doc: &GraphDocument,
        node: &NodeInstance,
        sig: &SignatureSnapshot,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Result<EntropyBinding, ResolveError> {
        if node.definition.id.as_str() != FIXTURE {
            return self.base.resolve_entropy(doc, node, sig, snapshot);
        }
        if node
            .provider
            .as_ref()
            .and_then(|p| orange_model::catalog::ProviderKey::of(doc, p))
            .as_ref()
            != Some(&self.key)
            || !snapshot.provides(&node.definition, node.config_version, &self.key)
        {
            return Err(ResolveError::Unavailable);
        }
        let shape = match sig.ports[0].ty.as_ref().unwrap().as_str() {
            BOOL => Shape::Bool,
            MAYBE => Shape::Maybe,
            NESTED => Shape::Nested,
            FLOAT => Shape::Float,
            _ => return Err(ResolveError::Contract),
        };
        Ok(EntropyBinding {
            inputs: Box::new([]),
            outputs: vec!["value".parse().unwrap()].into(),
            kernel: Arc::new(FixtureKernel {
                shape,
                fault: node
                    .config
                    .get("fault")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                calls: self.calls.clone(),
            }),
        })
    }
}
pub fn fixture_node(n: u32, ty: &str, fault: bool) -> serde_json::Value {
    json!({"id":id(n),"definition_id":FIXTURE,"definition_version":1,"config_version":1,
        "config":{"fault":fault},"type_pins":{"T":ty},"signature":ty,
        "provider":{"package":"acme.frame","version":"1.0.0","id":"native-fixture"}})
}
pub fn fixture(
    nodes: Vec<serde_json::Value>,
    edges: Vec<serde_json::Value>,
    exports: Vec<serde_json::Value>,
) -> (GraphDocument, Arc<RegistrySnapshot>, FixtureResolver) {
    use orange_model::catalog::{
        Determinism, NodeDefinition, Ports, ProviderDescriptor, ProviderKey, RegistryBuilder,
        TypeArgs, Volatility,
    };
    use orange_model::document::{
        DefinitionRef, NodeKind, OutputSource, PortDescriptor, TypeParam,
    };
    let (package, original) = catalog();
    let definition = NodeDefinition {
        definition: DefinitionRef {
            id: FIXTURE.parse().unwrap(),
            version: 1,
        },
        config_version: 1,
        kind: NodeKind::Volatile,
        volatility: Some(Volatility::Entropy),
        determinism: Determinism::Seeded,
        suspends: None,
        type_params: vec![TypeParam {
            name: "T".into(),
            constraints: vec![],
            extensions: Default::default(),
        }],
        ports: Ports::Declared(vec![PortDescriptor::data_out(
            "value",
            "$T".parse().unwrap(),
            OutputSource::Computed,
        )]),
        references: vec![],
        specializations: None,
        config_schema: None,
    };
    let key = ProviderKey {
        package: "acme.frame".into(),
        version: "1.0.0".into(),
        content_digest: format!("sha256:{}", "b".repeat(64)),
        id: "native-fixture".into(),
    };
    let args: Vec<TypeArgs> = [BOOL, MAYBE, NESTED, FLOAT]
        .into_iter()
        .map(|ty| [("T".into(), ty.parse().unwrap())].into())
        .collect();
    let provider = ProviderDescriptor {
        key: key.clone(),
        implements: [(definition.definition.clone(), 1)].into(),
        specializations: [(definition.definition.clone(), args.clone())].into(),
    };
    let snapshot = package
        .register(RegistryBuilder::core(original.origin().clone()))
        .unwrap()
        .definition(definition.clone())
        .provider(provider)
        .build()
        .unwrap();
    let mut raw: serde_json::Value =
        serde_json::from_str(&to_json(&graph(&package, nodes, edges, exports)).unwrap()).unwrap();
    raw["package_lock"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":key.package,"version":key.version,"content_digest":key.content_digest}));
    let Ports::Declared(ports) = definition.ports else {
        unreachable!()
    };
    for (ty, args) in [BOOL, MAYBE, NESTED, FLOAT].into_iter().zip(args) {
        raw["signatures"][ty] = serde_json::to_value(orange_model::signature::project(
            NodeKind::Volatile,
            ports.clone(),
            &args,
        ))
        .unwrap();
    }
    (
        from_json(&raw.to_string()).unwrap(),
        snapshot,
        FixtureResolver {
            base: Counting::new(package),
            key,
            calls: Arc::new(AtomicUsize::new(0)),
        },
    )
}
pub fn fixture_plan(
    doc: &GraphDocument,
    snapshot: &Arc<RegistrySnapshot>,
    resolver: &FixtureResolver,
) -> Arc<DataPlan> {
    let analysis = analyze(doc, snapshot);
    assert!(
        analysis.is_ready_for_lowering(doc, snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    Arc::new(
        compile_data(
            doc,
            snapshot.clone(),
            &analysis,
            resolver,
            CompileOptions::default(),
        )
        .unwrap(),
    )
}
