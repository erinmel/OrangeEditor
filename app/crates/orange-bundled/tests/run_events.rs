//! Compiled run-scope event execution regressions.
//!
//! Expected values are asserted at the public model/compiler/runtime boundary. In
//! particular, Emit must publish only an envelope; an On Event root is a later pump turn.
use crate::data_graph as support;

use orange_compiler::{CompileOptions, compile_control};
use orange_model::{GraphDocument, Value, analyze, from_json, to_json};
use orange_runtime::{ControlOutcome, Run, RunOptions, RunState};
use serde_json::{Value as Json, json};
use std::sync::Arc;
use support::{catalog, edge, graph, id, math};

const EVENT: &str = "40000000-0000-4000-8000-000000000001";
const STRING_EVENT: &str = "40000000-0000-4000-8000-000000000002";

fn core(n: u32, name: &str) -> Json {
    let mut node = json!({
        "id": id(n),
        "definition_id": format!("core/{name}"),
        "definition_version": 1,
        "config_version": 1,
        "config": {}
    });
    if matches!(name, "on-start" | "on-stop") {
        node["activation"] = activation(0, 4, "drop-newest");
    }
    node
}

fn activation(order: u32, capacity: u32, overflow: &str) -> Json {
    json!({
        "mode": "queue",
        "capacity": capacity,
        "overflow": overflow,
        "on_fault": "continue",
        "order": order
    })
}

fn on_event_for(n: u32, event: &str, order: u32, capacity: u32, overflow: &str) -> Json {
    let mut node = core(n, "on-event");
    node["config"] = json!({"event": event});
    node["activation"] = activation(order, capacity, overflow);
    node
}

fn on_event(n: u32, order: u32, capacity: u32, overflow: &str) -> Json {
    on_event_for(n, EVENT, order, capacity, overflow)
}

fn emit_for(n: u32, event: &str, value: u64) -> Json {
    let mut node = core(n, "emit");
    node["config"] = json!({"event": event});
    node["inputs"] = json!({
        "payload": {"type": "core/u64@1", "value": value.to_string()}
    });
    node
}

fn emit(n: u32, value: u64) -> Json {
    emit_for(n, EVENT, value)
}

fn emit_string(n: u32, event: &str, value: &str) -> Json {
    let mut node = core(n, "emit");
    node["config"] = json!({"event": event});
    node["inputs"] = json!({
        "payload": {"type": "core/string@1", "value": value}
    });
    node
}

fn string_variable(n: u32) -> Json {
    json!({
        "id": id(n),
        "name": format!("text{n}"),
        "type": "core/string@1",
        "scope": "run"
    })
}

fn variable(n: u32, scope: &str, initial: Option<u64>) -> Json {
    let mut value = json!({
        "id": id(n),
        "name": format!("v{n}"),
        "type": "core/u64@1",
        "scope": scope
    });
    if let Some(initial) = initial {
        value["init"] = json!({
            "type": "core/u64@1",
            "value": initial.to_string()
        });
    }
    value
}

fn variable_node(n: u32, name: &str, variable: u32) -> Json {
    let mut node = core(n, name);
    node["config"] = json!({"variable": id(variable)});
    node
}

fn wait_node(n: u32, duration: i64) -> Json {
    let mut node = core(n, "wait");
    node["inputs"] = json!({"duration":{"type":"core/duration@1","value":duration.to_string()}});
    node
}

fn update_local(n: u32, variable: u32) -> Json {
    let mut node = variable_node(n, "update-variable", variable);
    node["config"]["operation"] = json!("add");
    node["inputs"] = json!({
        "value": {"type": "core/u64@1", "value": "1"}
    });
    node
}

fn compile_with_event(
    nodes: Vec<Json>,
    edges: Vec<Json>,
    variables: Vec<Json>,
    event: Json,
) -> Result<(GraphDocument, Arc<orange_compiler::ControlPlan>), orange_compiler::CompileError> {
    let (package, registry) = catalog();
    let source = graph(&package, nodes, edges, vec![]);
    let mut saved: Json = serde_json::from_str(&to_json(&source).unwrap()).unwrap();
    saved["events"] = if event.is_array() {
        event
    } else {
        json!([event])
    };
    saved["variables"] = json!(variables);
    let document = from_json(&saved.to_string()).unwrap();
    let analysis = analyze(&document, &registry);
    compile_control(
        &document,
        registry,
        &analysis,
        &package,
        CompileOptions::default(),
    )
    .map(|plan| (document, Arc::new(plan)))
}

fn compile(
    nodes: Vec<Json>,
    edges: Vec<Json>,
    variables: Vec<Json>,
) -> (GraphDocument, Arc<orange_compiler::ControlPlan>) {
    let (document, plan) = compile_with_event(
        nodes,
        edges,
        variables,
        json!({
            "id": EVENT,
            "name": "pulse",
            "payload": "core/u64@1",
            "scope": "run"
        }),
    )
    .unwrap();
    let (_, registry) = catalog();
    let analysis = analyze(&document, &registry);
    assert!(
        analysis.is_ready_for_lowering(&document, &registry),
        "event graph should be ready: {:?}",
        analysis.report().diagnostics
    );
    (document, plan)
}
fn pump_until_idle(run: &mut Run) {
    for _ in 0..20_000 {
        if run.state() == RunState::Stopped {
            return;
        }
        let progress = run.pump(64).unwrap();
        let events = run.event_stats();
        if !progress.ready && events.pending == 0 {
            return;
        }
    }
    panic!(
        "event fixture did not become idle: state={:?}, now={}, stats={:?}, trace={:?}",
        run.state(),
        run.now(),
        run.event_stats(),
        run.trace()
    );
}

fn run_with_options(
    doc: &GraphDocument,
    plan: Arc<orange_compiler::ControlPlan>,
    options: RunOptions,
) -> Run {
    Run::start(plan.clone(), doc, plan.data().snapshot(), 17, options).unwrap()
}

fn run(doc: &GraphDocument, plan: Arc<orange_compiler::ControlPlan>) -> Run {
    run_with_options(doc, plan, RunOptions::default())
}

