use super::*;
use crate::layout::test_support::ahem_paragraph;
use std::time::Instant;

fn paragraph(count: usize) -> (Document, CascadeResult, usize, PageBox) {
    let (mut document, cascade, root) =
        ahem_paragraph(&"a ".repeat(count), "width:10px;orphans:1;widows:1");
    with_ahem(&mut document);
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 40.0;
    (document, cascade, root, page)
}

#[test]
fn public_pagination_projects_a_long_paragraph_without_rescanning_every_line() {
    for count in [64, 128, 256] {
        let (mut document, cascade, root, page) = paragraph(count);
        LINE_CENTER_EVALUATIONS.with(|value| value.set(0));
        let slices = layout_pages(&mut document, &cascade, page).unwrap();
        document
            .project_pages(&cascade, page, &slices, &[])
            .unwrap();
        let visits = LINE_CENTER_EVALUATIONS.with(std::cell::Cell::get);
        assert_eq!(slices.len(), count / 4);
        assert_eq!(
            document.nodes[root]
                .ifc
                .as_ref()
                .unwrap()
                .lines
                .as_ref()
                .unwrap()
                .lines
                .len(),
            count
        );
        assert!(
            visits <= count * 32,
            "{count} lines evaluated {visits} centers"
        );
        let text_lines: usize = slices
            .iter()
            .map(|slice| document.page_text_runs(&cascade, slice.page_index).len())
            .sum();
        assert_eq!(text_lines, count);
    }
}

#[test]
#[ignore = "manual release timing; stress sizes require explicit environment"]
fn line_range_pagination_probe() {
    let count: usize = std::env::var("LINE_RANGE_LINES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(16);
    let repeats: usize = std::env::var("LINE_RANGE_REPEATS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let mode = std::env::var("LINE_RANGE_MODE").unwrap_or_else(|_| "projection".to_owned());
    assert!(count > 0 && repeats > 0);
    let mut elapsed = std::time::Duration::ZERO;
    let mut evaluations = 0;
    let mut signature = String::new();
    let mut pages_count = 0;
    if mode == "projection" {
        let (mut document, cascade, _, page) = paragraph(count);
        let slices = layout_pages(&mut document, &cascade, page).unwrap();
        for _ in 0..repeats {
            LINE_CENTER_EVALUATIONS.with(|value| value.set(0));
            let started = Instant::now();
            document
                .project_pages(&cascade, page, &slices, &[])
                .unwrap();
            elapsed += started.elapsed();
            evaluations += LINE_CENTER_EVALUATIONS.with(std::cell::Cell::get);
            pages_count = slices.len();
            let current = format!(
                "{:?}",
                slices
                    .iter()
                    .map(|slice| {
                        (
                            slice,
                            document
                                .page_fragments(slice.page_index)
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Vec<_>>()
            );
            if !signature.is_empty() {
                assert_eq!(signature, current);
            }
            signature = current;
        }
    } else {
        assert_eq!(mode, "full");
        for _ in 0..repeats {
            let (mut document, cascade, _, page) = paragraph(count);
            LINE_CENTER_EVALUATIONS.with(|value| value.set(0));
            let started = Instant::now();
            let slices = layout_pages(&mut document, &cascade, page).unwrap();
            document
                .project_pages(&cascade, page, &slices, &[])
                .unwrap();
            elapsed += started.elapsed();
            evaluations += LINE_CENTER_EVALUATIONS.with(std::cell::Cell::get);
            pages_count = slices.len();
            let current = format!(
                "{:?}",
                slices
                    .iter()
                    .map(|slice| {
                        (
                            slice,
                            document
                                .page_fragments(slice.page_index)
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Vec<_>>()
            );
            if !signature.is_empty() {
                assert_eq!(signature, current);
            }
            signature = current;
        }
    }
    println!(
        "LINE_RANGE_RESULT {}",
        serde_json::json!({
            "lines":count,"repeats":repeats,"mode":mode,"ns":elapsed.as_nanos(),
            "evaluations":evaluations,"pages":pages_count,"signature":signature,
        })
    );
}
