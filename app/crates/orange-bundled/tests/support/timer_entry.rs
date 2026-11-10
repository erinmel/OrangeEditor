//! `core/on-timer`/`core/on-timeout` entries on the virtual clock (C04b2): ticks are
//! admitted through the shared bounded handler admission, with exact missed-tick counts.
use super::*;
use orange_model::id::NodeId;
use orange_runtime::{EventOrigin, EventTarget, StopMode};

const AT: u32 = 100;
const MISSED: u32 = 101;

fn timer(n: u32, kind: &str, config: Json, capacity: u32, overflow: &str) -> Json {
    let mut node = core(n, kind);
    node["config"] = config;
    node["activation"] = activation(0, capacity, overflow);
    node
}

/// The entry sets `AT` to each tick's `scheduled_at` and, for `on-timer`, adds its `missed`
/// to `MISSED`, so the handler observes every delivered tick.
fn timer_run(kind: &str, config: Json, capacity: u32, overflow: &str) -> Run {
    timer_run_with(kind, config, capacity, overflow, RunOptions::default()).unwrap()
}

fn timer_run_with(
    kind: &str,
    config: Json,
    capacity: u32,
    overflow: &str,
    options: RunOptions,
) -> Result<Run, orange_runtime::RunError> {
    let interval = kind == "on-timer";
    let mut nodes = vec![
        timer(1, kind, config, capacity, overflow),
        variable_node(2, "set-variable", AT),
    ];
    let mut edges = vec![
        edge(1, 1, "then", 2, "in"),
        edge(2, 1, "scheduled_at", 2, "value"),
    ];
    if interval {
        let mut add = variable_node(3, "update-variable", MISSED);
        add["config"]["operation"] = json!("add");
        nodes.push(add);
        edges.push(edge(3, 2, "then", 3, "in"));
        edges.push(edge(4, 1, "missed", 3, "value"));
    }
    let at = json!({"id": id(AT), "name": "at", "type": "core/duration@1", "scope": "run"});
    let (doc, plan) = compile(nodes, edges, vec![at, variable(MISSED, "run", Some(0))]);
    Run::start(plan.clone(), &doc, plan.data().snapshot(), 0, options)
}

fn node_id(n: u32) -> NodeId {
    id(n).parse().unwrap()
}

/// `(scheduled_at, missed total, delivered)` observed by the handler so far.
fn seen(run: &Run) -> (Option<i64>, u64, u64) {
    let at = match run.variable(id(AT).parse().unwrap()) {
        Some(Value::Duration(at)) => Some(*at),
        None => None,
        other => panic!("unexpected scheduled_at {other:?}"),
    };
    let missed = match run.variable(id(MISSED).parse().unwrap()) {
        Some(Value::U64(n)) => *n,
        // `core/on-timeout` has no `missed` output, so nothing reads the variable.
        None => 0,
        other => panic!("unexpected missed {other:?}"),
    };
    (at, missed, run.event_stats().delivered)
}

fn advance(run: &mut Run, now: u64) {
    run.advance_to(now).unwrap();
    pump_until_idle(run);
}

#[test]
fn interval_ticks_follow_the_schedule_without_drift() {
    let mut run = timer_run("on-timer", json!({"interval": "10"}), 4, "fault");
    let progress = run.pump(64).unwrap();
    assert_eq!(progress.next_wakeup, Some(10));
    assert_eq!(seen(&run), (None, 0, 0));
    // An armed timer entry is schedule, not a parked continuation.
    assert_eq!(run.pending_continuations(), 0);
    assert_eq!(run.ownership().timers, 1);

    advance(&mut run, 10);
    assert_eq!(seen(&run), (Some(10), 0, 1));
    let tick = run.last_event().unwrap();
    assert_eq!(tick.target, EventTarget::Timer(node_id(1)));
    assert_eq!(tick.origin, EventOrigin::Timer);
    assert_eq!(
        (tick.source_seq, tick.cause, tick.cause_depth, tick.wall_ns),
        (1, None, 0, None)
    );
    assert_eq!(tick.monotonic_ns, 10);

    // Handled late, the tick still reports its scheduled time and the next one stays on
    // the grid instead of drifting to now + interval.
    advance(&mut run, 25);
    assert_eq!(seen(&run), (Some(20), 0, 2));
    assert_eq!(run.last_event().unwrap().monotonic_ns, 25);
    assert_eq!(run.pump(0).unwrap().next_wakeup, Some(30));
}

