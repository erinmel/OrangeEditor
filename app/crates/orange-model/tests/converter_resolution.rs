//! C1: persisted converter nodes resolve their exact entry and implementation.
mod common;

use std::sync::Arc;

use common::{content, id_source, minimal_document, node_id, port as port_ref};
use orange_model::catalog::{
    NodeDefinition, Ports, ProviderDescriptor, ProviderKey, RegistryBuilder,
};
use orange_model::conversion::{
    ConnectionOutcome, ConverterKey, ConverterProvider, ConverterRegistryBuilder,
    ExplicitConverterChoice, PackageProvider, apply_connection, plan_connection,
};
use orange_model::document::{DefinitionRef, NodeKind, PortDescriptor};
use orange_model::types::TypeRegistry;
use orange_model::validation::{HostOrigin, validate, validate_nodes};
use orange_model::{GraphDocument, from_json, to_json};
use serde_json::{Value, json};

const DIGEST: &str = "sha256:abababababababababababababababababababababababababababababababab";

fn registry(package_converter: bool) -> Arc<orange_model::catalog::RegistrySnapshot> {
    let mut converters =
        ConverterRegistryBuilder::with_core_conversions(Arc::new(TypeRegistry::core())).unwrap();
    if package_converter {
        let key = ConverterKey {
            id: "core/i32-to-i64".parse().unwrap(),
            contract_version: 1,
        };
        converters
            .register_package_provider(
                "org.test",
                &key,
                PackageProvider {
                    package: "org.test".into(),
                    version: "1.0.0".into(),
                    content_digest: DIGEST.into(),
                    id: "convert".into(),
                    contract_version: 1,
                },
            )
            .unwrap();
    }
    let source = def(
        "org.test/source",
        PortDescriptor::data_out(
            "value",
            "core/i32@1".parse().unwrap(),
            orange_model::document::OutputSource::Computed,
        ),
    );
    let sink = def(
        "org.test/sink",
        PortDescriptor::data_in("value", "core/i64@1".parse().unwrap()),
    );
    let source_key = provider_key("source");
    let sink_key = provider_key("sink");
    let builder = RegistryBuilder::core(origin())
        .converters(converters.build())
        .definition(source.clone())
        .definition(sink.clone())
        .provider(provider(source_key, &source.definition))
        .provider(provider(sink_key, &sink.definition));
    builder.build().unwrap()
}

fn origin() -> HostOrigin {
    HostOrigin::new("1.0.0", format!("sha256:{}", "00".repeat(32))).unwrap()
}

fn provider_key(id: &str) -> ProviderKey {
    ProviderKey {
        package: "org.test".into(),
        version: "1.0.0".into(),
        content_digest: DIGEST.into(),
        id: id.into(),
    }
}

fn provider(key: ProviderKey, definition: &DefinitionRef) -> ProviderDescriptor {
    ProviderDescriptor {
        key,
        implements: [(definition.clone(), 1)].into(),
        specializations: Default::default(),
    }
}

fn def(id: &str, port: PortDescriptor) -> NodeDefinition {
    NodeDefinition {
        definition: DefinitionRef {
            id: id.parse().unwrap(),
            version: 1,
        },
        config_version: 1,
        kind: NodeKind::Pure,
        suspends: None,
        type_params: vec![],
        ports: Ports::Declared(vec![port]),
        references: vec![],
        specializations: None,
        config_schema: None,
    }
}

fn graph() -> GraphDocument {
    from_json(&minimal_document(json!({
        "package_lock": [{"id":"org.test","version":"1.0.0","content_digest":DIGEST}],
        "signatures": {
            "source_sig": {"kind":"pure","ports":[{"port_id":"value","direction":"output","kind":"data","type":"core/i32@1","source":"computed"}]},
            "sink_sig": {"kind":"pure","ports":[{"port_id":"value","direction":"input","kind":"data","type":"core/i64@1"}]}
        },
        "nodes": [
            {"id":node_id(1),"definition_id":"org.test/source","definition_version":1,"config_version":1,"config":{},"provider":{"package":"org.test","version":"1.0.0","id":"source"},"signature":"source_sig"},
            {"id":node_id(2),"definition_id":"org.test/sink","definition_version":1,"config_version":1,"config":{},"provider":{"package":"org.test","version":"1.0.0","id":"sink"},"signature":"sink_sig"}
        ], "edges": []
    }))).unwrap()
}

