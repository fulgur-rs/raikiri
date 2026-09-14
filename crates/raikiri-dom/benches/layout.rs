//! Layout throughput — `layout_single_page` micro-bench.
//!
//! ```text
//! cargo bench -p raikiri-dom --bench layout
//! ```

use criterion::Criterion;
use parley::FontContext;
use raikiri_dom::layout::layout_single_page;
use raikiri_html::{ParseOptions, parse};
use raikiri_style::{build_rule_tree, cascade};
use raikiri_traits::PageBox;

fn build_flat_html(n_divs: usize) -> Vec<u8> {
    let mut s = String::with_capacity(n_divs * 48 + 100);
    s.push_str("<html><head><style>div{width:100px;height:20px;margin:2px}</style></head><body>");
    for i in 0..n_divs {
        s.push_str(&format!("<div id=\"d{i}\">hello {i}</div>"));
    }
    s.push_str("</body></html>");
    s.into_bytes()
}

fn prepare(n_divs: usize) -> (Vec<u8>, PageBox) {
    let html = build_flat_html(n_divs);
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded = parse(html.as_slice(), &opts).expect("parse sanity");
    assert!(uncascaded.dom.node_count() > n_divs);
    (html, PageBox::A4)
}

fn bench_layout(c: &mut Criterion) {
    let mut group = c.benchmark_group("layout");
    let font_ctx = FontContext::new();
    for (name, n_divs) in [("layout_flat_500", 500u64), ("layout_flat_2000", 2000u64)] {
        let (html, page_box) = prepare(n_divs as usize);
        group.throughput(criterion::Throughput::Elements(n_divs));
        group.bench_function(name, |b| {
            b.iter_batched(
                || {
                    let opts = ParseOptions {
                        extra_stylesheets: &[],
                        network: None,
                        base_url: None,
                    };
                    let uncascaded = parse(html.as_slice(), &opts).expect("parse");
                    let rule_tree = build_rule_tree(&uncascaded.dom);
                    let cascade_result = cascade(&uncascaded.dom, &rule_tree).expect("cascade");
                    (uncascaded.dom, cascade_result)
                },
                |(mut doc, cascade_result)| {
                    let r =
                        layout_single_page(&mut doc, &cascade_result, page_box, font_ctx.clone());
                    let _ = std::hint::black_box(r);
                },
                criterion::BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

fn main() {
    let mut criterion = criterion::Criterion::default().configure_from_args();
    bench_layout(&mut criterion);
    criterion.final_summary();
}
