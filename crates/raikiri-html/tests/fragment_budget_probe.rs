//! Reproducible opt-in allocation and timing comparison for fragment parsing.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

struct CountingAllocator;
static COUNTING: AtomicBool = AtomicBool::new(false);
static CALLS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

// SAFETY: All operations delegate unchanged to the system allocator. Counters
// only observe requested sizes and do not inspect or alter allocated storage.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            CALLS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        // SAFETY: The caller supplies the required layout.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: The caller supplies a live allocation with its matching layout.
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            CALLS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(size as u64, Ordering::Relaxed);
        }
        // SAFETY: The caller supplies a live allocation with its matching layout.
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
#[ignore = "allocation/timing probe; run alone with --nocapture --test-threads=1"]
fn fragment_budget_allocation_probe() {
    let count = std::env::var("FRAGMENT_PROBE_ELEMENTS")
        .unwrap_or("100000".into())
        .parse::<usize>()
        .unwrap();
    let input = "<b>x</b>".repeat(count);
    let budgets = match std::env::var("FRAGMENT_PROBE_MODE").as_deref() {
        Ok("bounded") => vec![Some(1)],
        Ok("unlimited") => vec![None],
        _ => vec![None, Some(1)],
    };
    for remaining in budgets {
        let mut limits = raikiri_traits::RenderLimits::default();
        limits.max_dom_nodes = None;
        CALLS.store(0, Ordering::Relaxed);
        BYTES.store(0, Ordering::Relaxed);
        COUNTING.store(true, Ordering::Relaxed);
        let started = Instant::now();
        let result = raikiri_html::parse_fragment_with_limits(
            input.as_bytes(),
            &raikiri_html::ParseOptions::new(),
            "div",
            "http://www.w3.org/1999/xhtml",
            true,
            &limits,
            remaining,
        );
        let elapsed = started.elapsed();
        COUNTING.store(false, Ordering::Relaxed);
        let nodes = result
            .as_ref()
            .ok()
            .map(|document| document.dom.node_count());
        println!(
            "elements={count} remaining={remaining:?} elapsed_us={} allocation_calls={} requested_bytes={} nodes={nodes:?} error={:?}",
            elapsed.as_micros(),
            CALLS.load(Ordering::Relaxed),
            BYTES.load(Ordering::Relaxed),
            result.err()
        );
    }
}
