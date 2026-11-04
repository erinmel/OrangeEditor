//! G2 evidence: a synthetic third-party package installed, updated, rebound, removed and
//! rolled back through public APIs only, as a downstream product would (04 "Installation,
//! activation and override", "Hot update/removal"; 12 §8; ADR-07).
//!
//! `acme.math` ships its own node `acme.math/triple` and a provider `fast` that also fills
//! the bundled slot `node:org.orange.math/add@1`. The host starts from the bundled registry,
//! installs version 1.0.0 from its manifest fixture, selects it for new bindings and authors
//! a graph; the graph compiles and folds through the package's kernels. Version 2.0.0 moves
//! new bindings only; the graph keeps its exact 1.0.0 lock until an explicit rebind. Removing
//! 1.0.0 leaves graphs still locked to it unavailable, with no fallback to the native math
//! provider, and rollback restores the previous generation.

use std::sync::{Arc, Mutex};

use orange_bundled::MathPackage;
use orange_compiler::{
    BindingRequest, CompileError, CompileOptions, EvaluationError, EvaluationOptions,
    KernelBinding, KernelFault, PureKernel, PureKernelResolver, PurePlan, ResolveError,
    compile_pure,
};
use orange_model::catalog::{
    Determinism, NodeDefinition, Ports, ProviderKey, RegistryBuilder, RegistrySnapshot,
};
use orange_model::document::{BodyRef, DefinitionRef, NodeKind, OutputSource, PortDescriptor};
use orange_model::id::NodeId;
use orange_model::rebind::{
    RebindOutcome, RebindRequest, RebindSelection, apply_rebind, plan_rebind,
};
use orange_model::signature::{project, semantic_key};
use orange_model::validation::{DiagnosticCode, HostOrigin, Severity};
use orange_model::{DocCommand, GraphDocument, Value, analyze, from_json, validate};
use orange_packages::{
    HOST_API_VERSION, MemoryJournal, PackageContents, PackageKey, PackageManifest, PackageState,
    PackageStore,
};
use serde_json::json;

const MANIFEST_1: &[u8] = include_bytes!("fixtures/acme.math/manifest-1.0.0.json");
const MANIFEST_2: &[u8] = include_bytes!("fixtures/acme.math/manifest-2.0.0.json");
const PACKAGE: &str = "acme.math";
const TRIPLE: &str = "acme.math/triple";
const ADD: &str = "org.orange.math/add";
const I64: &str = "core/i64@1";

fn digest(byte: char) -> String {
    format!("sha256:{}", byte.to_string().repeat(64))
}

fn dref(id: &str) -> DefinitionRef {
    DefinitionRef {
        id: id.parse().unwrap(),
        version: 1,
    }
}

fn node_id(n: u32) -> NodeId {
    format!("10000000-0000-4000-8000-{n:012x}").parse().unwrap()
}

/// The bundled registry a product ships: core intrinsics plus the native math package.
fn bundled() -> (MathPackage, Arc<RegistrySnapshot>) {
    let package = MathPackage::new(digest('b')).unwrap();
    let snapshot = package
        .register(RegistryBuilder::core(
            HostOrigin::new("0.0.0", digest('c')).unwrap(),
        ))
        .unwrap()
        .build()
        .unwrap();
    (package, snapshot)
}

/// The definition `acme.math/triple` ships: one i64 in, three times it out.
fn triple_definition() -> NodeDefinition {
    let ty: orange_model::id::TypeExpr = I64.parse().unwrap();
    NodeDefinition {
        definition: dref(TRIPLE),
        config_version: 1,
        kind: NodeKind::Pure,
        volatility: None,
        determinism: Determinism::Deterministic,
        suspends: None,
        type_params: vec![],
        ports: Ports::Declared(vec![
            PortDescriptor::data_in("value", ty.clone()),
            PortDescriptor::data_out("result", ty, OutputSource::Computed),
        ]),
        references: vec![],
        specializations: None,
        config_schema: None,
    }
}

/// A staged package version: the decoded manifest fixture and its shipped definition.
fn acme(manifest: &[u8]) -> PackageContents {
    PackageContents::new(
        PackageManifest::from_json(manifest).unwrap(),
        vec![triple_definition()],
    )
}

fn package_key(version: &str) -> PackageKey {
    PackageKey {
        id: PACKAGE.parse().unwrap(),
        version: version.parse().unwrap(),
    }
}

fn fast(version: &str, hex: char) -> ProviderKey {
    ProviderKey {
        package: PACKAGE.into(),
        version: version.into(),
        content_digest: format!("sha256:{}", format!("a{hex}").repeat(32)),
        id: "fast".into(),
    }
}

