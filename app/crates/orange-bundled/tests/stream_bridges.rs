//! Bounded bridge behavior through the public embedding API.
use orange_compiler::KernelFault;
use orange_model::Value;
use orange_runtime::{
    CloseReason, PullStream, PushAdmission, Run, RunOptions, StreamBufferOptions, StreamConsumer,
    StreamDemand, StreamError, StreamOverflow, StreamRead, StreamStep,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
const ITEM: &str = "core/u64@1";

#[test]
fn shared_stream_failure_text_is_charged_and_freed_after_consumption() {
    for tight in [false, true] {
        let mut run = run(RunOptions::default());
        let push = run
            .open_push_stream(
                &ITEM.parse().unwrap(),
                StreamBufferOptions {
                    capacity: 1,
                    max_bytes: if tight { 1 } else { 1024 },
                    overflow: StreamOverflow::Backpressure,
                },
            )
            .unwrap();
        let mut consumer = run.claim_stream(push.stream).unwrap();
        let code: Arc<str> = Arc::from("acme.stream/Failed");
        let weak = Arc::downgrade(&code);
        let fault = KernelFault {
            code: orange_compiler::FaultText::Shared(code),
            message_key: String::from("acme.stream/failed").into(),
            message: "The producer failed.",
        };
        let bytes = fault.code.retained_bytes() + fault.message_key.retained_bytes();
        run.fail_push_stream(&push.source, fault).unwrap();
        assert_eq!(
            run.stream_bridge_stats(push.source.bridge())
                .unwrap()
                .staged_bytes,
            if tight { 0 } else { bytes }
        );
        let StreamRead::Failed(fault) = run.next_stream(&mut consumer).unwrap() else {
            panic!("terminal fault must reach the consumer")
        };
        assert_eq!(
            fault.code,
            if tight {
                "core/LimitExceeded"
            } else {
                "acme.stream/Failed"
            }
        );
        run.close_stream(&mut consumer).unwrap();
        assert_eq!(run.retention().reserved_bytes, 0);
        drop(fault);
        assert!(weak.upgrade().is_none());
    }
}
fn run(options: RunOptions) -> Run {
    let (_, _, mut run) = crate::effects::setup(true, options);
    idle(&mut run);
    run
}
fn idle(run: &mut Run) {
    for _ in 0..3000 {
        if !run.pump(1).unwrap().ready {
            return;
        }
    }
    panic!("did not idle");
}
fn options(overflow: StreamOverflow) -> StreamBufferOptions {
    StreamBufferOptions {
        capacity: 1,
        max_bytes: 16,
        overflow,
    }
}
fn item(run: &mut Run, consumer: &mut StreamConsumer) -> u64 {
    match run.next_stream(consumer).unwrap() {
        StreamRead::Item(Value::U64(n)) => n,
        result => panic!("{result:?}"),
    }
}
#[derive(Default, Clone)]
struct Probe {
    reads: Arc<AtomicUsize>,
    closes: Arc<AtomicUsize>,
    demands: Arc<Mutex<Vec<StreamDemand>>>,
}
struct Producer {
    probe: Probe,
    length: usize,
    asynchronous: bool,
}
impl PullStream for Producer {
    fn next(&mut self, demand: StreamDemand) -> Result<StreamStep, KernelFault> {
        let n = self.probe.reads.fetch_add(1, Ordering::Relaxed);
        self.probe.demands.lock().unwrap().push(demand);
        Ok(if self.asynchronous {
            StreamStep::Pending
        } else if n == self.length {
            StreamStep::End
        } else {
            StreamStep::Item(Value::U64(n as u64 + 1))
        })
    }
    fn close(self: Box<Self>, _: CloseReason) {
        self.probe.closes.fetch_add(1, Ordering::Relaxed);
    }
}
fn source(
    run: &mut Run,
    length: usize,
    asynchronous: bool,
) -> (orange_runtime::StreamHandle, Probe) {
    let probe = Probe::default();
    let handle = run
        .open_stream(
            &ITEM.parse().unwrap(),
            Box::new(Producer {
                probe: probe.clone(),
                length,
                asynchronous,
            }),
        )
        .unwrap();
    (handle, probe)
}

#[test]
fn push_backpressure_and_refusals_preserve_sequence_for_retry() {
    let mut run = run(RunOptions::default());
    let push = run
        .open_push_stream(
            &ITEM.parse().unwrap(),
            options(StreamOverflow::Backpressure),
        )
        .unwrap();
    let mut consumer = run.claim_stream(push.stream).unwrap();
    assert_eq!(
        run.push_stream(&push.source, 0, &Value::U64(1)),
        Err(StreamError::Sequence)
    );
    assert_eq!(
        run.push_stream(&push.source, 1, &Value::U64(1)),
        Ok(PushAdmission::Accepted)
    );
    let before = run.stream_bridge_stats(push.source.bridge()).unwrap();
    assert_eq!(
        run.push_stream(&push.source, 2, &Value::U64(2)),
        Err(StreamError::Full)
    );
    assert_eq!(
        run.stream_bridge_stats(push.source.bridge()).unwrap(),
        before
    );
    assert_eq!(item(&mut run, &mut consumer), 1);
    assert_eq!(
        run.push_stream(&push.source, 2, &Value::Bool(true)),
        Err(StreamError::Contract)
    );
    assert_eq!(
        run.push_stream(&push.source, 2, &Value::U64(2)),
        Ok(PushAdmission::Accepted)
    );
    assert_eq!(
        run.push_stream(&push.source, 2, &Value::U64(99)),
        Err(StreamError::Sequence)
    );
    run.finish_push_stream(&push.source).unwrap();
    assert_eq!(item(&mut run, &mut consumer), 2);
    assert!(matches!(
        run.next_stream(&mut consumer).unwrap(),
        StreamRead::End
    ));
    assert!(push.source.is_closed());
    assert_eq!(run.stream_bridges().count(), 0);
    assert_eq!(run.ownership().resources, 0);
}
#[test]
fn push_drop_policies_report_loss_and_fault_overflow_discards_unread_items() {
    for policy in [
        StreamOverflow::DropOldest,
        StreamOverflow::DropNewest,
        StreamOverflow::Fault,
    ] {
        let mut run = run(RunOptions::default());
        let push = run
            .open_push_stream(&ITEM.parse().unwrap(), options(policy))
            .unwrap();
        let mut consumer = run.claim_stream(push.stream).unwrap();
        run.push_stream(&push.source, 1, &Value::U64(1)).unwrap();
        let receipt = run.push_stream(&push.source, 2, &Value::U64(2));
        let stats = run
            .stream_bridge_stats(push.source.bridge())
            .unwrap()
            .branches[0]
            .unwrap();
        assert_eq!(stats.dropped, 1);
        if policy == StreamOverflow::Fault {
            assert_eq!(receipt, Err(StreamError::Overflow));
            assert!(
                matches!(run.next_stream(&mut consumer).unwrap(),StreamRead::Failed(f) if f.code=="core/QueueOverflow")
            );
        } else {
            assert_eq!(
                receipt,
                Ok(if policy == StreamOverflow::DropNewest {
                    PushAdmission::Dropped
                } else {
                    PushAdmission::Accepted
                })
            );
            assert_eq!(stats.accepted, 2);
            assert_eq!(
                item(&mut run, &mut consumer),
                if policy == StreamOverflow::DropOldest {
                    2
                } else {
                    1
                }
            );
            run.close_stream(&mut consumer).unwrap();
        }
        assert_eq!(run.stream_bridges().count(), 0);
        assert!(push.source.is_closed());
    }
}
#[test]
fn tee_backpressure_never_polls_and_one_closed_consumer_releases_the_other() {
    let mut run = run(RunOptions::default());
    let (source, probe) = source(&mut run, 3, false);
    let tee = run
        .tee_stream(source, options(StreamOverflow::Backpressure))
        .unwrap();
    assert_eq!(run.claim_stream(source).unwrap_err(), StreamError::Claimed);
    let mut left = run.claim_stream(tee.left).unwrap();
    let mut right = run.claim_stream(tee.right).unwrap();
    assert_eq!(item(&mut run, &mut left), 1);
    let StreamRead::Pending(pending) = run.next_stream(&mut left).unwrap() else {
        panic!()
    };
    assert_eq!(
        run.complete_stream(pending, Ok(Some(&Value::U64(99)))),
        Err(StreamError::Contract)
    );
    idle(&mut run);
    assert_eq!(probe.reads.load(Ordering::Relaxed), 1);
    assert_eq!(item(&mut run, &mut right), 1);
    idle(&mut run);
    assert_eq!(item(&mut run, &mut left), 2);
    assert!(matches!(
        run.next_stream(&mut left).unwrap(),
        StreamRead::Pending(_)
    ));
    run.close_stream(&mut right).unwrap();
    idle(&mut run);
    assert_eq!(item(&mut run, &mut left), 3);
    assert!(matches!(
        run.next_stream(&mut left).unwrap(),
        StreamRead::End
    ));
    assert_eq!(probe.reads.load(Ordering::Relaxed), 4);
    assert_eq!(probe.closes.load(Ordering::Relaxed), 1);
    assert_eq!(run.stream_bridges().count(), 0);
    assert_eq!(run.ownership().resources, 0);
}
#[test]
fn pending_upstream_wakes_both_tee_consumers_once_and_revocation_discards_results() {
    let mut run = run(RunOptions::default());
    let (source, probe) = source(&mut run, 0, true);
    let tee = run
        .tee_stream(source, options(StreamOverflow::Backpressure))
        .unwrap();
    let mut left = run.claim_stream(tee.left).unwrap();
    let mut right = run.claim_stream(tee.right).unwrap();
    assert!(matches!(
        run.next_stream(&mut left).unwrap(),
        StreamRead::Pending(_)
    ));
    assert!(matches!(
        run.next_stream(&mut right).unwrap(),
        StreamRead::Pending(_)
    ));
    idle(&mut run);
    assert_eq!(probe.reads.load(Ordering::Relaxed), 1);
    let demand = probe.demands.lock().unwrap()[0].clone();
    run.complete_stream(demand.id, Ok(Some(&Value::U64(7))))
        .unwrap();
    assert!(run.pump(0).unwrap().ready);
    idle(&mut run);
    assert_eq!(item(&mut run, &mut left), 7);
    assert_eq!(item(&mut run, &mut right), 7);
    assert!(matches!(
        run.next_stream(&mut left).unwrap(),
        StreamRead::Pending(_)
    ));
    let demand = probe.demands.lock().unwrap()[1].clone();
    run.complete_stream(demand.id, Ok(Some(&Value::U64(8))))
        .unwrap();
    run.revoke_stream_bridge(tee.bridge).unwrap();
    idle(&mut run);
    assert!(demand.is_closed());
    assert_eq!(probe.closes.load(Ordering::Relaxed), 1);
    assert!(run.next_stream(&mut left).is_err());
    assert_eq!(run.stream_bridges().count(), 0);
}
#[test]
fn aggregate_full_push_refuses_atomically_and_retries_after_one_item_is_consumed() {
    let mut limits = RunOptions::default();
    limits.limits.max_run_retained_bytes = 64;
    let mut run = run(limits);
    let push = run
        .open_push_stream(
            &ITEM.parse().unwrap(),
            StreamBufferOptions {
                capacity: 5,
                max_bytes: 80,
                overflow: StreamOverflow::Backpressure,
            },
        )
        .unwrap();
    let mut consumer = run.claim_stream(push.stream).unwrap();
    for sequence in 1..=4 {
        run.push_stream(&push.source, sequence, &Value::U64(sequence))
            .unwrap();
    }
    let before = run.stream_bridge_stats(push.source.bridge()).unwrap();
    assert_eq!(
        run.push_stream(&push.source, 5, &Value::U64(5)),
        Err(StreamError::Limit)
    );
    assert_eq!(
        run.stream_bridge_stats(push.source.bridge()).unwrap(),
        before
    );
    assert_eq!(item(&mut run, &mut consumer), 1);
    run.push_stream(&push.source, 5, &Value::U64(5)).unwrap();
    run.close_stream(&mut consumer).unwrap();
    assert_eq!(run.retention().reserved_bytes, 0);
}

#[test]
fn tee_loss_policies_bound_the_slow_queue_and_fault_has_no_partial_new_delivery() {
    for policy in [
        StreamOverflow::DropOldest,
        StreamOverflow::DropNewest,
        StreamOverflow::Fault,
    ] {
        let mut run = run(RunOptions::default());
        let (source, probe) = source(&mut run, 4, false);
        let tee = run.tee_stream(source, options(policy)).unwrap();
        let mut left = run.claim_stream(tee.left).unwrap();
        let mut right = run.claim_stream(tee.right).unwrap();
        assert_eq!(item(&mut run, &mut left), 1);
        if policy == StreamOverflow::Fault {
            assert!(
                matches!(run.next_stream(&mut left).unwrap(),StreamRead::Failed(f) if f.code=="core/QueueOverflow")
            );
            let stats = run.stream_bridge_stats(tee.bridge).unwrap();
            assert_eq!(stats.branches[1].unwrap().accepted, 1);
            assert!(
                matches!(run.next_stream(&mut right).unwrap(),StreamRead::Failed(f) if f.code=="core/QueueOverflow")
            );
        } else {
            for expected in 2..=4 {
                assert_eq!(item(&mut run, &mut left), expected);
            }
            let stats = run.stream_bridge_stats(tee.bridge).unwrap();
            let slow = stats.branches[1].unwrap();
            assert_eq!(
                (
                    slow.accepted,
                    slow.dropped,
                    slow.queued_items,
                    slow.queued_bytes
                ),
                (4, 3, 1, 16)
            );
            assert_eq!(
                item(&mut run, &mut right),
                if policy == StreamOverflow::DropOldest {
                    4
                } else {
                    1
                }
            );
            run.close_stream(&mut left).unwrap();
            assert_eq!(probe.closes.load(Ordering::Relaxed), 0);
            run.close_stream(&mut right).unwrap();
        }
        assert_eq!(probe.closes.load(Ordering::Relaxed), 1);
        assert_eq!(run.stream_bridges().count(), 0);
    }
}
#[test]
fn nested_tee_forwards_one_pending_source_and_last_close_reaps_the_whole_chain() {
    let mut run = run(RunOptions::default());
    let (source, probe) = source(&mut run, 0, true);
    let parent = run
        .tee_stream(source, options(StreamOverflow::Backpressure))
        .unwrap();
    let child = run
        .tee_stream(parent.left, options(StreamOverflow::Backpressure))
        .unwrap();
    let mut unused = run.claim_stream(parent.right).unwrap();
    run.close_stream(&mut unused).unwrap();
    let mut left = run.claim_stream(child.left).unwrap();
    let mut right = run.claim_stream(child.right).unwrap();
    assert!(matches!(
        run.next_stream(&mut left).unwrap(),
        StreamRead::Pending(_)
    ));
    assert!(matches!(
        run.next_stream(&mut right).unwrap(),
        StreamRead::Pending(_)
    ));
    assert_eq!(probe.reads.load(Ordering::Relaxed), 1);
    let demand = probe.demands.lock().unwrap()[0].clone();
    run.complete_stream(demand.id, Ok(Some(&Value::U64(9))))
        .unwrap();
    idle(&mut run);
    assert_eq!(item(&mut run, &mut left), 9);
    assert_eq!(item(&mut run, &mut right), 9);
    run.close_stream(&mut left).unwrap();
    assert_eq!(probe.closes.load(Ordering::Relaxed), 0);
    run.close_stream(&mut right).unwrap();
    assert_eq!(probe.closes.load(Ordering::Relaxed), 1);
    assert_eq!(run.ownership().resources, 0);
    assert_eq!(run.stream_bridges().count(), 0);
    assert!(demand.is_closed());
}
#[test]
fn tee_depth_and_resource_refusals_leave_the_input_unclaimed() {
    let mut limited = RunOptions::default();
    limited.limits.max_resources = 2;
    let mut limited = run(limited);
    let (handle, probe) = source(&mut limited, 1, false);
    assert_eq!(
        limited
            .tee_stream(handle, options(StreamOverflow::Backpressure))
            .unwrap_err(),
        StreamError::Limit
    );
    let mut input = limited.claim_stream(handle).unwrap();
    limited.close_stream(&mut input).unwrap();
    assert_eq!(probe.closes.load(Ordering::Relaxed), 1);
    let mut run = run(RunOptions::default());
    let (mut handle, probe) = source(&mut run, 1, false);
    for _ in 0..32 {
        let tee = run
            .tee_stream(handle, options(StreamOverflow::Backpressure))
            .unwrap();
        handle = tee.left;
        let mut unused = run.claim_stream(tee.right).unwrap();
        run.close_stream(&mut unused).unwrap();
    }
    assert_eq!(
        run.tee_stream(handle, options(StreamOverflow::Backpressure))
            .unwrap_err(),
        StreamError::Limit
    );
    assert_eq!(probe.reads.load(Ordering::Relaxed), 0);
    let mut input = run.claim_stream(handle).unwrap();
    run.close_stream(&mut input).unwrap();
    assert_eq!(probe.closes.load(Ordering::Relaxed), 1);
    assert_eq!(run.ownership().resources, 0);
}
#[test]
fn push_byte_limits_generation_and_foreign_source_refusals_are_atomic() {
    let mut other = run(RunOptions::default());
    let mut run = run(RunOptions::default());
    let push = run
        .open_push_stream(
            &ITEM.parse().unwrap(),
            StreamBufferOptions {
                capacity: 3,
                max_bytes: 32,
                overflow: StreamOverflow::Backpressure,
            },
        )
        .unwrap();
    assert_eq!(
        other.push_stream(&push.source, 1, &Value::U64(1)),
        Err(StreamError::WrongRun)
    );
    run.push_stream(&push.source, 1, &Value::U64(1)).unwrap();
    run.push_stream(&push.source, 2, &Value::U64(2)).unwrap();
    assert_eq!(
        run.push_stream(&push.source, 3, &Value::U64(3)),
        Err(StreamError::Full)
    );
    let mut consumer = run.claim_stream(push.stream).unwrap();
    assert_eq!(item(&mut run, &mut consumer), 1);
    run.push_stream(&push.source, 3, &Value::U64(3)).unwrap();
    run.close_stream(&mut consumer).unwrap();
    let replacement = run
        .open_push_stream(
            &ITEM.parse().unwrap(),
            options(StreamOverflow::Backpressure),
        )
        .unwrap();
    assert_ne!(push.source.bridge(), replacement.source.bridge());
    assert_eq!(
        run.push_stream(&push.source, 4, &Value::U64(4)),
        Err(StreamError::Stale)
    );
    run.revoke_stream_bridge(replacement.source.bridge())
        .unwrap();
    assert!(replacement.source.is_closed());
    assert_eq!(run.retention().reserved_bytes, 0);
}
#[test]
fn tee_joint_aggregate_refusal_never_publishes_the_new_item_to_only_one_branch() {
    let mut limits = RunOptions::default();
    limits.limits.max_run_retained_bytes = 32;
    let mut run = run(limits);
    let (source, probe) = source(&mut run, 3, false);
    let tee = run
        .tee_stream(
            source,
            StreamBufferOptions {
                capacity: 2,
                max_bytes: 32,
                overflow: StreamOverflow::DropOldest,
            },
        )
        .unwrap();
    let mut left = run.claim_stream(tee.left).unwrap();
    let mut right = run.claim_stream(tee.right).unwrap();
    assert_eq!(item(&mut run, &mut left), 1);
    assert!(
        matches!(run.next_stream(&mut left).unwrap(),StreamRead::Failed(f) if f.code=="core/LimitExceeded")
    );
    let stats = run.stream_bridge_stats(tee.bridge).unwrap();
    assert_eq!(stats.branches[1].unwrap().accepted, 1);
    assert!(
        matches!(run.next_stream(&mut right).unwrap(),StreamRead::Failed(f) if f.code=="core/LimitExceeded")
    );
    assert_eq!(probe.closes.load(Ordering::Relaxed), 1);
    assert_eq!(run.retention().reserved_bytes, 0);
}

#[test]
fn variable_byte_backpressure_stages_one_item_and_sleeps_until_space_opens() {
    let mut run = run(RunOptions::default());
    let ty = "core/string@1".parse().unwrap();
    let upstream = run
        .open_push_stream(
            &ty,
            StreamBufferOptions {
                capacity: 4,
                max_bytes: 128,
                overflow: StreamOverflow::Backpressure,
            },
        )
        .unwrap();
    let text = |value: &str| Value::Str(Arc::new(value.into()));
    run.push_stream(&upstream.source, 1, &text("")).unwrap();
    run.push_stream(&upstream.source, 2, &text("abcd")).unwrap();
    let tee = run
        .tee_stream(
            upstream.stream,
            StreamBufferOptions {
                capacity: 3,
                max_bytes: 32,
                overflow: StreamOverflow::Backpressure,
            },
        )
        .unwrap();
    let mut left = run.claim_stream(tee.left).unwrap();
    let mut right = run.claim_stream(tee.right).unwrap();
    assert!(
        matches!(run.next_stream(&mut left).unwrap(),StreamRead::Item(Value::Str(s)) if s.is_empty())
    );
    assert!(matches!(
        run.next_stream(&mut left).unwrap(),
        StreamRead::Pending(_)
    ));
    for _ in 0..20 {
        assert!(!run.pump(1).unwrap().ready);
    }
    let stats = run.stream_bridge_stats(tee.bridge).unwrap();
    assert_eq!(stats.staged_bytes, 20);
    assert_eq!(stats.source_sequence, 2);
    assert_eq!(
        (
            stats.branches[1].unwrap().queued_items,
            stats.branches[1].unwrap().queued_bytes
        ),
        (1, 16)
    );
    assert!(
        matches!(run.next_stream(&mut right).unwrap(),StreamRead::Item(Value::Str(s)) if s.is_empty())
    );
    idle(&mut run);
    assert!(
        matches!(run.next_stream(&mut left).unwrap(),StreamRead::Item(Value::Str(s)) if s.as_str()=="abcd")
    );
    assert!(
        matches!(run.next_stream(&mut right).unwrap(),StreamRead::Item(Value::Str(s)) if s.as_str()=="abcd")
    );
    run.finish_push_stream(&upstream.source).unwrap();
    assert!(matches!(
        run.next_stream(&mut left).unwrap(),
        StreamRead::End
    ));
    assert!(matches!(
        run.next_stream(&mut right).unwrap(),
        StreamRead::End
    ));
    assert_eq!(run.stream_bridges().count(), 0);
    assert_eq!(run.retention().reserved_bytes, 0);
}
#[test]
fn stop_or_drop_closes_pending_tee_upstream_and_push_authority_once() {
    for stop in [false, true] {
        let mut run = run(RunOptions::default());
        let (source, probe) = source(&mut run, 100, false);
        let tee = run
            .tee_stream(source, options(StreamOverflow::Backpressure))
            .unwrap();
        let mut left = run.claim_stream(tee.left).unwrap();
        let push = run
            .open_push_stream(
                &ITEM.parse().unwrap(),
                options(StreamOverflow::Backpressure),
            )
            .unwrap();
        run.push_stream(&push.source, 1, &Value::U64(9)).unwrap();
        assert_eq!(item(&mut run, &mut left), 1);
        assert!(matches!(
            run.next_stream(&mut left).unwrap(),
            StreamRead::Pending(_)
        ));
        if stop {
            run.stop().unwrap();
            while run.state() != orange_runtime::RunState::Stopped {
                run.pump(1).unwrap();
            }
            assert_eq!(run.stream_bridges().count(), 0);
            assert_eq!(run.retention().reserved_bytes, 0);
            run.dispose().unwrap();
        }
        drop(run);
        assert_eq!(probe.closes.load(Ordering::Relaxed), 1);
        assert!(push.source.is_closed());
    }
}
