use orange_compiler::KernelFault;
use orange_model::Value;
use orange_runtime::{
    CloseReason, PullStream, RunOptions, StreamCollectPoll, StreamDemand, StreamError, StreamRead,
    StreamStep,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

struct Fake {
    count: Arc<AtomicUsize>,
    closed: Arc<AtomicUsize>,
    demand: Arc<Mutex<Option<StreamDemand>>>,
    length: usize,
    asynchronous: bool,
}
impl PullStream for Fake {
    fn next(&mut self, demand: StreamDemand) -> Result<StreamStep, KernelFault> {
        let n = self.count.fetch_add(1, Ordering::Relaxed);
        *self.demand.lock().unwrap() = Some(demand);
        Ok(if self.asynchronous {
            StreamStep::Pending
        } else if n >= self.length {
            StreamStep::End
        } else {
            StreamStep::Item(Value::U64(n as u64))
        })
    }
    fn close(self: Box<Self>, _: CloseReason) {
        self.closed.fetch_add(1, Ordering::Relaxed);
    }
}
type ProducerProbe = (
    Box<Fake>,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
    Arc<Mutex<Option<StreamDemand>>>,
);
fn producer(length: usize, asynchronous: bool) -> ProducerProbe {
    let count = Arc::new(AtomicUsize::new(0));
    let closed = Arc::new(AtomicUsize::new(0));
    let demand = Arc::new(Mutex::new(None));
    (
        Box::new(Fake {
            count: count.clone(),
            closed: closed.clone(),
            demand: demand.clone(),
            length,
            asynchronous,
        }),
        count,
        closed,
        demand,
    )
}
#[test]
fn affine_pull_has_no_prefetch_and_length_independent_retention() {
    let (_, _, mut run) = crate::effects::setup(true, RunOptions::default());
    let (producer, count, closed, _) = producer(1000, false);
    let handle = run
        .open_stream(&"core/u64@1".parse().unwrap(), producer)
        .unwrap();
    let mut consumer = run.claim_stream(handle).unwrap();
    assert_eq!(run.claim_stream(handle).unwrap_err(), StreamError::Claimed);
    assert_eq!(count.load(Ordering::Relaxed), 0);
    let baseline = run.retention().reserved_bytes;
    for n in 0..1000 {
        assert!(
            matches!(run.next_stream(&mut consumer).unwrap(),StreamRead::Item(Value::U64(v)) if v==n)
        );
        assert_eq!(count.load(Ordering::Relaxed), n as usize + 1);
        assert_eq!(run.retention().reserved_bytes, baseline);
    }
    assert!(matches!(
        run.next_stream(&mut consumer).unwrap(),
        StreamRead::End
    ));
    assert_eq!(closed.load(Ordering::Relaxed), 1);
    run.close_stream(&mut consumer).unwrap();
    assert_eq!(closed.load(Ordering::Relaxed), 1);
    assert_eq!(run.ownership().resources, 0);
}
#[test]
fn pending_is_not_polled_and_results_are_typed_complete_once_and_cancellable() {
    let (_, _, mut run) = crate::effects::setup(true, RunOptions::default());
    let (producer, count, closed, demand) = producer(0, true);
    let handle = run
        .open_stream(&"core/u64@1".parse().unwrap(), producer)
        .unwrap();
    let mut consumer = run.claim_stream(handle).unwrap();
    let StreamRead::Pending(first) = run.next_stream(&mut consumer).unwrap() else {
        panic!()
    };
    for _ in 0..100 {
        assert!(
            matches!(run.next_stream(&mut consumer).unwrap(),StreamRead::Pending(id) if id==first)
        );
    }
    assert_eq!(count.load(Ordering::Relaxed), 1);
    assert_eq!(
        run.complete_stream(first, Ok(Some(&Value::Bool(true)))),
        Err(StreamError::Contract)
    );
    run.complete_stream(first, Ok(Some(&Value::U64(7))))
        .unwrap();
    assert_eq!(
        run.complete_stream(first, Ok(None)),
        Err(StreamError::Stale)
    );
    assert!(matches!(
        run.next_stream(&mut consumer).unwrap(),
        StreamRead::Item(Value::U64(7))
    ));
    let StreamRead::Pending(second) = run.next_stream(&mut consumer).unwrap() else {
        panic!()
    };
    assert_ne!(first, second);
    assert_eq!(
        run.complete_stream(first, Ok(None)),
        Err(StreamError::Stale)
    );
    run.close_stream(&mut consumer).unwrap();
    assert!(demand.lock().unwrap().as_ref().unwrap().is_closed());
    assert_eq!(
        run.complete_stream(second, Ok(None)),
        Err(StreamError::Stale)
    );
    assert_eq!(closed.load(Ordering::Relaxed), 1);
    assert_eq!(run.retention().reserved_bytes, 0);
}
#[test]
fn async_refusal_retries_and_run_stop_closes_once() {
    let options = RunOptions {
        limits: orange_runtime::RunLimits {
            max_stream_item_bytes: 15,
            ..Default::default()
        },
        ..Default::default()
    };
    let (_, _, mut run) = crate::effects::setup(true, options);
    let (producer, _, closed, demand) = producer(0, true);
    let h = run
        .open_stream(&"core/u64@1".parse().unwrap(), producer)
        .unwrap();
    let mut c = run.claim_stream(h).unwrap();
    let StreamRead::Pending(id) = run.next_stream(&mut c).unwrap() else {
        panic!()
    };
    assert_eq!(
        run.complete_stream(id, Ok(Some(&Value::U64(7)))),
        Err(StreamError::Limit)
    );
    run.complete_stream(id, Ok(None)).unwrap();
    run.stop().unwrap();
    for _ in 0..100 {
        if run.state() == orange_runtime::RunState::Stopped {
            break;
        }
        run.pump(1).unwrap();
    }
    assert_eq!(closed.load(Ordering::Relaxed), 1);
    assert!(demand.lock().unwrap().as_ref().unwrap().is_closed());
    assert_eq!(run.ownership().resources, 0);
    run.dispose().unwrap();
    assert_eq!(run.retention().reserved_bytes, 0);
}
#[test]
fn collect_yields_budgeted_whole_result_or_refuses_without_partial_result() {
    for (length, bound, success) in [(3, 3, true), (4, 3, false), (0, 0, true), (1, 0, false)] {
        let (_, _, mut run) = crate::effects::setup(true, RunOptions::default());
        let (producer, _, closed, _) = producer(length, false);
        let h = run
            .open_stream(&"core/u64@1".parse().unwrap(), producer)
            .unwrap();
        let mut c = run.claim_stream(h).unwrap();
        loop {
            match run.collect_stream(&mut c, bound, &mut 1) {
                Ok(StreamCollectPoll::Working) => {}
                Ok(StreamCollectPoll::Complete(Value::List(list))) => {
                    assert!(success);
                    assert_eq!(list.len(), length);
                    break;
                }
                Err(StreamError::Limit) => {
                    assert!(!success);
                    break;
                }
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(closed.load(Ordering::Relaxed), 1);
        assert_eq!(run.ownership().resources, 0);
        assert_eq!(run.retention().reserved_bytes, 0);
    }
}

#[test]
fn collection_resumes_one_pending_pull_and_stop_drops_partial_values() {
    let (_, _, mut run) = crate::effects::setup(true, RunOptions::default());
    let (producer, count, closed, _) = producer(0, true);
    let h = run
        .open_stream(&"core/u64@1".parse().unwrap(), producer)
        .unwrap();
    let mut c = run.claim_stream(h).unwrap();
    let StreamCollectPoll::Pending(first) = run.collect_stream(&mut c, 2, &mut 1).unwrap() else {
        panic!()
    };
    assert_eq!(run.next_stream(&mut c).unwrap_err(), StreamError::Claimed);
    run.complete_stream(first, Ok(Some(&Value::U64(3))))
        .unwrap();
    assert!(matches!(
        run.collect_stream(&mut c, 2, &mut 1).unwrap(),
        StreamCollectPoll::Working
    ));
    assert_eq!(count.load(Ordering::Relaxed), 1);
    assert_eq!(run.retention().reserved_bytes, 32);
    let StreamCollectPoll::Pending(_) = run.collect_stream(&mut c, 2, &mut 1).unwrap() else {
        panic!()
    };
    run.stop().unwrap();
    for _ in 0..100 {
        if run.state() == orange_runtime::RunState::Stopped {
            break;
        }
        run.pump(1).unwrap();
    }
    run.close_stream(&mut c).unwrap();
    assert_eq!(closed.load(Ordering::Relaxed), 1);
    assert_eq!(run.retention().reserved_bytes, 0);
}

#[test]
fn foreign_and_reused_resource_generations_never_alias_a_pull() {
    let (_, _, mut run) = crate::effects::setup(true, RunOptions::default());
    let (producer, _, closed, _) = producer(0, true);
    let h = run
        .open_stream(&"core/u64@1".parse().unwrap(), producer)
        .unwrap();
    let mut c = run.claim_stream(h).unwrap();
    let StreamRead::Pending(old) = run.next_stream(&mut c).unwrap() else {
        panic!()
    };
    let (_, _, mut foreign) = crate::effects::setup(true, RunOptions::default());
    assert_eq!(
        foreign.complete_stream(old, Ok(None)),
        Err(StreamError::WrongRun)
    );
    run.close_stream(&mut c).unwrap();
    let (replacement, _, _, _) = self::producer(0, true);
    let h2 = run
        .open_stream(&"core/u64@1".parse().unwrap(), replacement)
        .unwrap();
    assert_ne!(h, h2);
    let mut c2 = run.claim_stream(h2).unwrap();
    let StreamRead::Pending(new) = run.next_stream(&mut c2).unwrap() else {
        panic!()
    };
    assert_eq!(run.complete_stream(old, Ok(None)), Err(StreamError::Stale));
    run.complete_stream(new, Ok(None)).unwrap();
    assert!(matches!(run.next_stream(&mut c2).unwrap(), StreamRead::End));
    assert_eq!(closed.load(Ordering::Relaxed), 1);
}

#[test]
fn panicking_producer_is_closed_and_unacquired_producer_is_returned() {
    struct Panics(Arc<AtomicUsize>);
    impl PullStream for Panics {
        fn next(&mut self, _: StreamDemand) -> Result<StreamStep, KernelFault> {
            panic!("inert panic")
        }
        fn close(self: Box<Self>, _: CloseReason) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    let (_, _, mut run) = crate::effects::setup(true, RunOptions::default());
    let closed = Arc::new(AtomicUsize::new(0));
    let refused = run
        .open_stream(
            &"core/stream@1<core/u64@1>".parse().unwrap(),
            Box::new(Panics(closed.clone())),
        )
        .unwrap_err();
    assert_eq!(refused.error, StreamError::Contract);
    assert_eq!(closed.load(Ordering::Relaxed), 0);
    let h = run
        .open_stream(&"core/u64@1".parse().unwrap(), refused.producer)
        .unwrap();
    let mut c = run.claim_stream(h).unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run.next_stream(&mut c))).is_err()
    );
    run.close_stream(&mut c).unwrap();
    assert_eq!(closed.load(Ordering::Relaxed), 1);
    assert_eq!(run.ownership().resources, 0);
}

#[test]
fn collection_admits_exact_typed_list_bytes_and_refuses_one_under() {
    for bytes in [47, 48] {
        let options = RunOptions {
            limits: orange_runtime::RunLimits {
                max_stream_collect_bytes: bytes,
                ..Default::default()
            },
            ..Default::default()
        };
        let (_, _, mut run) = crate::effects::setup(true, options);
        let (producer, _, closed, _) = producer(2, false);
        let h = run
            .open_stream(&"core/u64@1".parse().unwrap(), producer)
            .unwrap();
        let mut c = run.claim_stream(h).unwrap();
        let outcome = run.collect_stream(&mut c, 2, &mut 100);
        if bytes == 48 {
            assert!(
                matches!(outcome,Ok(StreamCollectPoll::Complete(Value::List(items))) if items.len()==2)
            );
        } else {
            assert!(matches!(outcome, Err(StreamError::Limit)));
        }
        assert_eq!(closed.load(Ordering::Relaxed), 1);
        assert_eq!(run.retention().reserved_bytes, 0);
    }
}
