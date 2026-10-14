//! HTML parsing and CSS rule-tree construction of the same report as cascade_document.
//! Input generation and probes are untimed; output destruction is timed.

mod common {
    pub mod report;
}

use std::{cell::OnceCell, hint::black_box};

use criterion::{Criterion, Throughput};
use raikiri_html::{ParseOptions, build_rule_tree, parse};
use raikiri_style::{MediaContext, cascade_with_media_context};

fn bench_prepare(c: &mut Criterion) {
    let options = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    for stage in ["parse_document", "rule_tree_document"] {
        let mut group = c.benchmark_group(stage);
        group.sample_size(20);
        for (pages, rows) in [(10, 50), (40, 100)] {
            let slot = OnceCell::new();
            group.throughput(Throughput::Elements((pages * rows) as u64));
            group.bench_function(format!("report_{pages}x{rows}"), |b| {
                let (html, document) = slot.get_or_init(|| {
                    let html = common::report::report_html(pages, rows);
                    let document = parse(html.as_bytes(), &options).expect("the report parses");
                    let tree = build_rule_tree(&document);
                    let result =
                        cascade_with_media_context(&document.dom, &tree, &MediaContext::print())
                            .expect("the report cascades");
                    common::report::check_report(&result, pages, rows);
                    (html, document)
                });
                if stage == "parse_document" {
                    b.iter(|| {
                        black_box(parse(black_box(html.as_bytes()), &options).expect("parse"))
                    });
                } else {
                    b.iter(|| black_box(build_rule_tree(black_box(document))));
                }
            });
        }
        group.finish();
    }
}

fn main() {
    let mut c = Criterion::default().configure_from_args();
    bench_prepare(&mut c);
    c.final_summary();
}
