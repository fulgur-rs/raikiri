//! Streaming throughput and time to the first page callback with 4 KiB input chunks.
//! First-page timing ends at callback entry, then drains/validates the stream untimed.
//! It measures warm-font CPU latency, not network delay or PDF output.
//! First-page fixtures exceed the production checkpoint interval and every
//! sample requires delivery during a write with more input still to follow.

mod common {
    pub mod workloads;
}

use criterion::{Criterion, Throughput};
use raikiri_html::{
    ConsumerPropertyEvent, LayoutConfig, PageDefaults, PageSink, RenderResources, StreamPage,
    StreamStatus, StreamSummary, StreamingLayout,
};
use std::{
    cell::{Cell, OnceCell},
    hint::black_box,
    io::Write as _,
    rc::Rc,
    time::{Duration, Instant},
};

struct Sink {
    start: Instant,
    first: Option<Duration>,
    pages: u32,
    text: Option<Vec<u8>>,
    fed_bytes: Rc<Cell<Option<usize>>>,
    first_feed_bytes: Option<usize>,
}

struct StreamResult {
    pages: u32,
    first: Duration,
    text: Vec<u8>,
    first_feed_bytes: Option<usize>,
}

impl StreamResult {
    fn check_incremental(&self, input_bytes: usize) {
        assert!(
            self.first_feed_bytes
                .is_some_and(|bytes| bytes < input_bytes),
            "the first page must be delivered while writing, before EOF"
        );
    }
}

impl PageSink for Sink {
    type Output = StreamResult;

    fn page(
        &mut self,
        page: StreamPage<'_>,
        _events: Vec<ConsumerPropertyEvent>,
    ) -> std::io::Result<()> {
        if self.first.is_none() {
            self.first = Some(self.start.elapsed());
            self.first_feed_bytes = self.fed_bytes.get();
        }
        assert_eq!(page.index(), self.pages, "pages are delivered in order");
        self.pages += 1;
        if let Some(text) = &mut self.text {
            for run in page.page().text_runs() {
                assert!(!run.glyphs.is_empty(), "streamed text run has no glyphs");
                text.extend(run.text.bytes().filter(|byte| !byte.is_ascii_whitespace()));
            }
        }
        Ok(())
    }

    fn finish(self, summary: StreamSummary) -> std::io::Result<Self::Output> {
        assert_eq!(self.pages, summary.page_count);
        Ok(StreamResult {
            pages: self.pages,
            first: self.first.expect("at least one page is delivered"),
            text: self.text.unwrap_or_default(),
            first_feed_bytes: self.first_feed_bytes,
        })
    }
}

fn stream(html: &str, resources: &RenderResources<'_>, verify_text: bool) -> StreamResult {
    let fed_bytes = Rc::new(Cell::new(None));
    let sink = Sink {
        start: Instant::now(),
        first: None,
        pages: 0,
        text: verify_text.then(Vec::new),
        fed_bytes: fed_bytes.clone(),
        first_feed_bytes: None,
    };
    let mut stream = StreamingLayout::new(
        resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        sink,
    );
    for (index, chunk) in html.as_bytes().chunks(4096).enumerate() {
        // Record the end of this write; a callback cannot be counted as early
        // if it occurs during the final write or during finish().
        fed_bytes.set(Some(index * 4096 + chunk.len()));
        stream.write_all(black_box(chunk)).expect("stream input");
    }
    fed_bytes.set(None);
    let StreamStatus::Completed(result) = stream.finish().expect("stream finish") else {
        panic!("stream aborted without a signal");
    };
    result
}

fn bench_stream(c: &mut Criterion) {
    let resources = common::workloads::resources();
    for stage in ["streaming_document", "first_page_document"] {
        let mut group = c.benchmark_group(stage);
        group.sample_size(10);
        let page_counts = if stage == "first_page_document" {
            // Even the smallest input crosses the production 64 KiB checkpoint.
            [1500, 3000, 6000]
        } else {
            [10, 100, 500]
        };
        for pages in page_counts {
            let slot = OnceCell::new();
            // Only full-stream throughput has a pages/sec denominator.
            if stage == "streaming_document" {
                group.throughput(Throughput::Elements(pages as u64));
            }
            group.bench_function(format!("pages_{pages}"), |b| {
                let html = slot.get_or_init(|| {
                    let html = common::workloads::pages(pages);
                    let mut result = stream(&html, &resources, true);
                    assert_eq!(
                        result.pages as usize, pages,
                        "streaming preserves forced pages"
                    );
                    if stage == "first_page_document" {
                        result.check_incremental(html.len());
                        eprintln!(
                            "{stage}/pages_{pages}: input_bytes={}, first_callback_write_end_bytes={}",
                            html.len(),
                            result.first_feed_bytes.expect("incremental delivery checked")
                        );
                    }
                    result.text.sort_unstable();
                    let document = common::workloads::parse(&html, &resources);
                    assert_eq!(
                        result.text,
                        common::workloads::text_signature(document.dom()),
                        "streamed output preserves text"
                    );
                    html
                });
                if stage == "streaming_document" {
                    b.iter(|| black_box(stream(html, &resources, false)));
                } else {
                    b.iter_custom(|iterations| {
                        let mut total = Duration::ZERO;
                        for _ in 0..iterations {
                            let result = stream(html, &resources, false);
                            assert_eq!(result.pages as usize, pages);
                            result.check_incremental(html.len());
                            total += result.first;
                        }
                        total
                    });
                }
            });
        }
        group.finish();
    }
}

fn main() {
    let mut c = Criterion::default().configure_from_args();
    bench_stream(&mut c);
    c.final_summary();
}
