//! Bounded host input acceptance against real checked event graphs (C04a).
use super::*;
use orange_model::id::EventDeclId;
use orange_runtime::{EventOrigin, IngressError};

const OTHER_EVENT: &str = "40000000-0000-4000-8000-000000000003";

fn event_id() -> EventDeclId {
    EVENT.parse().unwrap()
}

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

fn ingress_graph() -> (GraphDocument, Arc<orange_compiler::ControlPlan>) {
    compile_with_event(
        vec![
            core(1, "on-start"),
            await_event(2, EVENT, None),
            variable_node(3, "set-variable", 100),
            on_event(10, 0, 4, "drop-newest"),
            variable_node(11, "set-variable", 101),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 2, "payload", 3, "value"),
            edge(4, 10, "then", 11, "in"),
            edge(5, 10, "payload", 11, "value"),
        ],
        vec![variable(100, "run", None), variable(101, "run", None)],
        json!({"id": EVENT, "name": "pulse", "payload": "core/u64@1", "scope": "run"}),
    )
    .unwrap()
}

fn timed_ingress_graph() -> (GraphDocument, Arc<orange_compiler::ControlPlan>) {
    let mut timed_out = variable_node(4, "set-variable", 101);
    timed_out["inputs"] = json!({"value": {"type": "core/u64@1", "value": "99"}});
    compile_with_event(
        vec![
            core(1, "on-start"),
            await_event(2, EVENT, Some(5)),
            variable_node(3, "set-variable", 100),
            timed_out,
            on_event(10, 0, 4, "drop-newest"),
            variable_node(11, "set-variable", 102),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 2, "payload", 3, "value"),
            edge(4, 2, "timed_out", 4, "in"),
            edge(5, 10, "then", 11, "in"),
            edge(6, 10, "payload", 11, "value"),
        ],
        vec![
            variable(100, "run", None),
            variable(101, "run", None),
            variable(102, "run", None),
        ],
        json!({"id": EVENT, "name": "pulse", "payload": "core/u64@1", "scope": "run"}),
    )
    .unwrap()
}

fn park_await(run: &mut Run) {
    for _ in 0..2000 {
        if run
            .trace()
            .iter()
            .any(|item| item.node.to_string() == id(2) && item.outcome == ControlOutcome::Suspended)
        {
            return;
        }
        run.pump(1).unwrap();
    }
    panic!("Await did not park: {:?}", run.trace());
}

fn pump_until_delivered(run: &mut Run) {
    for _ in 0..2000 {
        if run.event_stats().delivered == 2
            && run.variable(id(100).parse().unwrap()).is_some()
            && run.variable(id(101).parse().unwrap()).is_some()
        {
            return;
        }
        run.pump(1).unwrap();
    }
    panic!(
        "host delivery did not finish: stats={:?}, trace={:?}, ownership={:?}",
        run.event_stats(),
        run.trace(),
        run.ownership()
    );
}

#[test]
fn typed_ingress_wakes_handler_and_await_on_later_budgeted_actor_turns() {
    let (doc, plan) = ingress_graph();
    let mut run = run(&doc, plan);
    park_await(&mut run);
    let source = run.register_event_source(event_id()).unwrap();

    let metadata = run
        .ingest_event(&source, 1, &Value::U64(42), Some(123_456))
        .unwrap();
    assert_eq!(run.last_event(), Some(metadata));
    assert_eq!(metadata.event, event_id());
    assert_eq!(
        metadata.origin,
        EventOrigin::Host {
            source: source.id()
        }
    );
    assert_eq!(metadata.ingestion_seq, 1);
    assert_eq!(metadata.source_seq, 1);
    assert_eq!(metadata.cause, None);
    assert_eq!(metadata.cause_depth, 0);
    assert_eq!(metadata.monotonic_ns, run.now());
    assert_eq!(metadata.wall_ns, Some(123_456));
    assert_eq!(run.event_stats().accepted, 2, "one handler plus one waiter");
    assert_eq!(run.event_stats().delivered, 0);
    assert_eq!(run.event_stats().pending, 2);
    assert_eq!(run.variable(id(100).parse().unwrap()), None);
    assert_eq!(run.variable(id(101).parse().unwrap()), None);

    pump_until_delivered(&mut run);
    assert_eq!(
        run.variable(id(100).parse().unwrap()),
        Some(&Value::U64(42))
    );
    assert_eq!(
        run.variable(id(101).parse().unwrap()),
        Some(&Value::U64(42))
    );
    assert_eq!(run.ownership().host_sources, 1);
}

