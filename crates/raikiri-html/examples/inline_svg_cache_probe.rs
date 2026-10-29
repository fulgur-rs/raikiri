//! Measure retained requested heap after resolving inline SVG pictures.
//! Parse/layout and the optional caller handle Vec are outside the delta.
//! caller_holds=true deliberately includes caller-owned image allocations.
//! CPU times are references under the allocation-counting System wrapper.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::Instant;

use raikiri_html::{
    FontCollectionBuilder, ImageContent, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults,
    RenderResources,
};
use raikiri_traits::vector::VectorImage;

struct CountedSystem;
static LIVE: AtomicIsize = AtomicIsize::new(0);

// SAFETY: Every operation delegates unchanged pointer/layout requirements to
// System; the atomic only records successful requested allocation sizes.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for CountedSystem {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller supplies a valid allocation layout.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller supplies a valid allocation layout.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
        // SAFETY: The caller owns this allocation with the supplied layout.
        unsafe {
            System.dealloc(ptr, layout);
        }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: Pointer, original layout, and new size satisfy GlobalAlloc.
        let updated = unsafe { System.realloc(ptr, layout, new_size) };
        if !updated.is_null() {
            LIVE.fetch_add(
                new_size as isize - layout.size() as isize,
                Ordering::Relaxed,
            );
        }
        updated
    }
}

#[global_allocator]
static ALLOCATOR: CountedSystem = CountedSystem;

fn main() {
    let ahem: Arc<[u8]> = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
    ))
    .as_slice()
    .into();
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", ahem)
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    for (distinct, caller_holds) in [(false, false), (true, false), (true, true)] {
        for n in [32, 64, 128, 256, 1950] {
            let mut html = String::from(
                "<!doctype html><style>@page{size:400px 400px;margin:0}body{margin:0;font-family:Ahem;font-size:1px;line-height:1px}</style><body>",
            );
            let padding = "x".repeat(16 * 1024);
            for i in 0..n {
                let id = if distinct {
                    format!(" id='v{i}'")
                } else {
                    String::new()
                };
                html.push_str(&format!("<svg width='1' height='1' data-pad='{padding}'{id}><rect width='1' height='1' fill='red'/></svg>"));
            }
            html.push_str("</body>");
            let document =
                raikiri_html::parse_html_with_resources(html.as_bytes(), &resources).unwrap();
            let LayoutStatus::Completed(layout) = raikiri_html::layout(
                &document,
                PageDefaults::default(),
                LayoutConfig::default(),
                LayoutOptions::new().resources(&resources),
            )
            .unwrap() else {
                panic!("aborted")
            };
            let pages: Vec<_> = layout.pages().collect();
            let roots: Vec<_> = pages
                .iter()
                .enumerate()
                .flat_map(|(index, page)| {
                    page.fragments()
                        .filter(|f| page.dom().local_name(f.node()) == Some("svg"))
                        .map(move |fragment| (index, fragment))
                })
                .collect();
            assert_eq!(roots.len(), n);
            let mut held = Vec::with_capacity(if caller_holds { n } else { 0 });
            let before = LIVE.load(Ordering::Relaxed);
            let start = Instant::now();
            let mut first: Option<Weak<VectorImage>> = None;
            let mut scene_bytes = 0;
            for (index, fragment) in &roots {
                let image = pages[*index].inline_svg(fragment).unwrap().unwrap();
                let ImageContent::Vector(scene) = image.content else {
                    panic!("expected vector")
                };
                assert_eq!(scene.size, (1.0, 1.0));
                assert!(scene.node_count() > 1);
                scene_bytes += scene.estimated_heap_bytes();
                if !distinct {
                    let weak = Arc::downgrade(&scene);
                    if let Some(first) = &first {
                        assert!(Weak::ptr_eq(first, &weak));
                    } else {
                        first = Some(weak);
                    }
                }
                if caller_holds {
                    held.push(scene);
                }
            }
            let first_ms = start.elapsed().as_secs_f64() * 1000.0;
            let after = LIVE.load(Ordering::Relaxed);
            let start = Instant::now();
            for (index, fragment) in &roots {
                drop(pages[*index].inline_svg(fragment).unwrap().unwrap());
            }
            let revisit_ms = start.elapsed().as_secs_f64() * 1000.0;
            let revisited = LIVE.load(Ordering::Relaxed);
            println!(
                "inline svg distinct={distinct} caller_holds={caller_holds} roots={n} input_bytes={} retained_bytes={} revisit_delta={} summed_scene_estimate={scene_bytes} first_ms={first_ms:.6} revisit_ms={revisit_ms:.6}",
                html.len(),
                after - before,
                revisited - after
            );
            assert_eq!(held.len(), if caller_holds { n } else { 0 });
        }
    }
}
