use orange_model::document::{DefinitionRef, NodeKind, ResolvedPort, TypeParam};
use orange_model::inference::{GenericDefinition, ProviderKey, TemplateCatalog};
use orange_model::types::TypeRegistry;
use orange_model::validation::{DiagnosticCode, HostOrigin, ProviderCatalog, validate_nodes};
use orange_model::{GraphDocument, to_json};
use serde_json::{Value, json};

const NODE: &str = "10000000-0000-4000-8000-000000000001";
const SIG: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DIGEST: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn origin() -> HostOrigin {
    HostOrigin::new("0.1.0", format!("sha256:{}", "c".repeat(64))).unwrap()
}

fn document(
    provider: Option<Value>,
    lock: bool,
    signature: Option<Value>,
    inputs: Value,
) -> GraphDocument {
    let mut node = json!({
        "id": NODE,
        "definition_id": "example/value",
        "definition_version": 1,
        "config_version": 1,
        "config": {},
        "type_args": {"T":"core/string@1"},
        "inputs": inputs
    });
    if let Some(provider) = provider {
        node["provider"] = provider;
    }
    if signature.is_some() {
        node["signature"] = json!(SIG);
    }
    let mut value = json!({
        "format_version": 1,
        "graph_id": "20000000-0000-4000-8000-000000000001",
        "revision": 7,
        "nodes": [node],
        "edges": [],
        "package_lock": if lock { json!([{
            "id":"example.pkg", "version":"1.0.0", "content_digest":DIGEST
        }]) } else { json!([]) },
        "signatures": {}
    });
    if let Some(signature) = signature {
        value["signatures"][SIG] = signature;
    }
    serde_json::from_value(value).unwrap()
}

fn binding(id: &str) -> Value {
    json!({"package":"example.pkg", "version":"1.0.0", "id":id})
}

fn definition() -> DefinitionRef {
    DefinitionRef {
        id: "example/value".parse().unwrap(),
        version: 1,
    }
}

fn provider(id: &str, digest: &str) -> ProviderKey {
    ProviderKey {
        package: "example.pkg".into(),
        version: "1.0.0".into(),
        content_digest: digest.into(),
        id: id.into(),
    }
}

fn snapshot(ty: &str) -> Value {
    json!({
        "kind":"pure",
        "ports":[{"port_id":"value","direction":"output","kind":"data","type":ty,"source":"computed"}]
    })
}

fn concrete_contract() -> GenericDefinition {
    GenericDefinition {
        kind: NodeKind::Pure,
        type_params: vec![],
        ports: vec![
            serde_json::from_value(json!({
                "port_id":"value", "direction":"output", "kind":"data",
                "type":"core/i64@1", "source":"computed"
            }))
            .unwrap(),
        ],
    }
}

fn setup(id: &str) -> (TemplateCatalog, ProviderCatalog) {
    let mut templates = TemplateCatalog::default();
    templates.insert(definition(), concrete_contract());
    let mut providers = ProviderCatalog::default();
    providers.insert(definition(), 1, provider(id, DIGEST));
    (templates, providers)
}

fn codes(report: &orange_model::validation::NodeValidation) -> Vec<DiagnosticCode> {
    report.diagnostics.iter().map(|item| item.code).collect()
}

#[test]
fn distinct_provider_id_and_digest_are_required() {
    let (templates, providers) = setup("implementation-a");
    let doc = document(
        Some(binding("implementation-b")),
        true,
        Some(snapshot("core/i64@1")),
        json!({}),
    );
    let report = validate_nodes(
        &doc,
        &templates,
        &TypeRegistry::core(),
        &providers,
        &origin(),
    );
    assert_eq!(codes(&report), vec![DiagnosticCode::ProviderUnavailable]);
    assert!(report.resolved.is_empty());

    let mut providers = ProviderCatalog::default();
    providers.insert(
        definition(),
        1,
        provider("implementation-a", &format!("sha256:{}", "d".repeat(64))),
    );
    let doc = document(
        Some(binding("implementation-a")),
        true,
        Some(snapshot("core/i64@1")),
        json!({}),
    );
    let report = validate_nodes(
        &doc,
        &templates,
        &TypeRegistry::core(),
        &providers,
        &origin(),
    );
    assert_eq!(codes(&report), vec![DiagnosticCode::ProviderUnavailable]);
}

