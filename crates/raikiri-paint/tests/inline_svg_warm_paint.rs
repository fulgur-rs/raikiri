//! A repeated paint of an unchanged inline SVG allocates nothing for each of
//! the elements that its stylesheet declarations style.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyrender::{PaintScene, Scene};
use raikiri_html::{LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults, RenderResources};

struct Counting;
static ENABLED: AtomicBool = AtomicBool::new(false);
static CALLS: AtomicU64 = AtomicU64::new(0);

// SAFETY: Every allocation is forwarded unchanged to System. The counter only
// observes calls and never touches the allocated storage.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ENABLED.load(Ordering::Relaxed) {
            CALLS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: The caller supplies a valid layout.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: The caller supplies a live allocation and its layout.
        unsafe { System.dealloc(pointer, layout) }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if ENABLED.load(Ordering::Relaxed) {
            CALLS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: The caller supplies a live allocation and a valid new size.
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The allocations of one warm paint of an SVG whose `count` rectangles take
/// their paint, opacity and stroke from a stylesheet.
fn warm_paint_allocations(count: usize) -> u64 {
    let resources = RenderResources::new();
    let rects = r#"<rect class="s" width="2" height="2"/>"#.repeat(count);
    let html = format!(
        "<!doctype html><style>@page{{size:50px 50px;margin:0}}body{{margin:0}}\
         .s{{fill:#336699;stroke:#112233;stroke-width:2;opacity:.9;stroke-dasharray:1 2}}</style>\
         <svg width=\"20\" height=\"20\">{rects}</svg>"
    );
    let doc = raikiri_html::parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(layout) = raikiri_html::layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    let (document, cascade, page_box, _) = layout.page(0).unwrap().paint_inputs();
    let mut scene = Scene::new();
    raikiri_paint::paint_single_page(&mut scene, document, cascade, page_box).unwrap();
    scene.reset();
    ENABLED.store(true, Ordering::Relaxed);
    CALLS.store(0, Ordering::Relaxed);
    raikiri_paint::paint_single_page(&mut scene, document, cascade, page_box).unwrap();
    ENABLED.store(false, Ordering::Relaxed);
    CALLS.load(Ordering::Relaxed)
}

#[test]
fn a_warm_paint_allocates_the_same_for_a_larger_svg() {
    let small = warm_paint_allocations(250);
    let large = warm_paint_allocations(1000);
    // The walk's own work list grows by doubling, which adds a few calls.
    assert!(
        large <= small + 16,
        "{small} allocations for 250 elements, {large} for 1000"
    );
}
