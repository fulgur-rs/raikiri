//! Cascade time of a report-style document parsed through raikiri-html.
//!
//! Unlike the synthetic workloads of raikiri-style's own cascade benchmark,
//! this one cascades a realistic document: the user agent stylesheet plus a
//! report stylesheet over pages of item tables. The timed closure includes
//! dropping the result, because the cascade's per-node allocations make the
//! drop a real part of its cost.
//!
//! Run with:
//!
//! ```text
//! cargo bench -p raikiri-bench --bench cascade_document
//! ```

// cov:ignore: bench harness — `cargo test`/`cargo llvm-cov --workspace` never
// build this bench target, so nothing in this file has coverage
// instrumentation to attribute to.
mod common {
    pub mod report;
}

use std::cell::OnceCell;

use criterion::{Criterion, Throughput};
use raikiri_html::{ParseOptions, UncascadedDocument};
use raikiri_style::{CssColor, MediaContext, RuleTree};

/// Color of the `td.warn` cells, which only an important author declaration
/// gives them.
// cov:ignore: bench harness — same reason as `common` above.
const WARN: CssColor = CssColor {
    r: 0xb0,
    g: 0x00,
    b: 0x20,
    a: 0xff,
};

/// The parsed report and its rule tree, checked once so the benchmark cannot
/// silently time a cascade that drops the author stylesheet.
// cov:ignore: bench harness — same reason as `common` above.
fn parsed(pages: usize, rows: usize) -> (UncascadedDocument, RuleTree) {
    let html = common::report::report_html(pages, rows);
    let options = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let document = raikiri_html::parse(html.as_bytes(), &options).expect("the report parses");
    let tree = raikiri_html::build_rule_tree(&document);
    let probe =
        raikiri_style::cascade_with_media_context(&document.dom, &tree, &MediaContext::print())
            .expect("the cascade succeeds");
    assert_eq!(probe.computed.len(), document.dom.node_count());
    let warned = probe.computed.iter().filter(|cv| cv.color == WARN).count();
    assert!(
        warned >= pages * (rows / 7),
        "only {warned} nodes have the warning color"
    );
    (document, tree)
}

// cov:ignore: bench harness — same reason as `common` above.
fn bench_cascade_document(c: &mut Criterion) {
    let mut group = c.benchmark_group("cascade_document");
    let media = MediaContext::print();
    for (name, pages, rows, samples) in [
        ("report_10x50", 10usize, 50usize, 50usize),
        ("report_40x100", 40, 100, 10),
    ] {
        // Throughput in item rows; the document is built the first time its
        // benchmark runs, so a name filter skips the other configuration.
        let slot = OnceCell::new();
        group.sample_size(samples);
        group.throughput(Throughput::Elements((pages * rows) as u64));
        group.bench_function(name, |b| {
            let (document, tree) = slot.get_or_init(|| parsed(pages, rows));
            b.iter(|| {
                raikiri_style::cascade_with_media_context(&document.dom, tree, &media)
                    .expect("the cascade succeeds")
            });
        });
    }
    group.finish();
}

// criterion's `criterion_main!`/`criterion_group!` pair is inlined here: the
// macro generates a `pub fn`, which trips the workspace's
// `missing_docs = "warn"` (an error under the gate's `-D warnings`), and the
// `allow` cannot be attached to a macro invocation.
//
// cov:ignore: bench harness — same reason as `common` above.
fn main() {
    let mut criterion = Criterion::default().configure_from_args();
    bench_cascade_document(&mut criterion);
    criterion.final_summary();
}
