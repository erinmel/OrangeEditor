//! V12b: a literal or data-edge edit is re-checked only where it can reach and patched into
//! the retained report; every case that might change more falls back to the full analysis
//! (16 §3).
#[path = "support/data_graph.rs"]
#[allow(dead_code)]
mod support;

use std::collections::BTreeMap;

use orange_model::catalog::{RegistryBuilder, RegistrySnapshot};
use orange_model::document::{Edge, NodeInstance, PortRef};
use orange_model::validation::{
    AnalysisLimits, AnalysisOptions, AnalysisSession, SessionWork, analyze_with_options,
};
use orange_model::{BodyRef, DocCommand, GraphDocument};
use serde_json::{Value as Json, json};
use support::*;

fn set(node: u32, port: &str, value: Option<Json>) -> DocCommand {
    DocCommand::SetLiteral {
        port: PortRef {
            node: id(node).parse().unwrap(),
            port: port.parse().unwrap(),
        },
        literal: value.map(|value| serde_json::from_value(value).unwrap()),
    }
}

fn u64_text(value: &str) -> Option<Json> {
    Some(json!({"type": "core/u64@1", "value": value}))
}

/// Applies `commands`, updates the session and checks it against the full analysis.
fn edit(
    session: &mut AnalysisSession,
    doc: &mut GraphDocument,
    registry: &RegistrySnapshot,
    options: AnalysisOptions<'_>,
    commands: Vec<DocCommand>,
) -> SessionWork {
    let committed = doc.apply(commands).unwrap();
    let work = session
        .update(doc, registry, &committed.change_set, options)
        .work;
    assert_eq!(
        session.analysis(),
        &analyze_with_options(doc, registry, BTreeMap::new(), options)
    );
    work
}

/// Ten chains of three adds; every `a` but the first in a chain is wired.
fn chains() -> (GraphDocument, std::sync::Arc<RegistrySnapshot>) {
    let (package, registry) = catalog();
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for chain in 0..10 {
        let first = 10 + chain * 3;
        for link in 0..3 {
            nodes.push(math(first + link, "add", 0, 1));
            if link > 0 {
                edges.push(edge(
                    first + link,
                    first + link - 1,
                    "sum",
                    first + link,
                    "a",
                ));
            }
        }
    }
    (graph(&package, nodes, edges, vec![]), registry)
}

fn session(doc: &GraphDocument, registry: &RegistrySnapshot) -> AnalysisSession {
    let session = AnalysisSession::new(doc, registry, BTreeMap::new(), AnalysisOptions::default());
    assert!(session.analysis().report().is_valid());
    session
}

#[test]
fn literals_that_keep_every_signature_patch_only_their_inputs() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(11, "b", u64_text("5"))],
    );
    assert_eq!(work, SessionWork::Scoped);
    let stats = *session.stats();
    assert_eq!(stats.full_analysis_runs, 0);
    // The chain's three adds, and nothing from the other nine chains.
    assert_eq!(stats.input_nodes, 3);
    assert!(session.changed_nodes().is_empty());
    assert!(session.changed_subjects().is_empty());

    // Removing a wired literal drops its shadowed warning; restoring it brings it back.
    let shadowed = session.diagnostics().len();
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(11, "a", None)],
    );
    assert_eq!(work, SessionWork::Scoped);
    assert_eq!(session.diagnostics().len(), shadowed - 1);
    assert_eq!(session.changed_subjects().len(), 1);
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(11, "a", u64_text("2"))],
    );
    assert_eq!(work, SessionWork::Scoped);
    assert_eq!(session.diagnostics().len(), shadowed);

    // Undo and redo are literal edits too.
    let committed = doc.apply([set(14, "b", u64_text("3"))]).unwrap();
    session.update(&doc, &registry, &committed.change_set, options);
    let undone = doc.revert(committed.inverse).unwrap();
    session.update(&doc, &registry, &undone.change_set, options);
    assert_eq!(session.stats().work, SessionWork::Scoped);
    assert_eq!(
        session.analysis(),
        &analyze_with_options(&doc, &registry, BTreeMap::new(), options)
    );
}

#[test]
fn a_literal_that_changes_inference_runs_the_full_analysis() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    // An i64 literal conflicts with the chain's u64 literals.
    let other = Some(json!({"type": "core/i64@1", "value": "1"}));
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(10, "b", other)],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(10, "b", u64_text("1"))],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(session.analysis().report().is_valid());
}

#[test]
fn input_errors_are_patched_in_place_unless_a_node_is_incomplete() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    // In a complete report an input error suppresses nothing, so gaining, changing and
    // losing one are all scoped.
    for value in ["x", "y", "1"] {
        let work = edit(
            &mut session,
            &mut doc,
            &registry,
            options,
            vec![set(11, "b", u64_text(value))],
        );
        assert_eq!(work, SessionWork::Scoped);
    }
    assert!(session.analysis().report().is_valid());
    // A structure edit beside an input error is scoped too.
    edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(11, "b", u64_text("x"))],
    );
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![disconnect(14)],
    );
    assert_eq!(work, SessionWork::Scoped);
    // Once a node is incomplete, an error appearing or going could change which
    // dependency diagnostics are suppressed, so that runs in full.
    let mut unknown = math(60, "add", 0, 1);
    unknown["definition_id"] = json!("org.orange.math/unknown");
    let unknown = DocCommand::AddNode {
        body: BodyRef::Root,
        node: serde_json::from_value(unknown).unwrap(),
    };
    edit(&mut session, &mut doc, &registry, options, vec![unknown]);
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(11, "b", u64_text("1"))],
    );
    assert_eq!(work, SessionWork::Recompute);
}

#[test]
fn literals_with_structural_edits_run_the_full_analysis() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let connect = DocCommand::Connect {
        edge: serde_json::from_value::<Edge>(edge(99, 12, "sum", 13, "a")).unwrap(),
    };
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(11, "b", u64_text("1")), connect],
    );
    assert_eq!(work, SessionWork::Recompute);
    let node: NodeInstance = serde_json::from_value(math(90, "add", 1, 2)).unwrap();
    let add = DocCommand::AddNode {
        body: BodyRef::Root,
        node,
    };
    // A new node beside a changed one is neither a node edit nor a structure edit.
    let both = vec![add, set(11, "b", u64_text("2"))];
    assert_eq!(
        edit(&mut session, &mut doc, &registry, options, both),
        SessionWork::Recompute
    );
}

#[test]
fn a_literal_past_the_admission_quota_is_rejected_as_in_full() {
    let (mut doc, registry) = chains();
    let limited = |bytes: usize| AnalysisOptions {
        limits: AnalysisLimits {
            max_metadata_bytes: bytes,
            ..AnalysisLimits::default()
        },
        cancelled: None,
    };
    let fits = |bytes: usize, doc: &GraphDocument| {
        analyze_with_options(doc, &registry, BTreeMap::new(), limited(bytes))
            .report()
            .is_valid()
    };
    // The smallest metadata quota the document fits.
    let (mut low, mut high) = (0, 1 << 20);
    assert!(fits(high, &doc));
    while low + 1 < high {
        let middle = (low + high) / 2;
        if fits(middle, &doc) {
            high = middle;
        } else {
            low = middle;
        }
    }
    let options = limited(high + 2);
    let mut session = AnalysisSession::new(&doc, &registry, BTreeMap::new(), options);
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(11, "b", u64_text("12"))],
    );
    assert_eq!(work, SessionWork::Scoped);
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(11, "b", u64_text("123456789"))],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
}

fn connect(n: u32, from: u32, to: u32, input: &str) -> DocCommand {
    DocCommand::Connect {
        edge: serde_json::from_value::<Edge>(edge(n, from, "sum", to, input)).unwrap(),
    }
}

fn disconnect(n: u32) -> DocCommand {
    DocCommand::Disconnect {
        edge: serde_json::from_value::<Edge>(edge(n, 0, "sum", 0, "a"))
            .unwrap()
            .id,
    }
}

#[test]
fn data_edges_between_pure_nodes_patch_only_their_targets() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let shadowed = session.diagnostics().len();
    // The wired `a` literal is no longer shadowed once its edge goes.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![disconnect(11)],
    );
    assert_eq!(work, SessionWork::Scoped);
    assert_eq!(session.stats().full_analysis_runs, 0);
    assert_eq!(session.diagnostics().len(), shadowed - 1);
    assert!(session.changed_nodes().is_empty());
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![connect(11, 10, 11, "a")],
    );
    assert_eq!(work, SessionWork::Scoped);
    assert_eq!(session.diagnostics().len(), shadowed);

    // Joining two chains merges their components without changing a signature.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![connect(90, 12, 13, "a")],
    );
    assert_eq!(work, SessionWork::Scoped);
    // A second consumer of one output is fine for a plain value.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![connect(91, 12, 16, "a")],
    );
    assert_eq!(work, SessionWork::Scoped);

    // Undo and redo of an edge are edge edits too.
    let committed = doc.apply([connect(92, 18, 19, "a")]).unwrap();
    session.update(&doc, &registry, &committed.change_set, options);
    assert_eq!(session.stats().work, SessionWork::Scoped);
    let undone = doc.revert(committed.inverse).unwrap();
    session.update(&doc, &registry, &undone.change_set, options);
    assert_eq!(session.stats().work, SessionWork::Scoped);
    assert_eq!(
        session.analysis(),
        &analyze_with_options(&doc, &registry, BTreeMap::new(), options)
    );
}

