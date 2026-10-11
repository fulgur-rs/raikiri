//! Layout-only scaling probe for dense collapsed tables.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};
use std::time::Instant;

fn measure(mode: &str, side: usize) {
    assert!(matches!(mode, "fixed" | "auto"));
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let row = format!("<tr>{}</tr>", "<td>A</td>".repeat(side));
    let html = format!(
        "<!doctype html><style>@page{{size:{}px 100000px;margin:0}}body{{margin:0;font:10px/10px Ahem}}table{{table-layout:{mode};width:{}px;border-collapse:collapse}}td{{padding:0;border:1px solid}}</style><table>{}</table>",
        side * 16 + 40,
        side * 16,
        row.repeat(side),
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
    let elapsed = started.elapsed();
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let rss_kib: u64 = status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(document.page_count(), 1);
    eprintln!(
        "{mode} {side}x{side}: {:.2} ms, peak RSS {:.1} MiB, pages {}",
        elapsed.as_secs_f64() * 1e3,
        rss_kib as f64 / 1024.0,
        document.page_count(),
    );
}

#[test]
#[ignore = "performance probe"]
fn collapsed_table_scaling_probe() {
    if let Ok(case) = std::env::var("COLLAPSED_TABLE_CASE") {
        let (mode, side) = case.split_once(':').expect("fixed:n or auto:n");
        measure(mode, side.parse().unwrap());
    } else {
        for mode in ["fixed", "auto"] {
            measure(mode, 8);
        }
    }
}
