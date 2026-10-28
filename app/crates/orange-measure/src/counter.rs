//! Counting global allocator: the only unsafe code in this crate.
//!
//! Every call forwards unchanged to [`System`] and then updates relaxed atomics. The
//! harness is single-threaded, so relaxed ordering is enough to read consistent totals
//! between workload phases. A `realloc` counts as one allocation of its new size.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);
static LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);
static PEAK_LIVE_BYTES: AtomicUsize = AtomicUsize::new(0);

pub struct CountingAllocator;

fn allocated(size: usize) {
    ALLOCATIONS.fetch_add(1, Relaxed);
    ALLOCATED_BYTES.fetch_add(size as u64, Relaxed);
    let live = LIVE_BYTES.fetch_add(size, Relaxed) + size;
    PEAK_LIVE_BYTES.fetch_max(live, Relaxed);
}

fn released(size: usize) {
    LIVE_BYTES.fetch_sub(size, Relaxed);
}

#[allow(unsafe_code)]
// SAFETY: each method passes the caller's pointer and layout unchanged to `System`, which
// implements the `GlobalAlloc` contract, and returns its result unchanged. The bookkeeping
// only touches static atomics and never allocates, so it cannot re-enter the allocator.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller guarantees `layout` has a non-zero size, as `System` requires.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            allocated(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller guarantees `layout` has a non-zero size, as `System` requires.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            allocated(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the caller guarantees `pointer` came from this allocator (hence from
        // `System`) with this same `layout`.
        unsafe { System.dealloc(pointer, layout) };
        released(layout.size());
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller guarantees `pointer` came from this allocator with `layout`,
        // and that `new_size` is non-zero and valid for `layout.align()`.
        let moved = unsafe { System.realloc(pointer, layout, new_size) };
        if !moved.is_null() {
            released(layout.size());
            allocated(new_size);
        }
        moved
    }
}

/// Counter readings since the last [`Snapshot::start`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub allocations: u64,
    pub allocated_bytes: u64,
    /// Highest live heap above the live heap at the start of the window.
    pub peak_live_bytes: usize,
}

/// A measurement window. Starting one resets the peak to the current live heap, so
/// memory owned before the window (plans, documents) is excluded from its peak.
pub struct Snapshot {
    allocations: u64,
    allocated_bytes: u64,
    live_bytes: usize,
}

impl Snapshot {
    pub fn start() -> Self {
        let live_bytes = LIVE_BYTES.load(Relaxed);
        PEAK_LIVE_BYTES.store(live_bytes, Relaxed);
        Self {
            allocations: ALLOCATIONS.load(Relaxed),
            allocated_bytes: ALLOCATED_BYTES.load(Relaxed),
            live_bytes,
        }
    }

    pub fn finish(&self) -> Counts {
        Counts {
            allocations: ALLOCATIONS.load(Relaxed) - self.allocations,
            allocated_bytes: ALLOCATED_BYTES.load(Relaxed) - self.allocated_bytes,
            peak_live_bytes: PEAK_LIVE_BYTES
                .load(Relaxed)
                .saturating_sub(self.live_bytes),
        }
    }
}
