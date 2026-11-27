//! Acceptance fixtures for `core/await-event` (C03).
use super::*;

const OTHER_EVENT: &str = "40000000-0000-4000-8000-000000000003";

fn await_event(n: u32, event: &str, timeout: Option<i64>) -> Json {
    let mut node = core(n, "await-event");
    node["config"] = json!({"event": event});
    if let Some(timeout) = timeout {
        node["inputs"] = json!({
            "timeout": {
                "type": "core/option@1<core/duration@1>",
                "value": {"some": timeout.to_string()}
            }
        });
    }
    node
}

fn pump_until_trace(run: &mut Run, node: u32, outcome: ControlOutcome) {
    for _ in 0..2000 {
        if run
            .trace()
            .iter()
            .any(|item| item.node.to_string() == id(node) && item.outcome == outcome)
        {
            return;
        }
        run.pump(1).unwrap();
    }
    panic!(
        "node {node} did not reach {outcome:?}: trace={:?}, stats={:?}, ownership={:?}, retention={:?}",
        run.trace(),
        run.event_stats(),
        run.ownership(),
        run.retention()
    );
}

fn stop_and_assert_released(run: &mut Run) {
    run.request_stop(orange_runtime::StopMode::Immediate)
        .unwrap();
    for _ in 0..4000 {
        if run.state() == RunState::Stopped {
            break;
        }
        run.pump(64).unwrap();
    }
    assert_eq!(run.state(), RunState::Stopped);
    let owned = run.ownership();
    assert_eq!(
        (
            owned.activations,
            owned.strands,
            owned.regions,
            owned.timers,
            owned.event_waiters
        ),
        (0, 0, 0, 0, 0)
    );
    assert_eq!(run.event_stats().pending, 0);
    assert_eq!(run.event_stats().live_payload_bytes, 0);
    assert_eq!(run.retention().reserved_bytes, 0);
    run.dispose().unwrap();
}

#[test]
fn await_event_is_compile_ready_and_broadcasts_to_parked_waiters_without_backlog() {
    let (doc, plan) = compile_with_event(
        vec![
            core(1, "on-start"),
            emit(30, 1),
            await_event(2, EVENT, None),
            variable_node(3, "set-variable", 100),
            core(10, "on-start"),
            wait_node(13, 1),
            await_event(11, EVENT, None),
            variable_node(12, "set-variable", 101),
            core(20, "on-start"),
            wait_node(21, 1),
            emit_for(22, OTHER_EVENT, 41),
            wait_node(23, 1),
            emit(24, 73),
        ],
        vec![
            edge(1, 1, "then", 30, "in"),
            edge(2, 30, "then", 2, "in"),
            edge(3, 2, "then", 3, "in"),
            edge(4, 2, "payload", 3, "value"),
            edge(5, 10, "then", 13, "in"),
            edge(12, 13, "then", 11, "in"),
            edge(6, 11, "then", 12, "in"),
            edge(7, 11, "payload", 12, "value"),
            edge(8, 20, "then", 21, "in"),
            edge(9, 21, "then", 22, "in"),
            edge(10, 22, "then", 23, "in"),
            edge(11, 23, "then", 24, "in"),
        ],
        vec![variable(100, "run", None), variable(101, "run", None)],
        json!([
            {"id": EVENT, "name": "pulse", "payload": "core/u64@1", "scope": "run"},
            {"id": OTHER_EVENT, "name": "other", "payload": "core/u64@1", "scope": "run"}
        ]),
    )
    .unwrap();
    let mut run = run(&doc, plan.clone());
    for _ in 0..2000 {
        let first_waiting = run.trace().iter().any(|item| {
            item.node.to_string() == id(2) && item.outcome == ControlOutcome::Suspended
        });
        let second_root_waiting = run.trace().iter().any(|item| {
            item.node.to_string() == id(13) && item.outcome == ControlOutcome::Suspended
        });
        let wrong_emitter_waiting = run.trace().iter().any(|item| {
            item.node.to_string() == id(21) && item.outcome == ControlOutcome::Suspended
        });
        if first_waiting
            && second_root_waiting
            && wrong_emitter_waiting
            && run.event_stats().accepted == 0
            && run
                .trace()
                .iter()
                .any(|item| item.node.to_string() == id(30) && item.outcome == ControlOutcome::Then)
        {
            break;
        }
        run.pump(1).unwrap();
    }
    assert_eq!(
        run.event_stats().accepted,
        0,
        "pre-registration Emit is not replayed"
    );
    assert_eq!(run.variable(id(100).parse().unwrap()), None);
    assert_eq!(run.variable(id(101).parse().unwrap()), None);
    run.advance_to(1).unwrap();
    pump_until_trace(&mut run, 11, ControlOutcome::Suspended);
    pump_until_trace(&mut run, 22, ControlOutcome::Then);
    assert_eq!(run.variable(id(100).parse().unwrap()), None);
    assert_eq!(run.variable(id(101).parse().unwrap()), None);
    pump_until_trace(&mut run, 23, ControlOutcome::Suspended);
    run.advance_to(2).unwrap();
    pump_until_trace(&mut run, 24, ControlOutcome::Then);
    pump_until_trace(&mut run, 2, ControlOutcome::Then);
    pump_until_trace(&mut run, 11, ControlOutcome::Then);
    pump_until_trace(&mut run, 3, ControlOutcome::Then);
    pump_until_trace(&mut run, 12, ControlOutcome::Then);
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&Value::U64(73))
    );
    assert_eq!(
        run.variable(id(101).parse().unwrap()),
        Some(&Value::U64(73))
    );
    assert_eq!(
        run.event_stats().accepted,
        2,
        "one accepted delivery per parked waiter"
    );
}

