//! Compare head-only streaming feed costs with batch parsing.

use std::time::Instant;

use raikiri_html::{
    ConsumerPropertyEvent, LayoutConfig, PageDefaults, PageSink, RenderResources, StreamPage,
    StreamSummary, StreamingLayout,
};
struct NoPages;

impl PageSink for NoPages {
    type Output = ();

    fn page(&mut self, _: StreamPage<'_>, _: Vec<ConsumerPropertyEvent>) -> std::io::Result<()> {
        panic!("a head-only feed must not deliver a page");
    }

    fn finish(self, _: StreamSummary) -> std::io::Result<()> {
        panic!("the probe measures feed without finish");
    }
}

fn main() {
    for n in [512, 1024, 2048, 4096] {
        let mut html = String::from("<!doctype html><html><head>");
        let content = "x".repeat(950);
        for i in 0..n {
            html.push_str(&format!("<meta name='m{i}' content='{content}'>"));
        }
        html.push_str("</head>");
        let resources = RenderResources::new();
        for mode in ["stream_feed", "batch_parse"] {
            let mut samples = Vec::new();
            for _ in 0..5 {
                let elapsed = if mode == "stream_feed" {
                    let mut stream = StreamingLayout::new(
                        &resources,
                        PageDefaults::default(),
                        LayoutConfig::default(),
                        NoPages,
                    );
                    let start = Instant::now();
                    for chunk in html.as_bytes().chunks(4096) {
                        stream.feed(chunk).expect("head feed");
                    }
                    let elapsed = start.elapsed();
                    drop(stream);
                    elapsed
                } else {
                    let start = Instant::now();
                    let parsed = raikiri_html::parse(html.as_bytes(), &Default::default())
                        .expect("batch parse");
                    let elapsed = start.elapsed();
                    assert!(parsed.dom.node_count() >= n + 3);
                    drop(parsed);
                    elapsed
                };
                samples.push(elapsed.as_secs_f64() * 1000.0);
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "head mode={mode} n={n} bytes={} median_ms={:.6} samples={samples:?}",
                html.len(),
                samples[2]
            );
        }
    }
}
