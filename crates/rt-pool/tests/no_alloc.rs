//! A batch must not allocate on any thread: this runs many batches under a counting allocator and
//! requires the count not to move once the pool is warm.
#![allow(unsafe_code)]

use soundcraft_rt_pool::Pool;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: forwards every call to the system allocator unchanged, only counting allocations.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        // SAFETY: same contract as the caller's.
        unsafe { System.dealloc(p, l) }
    }
}

#[global_allocator]
static A: Counting = Counting;

#[test]
fn running_batches_allocates_nothing() {
    let pool = Pool::new(4);
    let mut items = vec![0u64; 64];
    for _ in 0..50 {
        pool.run(&mut items, &|x| *x += 1).unwrap(); // warm up: thread start-up, first parks
    }
    std::thread::sleep(std::time::Duration::from_millis(50));
    let before = ALLOCS.load(Ordering::Relaxed);
    for _ in 0..2_000 {
        pool.run(&mut items, &|x| *x = x.wrapping_mul(3).wrapping_add(1)).unwrap();
    }
    let after = ALLOCS.load(Ordering::Relaxed);
    assert_eq!(after - before, 0, "{} allocations across 2000 batches", after - before);
}
