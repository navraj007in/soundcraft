//! The shared job: one batch of items that the caller and the pool's workers claim one at a time.
//!
//! This is the only place a lifetime is erased. The public API is safe because of one protocol,
//! enforced by [`Job::run`] being the only way in:
//!
//! 1. `run` takes the `busy` flag (so only one batch is ever in flight), stores the batch's base
//!    pointer, closure and call function, then publishes `(generation, len, next = 0)` with a
//!    release store.
//! 2. A thread claims item `i` only by a compare-exchange on that cursor that moves `next` from `i`
//!    to `i + 1`, so each index goes to exactly one thread, and only while `next < len`.
//! 3. A claim is a [`Claim`], which only [`Job::claim`] can make. Running it reads the batch
//!    description (published before the cursor, so visible after the claiming acquire), calls the
//!    closure on item `i`, then decrements `remaining` with release.
//! 4. `run` does not return until `remaining` is zero (acquire): every claimed item has finished,
//!    and no unclaimed item remains (`next == len`), so no thread can touch the borrowed items or
//!    the closure after `run` returns. A late worker that loads the cursor afterwards sees
//!    `next >= len` and does nothing; one that sees a *newer* batch simply helps with that one.
//!
//! The 20-bit generation in the cursor makes a stale compare-exchange fail across batches (a thread
//! would have to sleep through 2^20 batches between its load and its exchange to be fooled).

#![allow(unsafe_code)]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};

const NEXT_BITS: u32 = 22;
const LEN_BITS: u32 = 22;
const MASK: u64 = (1 << 22) - 1;
const GEN_MASK: u64 = (1 << 20) - 1;

/// Most items one batch may hold.
pub const MAX_ITEMS: usize = (1 << NEXT_BITS) - 1;

fn pack(generation: u64, len: usize, next: usize) -> u64 {
    ((generation & GEN_MASK) << (NEXT_BITS + LEN_BITS)) | ((len as u64 & MASK) << NEXT_BITS) | (next as u64 & MASK)
}

fn unpack(c: u64) -> (u64, usize, usize) {
    (c >> (NEXT_BITS + LEN_BITS), ((c >> NEXT_BITS) & MASK) as usize, (c & MASK) as usize)
}

type Call = unsafe fn(*mut (), *const (), usize);

/// Why a batch did not run to completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunError {
    /// Another batch is already in flight on this pool; nothing was touched.
    Busy,
    /// The closure panicked on some item (the others still ran).
    Panicked,
}

pub struct Job {
    cursor: AtomicU64,
    remaining: AtomicUsize,
    busy: AtomicBool,
    panicked: AtomicBool,
    base: AtomicPtr<()>,
    holder: AtomicPtr<()>,
    call: AtomicPtr<()>,
}

/// The right to run one item of the batch that was current when it was made.
pub struct Claim {
    index: usize,
}

/// Calls `f` on item `i` of the batch at `base`.
///
/// # Safety
/// `base` points to a live `[T]` with more than `i` items, `holder` to a live `&(dyn Fn(&mut T) +
/// Sync)`, and no other thread is using item `i`.
unsafe fn call<T>(base: *mut (), holder: *const (), i: usize) {
    // SAFETY: by the contract above; `holder` was made from a `&&(dyn Fn(&mut T) + Sync)` of this `T`.
    let f: &(dyn Fn(&mut T) + Sync) = unsafe { *(holder as *const &(dyn Fn(&mut T) + Sync)) };
    // SAFETY: `i` is in bounds and claimed by exactly one thread, so this `&mut` is unique.
    let item = unsafe { &mut *(base as *mut T).add(i) };
    f(item);
}

impl Job {
    pub fn new() -> Job {
        Job {
            cursor: AtomicU64::new(pack(0, 0, 0)),
            remaining: AtomicUsize::new(0),
            busy: AtomicBool::new(false),
            panicked: AtomicBool::new(false),
            base: AtomicPtr::new(std::ptr::null_mut()),
            holder: AtomicPtr::new(std::ptr::null_mut()),
            call: AtomicPtr::new(std::ptr::null_mut()),
        }
    }

    /// Whether an item is waiting to be claimed.
    pub fn has_work(&self) -> bool {
        let (_, len, next) = unpack(self.cursor.load(Ordering::Acquire));
        next < len
    }

    /// Take the next unclaimed item of the current batch, if any.
    pub fn claim(&self) -> Option<Claim> {
        let mut c = self.cursor.load(Ordering::Acquire);
        loop {
            let (generation, len, next) = unpack(c);
            if next >= len {
                return None;
            }
            match self.cursor.compare_exchange_weak(c, pack(generation, len, next + 1), Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Some(Claim { index: next }),
                Err(now) => c = now,
            }
        }
    }