#[test]
fn await_event_timeout_wins_at_virtual_deadline_and_stop_cancels_indefinite_wait() {
    let (doc, plan) = compile(
        vec![core(1, "on-start"), await_event(2, EVENT, Some(5))],
        vec![edge(1, 1, "then", 2, "in")],
        vec![],
    );
    let mut timed = run(&doc, plan);
    pump_until_trace(&mut timed, 2, ControlOutcome::Suspended);
    timed.advance_to(5).unwrap();
    pump_until_trace(&mut timed, 2, ControlOutcome::TimedOut);

    let (doc, plan) = compile(
        vec![core(1, "on-start"), await_event(2, EVENT, None)],
        vec![edge(1, 1, "then", 2, "in")],
        vec![],
    );
    let mut run = run(&doc, plan);
    pump_until_trace(&mut run, 2, ControlOutcome::Suspended);
    assert!(
        !run.trace()
            .iter()
            .any(|item| { item.node.to_string() == id(2) && item.outcome == ControlOutcome::Then })
    );
    stop_and_assert_released(&mut run);
}
#[test]
fn same_activation_fork_can_emit_to_its_parked_await_without_inline_continuation() {
    let mut fork = core(2, "fork");
    fork["config"] = json!({"join":"all"});
    fork["variadic"] = json!({"branch":{"next":2,"order":[0,1]}});
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            fork,
            await_event(3, EVENT, None),
            variable_node(4, "set-variable", 100),
            wait_node(5, 1),
            emit(6, 73),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "branch#0", 3, "in"),
            edge(3, 3, "then", 4, "in"),
            edge(4, 3, "payload", 4, "value"),
            edge(5, 2, "branch#1", 5, "in"),
            edge(6, 5, "then", 6, "in"),
        ],
        vec![variable(100, "run", None)],
    );
    let mut run = run(&doc, plan);
    pump_until_trace(&mut run, 3, ControlOutcome::Suspended);
    pump_until_trace(&mut run, 5, ControlOutcome::Suspended);
    assert_eq!(run.variable(id(100).parse().unwrap()), None);
    run.advance_to(1).unwrap();
    pump_until_trace(&mut run, 6, ControlOutcome::Then);
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        None,
        "Emit must not run the resumed branch inline"
    );
    pump_until_trace(&mut run, 3, ControlOutcome::Then);
    pump_until_trace(&mut run, 4, ControlOutcome::Then);
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&Value::U64(73))
    );
}