#[test]
fn immediate_first_tick_is_due_at_start() {
    let mut run = timer_run(
        "on-timer",
        json!({"interval": "10", "first": "immediate"}),
        4,
        "fault",
    );
    pump_until_idle(&mut run);
    assert_eq!(seen(&run), (Some(0), 0, 1));
    assert_eq!(run.pump(0).unwrap().next_wakeup, Some(10));
}

#[test]
fn coalesce_delivers_the_latest_due_tick_with_the_missed_count() {
    let mut run = timer_run("on-timer", json!({"interval": "10"}), 4, "fault");
    advance(&mut run, 45);
    assert_eq!(seen(&run), (Some(40), 3, 1));
    assert_eq!(run.last_event().unwrap().source_seq, 4);
    assert_eq!(run.pump(0).unwrap().next_wakeup, Some(50));
}

#[test]
fn skip_delivers_only_ticks_handled_on_time() {
    let mut run = timer_run(
        "on-timer",
        json!({"interval": "10", "missed": "skip"}),
        4,
        "fault",
    );
    advance(&mut run, 45);
    assert_eq!(seen(&run), (None, 0, 0));
    assert_eq!(run.event_stats().accepted, 0);
    advance(&mut run, 50);
    assert_eq!(seen(&run), (Some(50), 4, 1));
    assert_eq!(run.last_event().unwrap().source_seq, 5);
}

#[test]
fn catch_up_delivers_the_latest_ticks_oldest_first() {
    let mut run = timer_run(
        "on-timer",
        json!({"interval": "10", "missed": "catch-up", "catch_up_max": 2}),
        4,
        "fault",
    );
    advance(&mut run, 45);
    // Ticks 3 (30, two missed before it) and 4 (40, none missed) in order.
    assert_eq!(seen(&run), (Some(40), 2, 2));
    assert_eq!(run.last_event().unwrap().source_seq, 4);
    advance(&mut run, 50);
    assert_eq!(seen(&run), (Some(50), 2, 3));
}

#[test]
fn catch_up_windows_are_charged_one_tick_per_pump_slice() {
    let mut run = timer_run(
        "on-timer",
        json!({"interval": "1", "missed": "catch-up", "catch_up_max": 1000}),
        1,
        "drop-newest",
    );
    run.advance_to(1000).unwrap();

    let progress = run.pump(1).unwrap();
    assert_eq!(progress.operations, 1);
    assert_eq!(run.event_stats().accepted + run.event_stats().dropped, 1);
    assert_eq!(run.event_stats().accepted, 1);

    // Zero budget does not advance the retained window.
    let before = run.event_stats();
    assert_eq!(run.pump(0).unwrap().operations, 0);
    assert_eq!(run.event_stats(), before);

    for expected_attempts in 2..=1000 {
        let progress = run.pump(1).unwrap();
        assert_eq!(progress.operations, 1);
        assert_eq!(
            run.event_stats().accepted + run.event_stats().dropped,
            expected_attempts
        );
    }
    let stats = run.event_stats();
    assert_eq!((stats.accepted, stats.dropped), (1, 999));
    assert_eq!(run.state(), RunState::Running);
}

