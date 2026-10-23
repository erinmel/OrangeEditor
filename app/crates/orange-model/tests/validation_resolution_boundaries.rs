mod common;

use common::registry::Facts;
use orange_model::GraphDocument;
use orange_model::catalog::{NodeContract, ProviderKey};
use orange_model::document::{DefinitionRef, NodeKind, PortDescriptor, TypeParam};
use orange_model::inference::NodeOverlays;
use orange_model::validation::{DiagnosticCode, HostOrigin, validate_nodes};
use serde_json::json;

const SOURCE: &str = "10000000-0000-4000-8000-000000000001";
const GENERIC: &str = "10000000-0000-4000-8000-000000000002";
const AMBIGUOUS: &str = "10000000-0000-4000-8000-000000000003";
const SOURCE_SIG: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const GENERIC_SIG: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const AMBIGUOUS_SIG: &str =
    "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
const SOURCE_DIGEST: &str =
    "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const GENERIC_DIGEST: &str =
    "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

#[test]
fn unresolved_concrete_neighbor_does_not_resolve_generic_from_its_snapshot() {
    let document: GraphDocument = serde_json::from_value(json!({
        "format_version":1,
        "graph_id":"20000000-0000-4000-8000-000000000001",
        "revision":0,
        "package_lock":[
            {"id":"source.pkg","version":"1.0.0","content_digest":SOURCE_DIGEST},
            {"id":"generic.pkg","version":"1.0.0","content_digest":GENERIC_DIGEST}
        ],
        "nodes":[
            {
                "id":SOURCE,"definition_id":"example/source","definition_version":1,
                "config_version":1,"config":{},"signature":SOURCE_SIG,
                "provider":{"package":"source.pkg","version":"1.0.0","id":"source-impl"}
            },
            {
                "id":GENERIC,"definition_id":"example/generic","definition_version":1,
                "config_version":1,"config":{},"signature":GENERIC_SIG,
                "provider":{"package":"generic.pkg","version":"1.0.0","id":"generic-impl"},
                "type_args":{"T":"core/i64@1"}
            },
            {
                "id":AMBIGUOUS,"definition_id":"example/ambiguous","definition_version":1,
                "config_version":1,"config":{},"signature":AMBIGUOUS_SIG,
                "provider":{"package":"generic.pkg","version":"1.0.0","id":"generic-impl"},
                "inputs":{"value":{"type":"core/i64@1","value":"1"}}
            }
        ],
        "edges":[{
            "id":"30000000-0000-4000-8000-000000000001",
            "from":{"node":SOURCE,"port":"out"},
            "to":{"node":GENERIC,"port":"value"}
        }],
        "signatures":{
            (SOURCE_SIG):{"kind":"pure","ports":[
                {"port_id":"out","direction":"output","kind":"data","type":"core/string@1","source":"computed"}
            ]},
            (GENERIC_SIG):{"kind":"pure","ports":[
                {"port_id":"value","direction":"input","kind":"data","type":"$T"}
            ]},
            (AMBIGUOUS_SIG):{"kind":"pure","ports":[
                {"port_id":"value","direction":"input","kind":"data","type":"$T"}
            ]}
        }
    })).unwrap();

    let generic_definition = DefinitionRef {
        id: "example/generic".parse().unwrap(),
        version: 1,
    };
    let generic_provider = ProviderKey {
        package: "generic.pkg".into(),
        version: "1.0.0".into(),
        content_digest: GENERIC_DIGEST.into(),
        id: "generic-impl".into(),
    };
    let origin = HostOrigin::new("1.0.0", format!("sha256:{}", "e".repeat(64))).unwrap();
    let mut facts = Facts::new(origin);
    facts.provider(generic_definition.clone(), 1, generic_provider.clone());
    facts.provider(
        DefinitionRef {
            id: "example/ambiguous".parse().unwrap(),
            version: 1,
        },
        1,
        generic_provider,
    );

    let input: PortDescriptor = serde_json::from_value(json!({
        "port_id":"value","direction":"input","kind":"data","type":"$T"
    }))
    .unwrap();
    facts.definition(
        generic_definition,
        NodeContract {
            kind: NodeKind::Pure,
            suspends: None,
            type_params: vec![TypeParam {
                name: "T".into(),
                constraints: vec![],
                extensions: Default::default(),
            }],
            ports: vec![input],
        },
    );
    let ambiguous_definition = DefinitionRef {
        id: "example/ambiguous".parse().unwrap(),
        version: 1,
    };
    let ambiguous_input: PortDescriptor = serde_json::from_value(json!({
        "port_id":"value","direction":"input","kind":"data","type":"$T"
    }))
    .unwrap();
    facts.definition(
        ambiguous_definition,
        NodeContract {
            kind: NodeKind::Pure,
            suspends: None,
            type_params: vec![TypeParam {
                name: "T".into(),
                constraints: vec![],
                extensions: Default::default(),
            }],
            ports: vec![ambiguous_input],
        },
    );
    facts.specializations(
        DefinitionRef {
            id: "example/ambiguous".parse().unwrap(),
            version: 1,
        },
        Some(ProviderKey {
            package: "generic.pkg".into(),
            version: "1.0.0".into(),
            content_digest: GENERIC_DIGEST.into(),
            id: "generic-impl".into(),
        }),
        vec![],
    );

    let report = validate_nodes(&document, &facts.snapshot(), &NodeOverlays::default());

    assert!(report.incomplete_nodes.contains(&SOURCE.parse().unwrap()));
    assert!(report.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == DiagnosticCode::NoSpecialization
            && diagnostic.subject.node == Some(AMBIGUOUS.parse().unwrap())
    }));
    assert!(!report.diagnostics.iter().any(|diagnostic| {
        diagnostic.subject.node == Some(GENERIC.parse().unwrap())
            && matches!(
                diagnostic.code,
                DiagnosticCode::TypeMismatch
                    | DiagnosticCode::TypeAmbiguous
                    | DiagnosticCode::ConstraintUnsatisfied
                    | DiagnosticCode::NoSpecialization
            )
    }));
    assert!(
        !report.resolved.contains_key(&GENERIC.parse().unwrap()),
        "a generic contract must not be resolved from an unresolved neighbour's persisted snapshot"
    );
}