#[test]
fn await_rearm_ignores_old_timeout_and_zero_timeout_suspends_once() {
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            await_event(2, EVENT, Some(5)),
            await_event(3, EVENT, Some(10)),
            core(10, "on-start"),
            wait_node(11, 1),
            emit(12, 9),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 10, "then", 11, "in"),
            edge(4, 11, "then", 12, "in"),
        ],
        vec![],
    );
    let mut rearmed = run(&doc, plan);
    pump_until_trace(&mut rearmed, 2, ControlOutcome::Suspended);
    pump_until_trace(&mut rearmed, 11, ControlOutcome::Suspended);
    rearmed.advance_to(1).unwrap();
    pump_until_trace(&mut rearmed, 2, ControlOutcome::Then);
    pump_until_trace(&mut rearmed, 3, ControlOutcome::Suspended);
    rearmed.advance_to(5).unwrap();
    rearmed.pump(64).unwrap();
    assert!(
        !rearmed.trace().iter().any(|item| {
            item.node.to_string() == id(3) && item.outcome == ControlOutcome::TimedOut
        }),
        "the first wait's stale deadline must not time out the rearmed wait"
    );
    rearmed.advance_to(11).unwrap();
    pump_until_trace(&mut rearmed, 3, ControlOutcome::TimedOut);

    let (doc, plan) = compile(
        vec![core(1, "on-start"), await_event(2, EVENT, Some(0))],
        vec![edge(1, 1, "then", 2, "in")],
        vec![],
    );
    let mut zero = run(&doc, plan);
    pump_until_trace(&mut zero, 2, ControlOutcome::Suspended);
    assert!(!zero.trace().iter().any(|item| {
        item.node.to_string() == id(2) && item.outcome == ControlOutcome::TimedOut
    }));
    pump_until_trace(&mut zero, 2, ControlOutcome::TimedOut);
}

fn quota_graph() -> (GraphDocument, Arc<orange_compiler::ControlPlan>) {
    let (doc, plan) = compile_with_event(
        vec![
            core(1, "on-start"),
            await_event(2, STRING_EVENT, None),
            wait_node(3, 10),
            variable_node(4, "set-variable", 201),
            core(10, "on-start"),
            wait_node(11, 1),
            emit_string(12, STRING_EVENT, "abcd"),
            on_event_for(20, STRING_EVENT, 0, 4, "drop-newest"),
            wait_node(21, 10),
            variable_node(22, "set-variable", 200),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 2, "payload", 4, "value"),
            edge(4, 3, "then", 4, "in"),
            edge(5, 10, "then", 11, "in"),
            edge(6, 11, "then", 12, "in"),
            edge(7, 20, "then", 21, "in"),
            edge(8, 20, "payload", 22, "value"),
            edge(9, 21, "then", 22, "in"),
        ],
        vec![string_variable(200), string_variable(201)],
        json!({"id": STRING_EVENT, "name":"text", "payload":"core/string@1", "scope":"run"}),
    )
    .unwrap();
    (doc, plan)
}