fn empty_graph() -> GraphDocument {
    from_json(
        &json!({"format":"orange.graph","format_version":1,
            "graph_id":"20000000-0000-4000-8000-000000000001","revision":0,
            "package_lock":[],"signatures":{},"nodes":[],"edges":[],"exports":[]})
        .to_string(),
    )
    .unwrap()
}

fn literal(value: i64) -> serde_json::Value {
    json!({"type": I64, "value": value.to_string()})
}

/// Authors one node as an editor would: the provider is the registry's new-binding default,
/// locked with its exact package version and digest, and the signature is projected from the
/// registered definition. Returns the provider the node was bound to.
fn author(
    document: &mut GraphDocument,
    registry: &RegistrySnapshot,
    n: u32,
    definition: &str,
    inputs: serde_json::Value,
) -> ProviderKey {
    let definition = dref(definition);
    let provider = registry
        .new_binding_default(&definition)
        .expect("the host selected a default for new bindings")
        .clone();
    let registered = registry.definition(&definition).unwrap();
    let Ports::Declared(ports) = registered.ports.clone() else {
        unreachable!("both definitions declare their ports")
    };
    let pins = if registered.type_params.is_empty() {
        json!({})
    } else {
        json!({"T": I64})
    };
    let args = serde_json::from_value(pins.clone()).unwrap();
    let snapshot = project(registered.kind, ports, &args);
    let key = semantic_key(&snapshot);
    let node = serde_json::from_value(json!({
        "id": node_id(n), "definition_id": definition.id, "definition_version": 1,
        "config_version": registered.config_version, "config": {}, "type_pins": pins,
        "signature": key, "inputs": inputs,
        "provider": {"package": provider.package, "version": provider.version, "id": provider.id}
    }))
    .unwrap();
    let lock = serde_json::from_value(json!({
        "id": provider.package, "version": provider.version,
        "content_digest": provider.content_digest
    }))
    .unwrap();
    let _ = document
        .apply([
            DocCommand::PutPackageLock(lock),
            DocCommand::PutSignature { key, snapshot },
            DocCommand::AddNode {
                body: BodyRef::Root,
                node,
            },
        ])
        .unwrap();
    provider
}

/// `add(20, 1) -> triple -> export`, every node bound through new-binding defaults.
fn author_graph(registry: &RegistrySnapshot) -> (GraphDocument, Vec<ProviderKey>) {
    let mut document = empty_graph();
    let bound = vec![
        author(
            &mut document,
            registry,
            1,
            ADD,
            json!({"a": literal(20), "b": literal(1)}),
        ),
        author(&mut document, registry, 2, TRIPLE, json!({})),
    ];
    let _ = document
        .apply([
            DocCommand::Connect {
                edge: serde_json::from_value(json!({
                    "id": "30000000-0000-4000-8000-000000000001",
                    "from": {"node": node_id(1), "port": "sum"},
                    "to": {"node": node_id(2), "port": "value"}
                }))
                .unwrap(),
            },
            DocCommand::UpsertExport(
                serde_json::from_value(json!({"id": "out", "node": node_id(2), "port": "result"}))
                    .unwrap(),
            ),
        ])
        .unwrap();
    (document, bound)
}

fn errors(document: &GraphDocument, registry: &RegistrySnapshot) -> Vec<(DiagnosticCode, String)> {
    validate(document, registry)
        .diagnostics
        .into_iter()
        .filter(|item| item.severity == Severity::Error)
        .map(|item| (item.code, item.message))
        .collect()
}

fn locked_versions(document: &GraphDocument) -> Vec<(String, String)> {
    document
        .package_lock()
        .iter()
        .map(|entry| (entry.id.clone(), entry.content_digest.clone()))
        .collect()
}

/// The package's own kernels, standing in for its worker: they resolve only an exact
/// `acme.math` provider published by the request's snapshot and record which one bound.
#[derive(Debug)]
struct AcmeKernel {
    triple: bool,
}

impl PureKernel for AcmeKernel {
    fn fold(&self, inputs: &[Value], outputs: &mut [Value]) -> Result<(), KernelFault> {
        const OVERFLOW: KernelFault = KernelFault {
            code: "core/Overflow",
            message_key: "acme.math/overflow",
            message: "The result does not fit.",
        };
        let result = match (self.triple, inputs) {
            (true, [Value::I64(value)]) => value.checked_mul(3),
            (false, [Value::I64(a), Value::I64(b)]) => a.checked_add(*b),
            _ => {
                return Err(KernelFault {
                    code: "core/InternalFault",
                    message_key: "acme.math/input-contract",
                    message: "Kernel inputs violate the resolved contract.",
                });
            }
        };
        outputs[0] = Value::I64(result.ok_or(OVERFLOW)?);
        Ok(())
    }
}

