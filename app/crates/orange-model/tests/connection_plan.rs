//! 00c.6a: read-only connection plans (11 §6): direct wire, replacement or converter node.

mod common;

use common::generic::{add, data_edge, document, registry, source, typed_sink, typed_source};
use common::{content, eid, id_source, nid, port};
use orange_model::conversion::{
    ConnectionError, ConnectionOutcome, ConnectionPlan, ConnectionRequest, ExplicitConverterChoice,
    SelectionRejection,
};
use orange_model::inference::{InferenceIssue, plan_edit};
use orange_model::{DocCommand, GraphDocument};
use serde_json::json;

fn request(from: (u8, &str), to: (u8, &str)) -> ConnectionRequest {
    ConnectionRequest {
        from: port(from.0, from.1),
        to: port(to.0, to.1),
        choice: None,
    }
}

fn plan(
    doc: &GraphDocument,
    request: ConnectionRequest,
) -> Result<ConnectionOutcome, ConnectionError> {
    let mut ids = id_source(3);
    orange_model::conversion::plan_connection(doc, &registry(None), request, &mut ids)
}

fn ready(outcome: Result<ConnectionOutcome, ConnectionError>) -> ConnectionPlan {
    match outcome.unwrap() {
        ConnectionOutcome::Ready(plan) => *plan,
        other => panic!("expected a plan, got {other:?}"),
    }
}

/// `from` node 1 (`ty`) and sink node 3 (`sink`).
fn pair(from: &str, sink: &str) -> GraphDocument {
    document(
        vec![typed_source(1, from), typed_sink(3, sink, json!({}))],
        vec![],
    )
}

#[test]
fn equal_types_plan_a_direct_wire_without_touching_the_document() {
    let doc = pair("i64", "i64");
    let before = content(&doc);
    let plan = ready(plan(&doc, request((1, "value"), (3, "value"))));
    assert!(plan.converter.is_none() && plan.replaced.is_empty());
    assert_eq!(plan.edges.len(), 1);
    assert!(matches!(plan.commands(), [DocCommand::Connect { .. }]));
    assert_eq!(content(&doc), before);
}

#[test]
fn a_unique_lossless_entry_plans_a_visible_converter_with_fresh_identities() {
    let doc = pair("i32", "i64");
    let mut calls = 0;
    let mut source = id_source(3);
    let outcome = orange_model::conversion::plan_connection(
        &doc,
        &registry(None),
        request((1, "value"), (3, "value")),
        || {
            calls += 1;
            source()
        },
    );
    let plan = ready(outcome);
    assert_eq!(calls, 3, "one node and two edges");
    let inserted = plan.converter.as_ref().unwrap();
    assert_eq!(inserted.resolved.entry.to_string(), "core/i32-to-i64@1");

    let [
        DocCommand::PutSignature { snapshot, .. },
        DocCommand::AddNode { node, .. },
        DocCommand::Connect { edge: into },
        DocCommand::Connect { edge: out_of },
    ] = plan.commands()
    else {
        panic!("{:?}", plan.commands());
    };
    assert_eq!(node.id, inserted.node);
    assert_eq!(node.definition.id.as_str(), "core/convert");
    assert_eq!(node.config["converter"], "core/i32-to-i64");
    assert_eq!(node.config["contract_version"], 1);
    assert!(
        node.provider.is_none(),
        "the core implementation is implicit"
    );
    assert_eq!([into.id, out_of.id], [plan.edges[0], plan.edges[1]]);
    assert_eq!((into.to.node, out_of.from.node), (node.id, node.id));
    assert_eq!(
        snapshot
            .ports
            .iter()
            .map(|p| p.port_id.as_str())
            .collect::<Vec<_>>(),
        ["value", "result"]
    );
}

#[test]
fn replacing_a_wire_removes_the_old_edge_first_and_keeps_literals() {
    let literal = json!({"inputs": {"value": {"type": "core/i64@1", "value": "7"}}});
    let doc = document(
        vec![
            typed_source(1, "i32"),
            typed_source(2, "i64"),
            typed_sink(3, "i64", literal),
        ],
        vec![common::generic::data_edge(1, (2, "value"), (3, "value"))],
    );
    let planned = ready(plan(&doc, request((1, "value"), (3, "value"))));
    assert_eq!(planned.replaced, [eid(1)]);
    assert_eq!(
        planned.commands()[0],
        DocCommand::Disconnect { edge: eid(1) }
    );
    assert!(
        planned
            .commands()
            .iter()
            .all(|command| !matches!(command, DocCommand::SetLiteral { .. }))
    );

    let again = plan(&doc, request((2, "value"), (3, "value"))).map(|_| ());
    assert_eq!(again, Err(ConnectionError::AlreadyConnected(eid(1))));
}

#[test]
fn choice_required_and_rejected_outcomes_carry_no_commands() {
    let partial = pair("i64", "i32");
    let outcome = plan(&partial, request((1, "value"), (3, "value"))).unwrap();
    assert!(matches!(outcome, ConnectionOutcome::ChoiceRequired { .. }));

    let none = pair("f64", "i64");
    let outcome = plan(&none, request((1, "value"), (3, "value"))).unwrap();
    assert!(matches!(
        outcome,
        ConnectionOutcome::Rejected(SelectionRejection::NoDirectConverter { .. })
    ));
}

