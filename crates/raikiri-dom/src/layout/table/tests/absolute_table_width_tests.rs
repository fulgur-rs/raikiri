//! The width of an absolutely positioned table whose `width` is `auto`.
//!
//! Like any other box that is positioned out of flow, such a table
//! shrink-wraps its content (CSS 2.1 §10.3.7 and §17.5.2.2): it is as wide as
//! its columns need, capped by the room its containing block leaves after
//! the `left` and `right` offsets. It never stretches to the containing block,
//! not even when both offsets are set. Every rectangle here is what Chromium
//! reports for the same markup (Ahem 10px, no body margin).

use crate::document::Document;
use crate::layout::test_support::{absolute_rect, page_box_800x600, with_ahem};
use raikiri_style::{build_rule_tree, cascade};

/// Content of the single table cell.
#[derive(Clone, Copy)]
enum Content {
    /// A 10x10 block.
    Block,
    /// A block as wide as the given width, 10px tall.
    Wide(f32),
    /// Forty characters of Ahem text in words of three: 390px on one line,
    /// 30px for the longest word.
    Words,
}

/// The table and its cell once the page below is laid out.
struct Laid {
    doc: Document,
    table: usize,
    cell: usize,
}

/// `body > div(position:relative; wrapper_css) > table(position:absolute;
/// table_css) > row > cell > content`, laid out on an 800x600 page.
fn lay_out(wrapper_css: &str, table_css: &str, content: Content) -> Laid {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;font-family:Ahem;font-size:10px;line-height:10px"),
    );
    let wrapper = doc.append_element(
        Some(body),
        "div",
        Some(&format!("display:block;position:relative;{wrapper_css}")),
    );
    let table = doc.append_element(
        Some(wrapper),
        "div",
        Some(&format!("display:table;position:absolute;{table_css}")),
    );
    let row = doc.append_element(Some(table), "div", Some("display:table-row"));
    let cell = doc.append_element(Some(row), "div", Some("display:table-cell"));
    match content {
        Content::Block => {
            doc.append_element(
                Some(cell),
                "div",
                Some("display:block;width:10px;height:10px"),
            );
        }
        Content::Wide(width) => {
            doc.append_element(
                Some(cell),
                "div",
                Some(&format!("display:block;width:{width}px;height:10px")),
            );
        }
        Content::Words => {
            doc.append_text(cell, "xxx xxx xxx xxx xxx xxx xxx xxx xxx xxx");
        }
    }
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    with_ahem(&mut doc);
    crate::layout::layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    Laid { doc, table, cell }
}

/// The table's border box for `wrapper_css`, `table_css` and `content`.
fn table_rect(wrapper_css: &str, table_css: &str, content: Content) -> (f32, f32, f32, f32) {
    let laid = lay_out(wrapper_css, table_css, content);
    absolute_rect(&laid.doc, laid.table)
}

#[test]
fn an_auto_width_table_is_as_wide_as_its_content() {
    let laid = lay_out("height:100px", "top:0", Content::Block);
    // The table and its cell take the 10px the content needs rather than the
    // 800px of the containing block.
    for (part, node) in [("table", laid.table), ("cell", laid.cell)] {
        assert_eq!(
            absolute_rect(&laid.doc, node),
            (0.0, 0.0, 10.0, 10.0),
            "{part}"
        );
    }
}

