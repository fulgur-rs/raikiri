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
fn appending_a_character_allocates_only_in_proportion_to_the_added_data() {
    for length in [4093, 65537, LENGTH] {
        let mut doc = Document::new();
        let text = doc.create_detached_text(&"x".repeat(length));
        let allocated = allocated_by(|| {
            for _ in 0..APPENDS {
                doc.append_character_data(text, "y").unwrap();
            }
        });
        assert!(
            allocated < 4096,
            "prefix {length}: allocated {allocated} bytes"
        );
        assert_eq!(doc.character_data(text).unwrap().len(), length + APPENDS);
    }
}

#[test]
fn shared_text_appends_do_not_copy_an_unread_or_materialized_prefix() {
    for read in [false, true] {
        let mut doc = Document::new();
        let text = doc.create_detached_text(&"x".repeat(LENGTH));
        doc.append_character_data(text, "seed").unwrap();
        if read {
            assert_eq!(doc.character_data(text).unwrap().len(), LENGTH + 4);
        }
        let original = doc.clone();
        let allocated = allocated_by(|| {
            for _ in 0..APPENDS {
                doc.append_character_data(text, "y").unwrap();
            }
        });
        assert!(
            allocated < 16384,
            "read cache {read}: allocated {allocated} bytes"
        );
        assert_eq!(
            doc.character_data(text).unwrap().len(),
            LENGTH + 4 + APPENDS
        );
        assert_eq!(original.character_data(text).unwrap().len(), LENGTH + 4);
    }
}

#[test]
fn clone_node_and_subtree_copy_share_pending_text_without_flattening_it() {
    for read in [false, true] {
        let mut doc = Document::new();
        let parent = doc.append_element(Some(0), "p", None::<&str>);
        let text = doc.append_text(parent, "x".repeat(LENGTH));
        doc.append_character_data(text, "seed").unwrap();
        if read {
            assert_eq!(doc.character_data(text).unwrap().len(), LENGTH + 4);
        }
        let copy = Cell::new(usize::MAX);
        let allocated = allocated_by(|| copy.set(doc.clone_node(text, false).unwrap()));
        assert!(
            allocated < 65536,
            "clone read cache {read}: {allocated} bytes"
        );
        let copy = copy.get();
        let allocated = allocated_by(|| doc.append_character_data(copy, "copy").unwrap());
        assert!(allocated < 16384, "clone append: {allocated} bytes");
        assert!(doc.character_data(copy).unwrap().ends_with("seedcopy"));
        assert!(doc.character_data(text).unwrap().ends_with("seed"));

        let mut target = Document::new();
        let target_parent = target.append_element(Some(0), "p", None::<&str>);
        let allocated = allocated_by(|| target.replace_children_from(target_parent, &doc, parent));
        assert!(allocated < 65536, "subtree copy: {allocated} bytes");
        let target_text = target.get_node(target_parent).unwrap().children[0];
        let allocated =
            allocated_by(|| target.append_character_data(target_text, "target").unwrap());
        assert!(allocated < 16384, "subtree append: {allocated} bytes");
        assert!(
            target
                .character_data(target_text)
                .unwrap()
                .ends_with("seedtarget")
        );
        assert!(doc.character_data(text).unwrap().ends_with("seed"));
    }
}

#[test]
fn coalesced_appends_share_the_same_prefix_independent_allocation_bound() {
    for before in [false, true] {
        let mut doc = Document::new();
        let parent = doc.append_element(Some(0), "p", None::<&str>);
        let text = doc.append_text(parent, "x".repeat(LENGTH));
        let following = if before {
            Some(doc.append_element(Some(parent), "b", None::<&str>))
        } else {
            None
        };
        let allocated = allocated_by(|| {
            for _ in 0..APPENDS {
                let merged = if let Some(following) = following {
                    doc.insert_text_before_coalesced(parent, following, "y")
                } else {
                    doc.append_text_coalesced(parent, "y")
                };
                assert_eq!(merged, text);
            }
        });
        assert!(
            allocated < 4096,
            "insert before {before}: {allocated} bytes"
        );
        assert_eq!(doc.character_data(text).unwrap().len(), LENGTH + APPENDS);
    }
}

#[test]
fn a_contiguous_read_is_cached_and_does_not_repeat_its_allocation() {
    let mut doc = Document::new();
    let text = doc.create_detached_text(&"x".repeat(LENGTH));
    for _ in 0..APPENDS {
        doc.append_character_data(text, "y").unwrap();
    }
    let first =
        allocated_by(|| assert_eq!(doc.character_data(text).unwrap().len(), LENGTH + APPENDS));
    assert!(
        first < 2 * (LENGTH + APPENDS) + 4096,
        "first borrowed read: {first} bytes"
    );
    let repeated = allocated_by(|| {
        for _ in 0..100 {
            assert_eq!(doc.character_data(text).unwrap().len(), LENGTH + APPENDS);
        }
    });
    assert_eq!(repeated, 0);
}