#[test]
fn an_explicit_choice_binds_the_exact_candidate_and_a_stale_one_is_rejected() {
    let doc = pair("i64", "i32");
    let ConnectionOutcome::ChoiceRequired { candidates, .. } =
        plan(&doc, request((1, "value"), (3, "value"))).unwrap()
    else {
        panic!("a partial conversion needs a choice");
    };
    let candidate = &candidates[0];
    let choose = |entry: &orange_model::conversion::ConverterKey| ConnectionRequest {
        choice: Some(ExplicitConverterChoice {
            entry: entry.clone(),
            provider: candidate.provider.clone(),
            options: Default::default(),
        }),
        ..request((1, "value"), (3, "value"))
    };
    let plan_ = ready(plan(&doc, choose(&candidate.entry)));
    assert_eq!(
        plan_.converter.unwrap().resolved.entry.to_string(),
        "core/i64-to-i32@1"
    );

    let stale = orange_model::conversion::ConverterKey {
        id: "core/i32-to-i64".parse().unwrap(),
        contract_version: 1,
    };
    assert!(matches!(
        plan(&doc, choose(&stale)).unwrap(),
        ConnectionOutcome::Rejected(SelectionRejection::StaleChoice)
    ));
}

#[test]
fn malformed_requests_are_typed_errors() {
    let doc = pair("i64", "i64");
    assert!(matches!(
        plan(&doc, request((3, "value"), (1, "value"))),
        Err(ConnectionError::WrongDirection { .. })
    ));
    assert!(matches!(
        plan(&doc, request((9, "value"), (3, "value"))),
        Err(ConnectionError::NodeNotFound(id)) if id == nid(9)
    ));
    assert!(matches!(
        plan(&doc, request((1, "nope"), (3, "value"))),
        Err(ConnectionError::NotADataPort { .. })
    ));
}

#[test]
fn generic_endpoints_connect_directly_and_carry_their_inferred_updates() {
    let doc = document(vec![typed_source(1, "i32"), add(2, json!({}))], vec![]);
    let plan = ready(plan(&doc, request((1, "value"), (2, "a"))));
    assert!(plan.converter.is_none() && plan.issues.is_empty());
    let [first, rest @ ..] = plan.commands() else {
        panic!()
    };
    assert!(matches!(first, DocCommand::Connect { .. }));
    assert!(rest.iter().any(|command| matches!(
        command,
        DocCommand::SetTypeArgs { node, args } if *node == nid(2) && args["T"].as_str() == "core/i32@1"
    )));

    // The plan is the edit plus `plan_edit`'s updates, nothing else.
    let edit = vec![first.clone()];
    let expected = plan_edit(&doc, edit, &registry(None)).unwrap();
    assert_eq!(plan.commands(), expected.commands.as_slice());
}

#[test]
fn incompatible_connections_are_refused_independently_of_cached_arguments() {
    // The surviving i64 source constrains add 2, regardless of its persisted cache.
    for args in [
        json!({}),
        json!({"T": "core/i64@1"}),
        json!({"T": "core/f64@1"}),
    ] {
        let doc = document(
            vec![
                source(1),
                add(2, json!({"type_args": args})),
                typed_source(4, "f64"),
            ],
            vec![data_edge(1, (1, "value"), (2, "a"))],
        );
        let before = content(&doc);
        assert!(matches!(
            plan(&doc, request((4, "value"), (2, "b"))).unwrap(),
            ConnectionOutcome::Rejected(SelectionRejection::NoDirectConverter { from, to })
                if from.as_str() == "core/f64@1" && to.as_str() == "core/i64@1"
        ));
        assert_eq!(content(&doc), before);
    }

    // A constraint or specialization failure is a conflict too.
    let doc = document(vec![typed_source(4, "f64"), add(2, json!({}))], vec![]);
    let only_i64 = registry(Some(vec!["core/i64@1"]));
    let refused = orange_model::conversion::plan_connection(
        &doc,
        &only_i64,
        request((4, "value"), (2, "a")),
        id_source(3),
    );
    let Err(ConnectionError::TypeConflict(issues)) = refused else {
        panic!("expected a type conflict, got {refused:?}");
    };
    assert_eq!(codes(&issues), ["E-NO-SPECIALIZATION"]);
}

#[test]
fn a_component_that_was_already_broken_stays_editable() {
    // Add 2 is fed i64 and f64 (an imported, conflicting graph); wiring it on is allowed.
    let doc = document(
        vec![
            source(1),
            add(2, json!({})),
            add(3, json!({})),
            typed_source(4, "f64"),
        ],
        vec![
            data_edge(1, (1, "value"), (2, "a")),
            data_edge(2, (4, "value"), (2, "b")),
        ],
    );
    let plan = ready(plan(&doc, request((2, "sum"), (3, "a"))));
    assert_eq!(codes(&plan.issues), ["E-TYPE-MISMATCH"]);
}

fn codes(issues: &[InferenceIssue]) -> Vec<&'static str> {
    issues.iter().map(InferenceIssue::code).collect()
}

#[test]
fn implicit_ports_are_exec_or_typed_by_their_kind() {
    let mut doc = pair("i64", "i64");
    let json = json!({
        "id": common::node_id(5), "definition_id": "org.test/act", "definition_version": 1,
        "config_version": 1, "config": {}, "signature": "s-act"
    });
    let mut value: serde_json::Value =
        serde_json::from_str(&orange_model::to_json(&doc).unwrap()).unwrap();
    value["signatures"]["s-act"] = json!({"kind": "action", "ports": []});
    value["nodes"].as_array_mut().unwrap().push(json);
    doc = orange_model::from_json(&value.to_string()).unwrap();

    // `then` is an exec port; `error` is a typed data output that is not an i64.
    assert!(matches!(
        plan(&doc, request((5, "then"), (3, "value"))),
        Err(ConnectionError::NotADataPort { .. })
    ));
    assert!(!matches!(
        plan(&doc, request((5, "error"), (3, "value"))),
        Err(ConnectionError::NotADataPort { .. })
    ));
}
