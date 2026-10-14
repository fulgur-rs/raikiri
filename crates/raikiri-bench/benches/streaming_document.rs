//! Streaming throughput and time to the first page callback with 4 KiB input chunks.
//! First-page timing ends at callback entry, then drains/validates the stream untimed.
//! It measures warm-font CPU latency, not network delay or PDF output.

mod common {
    pub mod workloads;
}

use criterion::{Criterion, Throughput};
use raikiri_html::{
    ConsumerPropertyEvent, LayoutConfig, PageDefaults, PageSink, RenderResources, StreamPage,
    StreamStatus, StreamSummary, StreamingLayout,
};
use std::{
    cell::OnceCell,
    hint::black_box,
    io::Write as _,
    time::{Duration, Instant},
};

struct Sink {
    start: Instant,
    first: Option<Duration>,
    pages: u32,
    text: Option<Vec<u8>>,
}

impl PageSink for Sink {
    type Output = (u32, Duration, Vec<u8>);

    fn page(
        &mut self,
        page: StreamPage<'_>,
        _events: Vec<ConsumerPropertyEvent>,
    ) -> std::io::Result<()> {
        self.first.get_or_insert_with(|| self.start.elapsed());
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
        Ok((
            self.pages,
            self.first.expect("at least one page is delivered"),
            self.text.unwrap_or_default(),
        ))
    }
}

fn stream(
    html: &str,
    resources: &RenderResources<'_>,
    verify_text: bool,
) -> (u32, Duration, Vec<u8>) {
    let sink = Sink {
        start: Instant::now(),
        first: None,
        pages: 0,
        text: verify_text.then(Vec::new),
    };
    let mut stream = StreamingLayout::new(
        resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        sink,
    );
    for chunk in html.as_bytes().chunks(4096) {
        stream.write_all(black_box(chunk)).expect("stream input");
    }
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
        for pages in [10, 100, 500] {
            let slot = OnceCell::new();
            // Only full-stream throughput has a pages/sec denominator.
            if stage == "streaming_document" {
                group.throughput(Throughput::Elements(pages as u64));
            }
            group.bench_function(format!("pages_{pages}"), |b| {
                let html = slot.get_or_init(|| {
                    let html = common::workloads::pages(pages);
                    let mut result = stream(&html, &resources, true);
                    assert_eq!(result.0 as usize, pages, "streaming preserves forced pages");
                    result.2.sort_unstable();
                    let document = common::workloads::parse(&html, &resources);
                    assert_eq!(
                        result.2,
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
                            let (count, first, _) = stream(html, &resources, false);
                            assert_eq!(count as usize, pages);
                            total += first;
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
