//! Uninstrumented public pagination and projection timing probe.

use raikiri_dom::{Document, build_wpt_font_collection, layout_pages};
use raikiri_style::{CascadeResult, build_rule_tree, cascade};
use raikiri_traits::PageBox;
use std::time::Instant;

fn paragraph(count: usize) -> (Document, CascadeResult, PageBox) {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Some("display:block"));
    let body = document.append_element(Some(html), "body", Some("display:block"));
    let root = document.append_element(Some(body), "div", Some(
        "display:block;font-family:Ahem;font-size:10px;line-height:10px;width:10px;orphans:1;widows:1"
    ));
    document.append_text(root, "a ".repeat(count).as_str());
    document.mark_in_document_flags();
    document.set_font_collection(
        build_wpt_font_collection(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/data/text-autospace"
        )))
        .unwrap(),
    );
    let cascade = cascade(&document, &build_rule_tree(&document)).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 40.0;
    (document, cascade, page)
}

#[test]
#[ignore = "manual release timing; stress sizes require explicit environment"]
fn native_line_range_pagination_probe() {
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
    let mut signature = String::new();
    let mut pages_count = 0;
    if mode == "projection" {
        let (mut document, cascade, page) = paragraph(count);
        let slices = layout_pages(&mut document, &cascade, page).unwrap();
        for _ in 0..repeats {
            let started = Instant::now();
            document
                .project_pages(&cascade, page, &slices, &[])
                .unwrap();
            elapsed += started.elapsed();
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
            let (mut document, cascade, page) = paragraph(count);
            let started = Instant::now();
            let slices = layout_pages(&mut document, &cascade, page).unwrap();
            document
                .project_pages(&cascade, page, &slices, &[])
                .unwrap();
            elapsed += started.elapsed();
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
            "pages":pages_count,"signature":signature,
        })
    );
}