#[test]
fn clock_advance_does_not_replace_a_partially_processed_window() {
    let mut run = timer_run(
        "on-timer",
        json!({"interval": "1", "missed": "catch-up", "catch_up_max": 5}),
        32,
        "fault",
    );
    run.advance_to(10).unwrap();
    assert_eq!(run.pump(1).unwrap().operations, 1);
    assert_eq!(run.pump(1).unwrap().operations, 1);
    assert_eq!(run.last_event().unwrap().source_seq, 7);

    // The selected 6..=10 window completes before the newly due 16..=20 window starts.
    run.advance_to(20).unwrap();
    for expected in 8..=10 {
        assert_eq!(run.pump(1).unwrap().operations, 1);
        assert_eq!(run.last_event().unwrap().source_seq, expected);
    }
    assert_eq!(run.pump(1).unwrap().operations, 1);
    assert_eq!(run.last_event().unwrap().source_seq, 16);
}

#[test]
fn sliced_catch_up_keeps_its_original_order_ahead_of_equal_due_timers() {
    let nodes = vec![
        timer(
            1,
            "on-timer",
            json!({"interval": "1", "missed": "catch-up", "catch_up_max": 3}),
            8,
            "fault",
        ),
        timer(
            2,
            "on-timer",
            json!({"interval": "1", "missed": "catch-up", "catch_up_max": 1}),
            8,
            "fault",
        ),
    ];
    let (doc, plan) = compile(nodes, vec![], vec![]);
    let mut run = Run::start(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        0,
        RunOptions::default(),
    )
    .unwrap();
    run.advance_to(3).unwrap();

    for expected_seq in 1..=3 {
        assert_eq!(run.pump(1).unwrap().operations, 1);
        let event = run.last_event().unwrap();
        assert_eq!(event.target, EventTarget::Timer(node_id(1)));
        assert_eq!(event.source_seq, expected_seq);
    }
    assert_eq!(run.pump(1).unwrap().operations, 1);
    let event = run.last_event().unwrap();
    assert_eq!(event.target, EventTarget::Timer(node_id(2)));
    assert_eq!(event.source_seq, 3);
}

#[test]
fn rearmed_timer_uses_normal_arrival_order_against_a_wait_at_the_next_tick() {
    let (doc, plan) = compile(
        vec![
            core(1, "on-start"),
            wait_node(2, 4),
            timer(10, "on-timer", json!({"interval": "2"}), 8, "fault"),
        ],
        vec![edge(1, 1, "then", 2, "in")],
        vec![],
    );
    let mut run = Run::start(
        plan.clone(),
        &doc,
        plan.data().snapshot(),
        0,
        RunOptions::default(),
    )
    .unwrap();
    pump_until_idle(&mut run);

    run.advance_to(2).unwrap();
    assert_eq!(run.pump(1).unwrap().operations, 1);
    assert_eq!(run.event_stats().accepted, 1);

    // The Wait was parked before the timer's next interval was armed, so at equal wake
    // times it keeps the earlier arrival sequence and resumes first.
    run.advance_to(4).unwrap();
    assert_eq!(run.pump(1).unwrap().operations, 1);
    assert!(run
        .trace()
        .iter()
        .any(|item| item.node == id(2).parse().unwrap() && item.outcome == ControlOutcome::Then));
    assert_eq!(run.event_stats().accepted, 1);

    assert_eq!(run.pump(1).unwrap().operations, 1);
    assert_eq!(run.event_stats().accepted, 2);
}

#[test]
fn max_operations_stops_a_large_catch_up_after_three_attempts() {
    let mut options = RunOptions::default();
    options.limits.max_operations = 3;
    let mut run = timer_run_with(
        "on-timer",
        json!({"interval": "1", "missed": "catch-up", "catch_up_max": 100000}),
        1,
        "drop-newest",
        options,
    )
    .unwrap();
    run.advance_to(100_000).unwrap();

    assert_eq!(run.pump(100), Err(orange_runtime::RunError::ResourceLimit));
    let stats = run.event_stats();
    assert_eq!(stats.accepted, 1);
    assert_eq!(stats.overflows, 2);
}