#[test]
fn emit_enqueues_typed_payload_and_handler_starts_on_a_later_pump_turn() {
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            emit(2, 42),
            on_event(3, 0, 4, "drop-newest"),
            variable_node(4, "set-variable", 100),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 3, "then", 4, "in"),
            edge(3, 3, "payload", 4, "value"),
        ],
        vec![variable(100, "run", None)],
    );
    let mut run = run(&doc, plan);
    for _ in 0..100 {
        if run
            .trace()
            .iter()
            .any(|event| event.node.to_string() == id(2))
        {
            break;
        }
        run.pump(1).unwrap();
    }
    assert!(
        run.trace()
            .iter()
            .any(|event| event.node.to_string() == id(2))
    );
    assert!(
        !run.trace()
            .iter()
            .any(|event| event.node.to_string() == id(3)),
        "On Event ran inline in the Emit turn"
    );
    assert_eq!(run.variable(id(100).parse().unwrap()), None);
    let stats = run.event_stats();
    assert_eq!((stats.accepted, stats.delivered, stats.pending), (1, 0, 1));
    let metadata = run.last_event().unwrap();
    assert_eq!(metadata.ingestion_seq, 1);
    assert_eq!(
        metadata.origin,
        orange_runtime::EventOrigin::Graph {
            node: id(2).parse().unwrap(),
            scope: 0,
        }
    );
    assert_eq!(metadata.source_seq, 1);
    assert_eq!(metadata.cause_depth, 1);

    pump_until_idle(&mut run);
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&Value::U64(42))
    );
    let stats = run.event_stats();
    assert_eq!((stats.accepted, stats.delivered, stats.pending), (1, 1, 0));
    assert!(
        run.trace().iter().any(|event| {
            event.node.to_string() == id(3) && event.outcome == ControlOutcome::Then
        })
    );
}

#[test]
fn handler_order_is_policy_order_and_each_delivery_gets_fresh_local_cells() {
    let mut first_emit = emit(2, 1);
    first_emit["inputs"]["payload"]["value"] = json!("1");
    let mut second_emit = emit(3, 1);
    second_emit["inputs"]["payload"]["value"] = json!("1");
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            first_emit,
            second_emit,
            on_event(10, 20, 4, "drop-newest"),
            on_event(11, 10, 4, "drop-newest"),
            update_local(20, 101),
            variable_node(21, "get-variable", 101),
            variable_node(22, "set-variable", 100),
            update_local(30, 101),
            variable_node(31, "get-variable", 101),
            variable_node(32, "set-variable", 100),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 10, "then", 20, "in"),
            edge(4, 11, "then", 30, "in"),
            edge(5, 20, "then", 22, "in"),
            edge(6, 21, "value", 22, "value"),
            edge(7, 30, "then", 32, "in"),
            edge(8, 31, "value", 32, "value"),
        ],
        vec![variable(100, "run", None), variable(101, "local", Some(0))],
    );
    let mut run = run(&doc, plan);
    pump_until_idle(&mut run);

    let handler_entries: Vec<_> = run
        .trace()
        .iter()
        .filter(|event| [id(10), id(11)].contains(&event.node.to_string()))
        .map(|event| (event.node.to_string(), event.activation))
        .collect();
    assert_eq!(
        handler_entries
            .iter()
            .map(|(node, _)| node.clone())
            .collect::<Vec<_>>(),
        vec![id(11), id(10), id(11), id(10)]
    );
    let activation_ids: Vec<_> = handler_entries
        .iter()
        .map(|(_, activation)| *activation)
        .collect();
    let unique: std::collections::BTreeSet<_> = activation_ids.iter().copied().collect();
    assert_eq!(
        unique.len(),
        activation_ids.len(),
        "handler activation IDs must not be reused"
    );
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(1)));
    let stats = run.event_stats();
    assert_eq!((stats.accepted, stats.delivered, stats.pending), (4, 4, 0));
}

#[test]
fn entry_fault_with_stop_run_policy_stops_without_leaking_pending_payloads() {
    let mut handler = on_event(3, 0, 4, "drop-newest");
    handler["activation"]["on_fault"] = json!("stop-run");
    let get = variable_node(4, "get-variable", 100);
    let set = variable_node(5, "set-variable", 101);
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            emit(2, 7),
            emit(6, 8),
            handler,
            get,
            set,
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 6, "in"),
            edge(3, 3, "then", 5, "in"),
            edge(4, 4, "value", 5, "value"),
        ],
        vec![variable(100, "run", None), variable(101, "run", None)],
    );
    let mut run = run(&doc, plan);
    pump_until_idle(&mut run);
    assert_eq!(run.state(), RunState::Stopped);
    assert!(run.trace().iter().any(|event| {
        event.node.to_string() == id(5) && event.outcome == ControlOutcome::Failed
    }));
    let stats = run.event_stats();
    assert_eq!(stats.pending, 0);
    assert_eq!(stats.pending_bytes, 0);
    assert_eq!(run.variable(id(101).parse().unwrap()), None);
}

#[test]
fn graceful_stop_drains_accepted_event_but_rejects_stop_handler_emission() {
    let stop = core(7, "on-stop");
    let handler = on_event(3, 0, 4, "drop-newest");
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            emit(2, 42),
            handler,
            variable_node(4, "set-variable", 100),
            stop,
            emit(8, 99),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 3, "then", 4, "in"),
            edge(3, 3, "payload", 4, "value"),
            edge(4, 7, "then", 8, "in"),
        ],
        vec![variable(100, "run", None)],
    );
    let mut run = run(&doc, plan);
    for _ in 0..100 {
        if run.event_stats().pending > 0 {
            break;
        }
        run.pump(1).unwrap();
    }
    assert_eq!(run.event_stats().pending, 1);
    run.request_stop(orange_runtime::StopMode::Graceful)
        .unwrap();
    pump_until_idle(&mut run);
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&Value::U64(42))
    );
    assert!(run.trace().iter().any(|event| {
        event.node.to_string() == id(8)
            && event.outcome == ControlOutcome::Failed
            && event
                .fault
                .as_ref()
                .is_some_and(|fault| fault.fault.code == "core/RunStopping")
    }));
    let stats = run.event_stats();
    assert_eq!((stats.accepted, stats.delivered, stats.pending), (1, 1, 0));
    assert_eq!(stats.pending_bytes, 0);
}

