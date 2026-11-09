use crate::Document;
use crate::layout::test_support::{page_box_800x600, with_ahem};
use raikiri_style::{build_rule_tree, cascade};
use raikiri_traits::QuirksMode;

/// Lay out a quirks mode document of `html`, with `between` (if any) between
/// it and `body`, holding a 10px tall block. Returns the border-box heights
/// of the root element and the body.
fn heights(html_css: &str, between: Option<&str>, body_css: &str) -> (f32, f32) {
    let mut doc = Document::new();
    doc.set_quirks_mode(QuirksMode::Quirks);
    let html = doc.append_element(Some(0), "html", Some(html_css));
    let parent = match between {
        Some(css) => doc.append_element(Some(html), "div", Some(css)),
        None => html,
    };
    let body = doc.append_element(Some(parent), "body", Some(body_css));
    doc.append_element(Some(body), "div", Some("display:block;height:10px"));
    doc.mark_in_document_flags();
    let cascade = cascade(&doc, &build_rule_tree(&doc)).expect("cascade");
    crate::layout::layout_pages(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    (
        doc.nodes[html].layout.size.height,
        doc.nodes[body].layout.size.height,
    )
}

#[test]
fn root_and_body_fill_the_page_area() {
    assert_eq!(
        heights("display:block", None, "display:block;margin:0"),
        (600.0, 600.0)
    );
}

#[test]
fn body_inside_another_element_keeps_its_height() {
    // Only a body that is a child of the root element fills it.
    assert_eq!(
        heights(
            "display:block",
            Some("display:block"),
            "display:block;margin:0"
        ),
        (600.0, 10.0)
    );
}

#[test]
fn sizing_keyword_minimum_on_the_root_leaves_both_unfilled() {
    // The root element keeps its usual height, and states no height for
    // the body to fill.
    assert_eq!(
        heights(
            "display:block;min-height:min-content",
            None,
            "display:block;margin:0"
        ),
        (10.0, 10.0)
    );
}
