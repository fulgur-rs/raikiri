//! Real-Document pipeline stage separation: parse vs rule-tree vs cascade.
//!
//! While the style crate's `cascade` bench times the cascade over a
//! hand-built minimal DOM and the html crate's `parse` bench times
//! `raikiri_html::parse` alone, this file splits the real-Document pipeline
//! (parse a real HTML document, build the rule tree from its `<style>`
//! elements, cascade over the real arena with real attribute lookup) into
//! three separately timed stages. The split attributes each stage's time and
//! allocation cost so an optimization candidate can be ordered against the
//! stage it actually shortens.
//!
//! Arena sizes and peak RSS are reported once per size at setup (outside any
//! timed region) via `eprintln!`, so a baseline run records them alongside
//! the timing table.
//!
//! Run with `cargo bench -p raikiri-dom --bench pipeline`.

use criterion::{Criterion, Throughput}; // cov:ignore: bench harness import — bench target never instrumented
use raikiri_dom::Document; // cov:ignore: bench harness import — bench target never instrumented
use raikiri_html::{ParseOptions, parse}; // cov:ignore: bench harness import — bench target never instrumented
// cov:ignore: bench harness import — bench target never instrumented
use raikiri_style::{
    CascadeResult, ComputedLengthPercentageOrAuto, ComputedValues, RuleTree, StyleDom,
    StyleElement, StyleNode, build_rule_tree, cascade,
};

// cov:ignore: bench harness — bench target never built under `cargo test` / `cargo llvm-cov --workspace`
fn parse_options() -> ParseOptions<'static> {
    ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    }
}

/// Flat real HTML: `n_divs` `div`s under `body`, each carrying an `id`, a
/// shared `hit` class, and a data attribute, under a `<style>` element that
/// mixes a type rule, a class rule, an id rule, and an `nth-child` rule.
/// Every selector family the cascade must resolve therefore exercises the
/// real-DOM lookup path (`id()` / `has_class()` / `attr()` on parsed
/// attribute storage, sibling scans over parsed children) rather than a
/// mock.
// cov:ignore: bench harness — bench target never built under `cargo test` / `cargo llvm-cov --workspace`
fn build_pipeline_html(n_divs: usize) -> Vec<u8> {
    let mut s = String::with_capacity(n_divs * 80 + 300);
    s.push_str(
        "<html><head><style>         div{margin:2px}          .hit{margin-top:7px}          #d7{font-size:20px}          div:nth-child(2n+1){color:rgb(9,8,7)}         </style></head><body>",
    );
    for i in 0..n_divs {
        s.push_str(&format!(
            "<div id=\"d{i}\" class=\"hit\" data-k=\"v{i}\">hello {i}</div>"
        ));
    }
    s.push_str("</body></html>");
    s.into_bytes()
}

// cov:ignore: bench harness — bench target never built under `cargo test` / `cargo llvm-cov --workspace`
fn peak_rss_kb() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("VmHWM:") {
                return rest.split_whitespace().next()?.parse().ok();
            }
        }
        None
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

// cov:ignore: bench harness — bench target never built under `cargo test` / `cargo llvm-cov --workspace`
fn report_pipeline_sizes(label: &str, dom: &Document, tree: &RuleTree, result: &CascadeResult) {
    let peak = peak_rss_kb().map_or("n/a".to_string(), |kb| format!("{kb} kB"));
    eprintln!(
        "pipeline[{label}]: nodes={} rules={} computed_bytes={} peak_rss={}",
        dom.node_count(),
        tree.style_rules().len(),
        result.computed.len() * std::mem::size_of::<ComputedValues>(),
        peak,
    );
}

