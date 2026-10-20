//! Allocation checks for the unnamed first-page fast path.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use raikiri_dom::LayoutStyle as Style;
use raikiri_dom::{Document, layout::first_page_name};

struct CountingAllocator;

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}

fn record_allocation() {
    let _ = ALLOCATIONS.try_with(|count| {
        if let Some(value) = count.get() {
            count.set(Some(value + 1));
        }
    });
}

// SAFETY: Every allocation operation forwards the original arguments to System.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        // SAFETY: The caller supplies the allocator's required layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        // SAFETY: The caller supplies the allocator's required layout.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record_allocation();
        // SAFETY: The caller supplies a live allocation and its matching layout.
        unsafe { System.realloc(ptr, layout, size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: The caller supplies a live allocation and its matching layout.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
fn unnamed_first_page_selection_allocates_nothing() {
    for (width, depth) in [(256, 1), (1, 256), (64, 4)] {
        let mut doc = Document::new();
        let html = doc.append_element(Some(0), "html", Style::default(), None::<&str>);
        let body = doc.append_element(Some(html), "body", Style::default(), None::<&str>);
        for _ in 0..width {
            let mut parent = body;
            for _ in 0..depth {
                parent = doc.append_element(Some(parent), "div", Style::default(), None::<&str>);
            }
            doc.append_text(parent, "x");
        }
        doc.mark_in_document_flags();
        let rules = raikiri_style::build_rule_tree(&doc);
        let cascade = raikiri_style::cascade(&doc, &rules).unwrap();
        ALLOCATIONS.with(|count| count.set(Some(0)));
        let selected = first_page_name(&doc, &cascade);
        let allocations = ALLOCATIONS.with(|count| count.replace(None).unwrap());
        assert_eq!(selected, None);
        assert_eq!(allocations, 0, "width={width} depth={depth}");
    }
}
