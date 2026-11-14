//! Heap and time costs of one long table, per stage.
//!
//! Run `cargo bench -p raikiri-bench --bench table_memory`.
//! `RAIKIRI_TABLE_ROWS=2500,5000,20000` selects the row counts. Each row of
//! output reports the peak live heap during a stage and the heap that stage
//! leaves behind, then the time of a separate run without counting.

// cov:ignore: benchmark harness; workspace coverage does not build this target.
mod common {
    pub mod workloads;
}

use std::alloc::{GlobalAlloc, Layout, System};
use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering::Relaxed};
use std::time::Instant;

// cov:ignore: benchmark harness; workspace coverage does not build this target.
use raikiri_html::{
    ConsumerPropertyEvent, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults, PageSink,
    RenderResources, StreamPage, StreamStatus, StreamSummary, StreamingLayout, layout,
    parse_html_with_resources,
};

struct Counting; // cov:ignore: benchmark-only allocator state.
static MEASURING: AtomicBool = AtomicBool::new(false); // cov:ignore: benchmark-only allocator state.
static LIVE: AtomicIsize = AtomicIsize::new(0); // cov:ignore: benchmark-only allocator state.
static PEAK: AtomicIsize = AtomicIsize::new(0); // cov:ignore: benchmark-only allocator state.

// cov:ignore: benchmark harness; workspace coverage does not build this target.
fn record(delta: isize) {
    if MEASURING.load(Relaxed) {
        let live = LIVE.fetch_add(delta, Relaxed).saturating_add(delta);
        PEAK.fetch_max(live, Relaxed);
    }
}

// cov:ignore: benchmark harness; workspace coverage does not build this target.
fn size(bytes: usize) -> isize {
    isize::try_from(bytes).unwrap_or(isize::MAX)
}

// SAFETY: every call forwards the allocator contract unchanged to System;
// counters do not allocate or affect the returned pointer.
// cov:ignore: benchmark harness; workspace coverage does not build this target.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds GlobalAlloc's allocation contract.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            record(size(layout.size()));
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds GlobalAlloc's allocation contract.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            record(size(layout.size()));
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: ptr and layout satisfy System's deallocation contract.
        unsafe { System.dealloc(ptr, layout) };
        record(-size(layout.size()));
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller upholds GlobalAlloc's reallocation contract.
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            record(size(new_size) - size(layout.size()));
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting; // cov:ignore: benchmark-only allocator state.

/// Peak live heap above the starting point, and the heap still live at the end.
// cov:ignore: benchmark harness; workspace coverage does not build this target.
fn measure<T>(f: impl FnOnce() -> T) -> (T, isize, isize) {
    LIVE.store(0, Relaxed);
    PEAK.store(0, Relaxed);
    MEASURING.store(true, Relaxed);
    let value = f();
    MEASURING.store(false, Relaxed);
    (value, PEAK.load(Relaxed), LIVE.load(Relaxed))
}

/// Three columns of short text, sized by their content.
// cov:ignore: benchmark harness; workspace coverage does not build this target.
fn table_document(rows: usize) -> String {
    let mut html = String::from(
        "<!doctype html><html><head><style>body { font-family: Ahem; font-size: 10px } \
         table { border-collapse: collapse; width: 100% } \
         td, th { border: 1px solid; padding: 2px }</style></head><body>\
         <table><thead><tr><th>Id</th><th>Name</th><th>Amount</th></tr></thead><tbody>",
    );
    for row in 0..rows {
        let _ = writeln!(
            html,
            "<tr><td>{row}</td><td>Item name {row}</td><td>{}.00</td></tr>",
            row * 7 % 10_000
        );
    }
    html.push_str("</tbody></table></body></html>");
    html
}

// cov:ignore: benchmark harness; workspace coverage does not build this target.
struct Count(u32);

// cov:ignore: benchmark harness; workspace coverage does not build this target.
impl PageSink for Count {
    type Output = u32;

    fn page(&mut self, _: StreamPage<'_>, _: Vec<ConsumerPropertyEvent>) -> std::io::Result<()> {
        self.0 += 1;
        Ok(())
    }

    fn finish(self, summary: StreamSummary) -> std::io::Result<u32> {
        assert_eq!(self.0, summary.page_count);
        Ok(self.0)
    }
}

// cov:ignore: benchmark harness; workspace coverage does not build this target.
fn stream(html: &str, resources: &RenderResources<'_>) -> u32 {
    let mut stream = StreamingLayout::new(
        resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        Count(0),
    );
    for chunk in html.as_bytes().chunks(4096) {
        stream.write_all(chunk).expect("stream input");
    }
    let StreamStatus::Completed(pages) = stream.finish().expect("stream finish") else {
        panic!("stream aborted without a signal");
    };
    pages
}

// cov:ignore: benchmark harness; workspace coverage does not build this target.
fn mb(bytes: isize) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

// cov:ignore: benchmark harness; workspace coverage does not build this target.
fn main() {
    if !std::env::args().any(|arg| arg == "--bench") {
        return;
    }
    let rows: Vec<usize> = std::env::var("RAIKIRI_TABLE_ROWS")
        .ok()
        .map(|list| {
            list.split(',')
                .map(|n| n.trim().parse().expect("row count"))
                .collect()
        })
        .unwrap_or_else(|| vec![2_500, 5_000]);
    let only = std::env::var("RAIKIRI_TABLE_MODE").ok();
    let wants = |mode: &str| only.as_deref().is_none_or(|only| only == mode);
    let resources = common::workloads::resources();
    let _ = stream(&table_document(1), &resources);
    for rows in rows {
        let html = table_document(rows);
        if wants("batch") {
            let start = Instant::now();
            let (document, parse_peak, parse_live) =
                measure(|| parse_html_with_resources(html.as_bytes(), &resources).expect("parse"));
            let parse_time = start.elapsed();
            let start = Instant::now();
            let (laid_out, layout_peak, layout_live) = measure(|| {
                let LayoutStatus::Completed(laid_out) = layout(
                    &document,
                    PageDefaults::default(),
                    LayoutConfig::default(),
                    LayoutOptions::new().resources(&resources),
                )
                .expect("layout") else {
                    panic!("layout aborted without a signal");
                };
                laid_out
            });
            let layout_time = start.elapsed();
            let cells = (rows + 1) * 3;
            println!(
                "rows={rows} input={} mode=batch pages={} parse_peak_mb={:.1} parse_live_mb={:.1} \
                 layout_peak_mb={:.1} layout_live_mb={:.1} total_peak_mb={:.1} \
                 peak_kb_per_cell={:.1} parse_s={:.2} layout_s={:.2}",
                html.len(),
                laid_out.page_count(),
                mb(parse_peak),
                mb(parse_live),
                mb(layout_peak),
                mb(layout_live),
                mb(parse_live + layout_peak),
                (parse_live + layout_peak) as f64 / 1024.0 / cells as f64,
                parse_time.as_secs_f64(),
                layout_time.as_secs_f64(),
            );
        }
        if wants("stream") {
            let start = Instant::now();
            let (pages, peak, live) = measure(|| stream(&html, &resources));
            println!(
                "rows={rows} input={} mode=stream pages={pages} peak_mb={:.1} end_live_mb={:.1} s={:.2}",
                html.len(),
                mb(peak),
                mb(live),
                start.elapsed().as_secs_f64(),
            );
        }
    }
}
