//! Drop and bounded Parallel at the real graph/compiler/Run boundary.
use super::*;
use orange_model::document::ReentrancyMode;
use orange_runtime::{RunError, StopMode};

fn handler(n: u32, mode: &str, capacity: u32, overflow: &str) -> Json {
    let mut node = on_event(n, 0, capacity, overflow);
    node["activation"]["mode"] = json!(mode);
    if mode == "parallel" {
        node["activation"]["max_parallel"] = json!(2);
    }
    node
}

/// Pending work may wait behind a live handler's timer. No eligible work remains.
fn quiesce(run: &mut Run) {
    for _ in 0..20_000 {
        if run.state() == RunState::Stopped || !run.pump(1).unwrap().ready {
            return;
        }
    }
    panic!("event fixture stayed ready: {:?}", run.event_stats());
}

fn release(mut run: Run) {
    run.request_stop(StopMode::Immediate).unwrap();
    quiesce(&mut run);
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(run.ownership().activations, 0);
    assert_eq!(run.ownership().strands, 0);
    assert_eq!(run.ownership().regions, 0);
    assert_eq!(run.ownership().timers, 0);
    assert_eq!(run.event_stats().pending, 0);
    assert_eq!(run.event_stats().live_payload_bytes, 0);
    assert_eq!(run.retention().reserved_bytes, 0);
    run.dispose().unwrap();
    assert_eq!(run.state(), RunState::Disposed);
}

#[test]
fn drop_is_busy_as_soon_as_a_delivery_is_pending_without_retaining_the_next() {
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            emit(2, 1),
            emit(3, 2),
            wait_node(4, 5),
            handler(10, "drop", 1, "fault"),
            variable_node(11, "set-variable", 100),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 3, "then", 4, "in"),
            edge(4, 10, "then", 11, "in"),
            edge(5, 10, "payload", 11, "value"),
        ],
        vec![variable(100, "run", None)],
    );
    assert_eq!(plan.events()[0].handlers()[0].mode(), ReentrancyMode::Drop);
    assert_eq!(plan.events()[0].handlers()[0].max_parallel(), 1);
    let mut options = RunOptions::default();
    // The emitter owns the only slot until its Wait returns: no handler can dispatch.
    options.limits.max_activations = 1;
    let mut run = run_with_options(&doc, plan, options);
    quiesce(&mut run);
    let stats = run.event_stats();
    assert_eq!((stats.accepted, stats.delivered, stats.pending), (1, 0, 1));
    assert_eq!((stats.dropped, stats.overflows), (1, 0));
    assert_eq!(stats.live_payload_bytes, stats.pending_bytes);
    assert_eq!(run.last_event().unwrap().ingestion_seq, 2);
    run.advance_to(5).unwrap();
    quiesce(&mut run);
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(1)));
    assert_eq!(run.event_stats().live_payload_bytes, 0);
    release(run);
}

#[test]
fn drop_skips_a_suspended_handler_then_accepts_after_full_root_retirement() {
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            emit(2, 1),
            wait_node(3, 5),
            emit(4, 2),
            wait_node(5, 20),
            emit(6, 3),
            handler(10, "drop", 1, "fault"),
            wait_node(11, 10),
            variable_node(12, "set-variable", 100),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 3, "then", 4, "in"),
            edge(4, 4, "then", 5, "in"),
            edge(5, 5, "then", 6, "in"),
            edge(6, 10, "then", 11, "in"),
            edge(7, 11, "then", 12, "in"),
            edge(8, 10, "payload", 12, "value"),
        ],
        vec![variable(100, "run", None)],
    );
    let mut run = run(&doc, plan);
    quiesce(&mut run);
    let charge = run.event_stats().live_payload_bytes;
    assert!(charge > 0);
    run.advance_to(5).unwrap();
    quiesce(&mut run);
    assert_eq!(
        (run.event_stats().accepted, run.event_stats().delivered),
        (1, 1)
    );
    assert_eq!(
        (run.event_stats().dropped, run.event_stats().overflows),
        (1, 0)
    );
    assert_eq!(run.event_stats().live_payload_bytes, charge);
    assert_eq!(run.event_stats().pending, 0);
    run.advance_to(10).unwrap();
    quiesce(&mut run);
    assert_eq!(run.event_stats().live_payload_bytes, 0);
    run.advance_to(25).unwrap();
    quiesce(&mut run);
    assert_eq!(
        (run.event_stats().accepted, run.event_stats().delivered),
        (2, 2)
    );
    run.advance_to(35).unwrap();
    quiesce(&mut run);
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(3)));
    assert_eq!(run.event_stats().live_payload_bytes, 0);
    release(run);
}

