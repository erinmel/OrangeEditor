//! Generic-node fixtures for the inference tests: `org.test/add<T: num>(a, b) -> sum` and a
//! non-generic `org.test/source` with an `i64` output `value`.

use std::sync::{Arc, LazyLock};

use orange_model::catalog::{
    NodeDefinition, Ports, ProviderDescriptor, ProviderKey, RegistryBuilder, RegistrySnapshot,
    TypeArgs,
};
use orange_model::conversion::{
    ConnectionOutcome, ConnectionPlan, ConnectionRequest, ConverterKey, ConverterProvider,
    ConverterRegistry, ConverterRegistryBuilder, PackageProvider, plan_connection,
};
use orange_model::document::{DefinitionRef, NodeKind, OutputSource, PortDescriptor, TypeParam};
use orange_model::types::TypeRegistry;
use orange_model::validation::HostOrigin;
use orange_model::{GraphDocument, from_json};
use serde_json::{Value as Json, json};

use super::{edge_id, id_source, minimal_document, node_id, port as port_ref};

pub const ADD: &str = "org.test/add";
const SOURCE: &str = "org.test/source";

pub fn port(port_id: &str, direction: &str, ty: &str) -> PortDescriptor {
    serde_json::from_value(
        json!({"port_id": port_id, "direction": direction, "kind": "data", "type": ty}),
    )
    .unwrap()
}

/// The `org.test/add` definition version 1.
pub fn add_definition() -> DefinitionRef {
    DefinitionRef {
        id: ADD.parse().unwrap(),
        version: 1,
    }
}

/// Argument sets binding `T` to each of `types`.
pub fn only(types: &[&str]) -> Vec<TypeArgs> {
    types
        .iter()
        .map(|ty| [("T".to_owned(), ty.parse().unwrap())].into())
        .collect()
}

pub fn origin() -> HostOrigin {
    HostOrigin::new("1.0.0", format!("sha256:{}", "0".repeat(64))).unwrap()
}

/// `org.test/add<T: num>(a: T, b: T) -> sum: T` as a registry builder over `converters`,
/// optionally limited to listed specializations.
pub fn registry_builder(
    converters: Arc<ConverterRegistry>,
    specializations: Option<Vec<&str>>,
) -> RegistryBuilder {
    RegistryBuilder::core(origin())
        .converters(converters)
        .definition(NodeDefinition {
            definition: add_definition(),
            config_version: 1,
            kind: NodeKind::Pure,
            suspends: None,
            type_params: vec![
                serde_json::from_value::<TypeParam>(json!({"name": "T", "constraints": ["num"]}))
                    .unwrap(),
            ],
            ports: Ports::Declared(vec![
                port("a", "input", "$T"),
                port("b", "input", "$T"),
                PortDescriptor::data_out("sum", "$T".parse().unwrap(), OutputSource::Computed),
            ]),
            references: Vec::new(),
            specializations: specializations.map(|types| only(&types)),
            config_schema: None,
        })
}

/// The registry over `converters` with `org.test/add`; a new snapshot on every call.
pub fn registry_with(
    converters: Arc<ConverterRegistry>,
    specializations: Option<Vec<&str>>,
) -> Arc<RegistrySnapshot> {
    registry_builder(converters, specializations)
        .build()
        .unwrap()
}

/// Core converters, with `org.test/add` optionally limited to listed specializations. The
/// unrestricted registry is one shared host snapshot, so plans made with it stay current.
pub fn registry(specializations: Option<Vec<&str>>) -> Arc<RegistrySnapshot> {
    static HOST: LazyLock<Arc<RegistrySnapshot>> =
        LazyLock::new(|| registry_with(core_converters(), None));
    match specializations {
        None => Arc::clone(&HOST),
        Some(_) => registry_with(core_converters(), specializations),
    }
}

/// The shared host snapshot over the package converters of [`package_converters`].
pub fn package_registry() -> Arc<RegistrySnapshot> {
    static HOST: LazyLock<Arc<RegistrySnapshot>> =
        LazyLock::new(|| registry_with(package_converters(), None));
    Arc::clone(&HOST)
}

pub fn add(n: u8, extra: Json) -> Json {
    let mut node = json!({"id": node_id(n), "definition_id": ADD, "definition_version": 1,
        "config_version": 1, "config": {}});
    node.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    node
}

/// A non-generic node whose snapshot declares an `i64` output `value`.
pub fn source(n: u8) -> Json {
    json!({"id": node_id(n), "definition_id": SOURCE, "definition_version": 1,
        "config_version": 1, "config": {}, "signature": "s-source"})
}

