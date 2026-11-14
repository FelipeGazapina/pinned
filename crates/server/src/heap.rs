//! Counts the bytes the hub holds on the heap, for the census: live bytes next to the resident
//! set tell memory the hub holds from memory the allocator keeps after the hub freed it.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Counting;

static LIVE: AtomicU64 = AtomicU64::new(0);
static LIVE_BIG: AtomicU64 = AtomicU64::new(0);
static ALLOCATED: AtomicU64 = AtomicU64::new(0);
/// Allocations of at least this many bytes are counted on their own as well.
const BIG: usize = 1 << 20;

fn grew(size: usize) {
    LIVE.fetch_add(size as u64, Ordering::Relaxed);
    ALLOCATED.fetch_add(size as u64, Ordering::Relaxed);
    if size >= BIG {
        LIVE_BIG.fetch_add(size as u64, Ordering::Relaxed);
    }
}

fn shrank(size: usize) {
    LIVE.fetch_sub(size as u64, Ordering::Relaxed);
    if size >= BIG {
        LIVE_BIG.fetch_sub(size as u64, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        System.dealloc(p, layout);
        shrank(layout.size());
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new: usize) -> *mut u8 {
        let q = System.realloc(p, layout, new);
        if !q.is_null() {
            shrank(layout.size());
            grew(new);
        }
        q
    }
}

/// `heap_live_mb`, `heap_big_live_mb` (allocations of 1 MB and more) and `heap_allocated_gb` since start.
pub fn census() -> String {
    format!(
        " heap_live_mb={} heap_big_live_mb={} heap_allocated_gb={}",
        LIVE.load(Ordering::Relaxed) >> 20,
        LIVE_BIG.load(Ordering::Relaxed) >> 20,
        ALLOCATED.load(Ordering::Relaxed) >> 30
    )
}