#[test]
fn busy_drop_still_checks_causal_depth_before_deciding_to_skip() {
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            emit(2, 1),
            handler(10, "drop", 1, "fault"),
            emit(11, 2),
        ],
        vec![edge(1, 1, "then", 2, "in"), edge(2, 10, "then", 11, "in")],
        vec![],
    );
    let options = RunOptions {
        max_cause_depth: 1,
        ..RunOptions::default()
    };
    let mut run = run_with_options(&doc, plan, options);
    quiesce(&mut run);
    assert_eq!(
        (run.event_stats().accepted, run.event_stats().delivered),
        (1, 1)
    );
    assert_eq!(
        (run.event_stats().dropped, run.event_stats().overflows),
        (0, 0)
    );
    assert!(run.trace().iter().any(|event| {
        event.node.to_string() == id(11)
            && event.outcome == ControlOutcome::Failed
            && event
                .fault
                .as_ref()
                .is_some_and(|fault| fault.fault.code == "core/RunawayEventChain")
    }));
    assert_eq!(run.last_event().unwrap().ingestion_seq, 1);
    release(run);
}

/// Each activation captures its payload in a fresh local before suspension and adds it
/// afterwards. A shared local/cache would corrupt the final sum.
fn parallel_sum(
    values: &[u64],
    overflow: &str,
) -> (GraphDocument, Arc<orange_compiler::ControlPlan>) {
    parallel_sum_with_capacity(values, overflow, 1)
}

fn parallel_sum_with_capacity(
    values: &[u64],
    overflow: &str,
    capacity: u32,
) -> (GraphDocument, Arc<orange_compiler::ControlPlan>) {
    let mut nodes = vec![core(1, "on-start")];
    let mut edges = Vec::new();
    let mut predecessor = 1;
    for (i, value) in values.iter().enumerate() {
        let n = 2 + i as u32;
        nodes.push(emit(n, *value));
        edges.push(edge(edges.len() as u32 + 1, predecessor, "then", n, "in"));
        predecessor = n;
    }
    let mut add = update_local(14, 100);
    add["inputs"] = json!({});
    nodes.extend([
        handler(10, "parallel", capacity, overflow),
        variable_node(11, "set-variable", 101),
        wait_node(12, 10),
        variable_node(13, "get-variable", 101),
        add,
    ]);
    for (from, output, to, input) in [
        (10, "then", 11, "in"),
        (10, "payload", 11, "value"),
        (11, "then", 12, "in"),
        (12, "then", 14, "in"),
        (13, "value", 14, "value"),
    ] {
        edges.push(edge(edges.len() as u32 + 1, from, output, to, input));
    }
    compile(
        nodes,
        edges,
        vec![
            variable(100, "run", Some(0)),
            variable(101, "local", Some(0)),
        ],
    )
}