#[test]
fn immediate_stop_discards_pending_event_and_never_starts_its_handler() {
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            emit(2, 42),
            on_event(3, 0, 4, "drop-newest"),
        ],
        vec![edge(1, 1, "then", 2, "in")],
        vec![],
    );
    let mut run = run(&doc, plan);
    for _ in 0..100 {
        if run.event_stats().pending > 0 {
            break;
        }
        run.pump(1).unwrap();
    }
    assert_eq!(run.event_stats().pending, 1);
    run.stop().unwrap();
    pump_until_idle(&mut run);
    assert_eq!(run.state(), RunState::Stopped);
    assert!(
        !run.trace()
            .iter()
            .any(|event| event.node.to_string() == id(3))
    );
    let stats = run.event_stats();
    assert_eq!(stats.pending, 0);
    assert_eq!(stats.pending_bytes, 0);
    assert_eq!((stats.accepted, stats.delivered, stats.dropped), (1, 0, 1));
}

#[test]
fn event_chain_cannot_exceed_configured_causal_depth() {
    let handler = on_event(3, 0, 4, "drop-newest");
    let mut recursive_emit = core(4, "emit");
    recursive_emit["config"] = json!({"event": EVENT});
    let (doc, plan) = compile(
        vec![core(1, "on-start"), emit(2, 1), handler, recursive_emit],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 3, "then", 4, "in"),
            edge(3, 3, "payload", 4, "payload"),
        ],
        vec![],
    );
    let options = RunOptions {
        max_cause_depth: 2,
        ..RunOptions::default()
    };
    let mut run = run_with_options(&doc, plan, options);
    pump_until_idle(&mut run);
    assert!(run.trace().iter().any(|event| {
        event.node.to_string() == id(4)
            && event.outcome == ControlOutcome::Failed
            && event
                .fault
                .as_ref()
                .is_some_and(|fault| fault.fault.code == "core/RunawayEventChain")
    }));
    assert_eq!(run.event_stats().pending, 0);
}

#[test]
fn accepted_event_queue_is_bounded_and_overflow_policy_selects_survivor() {
    for (overflow, expected, faults) in [
        ("drop-newest", 2, false),
        ("drop-oldest", 3, false),
        ("coalesce-latest", 3, false),
        ("fault", 2, true),
    ] {
        let (doc, plan) = compile(
            vec![
                core(1, "on-start"),
                emit(2, 1),
                emit(3, 2),
                emit(4, 3),
                on_event(5, 0, 1, overflow),
                wait_node(6, 10),
                variable_node(7, "set-variable", 100),
            ],
            vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 2, "then", 3, "in"),
                edge(3, 3, "then", 4, "in"),
                edge(4, 5, "then", 6, "in"),
                edge(5, 6, "then", 7, "in"),
                edge(6, 5, "payload", 7, "value"),
            ],
            vec![variable(100, "run", None)],
        );
        let mut run = run(&doc, plan);
        for _ in 0..200 {
            if run.event_stats().delivered > 0 && run.event_stats().overflows > 0 {
                break;
            }
            run.pump(1).unwrap();
        }
        assert_eq!(
            run.event_stats().delivered,
            1,
            "handler must be suspended in Wait"
        );
        assert_eq!(run.event_stats().pending, 1);
        if faults {
            assert!(run.trace().iter().any(|event| {
                event.node.to_string() == id(4)
                    && event.outcome == ControlOutcome::Failed
                    && event
                        .fault
                        .as_ref()
                        .is_some_and(|fault| fault.fault.code == "core/LimitExceeded")
            }));
        } else {
            assert!(run.event_stats().dropped >= 1);
        }
        for _ in 0..5 {
            let now = run.now().saturating_add(10);
            run.advance_to(now).unwrap();
            pump_until_idle(&mut run);
        }
        assert_eq!(
            run.variable(id(100).parse().unwrap()),
            Some(&Value::U64(expected))
        );
        assert_eq!(run.event_stats().pending, 0);
    }
}

#[test]
fn event_admission_rejects_unsupported_scopes_reentrancy_and_payload_types() {
    let event = |scope: &str, payload: &str| json!({"id":EVENT,"name":"pulse","payload":payload,"scope":scope});
    let mut replacement = on_event(3, 0, 4, "drop-newest");
    replacement["activation"]["mode"] = json!("replace");
    let (_, replace_plan) = compile_with_event(
        vec![replacement],
        vec![],
        vec![],
        event("run", "core/u64@1"),
    )
    .expect("Replace is supported for run-scoped events");
    assert_eq!(
        replace_plan.events()[0].handlers()[0].mode(),
        orange_model::document::ReentrancyMode::Replace
    );
    assert_eq!(replace_plan.events()[0].handlers()[0].max_parallel(), 1);
    let mut backpressure = on_event(3, 0, 4, "backpressure");
    backpressure["activation"]["overflow"] = json!("backpressure");
    assert!(
        compile_with_event(
            vec![backpressure],
            vec![],
            vec![],
            event("run", "core/u64@1")
        )
        .is_err()
    );
    assert!(
        compile_with_event(
            vec![on_event(3, 0, 4, "drop-newest")],
            vec![],
            vec![],
            event("workspace", "core/u64@1")
        )
        .is_err()
    );
    assert!(
        compile_with_event(
            vec![on_event(3, 0, 4, "drop-newest")],
            vec![],
            vec![],
            event("run", "core/stream@1<core/u64@1>")
        )
        .is_err()
    );
}

#[test]
fn pending_event_item_and_byte_limits_accept_exact_boundary_and_refuse_one_below() {
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            emit(2, 42),
            on_event(3, 0, 4, "drop-newest"),
        ],
        vec![edge(1, 1, "then", 2, "in")],
        vec![],
    );
    let mut baseline = run(&doc, plan.clone());
    for _ in 0..100 {
        if baseline
            .trace()
            .iter()
            .any(|event| event.node.to_string() == id(2))
        {
            break;
        }
        baseline.pump(1).unwrap();
    }
    let bytes = baseline.event_stats().pending_bytes;
    assert!(bytes > 0);
    assert_eq!(baseline.event_stats().pending, 1);

    let mut exact_options = RunOptions::default();
    exact_options.limits.max_pending_events = 1;
    exact_options.limits.max_pending_event_bytes = bytes;
    let mut exact = run_with_options(&doc, plan.clone(), exact_options);
    for _ in 0..100 {
        if exact
            .trace()
            .iter()
            .any(|event| event.node.to_string() == id(2))
        {
            break;
        }
        exact.pump(1).unwrap();
    }
    assert_eq!(exact.event_stats().accepted, 1);
    assert_eq!(exact.event_stats().pending, 1);
    assert_eq!(exact.event_stats().pending_bytes, bytes);

    let mut over_options = RunOptions::default();
    over_options.limits.max_pending_events = 0;
    over_options.limits.max_pending_event_bytes = bytes - 1;
    let mut over = run_with_options(&doc, plan, over_options);
    for _ in 0..100 {
        if over
            .trace()
            .iter()
            .any(|event| event.node.to_string() == id(2))
        {
            break;
        }
        over.pump(1).unwrap();
    }
    let stats = over.event_stats();
    assert_eq!(
        (stats.accepted, stats.pending, stats.pending_bytes),
        (0, 0, 0)
    );
    assert!(over.trace().iter().any(|event| {
        event.node.to_string() == id(2)
            && event.outcome == ControlOutcome::Failed
            && event
                .fault
                .as_ref()
                .is_some_and(|fault| fault.fault.code == "core/LimitExceeded")
    }));
}

