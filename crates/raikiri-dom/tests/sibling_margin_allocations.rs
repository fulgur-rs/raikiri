//! Cumulative allocation measurements for paginating sibling margins.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use raikiri_dom::{Document, layout::layout_pages};
use raikiri_traits::PageBox;

struct CountingAllocator;

thread_local! {
    static ALLOCATIONS: Cell<Option<(usize, usize)>> = const { Cell::new(None) };
}

fn record(bytes: usize) {
    let _ = ALLOCATIONS.try_with(|count| {
        if let Some((calls, allocated)) = count.get() {
            count.set(Some((calls + 1, allocated + bytes)));
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

#[test]
fn sibling_margin_layout_allocation_measurements_keep_page_positions() {
    for prefix in [128, 256, 512] {
        for breaks in [1, 8, 64] {
            let mut doc = Document::new();
            let html = doc.append_element(Some(0), "html", Some("display:block"));
            let body = doc.append_element(
                Some(html),
                "body",
                Some("display:block;margin:0;width:100px"),
            );
            for _ in 0..prefix {
                doc.append_element(Some(body), "div", Some("display:block;height:0px;margin:0"));
            }
            let mut blocks =
                vec![doc.append_element(Some(body), "div", Some("display:block;height:90px"))];
            for _ in 0..breaks {
                blocks.push(doc.append_element(
                    Some(body),
                    "div",
                    Some("display:block;height:90px;margin-top:30px"),
                ));
            }
            doc.mark_in_document_flags();
            let cascade =
                raikiri_style::cascade(&doc, &raikiri_style::build_rule_tree(&doc)).unwrap();
            let mut page = PageBox::new();
            page.width = 100.0;
            page.height = 100.0;
            ALLOCATIONS.set(Some((0, 0)));
            let start = std::time::Instant::now();
            let pages = layout_pages(&mut doc, &cascade, page).unwrap();
            let elapsed = start.elapsed();
            let (allocations, allocated_bytes) = ALLOCATIONS.replace(None).unwrap();
            assert_eq!(pages.len(), breaks + 1);
            for (i, &id) in blocks.iter().enumerate() {
                assert_eq!(
                    doc.get_node(id).unwrap().layout().location.y,
                    i as f32 * 100.0
                );
                assert_eq!(doc.get_node(id).unwrap().layout().size.height, 90.0);
            }
            eprintln!(
                "sibling_margin_layout prefix={prefix} breaks={breaks} allocations={allocations} allocated_bytes={allocated_bytes} elapsed={elapsed:?}"
            );
        }
    }
}
