//! Host pause: a paused run does no graph work and observes no time, yet still admits
//! input, and stop still reaches it.
use super::*;
use orange_model::id::NodeId;
use orange_runtime::{CancelToken, IngressError, RunError, StopMode};

const RESULT: u32 = 100;

fn node_id(n: u32) -> NodeId {
    id(n).parse().unwrap()
}

fn set_nine(n: u32) -> Json {
    let mut set = variable_node(n, "set-variable", RESULT);
    set["inputs"] = json!({"value": {"type": "core/u64@1", "value": "9"}});
    set
}

fn result(run: &Run) -> Option<&Value> {
    run.variable(id(RESULT).parse().unwrap())
}

/// On Start waits ten virtual nanoseconds, then sets `RESULT`.
fn waiting_run(options: RunOptions) -> Run {
    let (doc, plan) = compile(
        vec![core(1, "on-start"), wait_node(2, 10), set_nine(3)],
        vec![edge(1, 1, "then", 2, "in"), edge(2, 2, "then", 3, "in")],
        vec![variable(RESULT, "run", None)],
    );
    let mut run = run_with_options(&doc, plan, options);
    pump_until_idle(&mut run);
    run
}

/// A manual entry that sets `RESULT` to its payload.
fn manual_run(capacity: u32, overflow: &str) -> Run {
    let mut entry = core(1, "on-manual");
    entry["config"] = json!({"payload": "core/u64@1"});
    entry["activation"] = activation(0, capacity, overflow);
    let (doc, plan) = compile(
        vec![entry, variable_node(2, "set-variable", RESULT)],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 1, "payload", 2, "value"),
        ],
        vec![variable(RESULT, "run", None)],
    );
    let mut run = run(&doc, plan);
    pump_until_idle(&mut run);
    run
}

#[test]
fn a_paused_run_observes_no_time_and_resumes_where_it_stopped() {
    let mut run = waiting_run(RunOptions::default());
    run.advance_to(4).unwrap();
    run.pause().unwrap();
    assert_eq!(run.state(), RunState::Paused);

    assert_eq!(run.advance_to(50), Err(RunError::Paused));
    let progress = run.pump(64).unwrap();
    assert_eq!(
        (progress.operations, progress.ready, progress.next_wakeup),
        (0, false, None)
    );
    assert_eq!((run.now(), result(&run)), (4, None));

    run.resume().unwrap();
    assert_eq!((run.state(), run.now()), (RunState::Running, 4));
    assert_eq!(run.pump(0).unwrap().next_wakeup, Some(10));
    run.advance_to(9).unwrap();
    pump_until_idle(&mut run);
    assert_eq!(result(&run), None);
    run.advance_to(10).unwrap();
    pump_until_idle(&mut run);
    assert_eq!(result(&run), Some(&Value::U64(9)));
}

#[test]
fn input_admitted_while_paused_is_delivered_in_order_after_resume() {
    let mut run = manual_run(4, "fault");
    run.pause().unwrap();
    for payload in 1..=3 {
        let accepted = run
            .trigger(node_id(1), Some(&Value::U64(payload)), None)
            .unwrap();
        assert_eq!(accepted.source_seq, payload);
    }
    assert_eq!(run.pump(64).unwrap().operations, 0);
    let stats = run.event_stats();
    assert_eq!((stats.pending, stats.delivered), (3, 0));
    assert_eq!(result(&run), None);

    run.resume().unwrap();
    run.pump(1).unwrap();
    pump_until_idle(&mut run);
    let stats = run.event_stats();
    assert_eq!((stats.pending, stats.delivered), (0, 3));
    assert_eq!(result(&run), Some(&Value::U64(3)));
}

#[test]
fn a_full_queue_while_paused_refuses_without_faulting_the_run() {
    let mut run = manual_run(1, "fault");
    run.pause().unwrap();
    run.trigger(node_id(1), Some(&Value::U64(1)), None).unwrap();
    assert_eq!(
        run.trigger(node_id(1), Some(&Value::U64(2)), None),
        Err(IngressError::DeliveryLimit)
    );
    assert_eq!((run.state(), run.last_error()), (RunState::Paused, None));

    run.resume().unwrap();
    pump_until_idle(&mut run);
    assert_eq!(result(&run), Some(&Value::U64(1)));
    // The refusal consumed no sequence, so the sender can offer the same item again.
    let retried = run.trigger(node_id(1), Some(&Value::U64(2)), None).unwrap();
    assert_eq!(retried.source_seq, 2);
}

#[test]
fn graceful_stop_from_paused_runs_on_stop() {
    let (doc, plan) = compile(
        vec![core(1, "on-stop"), set_nine(2)],
        vec![edge(1, 1, "then", 2, "in")],
        vec![variable(RESULT, "run", None)],
    );
    let mut run = run(&doc, plan);
    pump_until_idle(&mut run);
    run.pause().unwrap();
    run.request_stop(StopMode::Graceful).unwrap();
    pump_until_idle(&mut run);
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(result(&run), Some(&Value::U64(9)));
    assert!(
        run.transitions()
            .windows(2)
            .any(|pair| pair == [RunState::Paused, RunState::Draining])
    );
}

#[test]
fn immediate_stop_and_cancellation_reach_a_paused_run() {
    let mut stopped = waiting_run(RunOptions::default());
    stopped.pause().unwrap();
    stopped.stop().unwrap();
    pump_until_idle(&mut stopped);
    assert_eq!(
        (stopped.state(), result(&stopped)),
        (RunState::Stopped, None)
    );

    let token = CancelToken::default();
    let mut options = RunOptions::default();
    options.data.cancel = Some(token.clone());
    let mut cancelled = waiting_run(options);
    cancelled.pause().unwrap();
    token.cancel();
    pump_until_idle(&mut cancelled);
    assert_eq!(
        (cancelled.state(), result(&cancelled)),
        (RunState::Stopped, None)
    );
}