#[test]
fn source_generation_type_and_sequence_refusals_are_side_effect_free() {
    let (doc, plan) = ingress_graph();
    let mut first_run = run(&doc, plan.clone());
    let mut other_run = run(&doc, plan.clone());
    let source = first_run.register_event_source(event_id()).unwrap();
    let before = first_run.last_event();

    assert_eq!(
        other_run.ingest_event(&source, 1, &Value::U64(1), None),
        Err(IngressError::WrongRun)
    );
    assert_eq!(
        first_run.ingest_event(&source, 0, &Value::U64(1), None),
        Err(IngressError::SequenceNotIncreasing)
    );
    assert_eq!(
        first_run.ingest_event(&source, 1, &Value::Bool(true), None),
        Err(IngressError::InvalidPayload)
    );
    assert_eq!(first_run.last_event(), before);
    assert_eq!(first_run.event_stats().accepted, 0);
    assert_eq!(first_run.event_stats().overflows, 0);

    let first = first_run
        .ingest_event(&source, 4, &Value::U64(4), None)
        .unwrap();
    assert_eq!(first.ingestion_seq, 1);
    assert_eq!(first.source_seq, 4, "source sequence gaps are allowed");
    for duplicate in [4, 3] {
        assert_eq!(
            first_run.ingest_event(&source, duplicate, &Value::U64(5), None),
            Err(IngressError::SequenceNotIncreasing)
        );
    }
    let next = first_run
        .ingest_event(&source, 9, &Value::U64(9), None)
        .unwrap();
    assert_eq!(next.ingestion_seq, 2);
    assert_eq!(next.source_seq, 9);

    assert!(first_run.revoke_event_source(&source).unwrap());
    assert!(!first_run.revoke_event_source(&source).unwrap());
    assert_eq!(
        first_run.ingest_event(&source, 10, &Value::U64(10), None),
        Err(IngressError::RevokedSource)
    );
    let reopened = first_run.register_event_source(event_id()).unwrap();
    assert_ne!(reopened.id(), source.id());
    assert_eq!(
        first_run
            .ingest_event(&reopened, 1, &Value::U64(1), None)
            .unwrap()
            .source_seq,
        1,
        "a fresh source generation starts its own source sequence"
    );
    assert_eq!(
        first_run
            .ingest_event(&reopened, u64::MAX, &Value::U64(2), None)
            .unwrap()
            .source_seq,
        u64::MAX
    );
    for exhausted in [u64::MAX, 1] {
        assert_eq!(
            first_run.ingest_event(&reopened, exhausted, &Value::U64(3), None),
            Err(IngressError::SequenceNotIncreasing),
            "source sequence must never wrap or be reused"
        );
    }
    assert_eq!(first_run.event_stats().overflows, 0);
}

#[test]
fn source_registration_limit_reopens_after_revoke_and_unknown_event_is_rejected() {
    let (doc, plan) = ingress_graph();
    let mut options = RunOptions::default();
    options.limits.max_host_sources = 1;
    let mut run = run_with_options(&doc, plan, options);
    let source = run.register_event_source(event_id()).unwrap();
    assert_eq!(run.ownership().host_sources, 1);
    assert_eq!(
        run.register_event_source("40000000-0000-4000-8000-000000000099".parse().unwrap()),
        Err(IngressError::UnknownEvent)
    );
    assert_eq!(
        run.register_event_source(event_id()),
        Err(IngressError::SourceLimit)
    );
    assert!(run.revoke_event_source(&source).unwrap());
    assert_eq!(run.ownership().host_sources, 0);
    let replacement = run.register_event_source(event_id()).unwrap();
    assert_ne!(replacement.id(), source.id());
}