#[test]
fn edges_that_may_change_more_run_the_full_analysis() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    // A second wire into a wired input.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![connect(90, 13, 11, "a")],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
    // Nothing is checked locally while the report has an error.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![connect(91, 15, 16, "b")],
    );
    assert_eq!(work, SessionWork::Recompute);
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![disconnect(90)],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(session.analysis().report().is_valid());

    // An edge closing a cycle.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![connect(92, 12, 10, "a")],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
}

#[test]
fn edges_feeding_an_export_are_scoped() {
    let (package, registry) = catalog();
    let nodes = (0..3).map(|link| math(10 + link, "add", 0, 1)).collect();
    let edges = vec![edge(11, 10, "sum", 11, "a")];
    let mut doc = graph(&package, nodes, edges, vec![export(12, "sum")]);
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![connect(12, 11, 12, "a")],
    );
    assert_eq!(work, SessionWork::Scoped);
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![disconnect(11)],
    );
    assert_eq!(work, SessionWork::Scoped);
}

#[test]
fn edges_that_close_a_cycle_after_reordering_run_the_full_analysis() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    // Chains 10-12, 13-15 and 16-18 feed one another in both id directions, so whatever
    // order the first edit builds, a later one runs against it.
    for (n, from, to) in [(90, 15, 10), (91, 12, 16), (92, 27, 19), (93, 24, 28)] {
        let work = edit(
            &mut session,
            &mut doc,
            &registry,
            options,
            vec![connect(n, from, to, "a")],
        );
        assert_eq!(work, SessionWork::Scoped);
    }
    // 18 -> 13 closes 13..15 -> 10..12 -> 16..18 -> 13.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![connect(94, 18, 13, "a")],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
}

#[test]
fn a_cycle_in_a_body_listed_out_of_id_order_runs_the_full_analysis() {
    // A body keeps its nodes in insertion order, which the data order must not rely on.
    let (package, registry) = catalog();
    let nodes = (10..13).chain(1..6).map(|n| math(n, "add", 0, 1)).collect();
    let edges = vec![edge(11, 10, "sum", 11, "a"), edge(12, 11, "sum", 12, "a")];
    let mut doc = graph(&package, nodes, edges, vec![]);
    let mut session = session(&doc, &registry);
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        AnalysisOptions::default(),
        vec![connect(13, 12, 10, "a")],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
}

#[test]
fn random_rewiring_equals_the_full_analysis() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    // A seed whose batches reach the pending-edge case.
    let mut seed: u64 = 23;
    let mut next = |bound: u32| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as u32 % bound
    };
    let mut wired: BTreeMap<(u32, &str), u32> = BTreeMap::new();
    for chain in 0..10 {
        for link in 1..3 {
            let to = 10 + chain * 3 + link;
            wired.insert((to, "a"), to);
        }
    }
    let mut scoped = 0;
    let mut n = 100;
    for _ in 0..300 {
        // One to three rewirings per edit, each of a different input.
        let mut commands = Vec::new();
        let mut inputs = Vec::new();
        for _ in 0..=next(3) {
            let (from, to) = (10 + next(30), 10 + next(30));
            let input = if next(2) == 0 { "a" } else { "b" };
            if inputs.contains(&(to, input)) {
                continue;
            }
            inputs.push((to, input));
            commands.push(match wired.remove(&(to, input)) {
                Some(edge) => disconnect(edge),
                None if from != to => {
                    n += 1;
                    wired.insert((to, input), n);
                    connect(n, from, to, input)
                }
                None => continue,
            });
        }
        if !commands.is_empty()
            && edit(&mut session, &mut doc, &registry, options, commands) == SessionWork::Scoped
        {
            scoped += 1;
        }
    }
    assert!(scoped >= 30, "{scoped}");
}

#[test]
fn an_edge_from_an_event_payload_runs_the_full_analysis() {
    let (package, registry) = catalog();
    let event = "40000000-0000-4000-8000-000000000001";
    let on_event = json!({"id": id(50), "definition_id": "core/on-event",
        "definition_version": 1, "config_version": 1, "config": {"event": event},
        "activation": {"mode": "queue", "capacity": 4, "overflow": "drop-oldest",
            "on_fault": "continue", "order": 0}});
    let base = graph(
        &package,
        vec![on_event, math(60, "add", 0, 1)],
        vec![],
        vec![],
    );
    let mut raw: Json = serde_json::from_str(&orange_model::to_json(&base).unwrap()).unwrap();
    raw["events"] = json!([{"id": event, "name": "tick", "payload": "core/u64@1", "scope": "run"}]);
    let mut doc = orange_model::from_json(&raw.to_string()).unwrap();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let payload = DocCommand::Connect {
        edge: serde_json::from_value::<Edge>(edge(61, 50, "payload", 60, "a")).unwrap(),
    };
    // The payload carries an availability requirement to whatever consumes the add.
    let work = edit(&mut session, &mut doc, &registry, options, vec![payload]);
    assert_eq!(work, SessionWork::Recompute);
}

#[test]
fn an_event_payload_reaching_an_export_runs_the_full_analysis() {
    let (package, registry) = catalog();
    let event = "40000000-0000-4000-8000-000000000001";
    let on_event = json!({"id": id(50), "definition_id": "core/on-event",
        "definition_version": 1, "config_version": 1, "config": {"event": event},
        "activation": {"mode": "queue", "capacity": 4, "overflow": "drop-oldest",
            "on_fault": "continue", "order": 0}});
    let base = graph(
        &package,
        vec![on_event, math(60, "add", 0, 1), math(61, "add", 0, 1)],
        vec![edge(62, 60, "sum", 61, "a")],
        vec![export(61, "sum")],
    );
    let mut raw: Json = serde_json::from_str(&orange_model::to_json(&base).unwrap()).unwrap();
    raw["events"] = json!([{"id": event, "name": "tick", "payload": "core/u64@1", "scope": "run"}]);
    let mut doc = orange_model::from_json(&raw.to_string()).unwrap();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let payload = DocCommand::Connect {
        edge: serde_json::from_value::<Edge>(edge(63, 50, "payload", 60, "a")).unwrap(),
    };
    let work = edit(&mut session, &mut doc, &registry, options, vec![payload]);
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
}

fn types(pairs: &[(&str, &str)]) -> BTreeMap<String, orange_model::id::TypeExpr> {
    pairs
        .iter()
        .map(|(name, ty)| ((*name).to_owned(), ty.parse().unwrap()))
        .collect()
}

#[test]
fn type_arguments_and_pins_that_keep_inference_are_scoped() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let node = || id(11).parse().unwrap();
    // Persisted arguments are an inference cache; only admission reads them.
    for ty in ["core/u64@1", "core/i64@1"] {
        let args = DocCommand::SetTypeArgs {
            node: node(),
            args: types(&[("T", ty)]),
        };
        let work = edit(&mut session, &mut doc, &registry, options, vec![args]);
        assert_eq!(work, SessionWork::Scoped);
    }
    // Setting the same arguments or literal again commits an edit that changes nothing.
    let again = vec![
        DocCommand::SetTypeArgs {
            node: node(),
            args: types(&[("T", "core/i64@1")]),
        },
        set(11, "b", u64_text("1")),
    ];
    let work = edit(&mut session, &mut doc, &registry, options, again);
    assert_eq!(work, SessionWork::Scoped);
    // A pin agreeing with the chain's literals changes no signature.
    let pin = |ty| DocCommand::SetTypePins {
        node: node(),
        pins: types(&[("T", ty)]),
    };
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![pin("core/u64@1")],
    );
    assert_eq!(work, SessionWork::Scoped);
    assert_eq!(session.stats().input_nodes, 3);
    // A conflicting pin changes inference.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![pin("core/i64@1")],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
    let clear = DocCommand::SetTypePins {
        node: node(),
        pins: BTreeMap::new(),
    };
    let work = edit(&mut session, &mut doc, &registry, options, vec![clear]);
    assert_eq!(work, SessionWork::Recompute);
    assert!(session.analysis().report().is_valid());
    // A literal and a pin in one transaction are still one node edit.
    let both = vec![set(11, "b", u64_text("4")), pin("core/u64@1")];
    let work = edit(&mut session, &mut doc, &registry, options, both);
    assert_eq!(work, SessionWork::Scoped);
}

fn configure(node: u32, config: Json) -> DocCommand {
    DocCommand::SetConfig {
        node: id(node).parse().unwrap(),
        config_version: 1,
        config: serde_json::from_value(config).unwrap(),
    }
}

