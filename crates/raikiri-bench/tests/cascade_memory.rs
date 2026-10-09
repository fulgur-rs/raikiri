//! Allocation volume and peak of one cascade over the report document.
//!
//! A counting global allocator records the allocations made while the
//! cascade runs: how many and how many bytes, the largest single block, the
//! peak of live bytes above the level before the call, and what the result
//! keeps alive. The ceilings sit well above the current values, so they catch
//! a gross regression without pinning today's exact numbers. This file holds
//! a single test, so no other test allocates while it measures.

#[path = "../benches/common/report.rs"]
mod report;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static TOTAL: AtomicUsize = AtomicUsize::new(0);
static COUNT: AtomicUsize = AtomicUsize::new(0);
static LARGEST: AtomicUsize = AtomicUsize::new(0);

fn record(size: usize) {
    let live = LIVE.fetch_add(size, Relaxed) + size;
    PEAK.fetch_max(live, Relaxed);
    TOTAL.fetch_add(size, Relaxed);
    COUNT.fetch_add(1, Relaxed);
    LARGEST.fetch_max(size, Relaxed);
}

// Counting allocations needs a `GlobalAlloc`, whose methods are unsafe to
// implement; nothing else in this file uses unsafe code.
// SAFETY: every method forwards its arguments unchanged to `System` and only
// updates counters on the side, so `System`'s guarantees carry over.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            record(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::alloc_zeroed`'s contract.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            record(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` was returned by this allocator for `layout`.
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::realloc`'s contract.
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            LIVE.fetch_sub(layout.size(), Relaxed);
            record(new_size);
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Allocator activity during one measured call.
#[derive(Debug)]
struct Usage {
    allocations: usize,
    bytes: usize,
    largest_block: usize,
    peak_live: usize,
    retained: usize,
}

fn measure<T>(f: impl FnOnce() -> T) -> (T, Usage) {
    let base = LIVE.load(Relaxed);
    PEAK.store(base, Relaxed);
    LARGEST.store(0, Relaxed);
    let (count, total) = (COUNT.load(Relaxed), TOTAL.load(Relaxed));
    let value = f();
    let usage = Usage {
        allocations: COUNT.load(Relaxed) - count,
        bytes: TOTAL.load(Relaxed) - total,
        largest_block: LARGEST.load(Relaxed),
        peak_live: PEAK.load(Relaxed) - base,
        retained: LIVE.load(Relaxed).saturating_sub(base),
    };
    (value, usage)
}

#[test]
fn cascade_allocations_stay_under_their_ceilings() {
    let html = report::report_html(10, 50);
    let options = raikiri_html::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let document = raikiri_html::parse(html.as_bytes(), &options).expect("the report parses");
    let tree = raikiri_html::build_rule_tree(&document);
    let media = raikiri_style::MediaContext::print();
    let (result, usage) = measure(|| {
        raikiri_style::cascade_with_media_context(&document.dom, &tree, &media)
            .expect("the cascade succeeds")
    });
    let nodes = result.computed.len();
    drop(result);
    println!("cascade of {nodes} nodes: {usage:?}");
    assert!(usage.allocations > 0 && usage.retained > 0, "{usage:?}");
    for (what, value, ceiling) in [
        ("allocations", usage.allocations, ALLOCATIONS_CEILING),
        ("allocated bytes", usage.bytes, BYTES_CEILING),
        ("largest block", usage.largest_block, LARGEST_BLOCK_CEILING),
        ("peak live bytes", usage.peak_live, PEAK_CEILING),
        ("retained bytes", usage.retained, RETAINED_CEILING),
    ] {
        assert!(
            value <= ceiling,
            "{what}: {value} exceeds {ceiling} for {nodes} nodes ({usage:?})"
        );
    }
}

// About 1.5 times what a debug build used when the ceilings were set, for
// 6,397 nodes: 198,928 allocations, 68.2 MB allocated, a 21 MiB largest block
// (the candidate array of the whole document), a 37.1 MB peak and 14.9 MB
// retained by the result. The largest-block ceiling allows no further
// doubling of that array.
const ALLOCATIONS_CEILING: usize = 300_000;
const BYTES_CEILING: usize = 100_000_000;
const LARGEST_BLOCK_CEILING: usize = 32 << 20;
const PEAK_CEILING: usize = 56_000_000;
const RETAINED_CEILING: usize = 22_000_000;
