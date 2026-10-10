//! HTML parse throughput — `raikiri_html::parse` micro-bench.
//!
//! While `cascade.rs` guards the per-declaration cost of cascading and `walk.rs` guards
//! the per-element DFS cost of painting, this file guards the per-node cost of parsing
//! (`html5ever` tokenizer + `RaikiriTreeSink`).
//!
//! ```text
//! cargo bench -p raikiri-html --bench parse
//! ```

use criterion::{Criterion, Throughput};
use raikiri_html::{ParseOptions, parse};

fn build_flat_html(n_divs: usize) -> Vec<u8> {
    let mut s = String::with_capacity(n_divs * 32 + 100);
    s.push_str("<html><head></head><body>");
    for i in 0..n_divs {
        s.push_str(&format!("<div id=\"d{i}\"><p>hello world {i}</p></div>"));
    }
    s.push_str("</body></html>");
    s.into_bytes()
}

fn build_deep_html(depth: usize) -> Vec<u8> {
    let mut s = String::new();
    s.push_str("<html><head></head><body>");
    for _ in 0..depth {
        s.push_str("<div>");
    }
    s.push_str("<p>deep</p>");
    for _ in 0..depth {
        s.push_str("</div>");
    }
    s.push_str("</body></html>");
    s.into_bytes()
}

fn build_with_style(n_divs: usize) -> Vec<u8> {
    let mut s = String::with_capacity(n_divs * 32 + 300);
    s.push_str(
        "<html><head><style>p{color:red} div{margin:4px} .x{padding:2px}</style></head><body>",
    );
    for i in 0..n_divs {
        s.push_str(&format!("<div class=\"x\"><p>hello {i}</p></div>"));
    }
    s.push_str("</body></html>");
    s.into_bytes()
}

/// One `<style>` element holding `n_rules` rules, one per line. The tokenizer
/// splits raw text at every newline, so this is the shape that turns
/// quadratic if each piece is merged into the text node separately.
fn build_long_style(n_rules: usize) -> Vec<u8> {
    let mut s = String::with_capacity(n_rules * 24 + 100);
    s.push_str("<html><head><style>\n");
    for i in 0..n_rules {
        s.push_str(&format!(".c{i}{{color:red}}\n"));
    }
    s.push_str("</style></head><body><p>x</p></body></html>");
    s.into_bytes()
}

fn bench_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse");
    // `min_nodes` is the parse sanity floor; the long-style case parses to a
    // handful of nodes, so its throughput unit (rules) is not a node count.
    for (name, html, n_elems, min_nodes) in [
        ("parse_flat_500", build_flat_html(500), 500u64, 500),
        ("parse_flat_2000", build_flat_html(2000), 2000u64, 2000),
        ("parse_deep_100", build_deep_html(100), 100u64, 100),
        ("parse_with_style_500", build_with_style(500), 500u64, 500),
        (
            "parse_long_style_20000",
            build_long_style(20000),
            20000u64,
            4,
        ),
    ] {
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        let doc = parse(html.as_slice(), &opts).expect("parse sanity");
        assert!(doc.dom.node_count() > min_nodes);
        group.throughput(Throughput::Elements(n_elems));
        group.bench_function(name, |b| {
            b.iter(|| {
                let opts = ParseOptions {
                    extra_stylesheets: &[],
                    network: None,
                    base_url: None,
                };
                let doc = parse(html.as_slice(), &opts).expect("parse");
                std::hint::black_box(doc);
            });
        });
    }
    group.finish();
}

fn main() {
    let mut criterion = Criterion::default().configure_from_args();
    bench_parse(&mut criterion);
    criterion.final_summary();
}