#[test]
fn package_node_configuration_is_scoped() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let saturating = configure(11, json!({"int_overflow": "saturating"}));
    let work = edit(&mut session, &mut doc, &registry, options, vec![saturating]);
    assert_eq!(work, SessionWork::Scoped);
    assert_eq!(session.stats().full_analysis_runs, 0);
    // An invalid value adds a configuration error; nothing is incomplete, so it is scoped.
    let invalid = configure(11, json!({"int_overflow": "sometimes"}));
    let work = edit(&mut session, &mut doc, &registry, options, vec![invalid]);
    assert_eq!(work, SessionWork::Scoped);
    assert!(!session.analysis().report().is_valid());
    let other = configure(11, json!({"int_overflow": "never"}));
    let work = edit(&mut session, &mut doc, &registry, options, vec![other]);
    assert_eq!(work, SessionWork::Scoped);
    let valid = configure(11, json!({"int_overflow": "checked"}));
    let work = edit(&mut session, &mut doc, &registry, options, vec![valid]);
    assert_eq!(work, SessionWork::Scoped);
    assert!(session.analysis().report().is_valid());
    // Configuration and a literal together are one node edit.
    let both = vec![
        configure(14, json!({"int_overflow": "saturating"})),
        set(14, "b", u64_text("6")),
    ];
    let work = edit(&mut session, &mut doc, &registry, options, both);
    assert_eq!(work, SessionWork::Scoped);
}

#[test]
fn intrinsic_configuration_runs_the_full_analysis() {
    let (package, registry) = catalog();
    let on_start = json!({"id": id(50), "definition_id": "core/on-start",
        "definition_version": 1, "config_version": 1, "config": {},
        "activation": {"mode": "queue", "capacity": 4, "overflow": "drop-newest",
            "on_fault": "continue", "order": 0}});
    let mut doc = graph(
        &package,
        vec![on_start, math(60, "add", 0, 1)],
        vec![],
        vec![],
    );
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![configure(50, json!({"x": 1}))],
    );
    assert_eq!(work, SessionWork::Recompute);
}

fn add_node(n: u32) -> DocCommand {
    DocCommand::AddNode {
        body: BodyRef::Root,
        node: serde_json::from_value(math(n, "add", 1, 2)).unwrap(),
    }
}

fn remove_nodes(ids: &[u32]) -> DocCommand {
    DocCommand::RemoveNodes {
        ids: ids.iter().map(|n| id(*n).parse().unwrap()).collect(),
    }
}

#[test]
fn pure_package_nodes_are_added_and_removed_in_place() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let new: orange_model::id::NodeId = id(90).parse().unwrap();
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![add_node(90)],
    );
    assert_eq!(work, SessionWork::Scoped);
    assert_eq!(session.stats().full_analysis_runs, 0);
    assert!(session.resolved(new).is_some());
    assert_eq!(
        session.changed_nodes().iter().copied().collect::<Vec<_>>(),
        [new]
    );
    // A pasted node with its wire, then its removal with that wire.
    let pasted = vec![add_node(91), connect(91, 12, 91, "a")];
    let work = edit(&mut session, &mut doc, &registry, options, pasted);
    assert_eq!(work, SessionWork::Scoped);
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![remove_nodes(&[91])],
    );
    assert_eq!(work, SessionWork::Scoped);
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![remove_nodes(&[90])],
    );
    assert_eq!(work, SessionWork::Scoped);
    assert!(session.resolved(new).is_none());

    // Removing a chain's middle node takes both its wires; undo restores all three.
    let committed = doc.apply([remove_nodes(&[11])]).unwrap();
    session.update(&doc, &registry, &committed.change_set, options);
    assert_eq!(session.stats().work, SessionWork::Scoped);
    assert_eq!(
        session.analysis(),
        &analyze_with_options(&doc, &registry, BTreeMap::new(), options)
    );
    let undone = doc.revert(committed.inverse).unwrap();
    session.update(&doc, &registry, &undone.change_set, options);
    assert_eq!(session.stats().work, SessionWork::Scoped);
    assert_eq!(
        session.analysis(),
        &analyze_with_options(&doc, &registry, BTreeMap::new(), options)
    );
}

#[test]
fn other_node_additions_and_removals_run_the_full_analysis() {
    let (package, registry) = catalog();
    let nodes = (0..3).map(|link| math(10 + link, "add", 0, 1)).collect();
    let mut doc = graph(&package, nodes, vec![], vec![export(12, "sum")]);
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    // A zone marker, which only the whole region pass pairs.
    let marker: NodeInstance = serde_json::from_value(json!({"id": id(50),
        "definition_id": "core/repeat-begin", "definition_version": 1, "config_version": 1,
        "config": {"zone": "z"}}))
    .unwrap();
    let marker = DocCommand::AddNode {
        body: BodyRef::Root,
        node: marker,
    };
    let work = edit(&mut session, &mut doc, &registry, options, vec![marker]);
    assert_eq!(work, SessionWork::Recompute);
    // Undo it so the report is clean again, then remove an exported node.
    let committed = doc.apply([remove_nodes(&[50])]).unwrap();
    session.update(&doc, &registry, &committed.change_set, options);
    assert!(session.analysis().report().is_valid());
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![remove_nodes(&[12])],
    );
    assert_eq!(work, SessionWork::Recompute);
    // A package node that does not resolve.
    let mut unknown = math(60, "add", 0, 1);
    unknown["definition_id"] = json!("org.orange.math/unknown");
    let unknown = DocCommand::AddNode {
        body: BodyRef::Root,
        node: serde_json::from_value(unknown).unwrap(),
    };
    let work = edit(&mut session, &mut doc, &registry, options, vec![unknown]);
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
}

/// On Start runs two Set Variable nodes in turn, each storing an add's sum.
fn exec_chain() -> (GraphDocument, std::sync::Arc<RegistrySnapshot>) {
    let (package, registry) = catalog();
    let variable = id(100);
    let core = |n: u32, name: &str| {
        let mut node = json!({"id": id(n), "definition_id": format!("core/{name}"),
            "definition_version": 1, "config_version": 1, "config": {}});
        if name == "on-start" {
            node["activation"] = json!({"mode": "queue", "capacity": 4,
                "overflow": "drop-newest", "on_fault": "continue", "order": 0});
        } else {
            node["config"] = json!({"variable": variable});
        }
        node
    };
    let nodes = vec![
        core(1, "on-start"),
        core(2, "set-variable"),
        core(3, "set-variable"),
        math(10, "add", 1, 2),
        math(11, "add", 3, 4),
    ];
    let edges = vec![
        edge(1, 1, "then", 2, "in"),
        edge(2, 2, "then", 3, "in"),
        edge(3, 10, "sum", 2, "value"),
        edge(4, 11, "sum", 3, "value"),
    ];
    let base = graph(&package, nodes, edges, vec![]);
    let mut raw: Json = serde_json::from_str(&orange_model::to_json(&base).unwrap()).unwrap();
    raw["variables"] = json!([{"id": variable, "name": "total", "type": "core/u64@1",
        "scope": "run", "init": {"type": "core/u64@1", "value": "0"}}]);
    (orange_model::from_json(&raw.to_string()).unwrap(), registry)
}

fn wire(n: u32, from: u32, output: &str, to: u32, input: &str) -> DocCommand {
    DocCommand::Connect {
        edge: serde_json::from_value::<Edge>(edge(n, from, output, to, input)).unwrap(),
    }
}

#[test]
fn exec_edges_rerun_only_the_control_flow_passes() {
    let (mut doc, registry) = exec_chain();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let before = session.diagnostics().len();
    // The second Set becomes unreachable, a warning the region pass adds.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![disconnect(2)],
    );
    assert_eq!(work, SessionWork::Scoped);
    assert_eq!(session.stats().full_analysis_runs, 0);
    assert_eq!(session.diagnostics().len(), before + 1);
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![wire(2, 2, "then", 3, "in")],
    );
    assert_eq!(work, SessionWork::Scoped);
    assert_eq!(session.diagnostics().len(), before);
}

#[test]
fn exec_edges_that_may_change_more_run_the_full_analysis() {
    let (mut doc, registry) = exec_chain();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    // A second connection on one exec output.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![wire(5, 1, "then", 3, "in")],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![disconnect(5)],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(session.analysis().report().is_valid());
    // A control cycle.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![wire(6, 3, "then", 2, "in")],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![disconnect(6)],
    );
    assert_eq!(work, SessionWork::Recompute);
    // Exec and data edges together rerun the control-flow passes for both.
    let mixed = vec![disconnect(2), disconnect(4)];
    let work = edit(&mut session, &mut doc, &registry, options, mixed);
    assert_eq!(work, SessionWork::Scoped);
}

