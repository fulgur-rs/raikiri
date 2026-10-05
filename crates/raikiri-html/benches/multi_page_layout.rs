//! Multi-page document layout — `raikiri_html::layout` macro-bench.
//!
//! Guards the per-page cost of the paged pipeline: page geometry is resolved
//! for every page slice in several bounded passes, so per-page work that
//! scales with the whole document shows up here as page count grows.
//!
//! ```text
//! cargo bench -p raikiri-html --bench multi_page_layout
//! ```

use criterion::{Criterion, Throughput};
use raikiri_html::{
    LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults, RenderResources, layout,
    parse_html_with_resources,
};

fn build_paged_html(n_pages: usize) -> String {
    let mut s = String::with_capacity(n_pages * 600 + 400);
    s.push_str(
        "<html><head><style>\
         html { font-size: 12px }\
         @page { size: 400px 300px; margin: 2em }\
         @page :left { margin-left: 30px }\
         @page :right { margin-right: 3em }\
         p { margin: 0 0 4px }\
         section { break-after: page }\
         </style></head><body>",
    );
    for page in 0..n_pages {
        s.push_str("<section>");
        for line in 0..4 {
            s.push_str(&format!(
                "<p>Page {page} paragraph {line}: the quick brown fox jumps over the lazy dog.</p>"
            ));
        }
        s.push_str("</section>");
    }
    s.push_str("</body></html>");
    s
}

fn bench_multi_page_layout(c: &mut Criterion) {
    let mut group = c.benchmark_group("multi_page_layout");
    let resources = RenderResources::new();
    for (name, n_pages) in [
        ("multi_page_layout_10", 10usize),
        ("multi_page_layout_30", 30usize),
    ] {
        let html = build_paged_html(n_pages);
        let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse sanity");
        let LayoutStatus::Completed(result) = layout(
            &doc,
            PageDefaults::default(),
            LayoutConfig::default(),
            LayoutOptions::new(),
        )
        .expect("layout sanity") else {
            panic!("layout aborted without a signal");
        };
        assert!(result.page_count() as usize >= n_pages);
        group.throughput(Throughput::Elements(n_pages as u64));
        group.bench_function(name, |b| {
            b.iter(|| {
                let status = layout(
                    &doc,
                    PageDefaults::default(),
                    LayoutConfig::default(),
                    LayoutOptions::new(),
                )
                .expect("layout");
                std::hint::black_box(status);
            });
        });
    }
    group.finish();
}

fn main() {
    let mut criterion = Criterion::default().configure_from_args();
    bench_multi_page_layout(&mut criterion);
    criterion.final_summary();
}