#[test]
fn handler_and_waiter_fanout_uses_combined_live_payload_quota_atomically() {
    let (doc, plan) = quota_graph();
    let mut baseline = run(&doc, plan.clone());
    pump_until_trace(&mut baseline, 2, ControlOutcome::Suspended);
    pump_until_trace(&mut baseline, 11, ControlOutcome::Suspended);
    baseline.advance_to(1).unwrap();
    pump_until_trace(&mut baseline, 12, ControlOutcome::Then);
    for node in [3, 21] {
        pump_until_trace(&mut baseline, node, ControlOutcome::Suspended);
    }
    assert_eq!(baseline.event_stats().accepted, 2);
    assert_eq!(baseline.event_stats().delivered, 2);
    let exact = baseline.event_stats().live_payload_bytes;
    assert!(exact > 0);
    let combined = exact.checked_mul(2).expect("two equal payload targets");
    assert_eq!(baseline.event_stats().pending, 0);
    stop_and_assert_released(&mut baseline);

    let limits = orange_runtime::RunLimits {
        max_live_event_payload_bytes: combined,
        ..orange_runtime::RunLimits::default()
    };
    let mut exact_run = run_with_options(
        &doc,
        plan.clone(),
        RunOptions {
            limits,
            ..RunOptions::default()
        },
    );
    pump_until_trace(&mut exact_run, 2, ControlOutcome::Suspended);
    pump_until_trace(&mut exact_run, 11, ControlOutcome::Suspended);
    exact_run.advance_to(1).unwrap();
    pump_until_trace(&mut exact_run, 12, ControlOutcome::Then);
    for node in [3, 21] {
        pump_until_trace(&mut exact_run, node, ControlOutcome::Suspended);
    }
    assert_eq!(exact_run.event_stats().live_payload_bytes, exact);
    stop_and_assert_released(&mut exact_run);

    let limits = orange_runtime::RunLimits {
        max_live_event_payload_bytes: combined - 1,
        ..orange_runtime::RunLimits::default()
    };
    let mut refused = run_with_options(
        &doc,
        plan,
        RunOptions {
            limits,
            ..RunOptions::default()
        },
    );
    pump_until_trace(&mut refused, 2, ControlOutcome::Suspended);
    pump_until_trace(&mut refused, 11, ControlOutcome::Suspended);
    refused.advance_to(1).unwrap();
    pump_until_trace(&mut refused, 12, ControlOutcome::Failed);
    assert_eq!(refused.event_stats().accepted, 0);
    assert_eq!(refused.event_stats().delivered, 0);
    assert_eq!(refused.event_stats().pending, 0);
    assert_eq!(refused.event_stats().live_payload_bytes, 0);
    assert_eq!(refused.ownership().event_waiters, 1);
    assert_ne!(
        refused.state(),
        RunState::Stopped,
        "ordinary Emit refusal must not stop the run"
    );
    stop_and_assert_released(&mut refused);
}