#[test]
fn later_fault_policy_failure_does_not_partially_replace_an_earlier_handler_queue() {
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            emit(2, 1),
            wait_node(3, 10),
            emit(4, 2),
            emit(5, 3),
            on_event(10, 0, 1, "drop-oldest"),
            on_event(12, 1, 1, "fault"),
            wait_node(20, 100),
            variable_node(21, "set-variable", 100),
            wait_node(30, 100),
            variable_node(31, "set-variable", 101),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 3, "then", 4, "in"),
            edge(4, 4, "then", 5, "in"),
            edge(5, 10, "then", 20, "in"),
            edge(6, 10, "payload", 21, "value"),
            edge(7, 20, "then", 21, "in"),
            edge(8, 12, "then", 30, "in"),
            edge(9, 12, "payload", 31, "value"),
            edge(10, 30, "then", 31, "in"),
        ],
        vec![variable(100, "run", None), variable(101, "run", None)],
    );
    let mut run = run(&doc, plan);
    for _ in 0..500 {
        let delivered = run.event_stats().delivered == 2;
        let on_start_waiting = run.trace().iter().any(|event| {
            event.node.to_string() == id(3) && event.outcome == ControlOutcome::Suspended
        });
        if delivered && on_start_waiting {
            break;
        }
        run.pump(1).unwrap();
    }
    run.advance_to(run.now().saturating_add(10)).unwrap();
    for _ in 0..500 {
        if run.event_stats().overflows > 0 {
            break;
        }
        run.pump(1).unwrap();
    }
    let stats = run.event_stats();
    assert_eq!(
        stats.accepted,
        4,
        "stats={stats:?}, trace={:?}",
        run.trace()
    );
    assert_eq!(stats.delivered, 2);
    assert_eq!(stats.pending, 2);
    assert!(run.trace().iter().any(|event| {
        event.node.to_string() == id(5)
            && event.outcome == ControlOutcome::Failed
            && event
                .fault
                .as_ref()
                .is_some_and(|fault| fault.fault.code == "core/LimitExceeded")
    }));

    run.advance_to(110).unwrap();
    pump_until_idle(&mut run);
    run.advance_to(210).unwrap();
    pump_until_idle(&mut run);
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(2)));
    assert_eq!(run.variable(id(101).parse().unwrap()), Some(&Value::U64(2)));
    assert_eq!(run.event_stats().pending, 0);
}

#[test]
fn more_than_sixty_four_emitted_waves_retire_iteratively_with_unique_ids() {
    let mut recursive_emit = core(4, "emit");
    recursive_emit["config"] = json!({"event": EVENT});
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            emit(2, 1),
            on_event(3, 0, 1, "fault"),
            recursive_emit,
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 3, "then", 4, "in"),
            edge(3, 3, "payload", 4, "payload"),
        ],
        vec![],
    );
    let options = RunOptions {
        max_cause_depth: 70,
        limits: orange_runtime::RunLimits {
            max_activations: 2,
            ..orange_runtime::RunLimits::default()
        },
        ..RunOptions::default()
    };
    let mut run = run_with_options(&doc, plan, options);
    pump_until_idle(&mut run);
    let stats = run.event_stats();
    assert_eq!(
        (stats.accepted, stats.delivered, stats.pending),
        (70, 70, 0)
    );
    let mut ids = run
        .trace()
        .iter()
        .filter(|event| event.node.to_string() == id(3))
        .map(|event| event.activation)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 70);
    assert_eq!(stats.activation_high_water, 1);
}

#[test]
fn forked_event_handlers_retire_before_next_root_id_is_minted() {
    let mut fork = core(20, "fork");
    fork["config"] = json!({"join":"all"});
    fork["variadic"] = json!({"branch":{"next":2,"order":[0,1]}});
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            emit(2, 1),
            emit(3, 2),
            on_event(10, 0, 1, "fault"),
            fork,
            core(21, "yield"),
            core(22, "yield"),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 10, "then", 20, "in"),
            edge(4, 20, "branch#0", 21, "in"),
            edge(5, 20, "branch#1", 22, "in"),
        ],
        vec![],
    );
    let mut run = run(&doc, plan);
    pump_until_idle(&mut run);
    assert_eq!(run.event_stats().delivered, 2);
    assert_eq!(run.event_stats().pending, 0);
    let mut activations = run
        .trace()
        .iter()
        .filter(|event| event.node.to_string() == id(10))
        .map(|event| event.activation)
        .collect::<Vec<_>>();
    activations.sort_unstable();
    activations.dedup();
    assert_eq!(activations.len(), 2);
    assert!(
        run.trace()
            .iter()
            .any(|event| event.node.to_string() == id(21))
    );
    assert!(
        run.trace()
            .iter()
            .any(|event| event.node.to_string() == id(22))
    );
}

