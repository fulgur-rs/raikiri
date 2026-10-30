//! Retained allocation guard for the public opaque inspection tree.
use raikiri_style::{Origin, RuleNode, RuleTree};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering::Relaxed};

struct CountingAllocator;
static LIVE: AtomicIsize = AtomicIsize::new(0);

// SAFETY: Every operation forwards its original allocation arguments to System.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller supplies a valid allocation layout.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            LIVE.fetch_add(layout.size() as isize, Relaxed);
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller supplies a valid allocation layout.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            LIVE.fetch_add(layout.size() as isize, Relaxed);
        }
        pointer
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: The caller supplies a live pointer, its layout, and a valid size.
        let result = unsafe { System.realloc(pointer, layout, size) };
        if !result.is_null() {
            LIVE.fetch_add(size as isize - layout.size() as isize, Relaxed);
        }
        result
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Relaxed);
        // SAFETY: The caller supplies the live pointer and its allocation layout.
        unsafe { System.dealloc(pointer, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
fn nested_opaque_inspection_retains_source_and_metadata_instead_of_every_body_copy() {
    const OPEN: &str = "@opaque {";
    for depth in [1, 4, 16, 32, 64] {
        let mut css = OPEN.repeat(depth);
        css.push_str("/*");
        css.push_str(&"x".repeat(32768));
        css.push_str("*/ .probe { color: red }");
        css.push_str(&"}".repeat(depth));
        let mut warm = RuleTree::empty();
        warm.add_stylesheet(&css, Origin::User);
        drop(warm);
        let before = LIVE.load(Relaxed);
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(&css, Origin::User);
        let retained = LIVE.load(Relaxed) - before;
        assert!(tree.limit_exceeded().is_none());
        assert!(tree.style_rules().is_empty());
        assert_eq!(tree.opaque_at_rules().len(), 1);
        let mut current = &tree.opaque_at_rules()[0];
        for level in 1..=depth {
            assert_eq!(current.origin, Origin::User);
            assert_eq!(current.source_order, 0);
            assert_eq!(
                current.body.as_block(),
                Some(&css[level * OPEN.len()..css.len() - level])
            );
            assert_eq!(current.children().len(), 1);
            match &current.children()[0] {
                RuleNode::AtRule(child) if level < depth => current = child,
                RuleNode::Qualified(child) if level == depth => {
                    assert_eq!(child.prelude.trim(), ".probe");
                    assert_eq!(child.body.trim(), "color: red");
                    assert_eq!(child.origin, Origin::User);
                    assert_eq!(child.source_order, 0);
                }
                _ => panic!("unexpected inspection shape"),
            }
        }
        let bound = 4 * css.len() + 1024 * depth;
        assert!(
            retained >= 0 && (retained as usize) < bound,
            "depth {depth}: retained {retained} requested bytes, bound {bound}"
        );
    }
}
