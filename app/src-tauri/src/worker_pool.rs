//! A small bounded pool for the blocking work of a request (the LLM call and
//! the tools). It replaces the thread-per-request spawn of PR 2b: a stuck
//! provider or adapter can now hold at most `workers` threads, and everything
//! beyond `workers + queue` is refused at once instead of piling up.
//!
//! The pool does not cancel anything: a call in flight is bounded by the
//! provider's own hard timeout (`OllamaProvider`), and a late result is
//! dropped by request id in the reducer.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

pub type Job = Box<dyn FnOnce() + Send + 'static>;

/// Worker threads (concurrent blocking calls) of the app's pool.
pub const WORKERS: usize = 4;
/// Jobs that may wait behind them.
pub const QUEUE: usize = 8;

/// The pool refused a job (every worker busy and the queue full). The job is
/// dropped unrun; the caller answers for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rejected;

pub struct WorkerPool {
    /// `None` only while dropping.
    sender: Option<SyncSender<Job>>,
    workers: Vec<JoinHandle<()>>,
}

impl WorkerPool {
    pub fn new(workers: usize, queue: usize) -> Self {
        let (sender, receiver) = sync_channel::<Job>(queue);
        let receiver: Arc<Mutex<Receiver<Job>>> = Arc::new(Mutex::new(receiver));
        let workers = (0..workers.max(1))
            .map(|n| {
                let receiver = receiver.clone();
                std::thread::Builder::new()
                    .name(format!("niva-worker-{n}"))
                    .spawn(move || run(&receiver))
                    .expect("spawn a worker thread")
            })
            .collect();
        WorkerPool {
            sender: Some(sender),
            workers,
        }
    }

    /// Queues `job`, or refuses it if the queue is full.
    pub fn submit(&self, job: Job) -> Result<(), Rejected> {
        let Some(sender) = &self.sender else {
            return Err(Rejected);
        };
        sender
            .try_send(job)
            .map_err(|full: TrySendError<Job>| match full {
                TrySendError::Full(_) | TrySendError::Disconnected(_) => Rejected,
            })
    }
}

/// One worker: takes the next job (holding the lock only while waiting for
/// it), runs it, survives a panic in it.
fn run(receiver: &Mutex<Receiver<Job>>) {
    loop {
        let next = {
            let guard = receiver
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.recv()
        };
        match next {
            Ok(job) => {
                let _ = catch_unwind(AssertUnwindSafe(job));
            }
            Err(_) => return,
        }
    }
}

impl Drop for WorkerPool {
    /// Closes the queue, lets the queued jobs finish and joins the workers.
    fn drop(&mut self) {
        drop(self.sender.take());
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{channel, sync_channel};
    use std::sync::{Arc, Mutex};

    use super::*;

    /// A job that says it started and then waits to be released.
    fn blocker() -> (
        Job,
        std::sync::mpsc::Receiver<()>,
        std::sync::mpsc::SyncSender<()>,
    ) {
        let (started_tx, started_rx) = channel();
        let (release_tx, release_rx) = sync_channel(1);
        let job: Job = Box::new(move || {
            let _ = started_tx.send(());
            let _ = release_rx.recv();
        });
        (job, started_rx, release_tx)
    }

    #[test]
    fn a_submitted_job_runs() {
        let pool = WorkerPool::new(1, 4);
        let (tx, rx) = channel();
        assert!(pool.submit(Box::new(move || tx.send(7).unwrap())).is_ok());
        assert_eq!(rx.recv().unwrap(), 7);
    }

    #[test]
    fn jobs_run_in_submission_order_on_one_worker() {
        let pool = WorkerPool::new(1, 8);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (done_tx, done_rx) = channel();
        for n in 0..5 {
            let seen = seen.clone();
            let done = done_tx.clone();
            pool.submit(Box::new(move || {
                seen.lock().unwrap().push(n);
                let _ = done.send(());
            }))
            .unwrap();
        }
        for _ in 0..5 {
            done_rx.recv().unwrap();
        }
        assert_eq!(*seen.lock().unwrap(), [0, 1, 2, 3, 4]);
    }

    #[test]
    fn the_pool_never_runs_more_jobs_than_it_has_workers() {
        let pool = WorkerPool::new(2, 8);
        let (a, a_started, a_release) = blocker();
        let (b, b_started, b_release) = blocker();
        let (c, c_started, c_release) = blocker();
        pool.submit(a).unwrap();
        pool.submit(b).unwrap();
        pool.submit(c).unwrap();
        a_started.recv().unwrap();
        b_started.recv().unwrap();
        assert!(c_started.try_recv().is_err(), "a third job ran at once");
        a_release.send(()).unwrap();
        c_started.recv().unwrap();
        b_release.send(()).unwrap();
        c_release.send(()).unwrap();
    }

    #[test]
    fn a_full_queue_refuses_the_job_and_never_runs_it() {
        let pool = WorkerPool::new(1, 1);
        let (running, running_started, release) = blocker();
        pool.submit(running).unwrap();
        running_started.recv().unwrap();
        // One job fits in the queue behind the running one.
        assert!(pool.submit(Box::new(|| {})).is_ok());
        let ran = Arc::new(Mutex::new(false));
        let flag = ran.clone();
        let rejected = pool.submit(Box::new(move || *flag.lock().unwrap() = true));
        assert_eq!(rejected, Err(Rejected), "the queue is full");
        release.send(()).unwrap();
        drop(pool); // finishes the queued job; the refused one is gone
        assert!(!*ran.lock().unwrap(), "a refused job must never run");
    }

    #[test]
    fn a_panicking_job_does_not_kill_the_worker() {
        let pool = WorkerPool::new(1, 4);
        pool.submit(Box::new(|| panic!("a hung adapter blew up")))
            .unwrap();
        let (tx, rx) = channel();
        pool.submit(Box::new(move || tx.send(()).unwrap())).unwrap();
        rx.recv().expect("the worker survived");
    }

    #[test]
    fn dropping_the_pool_finishes_queued_jobs_and_stops_the_workers() {
        let (tx, rx) = channel();
        {
            let pool = WorkerPool::new(1, 4);
            for n in 0..3 {
                let tx = tx.clone();
                pool.submit(Box::new(move || tx.send(n).unwrap())).unwrap();
            }
        }
        drop(tx);
        // The channel closes only when every job (and so every sender clone) is gone.
        assert_eq!(rx.iter().collect::<Vec<_>>(), [0, 1, 2]);
    }
}