#[test]
fn stopping_discards_a_partially_processed_catch_up_window() {
    let mut run = timer_run(
        "on-timer",
        json!({"interval": "1", "missed": "catch-up", "catch_up_max": 100}),
        16,
        "fault",
    );
    run.advance_to(100).unwrap();
    assert_eq!(run.pump(1).unwrap().operations, 1);
    assert_eq!(run.event_stats().accepted, 1);

    run.stop().unwrap();
    assert_eq!(run.ownership().timers, 0);
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(run.event_stats().accepted, 1);
}

#[test]
fn a_refused_tick_is_counted_as_missed_on_the_next_admitted_one() {
    let mut run = timer_run(
        "on-timer",
        json!({"interval": "10", "missed": "catch-up", "catch_up_max": 3}),
        1,
        "fault",
    );
    // Three due ticks: the first fills the one pending slot and the fault policy refuses
    // the other two, without faulting the run.
    advance(&mut run, 30);
    let stats = run.event_stats();
    assert_eq!((stats.accepted, stats.overflows), (1, 2));
    assert_eq!(seen(&run), (Some(10), 0, 1));
    assert_eq!(run.state(), RunState::Running);
    advance(&mut run, 40);
    assert_eq!(seen(&run), (Some(40), 2, 2));
}

#[test]
fn timeout_fires_once() {
    let mut run = timer_run("on-timeout", json!({"delay": "5"}), 4, "fault");
    assert_eq!(run.pump(64).unwrap().next_wakeup, Some(5));
    advance(&mut run, 100);
    assert_eq!(seen(&run), (Some(5), 0, 1));
    assert_eq!(run.pump(0).unwrap().next_wakeup, None);
    assert_eq!(run.ownership().timers, 0);
    advance(&mut run, 200);
    assert_eq!(run.event_stats().delivered, 1);
}

#[test]
fn stopping_disarms_timer_entries() {
    let mut run = timer_run("on-timer", json!({"interval": "10"}), 4, "fault");
    advance(&mut run, 10);
    run.request_stop(StopMode::Graceful).unwrap();
    assert_eq!(run.ownership().timers, 0);
    pump_until_idle(&mut run);
    assert_eq!(run.state(), RunState::Stopped);

    let mut run = timer_run("on-timeout", json!({"delay": "5"}), 4, "fault");
    run.stop().unwrap();
    assert_eq!(run.ownership().timers, 0);
    pump_until_idle(&mut run);
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(run.event_stats().delivered, 0);

    // Each entry holds one timer for the whole run, so the timer limit admits it at start.
    let mut options = RunOptions::default();
    options.limits.max_timers = 0;
    assert_eq!(
        timer_run_with("on-timer", json!({"interval": "10"}), 4, "fault", options).err(),
        Some(orange_runtime::RunError::ResourceLimit)
    );
}

#[test]
fn timer_config_is_validated() {
    let (package, registry) = catalog();
    for (kind, config) in [
        ("on-timer", json!({})),
        ("on-timer", json!({"interval": "0"})),
        ("on-timer", json!({"interval": "10", "missed": "catch-up"})),
        ("on-timer", json!({"interval": "10", "first": "soon"})),
        ("on-timeout", json!({"delay": "-1"})),
        ("on-timeout", json!({"delay": "1", "missed": "skip"})),
    ] {
        let node = timer(1, kind, config.clone(), 4, "fault");
        let document = graph(&package, vec![node], vec![], vec![]);
        let analysis = analyze(&document, &registry);
        let codes: Vec<_> = analysis
            .report()
            .diagnostics
            .iter()
            .map(|d| d.code.as_str())
            .collect();
        assert!(
            codes.contains(&"E-TIMER-CONFIG"),
            "{kind} {config}: {codes:?}"
        );
    }
}
