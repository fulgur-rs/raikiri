//! Peak heap and time of a streaming layout against a batch layout, by page
//! count.
//!
//! A streaming layout should hold less memory than a batch layout once its
//! pages are delivered: ideally its peak stays flat as the document grows,
//! while a batch layout's peak grows with the page count. This bench lays
//! the same text document out both ways at several lengths and prints, for
//! each, the peak of live heap bytes during the run and the wall time, then
//! the growth per page between the shortest and the longest document.
//!
//! The peak counts the allocations of every thread, because paragraphs are
//! built on several threads. It is the heap the layout holds, not the
//! process RSS, so it does not depend on the allocator's caching.
//!
//! Run with:
//!
//! ```text
//! cargo bench -p raikiri-bench --bench streaming_memory
//! ```
//!
//! Set `RAIKIRI_STREAM_SECTIONS` to a comma separated list of section counts
//! (for example `200,800,2000`) to measure other lengths. A section is three
//! paragraphs, and two sections fill about one page.

// cov:ignore: bench harness — `cargo test`/`cargo llvm-cov --workspace` never
// build this bench target, so nothing in this file has coverage
// instrumentation to attribute to.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering::Relaxed};
use std::time::{Duration, Instant};

use raikiri_html::{
    ConsumerPropertyEvent, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults, PageSink,
    RenderResources, StreamPage, StreamStatus, StreamSummary, StreamingLayout, layout,
    parse_html_with_resources,
};

struct Counting;

static MEASURING: AtomicBool = AtomicBool::new(false);
static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);

// cov:ignore: bench harness — same reason as the file header.
fn record(delta: isize) {
    if MEASURING.load(Relaxed) {
        let live = LIVE.fetch_add(delta, Relaxed).saturating_add(delta);
        PEAK.fetch_max(live, Relaxed);
    }
}

// cov:ignore: bench harness — same reason as the file header.
fn size(layout: Layout) -> isize {
    isize::try_from(layout.size()).unwrap_or(isize::MAX)
}

