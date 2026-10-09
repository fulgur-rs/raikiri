//! Allocation volume and peak of one cascade over the report document.
//!
//! A counting global allocator records the allocations the measuring thread
//! makes while the cascade runs: how many and how many bytes, the largest
//! single block, the peak of live bytes, and what the result keeps alive. The
//! ceilings sit well above the current values, so they catch a gross
//! regression without pinning today's exact numbers.

#[path = "../benches/common/report.rs"]
mod report;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering::Relaxed};

struct Counting;

thread_local! {
    // Constant-initialized and without a destructor, so reading it neither
    // allocates nor fails while a thread is being torn down.
    static MEASURING: Cell<bool> = const { Cell::new(false) };
}

static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
static COUNT: AtomicUsize = AtomicUsize::new(0);
static LARGEST: AtomicUsize = AtomicUsize::new(0);

fn measuring() -> bool {
    MEASURING.try_with(Cell::get).unwrap_or(false)
}

fn record_allocation(size: usize) {
    if measuring() {
        let size_delta = isize::try_from(size).unwrap_or(isize::MAX);
        let live = LIVE
            .fetch_add(size_delta, Relaxed)
            .saturating_add(size_delta);
        PEAK.fetch_max(live, Relaxed);
        BYTES.fetch_add(size, Relaxed);
        COUNT.fetch_add(1, Relaxed);
        LARGEST.fetch_max(size, Relaxed);
    }
}

fn record_release(size: usize) {
    if measuring() {
        LIVE.fetch_sub(isize::try_from(size).unwrap_or(isize::MAX), Relaxed);
    }
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
            record_allocation(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::alloc_zeroed`'s contract.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            record_allocation(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` was returned by this allocator for `layout`.
        unsafe { System.dealloc(ptr, layout) };
        record_release(layout.size());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::realloc`'s contract.
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            record_release(layout.size());
            record_allocation(new_size);
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Allocator activity of the measuring thread during one call.
///
/// Every successful `realloc` counts as releasing the old block and making
/// one allocation of the new size, so `bytes` includes each growth step of a
/// vector, and `peak_live` treats a `realloc` as in place: the moment the old
/// and the new block coexist is not counted. Memory released during the call
/// that was allocated before it lowers the live count.
#[derive(Debug)]
struct Usage {
    allocations: usize,
    bytes: usize,
    largest_block: usize,
    peak_live: usize,
    retained: usize,
}

fn measure<T>(f: impl FnOnce() -> T) -> (T, Usage) {
    for counter in [&LIVE, &PEAK] {
        counter.store(0, Relaxed);
    }
    for counter in [&BYTES, &COUNT, &LARGEST] {
        counter.store(0, Relaxed);
    }
    MEASURING.set(true);
    let value = f();
    MEASURING.set(false);
    let usage = Usage {
        allocations: COUNT.load(Relaxed),
        bytes: BYTES.load(Relaxed),
        largest_block: LARGEST.load(Relaxed),
        peak_live: usize::try_from(PEAK.load(Relaxed)).unwrap_or(0),
        retained: usize::try_from(LIVE.load(Relaxed)).unwrap_or(0),
    };
    (value, usage)
}

#[test]
fn cascade_allocations_stay_under_their_ceilings() {
    let (pages, rows) = (10, 50);
    let html = report::report_html(pages, rows);
    let options = raikiri_html::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let document = raikiri_html::parse(html.as_bytes(), &options).expect("the report parses");
    let tree = raikiri_html::build_rule_tree(&document);
    let media = raikiri_style::MediaContext::print();
    let cascade = || {
        raikiri_style::cascade_with_media_context(&document.dom, &tree, &media)
            .expect("the cascade succeeds")
    };
    // A first cascade fills the process-wide caches that every later one
    // reuses, so they stay out of the measurement.
    drop(cascade());
    let (result, usage) = measure(cascade);
    report::check_report(&result, pages, rows);
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

// Set from what the cascade used when the ceilings were introduced, for
// 6,397 nodes: 201,944 allocations, 69.4 MB allocated, a 21 MiB largest block
// (the array of every candidate declaration in the document), a 37.1 MB peak
// and 14.9 MB retained by the result. That array grows by
// doubling, so allocated bytes, the peak and the largest block move in steps:
// they stay flat while the candidate count grows by up to about 1.7 times,
// then all three pass their ceilings at once. Raise a ceiling together with a
// measurement that explains the growth, and lower them when the candidate
// storage shrinks.
const ALLOCATIONS_CEILING: usize = 300_000;
const BYTES_CEILING: usize = 100_000_000;
const LARGEST_BLOCK_CEILING: usize = 32 << 20;
const PEAK_CEILING: usize = 56_000_000;
const RETAINED_CEILING: usize = 22_000_000;
