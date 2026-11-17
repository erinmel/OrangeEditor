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
        queries.push_back(Box::new(work));
    }
    flow.unpark();
}

/// The producer the run owns for a service node's stream.
pub(crate) struct Pulled {
    /// `None` once the stream is closed. A worker holds the lock while it waits for an
    /// item, so the run's thread never takes it.
    items: Arc<Mutex<Option<Box<dyn Items>>>>,
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
            items: Arc::new(Mutex::new(Some(items))),
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
        self.pool.submit(move || {
            if demand.is_closed() {
                return;
            }
            let pulled = items
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_mut()
                .map(|items| items.next());
            // A stream closed while this waited has no one to give the item to.
            let Some(pulled) = pulled else {
                return;
            };
            let id = demand.id;
            ask(&shared, &flow, move |run| {
                let _ = match pulled {
                    Some(Ok(value)) => run.complete_stream(id, Ok(Some(&value))),
                    Some(Err(fault)) => run.complete_stream(id, Err(fault)),
                    None => run.complete_stream(id, Ok(None)),
                };
            });
        });
        Ok(StreamStep::Pending)
    }

    fn close(self: Box<Self>, _reason: CloseReason) {
        // The items are dropped on a worker: a producer waiting for its next item
        // would otherwise hold up the flow's thread.
        let items = Arc::clone(&self.items);
        self.pool.submit(move || {
            let _ = items.lock().unwrap_or_else(PoisonError::into_inner).take();
        });
    }
}
