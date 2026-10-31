//! Public-consumer qualification for the bounded node-local session cache.
mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use orange_model::catalog::{
    Ports, ProviderDescriptor, ProviderKey, RegistryBuilder, RegistrySnapshot,
};
use orange_model::document::{BodyRef, NodeInstance, NodeKind, PackageLockEntry};
use orange_model::id::NodeId;
use orange_model::validation::{
    AnalysisOptions, AnalysisSession, DiagnosticCode, SessionWork, Subject, ValidationAnalysis,
    analyze_with_options,
};
use orange_model::{DocCommand, GraphDocument, LayoutCommand};
use serde_json::json;

const DIGEST: &str = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

fn node_id(n: usize) -> NodeId {
    format!("10000000-0000-4000-8000-{n:012}").parse().unwrap()
}

fn fixture(count: usize) -> (GraphDocument, Arc<RegistrySnapshot>) {
    let source_registry = common::generic::registry(None);
    let mut definition = source_registry
        .definition(&common::generic::add_definition())
        .unwrap()
        .clone();
    definition.definition.id = "example/configured".parse().unwrap();
    definition.type_params.clear();
    definition.ports = Ports::Declared(Vec::new());
    definition.config_schema = Some(json!({
        "type":"object", "required":["enabled"],
        "properties":{"enabled":{"type":"boolean"}}, "additionalProperties":false
    }));
    let provider = ProviderDescriptor {
        key: ProviderKey {
            package: "example.pkg".into(),
            version: "1.0.0".into(),
            content_digest: DIGEST.into(),
            id: "example/configured".into(),
        },
        implements: [(definition.definition.clone(), 1)].into(),
        specializations: Default::default(),
    };
    let registry = RegistryBuilder::core(common::generic::origin())
        .definition(definition)
        .provider(provider)
        .build()
        .unwrap();
    let mut document = GraphDocument::new("00000000-0000-4000-8000-000000000001".parse().unwrap());
    let signature = orange_model::signature::project(NodeKind::Pure, [], &BTreeMap::new());
    let key = orange_model::signature::semantic_key(&signature);
    let mut commands = vec![
        DocCommand::PutPackageLock(PackageLockEntry {
            id: "example.pkg".into(),
            version: "1.0.0".into(),
            content_digest: DIGEST.into(),
            extensions: Default::default(),
        }),
        DocCommand::PutSignature {
            key: key.clone(),
            snapshot: signature,
        },
    ];
    commands.extend((1..=count).map(|n| {
        let node: NodeInstance = serde_json::from_value(json!({
            "id":node_id(n), "definition_id":"example/configured", "definition_version":1,
            "config_version":1, "config":{"enabled":true}, "signature":key,
            "provider":{"package":"example.pkg", "version":"1.0.0", "id":"example/configured"}
        }))
        .unwrap();
        DocCommand::AddNode {
            body: BodyRef::Root,
            node,
        }
    }));
    let _ = document.apply(commands).unwrap();
    (document, registry)
}

fn full(document: &GraphDocument, registry: &RegistrySnapshot) -> ValidationAnalysis {
    analyze_with_options(
        document,
        registry,
        BTreeMap::new(),
        AnalysisOptions::default(),
    )
}

fn config(node: usize, enabled: serde_json::Value) -> DocCommand {
    DocCommand::SetConfig {
        node: node_id(node),
        config_version: 1,
        config: serde_json::from_value(json!({"enabled":enabled})).unwrap(),
    }
}