#[test]
fn await_then_return_preserves_causal_depth_into_following_emit() {
    const NEXT_EVENT: &str = "40000000-0000-4000-8000-000000000004";
    let mut call = core(4, "call");
    call["config"] = json!({"function": id(100)});
    let function = json!({
        "id": id(100),
        "name": "wait then return",
        "kind": "async",
        "inputs": [],
        "outputs": [],
        "nodes": [core(101, "function-entry"), await_event(102, EVENT, None), core(103, "return")],
        "edges": [edge(2001, 101, "then", 102, "in"), edge(2002, 102, "then", 103, "in")]
    });
    let (package, registry) = catalog();
    let source = graph(
        &package,
        vec![
            core(1, "on-start"),
            call,
            emit_for(5, NEXT_EVENT, 8),
            core(10, "on-start"),
            wait_node(11, 1),
            emit(12, 1),
            on_event_for(30, NEXT_EVENT, 0, 4, "drop-newest"),
            variable_node(31, "set-variable", 1000),
        ],
        vec![
            edge(1, 1, "then", 4, "in"),
            edge(2, 4, "then", 5, "in"),
            edge(3, 10, "then", 11, "in"),
            edge(4, 11, "then", 12, "in"),
            edge(8, 30, "then", 31, "in"),
            edge(9, 30, "payload", 31, "value"),
        ],
        vec![],
    );
    let mut saved: Json = serde_json::from_str(&to_json(&source).unwrap()).unwrap();
    saved["events"] = json!([
        {"id": EVENT, "name":"pulse", "payload":"core/u64@1", "scope":"run"},
        {"id": NEXT_EVENT, "name":"after-return", "payload":"core/u64@1", "scope":"run"}
    ]);
    saved["functions"] = json!([function]);
    saved["variables"] = json!([variable(1000, "run", None)]);
    let doc = from_json(&saved.to_string()).unwrap();
    let analysis = analyze(&doc, &registry);
    assert!(
        analysis.is_ready_for_lowering(&doc, &registry),
        "{:?}",
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
    let mut run = run(&doc, plan.clone());
    pump_until_trace(&mut run, 102, ControlOutcome::Suspended);
    pump_until_trace(&mut run, 11, ControlOutcome::Suspended);
    run.advance_to(1).unwrap();
    pump_until_trace(&mut run, 5, ControlOutcome::Then);
    let metadata = run.last_event().expect("post-return Emit metadata");
    assert!(
        matches!(metadata.origin, orange_runtime::EventOrigin::Graph { node, .. } if node.to_string() == id(5))
    );
    assert_eq!(
        metadata.cause_depth, 2,
        "Await lineage must survive Return before Emit"
    );
    pump_until_trace(&mut run, 30, ControlOutcome::Then);
    pump_until_trace(&mut run, 31, ControlOutcome::Then);
    assert_eq!(
        run.variable(id(1000).parse().unwrap()),
        Some(&Value::U64(8))
    );

    let mut limited = run_with_options(
        &doc,
        plan,
        RunOptions {
            max_cause_depth: 1,
            ..RunOptions::default()
        },
    );
    pump_until_trace(&mut limited, 102, ControlOutcome::Suspended);
    pump_until_trace(&mut limited, 11, ControlOutcome::Suspended);
    limited.advance_to(1).unwrap();
    pump_until_trace(&mut limited, 5, ControlOutcome::Failed);
    assert!(limited.trace().iter().any(|item| {
        item.node.to_string() == id(5)
            && item
                .fault
                .as_ref()
                .is_some_and(|fault| fault.fault.code == "core/RunawayEventChain")
    }));
    assert_eq!(limited.event_stats().accepted, 1);
    assert_eq!(limited.event_stats().delivered, 1);
    assert_eq!(limited.event_stats().pending, 0);
    assert!(
        !limited.trace().iter().any(|item| {
            item.node.to_string() == id(30) && item.outcome == ControlOutcome::Then
        })
    );
    stop_and_assert_released(&mut limited);
}

#[test]
fn replace_retires_await_owned_function_frame_before_latest_root_and_dispose() {
    let mut handler = on_event(10, 0, 4, "fault");
    handler["activation"]["mode"] = json!("replace");
    let mut call = core(11, "call");
    call["config"] = json!({"function": id(100)});
    let mut set_after_return = variable_node(12, "set-variable", 200);
    set_after_return["inputs"] = json!({"value":{"type":"core/u64@1","value":"99"}});
    let function = json!({
        "id": id(100), "name":"awaiting handler", "kind":"async", "inputs":[], "outputs":[],
        "nodes":[core(101,"function-entry"),await_event(102,EVENT,None),core(103,"return")],
        "edges":[edge(2001,101,"then",102,"in"),edge(2002,102,"then",103,"in")]
    });
    let (package, registry) = catalog();
    let source = graph(
        &package,
        vec![
            core(1, "on-start"),
            emit(2, 1),
            wait_node(3, 1),
            emit(4, 2),
            handler,
            call,
            set_after_return,
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 3, "then", 4, "in"),
            edge(4, 10, "then", 11, "in"),
            edge(5, 11, "then", 12, "in"),
        ],
        vec![],
    );
    let mut saved: Json = serde_json::from_str(&to_json(&source).unwrap()).unwrap();
    saved["events"] = json!([{"id":EVENT,"name":"pulse","payload":"core/u64@1","scope":"run"}]);
    saved["functions"] = json!([function]);
    saved["variables"] = json!([variable(200, "run", Some(0))]);
    let doc = from_json(&saved.to_string()).unwrap();
    let analysis = analyze(&doc, &registry);
    assert!(
        analysis.is_ready_for_lowering(&doc, &registry),
        "{:?}",
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
    pump_until_trace(&mut run, 102, ControlOutcome::Suspended);
    pump_until_trace(&mut run, 3, ControlOutcome::Suspended);
    run.advance_to(1).unwrap();
    for _ in 0..4000 {
        if run.event_stats().delivered >= 2 && run.ownership().event_waiters == 1 {
            break;
        }
        run.pump(1).unwrap();
    }
    assert_eq!(run.event_stats().delivered, 2);
    assert_eq!(
        run.event_stats().accepted,
        3,
        "the second Emit admits the latest root and its matching waiter completion"
    );
    assert_eq!(
        run.event_stats().dropped,
        1,
        "replacement cancellation retires the accepted waiter completion"
    );
    assert_eq!(
        run.ownership().event_waiters,
        1,
        "latest root should own the sole live function waiter"
    );
    assert_eq!(
        run.trace()
            .iter()
            .filter(|item| item.node.to_string() == id(102) && item.outcome == ControlOutcome::Then)
            .count(),
        0,
        "the canceled completion must not resume the old function"
    );
    assert_eq!(run.variable(id(200).parse().unwrap()), Some(&Value::U64(0)));
    stop_and_assert_released(&mut run);
}

fn timer_quota_graph() -> (GraphDocument, Arc<orange_compiler::ControlPlan>) {
    let mut fork = core(2, "fork");
    fork["config"] = json!({"join":"all"});
    fork["variadic"] = json!({"branch":{"next":2,"order":[0,1]}});
    compile(
        vec![
            core(1, "on-start"),
            fork,
            await_event(3, EVENT, Some(5)),
            wait_node(4, 5),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "branch#0", 3, "in"),
            edge(3, 2, "branch#1", 4, "in"),
        ],
        vec![],
    )
}

