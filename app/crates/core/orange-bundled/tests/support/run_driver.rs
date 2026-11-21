//! `RunDriver`: one bounded mailbox through which many host threads feed one run.
use super::*;
use orange_model::id::NodeId;
use orange_runtime::{
    HostInput, InboxError, IngressError, Refused, RefusedReason, RunDriver, SourceOptions,
};

const RESULT: u32 = 100;
const COUNT: u32 = 101;

fn node_id(n: u32) -> NodeId {
    id(n).parse().unwrap()
}
fn result(run: &Run) -> Option<&Value> {
    run.variable(id(RESULT).parse().unwrap())
}
fn trigger(payload: u64) -> HostInput {
    HostInput::Trigger {
        node: node_id(1),
        payload: Some(Value::U64(payload)),
        wall_ns: None,
    }
}
fn refuse_none(refused: Refused) {
    panic!("unexpected refusal: {refused:?}");
}
fn settle(driver: &mut RunDriver, host_ns: u64) {
    for _ in 0..1000 {
        let turn = driver.turn(host_ns, 64, 64, &mut refuse_none).unwrap();
        if !turn.inbox_pending && !turn.progress.ready && driver.run().event_stats().pending == 0 {
            return;
        }
    }
    panic!("driver did not settle");
}

/// A manual entry that sets `RESULT` to its payload and counts its deliveries.
fn manual_driver(capacity: u32, inbox: usize) -> (RunDriver, orange_runtime::RunInbox) {
    let mut entry = core(1, "on-manual");
    entry["config"] = json!({"payload": "core/u64@1"});
    entry["activation"] = activation(0, capacity, "fault");
    let mut add = variable_node(3, "update-variable", COUNT);
    add["config"]["operation"] = json!("add");
    add["inputs"] = json!({"value": {"type": "core/u64@1", "value": "1"}});
    let (doc, plan) = compile(
        vec![entry, variable_node(2, "set-variable", RESULT), add],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 1, "payload", 2, "value"),
            edge(3, 2, "then", 3, "in"),
        ],
        vec![
            variable(RESULT, "run", None),
            variable(COUNT, "run", Some(0)),
        ],
    );
    RunDriver::new(run(&doc, plan), inbox, 0)
}

#[test]
fn inputs_from_several_threads_are_all_delivered_through_one_mailbox() {
    let (mut driver, inbox) = manual_driver(64, 64);
    let senders: Vec<_> = (0..4u64)
        .map(|thread| {
            let inbox = inbox.clone();
            std::thread::spawn(move || {
                for item in 0..8 {
                    inbox.send(trigger(thread * 100 + item)).unwrap();
                }
            })
        })
        .collect();
    for sender in senders {
        sender.join().unwrap();
    }
    settle(&mut driver, 0);
    assert_eq!(
        driver.run().variable(id(COUNT).parse().unwrap()),
        Some(&Value::U64(32))
    );
    assert_eq!(driver.run().event_stats().delivered, 32);
}

#[test]
fn a_full_mailbox_hands_the_input_back_without_blocking() {
    let (_driver, inbox) = manual_driver(4, 2);
    inbox.send(trigger(1)).unwrap();
    inbox.send(trigger(2)).unwrap();
    assert!(matches!(
        inbox.send(trigger(3)),
        Err(InboxError::Full(HostInput::Trigger { .. }))
    ));
}

#[test]
fn a_turn_takes_a_bounded_number_of_inputs_and_still_pumps() {
    let (mut driver, inbox) = manual_driver(8, 8);
    for payload in 1..=5 {
        inbox.send(trigger(payload)).unwrap();
    }
    let turn = driver.turn(0, 2, 64, &mut refuse_none).unwrap();
    assert_eq!((turn.applied, turn.inbox_pending), (2, true));
    assert!(turn.progress.operations > 0);
    settle(&mut driver, 0);
    assert_eq!(result(driver.run()), Some(&Value::U64(5)));
}