/// Two On Start activations, each running two Set Variable nodes fed by adds.
fn two_activations() -> (GraphDocument, std::sync::Arc<RegistrySnapshot>) {
    let (package, registry) = catalog();
    let variable = id(100);
    let start = |n: u32, order: u32| {
        json!({"id": id(n), "definition_id": "core/on-start", "definition_version": 1,
            "config_version": 1, "config": {}, "activation": {"mode": "queue", "capacity": 4,
            "overflow": "drop-newest", "on_fault": "continue", "order": order}})
    };
    let set = |n: u32| {
        json!({"id": id(n), "definition_id": "core/set-variable", "definition_version": 1,
            "config_version": 1, "config": {"variable": variable}})
    };
    let nodes = vec![
        start(1, 0),
        set(2),
        set(3),
        start(4, 1),
        set(5),
        set(6),
        math(10, "add", 1, 2),
        math(11, "add", 3, 4),
        math(12, "add", 5, 6),
        math(13, "add", 7, 8),
    ]
    .into_iter()
    // Unrelated nodes keep each activation a small part of the body.
    .chain((40..60).map(|n| math(n, "add", 0, 1)))
    .collect();
    let edges = vec![
        edge(1, 1, "then", 2, "in"),
        edge(2, 2, "then", 3, "in"),
        edge(4, 4, "then", 5, "in"),
        edge(5, 5, "then", 6, "in"),
        edge(20, 10, "sum", 2, "value"),
        edge(21, 11, "sum", 3, "value"),
        edge(22, 12, "sum", 5, "value"),
        edge(23, 13, "sum", 6, "value"),
    ];
    let base = graph(&package, nodes, edges, vec![]);
    let mut raw: Json = serde_json::from_str(&orange_model::to_json(&base).unwrap()).unwrap();
    raw["variables"] = json!([{"id": variable, "name": "total", "type": "core/u64@1",
        "scope": "run", "init": {"type": "core/u64@1", "value": "0"}}]);
    (orange_model::from_json(&raw.to_string()).unwrap(), registry)
}

#[test]
fn exec_edits_rerun_only_the_activations_they_join() {
    let (mut doc, registry) = two_activations();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    // Within one activation.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![disconnect(5)],
    );
    assert_eq!(work, SessionWork::Scoped);
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![wire(5, 5, "then", 6, "in")],
    );
    assert_eq!(work, SessionWork::Scoped);
    // A Set moved from the first activation to the end of the second.
    let moved = vec![disconnect(2), wire(30, 6, "then", 3, "in")];
    let work = edit(&mut session, &mut doc, &registry, options, moved);
    assert_eq!(work, SessionWork::Scoped);
    // Joining the activations' chains, then splitting them again.
    let undone = vec![disconnect(30), wire(2, 2, "then", 3, "in")];
    let work = edit(&mut session, &mut doc, &registry, options, undone);
    assert_eq!(work, SessionWork::Scoped);
}

/// An On Event activation whose Set reads the payload through two adds, and an On Start
/// activation with one Set, among unrelated adds.
fn payload_reader() -> (GraphDocument, std::sync::Arc<RegistrySnapshot>) {
    let (package, registry) = catalog();
    let (event, variable) = ("40000000-0000-4000-8000-000000000001", id(100));
    let nodes = vec![
        json!({"id": id(1), "definition_id": "core/on-event", "definition_version": 1,
            "config_version": 1, "config": {"event": event},
            "activation": {"mode": "queue", "capacity": 4, "overflow": "drop-oldest",
                "on_fault": "continue", "order": 0}}),
        json!({"id": id(2), "definition_id": "core/set-variable", "definition_version": 1,
            "config_version": 1, "config": {"variable": variable}}),
        json!({"id": id(3), "definition_id": "core/on-start", "definition_version": 1,
            "config_version": 1, "config": {},
            "activation": {"mode": "queue", "capacity": 4, "overflow": "drop-newest",
                "on_fault": "continue", "order": 1}}),
        json!({"id": id(4), "definition_id": "core/set-variable", "definition_version": 1,
            "config_version": 1, "config": {"variable": variable}}),
        math(10, "add", 0, 1),
        math(11, "add", 0, 1),
        math(12, "add", 0, 1),
    ]
    .into_iter()
    // Unrelated nodes keep the edited activations a small part of the body.
    .chain((20..40).map(|n| math(n, "add", 0, 1)))
    .collect();
    let edges = vec![
        edge(1, 1, "then", 2, "in"),
        // The payload reaches the Set through two adds, so only the cone expansion finds it.
        edge(2, 1, "payload", 10, "a"),
        edge(7, 10, "sum", 12, "a"),
        edge(3, 12, "sum", 2, "value"),
        edge(4, 3, "then", 4, "in"),
        edge(5, 11, "sum", 4, "value"),
    ];
    let base = graph(&package, nodes, edges, vec![]);
    let mut raw: Json = serde_json::from_str(&orange_model::to_json(&base).unwrap()).unwrap();
    raw["variables"] = json!([{"id": variable, "name": "total", "type": "core/u64@1",
        "scope": "run", "init": {"type": "core/u64@1", "value": "0"}}]);
    raw["events"] = json!([{"id": event, "name": "tick", "payload": "core/u64@1", "scope": "run"}]);
    (orange_model::from_json(&raw.to_string()).unwrap(), registry)
}

#[test]
fn moving_a_reader_of_another_activation_runs_the_full_analysis() {
    let (mut doc, registry) = payload_reader();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    // The Set reading the payload through an add moves to the On Start activation.
    let moved = vec![disconnect(1), wire(6, 4, "then", 2, "in")];
    let work = edit(&mut session, &mut doc, &registry, options, moved);
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
}

fn set_node(n: u32) -> DocCommand {
    DocCommand::AddNode {
        body: BodyRef::Root,
        node: serde_json::from_value(json!({"id": id(n), "definition_id": "core/set-variable",
            "definition_version": 1, "config_version": 1, "config": {"variable": id(100)}}))
        .unwrap(),
    }
}

#[test]
fn exec_nodes_are_added_and_removed_in_place() {
    let (mut doc, registry) = two_activations();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    // A Set pasted onto the end of the second activation, with its exec and data wires.
    let pasted = vec![
        set_node(7),
        wire(30, 6, "then", 7, "in"),
        wire(31, 13, "sum", 7, "value"),
    ];
    let work = edit(&mut session, &mut doc, &registry, options, pasted);
    assert_eq!(work, SessionWork::Scoped);
    assert_eq!(session.stats().full_analysis_runs, 0);
    // Removing the second activation's first Set leaves the rest unreachable; undo restores it.
    let committed = doc.apply([remove_nodes(&[5])]).unwrap();
    session.update(&doc, &registry, &committed.change_set, options);
    assert_eq!(session.stats().work, SessionWork::Scoped);
    assert_eq!(
        session.analysis(),
        &analyze_with_options(&doc, &registry, BTreeMap::new(), options)
    );
    let undone = doc.revert(committed.inverse).unwrap();
    session.update(&doc, &registry, &undone.change_set, options);
    assert_eq!(session.stats().work, SessionWork::Scoped);
    assert_eq!(
        session.analysis(),
        &analyze_with_options(&doc, &registry, BTreeMap::new(), options)
    );
    // A new, unwired On Start is an intrinsic entry.
    let start: NodeInstance = serde_json::from_value(json!({"id": id(60),
        "definition_id": "core/on-start", "definition_version": 1, "config_version": 1,
        "config": {}, "activation": {"mode": "queue", "capacity": 4,
            "overflow": "drop-newest", "on_fault": "continue", "order": 2}}))
    .unwrap();
    let start = DocCommand::AddNode {
        body: BodyRef::Root,
        node: start,
    };
    let work = edit(&mut session, &mut doc, &registry, options, vec![start]);
    assert_eq!(work, SessionWork::Scoped);
}

#[test]
fn a_flow_edit_reruns_the_consumers_its_data_edges_reach() {
    let (mut doc, registry) = payload_reader();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    // A new On Start makes this a control-flow edit; the payload wired into the add that
    // feeds the On Start activation's Set gives that Set a cross-activation read.
    let start: NodeInstance = serde_json::from_value(json!({"id": id(60),
        "definition_id": "core/on-start", "definition_version": 1, "config_version": 1,
        "config": {}, "activation": {"mode": "queue", "capacity": 4,
            "overflow": "drop-newest", "on_fault": "continue", "order": 2}}))
    .unwrap();
    let both = vec![
        DocCommand::AddNode {
            body: BodyRef::Root,
            node: start,
        },
        wire(8, 1, "payload", 11, "b"),
    ];
    let work = edit(&mut session, &mut doc, &registry, options, both);
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
}