#[test]
fn await_and_wait_share_the_bounded_timer_table() {
    let (doc, plan) = timer_quota_graph();
    let mut exact = run_with_options(
        &doc,
        plan.clone(),
        RunOptions {
            limits: orange_runtime::RunLimits {
                max_timers: 2,
                ..orange_runtime::RunLimits::default()
            },
            ..RunOptions::default()
        },
    );
    pump_until_trace(&mut exact, 3, ControlOutcome::Suspended);
    pump_until_trace(&mut exact, 4, ControlOutcome::Suspended);
    assert_eq!(exact.ownership().timers, 2);
    assert_eq!(exact.ownership().event_waiters, 1);
    exact.advance_to(5).unwrap();
    pump_until_trace(&mut exact, 3, ControlOutcome::TimedOut);
    pump_until_trace(&mut exact, 4, ControlOutcome::Then);
    stop_and_assert_released(&mut exact);

    let mut over = run_with_options(
        &doc,
        plan,
        RunOptions {
            limits: orange_runtime::RunLimits {
                max_timers: 1,
                ..orange_runtime::RunLimits::default()
            },
            ..RunOptions::default()
        },
    );
    let mut resource_error = None;
    for _ in 0..2000 {
        let failed = [3, 4].into_iter().any(|node| {
            over.trace().iter().any(|item| {
                item.node.to_string() == id(node) && item.outcome == ControlOutcome::Failed
            })
        });
        if failed {
            break;
        }
        match over.pump(1) {
            Ok(_) => {}
            Err(error) => {
                resource_error = Some(error);
                break;
            }
        }
    }
    let failed = [3, 4]
        .into_iter()
        .filter(|node| {
            over.trace().iter().any(|item| {
                item.node.to_string() == id(*node) && item.outcome == ControlOutcome::Failed
            })
        })
        .count();
    assert_eq!(failed, 0);
    assert_eq!(
        resource_error,
        Some(orange_runtime::RunError::ResourceLimit)
    );
    assert_eq!(over.state(), RunState::Stopped);
    let owned = over.ownership();
    assert_eq!(
        (
            owned.activations,
            owned.strands,
            owned.regions,
            owned.timers,
            owned.event_waiters
        ),
        (0, 0, 0, 0, 0)
    );
    assert_eq!(over.event_stats().live_payload_bytes, 0);
    assert_eq!(over.retention().reserved_bytes, 0);
    over.dispose().unwrap();
}

