//! A frameset document lays out with its frameset as the body element.

use raikiri_html::{
    LayoutOptions, LayoutStatus, RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

#[test]
fn a_frameset_document_lays_out_to_a_blank_page() {
    // HTML defines the body element as the `body` or `frameset` child of
    // the root element. Frames load no nested documents, so nothing is drawn.
    let document = parse_html_with_resources(
        br#"<!doctype html><frameset cols="*,100"><frame src="data:text/html,a"><frame></frameset>"#
            .as_slice(),
        &RenderResources::new(),
    )
    .unwrap();
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new(),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    assert_eq!(result.page_count(), 1);
    assert!(result.page(0).unwrap().text_runs().is_empty());
}