#[test]
fn parallel_caps_live_roots_preserves_locals_and_applies_pending_only_overflow() {
    for (overflow, total, accepted, dropped) in [
        ("drop-newest", 6, 3, 1),
        ("drop-oldest", 7, 4, 1),
        ("coalesce-latest", 7, 4, 1),
        ("fault", 6, 3, 0),
    ] {
        let (doc, plan) = parallel_sum(&[1, 2, 3, 4], overflow);
        assert_eq!(
            plan.events()[0].handlers()[0].mode(),
            ReentrancyMode::Parallel
        );
        assert_eq!(plan.events()[0].handlers()[0].max_parallel(), 2);
        let mut run = run(&doc, plan);
        quiesce(&mut run);
        let stats = run.event_stats();
        assert_eq!(
            (stats.accepted, stats.delivered, stats.pending),
            (accepted, 2, 1),
            "{overflow}"
        );
        assert_eq!((stats.dropped, stats.overflows), (dropped, 1), "{overflow}");
        assert_eq!(run.ownership().timers, 2);
        assert!(stats.live_payload_bytes > stats.pending_bytes);
        run.advance_to(10).unwrap();
        quiesce(&mut run);
        assert_eq!(
            run.variable(id(100).parse().unwrap()),
            Some(&Value::U64(3)),
            "{overflow}"
        );
        assert_eq!(run.event_stats().delivered, 3);
        assert_eq!(run.event_stats().pending, 0);
        run.advance_to(20).unwrap();
        quiesce(&mut run);
        assert_eq!(
            run.variable(id(100).parse().unwrap()),
            Some(&Value::U64(total)),
            "{overflow}"
        );
        assert_eq!(run.event_stats().live_payload_bytes, 0);
        release(run);
    }
}

#[test]
fn mixed_drop_parallel_fanout_is_atomic_at_the_exact_live_payload_budget() {
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            emit(2, 1),
            wait_node(3, 5),
            emit(4, 2),
            handler(10, "drop", 1, "drop-newest"),
            wait_node(11, 100),
            handler(20, "parallel", 1, "drop-newest"),
            wait_node(21, 100),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 3, "then", 4, "in"),
            edge(4, 10, "then", 11, "in"),
            edge(5, 20, "then", 21, "in"),
        ],
        vec![],
    );
    let mut baseline = run(&doc, plan.clone());
    quiesce(&mut baseline);
    let two = baseline.event_stats().live_payload_bytes;
    assert!(two > 0 && two.is_multiple_of(2));
    release(baseline);
    let exact = two + two / 2;
    for (limit, fits) in [(exact, true), (exact - 1, false)] {
        let mut options = RunOptions::default();
        options.limits.max_live_event_payload_bytes = limit;
        let mut run = run_with_options(&doc, plan.clone(), options);
        quiesce(&mut run);
        assert_eq!(
            (run.event_stats().accepted, run.event_stats().delivered),
            (2, 2)
        );
        run.advance_to(5).unwrap();
        quiesce(&mut run);
        let stats = run.event_stats();
        if fits {
            assert_eq!(
                (
                    stats.accepted,
                    stats.delivered,
                    stats.dropped,
                    stats.overflows
                ),
                (3, 3, 1, 0)
            );
            assert_eq!(run.last_event().unwrap().ingestion_seq, 2);
            assert_eq!(stats.live_payload_bytes, exact);
        } else {
            assert_eq!(
                (
                    stats.accepted,
                    stats.delivered,
                    stats.dropped,
                    stats.overflows
                ),
                (2, 2, 0, 1)
            );
            assert_eq!(run.last_event().unwrap().ingestion_seq, 1);
            assert_eq!(stats.live_payload_bytes, two);
            assert!(
                run.trace()
                    .iter()
                    .any(|event| event.node.to_string() == id(4)
                        && event.outcome == ControlOutcome::Failed)
            );
        }
        assert_eq!(stats.pending, 0);
        run.advance_to(105).unwrap();
        quiesce(&mut run);
        assert_eq!(run.event_stats().live_payload_bytes, 0);
        release(run);
    }
}

#[test]
fn a_later_fault_handler_does_not_commit_an_earlier_busy_drop() {
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            emit(2, 1),
            wait_node(3, 5),
            emit(4, 2),
            emit(5, 3),
            handler(10, "drop", 1, "fault"),
            wait_node(11, 100),
            on_event(20, 1, 1, "fault"),
            wait_node(21, 100),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 3, "then", 4, "in"),
            edge(4, 4, "then", 5, "in"),
            edge(5, 10, "then", 11, "in"),
            edge(6, 20, "then", 21, "in"),
        ],
        vec![],
    );
    let mut run = run(&doc, plan);
    quiesce(&mut run);
    assert_eq!(run.event_stats().delivered, 2);
    run.advance_to(5).unwrap();
    quiesce(&mut run);
    let stats = run.event_stats();
    assert_eq!((stats.accepted, stats.delivered, stats.pending), (3, 2, 1));
    assert_eq!((stats.dropped, stats.overflows), (1, 1));
    assert_eq!(run.last_event().unwrap().ingestion_seq, 2);
    assert!(run.trace().iter().any(|event| {
        event.node.to_string() == id(5) && event.outcome == ControlOutcome::Failed
    }));
    release(run);
}