fn plan(
    doc: &GraphDocument,
    registry: &orange_model::catalog::RegistrySnapshot,
    package: bool,
) -> orange_model::conversion::ConnectionPlan {
    let key = ConverterKey {
        id: "core/i32-to-i64".parse().unwrap(),
        contract_version: 1,
    };
    let provider = if package {
        ConverterProvider::Package(PackageProvider {
            package: "org.test".into(),
            version: "1.0.0".into(),
            content_digest: DIGEST.into(),
            id: "convert".into(),
            contract_version: 1,
        })
    } else {
        ConverterProvider::Core
    };
    let outcome = plan_connection(
        doc,
        registry,
        orange_model::conversion::ConnectionRequest {
            from: port_ref(1, "value"),
            to: port_ref(2, "value"),
            choice: package.then_some(ExplicitConverterChoice {
                entry: key,
                provider,
                options: Default::default(),
            }),
        },
        id_source(7),
    )
    .unwrap();
    match outcome {
        ConnectionOutcome::Ready(plan) => *plan,
        other => panic!("unexpected plan outcome {other:?}"),
    }
}

#[test]
fn plan_apply_and_combined_validation_resolve_core_and_exact_package_implementations() {
    for package in [false, true] {
        let registry = registry(package);
        let mut document = graph();
        let before = content(&document);
        let before_bytes = to_json(&document).unwrap();
        let before_revision = document.revision();
        let before_state = document.state_id();
        let plan = plan(&document, &registry, package);
        let converter_id = plan.converter.as_ref().unwrap().node;
        let committed = apply_connection(&mut document, &registry, plan).unwrap();
        let after = content(&document);
        let after_bytes = to_json(&document).unwrap();
        let applied_revision = document.revision();
        let applied_state = document.state_id();
        let report = validate(&document, &registry);
        assert!(report.is_valid(), "{:#?}", report.diagnostics);
        assert_eq!(to_json(&document).unwrap(), after_bytes);
        assert_eq!(document.revision(), applied_revision);
        assert_eq!(document.state_id(), applied_state);
        assert_eq!(applied_revision, before_revision + 1);
        assert_ne!(applied_state, before_state);
        let signature = &report.resolved[&converter_id];
        assert_eq!(
            signature
                .ports
                .iter()
                .map(|p| p.port_id.as_str())
                .collect::<Vec<_>>(),
            ["value", "result"]
        );
        assert_eq!(
            signature.ports[1].ty.as_ref().unwrap().as_str(),
            "core/i64@1"
        );
        let converter = document.node(converter_id).unwrap();
        assert_eq!(converter.provider.is_some(), package);
        if package {
            let binding = converter.provider.as_ref().unwrap();
            assert_eq!(
                (&*binding.package, &*binding.version, &*binding.id),
                ("org.test", "1.0.0", "convert")
            );
        }
        let redo = document.revert(committed.inverse).unwrap();
        assert!(document.node(converter_id).is_none());
        assert_eq!(content(&document), before);
        let _redo_commit = document.revert(redo.inverse).unwrap();
        assert!(document.node(converter_id).is_some());
        assert_eq!(content(&document), after);
        assert_ne!(after_bytes, before_bytes);
    }
}

fn convert_only(config: Value, provider: Option<Value>, signature: Option<Value>) -> GraphDocument {
    let mut signatures = serde_json::Map::new();
    let has_signature = signature.is_some();
    if let Some(signature) = signature {
        signatures.insert("sig".into(), signature);
    }
    let node = json!({"id":node_id(3),"definition_id":"core/convert","definition_version":1,
        "config_version":1,"config":config,"provider":provider,"signature":has_signature.then_some("sig")});
    from_json(&minimal_document(json!({"package_lock": [{"id":"org.test","version":"1.0.0","content_digest":DIGEST}],"nodes":[node],"signatures":signatures}))).unwrap()
}

fn saved_signature(ports: Value) -> Value {
    json!({"kind":"pure","ports":ports})
}

