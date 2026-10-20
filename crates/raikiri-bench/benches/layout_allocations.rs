//! Allocation traffic during fresh layout, with parsing, cascade and drops excluded.
//! Run separately from Criterion: allocator instrumentation distorts elapsed time.

mod common {
    pub mod workloads;
}

use common::workloads::{self, Shape};
use raikiri::PageBox;
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

struct Counting;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static COUNT: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

// cov:ignore: benchmark-only allocator, exercised by the release smoke run.
fn record(size: usize) {
    if ACTIVE.load(Relaxed) {
        COUNT.fetch_add(1, Relaxed);
        BYTES.fetch_add(size, Relaxed);
    }
}

// SAFETY: every allocation and release is forwarded unchanged to System.
// Counters include joined Rayon workers and never allocate.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Counting {
    // cov:ignore: benchmark-only allocator, exercised by the release smoke run.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplies System's required allocation contract.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            record(layout.size());
        }
        pointer
    }

    // cov:ignore: benchmark-only allocator, exercised by the release smoke run.
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplies System's required allocation contract.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            record(layout.size());
        }
        pointer
    }

    // cov:ignore: benchmark-only allocator, exercised by the release smoke run.
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the caller supplies the pointer and its original layout.
        unsafe { System.dealloc(pointer, layout) };
    }

    // cov:ignore: benchmark-only allocator, exercised by the release smoke run.
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: the caller supplies System's required reallocation contract.
        let replacement = unsafe { System.realloc(pointer, layout, size) };
        if !replacement.is_null() {
            record(size);
        }
        replacement
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

struct Measurement;

// cov:ignore: benchmark-only guard, exercised by the release smoke run.
impl Drop for Measurement {
    fn drop(&mut self) {
        ACTIVE.store(false, Relaxed);
    }
}

// cov:ignore: benchmark-only measurement, exercised by the release smoke run.
fn measure<T>(operation: impl FnOnce() -> T) -> (T, usize, usize) {
    COUNT.store(0, Relaxed);
    BYTES.store(0, Relaxed);
    ACTIVE.store(true, Relaxed);
    let guard = Measurement;
    let result = operation();
    drop(guard);
    (result, COUNT.load(Relaxed), BYTES.load(Relaxed))
}

// cov:ignore: benchmark entry point, exercised by the release smoke run.
fn main() {
    // Each call joins its layout workers before measurement ends. Run this
    // binary alone: unrelated concurrent work in this process would be counted.
    let resources = workloads::resources();
    let mut page = PageBox::new();
    page.width = 640.0;
    page.height = 480.0;
    println!("workload,items,sample,nodes,allocations,allocated_bytes");
    for shape in [Shape::Flex, Shape::Grid, Shape::Inline] {
        for items in [100, 1000] {
            let html = workloads::document(shape, items);
            // Warm shared caches and verify the requested formatting context.
            let (mut probe, cascade) = workloads::single_page_input(&html, &resources);
            workloads::check_shape(&probe.dom, &cascade, shape);
            raikiri_dom::layout_single_page(&mut probe.dom, &cascade, page).expect("probe layout");
            workloads::check_layout(&probe.dom, items);
            drop((probe, cascade));
            for sample in 1..=5 {
                let (mut document, cascade) = workloads::single_page_input(&html, &resources);
                let (result, allocations, bytes) = measure(|| {
                    raikiri_dom::layout_single_page(&mut document.dom, &cascade, black_box(page))
                });
                result.expect("layout");
                assert!(
                    allocations > 0 && bytes > 0,
                    "allocator instrumentation is active"
                );
                workloads::check_layout(&document.dom, items);
                println!(
                    "{},{items},{sample},{},{allocations},{bytes}",
                    shape.name(),
                    document.dom.node_count()
                );
            }
        }
    }
}
