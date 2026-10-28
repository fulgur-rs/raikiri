//! Allocation checks for the production font-context key implementation.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;

use raikiri_svg::{SvgFontFamily, SvgFontRequest, SvgFontStyle};
use raikiri_traits::vector::FontFace;

#[path = "../src/fonts/key.rs"]
mod key;

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}
struct Counting;
fn allocation() {
    let _ = ALLOCATIONS.try_with(|count| {
        if let Some(value) = count.get() {
            count.set(Some(value + 1));
        }
    });
}
// SAFETY: Every allocation operation forwards original arguments to System.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        allocation();
        // SAFETY: The caller provides a valid allocation layout.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: The caller provides the live pointer and matching layout.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        allocation();
        // SAFETY: The caller provides the allocation, matching layout and new size.
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[test]
fn borrowed_and_shared_key_lookups_allocate_nothing() {
    let face = FontFace::new(Arc::new(vec![1_u8, 2, 3]), 0);
    let identity = key::FaceIdentity::of(&face);
    let request = Arc::new(SvgFontRequest::new(vec![
        SvgFontFamily::Named("family-long-enough-to-require-owned-storage".into()),
        SvgFontFamily::Serif,
    ]));
    let mut index = HashMap::new();
    index.insert(key::ContextKey::shared(identity, Arc::clone(&request)), 7);
    for shared in [false, true] {
        ALLOCATIONS.with(|count| count.set(Some(0)));
        for _ in 0..1_000 {
            let query = if shared {
                key::ContextKey::shared(identity, Arc::clone(&request))
            } else {
                key::ContextKey::borrowed(identity, &request)
            };
            assert_eq!(index.get(&query), Some(&7));
        }
        let count = ALLOCATIONS.with(|count| count.replace(None).unwrap());
        assert_eq!(count, 0);
    }
    assert_eq!(Arc::strong_count(&request), 2);
}