#[test]
fn parallel_dispatch_respects_the_run_activation_limit_without_spinning() {
    let (doc, plan) = parallel_sum_with_capacity(&[1, 2, 3], "drop-newest", 3);
    for limit in [1, 2] {
        let mut options = RunOptions::default();
        options.limits.max_activations = limit;
        let mut run = run_with_options(&doc, plan.clone(), options);
        quiesce(&mut run);
        assert_eq!(run.event_stats().delivered, limit as u64);
        assert_eq!(run.event_stats().pending, 3 - limit);
        assert!(run.ownership().activations <= limit);
        for at in [10, 20, 30] {
            run.advance_to(at).unwrap();
            quiesce(&mut run);
            assert!(run.ownership().activations <= limit);
        }
        assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(6)));
        assert_eq!(run.event_stats().pending, 0);
        release(run);
    }
}

#[test]
fn parallel_frames_obey_the_aggregate_retained_value_boundary() {
    let (doc, plan) = parallel_sum(&[1, 2], "drop-newest");
    let refused = crate::budget_drive::boundary(|limit| {
        let mut options = RunOptions::default();
        options.limits.max_run_retained_bytes = limit;
        Run::start(plan.clone(), &doc, plan.data().snapshot(), 17, options)
    });
    assert!(matches!(
        refused,
        RunError::AggregateLimit | RunError::Data(orange_runtime::DataError::AggregateLimit)
    ));
}

#[test]
fn parallel_reuses_a_root_slot_only_after_race_loser_cleanup_finishes() {
    let mut entry = handler(10, "parallel", 1, "drop-newest");
    entry["activation"]["max_parallel"] = json!(1);
    let mut fork = core(11, "fork");
    fork["config"] = json!({"join":"race"});
    fork["variadic"] = json!({"branch":{"next":2,"order":[0,1]}});
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            emit(2, 1),
            emit(3, 2),
            entry,
            fork,
            wait_node(12, 5),
            wait_node(13, 100),
            variable_node(14, "set-variable", 100),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 10, "then", 11, "in"),
            edge(4, 11, "branch#0", 12, "in"),
            edge(5, 11, "branch#1", 13, "in"),
            edge(6, 13, "cancelled", 14, "in"),
            edge(7, 10, "payload", 14, "value"),
        ],
        vec![variable(100, "run", Some(0))],
    );
    let mut run = run(&doc, plan);
    quiesce(&mut run);
    assert_eq!(
        (run.event_stats().delivered, run.event_stats().pending),
        (1, 1)
    );
    run.advance_to(5).unwrap();
    quiesce(&mut run);
    assert_eq!(
        (run.event_stats().delivered, run.event_stats().pending),
        (2, 0)
    );
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(1)));
    let first_cleanup = run
        .trace()
        .iter()
        .position(|event| event.node.to_string() == id(14) && event.outcome == ControlOutcome::Then)
        .unwrap();
    let second_entry = run
        .trace()
        .iter()
        .enumerate()
        .filter(|(_, event)| {
            event.node.to_string() == id(10) && event.outcome == ControlOutcome::Then
        })
        .nth(1)
        .unwrap()
        .0;
    assert!(
        first_cleanup < second_entry,
        "slot was reused before its complete root retired"
    );
    run.advance_to(10).unwrap();
    quiesce(&mut run);
    assert_eq!(run.variable(id(100).parse().unwrap()), Some(&Value::U64(2)));
    assert_eq!(run.event_stats().live_payload_bytes, 0);
    release(run);
}