#[test]
fn event_payload_forwarded_through_action_function_emits_with_callee_cause_metadata() {
    const NEXT_EVENT: &str = "40000000-0000-4000-8000-000000000002";
    let mut relay_emit = emit_for(102, NEXT_EVENT, 0);
    relay_emit.as_object_mut().unwrap().remove("inputs");
    let relay = json!({
        "id": id(100),
        "name": "relay-event-payload",
        "kind": "action",
        "inputs": [{"port_id":"value","direction":"input","kind":"data","type":"core/u64@1"}],
        "outputs": [],
        "nodes": [core(101, "function-entry"), relay_emit],
        "edges": [
            edge(2001, 101, "then", 102, "in"),
            edge(2002, 101, "value", 102, "payload")
        ]
    });
    let mut call = core(4, "call");
    call["config"] = json!({"function": id(100)});
    let (package, registry) = catalog();
    let source = graph(
        &package,
        vec![
            core(1, "on-start"),
            emit(2, 73),
            on_event(3, 0, 4, "drop-newest"),
            call,
            on_event_for(5, NEXT_EVENT, 0, 4, "drop-newest"),
            variable_node(6, "set-variable", 1000),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 3, "then", 4, "in"),
            edge(3, 3, "payload", 4, "value"),
            edge(4, 5, "then", 6, "in"),
            edge(5, 5, "payload", 6, "value"),
        ],
        vec![],
    );
    let mut saved: Json = serde_json::from_str(&to_json(&source).unwrap()).unwrap();
    saved["events"] = json!([
        {"id":EVENT,"name":"incoming","payload":"core/u64@1","scope":"run"},
        {"id":NEXT_EVENT,"name":"forwarded","payload":"core/u64@1","scope":"run"}
    ]);
    saved["functions"] = json!([relay]);
    saved["variables"] = json!([variable(1000, "run", None)]);
    let doc = from_json(&saved.to_string()).unwrap();
    let analysis = analyze(&doc, &registry);
    assert!(
        analysis.is_ready_for_lowering(&doc, &registry),
        "function event graph should be ready: {:?}",
        analysis.report().diagnostics
    );
    let plan = Arc::new(
        compile_control(
            &doc,
            registry,
            &analysis,
            &package,
            CompileOptions::default(),
        )
        .unwrap(),
    );
    let mut run = run(&doc, plan);
    pump_until_idle(&mut run);

    assert_eq!(
        run.variable(id(1000).parse().unwrap()),
        Some(&Value::U64(73))
    );
    let stats = run.event_stats();
    assert_eq!((stats.accepted, stats.delivered, stats.pending), (2, 2, 0));
    let cause_activation = run
        .trace()
        .iter()
        .find(|event| event.node.to_string() == id(3))
        .expect("first event handler should execute")
        .activation;
    let metadata = run.last_event().expect("callee Emit should be ingested");
    assert_eq!(metadata.ingestion_seq, 2);
    assert!(
        matches!(metadata.origin, orange_runtime::EventOrigin::Graph { node, scope } if node.to_string() == id(102) && scope > 0)
    );
    assert_eq!(metadata.source_seq, 1);
    assert_eq!(metadata.cause, Some(cause_activation));
    assert_eq!(metadata.cause_depth, 2);
}

#[test]
fn async_action_wait_retains_event_argument_and_emits_with_callee_provenance() {
    const NEXT_EVENT: &str = "40000000-0000-4000-8000-000000000004";
    let mut relay_emit = emit_for(103, NEXT_EVENT, 0);
    relay_emit.as_object_mut().unwrap().remove("inputs");
    let relay_wait = wait_node(102, 5);
    let relay = json!({
        "id": id(100),
        "name": "async relay after wait",
        "kind": "async",
        "inputs": [{"port_id":"value","direction":"input","kind":"data","type":"core/u64@1"}],
        "outputs": [],
        "nodes": [core(101, "function-entry"), relay_wait, relay_emit],
        "edges": [
            edge(2001, 101, "then", 102, "in"),
            edge(2002, 102, "then", 103, "in"),
            edge(2003, 101, "value", 103, "payload")
        ]
    });
    let mut call = core(4, "call");
    call["config"] = json!({"function":id(100)});
    let (package, registry) = catalog();
    let source = graph(
        &package,
        vec![
            core(1, "on-start"),
            emit(2, 73),
            on_event(3, 0, 4, "drop-newest"),
            call,
            on_event_for(5, NEXT_EVENT, 0, 4, "drop-newest"),
            variable_node(6, "set-variable", 1000),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 3, "then", 4, "in"),
            edge(3, 3, "payload", 4, "value"),
            edge(4, 5, "then", 6, "in"),
            edge(5, 5, "payload", 6, "value"),
        ],
        vec![],
    );
    let mut saved: Json = serde_json::from_str(&to_json(&source).unwrap()).unwrap();
    saved["events"] = json!([
        {"id":EVENT,"name":"incoming","payload":"core/u64@1","scope":"run"},
        {"id":NEXT_EVENT,"name":"forwarded","payload":"core/u64@1","scope":"run"}
    ]);
    saved["functions"] = json!([relay]);
    saved["variables"] = json!([variable(1000, "run", None)]);
    let doc = from_json(&saved.to_string()).unwrap();
    let analysis = analyze(&doc, &registry);
    assert!(
        analysis.is_ready_for_lowering(&doc, &registry),
        "async event graph should be ready: {:?}",
        analysis.report().diagnostics
    );
    let plan = Arc::new(
        compile_control(
            &doc,
            registry,
            &analysis,
            &package,
            CompileOptions::default(),
        )
        .unwrap(),
    );
    let mut run = run(&doc, plan);
    for _ in 0..500 {
        if run.trace().iter().any(|event| {
            event.node.to_string() == id(102) && event.outcome == ControlOutcome::Suspended
        }) {
            break;
        }
        run.pump(1).unwrap();
    }
    assert!(run.trace().iter().any(|event| {
        event.node.to_string() == id(102) && event.outcome == ControlOutcome::Suspended
    }));
    assert_eq!(run.event_stats().accepted, 1);
    assert_eq!(run.event_stats().delivered, 1);
    assert_eq!(run.variable(id(1000).parse().unwrap()), None);
    let incoming = run.last_event().expect("incoming event should be recorded");
    assert_eq!(incoming.ingestion_seq, 1);
    assert_eq!(
        incoming.origin,
        orange_runtime::EventOrigin::Graph {
            node: id(2).parse().unwrap(),
            scope: 0,
        }
    );
    assert_eq!(incoming.monotonic_ns, 0);

    run.advance_to(5).unwrap();
    for _ in 0..500 {
        if run.variable(id(1000).parse().unwrap()) == Some(&Value::U64(73)) {
            break;
        }
        let progress = run.pump(1).unwrap();
        if !progress.ready {
            if let Some(next_wakeup) = progress.next_wakeup {
                run.advance_to(next_wakeup).unwrap();
            } else {
                break;
            }
        }
    }
    assert_eq!(
        run.variable(id(1000).parse().unwrap()),
        Some(&Value::U64(73)),
        "async callee must retain the original handler payload across Wait"
    );
    assert_eq!(
        (run.event_stats().accepted, run.event_stats().delivered),
        (2, 2)
    );
    let cause_activation = run
        .trace()
        .iter()
        .find(|event| event.node.to_string() == id(3))
        .expect("source handler should execute")
        .activation;
    let metadata = run.last_event().expect("callee Emit should be ingested");
    assert_eq!(metadata.ingestion_seq, 2);
    assert!(
        matches!(metadata.origin, orange_runtime::EventOrigin::Graph { node, scope } if node.to_string() == id(103) && scope > 0)
    );
    assert_eq!(metadata.source_seq, 1);
    assert_eq!(metadata.cause, Some(cause_activation));
    assert_eq!(metadata.cause_depth, 2);
    assert_eq!(metadata.monotonic_ns, 5);
}