/// Host composition: package kernels for `acme.math` bindings, the bundled resolver for
/// everything else. A binding to an unavailable `acme.math` provider is never handed to the
/// bundled resolver.
struct Host {
    native: MathPackage,
    resolved: Mutex<Vec<ProviderKey>>,
}

impl PureKernelResolver for Host {
    fn resolve_binding(&self, request: &BindingRequest<'_>) -> Result<KernelBinding, ResolveError> {
        let Some(provider) = request.provider().filter(|key| key.package == PACKAGE) else {
            return self.native.resolve_binding(request);
        };
        if !request
            .snapshot()
            .provides(request.definition(), request.config_version(), provider)
        {
            return Err(ResolveError::Unavailable);
        }
        let port = |name: &str| name.parse().map_err(|_| ResolveError::Contract);
        let (triple, inputs, output) = match request.definition().id.as_str() {
            TRIPLE => (true, vec![port("value")?], port("result")?),
            ADD => (false, vec![port("a")?, port("b")?], port("sum")?),
            _ => return Err(ResolveError::Unavailable),
        };
        self.resolved.lock().unwrap().push(provider.clone());
        Ok(KernelBinding {
            inputs: inputs.into(),
            outputs: vec![output].into(),
            kernel: Arc::new(AcmeKernel { triple }),
        })
    }
}

impl Host {
    /// Analyses, compiles and folds `document`, returning the export and the exact
    /// providers whose kernels were bound.
    fn run(
        &self,
        document: &GraphDocument,
        registry: &Arc<RegistrySnapshot>,
    ) -> (PurePlan, Value, Vec<ProviderKey>) {
        let analysis = analyze(document, registry);
        assert!(
            analysis.is_ready_for_lowering(document, registry),
            "{:?}",
            analysis.report().diagnostics
        );
        self.resolved.lock().unwrap().clear();
        let plan = compile_pure(
            document,
            Arc::clone(registry),
            &analysis,
            self,
            CompileOptions::default(),
        )
        .unwrap();
        let value = plan
            .evaluate(document, registry, EvaluationOptions::default())
            .unwrap()
            .export(0)
            .unwrap()
            .unwrap()
            .outcome
            .unwrap()
            .clone();
        let mut resolved = std::mem::take(&mut *self.resolved.lock().unwrap());
        resolved.dedup();
        (plan, value, resolved)
    }
}

