//! Parser, batch and streaming heap costs with deterministic bundled fonts.
//!
//! Run `cargo bench -p raikiri-bench --bench streaming_memory`.
//! `RAIKIRI_STREAM_SECTIONS=200,400,800,1600` selects the input lengths.
//! Each row measures requested allocation/reallocation bytes and peak live
//! heap, then times a separate run with allocation counting disabled. Realloc
//! bytes are the requested new sizes, not measured bytes copied by the allocator.
//! The parser retains the full source DOM; a flat streaming heap is not assumed.

// cov:ignore: benchmark harness; workspace coverage does not build this target.
mod common {
    pub mod workloads;
}

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::io::Write as _;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

// cov:ignore: benchmark harness; workspace coverage does not build this target.
use raikiri_html::{
    ConsumerPropertyEvent, ConsumerPropertyRegistration, LayoutConfig, LayoutOptions, LayoutStatus,
    PageDefaults, PageSink, ParseOptions, RenderResources, StreamPage, StreamStatus, StreamSummary,
    StreamingDiagnostics, StreamingLayout, layout, parse, parse_html_with_resources,
};

struct Counting;
static MEASURING: AtomicBool = AtomicBool::new(false);
static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0); // cov:ignore: benchmark-only allocator counter.
static REALLOC_BYTES: AtomicU64 = AtomicU64::new(0); // cov:ignore: benchmark-only allocator counter.
static ALLOC_CALLS: AtomicU64 = AtomicU64::new(0); // cov:ignore: benchmark-only allocator counter.
static REALLOC_CALLS: AtomicU64 = AtomicU64::new(0); // cov:ignore: benchmark-only allocator counter.

// cov:ignore: benchmark harness; workspace coverage does not build this target.
fn record(delta: isize) {
    let live = LIVE.fetch_add(delta, Relaxed).saturating_add(delta);
    PEAK.fetch_max(live, Relaxed);
}

// cov:ignore: benchmark harness; workspace coverage does not build this target.
fn size(layout: Layout) -> isize {
    isize::try_from(layout.size()).unwrap_or(isize::MAX)
}

