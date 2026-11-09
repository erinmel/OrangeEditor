//! K02c: a `.orangeplug` archive carrying the Rust probe component is verified, installed
//! and bound, and a graph mixing bundled math with the package's guest node compiles and
//! folds through the guest. Guest node errors and traps keep their producer.
use std::sync::Arc;

use orange_bundled::MathPackage;
use orange_compiler::{CompileError, CompileOptions, FoldedValue, PurePlan, compile_pure};
use orange_guest::{Grants, GuestEngine, GuestLimits, GuestLoadError, GuestPackage, WithGuests};
use orange_model::binding::{NewNode, new_node_edit};
use orange_model::catalog::{NodeDefinition, ProviderKey, RegistryBuilder, RegistrySnapshot};
use orange_model::document::DefinitionRef;
use orange_model::id::NodeId;
use orange_model::validation::HostOrigin;
use orange_model::{DocCommand, GraphDocument, Value, analyze};
use orange_packages::{
    ArchiveFiles, ArchiveLimits, HOST_API_VERSION, MemoryJournal, MemorySink, PackageStore,
    VerifiedArchive, read_archive,
};
use serde_json::{Value as Json, json};

const I64: &str = "core/i64@1";
const GUEST_ADD: &str = "org.example.probe/add";
const GUEST_TRAP: &str = "org.example.probe/trap";
const MATH_ADD: &str = "org.orange.math/add";

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
fn literal(value: i64) -> Json {
    json!({"type": I64, "value": value.to_string()})
}

fn manifest(world: &str) -> Json {
    json!({
        "format": "orange.plugin", "format_version": 1,
        "id": "org.example.probe", "version": "1.0.0", "publisher": "org.example",
        "host_api": {"major": 1, "min_minor": 0, "max_minor": 0},
        "execution": {"kind": "wasm-component", "entry": "component.wasm", "world": world},
        "dependencies": [],
        "contributions": {
            "nodes": [GUEST_ADD, GUEST_TRAP],
            "providers": [
                {"slot": format!("node:{GUEST_ADD}@1"), "id": "guest"},
                {"slot": format!("node:{GUEST_TRAP}@1"), "id": "guest"}
            ]
        },
        "permissions": [],
        "integrity": {"algorithm": "sha256", "content_digest": "0".repeat(64)},
        "license": "MIT"
    })
}

/// Packs the probe component with `manifest`, then reads it back as a host would.
fn archive(mut manifest: Json) -> (VerifiedArchive, MemorySink) {
    // The committed fixture, or the freshly built component CI names.
    let component = crate::plugin::probe_bytes();
    let files =
        ArchiveFiles::new(vec![("component.wasm".parse().unwrap(), &component[..])]).unwrap();
    let digest = files
        .content_digest(&serde_json::to_vec(&manifest).unwrap())
        .unwrap();
    manifest["integrity"]["content_digest"] = json!(digest.strip_prefix("sha256:").unwrap());
    let bytes = files.encode(&serde_json::to_vec(&manifest).unwrap());
    let mut sink = MemorySink::new();
    let verified = read_archive(&bytes[..], &ArchiveLimits::default(), &mut sink).unwrap();
    (verified, sink)
}

fn definition(id: &str, inputs: &[&str]) -> NodeDefinition {
    let mut ports: Vec<Json> = inputs
        .iter()
        .map(|name| json!({"port_id": name, "direction": "input", "kind": "data", "type": I64}))
        .collect();
    ports.push(json!({"port_id": "result", "direction": "output", "kind": "data", "type": I64, "source": "computed"}));
    NodeDefinition::from_json(
        &json!({"definition_id": id, "definition_version": 1, "config_version": 1,
            "kind": "pure", "determinism": "deterministic", "ports": ports})
        .to_string(),
    )
    .unwrap()
}

fn author(
    document: &mut GraphDocument,
    registry: &RegistrySnapshot,
    n: u32,
    id: &str,
    inputs: Json,
) -> ProviderKey {
    let mut node = NewNode::new(node_id(n), dref(id));
    node.inputs = serde_json::from_value(inputs).unwrap();
    if !registry
        .definition(&node.definition)
        .unwrap()
        .type_params
        .is_empty()
    {
        node.type_pins.insert("T".into(), I64.parse().unwrap());
    }
    let edit = new_node_edit(document, registry, node).unwrap();
    let _ = document.apply(edit.commands).unwrap();
    edit.provider
}

fn compile(
    document: &GraphDocument,
    registry: &Arc<RegistrySnapshot>,
    resolver: &WithGuests<MathPackage>,
) -> PurePlan {
    let analysis = analyze(document, registry);
    assert!(
        analysis.is_ready_for_lowering(document, registry),
        "{:?}",
        analysis.report().diagnostics
    );
    compile_pure(
        document,
        Arc::clone(registry),
        &analysis,
        resolver,
        CompileOptions::default(),
    )
    .unwrap()
}