// Counting allocations needs a `GlobalAlloc`, whose methods are unsafe to
// implement; nothing else in this file uses unsafe code.
// SAFETY: every method forwards its arguments unchanged to `System` and only
// updates counters on the side, so `System`'s guarantees carry over.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            record(size(layout));
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::alloc_zeroed`'s contract.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            record(size(layout));
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` was returned by this allocator for `layout`.
        unsafe { System.dealloc(ptr, layout) };
        record(-size(layout));
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::realloc`'s contract.
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            record(isize::try_from(new_size).unwrap_or(isize::MAX) - size(layout));
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Peak live heap bytes of every thread during `f`, and its wall time.
///
/// Memory released during the call that was allocated before it lowers the
/// live count, so `f` should own everything it measures.
// cov:ignore: bench harness — same reason as the file header.
fn measure<T>(f: impl FnOnce() -> T) -> (T, usize, Duration) {
    LIVE.store(0, Relaxed);
    PEAK.store(0, Relaxed);
    MEASURING.store(true, Relaxed);
    let start = Instant::now();
    let value = f();
    let elapsed = start.elapsed();
    MEASURING.store(false, Relaxed);
    let peak = usize::try_from(PEAK.load(Relaxed)).unwrap_or(0);
    (value, peak, elapsed)
}

/// A document of `sections` times three paragraphs of about 120 words; the
/// default page size fits about six of them.
// cov:ignore: bench harness — same reason as the file header.
fn text_document(sections: usize) -> String {
    const WORDS: [&str; 5] = ["lorem ", "ipsum ", "dolor ", "sit ", "amet "];
    let mut html = String::from(
        "<!doctype html><html><head><style>p { margin: 0 0 1em }</style></head><body>",
    );
    for paragraph in 0..sections * 3 {
        html.push_str("<p>Paragraph ");
        html.push_str(&paragraph.to_string());
        html.push_str(". ");
        for word in 0..120 {
            html.push_str(WORDS[(paragraph + word) % WORDS.len()]);
        }
        html.push_str("</p>\n");
    }
    html.push_str("</body></html>");
    html
}

/// A sink that drops every page, as a sink that writes pages out would.
struct Discard;

impl PageSink for Discard {
    type Output = u32;

    // cov:ignore: bench harness — same reason as the file header.
    fn page(
        &mut self,
        _page: StreamPage<'_>,
        _events: Vec<ConsumerPropertyEvent>,
    ) -> std::io::Result<()> {
        Ok(())
    }

    // cov:ignore: bench harness — same reason as the file header.
    fn finish(self, summary: StreamSummary) -> std::io::Result<u32> {
        Ok(summary.page_count)
    }
}

/// Pages, peak heap and time of a streaming layout fed in 4 KiB writes.
// cov:ignore: bench harness — same reason as the file header.
fn stream(html: &str, resources: &RenderResources<'_>) -> (u32, usize, Duration) {
    measure(|| {
        let mut stream = StreamingLayout::new(
            resources,
            PageDefaults::default(),
            LayoutConfig::default(),
            Discard,
        );
        for chunk in html.as_bytes().chunks(4096) {
            stream.write_all(chunk).expect("the input is accepted");
        }
        match stream.finish().expect("the streaming layout succeeds") {
            StreamStatus::Completed(pages) => pages,
            _ => unreachable!("nothing aborts the layout"),
        }
    })
}

/// Pages, peak heap and time of a parse and a batch layout.
// cov:ignore: bench harness — same reason as the file header.
fn batch(html: &str, resources: &RenderResources<'_>) -> (u32, usize, Duration) {
    measure(|| {
        let document =
            parse_html_with_resources(html.as_bytes(), resources).expect("the document parses");
        let status = layout(
            &document,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new().resources(resources),
        )
        .expect("the layout succeeds");
        let LayoutStatus::Completed(laid_out) = status else {
            unreachable!("nothing aborts the layout");
        };
        laid_out.page_count()
    })
}

/// One measured length.
struct Row {
    pages: u32,
    input_bytes: usize,
    batch_peak: usize,
    batch_time: Duration,
    stream_peak: usize,
    stream_time: Duration,
}

// cov:ignore: bench harness — same reason as the file header.
fn kib(bytes: usize) -> f64 {
    bytes as f64 / 1024.0
}

/// Heap growth per page from `first` to `last`, in KiB.
// cov:ignore: bench harness — same reason as the file header.
fn slope(first: (u32, usize), last: (u32, usize)) -> f64 {
    let pages = f64::from(last.0.saturating_sub(first.0).max(1));
    (kib(last.1) - kib(first.1)) / pages
}

// cov:ignore: bench harness — same reason as the file header.
fn main() {
    // `cargo bench` passes `--bench`; `cargo test --benches` runs the
    // target without it, in a debug build where the numbers mean nothing.
    if !std::env::args().any(|arg| arg == "--bench") {
        return;
    }
    let lengths: Vec<usize> = std::env::var("RAIKIRI_STREAM_SECTIONS")
        .ok()
        .map(|list| {
            list.split(',')
                .map(|n| {
                    n.trim()
                        .parse()
                        .expect("RAIKIRI_STREAM_SECTIONS holds numbers")
                })
                .collect()
        })
        .unwrap_or_else(|| vec![25, 50, 100, 200]);
    let resources = RenderResources::new();
    // A first layout fills the process-wide caches, such as loaded fonts,
    // that every later one reuses, so they stay out of the measurements.
    let _ = batch(&text_document(1), &resources);

    let mut rows = Vec::new();
    for &sections in &lengths {
        let html = text_document(sections);
        let (batch_pages, batch_peak, batch_time) = batch(&html, &resources);
        let (stream_pages, stream_peak, stream_time) = stream(&html, &resources);
        assert_eq!(
            batch_pages, stream_pages,
            "both layouts give the same pages"
        );
        rows.push(Row {
            pages: batch_pages,
            input_bytes: html.len(),
            batch_peak,
            batch_time,
            stream_peak,
            stream_time,
        });
    }

    println!(
        "{:>6} {:>10} {:>14} {:>10} {:>14} {:>10}",
        "pages", "input KiB", "batch peak KiB", "batch ms", "stream peak KiB", "stream ms"
    );
    for row in &rows {
        println!(
            "{:>6} {:>10.0} {:>14.0} {:>10.1} {:>15.0} {:>10.1}",
            row.pages,
            kib(row.input_bytes),
            kib(row.batch_peak),
            row.batch_time.as_secs_f64() * 1000.0,
            kib(row.stream_peak),
            row.stream_time.as_secs_f64() * 1000.0,
        );
    }
    if let (Some(first), Some(last)) = (rows.first(), rows.last())
        && rows.len() > 1
    {
        println!(
            "peak heap per page from {} to {} pages: batch {:.1} KiB, stream {:.1} KiB",
            first.pages,
            last.pages,
            slope(
                (first.pages, first.batch_peak),
                (last.pages, last.batch_peak)
            ),
            slope(
                (first.pages, first.stream_peak),
                (last.pages, last.stream_peak)
            ),
        );
    }
}