#[test]
fn one_config_edit_reuses_other_units_and_publishes_only_changed_subjects() {
    let (mut document, registry) = fixture(16);
    let mut session = AnalysisSession::new(
        &document,
        &registry,
        BTreeMap::new(),
        AnalysisOptions::default(),
    );
    assert!(session.analysis().report().is_valid());
    assert!(session.diagnostics().is_empty());
    assert_eq!(session.stats().local.config_computed, 16);
    assert_eq!(session.stats().local.config_hits, 0);
    assert_eq!(
        (
            session.stats().local.inputs_computed,
            session.stats().local.input_hits
        ),
        (16, 0)
    );

    let changed = document
        .apply([config(7, json!("invalid boolean"))])
        .unwrap();
    let stats = *session.update(
        &document,
        &registry,
        &changed.change_set,
        AnalysisOptions::default(),
    );
    // A configuration edit on a package node is scoped: only its own configuration and
    // input checks run, and no other node is visited.
    assert_eq!(stats.work, SessionWork::Scoped);
    assert_eq!(
        (stats.local.config_computed, stats.local.config_hits),
        (1, 0)
    );
    assert_eq!(stats.local.nodes_visited, 0);
    assert_eq!(
        (stats.local.inputs_computed, stats.local.input_hits),
        (1, 0)
    );
    assert_eq!(stats.full_analysis_runs, 0);
    assert_eq!(session.analysis(), &full(&document, &registry));

    let subject = Subject::new(Some(node_id(7)), None, None);
    let borrowed: Vec<_> = session.diagnostics_for(&subject).collect();
    assert_eq!(borrowed.len(), 1);
    assert_eq!(borrowed[0].code, DiagnosticCode::ConfigInvalid);
    assert!(std::ptr::eq(borrowed[0], &session.diagnostics()[0]));
    assert_eq!(session.changed_subjects(), &[subject.clone()].into());
    assert!(session.changed_nodes().is_empty());
    assert_eq!(
        session
            .diagnostics_for(&Subject::new(Some(node_id(8)), None, None))
            .count(),
        0
    );

    let layout = document
        .apply([DocCommand::Layout(LayoutCommand::Set {
            path: vec!["node7".into()],
            value: Some(json!({"x":40})),
        })])
        .unwrap();
    let stats = *session.update(
        &document,
        &registry,
        &layout.change_set,
        AnalysisOptions::default(),
    );
    assert_eq!(stats.work, SessionWork::None);
    assert_eq!(
        (
            stats.local.nodes_visited,
            stats.local.config_computed,
            stats.local.config_hits
        ),
        (0, 0, 0)
    );
    assert!(session.changed_subjects().is_empty());
    assert_eq!(session.analysis(), &full(&document, &registry));

    let repaired = document.apply([config(7, json!(false))]).unwrap();
    let stats = *session.update(
        &document,
        &registry,
        &repaired.change_set,
        AnalysisOptions::default(),
    );
    assert_eq!(
        (stats.local.config_computed, stats.local.config_hits),
        (1, 0)
    );
    assert!(session.diagnostics().is_empty());
    assert_eq!(session.changed_subjects(), &[subject.clone()].into());
    assert_eq!(session.diagnostics_for(&subject).count(), 0);
    assert_eq!(session.analysis(), &full(&document, &registry));

    let undone = document.revert(repaired.inverse).unwrap();
    session.update(
        &document,
        &registry,
        &undone.change_set,
        AnalysisOptions::default(),
    );
    assert_eq!(session.analysis(), &full(&document, &registry));
    assert_eq!(session.changed_subjects(), &[subject].into());
    assert_eq!(session.diagnostics()[0].code, DiagnosticCode::ConfigInvalid);
}