#[test]
fn host_source_is_pinned_to_run_identity_and_compiled_payload_type() {
    let (doc_a, plan_a) = ingress_graph();
    let string_entry = on_event(10, 0, 4, "drop-newest");
    let mut string_set = variable_node(11, "set-variable", 100);
    string_set["inputs"] = json!({"value": {"type": "core/string@1", "value": ""}});
    let (doc_b, plan_b) = compile_with_event(
        vec![string_entry, string_set],
        vec![
            edge(1, 10, "then", 11, "in"),
            edge(2, 10, "payload", 11, "value"),
        ],
        vec![string_variable(100)],
        json!({"id": EVENT, "name": "pulse", "payload": "core/string@1", "scope": "run"}),
    )
    .unwrap();
    let mut run_a = run(&doc_a, plan_a);
    let mut run_b = run(&doc_b, plan_b);
    assert_ne!(
        run_a.id(),
        run_b.id(),
        "same seed does not reuse run identity"
    );
    let source_a = run_a.register_event_source(event_id()).unwrap();
    let source_b = run_b.register_event_source(event_id()).unwrap();

    assert_eq!(
        run_b.ingest_event(&source_a, 1, &Value::U64(7), None),
        Err(IngressError::WrongRun)
    );
    assert_eq!(
        run_a.ingest_event(&source_a, 1, &Value::Str(Arc::new("wrong".into())), None),
        Err(IngressError::InvalidPayload)
    );
    let accepted_a = run_a
        .ingest_event(&source_a, 1, &Value::U64(7), None)
        .unwrap();
    let accepted_b = run_b
        .ingest_event(&source_b, 1, &Value::Str(Arc::new("pinned B".into())), None)
        .unwrap();
    assert_eq!(accepted_a.event, event_id());
    assert_eq!(accepted_b.event, event_id());
    assert_eq!(accepted_a.ingestion_seq, 1);
    assert_eq!(accepted_b.ingestion_seq, 1);
    assert_eq!(run_a.event_stats().overflows, 0);
    assert_eq!(run_b.event_stats().overflows, 0);
}

#[test]
fn stopping_and_disposal_reject_ingress_and_release_registered_sources() {
    for mode in [
        orange_runtime::StopMode::Graceful,
        orange_runtime::StopMode::Immediate,
    ] {
        let (doc, plan) = ingress_graph();
        let options = RunOptions {
            drain_timeout_ns: 0,
            ..RunOptions::default()
        };
        let mut run = run_with_options(&doc, plan, options);
        let source = run.register_event_source(event_id()).unwrap();
        run.request_stop(mode).unwrap();
        assert_eq!(
            run.ingest_event(&source, 1, &Value::U64(1), None),
            Err(IngressError::NotRunning)
        );
        for _ in 0..2000 {
            if run.state() == RunState::Stopped {
                break;
            }
            run.pump(8).unwrap();
        }
        assert_eq!(run.state(), RunState::Stopped);
        assert_eq!(run.ownership().host_sources, 0);
        assert_eq!(run.ownership().activations, 0);
        assert_eq!(run.ownership().event_waiters, 0);
        assert_eq!(run.event_stats().pending, 0);
        assert_eq!(run.event_stats().live_payload_bytes, 0);
        assert!(!run.revoke_event_source(&source).unwrap());
        assert_eq!(
            run.register_event_source(event_id()),
            Err(IngressError::NotRunning)
        );
        run.dispose().unwrap();
        assert_eq!(run.ownership().host_sources, 0);
    }
}

#[test]
fn combined_handler_and_waiter_quota_refusal_does_not_partially_wake() {
    let (doc, plan) = timed_ingress_graph();
    let mut options = RunOptions::default();
    options.limits.max_pending_events = 1;
    let mut run = run_with_options(&doc, plan, options);
    park_await(&mut run);
    let source = run.register_event_source(event_id()).unwrap();
    let before = run.last_event();

    assert_eq!(
        run.ingest_event(&source, 1, &Value::U64(42), None),
        Err(IngressError::DeliveryLimit)
    );
    assert_eq!(run.event_stats().accepted, 0);
    assert_eq!(run.event_stats().delivered, 0);
    assert_eq!(run.event_stats().pending, 0);
    assert_eq!(run.event_stats().overflows, 1);
    assert_eq!(run.ownership().event_waiters, 1);
    assert_eq!(run.last_event(), before);
    assert_eq!(run.variable(id(100).parse().unwrap()), None);
    assert_eq!(run.variable(id(101).parse().unwrap()), None);
    assert_eq!(
        run.ingest_event(&source, 1, &Value::U64(42), None),
        Err(IngressError::DeliveryLimit),
        "quota refusal does not consume source sequence"
    );
    assert_eq!(run.event_stats().overflows, 2);
    run.advance_to(5).unwrap();
    for _ in 0..1000 {
        if run
            .trace()
            .iter()
            .any(|item| item.node.to_string() == id(2) && item.outcome == ControlOutcome::TimedOut)
        {
            break;
        }
        run.pump(1).unwrap();
    }
    assert_eq!(run.ownership().event_waiters, 0);
    let retry = run.ingest_event(&source, 1, &Value::U64(43), None).unwrap();
    assert_eq!(
        retry.ingestion_seq, 1,
        "refused ingress did not advance run sequence"
    );
    assert_eq!(
        retry.source_seq, 1,
        "refused ingress did not advance source sequence"
    );
    assert_eq!(run.event_stats().accepted, 1);
    assert_eq!(run.event_stats().overflows, 2);
    for _ in 0..1000 {
        if run.variable(id(102).parse().unwrap()) == Some(&Value::U64(43)) {
            break;
        }
        run.pump(1).unwrap();
    }
    assert_eq!(
        run.variable(id(102).parse().unwrap()),
        Some(&Value::U64(43))
    );
}

