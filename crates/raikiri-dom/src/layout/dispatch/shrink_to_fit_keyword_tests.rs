//! A float or an inline-block whose `width` is `min-content` or `max-content`
//! takes the size the keyword measures, not the shrink-to-fit width of an
//! `auto` width (CSS Sizing 3 §3.1.1): the longest word of its text for
//! `min-content`, and its text on one line for `max-content`, however narrow
//! the room is. The text of the fixtures is `AAAA BBBB` in Ahem at 10px: 40px
//! wide at its longest word, 90px wide on one line. Every position is the one
//! Chromium gives.

use crate::layout::ifc::test_support::block_fixture;
use crate::layout::layout_single_page;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};

/// A border box `(x, y, width, height)`.
type BorderBox = (f32, f32, f32, f32);

/// What the box under test holds.
#[derive(Clone, Copy)]
enum Body {
    /// The text `AAAA BBBB`, so a block box is a paragraph root.
    Text,
    /// The text `AA ` and an inline-block of the given width holding the text
    /// `BBBB CC`.
    TextAndInlineBlock(&'static str),
}

/// The border box of the box `item_css` styles, in a root styled by
/// `root_css` that holds the text `lead` before it.
fn item_rect(root_css: &str, lead: &str, item_css: &str, body: Body) -> BorderBox {
    let mut item = 0;
    let fixture = block_fixture(&format!("line-height:10px;{root_css}"), |doc, root| {
        if !lead.is_empty() {
            doc.append_text(root, lead);
        }
        item = doc.append_element(
            Some(root),
            "div",
            Some(&format!("display:block;{item_css}")),
        );
        match body {
            Body::Text => {
                doc.append_text(item, "AAAA BBBB");
            }
            Body::TextAndInlineBlock(width) => {
                doc.append_text(item, "AA ");
                let inner = doc.append_element(
                    Some(item),
                    "span",
                    Some(&format!("display:inline-block;width:{width}")),
                );
                doc.append_text(inner, "BBBB CC");
            }
        }
    });
    let mut doc = fixture.doc;
    layout_single_page(with_ahem(&mut doc), &fixture.cascade, page_box_800x600()).expect("layout");
    absolute_rect(&doc, item)
}

/// The border box of a float styled by `item_css` in a block container of
/// the given width that holds nothing else.
fn float_rect(width: f32, item_css: &str) -> BorderBox {
    item_rect(
        &format!("width:{width}px"),
        "",
        &format!("float:left;{item_css}"),
        Body::Text,
    )
}

/// The border box of a float styled by `item_css` after the text `x` of a
/// paragraph of the given width.
fn float_in_paragraph_rect(width: f32, item_css: &str) -> BorderBox {
    item_rect(
        &format!("width:{width}px"),
        "x",
        &format!("float:left;{item_css}"),
        Body::Text,
    )
}

/// The border box of a box with the `display` given, after the text `x` of a
/// paragraph of the given width.
fn atomic_rect(width: f32, display: &str, item_css: &str) -> BorderBox {
    item_rect(
        &format!("width:{width}px"),
        "x",
        &format!("display:{display};{item_css}"),
        Body::Text,
    )
}

#[test]
fn a_float_takes_the_size_of_its_keyword_width_whatever_the_room() {
    for room in [100.0, 60.0] {
        assert_eq!(
            float_rect(room, "width:min-content"),
            (0.0, 0.0, 40.0, 20.0),
            "min-content in {room}px"
        );
        assert_eq!(
            float_rect(room, "width:max-content"),
            (0.0, 0.0, 90.0, 10.0),
            "max-content in {room}px"
        );
    }
}

#[test]
fn a_right_float_with_a_keyword_width_keeps_to_the_right_edge() {
    assert_eq!(
        float_rect(100.0, "float:right;width:min-content"),
        (60.0, 0.0, 40.0, 20.0)
    );
    assert_eq!(
        float_rect(100.0, "float:right;width:max-content"),
        (10.0, 0.0, 90.0, 10.0)
    );
    // The float overflows the container on the left, as the box does that
    // states a width of 90px.
    assert_eq!(
        float_rect(60.0, "float:right;width:max-content"),
        (-30.0, 0.0, 90.0, 10.0)
    );
}

#[test]
fn a_keyword_width_float_adds_its_padding_border_and_margin() {
    let edges = "box-sizing:content-box;padding:0 5px;border:2px solid";
    assert_eq!(
        float_rect(100.0, &format!("{edges};width:min-content")),
        (0.0, 0.0, 54.0, 24.0)
    );
    assert_eq!(
        float_rect(100.0, &format!("{edges};width:max-content")),
        (0.0, 0.0, 104.0, 14.0)
    );
    assert_eq!(
        float_rect(100.0, "margin:0 5px;width:min-content"),
        (5.0, 0.0, 40.0, 20.0)
    );
    assert_eq!(
        float_rect(100.0, "margin:0 5px;width:max-content"),
        (5.0, 0.0, 90.0, 10.0)
    );
}

#[test]
fn a_keyword_width_float_stays_within_its_min_and_max_width() {
    for keyword in ["min-content", "max-content"] {
        assert_eq!(
            float_rect(100.0, &format!("max-width:30px;width:{keyword}")),
            (0.0, 0.0, 30.0, 20.0),
            "{keyword} under max-width"
        );
        assert_eq!(
            float_rect(100.0, &format!("min-width:120px;width:{keyword}")),
            (0.0, 0.0, 120.0, 10.0),
            "{keyword} over min-width"
        );
    }
}

#[test]
fn a_keyword_width_float_follows_the_direction_of_its_container() {
    for (keyword, size) in [("min-content", (40.0, 20.0)), ("max-content", (90.0, 10.0))] {
        assert_eq!(
            item_rect(
                "width:100px;direction:rtl",
                "",
                &format!("float:left;width:{keyword}"),
                Body::Text
            ),
            (0.0, 0.0, size.0, size.1),
            "{keyword}"
        );
    }
}

#[test]
fn a_float_in_a_paragraph_takes_the_size_of_its_keyword_width() {
    assert_eq!(
        float_in_paragraph_rect(100.0, "width:min-content"),
        (0.0, 0.0, 40.0, 20.0)
    );
    assert_eq!(
        float_in_paragraph_rect(100.0, "width:max-content"),
        (0.0, 0.0, 90.0, 10.0)
    );
    assert_eq!(
        float_in_paragraph_rect(60.0, "width:min-content"),
        (0.0, 0.0, 40.0, 20.0)
    );
}

#[test]
fn an_inline_block_takes_the_size_of_its_keyword_width_whatever_the_room() {
    // After the `x` of the line, which is 10px wide.
    assert_eq!(
        atomic_rect(100.0, "inline-block", "width:min-content"),
        (10.0, 0.0, 40.0, 20.0)
    );
    assert_eq!(
        atomic_rect(100.0, "inline-block", "width:max-content"),
        (10.0, 0.0, 90.0, 10.0)
    );
    assert_eq!(
        atomic_rect(60.0, "inline-block", "width:min-content"),
        (10.0, 0.0, 40.0, 20.0)
    );
    // The box is wider than the room beside the `x`, so the line breaks
    // before it, and wider than the room of the paragraph, which it overflows.
    assert_eq!(
        atomic_rect(60.0, "inline-block", "width:max-content"),
        (0.0, 10.0, 90.0, 10.0)
    );
}

#[test]
fn an_inline_block_with_a_keyword_width_adds_its_edges_and_stays_within_its_limits() {
    let edges = "box-sizing:content-box;padding:0 5px;border:2px solid";
    assert_eq!(
        atomic_rect(100.0, "inline-block", &format!("{edges};width:min-content")),
        (10.0, 0.0, 54.0, 24.0)
    );
    assert_eq!(
        atomic_rect(100.0, "inline-block", &format!("{edges};width:max-content")),
        (0.0, 10.0, 104.0, 14.0)
    );
    for keyword in ["min-content", "max-content"] {
        assert_eq!(
            atomic_rect(
                100.0,
                "inline-block",
                &format!("max-width:30px;width:{keyword}")
            ),
            (10.0, 0.0, 30.0, 20.0),
            "{keyword} under max-width"
        );
    }
}

#[test]
fn an_inline_flex_container_takes_the_size_of_its_keyword_width() {
    assert_eq!(
        atomic_rect(100.0, "inline-flex", "width:min-content"),
        (10.0, 0.0, 40.0, 20.0)
    );
    assert_eq!(
        atomic_rect(60.0, "inline-flex", "width:min-content"),
        (10.0, 0.0, 40.0, 20.0)
    );
    assert_eq!(
        atomic_rect(100.0, "inline-flex", "width:max-content"),
        (10.0, 0.0, 90.0, 10.0)
    );
    assert_eq!(
        atomic_rect(60.0, "inline-flex", "width:max-content"),
        (0.0, 10.0, 90.0, 10.0)
    );
}

#[test]
fn an_inline_grid_container_takes_the_size_of_its_keyword_width() {
    assert_eq!(
        atomic_rect(100.0, "inline-grid", "width:min-content").2,
        40.0
    );
    assert_eq!(
        atomic_rect(60.0, "inline-grid", "width:max-content").2,
        90.0
    );
}

#[test]
fn a_keyword_width_nests_in_another_keyword_width() {
    // The inline-block of `BBBB CC` is 70px wide on one line; the float
    // around `AA ` and that box is 100px wide on one line, and one word
    // narrow on the narrowest.
    let float_css = "float:left;width:max-content";
    assert_eq!(
        item_rect(
            "width:100px",
            "",
            float_css,
            Body::TextAndInlineBlock("max-content")
        ),
        (0.0, 0.0, 100.0, 10.0)
    );
    assert_eq!(
        item_rect(
            "width:100px",
            "",
            "float:left;width:min-content",
            Body::TextAndInlineBlock("min-content")
        ),
        (0.0, 0.0, 40.0, 30.0)
    );
}
