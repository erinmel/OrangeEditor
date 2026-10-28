use orange_bundled::{
    MathPackage, Operation, RANDOM_U64_DEFINITION, math_definitions, random_u64_definition,
};
use orange_compiler::{
    CompileOptions, EntropyBinding, EntropyKernel, EntropyKernelResolver, EntropySource,
    KernelBinding, KernelFault, PureKernel, PureKernelResolver, ResolveError, compile_data,
};
use orange_model::catalog::{RegistryBuilder, RegistrySnapshot};
use orange_model::validation::HostOrigin;
use orange_model::{GraphDocument, Value, analyze, from_json};
use serde_json::{Value as Json, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
pub fn id(n: u32) -> String {
    format!("10000000-0000-4000-8000-{n:012x}")
}
pub fn catalog() -> (MathPackage, Arc<RegistrySnapshot>) {
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
pub fn provider() -> Json {
    json!({"package":"org.orange.math","version":"0.0.0","id":"native-v1"})
}
pub fn literal(value: u64) -> Json {
    json!({"type":"core/u64@1","value":value.to_string()})
}
pub fn math(n: u32, op: &str, a: u64, b: u64) -> Json {
    json!({"id":id(n),"definition_id":format!("org.orange.math/{op}"),"definition_version":1,"config_version":1,
        "config":{"int_overflow":"wrapping"},"signature":op,"provider":provider(),"inputs":{"a":literal(a),"b":literal(b)}})
}
pub fn random(n: u32) -> Json {
    json!({"id":id(n),"definition_id":RANDOM_U64_DEFINITION,"definition_version":1,"config_version":1,
        "config":{},"signature":"random","provider":provider()})
}
pub fn edge(n: u32, source: u32, output: &str, target: u32, input: &str) -> Json {
    json!({"id":format!("30000000-0000-4000-8000-{n:012x}"),"from":{"node":id(source),"port":output},
        "to":{"node":id(target),"port":input}})
}
pub fn export(n: u32, port: &str) -> Json {
    json!({"id":format!("out-{n}"),"node":id(n),"port":port})
}
pub fn graph(
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
pub struct Counting {
    pub package: MathPackage,
    pub pure: AtomicUsize,
    pub entropy: AtomicUsize,
    pub folds: Arc<AtomicUsize>,
    pub draws: Arc<AtomicUsize>,
    pub reject: Option<orange_model::id::NodeId>,
    pub bad_entropy_abi: bool,
}
impl Counting {
    pub fn new(package: MathPackage) -> Self {
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
    fn resolve_binding(
        &self,
        request: &orange_compiler::BindingRequest<'_>,
    ) -> Result<KernelBinding, ResolveError> {
        self.pure.fetch_add(1, Ordering::Relaxed);
        if self.reject == Some(request.node()) {
            return Err(ResolveError::Unavailable);
        }
        let mut binding = self.package.resolve_binding(request)?;
        binding.kernel = Arc::new(CountPure {
            inner: binding.kernel,
            calls: self.folds.clone(),
        });
        Ok(binding)
    }
}
impl EntropyKernelResolver for Counting {
    fn resolve_entropy_binding(
        &self,
        request: &orange_compiler::BindingRequest<'_>,
    ) -> Result<EntropyBinding, ResolveError> {
        self.entropy.fetch_add(1, Ordering::Relaxed);
        if self.reject == Some(request.node()) {
            return Err(ResolveError::Unavailable);
        }
        let mut binding = self.package.resolve_entropy_binding(request)?;
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
pub fn compile(
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