#[test]
fn unsupported_event_scope_admission_precedes_demanded_provider_folding() {
    let (package, registry) = catalog();
    let scope = "workspace";
    {
        let handler = on_event(4, 0, 4, "drop-newest");
        let mut emitted = emit(2, 0);
        emitted.as_object_mut().unwrap().remove("inputs");
        let source = graph(
            &package,
            vec![core(1, "on-start"), emitted, math(3, "add", 1, 2), handler],
            vec![edge(1, 1, "then", 2, "in"), edge(2, 3, "sum", 2, "payload")],
            vec![],
        );
        let mut saved: Json = serde_json::from_str(&to_json(&source).unwrap()).unwrap();
        saved["events"] = json!([{
            "id":EVENT,
            "name":"unsupported",
            "payload":"core/u64@1",
            "scope":scope
        }]);
        let doc = from_json(&saved.to_string()).unwrap();
        let analysis = analyze(&doc, &registry);
        assert!(
            analysis.is_ready_for_lowering(&doc, &registry),
            "fixture should pass model validation so compiler admission is exercised: {:?}",
            analysis.report().diagnostics
        );
        let result = compile_control(
            &doc,
            registry.clone(),
            &analysis,
            &PanickingEventResolver(package.clone()),
            CompileOptions::default(),
        );
        let expected_node = id(2).parse().unwrap();
        assert!(
            matches!(
                result,
                Err(orange_compiler::CompileError::UnsupportedNode { node })
                    if node == expected_node
            ),
            "event scope should be typed unsupported before resolving/folding math: {result:?}"
        );
    }
}

/// Trusted provider failure while a dynamic event activation owns its data frame.
struct PanickingEventResolver(orange_bundled::MathPackage);

#[derive(Debug)]
struct PanickingEventKernel;

impl orange_compiler::PureKernel for PanickingEventKernel {
    fn fold(&self, _: &[Value], _: &mut [Value]) -> Result<(), orange_compiler::KernelFault> {
        panic!("synthetic provider panic during event activation")
    }
}

impl orange_compiler::PureKernelResolver for PanickingEventResolver {
    fn resolve_binding(
        &self,
        request: &orange_compiler::BindingRequest<'_>,
    ) -> Result<orange_compiler::KernelBinding, orange_compiler::ResolveError> {
        let mut binding = self.0.resolve_binding(request)?;
        binding.kernel = Arc::new(PanickingEventKernel);
        Ok(binding)
    }
}

impl orange_compiler::EntropyKernelResolver for PanickingEventResolver {
    fn resolve_entropy_binding(
        &self,
        request: &orange_compiler::BindingRequest<'_>,
    ) -> Result<orange_compiler::EntropyBinding, orange_compiler::ResolveError> {
        self.0.resolve_entropy_binding(request)
    }
}

#[test]
fn panicking_provider_retires_checked_out_event_activation_on_stop_and_dispose() {
    const NEXT_EVENT: &str = "40000000-0000-4000-8000-000000000003";
    let mut add = math(4, "add", 0, 1);
    add["inputs"] = json!({"b":{"type":"core/u64@1","value":"1"}});
    let (package, registry) = catalog();
    let source = graph(
        &package,
        vec![
            core(1, "on-start"),
            emit(2, 41),
            on_event(3, 0, 4, "drop-newest"),
            add,
            emit_for(5, NEXT_EVENT, 0),
            on_event_for(6, NEXT_EVENT, 0, 4, "drop-newest"),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 3, "then", 5, "in"),
            edge(3, 3, "payload", 4, "a"),
            edge(4, 4, "sum", 5, "payload"),
        ],
        vec![],
    );
    let mut saved: Json = serde_json::from_str(&orange_model::to_json(&source).unwrap()).unwrap();
    saved["events"] = json!([
        {"id":EVENT,"name":"incoming","payload":"core/u64@1","scope":"run"},
        {"id":NEXT_EVENT,"name":"after-panic","payload":"core/u64@1","scope":"run"}
    ]);
    let doc = from_json(&saved.to_string()).unwrap();
    let analysis = analyze(&doc, &registry);
    assert!(
        analysis.is_ready_for_lowering(&doc, &registry),
        "panicking event graph should be ready: {:?}",
        analysis.report().diagnostics
    );
    let plan = Arc::new(
        compile_control(
            &doc,
            registry.clone(),
            &analysis,
            &PanickingEventResolver(package),
            CompileOptions::default(),
        )
        .unwrap(),
    );
    let mut run = Run::start(plan, &doc, &registry, 29, RunOptions::default()).unwrap();
    let host_source = run.register_event_source(EVENT.parse().unwrap()).unwrap();

    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run.pump(1024)));
    assert!(unwound.is_err());
    assert_eq!(
        run.ingest_event(&host_source, 1, &Value::U64(5), None),
        Err(orange_runtime::IngressError::NotRunning),
        "an unwound run rejects host input before explicit cleanup"
    );
    assert_eq!(
        run.register_event_source(EVENT.parse().unwrap()),
        Err(orange_runtime::IngressError::NotRunning)
    );
    assert_eq!(run.state(), RunState::Faulted);
    assert_eq!(
        run.last_error(),
        Some(orange_runtime::RunError::Interrupted)
    );
    assert!(matches!(
        run.pump(1024),
        Err(orange_runtime::RunError::Interrupted)
    ));
    let stats = run.event_stats();
    assert_eq!((stats.accepted, stats.delivered, stats.pending), (1, 1, 0));
    assert!(run.last_event().is_some_and(|event| matches!(
        event.origin,
        orange_runtime::EventOrigin::Graph { node, scope: 0 }
            if node == id(2).parse().unwrap()
    )));
    assert!(
        !run.trace()
            .iter()
            .any(|event| event.node.to_string() == id(6))
    );

    run.stop().unwrap();
    for _ in 0..64 {
        if run.state() == RunState::Stopped {
            break;
        }
        let _ = run.pump(1024);
    }
    assert_eq!(run.state(), RunState::Stopped);
    let owners = run.ownership();
    assert_eq!(
        (owners.activations, owners.strands, owners.regions),
        (0, 0, 0)
    );
    assert_eq!(run.event_stats().pending_bytes, 0);
    assert_eq!(run.event_stats().live_payload_bytes, 0);
    run.dispose().unwrap();
    assert_eq!(run.state(), RunState::Disposed);
    assert_eq!(
        run.ownership(),
        orange_runtime::RunOwnership {
            activations: 0,
            strands: 0,
            regions: 0,
            timers: 0,
            event_waiters: 0,
            host_sources: 0,
            resources: 0,
        }
    );
    assert_eq!(run.event_stats().live_payload_bytes, 0);
}