/// On Start calls a graph-local action that stores its argument in a local, reads it back
/// and returns it plus one through an add.
fn called_function() -> (GraphDocument, std::sync::Arc<RegistrySnapshot>) {
    let (package, registry) = catalog();
    let (total, local) = (id(100), id(101));
    let core = |n: u32, name: &str, config: Json| {
        json!({"id": id(n), "definition_id": format!("core/{name}"), "definition_version": 1,
            "config_version": 1, "config": config})
    };
    let port = |name: &str, direction: &str| json!({"port_id": name, "direction": direction, "kind": "data", "type": "core/u64@1"});
    let mut start = core(1, "on-start", json!({}));
    start["activation"] = json!({"mode": "queue", "capacity": 4, "overflow": "drop-newest",
        "on_fault": "continue", "order": 0});
    let nodes = vec![
        start,
        core(2, "get-variable", json!({"variable": total})),
        core(3, "call", json!({"function": id(200)})),
        core(4, "set-variable", json!({"variable": total})),
    ];
    let edges = vec![
        edge(1, 1, "then", 3, "in"),
        edge(2, 2, "value", 3, "value"),
        edge(3, 3, "then", 4, "in"),
        edge(4, 3, "result", 4, "value"),
    ];
    let base = graph(&package, nodes, edges, vec![]);
    let mut raw: Json = serde_json::from_str(&orange_model::to_json(&base).unwrap()).unwrap();
    let mut add = math(204, "add", 0, 1);
    add["provider"] = json!({"package": "org.orange.math", "version": "0.0.0", "id": "native-v1"});
    raw["functions"] = json!([{"id": id(200), "name": "increment", "kind": "action",
        "inputs": [port("value", "input")], "outputs": [port("result", "output")],
        "nodes": [core(201, "function-entry", json!({})),
            core(202, "set-variable", json!({"variable": local})),
            core(203, "get-variable", json!({"variable": local})),
            add, core(205, "return", json!({}))],
        "edges": [edge(201, 201, "then", 202, "in"), edge(202, 201, "value", 202, "value"),
            edge(203, 202, "then", 205, "in"), edge(204, 203, "value", 204, "a"),
            edge(205, 204, "sum", 205, "result")]}]);
    raw["variables"] = json!([
        {"id": total, "name": "total", "type": "core/u64@1", "scope": "run",
            "init": {"type": "core/u64@1", "value": "0"}},
        {"id": local, "name": "local", "type": "core/u64@1", "scope": "local",
            "init": {"type": "core/u64@1", "value": "0"}},
    ]);
    (orange_model::from_json(&raw.to_string()).unwrap(), registry)
}

#[test]
fn function_body_node_edits_are_scoped_while_acceptance_holds() {
    let (mut doc, registry) = called_function();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(204, "b", u64_text("5"))],
    );
    assert_eq!(work, SessionWork::Scoped);
    assert_eq!(session.stats().full_analysis_runs, 0);
    let pin = DocCommand::SetTypePins {
        node: id(204).parse().unwrap(),
        pins: types(&[("T", "core/u64@1")]),
    };
    let work = edit(&mut session, &mut doc, &registry, options, vec![pin]);
    assert_eq!(work, SessionWork::Scoped);
    // An input error inside the body blocks the function, which its caller reads.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(204, "b", u64_text("x"))],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(204, "b", u64_text("1"))],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(session.analysis().report().is_valid());
    // A literal that changes the body's inference changes a signature.
    let other = Some(json!({"type": "core/i64@1", "value": "1"}));
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(204, "b", other)],
    );
    assert_eq!(work, SessionWork::Recompute);
}

#[test]
fn function_body_structure_edits_are_scoped_while_acceptance_holds() {
    let (mut doc, registry) = called_function();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    // The add's `a` loses its wire, so its literal is no longer shadowed; the function's
    // acceptance and summary do not change.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![disconnect(204)],
    );
    assert_eq!(work, SessionWork::Scoped);
    assert_eq!(session.stats().full_analysis_runs, 0);
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![wire(204, 203, "value", 204, "a")],
    );
    assert_eq!(work, SessionWork::Scoped);
    // A pure node added to the body, then removed.
    let inside = DocCommand::AddNode {
        body: BodyRef::Function(id(200).parse().unwrap()),
        node: serde_json::from_value(math(206, "add", 1, 2)).unwrap(),
    };
    let work = edit(&mut session, &mut doc, &registry, options, vec![inside]);
    assert_eq!(work, SessionWork::Scoped);
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![remove_nodes(&[206])],
    );
    assert_eq!(work, SessionWork::Scoped);
    // Cutting the path to Return changes what the function's callers can rely on.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![disconnect(203)],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
}

#[test]
fn an_uncalled_function_publishes_its_own_changes_in_place() {
    let (mut doc, registry) = called_function();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    // Without its only call site, nothing reads the function's acceptance or summary.
    edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![remove_nodes(&[3, 4])],
    );
    assert!(session.analysis().report().is_valid());
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![disconnect(203)],
    );
    assert_eq!(work, SessionWork::Scoped);
    assert!(!session.analysis().report().is_valid());
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![wire(203, 202, "then", 205, "in")],
    );
    assert_eq!(work, SessionWork::Scoped);
    assert!(session.analysis().report().is_valid());
}

#[test]
fn a_called_function_summary_change_reaches_its_callers_in_place() {
    let (mut doc, registry) = called_function();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    // The function now also stores its argument in the run variable its caller sets: its
    // summary gains a run write, while its kind and suspension, which the call reads, hold.
    let store = DocCommand::AddNode {
        body: BodyRef::Function(id(200).parse().unwrap()),
        node: serde_json::from_value(json!({"id": id(207), "definition_id": "core/set-variable",
            "definition_version": 1, "config_version": 1, "config": {"variable": id(100)}}))
        .unwrap(),
    };
    let rewired = vec![
        store,
        disconnect(203),
        wire(206, 202, "then", 207, "in"),
        wire(207, 207, "then", 205, "in"),
        wire(208, 201, "value", 207, "value"),
    ];
    let before = session.analysis().report().functions.clone();
    let work = edit(&mut session, &mut doc, &registry, options, rewired);
    assert_eq!(work, SessionWork::Scoped);
    assert_ne!(session.analysis().report().functions, before);
}

/// [`called_function`] with an outer function between On Start and the incrementing one.
fn nested_calls() -> (GraphDocument, std::sync::Arc<RegistrySnapshot>) {
    let (doc, registry) = called_function();
    let mut raw: Json = serde_json::from_str(&orange_model::to_json(&doc).unwrap()).unwrap();
    let port = |name: &str, direction: &str| json!({"port_id": name, "direction": direction, "kind": "data", "type": "core/u64@1"});
    let core = |n: u32, name: &str, config: Json| {
        json!({"id": id(n), "definition_id": format!("core/{name}"), "definition_version": 1,
            "config_version": 1, "config": config})
    };
    raw["functions"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id": id(300), "name": "outer",
        "kind": "action", "inputs": [port("value", "input")], "outputs": [port("result", "output")],
        "nodes": [core(301, "function-entry", json!({})),
            core(302, "call", json!({"function": id(200)})),
            core(303, "return", json!({}))],
        "edges": [edge(301, 301, "then", 302, "in"), edge(302, 301, "value", 302, "value"),
            edge(303, 302, "then", 303, "in"), edge(304, 302, "result", 303, "result")]}));
    for node in raw["nodes"].as_array_mut().unwrap() {
        if node["definition_id"] == "core/call" {
            node["config"]["function"] = json!(id(300));
        }
    }
    (orange_model::from_json(&raw.to_string()).unwrap(), registry)
}

#[test]
fn a_summary_change_propagates_through_an_outer_caller() {
    let (mut doc, registry) = nested_calls();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let store = DocCommand::AddNode {
        body: BodyRef::Function(id(200).parse().unwrap()),
        node: serde_json::from_value(json!({"id": id(207), "definition_id": "core/set-variable",
            "definition_version": 1, "config_version": 1, "config": {"variable": id(100)}}))
        .unwrap(),
    };
    let rewired = vec![
        store,
        disconnect(203),
        wire(206, 202, "then", 207, "in"),
        wire(207, 207, "then", 205, "in"),
        wire(208, 201, "value", 207, "value"),
    ];
    let work = edit(&mut session, &mut doc, &registry, options, rewired);
    assert_eq!(work, SessionWork::Scoped);
    let outer: orange_model::id::FunctionId = id(300).parse().unwrap();
    assert!(
        !session.analysis().report().functions[&outer]
            .run_writes
            .is_empty()
    );
}

/// [`called_function`] whose call runs in one Fork branch while the other sets the run
/// variable the function may come to write.
fn forked_call() -> (GraphDocument, std::sync::Arc<RegistrySnapshot>) {
    let (doc, registry) = called_function();
    let mut raw: Json = serde_json::from_str(&orange_model::to_json(&doc).unwrap()).unwrap();
    let nodes = raw["nodes"].as_array_mut().unwrap();
    nodes.retain(|node| node["id"] != json!(id(4)));
    nodes.push(
        json!({"id": id(5), "definition_id": "core/fork", "definition_version": 1,
        "config_version": 1, "config": {"join": "all"},
        "variadic": {"branch": {"next": 2, "order": [0, 1]}}}),
    );
    nodes.push(
        json!({"id": id(6), "definition_id": "core/set-variable", "definition_version": 1,
        "config_version": 1, "config": {"variable": id(100)},
        "inputs": {"value": {"type": "core/u64@1", "value": "7"}}}),
    );
    raw["edges"] = json!([
        edge(1, 1, "then", 5, "in"),
        edge(2, 2, "value", 3, "value"),
        edge(5, 5, "branch#0", 3, "in"),
        edge(6, 5, "branch#1", 6, "in"),
    ]);
    (orange_model::from_json(&raw.to_string()).unwrap(), registry)
}

