mod common;

use common::registry::Facts;
use orange_model::GraphDocument;
use orange_model::catalog::{NodeContract, ProviderKey};
use orange_model::document::{DefinitionRef, NodeKind, PortDescriptor, Suspends};
use orange_model::validation::{DiagnosticCode, HostOrigin, validate_nodes};
use serde_json::json;

const NODE: &str = "10000000-0000-4000-8000-000000000001";
const DIGEST: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn origin() -> HostOrigin {
    HostOrigin::new(
        "0.1.0",
        "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    )
    .unwrap()
}

fn document(kind: &str, suspends: Option<&str>) -> GraphDocument {
    let mut snapshot = json!({"kind":kind,"ports":[]});
    if let Some(suspends) = suspends {
        snapshot["suspends"] = json!(suspends);
    }
    serde_json::from_value(json!({
        "format_version":1,
        "graph_id":"20000000-0000-4000-8000-000000000001",
        "revision":0,
        "nodes":[{
            "id":NODE,
            "definition_id":"example/value",
            "definition_version":1,
            "config_version":1,
            "config":{},
            "signature":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "provider":{"package":"example.pkg","version":"1.0.0","id":"impl"}
        }],
        "edges":[],
        "package_lock":[{"id":"example.pkg","version":"1.0.0","content_digest":DIGEST}],
        "signatures":{"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa":snapshot}
    })).unwrap()
}

fn validate(
    kind: NodeKind,
    suspends: Option<Suspends>,
    doc: &GraphDocument,
) -> Vec<DiagnosticCode> {
    let definition = DefinitionRef {
        id: "example/value".parse().unwrap(),
        version: 1,
    };
    let mut facts = Facts::new(origin());
    facts.definition(
        definition.clone(),
        NodeContract {
            kind,
            suspends,
            type_params: vec![],
            ports: vec![
                serde_json::from_value::<PortDescriptor>(json!({
                    "port_id":"value","direction":"output","kind":"data",
                    "type":"core/i64@1","source":"computed"
                }))
                .unwrap(),
            ],
        },
    );
    facts.provider(
        definition,
        1,
        ProviderKey {
            package: "example.pkg".into(),
            version: "1.0.0".into(),
            content_digest: DIGEST.into(),
            id: "impl".into(),
        },
    );
    validate_nodes(doc, &facts.snapshot())
        .diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.code)
        .collect()
}

#[test]
fn async_contract_requires_explicit_suspension_metadata() {
    let doc = document("async", None);
    assert_eq!(
        validate(NodeKind::Async, None, &doc),
        [DiagnosticCode::SuspendsMissing]
    );
    let doc = document("async", Some("maybe"));
    assert!(
        !validate(NodeKind::Async, Some(Suspends::Maybe), &doc)
            .contains(&DiagnosticCode::SuspendsMissing)
    );
    let doc = document("async", Some("always"));
    assert!(
        !validate(NodeKind::Async, Some(Suspends::Always), &doc)
            .contains(&DiagnosticCode::SuspendsMissing)
    );
}

#[test]
fn synchronous_contract_rejects_non_never_suspension() {
    let doc = document("pure", Some("maybe"));
    assert_eq!(
        validate(NodeKind::Pure, Some(Suspends::Maybe), &doc),
        [DiagnosticCode::SuspendsInvalid]
    );
}

#[test]
fn async_and_iterator_contracts_reject_never() {
    for (name, kind) in [("async", NodeKind::Async), ("iterator", NodeKind::Iterator)] {
        let doc = document(name, Some("never"));
        assert_eq!(
            validate(kind, Some(Suspends::Never), &doc),
            [DiagnosticCode::SuspendsInvalid]
        );
    }
}
