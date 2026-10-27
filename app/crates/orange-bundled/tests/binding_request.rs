//! The compiler's narrow binding request and its compatibility path for resolvers
//! that implement only the original whole-document method.
use orange_bundled::{MathPackage, math_definitions};
use orange_compiler::{
    BindingRequest, CompileOptions, FoldedValue, KernelBinding, PureKernelResolver, ResolveError,
    compile_pure,
};
use orange_model::catalog::{Ports, ProviderKey, RegistryBuilder, RegistrySnapshot};
use orange_model::document::{DefinitionRef, NodeInstance, NodeKind, SignatureSnapshot};
use orange_model::id::NodeId;
use orange_model::validation::HostOrigin;
use orange_model::{GraphDocument, Value, analyze, from_json};
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::sync::Arc;

const NODE: &str = "10000000-0000-4000-8000-000000000001";

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

fn graph(package: &MathPackage) -> GraphDocument {
    let key = package.provider_key();
    let Ports::Declared(ports) = math_definitions().unwrap().remove(0).ports else {
        panic!("expected declared ports");
    };
    let args = [("T".into(), "core/i64@1".parse().unwrap())].into();
    let signature = orange_model::signature::project(NodeKind::Pure, ports, &args);
    from_json(&json!({"format":"orange.graph","format_version":1,"graph_id":"20000000-0000-4000-8000-000000000001",
        "revision":0,"package_lock":[{"id":key.package,"version":key.version,"content_digest":key.content_digest}],
        "nodes":[{"id":NODE,"definition_id":"org.orange.math/add","definition_version":1,
            "config_version":1,"config":{},"provider":{"package":key.package,"version":key.version,"id":key.id},
            "signature":"add-i64","inputs":{"a":{"type":"core/i64@1","value":"20"},"b":{"type":"core/i64@1","value":"22"}}}],
        "signatures":{"add-i64":signature},"exports":[{"id":"answer","node":NODE,"port":"sum"}]
    }).to_string()).unwrap()
}

fn compile_answer(
    doc: &GraphDocument,
    snapshot: &Arc<RegistrySnapshot>,
    resolver: &impl PureKernelResolver,
) -> Value {
    let analysis = analyze(doc, snapshot);
    assert!(
        analysis.is_ready_for_lowering(doc, snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    let plan = compile_pure(
        doc,
        snapshot.clone(),
        &analysis,
        resolver,
        CompileOptions::default(),
    )
    .unwrap();
    let FoldedValue::Value(value) = &plan.constants()[plan.exports()[0].slot.index()] else {
        panic!("expected a folded value");
    };
    value.clone()
}

/// A downstream resolver written before the narrow request existed.
struct Legacy {
    package: MathPackage,
    calls: Cell<usize>,
}
impl PureKernelResolver for Legacy {
    fn resolve(
        &self,
        document: &GraphDocument,
        node: &NodeInstance,
        signature: &SignatureSnapshot,
        snapshot: Arc<RegistrySnapshot>,
    ) -> Result<KernelBinding, ResolveError> {
        self.calls.set(self.calls.get() + 1);
        PureKernelResolver::resolve(&self.package, document, node, signature, snapshot)
    }
}

#[test]
fn a_resolver_with_only_the_original_method_still_compiles_and_binds() {
    let (package, snapshot) = catalog();
    let doc = graph(&package);
    let legacy = Legacy {
        package,
        calls: Cell::new(0),
    };
    assert_eq!(compile_answer(&doc, &snapshot, &legacy), Value::I64(42));
    assert_eq!(legacy.calls.get(), 1);
}

type Seen = (
    NodeId,
    DefinitionRef,
    Option<ProviderKey>,
    u32,
    SignatureSnapshot,
);

/// Reads only the narrow request; the whole-document entry must not be used.
struct Narrow {
    package: MathPackage,
    seen: RefCell<Vec<Seen>>,
}
impl PureKernelResolver for Narrow {
    fn resolve(
        &self,
        _: &GraphDocument,
        _: &NodeInstance,
        _: &SignatureSnapshot,
        _: Arc<RegistrySnapshot>,
    ) -> Result<KernelBinding, ResolveError> {
        panic!("the compiler binds through the narrow request")
    }
    fn resolve_binding(&self, request: &BindingRequest<'_>) -> Result<KernelBinding, ResolveError> {
        self.seen.borrow_mut().push((
            request.node(),
            request.definition().clone(),
            request.provider().cloned(),
            request.config_version(),
            request.signature().clone(),
        ));
        self.package.resolve_binding(request)
    }
}

#[test]
fn the_narrow_request_carries_the_exact_locked_provider() {
    let (package, snapshot) = catalog();
    let doc = graph(&package);
    let exact = package.provider_key().clone();
    let narrow = Narrow {
        package,
        seen: RefCell::default(),
    };
    assert_eq!(compile_answer(&doc, &snapshot, &narrow), Value::I64(42));
    let analysis = analyze(&doc, &snapshot);
    let node: NodeId = NODE.parse().unwrap();
    let seen = narrow.seen.into_inner();
    assert_eq!(seen.len(), 1);
    let (id, definition, provider, config_version, signature) = &seen[0];
    assert_eq!(*id, node);
    assert_eq!(definition, &doc.root().nodes[&node].definition);
    assert_eq!(provider.as_ref(), Some(&exact));
    assert_eq!(*config_version, 1);
    assert_eq!(signature, &analysis.report().resolved[&node]);

    // A bound provider is exact only through the document lock.
    let mut unlocked = doc.root().nodes[&node].clone();
    unlocked.provider.as_mut().unwrap().version = "9.9.9".into();
    assert_eq!(
        BindingRequest::new(&doc, &unlocked, signature, snapshot).unwrap_err(),
        ResolveError::Unavailable
    );
}
