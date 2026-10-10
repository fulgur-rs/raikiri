//! Scaling of multicolumn layout with a trailing `column-span: all` element.
//!
//! The ignored probe prints layout time and peak RSS per case. Run it with:
//! `cargo test --release -p raikiri-html --test multicol_scaling -- --ignored --nocapture`

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};
use std::time::Instant;

/// Peak resident set size of this process, in KiB (Linux only).
fn peak_rss_kib() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))?
        .trim()
        .trim_end_matches(" kB")
        .parse()
        .ok()
}

fn lay_out(page: &str, css: &str, body: &str) -> (std::time::Duration, u32) {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    // `MULTICOL_NO_SPANNER=1` drops the trailing spanner for comparison.
    let spanner = if std::env::var_os("MULTICOL_NO_SPANNER").is_some() {
        ""
    } else {
        "<h2 style=\"column-span:all\">S</h2>"
    };
    let html = format!(
        "<!doctype html><style>@page{{size:{page};margin:0}}body{{margin:0;font:10px/10px Ahem}}p,h2{{margin:0;font:inherit}}{css}</style><div class=mc>{body}{spanner}</div>"
    );
    let parsed = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let started = Instant::now();
    let LayoutStatus::Completed(document) = layout(
        &parsed,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("completed layout")
    };
    (started.elapsed(), document.page_count())
}

// Paginating the column group before a spanner fills one page at a time. Each
// page must cost its own paragraphs, not the whole group's, or a long group
// exhausts the layout work budget.
#[test]
fn a_long_paged_group_before_a_spanner_stays_within_the_work_budget() {
    let (page, css, body) = case("paged", 1000);
    let (_, pages) = lay_out(&page, &css, &body);
    assert_eq!(pages, 13);
}

fn case(kind: &str, n: usize) -> (String, String, String) {
    match kind {
        // One single-line paragraph per column.
        "columns" => (
            format!("{}px 2000px", n * 10),
            format!(".mc{{column-count:{n};column-gap:0}}"),
            "<p>A</p>".repeat(n),
        ),
        // One paragraph whose lines spread over every column.
        "paragraph" => (
            format!("{}px 2000px", n * 10),
            format!(".mc{{column-count:{n};column-gap:0}}"),
            format!("<p>{}</p>", vec!["A"; n].join("<br>")),
        ),
        // A wrapper chain of depth n around a paragraph with inline boxes.
        "depth" => (
            "400px 2000px".into(),
            ".mc{column-count:4;column-gap:0}".into(),
            format!(
                "{}<p>{}</p>{}",
                "<div>".repeat(n),
                vec!["<span>A</span>"; 64].join("<br>"),
                "</div>".repeat(n)
            ),
        ),
        // n inline boxes spread over a fixed column count.
        "inline" => (
            "400px 100000px".into(),
            ".mc{column-count:4;column-gap:0}".into(),
            format!("<p>{}</p>", vec!["<span>A</span>"; n].join("<br>")),
        ),
        // n paragraphs paginated into column groups before the spanner.
        "paged" => (
            "400px 200px".into(),
            ".mc{column-count:4;column-gap:0}".into(),
            "<p>A</p>".repeat(n),
        ),
        // A wrapped paragraph whose lines spread over n columns.
        "wrapped" => (
            format!("{}px 2000px", n * 10),
            format!(".mc{{column-count:{n};column-gap:0}}"),
            format!("<div><p>{}</p></div>", vec!["A"; n].join("<br>")),
        ),
        // 2000 paragraphs at the bottom of a wrapper chain of depth n.
        "deep-wide" => (
            "400px 100000px".into(),
            ".mc{column-count:4;column-gap:0}".into(),
            format!(
                "{}{}{}",
                "<div>".repeat(n),
                "<p>A</p>".repeat(2000),
                "</div>".repeat(n)
            ),
        ),
        // The same inline boxes without columns, as a control.
        "inline-control" => (
            "400px 100000px".into(),
            String::new(),
            format!("<p>{}</p>", vec!["<span>A</span>"; n].join("<br>")),
        ),
        _ => panic!("unknown case {kind}"),
    }
}

#[test]
#[ignore = "performance probe"]
fn multicol_scaling_probe() {
    // `MULTICOL_CASE=kind:n` runs one case so its peak RSS is isolated.
    let selected = std::env::var("MULTICOL_CASE").ok();
    let cases: Vec<(String, usize)> = match &selected {
        Some(value) => {
            let (kind, n) = value.split_once(':').expect("kind:n");
            vec![(kind.to_string(), n.parse().expect("size"))]
        }
        None => [
            "columns",
            "paragraph",
            "wrapped",
            "depth",
            "deep-wide",
            "inline",
            "paged",
        ]
        .into_iter()
        .flat_map(|kind| [64, 256].map(|n| (kind.to_string(), n)))
        .collect(),
    };
    for (kind, n) in cases {
        let (page, css, body) = case(&kind, n);
        let (elapsed, pages) = lay_out(&page, &css, &body);
        eprintln!(
            "{:<28} {:>10.2} ms  peak RSS {:>8.1} MiB  pages {pages}",
            format!("{kind} n={n}"),
            elapsed.as_secs_f64() * 1e3,
            peak_rss_kib().unwrap_or(0) as f64 / 1024.0,
        );
    }
}
