//! A block child of a paragraph root whose size along the root's inline axis
//! is `min-content` takes its minimum content size, not the room the root
//! leaves it (CSS Sizing 3 §5.1).

use crate::layout::ifc::test_support::block_fixture;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};

/// The border box size `(width, height)` of a block child laid out after
/// `lead`, in a root styled by `root_css`. The child's text is `AAAA BBBB`.
fn child_size(root_css: &str, lead: &str, child_css: &str) -> (f32, f32) {
    let mut child = 0;
    let fixture = block_fixture(&format!("line-height:10px;{root_css}"), |doc, root| {
        if !lead.is_empty() {
            doc.append_text(root, lead);
        }
        child = doc.append_element(
            Some(root),
            "div",
            taffy::Style::default(),
            Some(&format!("display:block;{child_css}")),
        );
        doc.append_text(child, "AAAA BBBB");
    });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    let (_, _, width, height) = absolute_rect(&doc, child);
    (width, height)
}

#[test]
fn a_block_in_a_paragraph_shrinks_to_its_min_content_width() {
    // Each of the two words is 40px wide, so the box wraps at the space.
    assert_eq!(
        child_size("width: 100px", "x", "width: min-content"),
        (40.0, 20.0)
    );
    // Without a keyword it fills the room, the line staying whole.
    assert_eq!(child_size("width: 100px", "x", ""), (100.0, 10.0));
}

#[test]
fn a_block_with_its_own_formatting_context_shrinks_to_its_min_content_width() {
    assert_eq!(
        child_size("width: 100px", "x", "overflow: hidden; width: min-content"),
        (40.0, 20.0)
    );
}

#[test]
fn a_min_content_width_stays_within_the_max_width() {
    assert_eq!(
        child_size("width: 100px", "x", "width: min-content; max-width: 30px"),
        (30.0, 20.0)
    );
}

#[test]
fn a_block_of_a_vertical_root_takes_its_min_content_inline_size() {
    let root = "writing-mode: vertical-lr; width: auto; height: 100px";
    // The inline axis is the physical height: the words stack in two lines.
    assert_eq!(
        child_size(root, "x", "inline-size: min-content"),
        (20.0, 40.0)
    );
    assert_eq!(child_size(root, "x", ""), (10.0, 100.0));
}

#[test]
fn a_min_content_inline_size_may_exceed_the_vertical_root() {
    // The unbreakable text is 90px long in the 50px root.
    let root = "writing-mode: vertical-lr; width: auto; height: 50px";
    assert_eq!(
        child_size(root, "x", "inline-size: min-content; white-space: pre"),
        (10.0, 90.0)
    );
}
