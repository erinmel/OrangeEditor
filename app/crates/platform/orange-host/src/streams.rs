//! Streams a service node produces: the node's items are pulled on worker threads, one
//! at a time and only when the graph asks, and handed to the run on its own thread.
use crate::effects::Pool;
use crate::flow::Shared;
use orange_runtime::{CloseReason, PullStream, Run, StreamDemand, StreamStep};
use orange_sdk::{Items, KernelFault};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::Thread;

/// Runs `work` on the flow's thread with its run. Dropped if the flow has ended.
pub(crate) fn ask(shared: &Shared, flow: &Thread, work: impl FnOnce(&mut Run) + Send + 'static) {
    if let Some(queries) = shared
        .queries
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_mut()
    {
        let wake = queries.is_empty();
        queries.push_back(Box::new(move |run| {
            work(run);
            true
        }));
        if wake {
            flow.unpark();
        }
    }
}

/// The producer the run owns for a service node's stream.
pub(crate) struct Pulled {
    /// A worker owns a reference throughout a pull, so closing never waits on its lock.
    items: Arc<Mutex<Box<dyn Items>>>,
    pool: Pool,
    shared: Arc<Shared>,
    flow: Thread,
}
impl Pulled {
    pub(crate) fn new(
        items: Box<dyn Items>,
        pool: Pool,
        shared: Arc<Shared>,
        flow: Thread,
    ) -> Self {
        Self {
            items: Arc::new(Mutex::new(items)),
            pool,
            shared,
            flow,
        }
    }
}
impl PullStream for Pulled {
    fn next(&mut self, demand: StreamDemand) -> Result<StreamStep, KernelFault> {
        let (items, shared, flow) = (
            Arc::clone(&self.items),
            Arc::clone(&self.shared),
            self.flow.clone(),
        );
        let submitted = self.pool.submit(move || {
            if demand.is_closed() {
                return;
            }
            let pulled = match crate::effects::guard(|| {
                items.lock().ok().and_then(|mut items| items.next())
            }) {
                Ok(pulled) => pulled,
                Err(fault) => Some(Err(fault)),
            };
            // A stream closed while this waited has no one to give the item to.
            if demand.is_closed() {
                return;
            }
            let id = demand.id;
            ask(&shared, &flow, move |run| {
                let _ = match pulled {
                    Some(Ok(value)) => run.complete_stream(id, Ok(Some(&value))),
                    Some(Err(fault)) => run.complete_stream(id, Err(fault)),
                    None => run.complete_stream(id, Ok(None)),
                };
            });
        });
        // Work the host cannot hold now fails at once instead of queueing.
        submitted.map_err(|_| crate::flow::BUSY)?;
        Ok(StreamStep::Pending)
    }

    fn close(self: Box<Self>, _reason: CloseReason) {
        // If cleanup cannot be queued, the rejected job releases its reference
        // here. Any outstanding pull keeps the producer alive until it returns.
        let items = self.items;
        let _ = self.pool.submit(move || drop(items));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orange_runtime::RunState;
    use std::sync::mpsc::channel;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn a_full_worker_queue_does_not_make_close_wait_for_a_pull() {
        let pool = Pool::new(1, 1, 1).unwrap();
        let shared = Arc::new(Shared {
            state: Mutex::new(RunState::Running),
            queries: Mutex::new(Some(Default::default())),
            closing: Default::default(),
            observed: Default::default(),
            swap: Mutex::new(None),
        });
        let stream = Box::new(Pulled::new(
            Box::new(std::iter::empty()),
            pool.clone(),
            shared,
            thread::current(),
        ));
        let items = Arc::clone(&stream.items);
        let (started, pulling) = channel();
        let (release, blocked) = channel::<()>();
        let (dropped, gone) = channel();
        pool.submit(move || {
            {
                let _pull = items.lock().unwrap();
                started.send(()).unwrap();
                let _ = blocked.recv();
            }
            assert_eq!(Arc::strong_count(&items), 1);
            drop(items);
            dropped.send(()).unwrap();
        })
        .unwrap();
        let timeout = Duration::from_secs(4);
        pulling.recv_timeout(timeout).unwrap();
        pool.submit(|| {}).unwrap();

        let (closed, done) = channel();
        let close = thread::spawn(move || {
            stream.close(CloseReason::Closed);
            let _ = closed.send(());
        });
        let result = done.recv_timeout(timeout);
        let _ = release.send(());
        close.join().unwrap();
        result.expect("close returns before the in-flight pull");
        gone.recv_timeout(timeout).unwrap();
    }
}
