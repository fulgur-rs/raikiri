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

/// A page of `n` paragraphs, each long enough to wrap and holding one inline
/// element. The block displays are authored: the cascade used here does not
/// apply the user-agent sheet, and an inline `<p>` is not a paragraph root.
fn build_paragraph_html(n: usize) -> Vec<u8> {
    let mut html =
        String::from("<html><head><style>html,body,p{display:block}</style></head><body>");
    for i in 0..n {
        html.push_str(&format!(
            "<p>paragraph {i} of the benchmark document with enough words to wrap across \
             several lines when the page is narrow, followed by <span>an inline span</span> \
             and some closing text.</p>"
        ));
    }
    html.push_str("</body></html>");
    html.into_bytes()
}

/// `layout_single_page` with the shodo inline engine on, over a bundled-only
/// (Ahem) font collection, with the paragraphs built in sequence and, for the
/// `_parallel` cases, on several threads. The parley side still gets a system font context,
/// so compare these numbers only with each other, not with `layout`.
fn bench_layout_ifc(c: &mut Criterion) {
    let mut group = c.benchmark_group("layout_ifc");
    let fonts_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/text-autospace");
    let shared = raikiri_dom::build_wpt_font_collection(&fonts_dir).expect("font collection");
    let font_ctx = FontContext::new();
    for (name, n, parallel) in [
        ("paragraphs_100", 100usize, false),
        ("paragraphs_1000", 1000usize, false),
        ("paragraphs_100_parallel", 100usize, true),
        ("paragraphs_1000_parallel", 1000usize, true),
    ] {
        let html = build_paragraph_html(n);
        let page_box = PageBox::A4;
        group.throughput(criterion::Throughput::Elements(n as u64));
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
                    let mut dom = uncascaded.dom;
                    dom.enable_inline_formatting(shared.clone(), shodo::limits::Limits::default());
                    // The collection holds Ahem only, no system faces.
                    dom.set_ifc_parallel_build(parallel);
                    (dom, cascade_result)
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
    bench_layout_ifc(&mut criterion);
    criterion.final_summary();
}
