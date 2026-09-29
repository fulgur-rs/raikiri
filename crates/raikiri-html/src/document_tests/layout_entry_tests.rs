use super::*;
use crate::{LayoutOptions, LayoutStatus, layout};
#[test]
fn layout_returns_one_completed_page() {
    let doc = parse_html(
        b"<p>Hi</p>".as_slice(),
        &ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        },
    )
    .unwrap();
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new(),
    )
    .unwrap() else {
        panic!("expected layout")
    };
    assert_eq!(result.page_count(), 1);
}
#[test]
fn layout_preabort_returns_no_partial_result() {
    let doc = parse_html(
        b"<p>Hi</p>".as_slice(),
        &ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        },
    )
    .unwrap();
    let controller = AbortController::new();
    controller.abort();
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::builder()
            .signal(Some(controller.signal.clone()))
            .build(),
        LayoutOptions::new(),
    )
    .unwrap();
    assert!(matches!(status, LayoutStatus::Aborted));
}