#[test]
fn intrinsic_implicit_error_port_supplies_generic_type_evidence() {
    let branch = "10000000-0000-4000-8000-000000000010";
    let document: GraphDocument = serde_json::from_value(json!({
        "format_version":1,
        "graph_id":"20000000-0000-4000-8000-000000000001",
        "revision":0,
        "package_lock":[
            {"id":"generic.pkg","version":"1.0.0","content_digest":GENERIC_DIGEST}
        ],
        "nodes":[
            {
                "id":branch,"definition_id":"core/branch","definition_version":1,
                "config_version":1,"config":{}
            },
            {
                "id":GENERIC,"definition_id":"example/generic","definition_version":1,
                "config_version":1,"config":{},"signature":GENERIC_SIG,
                "provider":{"package":"generic.pkg","version":"1.0.0","id":"generic-impl"}
            }
        ],
        "edges":[{
            "id":"30000000-0000-4000-8000-000000000010",
            "from":{"node":branch,"port":"error"},
            "to":{"node":GENERIC,"port":"value"}
        }],
        "signatures":{
            (GENERIC_SIG):{"kind":"pure","ports":[
                {"port_id":"value","direction":"input","kind":"data","type":"core/error@1"}
            ]}
        }
    }))
    .unwrap();
    let definition = DefinitionRef {
        id: "example/generic".parse().unwrap(),
        version: 1,
    };
    let provider = ProviderKey {
        package: "generic.pkg".into(),
        version: "1.0.0".into(),
        content_digest: GENERIC_DIGEST.into(),
        id: "generic-impl".into(),
    };
    let origin = HostOrigin::new("1.0.0", format!("sha256:{}", "e".repeat(64))).unwrap();
    let mut facts = Facts::new(origin);
    facts.provider(definition.clone(), 1, provider);
    let input: PortDescriptor = serde_json::from_value(json!({
        "port_id":"value","direction":"input","kind":"data","type":"$T"
    }))
    .unwrap();
    facts.definition(
        definition,
        NodeContract {
            kind: NodeKind::Pure,
            suspends: None,
            type_params: vec![TypeParam {
                name: "T".into(),
                constraints: vec![],
                extensions: Default::default(),
            }],
            ports: vec![input],
        },
    );
    let report = validate_nodes(&document, &facts.snapshot(), &NodeOverlays::default());
    assert!(report.is_valid(), "{:?}", report.diagnostics);
    assert_eq!(
        report.resolved[&GENERIC.parse().unwrap()].ports[0]
            .ty
            .as_ref()
            .unwrap()
            .as_str(),
        "core/error@1"
    );
}