#[test]
fn offsets_place_a_shrink_wrapped_table_without_stretching_it() {
    let wide = "width:300px;height:100px";
    let cases = [
        ("top:0;left:20px", (20.0, 0.0, 10.0, 10.0)),
        ("top:0;right:20px", (270.0, 0.0, 10.0, 10.0)),
        // Both offsets leave the width to the content, unlike a block.
        ("top:0;left:10px;right:10px", (10.0, 0.0, 10.0, 10.0)),
        ("top:0;bottom:0;left:0;right:0", (0.0, 0.0, 10.0, 10.0)),
        // A vertical table shrink-wraps along its width too.
        (
            "top:0;left:0;right:0;writing-mode:vertical-rl",
            (0.0, 0.0, 10.0, 10.0),
        ),
        (
            "top:0;right:20px;writing-mode:vertical-rl",
            (270.0, 0.0, 10.0, 10.0),
        ),
        ("bottom:0", (0.0, 90.0, 10.0, 10.0)),
        ("top:0;margin:5px 7px", (7.0, 5.0, 10.0, 10.0)),
    ];
    for (table_css, expected) in cases {
        assert_eq!(
            table_rect(wide, table_css, Content::Block),
            expected,
            "{table_css}"
        );
    }
}

#[test]
fn the_box_of_a_shrink_wrapped_table_includes_its_padding_borders_and_spacing() {
    let cases = [
        ("top:0;border:3px solid;padding:4px", 24.0),
        ("top:0;border-collapse:collapse;border:3px solid", 16.0),
        ("top:0;border-spacing:5px", 20.0),
    ];
    for (table_css, size) in cases {
        assert_eq!(
            table_rect("height:100px", table_css, Content::Block),
            (0.0, 0.0, size, size),
            "{table_css}"
        );
    }
}

#[test]
fn an_authored_width_still_sizes_an_absolutely_positioned_table() {
    let wide = "width:300px;height:100px";
    let cases = [
        ("top:0;width:50px", (0.0, 0.0, 50.0, 10.0)),
        ("top:0;width:50%", (0.0, 0.0, 150.0, 10.0)),
        ("top:0;min-width:100px", (0.0, 0.0, 100.0, 10.0)),
        ("top:0;table-layout:fixed", (0.0, 0.0, 10.0, 10.0)),
    ];
    for (table_css, expected) in cases {
        assert_eq!(
            table_rect(wide, table_css, Content::Block),
            expected,
            "{table_css}"
        );
    }
}

#[test]
fn the_room_left_by_the_offsets_caps_a_table_whose_text_can_wrap() {
    let cb = "width:200px;height:100px";
    let cases = [
        // 200px minus `left` = 150px; the words wrap into three lines.
        ("top:0;left:50px", (50.0, 0.0, 150.0, 30.0)),
        ("top:0;right:50px", (0.0, 0.0, 150.0, 30.0)),
        ("top:0;left:10px;right:10px", (10.0, 0.0, 180.0, 30.0)),
        // The padding comes out of the room; the cell keeps the rest.
        ("top:0;padding:6px", (0.0, 0.0, 200.0, 42.0)),
    ];
    for (table_css, expected) in cases {
        assert_eq!(
            table_rect(cb, table_css, Content::Words),
            expected,
            "{table_css}"
        );
    }
}

#[test]
fn content_that_cannot_wrap_makes_the_table_wider_than_its_containing_block() {
    assert_eq!(
        table_rect(
            "width:200px;height:100px",
            "top:0;left:0",
            Content::Wide(1000.0)
        ),
        (0.0, 0.0, 1000.0, 10.0)
    );
}

#[test]
fn an_absolutely_positioned_table_in_a_flex_container_shrink_wraps_too() {
    // The flex container does not size a child that is out of flow.
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    let flex = doc.append_element(
        Some(body),
        "div",
        Some("display:flex;position:relative;height:100px"),
    );
    let table = doc.append_element(
        Some(flex),
        "div",
        Some("display:table;position:absolute;top:0"),
    );
    let row = doc.append_element(Some(table), "div", Some("display:table-row"));
    let cell = doc.append_element(Some(row), "div", Some("display:table-cell"));
    doc.append_element(
        Some(cell),
        "div",
        Some("display:block;width:10px;height:10px"),
    );
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).expect("cascade");
    crate::layout::layout_single_page(&mut doc, &cascade, page_box_800x600()).expect("layout");
    assert_eq!(absolute_rect(&doc, table), (0.0, 0.0, 10.0, 10.0));
}
