//! A pool of persistent worker threads for the audio callback.
//!
//! The callback hands the pool a slice of independent items (a block's strips) and a closure.
//! [`Pool::run`] publishes the batch, wakes the workers, and **works through it itself** alongside
//! them, so the callback never waits for a thread that has not started: workers only ever take items
//! the caller has not. The one thing it can still wait for is a worker that is part-way through an
//! item (the OS may deschedule it there). Measured under heavy CPU contention that bounds a block's
//! worst case near the serial one, where the shared rayon pool's was several times worse.
//!
//! Steady-state cost per batch: a few atomic operations and one `unpark` per woken worker. No
//! allocation, no locks, no channels. Threads are created by [`Pool::new`], so build the pool off the
//! audio thread.
//!
//! This is an isolated `unsafe` crate (like the plugin hosts): `unsafe` is denied crate-wide and
//! allowed only in `job` (one erased-lifetime batch, with its protocol documented there). The public
//! API is safe and never panics.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod job;

pub use job::{MAX_ITEMS, RunError};

use job::Job;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

struct Shared {
    job: Job,
    shutdown: AtomicBool,
}

/// Persistent worker threads plus the calling thread.
pub struct Pool {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
}

/// How many helper threads to start on this machine: leave two cores for the rest of the system, at
/// most eight.
pub fn default_workers() -> usize {
    std::thread::available_parallelism().map_or(0, |n| n.get().saturating_sub(2)).min(8)
}

impl Pool {
    /// Start `workers` helper threads (fewer if the system refuses some). With none, [`Pool::run`]
    /// simply runs everything on the caller.
    pub fn new(workers: usize) -> Pool {
        let shared = Arc::new(Shared { job: Job::new(), shutdown: AtomicBool::new(false) });
        let threads = (0..workers)
            .filter_map(|i| {
                let s = Arc::clone(&shared);
                std::thread::Builder::new().name(format!("soundcraft-rt-{i}")).spawn(move || worker(&s)).ok()
            })
            .collect();
        Pool { shared, threads }
    }

    /// Helper threads running (the caller makes one more participant).
    pub fn workers(&self) -> usize {
        self.threads.len()
    }

    /// Run `f` on every item, in parallel with the pool's workers, and return when all are done.
    /// `Err(Busy)` (nothing run) if another batch is in flight; `Err(Panicked)` if `f` panicked on
    /// some item (the rest still ran).
    pub fn run<T: Send>(&self, items: &mut [T], f: &(dyn Fn(&mut T) + Sync)) -> Result<(), RunError> {
        let helpers = self.threads.len().min(items.len().saturating_sub(1));
        self.shared.job.run(items, f, || {
            for t in self.threads.iter().take(helpers) {
                t.thread().unpark();
            }
        })
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Release);
        for t in &self.threads {
            t.thread().unpark();
        }
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

fn worker(shared: &Shared) {
    loop {
        while let Some(claim) = shared.job.claim() {
            shared.job.execute(claim);
        }
        if shared.shutdown.load(Ordering::Acquire) {
            return;
        }
        // Batches arrive every few milliseconds: spin briefly in case the next one is imminent,
        // then sleep until the caller unparks us (a token left by an earlier unpark makes `park`
        // return at once, so a wakeup is never lost).
        let mut spins = 0u32;
        while !shared.job.has_work() && !shared.shutdown.load(Ordering::Acquire) {
            spins += 1;
            if spins < 1_000 {
                std::hint::spin_loop();
            } else {
                std::thread::park();
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn every_item_runs_exactly_once_and_results_are_visible() {
        let pool = Pool::new(4);
        for round in 0..200usize {
            let n = 1 + round % 97;
            let mut items: Vec<(usize, u64)> = (0..n).map(|i| (0, i as u64)).collect();
            pool.run(&mut items, &|it| {
                it.0 += 1;
                it.1 = it.1 * 2 + 1;
            })
            .unwrap();
            for (i, it) in items.iter().enumerate() {
                assert_eq!(it.0, 1, "item {i} of round {round} ran {} times", it.0);
                assert_eq!(it.1, i as u64 * 2 + 1);
            }
        }
    }

    #[test]
    fn work_is_shared_with_the_workers() {
        let pool = Pool::new(3);
        let ids = std::sync::Mutex::new(std::collections::HashSet::new());
        let mut items = vec![(); 400];
        pool.run(&mut items, &|_| {
            ids.lock().map(|mut s| s.insert(std::thread::current().id())).ok();
            std::thread::sleep(std::time::Duration::from_micros(200));
        })
        .unwrap();
        assert!(ids.lock().map(|s| s.len()).unwrap_or(0) > 1, "more than one thread took part");
    }

    #[test]
    fn a_pool_with_no_workers_runs_on_the_caller() {
        let pool = Pool::new(0);
        let mut items = vec![1u32; 50];
        pool.run(&mut items, &|x| *x += 1).unwrap();
        assert!(items.iter().all(|&x| x == 2));
        assert_eq!(pool.run(&mut Vec::<u32>::new(), &|_| {}), Ok(()));
    }

    #[test]
    fn a_batch_started_inside_a_batch_is_refused_not_run() {
        let pool = Pool::new(2);
        let inner = AtomicUsize::new(0);
        let mut items = vec![0u8; 8];
        pool.run(&mut items, &|_| {
            let mut other = vec![0u8; 4];
            if pool.run(&mut other, &|x| *x = 1) == Err(RunError::Busy) {
                inner.fetch_add(1, Ordering::Relaxed);
            }
        })
        .unwrap();
        assert_eq!(inner.load(Ordering::Relaxed), 8);
    }

    #[test]
    fn a_panicking_item_is_reported_and_the_pool_keeps_working() {
        let pool = Pool::new(3);
        let mut items: Vec<u32> = (0..64).collect();
        let r = pool.run(&mut items, &|x| {
            if *x == 13 {
                std::panic::resume_unwind(Box::new("boom"));
            }
            *x += 1000;
        });
        assert_eq!(r, Err(RunError::Panicked));
        assert_eq!(items.iter().filter(|&&x| x >= 1000).count(), 63, "every other item still ran");
        let mut again = vec![0u32; 64];
        pool.run(&mut again, &|x| *x = 7).unwrap();
        assert!(again.iter().all(|&x| x == 7));
    }

    #[test]
    fn dropping_the_pool_stops_its_threads() {
        let pool = Pool::new(4);
        assert_eq!(pool.workers(), 4);
        drop(pool);
    }

    #[test]
    fn many_small_batches_from_the_caller_never_lose_or_repeat_an_item() {
        let pool = Pool::new(default_workers().max(2));
        let total = AtomicUsize::new(0);
        let mut expected = 0usize;
        for round in 0..5_000usize {
            let n = round % 33;
            expected += n;
            let mut items = vec![0u8; n];
            pool.run(&mut items, &|_| {
                total.fetch_add(1, Ordering::Relaxed);
            })
            .unwrap();
        }
        assert_eq!(total.load(Ordering::Relaxed), expected);
    }
}