#[test]
fn third_party_package_installs_replaces_rebinds_and_rolls_back() {
    let (native, base) = bundled();
    let host = Host {
        native: native.clone(),
        resolved: Mutex::new(Vec::new()),
    };
    let (v1, v2) = (fast("1.0.0", '1'), fast("2.0.0", '2'));
    let mut store = PackageStore::new(Arc::clone(&base), HOST_API_VERSION, MemoryJournal::new());

    // 1. Install 1.0.0 from its manifest. Installing picks no default: the bundled native
    //    provider keeps nothing selected and acme gains nothing implicitly.
    let installed = store.install(acme(MANIFEST_1)).unwrap();
    assert!(installed.generation() > base.generation());
    assert_eq!(
        store.package(&package_key("1.0.0")).unwrap().state(),
        PackageState::Active
    );
    assert_eq!(installed.new_binding_default(&dref(ADD)), None);
    assert!(installed.provides(&dref(ADD), 1, native.provider_key()));
    assert!(installed.provides(&dref(ADD), 1, &v1));
    assert!(installed.provides(&dref(TRIPLE), 1, &v1));

    // 2. The user selects acme for new bindings of its node and of the bundled `add` slot.
    let _ = store.select_default(dref(ADD), v1.clone()).unwrap();
    let selected = store.select_default(dref(TRIPLE), v1.clone()).unwrap();

    // 3. A newly authored graph binds to the defaults, analyses clean, and compiles and folds
    //    through acme's kernels: (20 + 1) * 3.
    let (mut graph, bound) = author_graph(&selected);
    assert_eq!(bound, [v1.clone(), v1.clone()]);
    assert_eq!(errors(&graph, &selected), []);
    let (plan_v1, value, resolved) = host.run(&graph, &selected);
    assert_eq!(value, Value::I64(63));
    assert_eq!(resolved, std::slice::from_ref(&v1));
    // The bundled resolver alone never substitutes native math for the acme binding.
    let analysis = analyze(&graph, &selected);
    assert!(matches!(
        compile_pure(
            &graph,
            Arc::clone(&selected),
            &analysis,
            &native,
            CompileOptions::default()
        ),
        Err(CompileError::KernelUnavailable { node }) if node == node_id(1)
    ));

    // 4. Update to 2.0.0: both versions are published and defaults move to 2.0.0. The
    //    existing graph keeps its exact 1.0.0 lock, still validates and still binds 1.0.0
    //    code; its old plan is stale against the new generation.
    let updated = store
        .update(&package_key("1.0.0"), acme(MANIFEST_2))
        .unwrap();
    assert_eq!(updated.new_binding_default(&dref(ADD)), Some(&v2));
    assert_eq!(updated.new_binding_default(&dref(TRIPLE)), Some(&v2));
    assert_eq!(
        locked_versions(&graph),
        [(PACKAGE.into(), v1.content_digest.clone())]
    );
    assert_eq!(errors(&graph, &updated), []);
    assert_eq!(
        plan_v1
            .evaluate(&graph, &updated, EvaluationOptions::default())
            .unwrap_err(),
        EvaluationError::StaleRegistry
    );
    let (_, value, resolved) = host.run(&graph, &updated);
    assert_eq!((value, resolved), (Value::I64(63), vec![v1.clone()]));

    // 5. New bindings pick 2.0.0.
    let (fresh, bound) = author_graph(&updated);
    assert_eq!(bound, [v2.clone(), v2.clone()]);
    assert_eq!(errors(&fresh, &updated), []);

    // 6. An explicit rebind previews the affected nodes on a checked copy and moves the graph
    //    to 2.0.0 in one undoable transaction; the 1.0.0 lock entry stays.
    let pinned = graph.clone();
    let RebindOutcome::Ready(plan) = plan_rebind(
        &graph,
        &updated,
        RebindRequest {
            from: RebindSelection::Provider(v1.clone()),
            to: v2.clone(),
        },
    )
    .unwrap() else {
        panic!("the rebind to a compatible provider is accepted")
    };
    assert_eq!(plan.affected, [node_id(1), node_id(2)]);
    let revision = graph.revision();
    let _ = apply_rebind(&mut graph, &updated, plan).unwrap();
    assert_eq!(graph.revision(), revision + 1);
    assert_eq!(
        locked_versions(&graph),
        [
            (PACKAGE.into(), v1.content_digest.clone()),
            (PACKAGE.into(), v2.content_digest.clone())
        ]
    );
    let (_, value, resolved) = host.run(&graph, &updated);
    assert_eq!((value, resolved), (Value::I64(63), vec![v2.clone()]));

    // 7. Remove 1.0.0. The graph still locked to it reports E-PROVIDER-UNAVAILABLE for both
    //    nodes and cannot be lowered or compiled: nothing falls back to native math or 2.0.0.
    let removed = store.remove(&package_key("1.0.0")).unwrap();
    assert_eq!(
        store.package(&package_key("1.0.0")).unwrap().state(),
        PackageState::Removed
    );
    assert!(removed.provider(&v1).is_none());
    let unavailable: Vec<_> = validate(&pinned, &removed)
        .diagnostics
        .into_iter()
        .filter(|item| item.code == DiagnosticCode::ProviderUnavailable)
        .collect();
    assert_eq!(unavailable.len(), 2, "{unavailable:?}");
    assert_eq!(unavailable[0].code.as_str(), "E-PROVIDER-UNAVAILABLE");
    let report = validate(&pinned, &removed);
    assert!(!report.resolved.contains_key(&node_id(1)));
    assert!(!report.resolved.contains_key(&node_id(2)));
    let analysis = analyze(&pinned, &removed);
    assert!(!analysis.is_ready_for_lowering(&pinned, &removed));
    assert_eq!(
        compile_pure(
            &pinned,
            Arc::clone(&removed),
            &analysis,
            &host,
            CompileOptions::default()
        )
        .unwrap_err(),
        CompileError::ValidationIncomplete
    );
    assert_eq!(
        locked_versions(&pinned),
        [(PACKAGE.into(), v1.content_digest.clone())]
    );
    // The rebound graph and new bindings are unaffected.
    assert_eq!(errors(&graph, &removed), []);
    assert_eq!(removed.new_binding_default(&dref(ADD)), Some(&v2));

    // 8. Rollback restores the previous generation: 1.0.0 is active again and the pinned
    //    graph validates and runs on its original lock.
    let rolled_back = store.rollback().unwrap();
    assert!(Arc::ptr_eq(&rolled_back, &updated));
    assert_eq!(
        store.package(&package_key("1.0.0")).unwrap().state(),
        PackageState::Active
    );
    assert_eq!(errors(&pinned, &rolled_back), []);
    let (_, value, resolved) = host.run(&pinned, &rolled_back);
    assert_eq!((value, resolved), (Value::I64(63), vec![v1]));
}