/// Prove the parsed pipeline is the one it claims to be: the rule tree holds
/// the three `<style>` rules, and a sweep over the `body` `div`s checks the
/// id winner (`#d7` → 20px), the class winner (every `div` → 7px top margin
/// via real attribute lookup), and the structural winner (odd positions →
/// the `nth-child` color, even positions → initial).
// cov:ignore: bench harness — bench target never built under `cargo test` / `cargo llvm-cov --workspace`
fn probe_pipeline(dom: &Document, result: &CascadeResult, n_divs: usize, tree: &RuleTree) {
    assert_eq!(
        tree.style_rules().len(),
        4,
        "pipeline <style> did not parse into four rules"
    );
    assert_eq!(
        result.computed.len(),
        dom.node_count(),
        "cascade did not produce one entry per arena node"
    );
    let initial = ComputedValues::initial();
    let want_color = raikiri_style::CssColor {
        r: 9,
        g: 8,
        b: 7,
        a: 255,
    };

    let mut body_children = Vec::new();
    for idx in 0..dom.node_count() {
        let id = raikiri_style::StyleNodeId::new(idx as u64);
        if let Some(node) = dom.node(id)
            && let Some(elem) = node.as_element()
            && elem.tag_name() == "body"
        {
            body_children = dom.child_ids(id).collect::<Vec<_>>();
            break;
        }
    }
    let divs: Vec<_> = body_children
        .iter()
        .filter_map(|id| {
            dom.node(*id).and_then(|node| {
                node.as_element().and_then(|elem| {
                    (elem.tag_name() == "div").then_some((*id, elem.id().map(str::to_string)))
                })
            })
        })
        .collect();
    assert_eq!(divs.len(), n_divs, "body does not hold n_divs divs");

    for (k, (id, _)) in divs.iter().enumerate() {
        let cv = &result.computed[id.0 as usize];
        // Id rule: only `#d7` (the 8th div, 0-indexed 7) wins 20px.
        let want_font = if k == 7 { 20.0 } else { initial.font_size.px() };
        assert_eq!(
            cv.font_size.px(),
            want_font,
            "div {k} of {n_divs}: id-rule font-size mismatch"
        );
        // Class rule: every div carries `hit` in real parsed storage.
        assert_eq!(
            cv.margin.top,
            ComputedLengthPercentageOrAuto::Px(7.0),
            "div {k} of {n_divs}: class-rule margin-top missing — real-DOM class lookup did not match"
        );
        // Structural rule: odd sibling positions (0-indexed even `k`).
        if k % 2 == 0 {
            assert_eq!(
                cv.color, want_color,
                "odd-position div {k} of {n_divs} should carry the nth-child color"
            );
        } else {
            assert_eq!(
                cv.color, initial.color,
                "even-position div {k} of {n_divs} must keep the initial color"
            );
        }
    }
}

// cov:ignore: bench harness — bench target never built under `cargo test` / `cargo llvm-cov --workspace`
fn prepare(n_divs: usize) -> (Vec<u8>, Document, RuleTree, CascadeResult) {
    let html = build_pipeline_html(n_divs);
    let uncascaded = parse(html.as_slice(), &parse_options()).expect("parse sanity");
    let tree = build_rule_tree(&uncascaded.dom);
    let result = cascade(&uncascaded.dom, &tree).expect("cascade sanity");
    probe_pipeline(&uncascaded.dom, &result, n_divs, &tree);
    report_pipeline_sizes(&format!("setup_{n_divs}"), &uncascaded.dom, &tree, &result);
    (html, uncascaded.dom, tree, result)
}

// cov:ignore: bench harness — bench target never built under `cargo test` / `cargo llvm-cov --workspace`
fn bench_pipeline(c: &mut Criterion) {
    let mut group = c.benchmark_group("pipeline");
    for n_divs in [500usize, 2000] {
        let (html, dom, tree, result) = prepare(n_divs);
        drop(result);

        group.throughput(Throughput::Elements(n_divs as u64));
        group.bench_function(format!("pipeline_parse_{n_divs}"), |b| {
            b.iter(|| {
                let uncascaded = parse(html.as_slice(), &parse_options()).expect("parse");
                std::hint::black_box(uncascaded);
            });
        });
        group.bench_function(format!("pipeline_rule_tree_{n_divs}"), |b| {
            b.iter(|| {
                let tree = build_rule_tree(&dom);
                std::hint::black_box(tree);
            });
        });
        group.bench_function(format!("pipeline_cascade_{n_divs}"), |b| {
            b.iter_with_large_drop(|| cascade(&dom, &tree).expect("cascade"));
        });
    }
    group.finish();
}

// cov:ignore: bench harness — bench target never built under `cargo test` / `cargo llvm-cov --workspace`
fn main() {
    let mut criterion = Criterion::default().configure_from_args();
    bench_pipeline(&mut criterion);
    criterion.final_summary();
}