    /// Run a claimed item. A panic in the closure is caught and reported by [`Job::run`].
    pub fn execute(&self, claim: Claim) {
        let base = self.base.load(Ordering::Acquire);
        let holder = self.holder.load(Ordering::Acquire) as *const ();
        let call_ptr = self.call.load(Ordering::Acquire);
        // SAFETY: `call_ptr` was stored by `run` from a `Call` fn pointer before the cursor was
        // published, and we hold a claim, so that batch is still in flight (`run` cannot return
        // before our `fetch_sub` below).
        let call: Call = unsafe { std::mem::transmute::<*mut (), Call>(call_ptr) };
        // SAFETY: the batch is in flight (see above), `claim.index < len`, and the claim is unique.
        let outcome = catch_unwind(AssertUnwindSafe(|| unsafe { call(base, holder, claim.index) }));
        if outcome.is_err() {
            self.panicked.store(true, Ordering::Relaxed);
        }
        self.remaining.fetch_sub(1, Ordering::Release);
    }

    /// Run `f` on every item, sharing the work with whoever calls [`Job::claim`] meanwhile.
    /// `wake` is called once after the batch is published (to unpark workers). Returns when every
    /// item is done. No allocation, no locking.
    pub fn run<T: Send>(&self, items: &mut [T], f: &(dyn Fn(&mut T) + Sync), wake: impl FnOnce()) -> Result<(), RunError> {
        let len = items.len();
        if len == 0 {
            return Ok(());
        }
        if len > MAX_ITEMS {
            return Err(RunError::Busy);
        }
        if self.busy.swap(true, Ordering::Acquire) {
            return Err(RunError::Busy);
        }
        let holder: &&(dyn Fn(&mut T) + Sync) = &f;
        self.base.store(items.as_mut_ptr() as *mut (), Ordering::Relaxed);
        self.holder.store(holder as *const &(dyn Fn(&mut T) + Sync) as *mut (), Ordering::Relaxed);
        self.call.store(call::<T> as unsafe fn(*mut (), *const (), usize) as *mut (), Ordering::Relaxed);
        self.panicked.store(false, Ordering::Relaxed);
        self.remaining.store(len, Ordering::Relaxed);
        let generation = unpack(self.cursor.load(Ordering::Relaxed)).0 + 1;
        self.cursor.store(pack(generation, len, 0), Ordering::Release);
        // From here workers may touch `items` and `f`; the guard makes sure none still does when this
        // frame ends, even by unwinding (a panicking `wake`).
        let mut guard = Finish { job: self, done: false };
        wake();
        while let Some(claim) = self.claim() {
            self.execute(claim);
        }
        let panicked = guard.finish();
        if panicked { Err(RunError::Panicked) } else { Ok(()) }
    }
}

/// Ends a batch: nobody may use its borrowed data once this has run.
struct Finish<'a> {
    job: &'a Job,
    done: bool,
}

impl Finish<'_> {
    /// Wait for the items workers are still running, free the pool for the next batch, and say
    /// whether any item panicked.
    fn finish(&mut self) -> bool {
        // Items still running on workers: wait for them. Spin briefly, then yield the core.
        let mut spins = 0u32;
        while self.job.remaining.load(Ordering::Acquire) != 0 {
            spins = spins.saturating_add(1);
            if spins < 2_000 {
                std::hint::spin_loop();
            } else {
                std::thread::yield_now();
            }
        }
        let panicked = self.job.panicked.load(Ordering::Relaxed);
        self.job.busy.store(false, Ordering::Release);
        self.done = true;
        panicked
    }
}

impl Drop for Finish<'_> {
    /// Only reached before `finish` when the caller is unwinding: abandon the items nobody has
    /// claimed (counting them off so the wait ends), then wait for the ones in flight.
    fn drop(&mut self) {
        if !self.done {
            while self.job.claim().is_some() {
                self.job.remaining.fetch_sub(1, Ordering::Release);
            }
            self.job.panicked.store(true, Ordering::Relaxed);
            self.finish();
        }
    }
}

impl Default for Job {
    fn default() -> Self {
        Job::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    /// Helper threads that claim and run items, as pool workers do.
    fn workers(job: &Arc<Job>, n: usize, stop: &Arc<AtomicBool>) -> Vec<std::thread::JoinHandle<()>> {
        (0..n)
            .map(|_| {
                let (job, stop) = (Arc::clone(job), Arc::clone(stop));
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        match job.claim() {
                            Some(c) => job.execute(c),
                            None => std::hint::spin_loop(),
                        }
                    }
                })
            })
            .collect()
    }

    #[test]
    fn a_panic_while_waking_workers_leaves_nothing_using_the_callers_data() {
        let job = Arc::new(Job::new());
        let stop = Arc::new(AtomicBool::new(false));
        let handles = workers(&job, 3, &stop);
        let touched = AtomicUsize::new(0);
        {
            // These live only inside this block: if a worker used them afterwards, Miri would say so.
            let mut items = vec![0u64; 200];
            let f = |x: &mut u64| {
                *x += 1;
                touched.fetch_add(1, Ordering::Relaxed);
                std::thread::sleep(std::time::Duration::from_micros(50));
            };
            let r = catch_unwind(AssertUnwindSafe(|| job.run(&mut items, &f, || std::panic::resume_unwind(Box::new("wake failed")))));
            assert!(r.is_err(), "the panic propagates to the caller");
        }
        // The pool is free again and works.
        let mut again = vec![0u8; 16];
        assert_eq!(job.run(&mut again, &|x| *x = 9, || {}), Ok(()));
        assert!(again.iter().all(|&x| x == 9));
        stop.store(true, Ordering::Release);
        for h in handles {
            h.join().unwrap();
        }
    }
}