#[test]
fn sequential_emits_do_not_accumulate_causal_depth() {
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            emit(2, 1),
            emit(3, 2),
            on_event(10, 0, 4, "drop-newest"),
            core(11, "yield"),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 10, "then", 11, "in"),
        ],
        vec![],
    );
    let mut run = run_with_options(
        &doc,
        plan,
        RunOptions {
            max_cause_depth: 1,
            ..RunOptions::default()
        },
    );
    pump_until_trace(&mut run, 3, ControlOutcome::Then);
    assert_eq!(run.event_stats().accepted, 2);
    assert_eq!(run.event_stats().overflows, 0);
    assert_eq!(run.last_event().unwrap().cause_depth, 1);
    stop_and_assert_released(&mut run);
}

#[test]
fn fork_sibling_emits_keep_independent_cause_depth_after_await_resumes() {
    const SECOND_EVENT: &str = "40000000-0000-4000-8000-000000000005";
    const NEXT_EVENT: &str = "40000000-0000-4000-8000-000000000006";
    let mut fork = core(11, "fork");
    fork["config"] = json!({"join":"all"});
    fork["variadic"] = json!({"branch":{"next":2,"order":[0,1]}});
    let (package, registry) = catalog();
    let source = graph(
        &package,
        vec![
            core(1, "on-start"),
            emit(2, 1),
            on_event(10, 0, 4, "drop-newest"),
            fork,
            await_event(12, SECOND_EVENT, None),
            emit_for(13, NEXT_EVENT, 1),
            wait_node(14, 5),
            emit_for(15, NEXT_EVENT, 2),
            on_event_for(20, EVENT, 1, 4, "drop-newest"),
            wait_node(21, 1),
            emit_for(22, SECOND_EVENT, 3),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 10, "then", 11, "in"),
            edge(3, 11, "branch#0", 12, "in"),
            edge(4, 12, "then", 13, "in"),
            edge(5, 11, "branch#1", 14, "in"),
            edge(6, 14, "then", 15, "in"),
            edge(7, 20, "then", 21, "in"),
            edge(8, 21, "then", 22, "in"),
        ],
        vec![],
    );
    let mut saved: Json = serde_json::from_str(&to_json(&source).unwrap()).unwrap();
    saved["events"] = json!([
        {"id":EVENT,"name":"pulse","payload":"core/u64@1","scope":"run"},
        {"id":SECOND_EVENT,"name":"second","payload":"core/u64@1","scope":"run"},
        {"id":NEXT_EVENT,"name":"next","payload":"core/u64@1","scope":"run"}
    ]);
    let doc = from_json(&saved.to_string()).unwrap();
    let analysis = analyze(&doc, &registry);
    assert!(
        analysis.is_ready_for_lowering(&doc, &registry),
        "{:?}",
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
    pump_until_trace(&mut run, 12, ControlOutcome::Suspended);
    pump_until_trace(&mut run, 14, ControlOutcome::Suspended);
    pump_until_trace(&mut run, 21, ControlOutcome::Suspended);
    run.advance_to(1).unwrap();
    pump_until_trace(&mut run, 22, ControlOutcome::Then);
    pump_until_trace(&mut run, 13, ControlOutcome::Then);
    assert!(
        matches!(run.last_event().unwrap().origin, orange_runtime::EventOrigin::Graph { node, .. } if node.to_string() == id(13))
    );
    assert_eq!(run.last_event().unwrap().cause_depth, 3);
    run.advance_to(5).unwrap();
    pump_until_trace(&mut run, 15, ControlOutcome::Then);
    assert!(
        matches!(run.last_event().unwrap().origin, orange_runtime::EventOrigin::Graph { node, .. } if node.to_string() == id(15))
    );
    assert_eq!(
        run.last_event().unwrap().cause_depth,
        2,
        "unrelated Fork sibling keeps the parent's depth"
    );
}