#[test]
fn host_single_handler_item_and_byte_quotas_accept_exact_boundary_and_refuse_one_under() {
    let mut set = variable_node(11, "set-variable", 100);
    set["inputs"] = json!({"value": {"type": "core/u64@1", "value": "0"}});
    let (doc, plan) = compile(
        vec![on_event(10, 0, 4, "drop-newest"), set],
        vec![
            edge(1, 10, "then", 11, "in"),
            edge(2, 10, "payload", 11, "value"),
        ],
        vec![variable(100, "run", None)],
    );
    let mut baseline = run(&doc, plan.clone());
    let baseline_source = baseline.register_event_source(event_id()).unwrap();
    baseline
        .ingest_event(&baseline_source, 1, &Value::U64(8), None)
        .unwrap();
    let expected = baseline.event_stats();
    assert_eq!(expected.pending, 1);

    for (dimension, exact) in [
        (0, expected.pending),
        (1, expected.pending_bytes),
        (2, expected.live_payload_bytes),
    ] {
        for one_under in [false, true] {
            let mut options = RunOptions::default();
            let limit = exact - usize::from(one_under);
            match dimension {
                0 => options.limits.max_pending_events = limit,
                1 => options.limits.max_pending_event_bytes = limit,
                _ => options.limits.max_live_event_payload_bytes = limit,
            }
            let mut run = run_with_options(&doc, plan.clone(), options);
            let source = run.register_event_source(event_id()).unwrap();
            let result = run.ingest_event(&source, 1, &Value::U64(8), None);
            if one_under {
                assert_eq!(result, Err(IngressError::DeliveryLimit));
                assert_eq!(run.event_stats().accepted, 0);
                assert_eq!(run.event_stats().pending, 0);
                assert_eq!(run.event_stats().overflows, 1);
                assert_eq!(run.last_event(), None);
            } else {
                assert!(result.is_ok(), "dimension={dimension}");
                assert_eq!(run.event_stats().accepted, 1);
                assert_eq!(run.event_stats().pending, 1);
                assert_eq!(run.event_stats().pending_bytes, expected.pending_bytes);
                assert_eq!(
                    run.event_stats().live_payload_bytes,
                    expected.live_payload_bytes
                );
                assert_eq!(run.event_stats().overflows, 0);
            }
        }
    }
}