#[test]
fn live_payload_budget_is_reserved_through_active_handlers_and_released_on_retirement() {
    let event = json!({
        "id": STRING_EVENT,
        "name": "text pulse",
        "payload": "core/string@1",
        "scope": "run"
    });
    let nodes = vec![
        core(1, "on-start"),
        emit_string(2, STRING_EVENT, "first-payload"),
        wait_node(3, 5),
        emit_string(4, STRING_EVENT, "second-payload"),
        on_event_for(10, STRING_EVENT, 0, 4, "drop-newest"),
        wait_node(11, 20),
        variable_node(12, "set-variable", 100),
        on_event_for(20, STRING_EVENT, 1, 4, "drop-newest"),
        wait_node(21, 20),
        variable_node(22, "set-variable", 101),
    ];
    let edges = vec![
        edge(1, 1, "then", 2, "in"),
        edge(2, 2, "then", 3, "in"),
        edge(3, 3, "then", 4, "in"),
        edge(4, 10, "then", 11, "in"),
        edge(5, 10, "payload", 12, "value"),
        edge(6, 11, "then", 12, "in"),
        edge(7, 20, "then", 21, "in"),
        edge(8, 20, "payload", 22, "value"),
        edge(9, 21, "then", 22, "in"),
    ];
    let variables = vec![string_variable(100), string_variable(101)];
    let (doc, plan) = compile_with_event(nodes, edges, variables, event).unwrap();

    // Discover the exact nominal charge for one payload fanned out to both handlers.
    let mut baseline = run(&doc, plan.clone());
    for _ in 0..2_000 {
        if baseline.event_stats().delivered == 2
            && baseline
                .trace()
                .iter()
                .filter(|item| item.outcome == ControlOutcome::Suspended)
                .count()
                >= 3
        {
            break;
        }
        baseline.pump(1).unwrap();
    }
    let exact_charge = baseline.event_stats().live_payload_bytes;
    assert!(exact_charge > 0);
    assert_eq!(baseline.event_stats().accepted, 2);
    assert_eq!(baseline.event_stats().delivered, 2);
    assert_eq!(baseline.event_stats().pending, 0);

    let options = RunOptions {
        limits: orange_runtime::RunLimits {
            max_live_event_payload_bytes: exact_charge,
            ..orange_runtime::RunLimits::default()
        },
        ..RunOptions::default()
    };
    let mut run = run_with_options(&doc, plan, options);
    for _ in 0..2_000 {
        if run.event_stats().delivered == 2
            && run
                .trace()
                .iter()
                .filter(|item| item.outcome == ControlOutcome::Suspended)
                .count()
                >= 3
        {
            break;
        }
        run.pump(1).unwrap();
    }
    assert_eq!(run.event_stats().live_payload_bytes, exact_charge);
    assert_eq!(run.event_stats().accepted, 2);

    // The first two handler roots own the first payload while suspended. The next
    // distinct payload would exceed the aggregate budget and must be rejected atomically.
    run.advance_to(5).unwrap();
    for _ in 0..500 {
        if run
            .trace()
            .iter()
            .any(|item| item.node.to_string() == id(4))
        {
            break;
        }
        run.pump(1).unwrap();
    }
    assert!(run.trace().iter().any(|item| {
        item.node.to_string() == id(4)
            && item.outcome == ControlOutcome::Failed
            && item
                .fault
                .as_ref()
                .is_some_and(|fault| fault.fault.code == "core/LimitExceeded")
    }));
    assert_eq!(run.event_stats().accepted, 2);
    assert_eq!(run.event_stats().pending, 0);
    assert_eq!(run.event_stats().live_payload_bytes, exact_charge);

    // Once both handler roots retire, their reservations are released.
    run.advance_to(20).unwrap();
    for _ in 0..500 {
        if run.event_stats().live_payload_bytes == 0 {
            break;
        }
        run.pump(1).unwrap();
    }
    assert_eq!(run.event_stats().live_payload_bytes, 0);
}