#[test]
fn pause_and_resume_are_refused_outside_their_states() {
    let mut run = waiting_run(RunOptions::default());
    assert_eq!(run.resume(), Err(RunError::InvalidState));
    run.pause().unwrap();
    assert_eq!(run.pause(), Err(RunError::InvalidState));
    run.stop().unwrap();
    pump_until_idle(&mut run);
    assert_eq!(run.pause(), Err(RunError::InvalidState));
    assert_eq!(run.resume(), Err(RunError::InvalidState));
}

/// A manual entry with the given paused policy and reentrancy mode. With `wait`, its
/// handler waits ten virtual nanoseconds before setting `RESULT` to its payload.
fn policy_run(while_paused: &str, mode: &str, wait: bool) -> Run {
    let mut entry = core(1, "on-manual");
    entry["config"] = json!({"payload": "core/u64@1"});
    entry["activation"] = activation(0, 4, "fault");
    entry["activation"]["mode"] = json!(mode);
    entry["activation"]["while_paused"] = json!(while_paused);
    let set = variable_node(3, "set-variable", RESULT);
    let (nodes, edges) = if wait {
        (
            vec![entry, wait_node(2, 10), set],
            vec![
                edge(1, 1, "then", 2, "in"),
                edge(2, 2, "then", 3, "in"),
                edge(3, 1, "payload", 3, "value"),
            ],
        )
    } else {
        (
            vec![entry, set],
            vec![
                edge(1, 1, "then", 3, "in"),
                edge(3, 1, "payload", 3, "value"),
            ],
        )
    };
    let (doc, plan) = compile(nodes, edges, vec![variable(RESULT, "run", None)]);
    let mut run = run(&doc, plan);
    pump_until_idle(&mut run);
    run
}

fn trigger(run: &mut Run, payload: u64) -> orange_runtime::EventMetadata {
    run.trigger(node_id(1), Some(&Value::U64(payload)), None)
        .unwrap()
}

#[test]
fn a_latest_entry_keeps_only_the_newest_input_received_while_paused() {
    let mut run = policy_run("latest", "queue", false);
    run.pause().unwrap();
    for payload in 1..=3 {
        trigger(&mut run, payload);
    }
    let stats = run.event_stats();
    assert_eq!(
        (stats.pending, stats.paused_dropped, stats.overflows),
        (1, 2, 0)
    );
    run.resume().unwrap();
    run.pump(1).unwrap();
    pump_until_idle(&mut run);
    assert_eq!(result(&run), Some(&Value::U64(3)));
    assert_eq!(run.event_stats().delivered, 1);

    // The policy applies only while paused: a running entry queues every delivery.
    for payload in 4..=6 {
        trigger(&mut run, payload);
    }
    assert_eq!(run.event_stats().pending, 3);
}

#[test]
fn a_drop_entry_discards_input_received_while_paused_but_accepts_its_sequence() {
    let mut run = policy_run("drop", "queue", false);
    run.pause().unwrap();
    assert_eq!(trigger(&mut run, 1).source_seq, 1);
    assert_eq!(trigger(&mut run, 2).source_seq, 2);
    let stats = run.event_stats();
    assert_eq!(
        (stats.pending, stats.dropped, stats.paused_dropped),
        (0, 2, 2)
    );
    run.resume().unwrap();
    pump_until_idle(&mut run);
    assert_eq!(result(&run), None);

    trigger(&mut run, 3);
    pump_until_idle(&mut run);
    assert_eq!(result(&run), Some(&Value::U64(3)));
}

#[test]
fn a_drop_entry_in_replace_mode_does_not_cancel_its_running_handler() {
    let mut run = policy_run("drop", "replace", true);
    trigger(&mut run, 1);
    pump_until_idle(&mut run);
    run.pause().unwrap();
    trigger(&mut run, 2);
    run.resume().unwrap();
    pump_until_idle(&mut run);
    run.advance_to(10).unwrap();
    pump_until_idle(&mut run);
    assert_eq!(result(&run), Some(&Value::U64(1)));
    assert_eq!(run.event_stats().paused_dropped, 1);
}

#[test]
fn a_rejecting_source_is_refused_while_paused_and_can_resend_the_same_item() {
    use orange_runtime::{PausedSource, SourceOptions};
    let (doc, plan) = compile(
        vec![
            on_event(1, 0, 4, "fault"),
            variable_node(2, "set-variable", RESULT),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 1, "payload", 2, "value"),
        ],
        vec![variable(RESULT, "run", None)],
    );
    let mut run = run(&doc, plan);
    pump_until_idle(&mut run);
    let event = EVENT.parse().unwrap();
    let rejecting = run
        .register_event_source_with(
            event,
            SourceOptions::default().while_paused(PausedSource::Reject),
        )
        .unwrap();
    let admitting = run.register_event_source(event).unwrap();

    run.pause().unwrap();
    assert_eq!(
        run.ingest_event(&rejecting, 1, &Value::U64(7), None),
        Err(IngressError::Paused)
    );
    let stats = run.event_stats();
    assert_eq!((stats.paused_refused, stats.pending), (1, 0));
    run.ingest_event(&admitting, 1, &Value::U64(5), None)
        .unwrap();

    run.resume().unwrap();
    assert_eq!(
        run.ingest_event(&rejecting, 1, &Value::U64(7), None)
            .unwrap()
            .source_seq,
        1
    );
    run.pump(1).unwrap();
    pump_until_idle(&mut run);
    assert_eq!(result(&run), Some(&Value::U64(7)));
    assert_eq!(run.event_stats().delivered, 2);
}
