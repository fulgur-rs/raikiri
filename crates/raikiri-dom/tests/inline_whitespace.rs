//! Public-layout regression coverage for whitespace between inline boxes.

use raikiri_dom::{Document, layout_single_page};
use raikiri_style::{build_rule_tree, cascade};
use raikiri_traits::PageBox;
use taffy::Style;

#[test]
fn public_layout_preserves_collapsed_whitespace_between_inline_boxes() {
    let mut document = Document::new();
    let html = document.append_element(Some(0), "html", Style::default(), None::<&str>);
    let body = document.append_element(Some(html), "body", Style::default(), None::<&str>);
    let block =
        document.append_element(Some(body), "div", Style::default(), Some("display: block"));
    let left = document.append_element(
        Some(block),
        "span",
        Style::default(),
        Some("display: inline"),
    );
    document.append_text(left, "left");
    let whitespace = document.append_text(block, "\n  ");
    let right = document.append_element(
        Some(block),
        "span",
        Style::default(),
        Some("display: inline"),
    );
    document.append_text(right, "right");

    let rules = build_rule_tree(&document);
    let cascade = cascade(&document, &rules).expect("cascade should succeed");
    layout_single_page(&mut document, &cascade, PageBox::A4)
        .expect("single-page layout should succeed");

    // The collapsible white space between the spans is one space on the
    // line: the second span starts after the first one plus that space.
    let _ = whitespace;
    let pieces = document
        .get_node(block)
        .and_then(|node| node.ifc_inline_boxes())
        .expect("the block is a paragraph of the inline engine");
    let piece = |node: usize| {
        pieces
            .iter()
            .find(|piece| piece.node == node)
            .expect("a piece of the span")
            .border_box
    };
    let (left, right) = (piece(left), piece(right));
    let gap = right.x - (left.x + left.width);
    assert!(gap > 0.0, "preserved whitespace width was {gap}");
}