#[test]
fn an_archived_guest_node_compiles_beside_bundled_math_and_keeps_fault_sources() {
    let native = MathPackage::new(digest('b')).unwrap();
    let base = native
        .register(RegistryBuilder::core(
            HostOrigin::new("0.0.0", digest('c')).unwrap(),
        ))
        .unwrap()
        .build()
        .unwrap();
    let engine = GuestEngine::new(GuestLimits::default()).unwrap();
    let (verified, files) = archive(manifest("orange:expansion/plugin@1.0.0"));
    let guest =
        GuestPackage::from_archive(&engine, &verified, &files, "guest", Grants { log: true })
            .unwrap();
    assert_eq!(guest.provider().package, "org.example.probe");
    assert_eq!(guest.provider().content_digest, verified.content_digest());

    let mut store = PackageStore::new(Arc::clone(&base), HOST_API_VERSION, MemoryJournal::new());
    let _ = store
        .install(verified.into_contents(vec![
            definition(GUEST_ADD, &["a", "b"]),
            definition(GUEST_TRAP, &[]),
        ]))
        .unwrap();
    // A product ships bundled math selected for new bindings; the package gets its own.
    let _ = store
        .select_default(dref(MATH_ADD), native.provider_key().clone())
        .unwrap();
    let _ = store
        .select_default(dref(GUEST_ADD), guest.provider().clone())
        .unwrap();
    let registry = store
        .select_default(dref(GUEST_TRAP), guest.provider().clone())
        .unwrap();
    let mut resolver = WithGuests::new(native.clone());
    resolver.add(guest.clone());

    // bundled add(20, 1) -> guest add(sum, 21) -> export.
    let mut document = GraphDocument::new("20000000-0000-4000-8000-000000000001".parse().unwrap());
    let math = author(
        &mut document,
        &registry,
        1,
        MATH_ADD,
        json!({"a": literal(20), "b": literal(1)}),
    );
    let bound = author(
        &mut document,
        &registry,
        2,
        GUEST_ADD,
        json!({"b": literal(21)}),
    );
    assert_eq!(&math, native.provider_key());
    assert_eq!(&bound, guest.provider());
    author(
        &mut document,
        &registry,
        3,
        GUEST_ADD,
        json!({"a": literal(i64::MAX), "b": literal(1)}),
    );
    author(&mut document, &registry, 4, GUEST_TRAP, json!({}));
    let _ = document
        .apply([
            DocCommand::Connect {
                edge: serde_json::from_value(json!({
                    "id": "30000000-0000-4000-8000-000000000001",
                    "from": {"node": node_id(1), "port": "sum"},
                    "to": {"node": node_id(2), "port": "a"}
                }))
                .unwrap(),
            },
            DocCommand::UpsertExport(
                serde_json::from_value(json!({"id": "sum", "node": node_id(2), "port": "result"}))
                    .unwrap(),
            ),
            DocCommand::UpsertExport(
                serde_json::from_value(
                    json!({"id": "overflow", "node": node_id(3), "port": "result"}),
                )
                .unwrap(),
            ),
            DocCommand::UpsertExport(
                serde_json::from_value(json!({"id": "trap", "node": node_id(4), "port": "result"}))
                    .unwrap(),
            ),
        ])
        .unwrap();
    let plan = compile(&document, &registry, &resolver);
    let constant = |n: usize| plan.constants()[plan.exports()[n].slot.index()].clone();
    assert_eq!(constant(0), FoldedValue::Value(Value::I64(42)));
    // The guest's own namespaced error identifiers survive as the fault.
    let FoldedValue::Fault(overflow) = constant(1) else {
        panic!("guest overflow must fault")
    };
    assert_eq!(overflow.source, node_id(3));
    assert_eq!(overflow.fault.code, "core/NumericOverflow");
    assert_eq!(overflow.fault.message_key, "org.example.probe/failed");
    // A trap becomes a package fault of the trapping node only.
    let FoldedValue::Fault(trap) = constant(2) else {
        panic!("guest trap must fault")
    };
    assert_eq!(trap.source, node_id(4));
    assert_eq!(trap.fault.code, "core/PackageFault");
    assert_eq!(trap.fault.message_key, "core/guest-trap");

    // The bundled resolver alone never stands in for the package's provider.
    let analysis = analyze(&document, &registry);
    assert!(matches!(
        compile_pure(
            &document,
            Arc::clone(&registry),
            &analysis,
            &native,
            CompileOptions::default()
        ),
        Err(CompileError::KernelUnavailable { .. })
    ));
}

#[test]
fn loading_refuses_other_worlds_and_undeclared_providers() {
    let engine = GuestEngine::new(GuestLimits::default()).unwrap();
    let (verified, files) = archive(manifest("orange:expansion/plugin@1.0.0"));
    assert_eq!(
        GuestPackage::from_archive(&engine, &verified, &files, "other", Grants::default())
            .unwrap_err(),
        GuestLoadError::Provider
    );
    let (verified, files) = archive(manifest("orange:expansion/panel@1.0.0"));
    assert_eq!(
        GuestPackage::from_archive(&engine, &verified, &files, "guest", Grants::default())
            .unwrap_err(),
        GuestLoadError::World
    );
    let (verified, files) = archive(manifest("orange:expansion/plugin@2.0.0"));
    assert_eq!(
        GuestPackage::from_archive(&engine, &verified, &files, "guest", Grants::default())
            .unwrap_err(),
        GuestLoadError::World
    );
}