#[test]
fn primary_resolution_errors_do_not_cascade_or_fall_back_to_core() {
    let (templates, providers) = setup("implementation-a");
    let missing_provider = document(None, true, Some(snapshot("core/i64@1")), json!({}));
    let report = validate_nodes(
        &missing_provider,
        &templates,
        &TypeRegistry::core(),
        &providers,
        &origin(),
    );
    assert_eq!(codes(&report), vec![DiagnosticCode::ProviderMissing]);
    assert!(report.resolved.is_empty());

    let missing_lock = document(
        Some(binding("implementation-a")),
        false,
        Some(snapshot("core/i64@1")),
        json!({}),
    );
    let report = validate_nodes(
        &missing_lock,
        &templates,
        &TypeRegistry::core(),
        &providers,
        &origin(),
    );
    assert_eq!(codes(&report), vec![DiagnosticCode::PackageLockMissing]);

    let missing_signature = document(Some(binding("implementation-a")), true, None, json!({}));
    let report = validate_nodes(
        &missing_signature,
        &templates,
        &TypeRegistry::core(),
        &providers,
        &origin(),
    );
    assert_eq!(codes(&report), vec![DiagnosticCode::SignatureMissing]);
}

#[test]
fn generic_resolution_uses_fresh_evidence_instead_of_cached_type_args() {
    let doc = document(
        Some(binding("implementation-a")),
        true,
        Some(json!({"kind":"pure","ports":[{
            "port_id":"value","direction":"input","kind":"data","type":"$T"
        }]})),
        json!({"value":{"type":"core/i64@1","value":3}}),
    );
    let mut templates = TemplateCatalog::default();
    let mut port: ResolvedPort = serde_json::from_value(json!({
        "port_id":"value","direction":"input","kind":"data","type":"$T"
    }))
    .unwrap();
    port.extensions = Default::default();
    templates.insert(
        definition(),
        GenericDefinition {
            kind: NodeKind::Pure,
            type_params: vec![TypeParam {
                name: "T".into(),
                constraints: vec![],
                extensions: Default::default(),
            }],
            ports: vec![port],
        },
    );
    let mut providers = ProviderCatalog::default();
    providers.insert(definition(), 1, provider("implementation-a", DIGEST));
    let report = validate_nodes(
        &doc,
        &templates,
        &TypeRegistry::core(),
        &providers,
        &origin(),
    );
    assert_eq!(codes(&report), vec![DiagnosticCode::SignatureDrift]);
    assert_eq!(
        report.resolved[&NODE.parse().unwrap()].ports[0]
            .ty
            .as_ref()
            .unwrap()
            .as_str(),
        "core/i64@1"
    );
}

#[test]
fn validation_does_not_change_document_content_or_revision() {
    let (templates, providers) = setup("implementation-a");
    let doc = document(
        Some(binding("implementation-a")),
        true,
        Some(snapshot("core/i64@1")),
        json!({}),
    );
    let before = to_json(&doc).unwrap();
    let revision = doc.revision();
    let state_id = doc.state_id();
    let _ = validate_nodes(
        &doc,
        &templates,
        &TypeRegistry::core(),
        &providers,
        &origin(),
    );
    assert_eq!(to_json(&doc).unwrap(), before);
    assert_eq!(doc.revision(), revision);
    assert_eq!(doc.state_id(), state_id);
}

#[test]
fn host_intrinsics_resolve_without_snapshots_but_reject_package_overrides() {
    let value = json!({
        "format_version": 1,
        "graph_id": "20000000-0000-4000-8000-000000000001",
        "revision": 0,
        "nodes": [{
            "id": NODE,
            "definition_id": "core/branch",
            "definition_version": 1,
            "config_version": 1,
            "config": {}
        }],
        "edges": [],
        "signatures": {}
    });
    let doc: GraphDocument = serde_json::from_value(value.clone()).unwrap();
    let report = validate_nodes(
        &doc,
        &TemplateCatalog::default(),
        &TypeRegistry::core(),
        &ProviderCatalog::default(),
        &origin(),
    );
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert!(report.resolved.contains_key(&NODE.parse().unwrap()));

    let mut overridden = value;
    overridden["nodes"][0]["provider"] = binding("package-override");
    let doc: GraphDocument = serde_json::from_value(overridden).unwrap();
    let report = validate_nodes(
        &doc,
        &TemplateCatalog::default(),
        &TypeRegistry::core(),
        &ProviderCatalog::default(),
        &origin(),
    );
    assert_eq!(codes(&report), vec![DiagnosticCode::ProviderUnavailable]);
    assert!(report.resolved.is_empty());
}

