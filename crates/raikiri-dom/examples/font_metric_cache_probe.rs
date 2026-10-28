//! Manual retained-heap and hit-heavy font-metric cache measurements.
//! Run the release binary on a fixed CPU without concurrent builds.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use raikiri_dom::Document;
use raikiri_style::property::{FontOpticalSizing, FontStyle};
use raikiri_style::{ChFontKey, ComputedLength, FontFamilyName};
use shodo::font::FontCollection;
use shodo::limits::Limits;

#[path = "../../raikiri-bench/benches/common/report.rs"]
mod report;

struct Counting;
static LIVE: AtomicIsize = AtomicIsize::new(0);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: All requests and pointers are forwarded unchanged to System.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller provides a valid allocation layout.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            LIVE.fetch_add(layout.size() as isize, Relaxed);
            ALLOCS.fetch_add(1, Relaxed);
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The caller provides a valid allocation layout.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            LIVE.fetch_add(layout.size() as isize, Relaxed);
            ALLOCS.fetch_add(1, Relaxed);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Relaxed);
        // SAFETY: The caller provides the live pointer's matching layout.
        unsafe { System.dealloc(ptr, layout) };
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: The caller provides a live pointer, its layout, and valid size.
        let next = unsafe { System.realloc(ptr, layout, size) };
        if !next.is_null() {
            LIVE.fetch_add(size as isize - layout.size() as isize, Relaxed);
            ALLOCS.fetch_add(1, Relaxed);
        }
        next
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn document() -> Document {
    let mut doc = Document::new();
    doc.set_font_collection(FontCollection::new(&Limits::default()));
    doc
}

fn key() -> ChFontKey {
    ChFontKey::new(
        Arc::new(vec![FontFamilyName::named("MissingPerfProbeFont")]),
        ComputedLength(16.0),
        400.0,
        FontStyle::Normal,
        FontOpticalSizing::None,
    )
}

fn report_cascades() {
    let html = report::report_html(10, 50);
    let mut parsed =
        raikiri_html::parse(html.as_bytes(), &raikiri_html::ParseOptions::new()).unwrap();
    parsed
        .dom
        .set_font_collection(FontCollection::new(&Limits::default()));
    let tree = raikiri_html::build_rule_tree(&parsed);
    let media = raikiri_style::MediaContext::print();
    let warm = raikiri_style::cascade_with_media_context(&parsed.dom, &tree, &media).unwrap();
    report::check_report(&warm, 10, 50);
    drop(warm);
    let mut times = Vec::new();
    let mut allocations = Vec::new();
    for _ in 0..5 {
        let before = ALLOCS.load(Relaxed);
        let start = Instant::now();
        for _ in 0..20 {
            black_box(
                raikiri_style::cascade_with_media_context(&parsed.dom, &tree, &media).unwrap(),
            );
        }
        let elapsed = start.elapsed().as_secs_f64() * 1_000.0;
        let count = ALLOCS.load(Relaxed) - before;
        times.push(elapsed);
        allocations.push(count);
    }
    times.sort_by(f64::total_cmp);
    eprintln!(
        "font cache report=10x50 cascades=20 median_ms={:.6} samples={times:?} allocations={allocations:?}",
        times[2]
    );
}

fn main() {
    let long_history = std::env::args().any(|arg| arg == "--long-history");
    let sizes: &[usize] = if long_history {
        &[64_000, 256_000]
    } else {
        &[1_000, 2_000, 4_000, 16_000]
    };
    for &n in sizes {
        let doc = document();
        let source = doc.normal_line_height();
        let mut font = key();
        black_box(source.0.normal_line_height(&font));
        black_box(source.0.ch_advance(&font));
        let before = LIVE.load(Relaxed);
        for i in 0..n {
            font.size = ComputedLength(17.0 + i as f32 / 16.0);
            black_box(source.0.normal_line_height(&font));
            black_box(source.0.ch_advance(&font));
        }
        let retained = LIVE.load(Relaxed) - before;
        let before = LIVE.load(Relaxed);
        for i in 0..n {
            font.size = ComputedLength(17.0 + i as f32 / 16.0);
            black_box(source.0.normal_line_height(&font));
            black_box(source.0.ch_advance(&font));
        }
        eprintln!(
            "font cache distinct={n} retained_bytes={retained} revisit_delta_bytes={}",
            LIVE.load(Relaxed) - before
        );
    }
    if long_history {
        return;
    }
    for mode in ["hits", "churn"] {
        let mut times = Vec::new();
        let mut allocations = Vec::new();
        for _ in 0..5 {
            let doc = document();
            let source = doc.normal_line_height();
            let font = key();
            let mut cold = font.clone();
            black_box(source.0.normal_line_height(&font));
            black_box(source.0.ch_advance(&font));
            let before = ALLOCS.load(Relaxed);
            let start = Instant::now();
            for i in 0..100_000 {
                black_box(source.0.normal_line_height(black_box(&font)));
                black_box(source.0.ch_advance(black_box(&font)));
                if mode == "churn" && i % 16 == 0 {
                    cold.size = ComputedLength(17.0 + i as f32 / 16.0);
                    black_box(source.0.normal_line_height(&cold));
                    black_box(source.0.ch_advance(&cold));
                }
            }
            let elapsed = start.elapsed().as_secs_f64() * 1_000.0;
            let count = ALLOCS.load(Relaxed) - before;
            times.push(elapsed);
            allocations.push(count);
        }
        times.sort_by(f64::total_cmp);
        eprintln!(
            "font cache mode={mode} hits=100000 median_ms={:.6} samples={times:?} allocations={allocations:?}",
            times[2]
        );
    }
    report_cascades();
}