#[test]
fn lossy_and_checked_contracts_derive_their_entry_ports_and_saved_options_are_complete() {
    let registry = orange_model::catalog::RegistrySnapshot::core(origin());
    let lossy = convert_only(
        json!({"converter":"core/f64-to-f32","contract_version":1,"options":{"rounding":"nearest-even","overflow":"reject"}}),
        None,
        Some(saved_signature(json!([
            {"port_id":"value","direction":"input","kind":"data","type":"core/f64@1"},
            {"port_id":"result","direction":"output","kind":"data","type":"core/f32@1","source":"computed"},
            {"port_id":"loss","direction":"output","kind":"data","type":"core/json@1","source":"computed"}
        ]))),
    );
    let resolved = validate_nodes(&lossy, &registry);
    assert!(resolved.is_valid(), "{:#?}", resolved.diagnostics);
    assert_eq!(resolved.resolved.values().next().unwrap().ports.len(), 3);

    let checked = convert_only(
        json!({"converter":"core/i64-to-i32","contract_version":1,"options":{}}),
        None,
        Some(saved_signature(json!([
            {"port_id":"value","direction":"input","kind":"data","type":"core/i64@1"},
            {"port_id":"result","direction":"output","kind":"data","type":"core/i32@1","source":"computed"}
        ]))),
    );
    assert!(validate_nodes(&checked, &registry).is_valid());

    let omitted = convert_only(
        json!({"converter":"core/f64-to-f32","contract_version":1,"options":{"rounding":"nearest-even"}}),
        None,
        None,
    );
    assert!(
        validate_nodes(&omitted, &registry)
            .diagnostics
            .iter()
            .any(|d| d.code.as_str() == "E-SIGNATURE-MISSING")
    );
    let omitted_options = convert_only(
        json!({"converter":"core/f64-to-f32","contract_version":1,"options":{"rounding":"nearest-even"}}),
        None,
        Some(saved_signature(json!([]))),
    );
    let report = validate_nodes(&omitted_options, &registry);
    assert!(
        report
            .incomplete_nodes
            .contains(&node_id(3).parse().unwrap())
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code.as_str() == "E-CONVERTER-OPTIONS")
    );
}

#[test]
fn converter_snapshot_is_compared_and_function_summary_keeps_pure_semantics() {
    let registry = orange_model::catalog::RegistrySnapshot::core(origin());
    let config = json!({"converter":"core/i32-to-i64","contract_version":1,"options":{}});
    let drifted = convert_only(
        config.clone(),
        None,
        Some(saved_signature(json!([
            {"port_id":"value","direction":"input","kind":"data","type":"core/i32@1"},
            {"port_id":"result","direction":"output","kind":"data","type":"core/f64@1","source":"computed"}
        ]))),
    );
    let report = validate_nodes(&drifted, &registry);
    let id = node_id(3).parse().unwrap();
    assert!(report.incomplete_nodes.contains(&id));
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code.as_str() == "E-SIGNATURE-DRIFT")
    );
    assert_eq!(
        report.resolved[&id].ports[1].ty.as_ref().unwrap().as_str(),
        "core/i64@1"
    );

    let mut value: Value = serde_json::from_str(&to_json(&convert_only(config, None, Some(saved_signature(json!([
        {"port_id":"value","direction":"input","kind":"data","type":"core/i32@1"},
        {"port_id":"result","direction":"output","kind":"data","type":"core/i64@1","source":"computed"}
    ]))))).unwrap()).unwrap();
    let node = value["nodes"][0].take();
    value["nodes"] = json!([]);
    value["functions"] = json!([{"id":"50000000-0000-4000-8000-000000000001","name":"convert","kind":"pure","nodes":[node],"edges":[]}]);
    let function_doc = from_json(&value.to_string()).unwrap();
    let combined = validate(&function_doc, &registry);
    assert!(
        combined.incomplete_functions.is_empty(),
        "{:#?}",
        combined.diagnostics
    );
    assert_eq!(
        combined.functions.values().next().unwrap().kind,
        NodeKind::Pure
    );
}

