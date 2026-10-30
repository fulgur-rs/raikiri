//! Cumulative allocation measurements for appending to character data.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use raikiri_dom::Document;

struct CountingAllocator;

thread_local! {
    static ALLOCATED: Cell<Option<usize>> = const { Cell::new(None) };
}

fn record(bytes: usize) {
    let _ = ALLOCATED.try_with(|count| {
        if let Some(allocated) = count.get() {
            count.set(Some(allocated + bytes));
        }
    });
}

// SAFETY: All operations forward their original arguments to System.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        // SAFETY: The caller provides a valid allocator layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        // SAFETY: The caller provides a valid allocator layout.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record(size);
        // SAFETY: The caller provides a live allocation and matching layout.
        unsafe { System.realloc(ptr, layout, size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: The caller provides a live allocation and matching layout.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

const LENGTH: usize = 1 << 20;
const APPENDS: usize = 100;

fn allocated_by(run: impl FnOnce()) -> usize {
    ALLOCATED.set(Some(0));
    run();
    ALLOCATED.replace(None).unwrap()
}

#[test]
fn appending_nothing_to_a_long_text_allocates_nothing_in_proportion_to_its_length() {
    let mut doc = Document::new();
    let text = doc.create_detached_text(&"x".repeat(LENGTH));
    let allocated = allocated_by(|| {
        for _ in 0..APPENDS {
            doc.append_character_data(text, "").unwrap();
        }
    });
    assert!(allocated < 4096, "allocated {allocated} bytes");
    assert_eq!(doc.character_data(text).unwrap().len(), LENGTH);
}

#[test]
fn appending_a_character_to_a_long_text_builds_one_joined_string_per_append() {
    let mut doc = Document::new();
    let text = doc.create_detached_text(&"x".repeat(LENGTH));
    let allocated = allocated_by(|| {
        for _ in 0..APPENDS {
            doc.append_character_data(text, "y").unwrap();
        }
    });
    // The joined string, and the shared string it is moved into: two copies
    // of a text that grows by one byte per append.
    assert!(
        allocated < APPENDS * (2 * LENGTH + 4096),
        "allocated {allocated} bytes"
    );
    assert_eq!(doc.character_data(text).unwrap().len(), LENGTH + APPENDS);
}
