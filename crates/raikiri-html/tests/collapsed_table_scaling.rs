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
    assert_eq!(document.page_count(), 1);
    let peak_rss = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix("VmHWM:"))?
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()
        })
        .map(|rss_kib| format!("{:.1} MiB", rss_kib as f64 / 1024.0))
        .unwrap_or_else(|| "unavailable".to_owned());
    eprintln!(
        "{mode} {side}x{side}: {:.2} ms, peak RSS {peak_rss}, pages {}",
        elapsed.as_secs_f64() * 1e3,
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