#[test]
fn session_trial_planning_reuses_the_index_without_publishing_trial_analysis() {
    let registry = common::generic::registry(None);
    let mut document = common::generic::document(
        vec![
            common::generic::typed_source(1, "i64"),
            common::generic::add(2, json!({})),
        ],
        vec![],
    );
    let mut session = AnalysisSession::new(
        &document,
        &registry,
        BTreeMap::new(),
        AnalysisOptions::default(),
    );
    let before = document.clone();
    let before_analysis = session.materialize();
    let identities = (
        document.state_id(),
        document.semantic_state_id(),
        document.revision(),
    );
    let command = DocCommand::Connect {
        edge: serde_json::from_value(common::generic::data_edge(1, (1, "value"), (2, "a")))
            .unwrap(),
    };
    let expected =
        orange_model::inference::plan_edit(&document, [command.clone()], &registry).unwrap();
    let planned = session
        .plan_edit(&mut document, [command], &registry)
        .unwrap();
    assert_eq!(planned, expected);
    assert_eq!(document, before);
    assert_eq!(
        (
            document.state_id(),
            document.semantic_state_id(),
            document.revision()
        ),
        identities
    );
    assert_eq!(session.analysis(), &before_analysis);
    assert!(session.index().unwrap().is_current(&document));

    let failed = session.plan_edit(
        &mut document,
        [DocCommand::Disconnect {
            edge: common::eid(99),
        }],
        &registry,
    );
    assert!(failed.is_err());
    assert_eq!(document, before);
    assert_eq!(
        (
            document.state_id(),
            document.semantic_state_id(),
            document.revision()
        ),
        identities
    );
    assert_eq!(session.analysis(), &before_analysis);
    assert!(session.is_current(&document, &registry));
    assert!(session.index().unwrap().is_current(&document));
}

#[test]
fn published_session_and_analysis_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<AnalysisSession>();
    assert_send_sync::<ValidationAnalysis>();
}

/// Descriptive Windows release probe, not an NFR-15 gate or timing assertion.
/// Run: cargo test -p orange-model --test analysis_session_units --release --locked
/// --offline measure_config_unit_reuse -- --ignored --nocapture
#[test]
#[ignore = "manual descriptive release timing; full global passes remain"]
fn measure_config_unit_reuse() {
    use std::time::Instant;
    const WARMUPS: usize = 5;
    const SAMPLES: usize = 40;
    fn percentiles(samples: &mut [u128]) -> (f64, f64) {
        samples.sort_unstable();
        let at =
            |percent: usize| samples[(samples.len() * percent).div_ceil(100) - 1] as f64 / 1000.0;
        (at(50), at(95))
    }
    for count in [128, 1024] {
        let (mut document, registry) = fixture(count);
        let mut session = AnalysisSession::new(
            &document,
            &registry,
            BTreeMap::new(),
            AnalysisOptions::default(),
        );
        assert!(session.analysis().report().is_valid());
        let (mut full_ns, mut session_ns, mut materialize_ns) =
            (Vec::new(), Vec::new(), Vec::new());
        for sample in 0..WARMUPS + SAMPLES {
            let change = document.apply([config(1, json!(sample % 2 == 0))]).unwrap();
            let start = Instant::now();
            let oracle = full(&document, &registry);
            let full_elapsed = start.elapsed().as_nanos();
            let start = Instant::now();
            let stats = *session.update(
                &document,
                &registry,
                &change.change_set,
                AnalysisOptions::default(),
            );
            let session_elapsed = start.elapsed().as_nanos();
            let start = Instant::now();
            let materialized = std::hint::black_box(session.materialize());
            let materialize_elapsed = start.elapsed().as_nanos();
            // Correctness checks and rendering are outside all timed sections.
            assert_eq!(materialized, oracle);
            assert_eq!(stats.local.config_computed, 1);
            assert_eq!(stats.local.config_hits, count - 1);
            assert_eq!(stats.local.nodes_visited, count);
            assert_eq!(stats.full_analysis_runs, 1);
            if sample >= WARMUPS {
                full_ns.push(full_elapsed);
                session_ns.push(session_elapsed);
                materialize_ns.push(materialize_elapsed);
            }
        }
        let full = percentiles(&mut full_ns);
        let session = percentiles(&mut session_ns);
        let materialize = percentiles(&mut materialize_ns);
        println!(
            "config units={count} warmups={WARMUPS} samples={SAMPLES} full_us_p50_p95={full:?} session_us_p50_p95={session:?} materialize_us_p50_p95={materialize:?}; per edit: config computed=1 hits={} visited={count}, full_global_runs=1; allocations/retained-memory not measured",
            count - 1
        );
    }
}
