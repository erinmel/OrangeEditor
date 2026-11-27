//! Event-handler activations against the run's aggregate budget.
use super::*;
use crate::budget_drive::{HUGE, boundary};
use orange_runtime::{RunError, StopMode};

/// Two handlers of one event stay live on a timer, each holding a local cell and the
/// retained payload. Every handler fits `max_state_bytes` alone.
fn waiting_handlers() -> (GraphDocument, Arc<orange_compiler::ControlPlan>) {
    compile(
        vec![
            core(1, "on-start"),
            emit(2, 5),
            on_event(10, 0, 4, "drop-newest"),
            on_event(11, 1, 4, "drop-newest"),
            update_local(20, 101),
            update_local(21, 101),
            wait_node(30, 10),
            wait_node(31, 10),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 10, "then", 20, "in"),
            edge(3, 11, "then", 21, "in"),
            edge(4, 20, "then", 30, "in"),
            edge(5, 21, "then", 31, "in"),
        ],
        vec![variable(101, "local", Some(0))],
    )
}

fn handler_run(
    doc: &GraphDocument,
    plan: &Arc<orange_compiler::ControlPlan>,
    limit: usize,
) -> Result<Run, RunError> {
    let mut options = RunOptions::default();
    options.limits.max_state_bytes = 64;
    options.limits.max_run_retained_bytes = limit;
    Run::start(plan.clone(), doc, plan.data().snapshot(), 17, options)
}

#[test]
fn concurrent_event_handlers_are_refused_together_by_the_aggregate_budget() {
    let (doc, plan) = waiting_handlers();
    let refusal = boundary(|limit| handler_run(&doc, &plan, limit));
    assert_eq!(refusal, RunError::AggregateLimit);
}

#[test]
fn stopping_with_live_handlers_returns_their_reservation() {
    let (doc, plan) = waiting_handlers();
    let mut run = handler_run(&doc, &plan, HUGE).unwrap();
    for _ in 0..100 {
        run.pump(1).unwrap();
        if run.ownership().timers == 2 {
            break;
        }
    }
    assert_eq!(run.ownership().timers, 2, "both handlers are waiting");
    assert!(run.retention().reserved_bytes > 0);
    run.request_stop(StopMode::Immediate).unwrap();
    pump_until_idle(&mut run);
    assert_eq!(run.state(), RunState::Stopped);
    assert_eq!(run.ownership().activations, 0);
    assert_eq!(run.retention().reserved_bytes, 0);
}