#[test]
fn a_summary_change_that_conflicts_with_a_caller_runs_the_full_analysis() {
    let (mut doc, registry) = forked_call();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let store = DocCommand::AddNode {
        body: BodyRef::Function(id(200).parse().unwrap()),
        node: serde_json::from_value(json!({"id": id(207), "definition_id": "core/set-variable",
            "definition_version": 1, "config_version": 1, "config": {"variable": id(100)}}))
        .unwrap(),
    };
    let rewired = vec![
        store,
        disconnect(203),
        wire(206, 202, "then", 207, "in"),
        wire(207, 207, "then", 205, "in"),
        wire(208, 201, "value", 207, "value"),
    ];
    let work = edit(&mut session, &mut doc, &registry, options, rewired);
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
}

fn unknown_node(n: u32) -> DocCommand {
    let mut node = math(n, "add", 0, 1);
    node["definition_id"] = json!("org.orange.math/unknown");
    DocCommand::AddNode {
        body: BodyRef::Root,
        node: serde_json::from_value(node).unwrap(),
    }
}

#[test]
fn structure_edits_away_from_incomplete_nodes_are_scoped() {
    let (mut doc, registry) = chains();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    // An unresolvable node feeding the second chain's first add leaves both incomplete.
    let broken = vec![unknown_node(60), connect(60, 60, 13, "a")];
    edit(&mut session, &mut doc, &registry, options, broken);
    assert!(!session.analysis().report().incomplete_nodes.is_empty());
    // Far from them, wiring edits and node edits stay local.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![disconnect(29)],
    );
    assert_eq!(work, SessionWork::Scoped);
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![connect(29, 28, 29, "a")],
    );
    assert_eq!(work, SessionWork::Scoped);
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![add_node(61)],
    );
    assert_eq!(work, SessionWork::Scoped);
    // Next to an incomplete node, the edit runs in full.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![disconnect(14)],
    );
    assert_eq!(work, SessionWork::Recompute);
    // Repairing the graph runs in full too.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![remove_nodes(&[60])],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(session.analysis().report().incomplete_nodes.is_empty());
}

#[test]
fn exec_edits_away_from_incomplete_nodes_are_scoped() {
    let (mut doc, registry) = two_activations();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![unknown_node(70)],
    );
    assert!(!session.analysis().report().incomplete_nodes.is_empty());
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![disconnect(5)],
    );
    assert_eq!(work, SessionWork::Scoped);
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![wire(5, 5, "then", 6, "in")],
    );
    assert_eq!(work, SessionWork::Scoped);
}

#[test]
fn control_flow_beside_an_unresolved_feed_runs_the_full_analysis() {
    let (mut doc, registry) = two_activations();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    // A Set fed by an unresolvable node leads into the second activation. Nothing reaches
    // it, but the unresolved feed exempts it from the unreachable warning.
    let feed = vec![
        unknown_node(70),
        set_node(7),
        wire(40, 70, "sum", 7, "value"),
        wire(41, 7, "then", 6, "in"),
    ];
    edit(&mut session, &mut doc, &registry, options, feed);
    assert!(!session.analysis().report().incomplete_nodes.is_empty());
    // An exec edit in that activation reruns its control flow, which includes the Set.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![disconnect(5)],
    );
    assert_eq!(work, SessionWork::Recompute);
}

/// A zone marker of the `zone` pair, carrying `u64` values.
fn marker(n: u32, definition: &str, zone: &str) -> Json {
    let mut node = json!({"id": id(n), "definition_id": format!("core/{definition}"),
        "definition_version": 1, "config_version": 1, "config": {"zone": zone},
        "type_pins": {"A": "core/u64@1"}});
    if definition.ends_with("begin") {
        node["inputs"] = json!({"count": {"type": "core/i64@1", "value": "3"}});
    }
    node
}

/// The ten chains of [`chains`] and two Repeat zones. Zone `z` (begin 1, end 3) adds `13`'s
/// sum to `10`'s three times through body adds 2 and 5, and add 4 reads its result, which
/// is exported. Zone `w` (begin 6, end 7) has an empty body: its end takes `19`'s sum, and
/// add 8 reads its result, so only the data-cycle pass's begin-to-end dependency joins
/// `16`, which feeds its begin, to 8.
fn zoned() -> (GraphDocument, std::sync::Arc<RegistrySnapshot>) {
    let (package, registry) = catalog();
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for chain in 0..10 {
        let first = 10 + chain * 3;
        for link in 0..3 {
            nodes.push(math(first + link, "add", 0, 1));
            if link > 0 {
                let to = first + link;
                edges.push(edge(to, to - 1, "sum", to, "a"));
            }
        }
    }
    nodes.extend([
        marker(1, "repeat-begin", "z"),
        math(2, "add", 0, 1),
        marker(3, "repeat-end", "z"),
        math(4, "add", 0, 1),
        math(5, "add", 0, 1),
        marker(6, "repeat-begin", "w"),
        marker(7, "repeat-end", "w"),
        math(8, "add", 0, 1),
    ]);
    edges.extend([
        edge(200, 10, "sum", 1, "initial"),
        edge(201, 1, "carried", 2, "a"),
        edge(202, 13, "sum", 2, "b"),
        edge(203, 2, "sum", 5, "a"),
        edge(204, 5, "sum", 3, "next"),
        edge(205, 3, "result", 4, "a"),
        edge(206, 16, "sum", 6, "initial"),
        edge(207, 19, "sum", 7, "next"),
        edge(208, 7, "result", 8, "a"),
    ]);
    (
        graph(&package, nodes, edges, vec![export(4, "sum")]),
        registry,
    )
}

#[test]
fn structure_edits_away_from_zones_are_scoped() {
    let (mut doc, registry) = zoned();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    assert_eq!(session.analysis().report().regions.zones.iter().len(), 2);
    let edits = [
        vec![connect(90, 22, 23, "b")],
        vec![disconnect(90)],
        // A capture's producer and the zone result's reader are outside the zone.
        vec![connect(91, 13, 25, "b")],
        vec![connect(92, 4, 28, "b")],
        // A cone through an end reaches the begin's iteration projection, which no exec
        // region owns, so the edge carries no availability requirement.
        vec![connect(93, 8, 31, "b")],
        vec![add_node(94), connect(94, 12, 94, "a")],
        vec![remove_nodes(&[94])],
        vec![disconnect(92), disconnect(91)],
    ];
    for commands in edits {
        let work = edit(&mut session, &mut doc, &registry, options, commands);
        assert_eq!(work, SessionWork::Scoped);
    }
    assert_eq!(session.stats().full_analysis_runs, 0);
    assert!(session.analysis().report().is_valid());
}

#[test]
fn edits_that_reach_a_zone_run_the_full_analysis() {
    let (mut doc, registry) = zoned();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let body = |session: &AnalysisSession| {
        let zones = &session.analysis().report().regions.zones;
        let end: orange_model::id::NodeId = id(3).parse().unwrap();
        (10..100)
            .filter(|n| {
                zones
                    .owner(id(*n).parse().unwrap())
                    .is_some_and(|zone| zone.end == end)
            })
            .count()
    };
    // A new node wired between two body nodes joins the body.
    let pulled = vec![
        add_node(95),
        connect(95, 2, 95, "a"),
        connect(96, 95, 5, "b"),
    ];
    let work = edit(&mut session, &mut doc, &registry, options, pulled);
    assert_eq!(work, SessionWork::Recompute);
    assert!(session.analysis().report().is_valid());
    assert_eq!(body(&session), 1);
    // Removing a body node, or a capture's edge, touches a body node.
    for commands in [vec![remove_nodes(&[95])], vec![disconnect(202)]] {
        let work = edit(&mut session, &mut doc, &registry, options, commands);
        assert_eq!(work, SessionWork::Recompute);
        assert!(session.analysis().report().is_valid());
    }
    // A value leaving the body.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![connect(97, 2, 34, "b")],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![disconnect(97)],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(session.analysis().report().is_valid());
    // An edge between outside nodes that closes a cycle only through zone `w`'s result
    // reading its begin's prefix.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![connect(98, 8, 16, "b")],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
}

#[test]
fn a_payload_behind_a_zone_prefix_reaching_an_export_runs_the_full_analysis() {
    let (package, registry) = catalog();
    let event = "40000000-0000-4000-8000-000000000001";
    let on_event = json!({"id": id(50), "definition_id": "core/on-event",
        "definition_version": 1, "config_version": 1, "config": {"event": event},
        "activation": {"mode": "queue", "capacity": 4, "overflow": "drop-oldest",
            "on_fault": "continue", "order": 0}});
    // The payload feeds the begin's prefix; the end takes a capture, so no body path joins
    // the begin to the reader of the result.
    let nodes = vec![
        on_event,
        math(60, "add", 0, 1),
        marker(61, "repeat-begin", "z"),
        marker(62, "repeat-end", "z"),
        math(63, "add", 0, 1),
        math(64, "add", 0, 1),
        math(65, "add", 0, 1),
    ];
    let edges = vec![
        edge(70, 50, "payload", 60, "a"),
        edge(71, 60, "sum", 61, "initial"),
        edge(72, 63, "sum", 62, "next"),
        edge(73, 62, "result", 64, "a"),
    ];
    let base = graph(&package, nodes, edges, vec![export(65, "sum")]);
    let mut raw: Json = serde_json::from_str(&orange_model::to_json(&base).unwrap()).unwrap();
    raw["events"] = json!([{"id": event, "name": "tick", "payload": "core/u64@1", "scope": "run"}]);
    let mut doc = orange_model::from_json(&raw.to_string()).unwrap();
    let mut session = session(&doc, &registry);
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        AnalysisOptions::default(),
        vec![connect(74, 64, 65, "a")],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
}