#[test]
fn immediate_stop_releases_active_handler_and_await_retention_once() {
    let mut fork = core(11, "fork");
    fork["config"] = json!({"join":"all"});
    fork["variadic"] = json!({"branch":{"next":2,"order":[0,1]}});
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            emit(2, 1),
            wait_node(3, 1),
            emit(4, 2),
            on_event(10, 0, 4, "drop-newest"),
            fork,
            await_event(12, EVENT, None),
            variable_node(13, "set-variable", 200),
            wait_node(14, 100),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 3, "then", 4, "in"),
            edge(4, 10, "then", 11, "in"),
            edge(5, 11, "branch#0", 12, "in"),
            edge(6, 12, "then", 13, "in"),
            edge(7, 12, "payload", 13, "value"),
            edge(8, 11, "branch#1", 14, "in"),
        ],
        vec![variable(200, "run", None)],
    );
    let mut run = run(&doc, plan);
    pump_until_trace(&mut run, 12, ControlOutcome::Suspended);
    pump_until_trace(&mut run, 14, ControlOutcome::Suspended);
    pump_until_trace(&mut run, 3, ControlOutcome::Suspended);
    let active_handler_charge = run.event_stats().live_payload_bytes;
    assert!(active_handler_charge > 0);
    assert_eq!(run.event_stats().delivered, 1);
    run.advance_to(1).unwrap();
    pump_until_trace(&mut run, 4, ControlOutcome::Then);
    assert_eq!(
        run.event_stats().accepted,
        3,
        "second Emit admitted handler and waiter targets"
    );
    assert_eq!(
        run.event_stats().delivered,
        1,
        "budget exhaustion leaves waiter publication for a later actor item"
    );
    assert_eq!(
        run.event_stats().pending,
        2,
        "queued handler envelope and matched waiter completion remain pending"
    );
    assert_eq!(
        run.ownership().event_waiters,
        0,
        "the match is no longer waiting for a future event"
    );
    assert!(
        run.event_stats().live_payload_bytes > active_handler_charge,
        "the old active handler reservation remains while new fanout is staged"
    );
    assert_eq!(run.variable(id(200).parse().unwrap()), None);
    stop_and_assert_released(&mut run);
    assert_eq!(run.variable(id(200).parse().unwrap()), None);
}

#[test]
fn parked_indefinite_waiter_keeps_blocked_queue_idle_instead_of_resource_failure() {
    let (package, registry) = catalog();
    let source = graph(
        &package,
        vec![
            core(1, "on-start"),
            emit(2, 1),
            emit(3, 2),
            emit(4, 3),
            on_event(10, 0, 4, "drop-newest"),
            await_event(11, OTHER_EVENT, None),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 3, "then", 4, "in"),
            edge(4, 10, "then", 11, "in"),
        ],
        vec![],
    );
    let mut saved: Json = serde_json::from_str(&to_json(&source).unwrap()).unwrap();
    saved["events"] = json!([
        {"id":EVENT,"name":"pulse","payload":"core/u64@1","scope":"run"},
        {"id":OTHER_EVENT,"name":"other","payload":"core/u64@1","scope":"run"}
    ]);
    let doc = from_json(&saved.to_string()).unwrap();
    let analysis = analyze(&doc, &registry);
    assert!(
        analysis.is_ready_for_lowering(&doc, &registry),
        "{:?}",
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
    pump_until_trace(&mut run, 11, ControlOutcome::Suspended);
    pump_until_trace(&mut run, 4, ControlOutcome::Then);
    let progress = run.pump(64).unwrap();
    assert_eq!(run.state(), RunState::Running);
    assert!(
        !progress.ready,
        "parked Await is an intentional logical wait"
    );
    assert_eq!(progress.next_wakeup, None);
    assert_eq!(run.event_stats().accepted, 3);
    assert_eq!(run.event_stats().delivered, 1);
    assert_eq!(run.event_stats().pending, 2);
    assert_eq!(run.ownership().event_waiters, 1);
    assert_eq!(run.ownership().timers, 0);
    stop_and_assert_released(&mut run);
}
