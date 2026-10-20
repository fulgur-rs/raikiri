//! Measures projection and all-page text/paint preparation independently of
//! parsing, cascading and layout. Run with `cargo bench -p raikiri-dom --bench
//! page_preparation`; `RAIKIRI_BENCH_ROUNDS` controls the median sample count.

// cov:ignore: benchmark harness runs under cargo bench, outside the instrumented test targets.
use std::hint::black_box;
// cov:ignore: benchmark harness runs under cargo bench, outside the instrumented test targets.
use std::time::Instant;

// cov:ignore: benchmark harness runs under cargo bench, outside the instrumented test targets.
use raikiri_dom::{Document, PageSlice, build_wpt_font_collection, layout_pages};
// cov:ignore: benchmark harness runs under cargo bench, outside the instrumented test targets.
use raikiri_html::{ParseOptions, parse};
// cov:ignore: benchmark harness runs under cargo bench, outside the instrumented test targets.
use raikiri_style::{CascadeResult, build_rule_tree, cascade};
// cov:ignore: benchmark harness runs under cargo bench, outside the instrumented test targets.
use raikiri_traits::PageBox;

// cov:ignore: benchmark harness runs under cargo bench, outside the instrumented test targets.
fn fixture(kind: &str, pages: usize) -> (Document, CascadeResult, PageBox, Vec<PageSlice>, usize) {
    let mut html = String::from(
        "<html><head><style>html,body,div{display:block;margin:0;padding:0}head{display:none}body{font:10px/10px Ahem}div{width:10px}",
    );
    if matches!(kind, "spans" | "opacity") {
        html.push_str("div::first-line{opacity:.5}");
    }
    html.push_str("</style></head><body>");
    let expected = match kind {
        "plain" => {
            html.push_str("<div>");
            html.push_str(&vec!["X"; pages * 10].join(" "));
            html.push_str("</div>");
            pages * 10
        }
        "spans" | "opacity" => {
            html.push_str("<div>");
            for index in 0..pages * 10 {
                let style = if kind == "opacity" {
                    "opacity:.5"
                } else if index % 2 == 0 {
                    "color:blue"
                } else {
                    "color:red"
                };
                html.push_str(&format!("<span style='{style}'>X </span>"));
            }
            html.push_str("</div>");
            pages * 10
        }
        "columns" => {
            for _ in 0..pages {
                html.push_str("<div style='width:30px;height:100px;column-count:2;column-gap:10px;column-fill:auto'>");
                html.push_str(&["X"; 20].join(" "));
                html.push_str("</div>");
            }
            pages * 20
        }
        _ => unreachable!(),
    };
    html.push_str("</body></html>");
    let parsed = parse(
        html.as_bytes(),
        &ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        },
    )
    .unwrap();
    let mut doc = parsed.dom;
    doc.set_font_collection(
        build_wpt_font_collection(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/data/text-autospace"
        )))
        .unwrap(),
    );
    let cascade = cascade(&doc, &build_rule_tree(&doc)).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(&mut doc, &cascade, page).unwrap();
    assert_eq!(slices.len(), pages, "{kind}: actual page count");
    (doc, cascade, page, slices, expected)
}

// cov:ignore: benchmark harness runs under cargo bench, outside the instrumented test targets.
fn prepare_pages(doc: &Document, cascade: &CascadeResult, slices: &[PageSlice]) -> (usize, usize) {
    let mut glyphs = 0;
    let mut events = 0;
    for slice in slices {
        let runs = doc.page_text_runs(cascade, slice.page_index);
        glyphs += runs
            .iter()
            .map(|run| run.text.matches('X').count())
            .sum::<usize>();
        events += doc
            .page_paint_order_for_text_runs(cascade, slice.page_index, &runs)
            .len();
        black_box(runs);
    }
    (glyphs, events)
}

// cov:ignore: benchmark harness runs under cargo bench, outside the instrumented test targets.
fn median(samples: &mut [f64]) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

// cov:ignore: benchmark harness runs under cargo bench, outside the instrumented test targets.
fn main() {
    let rounds =
        std::env::var("RAIKIRI_BENCH_ROUNDS").map_or(9, |value| value.parse::<usize>().unwrap());
    assert!(rounds > 0);
    println!("kind,pages,nodes,glyphs,events,projection_us,cold_all_pages_us,warm_all_pages_us");
    for kind in ["plain", "spans", "opacity", "columns"] {
        for pages in [8, 32, 128] {
            let (source, cascade, page, slices, expected) = fixture(kind, pages);
            let mut projection = Vec::new();
            let mut cold = Vec::new();
            let mut warm = Vec::new();
            let mut fingerprint = None;
            for _ in 0..rounds {
                // Cloning the laid-out source is outside every timed stage.
                let mut doc = source.clone();
                let start = Instant::now();
                doc.project_pages(&cascade, page, &slices, &[]).unwrap();
                projection.push(start.elapsed().as_secs_f64() * 1e6);
                let start = Instant::now();
                let actual = prepare_pages(&doc, &cascade, &slices);
                cold.push(start.elapsed().as_secs_f64() * 1e6);
                assert_eq!(actual.0, expected);
                if let Some(previous) = fingerprint {
                    assert_eq!(actual, previous);
                }
                fingerprint = Some(actual);
                let start = Instant::now();
                let repeated = prepare_pages(&doc, &cascade, &slices);
                warm.push(start.elapsed().as_secs_f64() * 1e6);
                assert_eq!(repeated, actual);
            }
            let (glyphs, events) = fingerprint.unwrap();
            println!(
                "{kind},{pages},{},{glyphs},{events},{:.3},{:.3},{:.3}",
                source.node_count(),
                median(&mut projection),
                median(&mut cold),
                median(&mut warm)
            );
        }
    }
}