// SAFETY: every call forwards the allocator contract unchanged to System;
// counters do not allocate or affect the returned pointer.
// cov:ignore: benchmark harness; workspace coverage does not build this target.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds GlobalAlloc's allocation contract.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() && MEASURING.load(Relaxed) {
            ALLOC_BYTES.fetch_add(layout.size() as u64, Relaxed);
            ALLOC_CALLS.fetch_add(1, Relaxed);
            record(size(layout));
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds GlobalAlloc's allocation contract.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() && MEASURING.load(Relaxed) {
            ALLOC_BYTES.fetch_add(layout.size() as u64, Relaxed);
            ALLOC_CALLS.fetch_add(1, Relaxed);
            record(size(layout));
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: ptr and layout satisfy System's deallocation contract.
        unsafe { System.dealloc(ptr, layout) };
        if MEASURING.load(Relaxed) {
            record(-size(layout));
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller upholds GlobalAlloc's reallocation contract.
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() && MEASURING.load(Relaxed) {
            REALLOC_BYTES.fetch_add(new_size as u64, Relaxed);
            REALLOC_CALLS.fetch_add(1, Relaxed);
            record(isize::try_from(new_size).unwrap_or(isize::MAX) - size(layout));
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

// cov:ignore: benchmark harness; workspace coverage does not build this target.
#[derive(Debug)]
struct Heap {
    peak: isize,
    end_live: isize,
    alloc_bytes: u64,
    realloc_bytes: u64,
    alloc_calls: u64,
    realloc_calls: u64,
}

// cov:ignore: benchmark harness; workspace coverage does not build this target.
fn measure<T>(f: impl Fn() -> T) -> (T, Heap, Duration) {
    LIVE.store(0, Relaxed);
    PEAK.store(0, Relaxed);
    ALLOC_BYTES.store(0, Relaxed);
    REALLOC_BYTES.store(0, Relaxed);
    ALLOC_CALLS.store(0, Relaxed);
    REALLOC_CALLS.store(0, Relaxed);
    MEASURING.store(true, Relaxed);
    let value = f();
    MEASURING.store(false, Relaxed);
    let heap = Heap {
        peak: PEAK.load(Relaxed),
        end_live: LIVE.load(Relaxed),
        alloc_bytes: ALLOC_BYTES.load(Relaxed),
        realloc_bytes: REALLOC_BYTES.load(Relaxed),
        alloc_calls: ALLOC_CALLS.load(Relaxed),
        realloc_calls: REALLOC_CALLS.load(Relaxed),
    };
    let start = Instant::now();
    let _ = f();
    (value, heap, start.elapsed())
}

// cov:ignore: benchmark harness; workspace coverage does not build this target.
fn text_document(sections: usize) -> String {
    const WORDS: [&str; 5] = ["lorem ", "ipsum ", "dolor ", "sit ", "amet "];
    let mut html = String::from(
        "<!doctype html><html><head><style>body { font-family: Ahem } \
         p { margin: 0 0 1em; bookmark-level: 1 }</style></head><body>",
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

// cov:ignore: benchmark harness; workspace coverage does not build this target.
#[derive(Default)]
struct Output {
    pages: u32,
    text: Vec<u8>,
    events: Vec<ConsumerPropertyEvent>,
    first_feed: Option<usize>,
    diagnostics: StreamingDiagnostics,
}

// cov:ignore: benchmark harness; workspace coverage does not build this target.
struct Discard {
    output: Output,
    verify: bool,
    fed: Rc<Cell<Option<usize>>>,
}

// cov:ignore: benchmark harness; workspace coverage does not build this target.
impl PageSink for Discard {
    type Output = Output;

    fn page(
        &mut self,
        page: StreamPage<'_>,
        events: Vec<ConsumerPropertyEvent>,
    ) -> std::io::Result<()> {
        assert_eq!(page.index(), self.output.pages);
        if self.output.pages == 0 {
            self.output.first_feed = self.fed.get();
        }
        self.output.pages += 1;
        if self.verify {
            for run in page.page().text_runs() {
                self.output
                    .text
                    .extend(run.text.bytes().filter(|b| !b.is_ascii_whitespace()));
            }
            self.output.events.extend(events);
        }
        Ok(())
    }

    fn finish(mut self, summary: StreamSummary) -> std::io::Result<Output> {
        assert_eq!(self.output.pages, summary.page_count);
        assert!(summary.unplaced_events.is_empty());
        self.output.diagnostics = summary.diagnostics;
        Ok(self.output)
    }
}

// cov:ignore: benchmark harness; workspace coverage does not build this target.
fn stream(html: &str, resources: &RenderResources<'_>, verify: bool) -> Output {
    let fed = Rc::new(Cell::new(None));
    let registrations = [ConsumerPropertyRegistration::integer("bookmark-level")];
    let sink = Discard {
        output: Output::default(),
        verify,
        fed: Rc::clone(&fed),
    };
    let mut stream = StreamingLayout::new(
        resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        sink,
    )
    .consumer_properties(&registrations);
    for (index, chunk) in html.as_bytes().chunks(4096).enumerate() {
        fed.set(Some(index * 4096 + chunk.len()));
        stream.write_all(chunk).expect("stream input");
    }
    fed.set(None);
    let StreamStatus::Completed(result) = stream.finish().expect("stream finish") else {
        panic!("stream aborted without a signal");
    };
    assert!(result.first_feed.is_some_and(|bytes| bytes < html.len()));
    result
}

// cov:ignore: benchmark harness; workspace coverage does not build this target.
fn batch(html: &str, resources: &RenderResources<'_>, verify: bool) -> Output {
    let document = parse_html_with_resources(html.as_bytes(), resources).expect("batch parse");
    let registrations = [ConsumerPropertyRegistration::integer("bookmark-level")];
    let mut events = Vec::new();
    let mut observer = |event| {
        if verify {
            events.push(event);
        }
        Ok(())
    };
    let LayoutStatus::Completed(laid_out) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new()
            .resources(resources)
            .consumer_properties(&registrations, &mut observer),
    )
    .expect("batch layout") else {
        panic!("batch aborted without a signal");
    };
    let text = if verify {
        laid_out
            .pages()
            .flat_map(|page| page.text_runs())
            .flat_map(|run| {
                run.text
                    .bytes()
                    .filter(|b| !b.is_ascii_whitespace())
                    .collect::<Vec<_>>()
            })
            .collect()
    } else {
        Vec::new()
    };
    Output {
        pages: laid_out.page_count(),
        text,
        events,
        ..Output::default()
    }
}

// cov:ignore: benchmark harness; workspace coverage does not build this target.
fn print_row(
    sections: usize,
    input: usize,
    mode: &str,
    nodes: usize,
    output: &Output,
    heap: &Heap,
    time: Duration,
) {
    println!(
        "sections={sections} input={input} mode={mode} parser_nodes={nodes} pages={} peak={} end_live={} alloc_bytes={} realloc_bytes={} alloc_calls={} realloc_calls={} ms={:.6} first_feed={:?} diagnostics={:?}",
        output.pages,
        heap.peak,
        heap.end_live,
        heap.alloc_bytes,
        heap.realloc_bytes,
        heap.alloc_calls,
        heap.realloc_calls,
        time.as_secs_f64() * 1000.0,
        output.first_feed,
        output.diagnostics
    );
}

// cov:ignore: benchmark harness; workspace coverage does not build this target.
fn main() {
    if !std::env::args().any(|arg| arg == "--bench") {
        return;
    }
    let lengths: Vec<usize> = std::env::var("RAIKIRI_STREAM_SECTIONS")
        .ok()
        .map(|list| {
            list.split(',')
                .map(|n| n.trim().parse().expect("section count"))
                .collect()
        })
        .unwrap_or_else(|| vec![200, 400, 800, 1600]);
    let resources = common::workloads::resources();
    let _ = batch(&text_document(1), &resources, false);
    for sections in lengths {
        let html = text_document(sections);
        let (nodes, heap, duration) = measure(|| {
            parse(html.as_bytes(), &ParseOptions::new())
                .expect("parse only")
                .dom
                .node_count()
        });
        print_row(
            sections,
            html.len(),
            "parse",
            nodes,
            &Output::default(),
            &heap,
            duration,
        );
        let expected = batch(&html, &resources, true);
        let actual = stream(&html, &resources, true);
        assert_eq!(actual.pages, expected.pages);
        assert_eq!(actual.text, expected.text);
        assert_eq!(actual.events, expected.events);
        assert_eq!(actual.events.len(), sections * 3);
        drop((actual, expected));
        let (output, heap, duration) = measure(|| batch(&html, &resources, false));
        print_row(
            sections,
            html.len(),
            "batch",
            nodes,
            &output,
            &heap,
            duration,
        );
        let (output, heap, duration) = measure(|| stream(&html, &resources, false));
        assert_eq!(output.diagnostics.parser_nodes, nodes as u64);
        print_row(
            sections,
            html.len(),
            "stream",
            nodes,
            &output,
            &heap,
            duration,
        );
        std::io::stdout().flush().expect("flush evidence");
    }
}