#[test]
fn drop_newest_only_drops_the_full_fanout_target() {
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            emit(2, 1),
            wait_node(3, 5),
            emit(4, 2),
            emit(5, 3),
            on_event(10, 0, 1, "drop-newest"),
            wait_node(11, 20),
            variable_node(12, "set-variable", 100),
            on_event(20, 1, 4, "drop-newest"),
            wait_node(21, 20),
            variable_node(22, "set-variable", 101),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 3, "then", 4, "in"),
            edge(4, 4, "then", 5, "in"),
            edge(5, 10, "then", 11, "in"),
            edge(6, 10, "payload", 12, "value"),
            edge(7, 11, "then", 12, "in"),
            edge(8, 20, "then", 21, "in"),
            edge(9, 20, "payload", 22, "value"),
            edge(10, 21, "then", 22, "in"),
        ],
        vec![variable(100, "run", None), variable(101, "run", None)],
    );
    let mut run = run(&doc, plan);
    for _ in 0..500 {
        if run.event_stats().delivered == 2
            && run.trace().iter().any(|item| {
                item.node.to_string() == id(3) && item.outcome == ControlOutcome::Suspended
            })
            && run
                .trace()
                .iter()
                .filter(|item| {
                    matches!(item.node.to_string().as_str(), value if value == id(11) || value == id(21))
                        && item.outcome == ControlOutcome::Suspended
                })
                .count()
                == 2
        {
            break;
        }
        run.pump(1).unwrap();
    }
    run.advance_to(5).unwrap();
    for _ in 0..500 {
        if run.event_stats().accepted == 5 {
            break;
        }
        run.pump(1).unwrap();
    }
    let stats = run.event_stats();
    assert_eq!((stats.accepted, stats.dropped, stats.pending), (5, 1, 3));
    assert_eq!(stats.delivered, 2);
    assert_eq!(run.variable(id(100).parse().unwrap()), None);
    assert_eq!(run.variable(id(101).parse().unwrap()), None);

    run.advance_to(20).unwrap();
    for _ in 0..500 {
        let both_waiting = run.trace().iter().any(|item| {
            item.node.to_string() == id(11) && item.outcome == ControlOutcome::Suspended
        }) && run.trace().iter().any(|item| {
            item.node.to_string() == id(21) && item.outcome == ControlOutcome::Suspended
        });
        if run.event_stats().delivered == 4 && both_waiting {
            break;
        }
        run.pump(1).unwrap();
    }
    assert_eq!(run.event_stats().delivered, 4);
    assert_eq!(run.event_stats().pending, 1);
    for _ in 0..500 {
        if run.event_stats().delivered == 5 {
            break;
        }
        let progress = run.pump(1).unwrap();
        if !progress.ready {
            if let Some(next_wakeup) = progress.next_wakeup {
                run.advance_to(next_wakeup).unwrap();
            } else {
                break;
            }
        }
    }
    assert_eq!(
        run.event_stats().delivered,
        5,
        "stats={:?}, now={}, trace={:?}",
        run.event_stats(),
        run.now(),
        run.trace()
    );
    for _ in 0..500 {
        if run.variable(id(100).parse().unwrap()) == Some(&Value::U64(2))
            && run.variable(id(101).parse().unwrap()) == Some(&Value::U64(3))
        {
            break;
        }
        let progress = run.pump(1).unwrap();
        if !progress.ready {
            if let Some(next_wakeup) = progress.next_wakeup {
                run.advance_to(next_wakeup).unwrap();
            } else {
                break;
            }
        }
    }
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(2)));
    assert_eq!(run.variable(id(101).parse().unwrap()), Some(&Value::U64(3)));
    assert_eq!(run.event_stats().pending, 0);
}

#[test]
fn accepted_handler_drain_refuses_descendant_emit() {
    let child_event = "40000000-0000-4000-8000-000000000003";
    let parent = json!([{
        "id": EVENT,
        "name": "parent pulse",
        "payload": "core/u64@1",
        "scope": "run"
    }, {
        "id": child_event,
        "name": "child pulse",
        "payload": "core/u64@1",
        "scope": "run"
    }]);
    let nodes = vec![
        core(1, "on-start"),
        emit(2, 7),
        on_event(3, 0, 4, "drop-newest"),
        wait_node(4, 10),
        emit_for(5, child_event, 9),
        on_event_for(6, child_event, 0, 4, "drop-newest"),
    ];
    let (doc, plan) = compile_with_event(
        nodes,
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 3, "then", 4, "in"),
            edge(3, 4, "then", 5, "in"),
        ],
        vec![],
        parent,
    )
    .unwrap();
    let mut run = run(&doc, plan);
    for _ in 0..500 {
        let waiting_handler = run.trace().iter().any(|item| {
            item.node.to_string() == id(4) && item.outcome == ControlOutcome::Suspended
        });
        if run.event_stats().delivered == 1 && waiting_handler {
            break;
        }
        run.pump(1).unwrap();
    }
    assert_eq!(run.event_stats().delivered, 1);
    run.request_stop(orange_runtime::StopMode::Graceful)
        .unwrap();
    run.advance_to(10).unwrap();
    pump_until_idle(&mut run);

    assert_eq!(run.state(), RunState::Stopped);
    assert!(run.trace().iter().any(|item| {
        item.node.to_string() == id(5)
            && item.outcome == ControlOutcome::Failed
            && item
                .fault
                .as_ref()
                .is_some_and(|fault| fault.fault.code == "core/RunStopping")
    }));
    assert!(
        !run.trace()
            .iter()
            .any(|item| item.node.to_string() == id(6))
    );
    assert_eq!(run.event_stats().accepted, 1);
    assert_eq!(run.event_stats().delivered, 1);
    assert_eq!(run.event_stats().pending, 0);
    assert_eq!(run.event_stats().pending_bytes, 0);
}

#[test]
fn queued_handler_blocked_by_activation_limit_waits_for_timer_without_losing_work() {
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            emit(2, 42),
            wait_node(3, 100),
            on_event(4, 0, 4, "drop-newest"),
            variable_node(5, "set-variable", 100),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 4, "then", 5, "in"),
            edge(4, 4, "payload", 5, "value"),
        ],
        vec![variable(100, "run", None)],
    );
    let options = RunOptions {
        limits: orange_runtime::RunLimits {
            max_activations: 1,
            ..orange_runtime::RunLimits::default()
        },
        ..RunOptions::default()
    };
    let mut run = run_with_options(&doc, plan, options);
    let mut progress = None;
    for _ in 0..500 {
        let current = run.pump(1).unwrap();
        if current.next_wakeup == Some(100) && run.event_stats().pending == 1 {
            progress = Some(current);
            break;
        }
    }
    let progress =
        progress.expect("start wait and queued event should reach a stable blocked point");
    assert!(!progress.ready);
    assert_eq!(progress.next_wakeup, Some(100));
    assert_eq!(run.event_stats().accepted, 1);
    assert_eq!(run.event_stats().delivered, 0);
    assert_eq!(run.event_stats().pending, 1);
    assert_eq!(run.variable(id(100).parse().unwrap()), None);

    run.advance_to(100).unwrap();
    pump_until_idle(&mut run);
    assert_eq!(run.state(), RunState::Running);
    assert_eq!(run.event_stats().delivered, 1);
    assert_eq!(run.event_stats().pending, 0);
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&Value::U64(42))
    );
}

#[path = "support/budget_events.rs"]
mod budget_events;

#[path = "support/event_modes.rs"]
mod event_modes;

#[path = "support/event_replace.rs"]
mod event_replace;

#[path = "support/event_replace_limits.rs"]
mod event_replace_limits;

#[path = "support/event_replace_owned.rs"]
mod event_replace_owned;

#[path = "support/event_await.rs"]
mod event_await;

#[path = "support/event_ingress.rs"]
mod event_ingress;

#[path = "support/manual_entry.rs"]
mod manual_entry;

#[path = "support/timer_entry.rs"]
mod timer_entry;

#[path = "support/clock_reader.rs"]
mod clock_reader;

#[path = "support/run_resources.rs"]
mod run_resources;