#[test]
fn an_input_the_run_refuses_is_handed_back_with_its_reason() {
    let (mut driver, inbox) = manual_driver(1, 8);
    inbox.send(trigger(1)).unwrap();
    inbox.send(trigger(2)).unwrap();
    let mut refused = Vec::new();
    let turn = driver
        .turn(0, 8, 0, &mut |refusal| refused.push(refusal))
        .unwrap();
    assert_eq!((turn.applied, turn.refused), (1, 1));
    assert!(matches!(
        refused.as_slice(),
        [Refused {
            input: HostInput::Trigger {
                payload: Some(Value::U64(2)),
                ..
            },
            reason: RefusedReason::Ingress(IngressError::DeliveryLimit),
        }]
    ));
}

#[test]
fn host_time_that_passes_while_paused_is_not_shown_to_the_run() {
    let mut set = variable_node(3, "set-variable", RESULT);
    set["inputs"] = json!({"value": {"type": "core/u64@1", "value": "9"}});
    let (doc, plan) = compile(
        vec![core(1, "on-start"), wait_node(2, 10), set],
        vec![edge(1, 1, "then", 2, "in"), edge(2, 2, "then", 3, "in")],
        vec![variable(RESULT, "run", None)],
    );
    let (mut driver, inbox) = RunDriver::new(run(&doc, plan), 8, 1_000);
    // The wait begins at virtual time zero, before any host time has passed.
    settle(&mut driver, 1_000);
    settle(&mut driver, 1_004);
    assert_eq!(driver.run().now(), 4);

    inbox.send(HostInput::Pause).unwrap();
    settle(&mut driver, 1_005);
    assert_eq!(
        (driver.run().state(), driver.run().now()),
        (RunState::Paused, 5)
    );
    settle(&mut driver, 9_000);
    assert_eq!((driver.run().now(), result(driver.run())), (5, None));

    inbox.send(HostInput::Resume).unwrap();
    settle(&mut driver, 9_001);
    assert_eq!(
        (driver.run().state(), driver.run().now()),
        (RunState::Running, 5)
    );
    settle(&mut driver, 9_005);
    assert_eq!(result(driver.run()), None);
    settle(&mut driver, 9_006);
    assert_eq!(result(driver.run()), Some(&Value::U64(9)));
}

#[test]
fn the_driver_assigns_source_sequences_and_a_stopped_run_refuses_input() {
    let (doc, plan) = compile(
        vec![
            on_event(1, 0, 8, "fault"),
            variable_node(2, "set-variable", RESULT),
        ],
        vec![
            edge(1, 1, "then", 2, "in"),
            edge(2, 1, "payload", 2, "value"),
        ],
        vec![variable(RESULT, "run", None)],
    );
    let (mut driver, inbox) = RunDriver::new(run(&doc, plan), 8, 0);
    let source = driver
        .register_source(EVENT.parse().unwrap(), SourceOptions::default())
        .unwrap();
    let event = |payload| HostInput::Event {
        source,
        payload: Value::U64(payload),
        wall_ns: None,
    };
    inbox.send(event(1)).unwrap();
    inbox.send(event(2)).unwrap();
    settle(&mut driver, 0);
    assert_eq!(driver.run().last_event().unwrap().source_seq, 2);
    assert_eq!(result(driver.run()), Some(&Value::U64(2)));

    inbox
        .send(HostInput::Stop(orange_runtime::StopMode::Immediate))
        .unwrap();
    inbox.send(event(3)).unwrap();
    let mut reasons = Vec::new();
    for _ in 0..8 {
        driver
            .turn(0, 8, 64, &mut |refusal| reasons.push(refusal.reason))
            .unwrap();
    }
    assert_eq!(driver.run().state(), RunState::Stopped);
    assert_eq!(reasons, [RefusedReason::Ingress(IngressError::NotRunning)]);
}