#[test]
fn intrinsic_data_port_supplies_fresh_generic_inference_evidence() {
    let intrinsic = "10000000-0000-4000-8000-000000000010";
    let edge = "20000000-0000-4000-8000-000000000010";
    let doc: GraphDocument = serde_json::from_value(json!({
        "format_version": 1,
        "graph_id": "20000000-0000-4000-8000-000000000001",
        "revision": 0,
        "package_lock": [{"id":"example.pkg","version":"1.0.0","content_digest":DIGEST}],
        "nodes": [
            {
                "id": intrinsic,
                "definition_id":"core/count",
                "definition_version":1,
                "config_version":1,
                "config":{}
            },
            {
                "id": NODE,
                "definition_id":"example/value",
                "definition_version":1,
                "config_version":1,
                "config":{},
                "type_args":{"T":"core/string@1"},
                "provider":{"package":"example.pkg","version":"1.0.0","id":"implementation-a"},
                "signature":SIG
            }
        ],
        "edges": [{"id":edge,"from":{"node":intrinsic,"port":"index"},"to":{"node":NODE,"port":"value"}}],
        "signatures": { (SIG): {"kind":"pure","ports":[
            {"port_id":"value","direction":"input","kind":"data","type":"$T"}
        ]}}
    })).unwrap();
    let mut templates = TemplateCatalog::default();
    templates.insert(
        definition(),
        GenericDefinition {
            kind: NodeKind::Pure,
            type_params: vec![TypeParam {
                name: "T".into(),
                constraints: vec![],
                extensions: Default::default(),
            }],
            ports: vec![
                serde_json::from_value(json!({
                    "port_id":"value","direction":"input","kind":"data","type":"$T"
                }))
                .unwrap(),
            ],
        },
    );
    let mut providers = ProviderCatalog::default();
    providers.insert(definition(), 1, provider("implementation-a", DIGEST));
    let report = validate_nodes(
        &doc,
        &templates,
        &TypeRegistry::core(),
        &providers,
        &origin(),
    );
    assert_eq!(codes(&report), vec![DiagnosticCode::SignatureDrift]);
    assert!(report.incomplete_nodes.is_empty());
    assert_eq!(
        report.resolved[&NODE.parse().unwrap()].ports[0]
            .ty
            .as_ref()
            .unwrap()
            .as_str(),
        "core/i64@1"
    );
}

#[test]
fn combined_template_and_argument_depth_is_a_complexity_diagnostic() {
    let deep_type = format!("{}core/i64@1{}", "core/list@1<".repeat(64), ">".repeat(64));
    let doc: GraphDocument = serde_json::from_value(json!({
        "format_version":1,
        "graph_id":"20000000-0000-4000-8000-000000000001",
        "revision":0,
        "package_lock":[{"id":"example.pkg","version":"1.0.0","content_digest":DIGEST}],
        "nodes":[{
            "id":NODE,
            "definition_id":"example/value",
            "definition_version":1,
            "config_version":1,
            "config":{},
            "type_pins":{"T":deep_type},
            "provider":{"package":"example.pkg","version":"1.0.0","id":"implementation-a"},
            "signature":SIG
        }],
        "edges":[],
        "signatures":{(SIG):{"kind":"pure","ports":[{
            "port_id":"value","direction":"output","kind":"data",
            "type":"core/list@1<$T>","source":"computed"
        }]}}
    }))
    .unwrap();
    let mut templates = TemplateCatalog::default();
    templates.insert(
        definition(),
        GenericDefinition {
            kind: NodeKind::Pure,
            type_params: vec![TypeParam {
                name: "T".into(),
                constraints: vec![],
                extensions: Default::default(),
            }],
            ports: vec![
                serde_json::from_value(json!({
                    "port_id":"value","direction":"output","kind":"data",
                    "type":"core/list@1<$T>","source":"computed"
                }))
                .unwrap(),
            ],
        },
    );
    let mut providers = ProviderCatalog::default();
    providers.insert(definition(), 1, provider("implementation-a", DIGEST));
    let report = validate_nodes(
        &doc,
        &templates,
        &TypeRegistry::core(),
        &providers,
        &origin(),
    );
    assert_eq!(codes(&report), vec![DiagnosticCode::TypeComplexity]);
    assert!(report.resolved.is_empty());
    assert!(report.incomplete_nodes.contains(&NODE.parse().unwrap()));
}