#[test]
fn host_replace_is_deferred_until_pump_and_revocation_keeps_accepted_delivery() {
    let replace = super::event_replace::replacement(10, 4, "fault");
    let mut after_old_timer = variable_node(4, "set-variable", 200);
    after_old_timer["inputs"] = json!({"value": {"type": "core/u64@1", "value": "1"}});
    let mut after_cancel = variable_node(5, "set-variable", 201);
    after_cancel["inputs"] = json!({"value": {"type": "core/u64@1", "value": "1"}});
    let mut latest_done = variable_node(12, "set-variable", 202);
    latest_done["inputs"] = json!({"value": {"type": "core/u64@1", "value": "1"}});
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            emit(2, 1),
            wait_node(11, 100),
            after_old_timer,
            after_cancel,
            replace,
            latest_done,
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 10, "then", 11, "in"),
            edge(3, 11, "then", 4, "in"),
            edge(4, 11, "cancelled", 5, "in"),
            edge(5, 4, "then", 12, "in"),
        ],
        vec![
            variable(200, "run", None),
            variable(201, "run", None),
            variable(202, "run", None),
        ],
    );
    let mut run = run(&doc, plan);
    for _ in 0..1000 {
        if run.trace().iter().any(|item| {
            item.node.to_string() == id(11) && item.outcome == ControlOutcome::Suspended
        }) {
            break;
        }
        run.pump(1).unwrap();
    }
    assert!(
        run.trace().iter().any(|item| {
            item.node.to_string() == id(11) && item.outcome == ControlOutcome::Suspended
        }),
        "trace={:?}, state={:?}, stats={:?}",
        run.trace(),
        run.state(),
        run.event_stats()
    );
    run.advance_to(100).unwrap();
    let source = run.register_event_source(event_id()).unwrap();
    run.ingest_event(&source, 1, &Value::U64(2), None).unwrap();
    assert!(!run.trace().iter().any(|item| {
        item.node.to_string() == id(11) && item.outcome == ControlOutcome::Cancelled
    }));
    assert!(run.pump(0).unwrap().ready);
    assert!(!run.trace().iter().any(|item| {
        item.node.to_string() == id(11) && item.outcome == ControlOutcome::Cancelled
    }));
    assert_eq!(run.variable(id(200).parse().unwrap()), None);

    // Revocation closes future host input but does not retract the accepted replacement.
    assert!(run.revoke_event_source(&source).unwrap());
    assert_eq!(
        run.ingest_event(&source, 2, &Value::U64(3), None),
        Err(IngressError::RevokedSource)
    );
    for _ in 0..1000 {
        if run.trace().iter().any(|item| {
            item.node.to_string() == id(11) && item.outcome == ControlOutcome::Cancelled
        }) {
            break;
        }
        run.pump(1).unwrap();
    }
    assert!(run.trace().iter().any(|item| {
        item.node.to_string() == id(11) && item.outcome == ControlOutcome::Cancelled
    }));
    for _ in 0..1000 {
        if run.variable(id(201).parse().unwrap()) == Some(&Value::U64(1)) {
            break;
        }
        run.pump(1).unwrap();
    }
    assert_eq!(
        run.variable(id(200).parse().unwrap()),
        None,
        "due timer lost to host Replace"
    );
    assert_eq!(run.variable(id(201).parse().unwrap()), Some(&Value::U64(1)));
    for _ in 0..1000 {
        if run.event_stats().delivered == 2
            && run
                .trace()
                .iter()
                .filter(|item| {
                    item.node.to_string() == id(11) && item.outcome == ControlOutcome::Suspended
                })
                .count()
                == 2
        {
            break;
        }
        run.pump(1).unwrap();
    }
    assert_eq!(run.event_stats().delivered, 2);
    assert_eq!(run.variable(id(202).parse().unwrap()), None);
    run.advance_to(200).unwrap();
    pump_until_idle(&mut run);
    assert_eq!(run.variable(id(202).parse().unwrap()), Some(&Value::U64(1)));
}