#[test]
fn zone_markers_run_the_full_analysis() {
    let (mut doc, registry) = zoned();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let unpaired = DocCommand::AddNode {
        body: BodyRef::Root,
        node: serde_json::from_value(marker(50, "repeat-begin", "u")).unwrap(),
    };
    let work = edit(&mut session, &mut doc, &registry, options, vec![unpaired]);
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![remove_nodes(&[50])],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(session.analysis().report().is_valid());
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![remove_nodes(&[7])],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
}

#[test]
fn random_rewiring_around_zones_equals_the_full_analysis() {
    let (mut doc, registry) = zoned();
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let mut seed: u64 = 41;
    let mut next = |bound: u32| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as u32 % bound
    };
    // The chains and both zones' readers, which also feed or capture into body add 2's
    // `b`; a body node feeding anything else would leave the report invalid for good.
    let readers: Vec<u32> = (10..40).chain([4, 8]).collect();
    let mut wired: BTreeMap<(u32, &str), u32> = BTreeMap::new();
    for chain in 0..10 {
        for link in 1..3 {
            let to = 10 + chain * 3 + link;
            wired.insert((to, "a"), to);
        }
    }
    wired.extend([((2, "b"), 202), ((4, "a"), 205), ((8, "a"), 208)]);
    let mut scoped = 0;
    let mut n = 300;
    for _ in 0..300 {
        let mut commands = Vec::new();
        let mut inputs = Vec::new();
        for _ in 0..=next(3) {
            let from = readers[next(readers.len() as u32) as usize];
            let to = match next(readers.len() as u32 + 1) as usize {
                at if at == readers.len() => 2,
                at => readers[at],
            };
            let input = if to == 2 || next(2) != 0 { "b" } else { "a" };
            if inputs.contains(&(to, input)) {
                continue;
            }
            inputs.push((to, input));
            commands.push(match wired.remove(&(to, input)) {
                Some(edge) => disconnect(edge),
                None if from != to => {
                    n += 1;
                    wired.insert((to, input), n);
                    connect(n, from, to, input)
                }
                None => continue,
            });
        }
        if !commands.is_empty()
            && edit(&mut session, &mut doc, &registry, options, commands) == SessionWork::Scoped
        {
            scoped += 1;
        }
    }
    assert!(scoped >= 30, "{scoped}");
}

/// One generic component: `length` adds from node 10, each reading the previous sum into
/// both inputs. Only the first add's literals are active, so they ground every add's `T`;
/// with `literals`, every other add also keeps its (shadowed) literals.
fn connected_chain(
    length: u32,
    literals: bool,
) -> (GraphDocument, std::sync::Arc<RegistrySnapshot>) {
    let (package, registry) = catalog();
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for k in 0..length {
        let mut node = math(10 + k, "add", 0, 1);
        if k > 0 {
            if !literals {
                node["inputs"] = json!({});
            }
            edges.push(edge(1000 + 2 * k, 9 + k, "sum", 10 + k, "a"));
            edges.push(edge(1001 + 2 * k, 9 + k, "sum", 10 + k, "b"));
        }
        nodes.push(node);
    }
    (graph(&package, nodes, edges, vec![]), registry)
}

/// The edit was scoped without solving any component in full.
fn kept(session: &AnalysisSession, work: SessionWork) -> bool {
    work == SessionWork::Scoped && session.stats().local.components_computed == 0
}

#[test]
fn edits_that_keep_a_connected_solution_skip_the_component_solve() {
    let (mut doc, registry) = connected_chain(1200, true);
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let pin = |node: u32, ty: &str| DocCommand::SetTypePins {
        node: id(node).parse().unwrap(),
        pins: types(&[("T", ty)]),
    };
    // A literal, a pin and a type argument that keep `T` are checked beside the node.
    for commands in [
        vec![set(10, "b", u64_text("7"))],
        vec![set(300, "a", u64_text("7"))],
        vec![pin(300, "core/u64@1")],
        vec![DocCommand::SetTypeArgs {
            node: id(300).parse().unwrap(),
            args: types(&[("T", "core/i64@1")]),
        }],
    ] {
        let work = edit(&mut session, &mut doc, &registry, options, commands);
        assert!(kept(&session, work), "{work:?}");
        // The node and its two neighbours, of 1,200.
        assert!(session.stats().input_nodes <= 3, "{:?}", session.stats());
    }
    // Disconnecting an input activates its literal; the other input still joins the
    // chain, so the node and its source keep their types.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![disconnect(1000 + 2 * 990)],
    );
    assert!(kept(&session, work), "{work:?}");
    // Reconnecting it shadows that literal again, and the nearest other evidence, the pin,
    // is 700 adds away, past the local budget: the component is solved in full.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![connect(1000 + 2 * 990, 999, 1000, "a")],
    );
    assert_eq!(work, SessionWork::Scoped);
    assert_eq!(session.stats().local.components_computed, 1);
    // Package configuration that keeps the contract keeps the template.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![configure(300, json!({"int_overflow": "saturating"}))],
    );
    assert!(kept(&session, work), "{work:?}");
}

#[test]
fn incomplete_report_prevents_kept_solution_until_repaired() {
    let (package, registry) = catalog();
    let nodes: Vec<Json> = (10..16).map(|n| math(n, "add", 0, 1)).collect();
    let edges: Vec<Json> = (11..16)
        .map(|n| edge(1000 + n, n - 1, "sum", n, "a"))
        .collect();
    let mut doc = graph(&package, nodes, edges, vec![]);
    let options = AnalysisOptions::default();
    let mut session = session(&doc, &registry);
    let incomplete = doc.apply(vec![unknown_node(60)]).unwrap();
    session.update(&doc, &registry, &incomplete.change_set, options);
    assert_eq!(
        session.analysis(),
        &analyze_with_options(&doc, &registry, BTreeMap::new(), options)
    );
    assert!(!session.analysis().report().incomplete_nodes.is_empty());

    // This changes only a literal value, so the generic chain keeps its signatures. The
    // incomplete node is unrelated, but completeness still prevents the kept-solution
    // proof and the safe path resolves the touched component.
    let literal = doc.apply(vec![set(12, "b", u64_text("9"))]).unwrap();
    let work = session
        .update(&doc, &registry, &literal.change_set, options)
        .work;
    assert_eq!(work, SessionWork::Scoped);
    assert!(session.stats().local.components_computed > 0);
    assert_eq!(
        session.analysis(),
        &analyze_with_options(&doc, &registry, BTreeMap::new(), options)
    );

    // Undo in history order: first restore the literal, then remove the incomplete node.
    let undone_literal = doc.revert(literal.inverse).unwrap();
    session.update(&doc, &registry, &undone_literal.change_set, options);
    assert_eq!(
        session.analysis(),
        &analyze_with_options(&doc, &registry, BTreeMap::new(), options)
    );
    let undone = doc.revert(incomplete.inverse).unwrap();
    let work = session
        .update(&doc, &registry, &undone.change_set, options)
        .work;
    assert_eq!(work, SessionWork::Recompute);
    assert_eq!(
        session.analysis(),
        &analyze_with_options(&doc, &registry, BTreeMap::new(), options)
    );
    assert!(session.analysis().report().incomplete_nodes.is_empty());
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(12, "b", u64_text("10"))],
    );
    assert!(kept(&session, work), "{work:?}");
}

#[test]
fn for_each_mode_configuration_recomputes_its_projected_contract() {
    let (package, registry) = catalog();
    let node = json!({
        "id": id(50),
        "definition_id": "core/for-each",
        "definition_version": 1,
        "config_version": 1,
        "config": {"mode": "list"},
        "type_pins": {"T": "core/u64@1"},
        "inputs": {"items": {"type": "core/list@1<core/u64@1>", "value": []}}
    });
    let mut doc = graph(&package, vec![node.clone()], vec![], vec![]);
    let options = AnalysisOptions::default();
    let mut session = AnalysisSession::new(&doc, &registry, BTreeMap::new(), options);
    let before = registry
        .contract(&doc, doc.node(id(50).parse().unwrap()).unwrap())
        .unwrap()
        .unwrap();
    assert!(
        before
            .ports
            .iter()
            .any(|port| port.port_id.as_str() == "element")
    );

    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![configure(50, json!({"mode": "map"}))],
    );
    assert_eq!(work, SessionWork::Recompute);
    let after = registry
        .contract(&doc, doc.node(id(50).parse().unwrap()).unwrap())
        .unwrap()
        .unwrap();
    assert!(
        !after
            .ports
            .iter()
            .any(|port| port.port_id.as_str() == "element")
    );
    assert!(
        after
            .ports
            .iter()
            .any(|port| port.port_id.as_str() == "key")
    );
    assert!(
        after
            .ports
            .iter()
            .any(|port| port.port_id.as_str() == "value")
    );
    assert_ne!(before, after);
}

