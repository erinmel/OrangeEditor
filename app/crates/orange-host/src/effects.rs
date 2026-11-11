//! A fixed set of worker threads for work a run hands to the host.
//!
//! A run never executes blocking or guest-asynchronous work itself. It issues a request,
//! the flow's thread passes it here, and the result returns through the run's mailbox.
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

type Job = Box<dyn FnOnce() + Send>;

/// Cloneable handle to the workers. They exit once every handle is dropped.
#[derive(Debug, Clone)]
pub(crate) struct Pool(Sender<Job>);

impl Pool {
    pub fn new(workers: usize) -> std::io::Result<Self> {
        let (jobs, queue) = channel::<Job>();
        let queue = Arc::new(Mutex::new(queue));
        for index in 0..workers.max(1) {
            let queue = Arc::clone(&queue);
            let _ = thread::Builder::new()
                .name(format!("orange-effect-{index}"))
                .spawn(move || work(&queue))?;
        }
        Ok(Self(jobs))
    }
    /// Queues `job`. A job submitted after the workers are gone is dropped.
    pub fn submit(&self, job: impl FnOnce() + Send + 'static) {
        let _ = self.0.send(Box::new(job));
    }
}

fn work(queue: &Mutex<Receiver<Job>>) {
    loop {
        // The lock is held only while waiting for one job, never while running it.
        let job = queue.lock().unwrap_or_else(PoisonError::into_inner).recv();
        match job {
            Ok(job) => job(),
            Err(_) => return,
        }
    }
}