#[test]
fn later_fault_target_cannot_commit_staged_host_replace_cancellation() {
    let mut replace = super::event_replace::replacement(10, 4, "fault");
    replace["activation"]["order"] = json!(0);
    let queue = on_event(20, 1, 1, "fault");
    let (doc, plan) = compile_with_event(
        vec![
            core(1, "on-start"),
            await_event(2, OTHER_EVENT, None),
            replace,
            wait_node(11, 100),
            variable_node(12, "set-variable", 100),
            queue,
            wait_node(21, 100),
            variable_node(22, "set-variable", 101),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 10, "then", 11, "in"),
            edge(3, 10, "payload", 12, "value"),
            edge(4, 11, "then", 12, "in"),
            edge(5, 20, "then", 21, "in"),
            edge(6, 20, "payload", 22, "value"),
            edge(7, 21, "then", 22, "in"),
        ],
        vec![variable(100, "run", None), variable(101, "run", None)],
        json!([
            {"id":EVENT,"name":"pulse","payload":"core/u64@1","scope":"run"},
            {"id":OTHER_EVENT,"name":"other","payload":"core/u64@1","scope":"run"}
        ]),
    )
    .unwrap();
    let mut options = RunOptions::default();
    options.limits.max_activations = 2;
    let mut run = run_with_options(&doc, plan, options);
    park_await(&mut run);
    let source = run.register_event_source(event_id()).unwrap();
    let first = run.ingest_event(&source, 1, &Value::U64(1), None).unwrap();
    for _ in 0..2000 {
        let old_waiting = run.trace().iter().any(|item| {
            item.node.to_string() == id(11) && item.outcome == ControlOutcome::Suspended
        });
        if old_waiting && run.event_stats().pending == 1 {
            break;
        }
        run.pump(1).unwrap();
    }
    assert!(run.trace().iter().any(|item| {
        item.node.to_string() == id(11) && item.outcome == ControlOutcome::Suspended
    }));
    assert_eq!(
        run.event_stats().pending,
        1,
        "Queue handler is capacity blocked"
    );
    assert_eq!(
        run.event_stats().delivered,
        1,
        "only Replace root has started"
    );
    assert_eq!(
        run.ownership().event_waiters,
        1,
        "unrelated Await remains parked"
    );
    assert_eq!(run.ownership().activations, 2);
    let stats_before = run.event_stats();

    assert_eq!(
        run.ingest_event(&source, 2, &Value::U64(2), None),
        Err(IngressError::DeliveryLimit),
        "the later full Fault handler refuses the complete fan-out"
    );
    let mut expected_stats = stats_before;
    expected_stats.overflows += 1;
    assert_eq!(run.event_stats(), expected_stats);
    assert_eq!(run.last_event(), Some(first));
    assert_eq!(run.ownership().event_waiters, 1);
    assert!(!run.trace().iter().any(|item| {
        item.node.to_string() == id(11) && item.outcome == ControlOutcome::Cancelled
    }));

    run.advance_to(100).unwrap();
    for _ in 0..2000 {
        if run.variable(id(100).parse().unwrap()) == Some(&Value::U64(1)) {
            break;
        }
        run.pump(1).unwrap();
    }
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(1)));
    assert!(
        run.trace().iter().any(|item| {
            item.node.to_string() == id(11) && item.outcome == ControlOutcome::Then
        })
    );

    run.request_stop(orange_runtime::StopMode::Immediate)
        .unwrap();
    for _ in 0..2000 {
        if run.state() == RunState::Stopped {
            break;
        }
        run.pump(16).unwrap();
    }
    assert_eq!(run.state(), RunState::Stopped);
    let owners = run.ownership();
    assert_eq!(
        (
            owners.activations,
            owners.strands,
            owners.regions,
            owners.timers,
            owners.event_waiters,
            owners.host_sources,
        ),
        (0, 0, 0, 0, 0, 0)
    );
    assert_eq!(run.event_stats().pending, 0);
    assert_eq!(run.event_stats().live_payload_bytes, 0);
    run.dispose().unwrap();
}

#[test]
fn explicit_host_call_order_resolves_equal_deadline_await_race() {
    let (doc, plan) = timed_ingress_graph();
    let mut event_first = run(&doc, plan.clone());
    park_await(&mut event_first);
    event_first.advance_to(5).unwrap();
    let source = event_first.register_event_source(event_id()).unwrap();
    event_first
        .ingest_event(&source, 1, &Value::U64(5), None)
        .unwrap();
    for _ in 0..2000 {
        if event_first.event_stats().delivered == 2
            && event_first.variable(id(100).parse().unwrap()).is_some()
            && event_first.variable(id(102).parse().unwrap()).is_some()
        {
            break;
        }
        event_first.pump(1).unwrap();
    }
    assert_eq!(
        event_first.variable(id(100).parse().unwrap()),
        Some(&Value::U64(5))
    );
    assert_eq!(event_first.variable(id(101).parse().unwrap()), None);
    assert_eq!(
        event_first.variable(id(102).parse().unwrap()),
        Some(&Value::U64(5))
    );

    let mut timeout_first = run(&doc, plan);
    park_await(&mut timeout_first);
    timeout_first.advance_to(5).unwrap();
    for _ in 0..100 {
        if timeout_first
            .trace()
            .iter()
            .any(|item| item.node.to_string() == id(2) && item.outcome == ControlOutcome::TimedOut)
        {
            break;
        }
        timeout_first.pump(1).unwrap();
    }
    assert!(timeout_first.trace().iter().any(|item| {
        item.node.to_string() == id(2) && item.outcome == ControlOutcome::TimedOut
    }));
    let source = timeout_first.register_event_source(event_id()).unwrap();
    timeout_first
        .ingest_event(&source, 1, &Value::U64(7), None)
        .unwrap();
    pump_until_idle(&mut timeout_first);
    assert_eq!(timeout_first.variable(id(100).parse().unwrap()), None);
    assert_eq!(
        timeout_first.variable(id(101).parse().unwrap()),
        Some(&Value::U64(99))
    );
    assert_eq!(
        timeout_first.variable(id(102).parse().unwrap()),
        Some(&Value::U64(7))
    );
}
