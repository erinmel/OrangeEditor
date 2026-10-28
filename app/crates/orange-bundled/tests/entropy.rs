use orange_bundled::{
    BindingError, MathPackage, RANDOM_U64_DEFINITION, math_definitions, random_u64_definition,
};
use orange_compiler::{
    CompileError, CompileOptions, EntropySource, KernelFault, ResolveError, compile_pure,
};
use orange_compiler::{ResolveEntropyInDocument, ResolveInDocument};
use orange_model::catalog::{RegistryBuilder, RegistrySnapshot};
use orange_model::document::{NodeKind, SignatureSnapshot};
use orange_model::validation::HostOrigin;
use orange_model::{GraphDocument, Value, analyze, from_json};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::Arc;

const NODE: &str = "10000000-0000-4000-8000-000000000001";
fn catalog(digit: &str) -> (MathPackage, Arc<RegistrySnapshot>) {
    let digest = format!("sha256:{}", digit.repeat(64));
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
fn signature() -> SignatureSnapshot {
    let orange_model::catalog::Ports::Declared(ports) = random_u64_definition().unwrap().ports
    else {
        unreachable!()
    };
    orange_model::signature::project(NodeKind::Volatile, ports, &Default::default())
}
fn graph(package: &MathPackage) -> GraphDocument {
    let provider = package.provider_key();
    from_json(&json!({"format":"orange.graph","format_version":1,"graph_id":"20000000-0000-4000-8000-000000000001","revision":0,
        "package_lock":[{"id":provider.package,"version":provider.version,"content_digest":provider.content_digest}],
        "signatures":{"random":signature()},"nodes":[{"id":NODE,"definition_id":RANDOM_U64_DEFINITION,
            "definition_version":1,"config_version":1,"config":{},"signature":"random",
            "provider":{"package":provider.package,"version":provider.version,"id":provider.id}}],"edges":[],
        "exports":[{"id":"sample","node":NODE,"port":"value"}]}).to_string()).unwrap()
}
const SOURCE_FAULT: KernelFault = KernelFault {
    code: "core/LimitExceeded",
    message_key: "core/entropy-stream-exhausted",
    message: "The entropy stream is exhausted.",
};
struct Source {
    values: VecDeque<u64>,
    calls: usize,
}
impl EntropySource for Source {
    fn next_u64(&mut self) -> Result<u64, KernelFault> {
        self.calls += 1;
        self.values.pop_front().ok_or(SOURCE_FAULT)
    }
}

#[test]
fn product_entropy_graph_validates_and_uses_only_the_distinct_entropy_abi() {
    let (package, snapshot) = catalog("a");
    let doc = graph(&package);
    let analysis = analyze(&doc, &snapshot);
    assert!(
        analysis.is_ready_for_lowering(&doc, &snapshot),
        "{:?}",
        analysis.report().diagnostics
    );
    let node = &doc.root().nodes[&NODE.parse::<orange_model::id::NodeId>().unwrap()];
    let sig = &analysis.report().resolved[&node.id];
    assert!(matches!(
        compile_pure(
            &doc,
            snapshot.clone(),
            &analysis,
            &package,
            CompileOptions::default()
        ),
        Err(CompileError::UnsupportedNode { .. })
    ));
    assert!(ResolveInDocument::resolve(&package, &doc, node, sig, snapshot.clone()).is_err());
    let binding = package.resolve_entropy(&doc, node, sig, snapshot).unwrap();
    assert!(binding.inputs.is_empty());
    assert_eq!(binding.outputs.as_ref(), &["value".parse().unwrap()]);
    let mut source = Source {
        values: [0, u64::MAX].into(),
        calls: 0,
    };
    for expected in [0, u64::MAX] {
        let mut outputs = [Value::Unit];
        binding
            .kernel
            .evaluate(&[], &mut outputs, &mut source)
            .unwrap();
        assert_eq!(outputs, [Value::U64(expected)]);
    }
    assert_eq!(source.calls, 2);
}

#[test]
fn malformed_abi_does_not_draw_and_stream_failure_does_not_publish_a_partial_value() {
    let (package, snapshot) = catalog("a");
    let doc = graph(&package);
    let node = &doc.root().nodes[&NODE.parse::<orange_model::id::NodeId>().unwrap()];
    let binding = package
        .resolve_entropy(&doc, node, &signature(), snapshot)
        .unwrap();
    let mut source = Source {
        values: VecDeque::new(),
        calls: 0,
    };
    assert!(
        binding
            .kernel
            .evaluate(&[Value::Unit], &mut [Value::Unit], &mut source)
            .is_err()
    );
    assert!(binding.kernel.evaluate(&[], &mut [], &mut source).is_err());
    assert!(
        binding
            .kernel
            .evaluate(&[], &mut [Value::Unit, Value::Unit], &mut source)
            .is_err()
    );
    assert_eq!(source.calls, 0);
    let mut outputs = [Value::Unit];
    assert_eq!(
        binding.kernel.evaluate(&[], &mut outputs, &mut source),
        Err(SOURCE_FAULT)
    );
    assert_eq!(outputs, [Value::Unit]);
    assert_eq!(source.calls, 1);
}

#[test]
fn native_entropy_requires_exact_provider_version_config_and_semantic_signature() {
    let (package, snapshot) = catalog("a");
    let doc = graph(&package);
    let node = &doc.root().nodes[&NODE.parse::<orange_model::id::NodeId>().unwrap()];
    let mut provider = package.provider_key().clone();
    provider.content_digest = format!("sha256:{}", "b".repeat(64));
    assert_eq!(
        package
            .bind_entropy(
                snapshot.clone(),
                &provider,
                &node.definition,
                1,
                &node.config
            )
            .unwrap_err(),
        BindingError::Unavailable
    );
    assert_eq!(
        package
            .bind_entropy(
                snapshot.clone(),
                package.provider_key(),
                &node.definition,
                2,
                &node.config
            )
            .unwrap_err(),
        BindingError::Unavailable
    );
    let mut changed = node.definition.clone();
    changed.version = 2;
    assert_eq!(
        package
            .bind_entropy(
                snapshot.clone(),
                package.provider_key(),
                &changed,
                1,
                &node.config
            )
            .unwrap_err(),
        BindingError::Unavailable
    );
    let mut config = node.config.clone();
    config.insert("seed".into(), json!(123));
    assert_eq!(
        package
            .bind_entropy(
                snapshot.clone(),
                package.provider_key(),
                &node.definition,
                1,
                &config
            )
            .unwrap_err(),
        BindingError::InvalidConfig
    );
    let mut forged = signature();
    forged.ports[0].ty = Some("core/i64@1".parse().unwrap());
    assert_eq!(
        package
            .resolve_entropy(&doc, node, &forged, snapshot)
            .unwrap_err(),
        ResolveError::Contract
    );
}

fn changed_metadata(
    package: &MathPackage,
    original: &RegistrySnapshot,
    presentation: bool,
) -> Arc<RegistrySnapshot> {
    let mut definition = random_u64_definition().unwrap();
    let orange_model::catalog::Ports::Declared(ports) = &mut definition.ports else {
        unreachable!()
    };
    if presentation {
        ports[0].extensions.insert(
            "label".into(),
            json!({"key":"org.orange.math/random-value","fallback":"Valor aleatorio"}),
        );
        ports[0]
            .extensions
            .insert("group".into(), json!("Grupo traducido"));
    } else {
        ports[0].ty = Some("core/i64@1".parse().unwrap());
    }
    let mut builder = RegistryBuilder::core(original.origin().clone());
    for definition in math_definitions().unwrap().into_iter().chain([definition]) {
        builder = builder.definition(definition);
    }
    builder
        .provider(original.provider(package.provider_key()).unwrap().clone())
        .build()
        .unwrap()
}

#[test]
fn presentation_does_not_change_native_binding_but_semantic_metadata_drift_does() {
    let (package, original) = catalog("a");
    let doc = graph(&package);
    let node = &doc.root().nodes[&NODE.parse::<orange_model::id::NodeId>().unwrap()];
    let localized = changed_metadata(&package, &original, true);
    assert!(
        package
            .resolve_entropy(&doc, node, &signature(), localized)
            .is_ok()
    );
    let drifted = changed_metadata(&package, &original, false);
    assert_eq!(
        package
            .bind_entropy(
                drifted,
                package.provider_key(),
                &node.definition,
                1,
                &node.config
            )
            .unwrap_err(),
        BindingError::Unavailable
    );
}

#[test]
fn a_bound_native_entropy_kernel_retains_its_provider_snapshot_after_update() {
    let (package, snapshot) = catalog("a");
    let doc = graph(&package);
    let node = &doc.root().nodes[&NODE.parse::<orange_model::id::NodeId>().unwrap()];
    let bound = package
        .bind_entropy(
            snapshot.clone(),
            package.provider_key(),
            &node.definition,
            1,
            &node.config,
        )
        .unwrap();
    assert!(Arc::ptr_eq(bound.snapshot(), &snapshot));
    assert_eq!(bound.provider(), package.provider_key());
    let (_, replacement) = catalog("b");
    assert_eq!(
        package
            .bind_entropy(
                replacement,
                package.provider_key(),
                &node.definition,
                1,
                &node.config
            )
            .unwrap_err(),
        BindingError::Unavailable
    );
    drop(snapshot);
    let mut source = Source {
        values: [42].into(),
        calls: 0,
    };
    assert_eq!(bound.evaluate(&mut source), Ok(42));
    assert!(bound.snapshot().definition(&node.definition).is_some());
}