#[test]
fn malformed_versions_entries_options_and_exact_package_identity_are_diagnosed() {
    let registry = registry(true);
    let signature = saved_signature(json!([
        {"port_id":"value","direction":"input","kind":"data","type":"core/i32@1"},
        {"port_id":"result","direction":"output","kind":"data","type":"core/i64@1","source":"computed"}
    ]));
    let cases = [
        (
            json!({"converter":"core/i32-to-i64","contract_version":2,"options":{}}),
            None,
            "E-CONVERTER-ENTRY",
        ),
        (
            json!({"converter":"core/i32-to-i64","contract_version":1,"options":{"unknown":"x"}}),
            None,
            "E-CONVERTER-OPTIONS",
        ),
        (
            json!({"converter":"core/i32-to-i64","contract_version":1,"options":{"x":null}}),
            None,
            "E-CONVERTER-OPTIONS",
        ),
        (
            json!({"converter":"core/missing","contract_version":1,"options":{}}),
            None,
            "E-CONVERTER-ENTRY",
        ),
    ];
    for (config, provider, code) in cases {
        let document = convert_only(config, provider, Some(signature.clone()));
        assert!(
            validate_nodes(&document, &registry)
                .diagnostics
                .iter()
                .any(|d| d.code.as_str() == code)
        );
    }
    let mut malformed: Value = serde_json::from_str(
        &to_json(&convert_only(
            json!({"converter":"core/i32-to-i64","contract_version":1,"options":{}}),
            None,
            Some(saved_signature(json!([]))),
        ))
        .unwrap(),
    )
    .unwrap();
    malformed["nodes"][0]["config_version"] = json!(2);
    let malformed = from_json(&malformed.to_string()).unwrap();
    assert!(
        validate_nodes(&malformed, &registry)
            .diagnostics
            .iter()
            .any(|d| d.code.as_str() == "E-CONVERTER-CONFIG")
    );
    let mut bad_definition_version: Value = serde_json::from_str(
        &to_json(&convert_only(
            json!({"converter":"core/i32-to-i64","contract_version":1,"options":{}}),
            None,
            Some(saved_signature(json!([]))),
        ))
        .unwrap(),
    )
    .unwrap();
    bad_definition_version["nodes"][0]["definition_version"] = json!(2);
    let bad_definition_version = from_json(&bad_definition_version.to_string()).unwrap();
    assert!(
        validate_nodes(&bad_definition_version, &registry)
            .diagnostics
            .iter()
            .any(|d| d.code.as_str() == "E-CONVERTER-CONFIG")
    );

    let package = json!({"package":"org.test","version":"1.0.0","id":"convert"});
    let wrong_digest_doc = convert_only(
        json!({"converter":"core/i32-to-i64","contract_version":1,"options":{}}),
        Some(package),
        Some(signature),
    );
    let mut value: Value = serde_json::from_str(&to_json(&wrong_digest_doc).unwrap()).unwrap();
    value["package_lock"][0]["content_digest"] = json!(format!("sha256:{}", "cd".repeat(32)));
    let wrong_digest = from_json(&value.to_string()).unwrap();
    assert!(
        validate_nodes(&wrong_digest, &registry)
            .diagnostics
            .iter()
            .any(|d| d.code.as_str() == "E-PROVIDER-UNAVAILABLE")
    );
    let unknown_provider = convert_only(
        json!({"converter":"core/i32-to-i64","contract_version":1,"options":{}}),
        Some(json!({"package":"org.test","version":"1.0.0","id":"other"})),
        Some(saved_signature(json!([]))),
    );
    assert!(
        validate_nodes(&unknown_provider, &registry)
            .diagnostics
            .iter()
            .any(|d| d.code.as_str() == "E-PROVIDER-UNAVAILABLE")
    );

    let mut no_lock: Value = serde_json::from_str(
        &to_json(&convert_only(
            json!({"converter":"core/i32-to-i64","contract_version":1,"options":{}}),
            Some(json!({"package":"org.test","version":"1.0.0","id":"convert"})),
            Some(saved_signature(json!([]))),
        ))
        .unwrap(),
    )
    .unwrap();
    no_lock["package_lock"] = json!([]);
    let no_lock = from_json(&no_lock.to_string()).unwrap();
    assert!(
        validate_nodes(&no_lock, &registry)
            .diagnostics
            .iter()
            .any(|d| d.code.as_str() == "E-PACKAGE-LOCK-MISSING")
    );
}
