//! Worker threads for work a run hands to the host.
//!
//! A run never executes blocking or guest-asynchronous work itself. It issues a request,
//! the flow's thread passes it here, and the result returns through the run's mailbox.
//!
//! Some work holds its thread for a long time: a sound that plays to its end, a slow
//! server, a recording. A fixed set of threads would let a few of those stall every other
//! node of every flow, so the pool keeps `core` threads and adds more, up to `max`, when
//! every thread is busy. An added thread leaves after it has been idle for [`LINGER`].
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

type Job = Box<dyn FnOnce() + Send>;

/// How long a thread above the core count waits for work before it ends.
pub(crate) const LINGER: Duration = Duration::from_secs(10);

/// Cloneable handle to the workers. They exit once every handle is dropped.
#[derive(Debug, Clone)]
pub(crate) struct Pool {
    jobs: Sender<Job>,
    shared: Arc<Shared>,
}

#[derive(Debug)]
struct Shared {
    queue: Mutex<Receiver<Job>>,
    counts: Mutex<Counts>,
    core: usize,
    max: usize,
    /// Most jobs waiting for a thread, across every flow of the host.
    max_waiting: usize,
}

/// The pool already holds as many waiting jobs as it may.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Busy;

/// Threads alive and threads waiting for a job.
#[derive(Debug, Default)]
struct Counts {
    threads: usize,
    idle: usize,
    /// Jobs submitted that no thread has taken yet.
    waiting: usize,
}

impl Pool {
    /// A pool that keeps `core` threads, grows to at most `max` while all are busy, and
    /// holds at most `max_waiting` jobs no thread has taken yet.
    pub(crate) fn new(core: usize, max: usize, max_waiting: usize) -> std::io::Result<Self> {
        let (jobs, queue) = channel::<Job>();
        let core = core.max(1);
        let pool = Self {
            jobs,
            shared: Arc::new(Shared {
                queue: Mutex::new(queue),
                counts: Mutex::new(Counts::default()),
                core,
                max: max.max(core),
                max_waiting: max_waiting.max(1),
            }),
        };
        for _ in 0..core {
            pool.shared.counts().threads += 1;
            pool.spawn()?;
        }
        Ok(pool)
    }

    /// Queues `job`, adding a thread when none is free and the pool may grow. Refused
    /// with [`Busy`] when the host already has as many jobs waiting as it may hold. A
    /// job submitted after the workers are gone is dropped.
    pub(crate) fn submit(&self, job: impl FnOnce() + Send + 'static) -> Result<(), Busy> {
        // Deciding to grow and counting the new thread happen under one lock, so two
        // submissions at once never both take the last free place.
        let grow = {
            let mut counts = self.shared.counts();
            if counts.waiting >= self.shared.max_waiting {
                return Err(Busy);
            }
            counts.waiting += 1;
            let grow = counts.waiting > counts.idle && counts.threads < self.shared.max;
            if grow {
                counts.threads += 1;
            }
            grow
        };
        if self.jobs.send(Box::new(job)).is_err() {
            let mut counts = self.shared.counts();
            counts.waiting -= 1;
            if grow {
                counts.threads -= 1;
            }
            return Ok(());
        }
        // A thread that cannot be started leaves the job to the ones there are.
        if grow {
            let _ = self.spawn();
        }
        Ok(())
    }

    /// Threads alive now.
    #[cfg(test)]
    fn threads(&self) -> usize {
        self.shared.counts().threads
    }

    /// Starts a thread the caller has already counted.
    fn spawn(&self) -> std::io::Result<()> {
        let index = self.shared.counts().threads;
        let shared = Arc::clone(&self.shared);
        thread::Builder::new()
            .name(format!("orange-effect-{index}"))
            .spawn(move || work(&shared))
            .map(drop)
            .inspect_err(|_| self.shared.counts().threads -= 1)
    }
}