#[test]
fn parallel_graceful_and_immediate_stop_release_active_and_queued_deliveries() {
    let (doc, plan) = parallel_sum(&[1, 2, 3], "drop-newest");
    for mode in [StopMode::Immediate, StopMode::Graceful] {
        let mut run = run(&doc, plan.clone());
        quiesce(&mut run);
        assert_eq!(
            (run.event_stats().delivered, run.event_stats().pending),
            (2, 1)
        );
        run.request_stop(mode).unwrap();
        quiesce(&mut run);
        if mode == StopMode::Graceful {
            run.advance_to(10).unwrap();
            quiesce(&mut run);
            run.advance_to(20).unwrap();
            quiesce(&mut run);
            assert_eq!(run.event_stats().delivered, 3);
        } else {
            assert_eq!(run.event_stats().delivered, 2);
            assert_eq!(run.event_stats().dropped, 1);
        }
        assert_eq!(run.state(), RunState::Stopped);
        assert_eq!(run.event_stats().live_payload_bytes, 0);
        assert_eq!(run.retention().reserved_bytes, 0);
        assert_eq!(run.ownership().activations, 0);
        run.dispose().unwrap();
    }
}

#[test]
fn a_parallel_handler_fault_stops_sibling_roots_and_finishes_their_cleanup() {
    let mut faulting = handler(10, "parallel", 1, "drop-newest");
    faulting["activation"]["on_fault"] = json!("stop-run");
    let mut cleanup = variable_node(22, "set-variable", 102);
    cleanup["inputs"] = json!({"value":{"type":"core/u64@1","value":"9"}});
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            emit(2, 1),
            emit(3, 2),
            emit(4, 3),
            faulting,
            wait_node(11, 5),
            variable_node(12, "get-variable", 100),
            variable_node(13, "set-variable", 101),
            handler(20, "parallel", 1, "drop-newest"),
            wait_node(21, 100),
            cleanup,
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 2, "then", 3, "in"),
            edge(3, 3, "then", 4, "in"),
            edge(4, 10, "then", 11, "in"),
            edge(5, 11, "then", 13, "in"),
            edge(6, 12, "value", 13, "value"),
            edge(7, 20, "then", 21, "in"),
            edge(8, 21, "cancelled", 22, "in"),
        ],
        vec![
            variable(100, "run", None),
            variable(101, "run", None),
            variable(102, "run", Some(0)),
        ],
    );
    let mut run = run(&doc, plan);
    quiesce(&mut run);
    assert_eq!(
        (run.event_stats().delivered, run.event_stats().pending),
        (4, 2)
    );
    run.advance_to(5).unwrap();
    quiesce(&mut run);
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(run.event_stats().delivered, 4);
    assert_eq!(run.event_stats().dropped, 2);
    assert_eq!(run.event_stats().pending, 0);
    assert_eq!(run.event_stats().live_payload_bytes, 0);
    assert_eq!(run.retention().reserved_bytes, 0);
    assert_eq!(run.ownership().activations, 0);
    assert_eq!(run.ownership().timers, 0);
    assert_eq!(run.variable(id(102).parse().unwrap()), Some(&Value::U64(9)));
    assert_eq!(
        run.trace()
            .iter()
            .filter(
                |event| event.node.to_string() == id(22) && event.outcome == ControlOutcome::Then
            )
            .count(),
        2,
        "both suspended sibling roots must finish their cleanup"
    );
    run.dispose().unwrap();
}

#[test]
fn parallel_admission_rejects_zero_or_absent_maximum() {
    for maximum in [None, Some(0)] {
        let mut entry = handler(10, "parallel", 1, "drop-newest");
        if let Some(maximum) = maximum {
            entry["activation"]["max_parallel"] = json!(maximum);
        } else {
            entry["activation"]
                .as_object_mut()
                .unwrap()
                .remove("max_parallel");
        }
        assert!(
            compile_with_event(
                vec![entry],
                vec![],
                vec![],
                json!({"id":EVENT,
            "name":"pulse","payload":"core/u64@1","scope":"run"})
            )
            .is_err()
        );
    }
}