pub fn data_edge(n: u8, from: (u8, &str), to: (u8, &str)) -> Json {
    json!({"id": edge_id(n), "from": {"node": node_id(from.0), "port": from.1},
        "to": {"node": node_id(to.0), "port": to.1}})
}

pub fn document(nodes: Vec<Json>, edges: Vec<Json>) -> GraphDocument {
    from_json(&minimal_document(json!({
        "signatures": signatures(),
        "nodes": nodes,
        "edges": edges,
    })))
    .unwrap()
}

/// A non-generic node with a single data port `value` of type `core/<ty>@1`: an output
/// (`typed_source`) or an input (`typed_sink`). Types must be one of `TYPED`.
pub const TYPED: [&str; 3] = ["i32", "i64", "f64"];

pub fn typed_source(n: u8, ty: &str) -> Json {
    json!({"id": node_id(n), "definition_id": format!("org.test/out-{ty}"), "definition_version": 1,
        "config_version": 1, "config": {}, "signature": format!("s-out-{ty}")})
}

pub fn typed_sink(n: u8, ty: &str, extra: Json) -> Json {
    let mut node = json!({"id": node_id(n), "definition_id": format!("org.test/in-{ty}"),
        "definition_version": 1, "config_version": 1, "config": {}, "signature": format!("s-in-{ty}")});
    node.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    node
}

fn typed_signatures() -> Json {
    let mut signatures = serde_json::Map::new();
    for ty in TYPED {
        for (key, direction) in [("out", "output"), ("in", "input")] {
            signatures.insert(
                format!("s-{key}-{ty}"),
                json!({"kind": "pure", "ports": [
                    {"port_id": "value", "direction": direction, "kind": "data",
                     "type": format!("core/{ty}@1")}
                ]}),
            );
        }
    }
    Json::Object(signatures)
}

fn signatures() -> Json {
    let mut signatures = typed_signatures();
    signatures["s-source"] = json!({"kind": "pure", "ports": [
        {"port_id": "value", "direction": "output", "kind": "data", "type": "core/i64@1"}
    ]});
    signatures
}

/// Core converters over the core type registry.
pub fn core_converters() -> Arc<ConverterRegistry> {
    builder().build()
}

/// Core converters plus a package implementation of `i32 → i64`, set as that entry's
/// new-binding default.
pub fn package_converters() -> Arc<ConverterRegistry> {
    let mut builder = builder();
    let key = ConverterKey {
        id: "core/i32-to-i64".parse().unwrap(),
        contract_version: 1,
    };
    let provider = PackageProvider {
        package: "org.test".to_owned(),
        version: "1.0.0".to_owned(),
        content_digest: package_digest(),
        id: "native".to_owned(),
        contract_version: 1,
    };
    builder
        .register_package_provider("org.test", &key, provider.clone())
        .unwrap();
    builder
        .set_new_binding_default(&key, ConverterProvider::Package(provider))
        .unwrap();
    builder.build()
}

pub fn package_digest() -> String {
    format!("sha256:{}", "ab".repeat(32))
}

fn builder() -> ConverterRegistryBuilder {
    ConverterRegistryBuilder::with_core_conversions(Arc::new(TypeRegistry::core())).unwrap()
}

/// The plan connecting `from` to `to`; panics unless the planner answers `Ready`.
pub fn ready_plan(
    doc: &GraphDocument,
    registry: &RegistrySnapshot,
    from: (u8, &str),
    to: (u8, &str),
) -> ConnectionPlan {
    let request = ConnectionRequest {
        from: port_ref(from.0, from.1),
        to: port_ref(to.0, to.1),
        choice: None,
    };
    match plan_connection(doc, registry, request, id_source(5)).unwrap() {
        ConnectionOutcome::Ready(plan) => *plan,
        other => panic!("expected a plan, got {other:?}"),
    }
}

/// The core-converter registry with `provider` implementing `org.test/add`.
pub fn registry_with_provider(provider: ProviderKey) -> Arc<RegistrySnapshot> {
    registry_builder(core_converters(), None)
        .provider(ProviderDescriptor {
            key: provider,
            implements: [(add_definition(), 1)].into(),
            specializations: Default::default(),
        })
        .build()
        .unwrap()
}

/// The core-converter registry where `provider` limits `org.test/add` to `supported`.
pub fn registry_with_provider_specializations(
    provider: ProviderKey,
    supported: Vec<TypeArgs>,
) -> Arc<RegistrySnapshot> {
    registry_builder(core_converters(), None)
        .provider(ProviderDescriptor {
            key: provider,
            implements: Default::default(),
            specializations: [(add_definition(), supported)].into(),
        })
        .build()
        .unwrap()
}