impl Shared {
    fn counts(&self) -> std::sync::MutexGuard<'_, Counts> {
        self.counts.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn work(shared: &Shared) {
    loop {
        shared.counts().idle += 1;
        // The lock is held only while waiting for one job, never while running it.
        let job = {
            let queue = shared.queue.lock().unwrap_or_else(PoisonError::into_inner);
            queue.recv_timeout(LINGER)
        };
        let mut counts = shared.counts();
        counts.idle -= 1;
        match job {
            Ok(job) => {
                counts.waiting = counts.waiting.saturating_sub(1);
                drop(counts);
                job();
            }
            // Idle beyond the core count: this thread is not needed now.
            Err(RecvTimeoutError::Timeout) if counts.threads > shared.core => {
                counts.threads -= 1;
                return;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                counts.threads -= 1;
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::sync_channel;
    use std::time::Instant;

    #[test]
    fn long_jobs_do_not_stall_short_ones() {
        let pool = Pool::new(2, 16, 64).unwrap();
        // Four jobs hold their threads until released, more than the core count.
        let (release, held) = channel::<()>();
        let held = Arc::new(Mutex::new(held));
        for _ in 0..4 {
            let held = Arc::clone(&held);
            pool.submit(move || {
                let _ = held.lock().unwrap().recv();
            })
            .unwrap();
        }
        let (done, finished) = sync_channel(1);
        pool.submit(move || done.send(()).unwrap()).unwrap();
        assert!(
            finished.recv_timeout(Duration::from_secs(5)).is_ok(),
            "a short job ran while long ones held their threads"
        );
        assert!(pool.threads() >= 5);
        assert!(pool.threads() <= 16);
        for _ in 0..4 {
            release.send(()).unwrap();
        }
    }

    #[test]
    fn the_pool_never_grows_past_its_maximum() {
        let pool = Pool::new(1, 3, 64).unwrap();
        let (release, held) = channel::<()>();
        let held = Arc::new(Mutex::new(held));
        for _ in 0..10 {
            let held = Arc::clone(&held);
            pool.submit(move || {
                let _ = held.lock().unwrap().recv();
            })
            .unwrap();
        }
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(200) {
            assert!(pool.threads() <= 3);
            thread::yield_now();
        }
        for _ in 0..10 {
            release.send(()).unwrap();
        }
    }

    /// Past its bound of waiting jobs the pool refuses new work at once, and takes work
    /// again once a thread has taken a job.
    #[test]
    fn a_full_queue_refuses_new_work_until_a_job_is_taken() {
        let pool = Pool::new(1, 1, 2).unwrap();
        let (release, held) = channel::<()>();
        let held = Arc::new(Mutex::new(held));
        let hold = || {
            let held = Arc::clone(&held);
            move || {
                let _ = held.lock().unwrap().recv();
            }
        };
        // The one thread takes the first job; two more wait.
        pool.submit(hold()).unwrap();
        let started = Instant::now();
        while pool.shared.counts().waiting > 0 {
            assert!(started.elapsed() < Duration::from_secs(5));
            thread::yield_now();
        }
        pool.submit(hold()).unwrap();
        pool.submit(hold()).unwrap();
        assert_eq!(pool.submit(|| {}), Err(Busy));
        release.send(()).unwrap();
        let started = Instant::now();
        while pool.shared.counts().waiting >= 2 {
            assert!(started.elapsed() < Duration::from_secs(5));
            thread::yield_now();
        }
        assert_eq!(pool.submit(|| {}), Ok(()));
        for _ in 0..3 {
            let _ = release.send(());
        }
    }

    /// Many submissions at once never start more threads than the pool's maximum.
    #[test]
    fn concurrent_submissions_never_pass_the_maximum() {
        let pool = Pool::new(1, 4, 1024).unwrap();
        let (release, held) = channel::<()>();
        let held = Arc::new(Mutex::new(held));
        let submitters: Vec<_> = (0..8)
            .map(|_| {
                let (pool, held) = (pool.clone(), Arc::clone(&held));
                thread::spawn(move || {
                    for _ in 0..16 {
                        let held = Arc::clone(&held);
                        pool.submit(move || {
                            let _ = held.lock().unwrap().recv_timeout(Duration::from_millis(1));
                        })
                        .unwrap();
                        assert!(pool.threads() <= 4);
                    }
                })
            })
            .collect();
        for submitter in submitters {
            submitter.join().unwrap();
        }
        assert!(pool.threads() <= 4);
        drop(release);
    }
}