#[test]
fn missing_package_lock_prevents_core_specialization_fallback() {
    let node = "10000000-0000-4000-8000-000000000020";
    let signature = "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
    let document: GraphDocument = serde_json::from_value(json!({
        "format_version":1,
        "graph_id":"20000000-0000-4000-8000-000000000001",
        "revision":0,
        "package_lock":[],
        "nodes":[{
            "id":node,"definition_id":"example/generic","definition_version":1,
            "config_version":1,"config":{},"signature":signature,
            "provider":{"package":"generic.pkg","version":"1.0.0","id":"generic-impl"},
            "type_args":{"T":"core/f64@1"},
            "type_pins":{"T":"core/f64@1"}
        }],
        "edges":[],
        "signatures":{
            (signature):{"kind":"pure","ports":[
                {"port_id":"value","direction":"input","kind":"data","type":"core/f64@1"}
            ]}
        }
    }))
    .unwrap();
    let definition = DefinitionRef {
        id: "example/generic".parse().unwrap(),
        version: 1,
    };
    let origin = HostOrigin::new("1.0.0", format!("sha256:{}", "e".repeat(64))).unwrap();
    let mut facts = Facts::new(origin);
    let input: PortDescriptor = serde_json::from_value(json!({
        "port_id":"value","direction":"input","kind":"data","type":"$T"
    }))
    .unwrap();
    facts.definition(
        definition.clone(),
        NodeContract {
            kind: NodeKind::Pure,
            suspends: None,
            type_params: vec![TypeParam {
                name: "T".into(),
                constraints: vec![],
                extensions: Default::default(),
            }],
            ports: vec![input],
        },
    );
    facts.specializations(
        definition.clone(),
        None,
        ["core/i64@1", "core/i32@1"]
            .into_iter()
            .map(|ty| [("T".to_owned(), ty.parse().unwrap())].into())
            .collect(),
    );
    facts.provider(
        definition,
        1,
        ProviderKey {
            package: "generic.pkg".into(),
            version: "1.0.0".into(),
            content_digest: GENERIC_DIGEST.into(),
            id: "generic-impl".into(),
        },
    );

    // Low-level inference delegates this package binding to constraints; it must not
    // consult the core list. Compilation resolution independently requires the lock.
    let components = orange_model::inference::infer_components(
        &document,
        &orange_model::DocIndex::build(&document),
        [node.parse().unwrap()],
        &facts.snapshot(),
        &NodeOverlays::default(),
    );
    assert!(components[0].issues.is_empty());
    assert_eq!(
        components[0].type_args[&node.parse().unwrap()]["T"].as_str(),
        "core/f64@1"
    );

    let report = validate_nodes(&document, &facts.snapshot(), &NodeOverlays::default());

    assert_eq!(
        report
            .diagnostics
            .iter()
            .map(|item| item.code)
            .collect::<Vec<_>>(),
        vec![DiagnosticCode::PackageLockMissing]
    );
    assert!(report.resolved.is_empty());
    assert!(report.incomplete_nodes.contains(&node.parse().unwrap()));
}