#[test]
fn oversized_resolved_type_uses_component_fallback() {
    let nominal = format!("x/{}@1", "a".repeat(16_400));
    let nominal_expr = nominal.parse().unwrap();
    let mut type_builder = orange_model::types::TypeRegistryBuilder::core();
    type_builder
        .register_handle_kind("x", &nominal_expr)
        .unwrap();
    let type_registry = std::sync::Arc::new(type_builder.build().unwrap());
    let origin =
        orange_model::validation::HostOrigin::new("0.0.0", format!("sha256:{}", "a".repeat(64)))
            .unwrap();
    let registry = RegistryBuilder::core(origin)
        .types(type_registry)
        .build()
        .unwrap();
    let collection = format!("core/list@1<{nominal}>");
    let node = json!({
        "id": id(50),
        "definition_id": "core/for-each",
        "definition_version": 1,
        "config_version": 1,
        "config": {"mode": "list"},
        "type_pins": {"T": nominal},
        "inputs": {"items": {"type": collection, "value": []}}
    });
    let mut doc = orange_model::from_json(
        &json!({
            "format": "orange.graph",
            "format_version": 1,
            "graph_id": "20000000-0000-4000-8000-000000000001",
            "revision": 0,
            "nodes": [node],
            "edges": []
        })
        .to_string(),
    )
    .unwrap();
    let options = AnalysisOptions::default();
    let mut session = session(&doc, &registry);
    // Re-setting the same pin is a journaled scoped edit. Its contract and signatures are
    // unchanged. The resolved input port contains the oversized concrete argument; the
    // published type-byte guard excludes it from kept-solution use.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![DocCommand::SetTypePins {
            node: id(50).parse().unwrap(),
            pins: types(&[("T", &nominal)]),
        }],
    );
    assert_eq!(work, SessionWork::Scoped);
    assert!(session.stats().local.components_computed > 0);
}

#[test]
fn edits_that_change_a_connected_solution_still_equal_the_full_analysis() {
    let (mut doc, registry) = connected_chain(600, false);
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let i64_text = |value: &str| Some(json!({"type": "core/i64@1", "value": value}));
    // A pin that conflicts with the chain's type.
    let pin = |ty: &str| DocCommand::SetTypePins {
        node: id(300).parse().unwrap(),
        pins: types(&[("T", ty)]),
    };
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![pin("core/i64@1")],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
    let clear = DocCommand::SetTypePins {
        node: id(300).parse().unwrap(),
        pins: BTreeMap::new(),
    };
    let work = edit(&mut session, &mut doc, &registry, options, vec![clear]);
    assert_eq!(work, SessionWork::Recompute);
    assert!(session.analysis().report().is_valid());
    // Literals that move the whole chain to another type, consistently (away from its
    // stored signatures), and back.
    for ty in [i64_text, u64_text] {
        let both = vec![set(10, "a", ty("1")), set(10, "b", ty("2"))];
        let work = edit(&mut session, &mut doc, &registry, options, both);
        assert_eq!(work, SessionWork::Recompute);
    }
    assert!(session.analysis().report().is_valid());
}

#[test]
fn removing_the_only_evidence_of_a_connected_chain_is_not_kept() {
    // Every published type downstream still reads u64, but nothing grounds it any more:
    // fixing the neighbours' published types would wrongly keep the solution.
    let (mut doc, registry) = connected_chain(40, false);
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![disconnect(1002), disconnect(1003)],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![connect(1002, 10, 11, "a"), connect(1003, 10, 11, "b")],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(session.analysis().report().is_valid());
    // With one grounding input left, the first add keeps its type; removing the other
    // literal leaves the whole chain without evidence.
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(10, "b", None)],
    );
    assert!(kept(&session, work), "{work:?}");
    let work = edit(
        &mut session,
        &mut doc,
        &registry,
        options,
        vec![set(10, "a", None)],
    );
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
}

#[test]
fn random_edits_of_a_connected_component_equal_the_full_analysis() {
    // Adds 10..70; add k reads k-1 into `a` and k-2 into `b`, all one component.
    let (package, registry) = catalog();
    let mut nodes = Vec::new();
    let mut wired: BTreeMap<(u32, &str), u32> = BTreeMap::new();
    let mut edges = Vec::new();
    for k in 10..70 {
        nodes.push(math(k, "add", 0, 1));
        for (input, from) in [("a", k - 1), ("b", k - 2)] {
            if from >= 10 {
                let n = 2000 + 2 * k + u32::from(input == "b");
                edges.push(edge(n, from, "sum", k, input));
                wired.insert((k, input), n);
            }
        }
    }
    let mut doc = graph(&package, nodes, edges, vec![]);
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let mut seed: u64 = 0x00c0_ffee;
    let mut next = |bound: u32| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as u32 % bound
    };
    let (mut kept_edits, mut n) = (0, 3000);
    for _ in 0..400 {
        let node = 10 + next(60);
        let input = if next(2) == 0 { "a" } else { "b" };
        // Mostly the chain's type; sometimes another one, which changes or breaks it.
        let ty = if next(8) == 0 {
            "core/i64@1"
        } else {
            "core/u64@1"
        };
        let commands = match next(5) {
            0 => vec![set(node, input, Some(json!({"type": ty, "value": "3"})))],
            1 => vec![set(node, input, None)],
            2 => vec![DocCommand::SetTypePins {
                node: id(node).parse().unwrap(),
                pins: if next(2) == 0 {
                    types(&[("T", ty)])
                } else {
                    BTreeMap::new()
                },
            }],
            _ => match wired.remove(&(node, input)) {
                Some(edge) => vec![disconnect(edge)],
                // Only from an earlier add, so no cycle forms.
                None if node > 10 => {
                    n += 1;
                    wired.insert((node, input), n);
                    vec![connect(n, 10 + next(node - 10), node, input)]
                }
                None => continue,
            },
        };
        let work = edit(&mut session, &mut doc, &registry, options, commands);
        if kept(&session, work) {
            kept_edits += 1;
        }
    }
    // About a third of the edits; most others change or break the chain's type.
    assert!(kept_edits >= 60, "{kept_edits}");
}

#[test]
fn a_shared_concrete_source_is_not_expanded() {
    // One random source read by a hundred separate adds, as one Get Variable is read by
    // many clusters: an edit of one add re-resolves it and its source, not the source's
    // other readers.
    let (package, registry) = catalog();
    let mut nodes = vec![random(10)];
    let mut edges = Vec::new();
    for k in 0..100 {
        nodes.push(math(11 + k, "add", 0, 1));
        edges.push(edge(1000 + k, 10, "value", 11 + k, "b"));
    }
    let mut doc = graph(&package, nodes, edges, vec![]);
    let mut session = session(&doc, &registry);
    let options = AnalysisOptions::default();
    let pin = DocCommand::SetTypePins {
        node: id(50).parse().unwrap(),
        pins: types(&[("T", "core/u64@1")]),
    };
    for commands in [
        vec![pin],
        vec![set(50, "a", u64_text("4"))],
        vec![disconnect(1039)],
        vec![DocCommand::Connect {
            edge: serde_json::from_value(edge(1039, 10, "value", 50, "b")).unwrap(),
        }],
    ] {
        let work = edit(&mut session, &mut doc, &registry, options, commands);
        assert!(kept(&session, work), "{work:?}");
        assert!(session.stats().input_nodes <= 2, "{:?}", session.stats());
    }
    // A node added in place keeps its solved arguments too, so a later edit of it is
    // checked beside it.
    let added = vec![
        add_node(200),
        DocCommand::Connect {
            edge: serde_json::from_value(edge(1200, 10, "value", 200, "a")).unwrap(),
        },
    ];
    let work = edit(&mut session, &mut doc, &registry, options, added);
    assert_eq!(work, SessionWork::Scoped);
    let pin = DocCommand::SetTypePins {
        node: id(200).parse().unwrap(),
        pins: types(&[("T", "core/u64@1")]),
    };
    let work = edit(&mut session, &mut doc, &registry, options, vec![pin]);
    assert!(kept(&session, work), "{work:?}");
    // A configuration version no provider supports leaves the shared source unresolved,
    // which the source's own resolution shows.
    let version = DocCommand::SetConfig {
        node: id(10).parse().unwrap(),
        config_version: 2,
        config: serde_json::from_value(json!({})).unwrap(),
    };
    let work = edit(&mut session, &mut doc, &registry, options, vec![version]);
    assert_eq!(work, SessionWork::Recompute);
    assert!(!session.analysis().report().is_valid());
}
