//! A box that holds no line takes the forced break that the box before it
//! ends in, wherever the two sit in the flow (CSS Fragmentation 3, section
//! 3.1: a forced break falls between the boxes it is set between).
//!
//! The pages are 480px by 288px with 48px margins, so the page area is 384px
//! wide and 192px high.

use raikiri_html::{
    LayoutOptions, LayoutStatus, RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The margin of the page, which fragments are placed inside.
const MARGIN: f32 = 48.0;

/// The page, from 0, and the top and height on that page's area of the first
/// fragment of the element with the given `id`, and the number of pages.
fn place(body: &str, id: &str) -> (usize, (usize, f32, f32)) {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 480px 288px; margin: 48px }} \
         body {{ margin: 0 }}</style>{body}"
    );
    let resources = RenderResources::new();
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let mut pages = 0;
    let mut found = None;
    for (index, page) in result.pages().enumerate() {
        pages = index + 1;
        let dom = page.dom();
        for fragment in page.fragments() {
            if found.is_none() && dom.attr(fragment.node(), "id") == Some(id) {
                let rect = fragment.rect();
                found = Some((index, rect.y - MARGIN, rect.height));
            }
        }
    }
    (
        pages,
        found.unwrap_or_else(|| panic!("no fragment for #{id}")),
    )
}

/// Compares a placement with the expected one.
fn assert_at(got: (usize, f32, f32), expected: (usize, f32, f32), label: &str) {
    assert!(
        got.0 == expected.0
            && (got.1 - expected.1).abs() <= 0.04
            && (got.2 - expected.2).abs() <= 0.04,
        "{label}: placed {got:?}, expected {expected:?}"
    );
}

/// An empty box in a block, after a box in it that ends in a forced break,
/// starts the next page, as it does when the two sit under the body.
#[test]
fn an_empty_box_in_a_block_takes_the_break_of_the_box_before_it() {
    let nested = "<div style='background: yellow'>\
                  <div style='break-after: page'>first</div>\
                  <div id=b style='height: 40px; background: blue'></div></div>";
    let (pages, b) = place(nested, "b");
    assert_eq!(pages, 2);
    assert_at(b, (1, 0.0, 40.0), "nested");
    let flat = "<div style='break-after: page'>first</div>\
                <div id=b style='height: 40px; background: blue'></div>";
    let (pages, b) = place(flat, "b");
    assert_eq!(pages, 2);
    assert_at(b, (1, 0.0, 40.0), "flat");
}

/// The page ends before the empty box, not before the paragraph that follows
/// the block: both stand on the second page, one under the other.
#[test]
fn the_break_falls_before_the_empty_box_and_not_before_what_follows_it() {
    let body = "<div style='background: yellow'>\
                <div style='break-after: page'>first</div>\
                <div id=b style='height: 40px; background: blue'></div></div>\
                <div id=p>after</div>";
    let (pages, b) = place(body, "b");
    assert_eq!(pages, 2);
    assert_at(b, (1, 0.0, 40.0), "empty box");
    let (_, p) = place(body, "p");
    assert_at((p.0, p.1, 0.0), (1, 40.0, 0.0), "paragraph");
}

/// The break is the one at the end of the last box inside the box before the
/// empty one, however deep.
#[test]
fn a_break_at_the_end_of_a_wrapper_applies_to_the_next_empty_box() {
    let body = "<div><div><div style='break-after: page'>first</div></div>\
                <div id=b style='height: 40px; background: blue'></div></div>";
    let (pages, b) = place(body, "b");
    assert_eq!(pages, 2);
    assert_at(b, (1, 0.0, 40.0), "empty box");
}

/// Two empty boxes: the break set after the first one is taken by the second.
#[test]
fn an_empty_box_takes_the_break_of_an_empty_box_before_it() {
    let body = "<div><div style='height: 20px; break-after: page'></div>\
                <div id=b style='height: 40px; background: blue'></div></div>";
    let (pages, b) = place(body, "b");
    assert_eq!(pages, 2);
    assert_at(b, (1, 0.0, 40.0), "empty box");
}

/// Without a forced break the empty box stays after the box before it.
#[test]
fn an_empty_box_stays_after_the_box_before_it_without_a_forced_break() {
    let body = "<div><div>first</div>\
                <div id=b style='height: 40px; background: blue'></div></div>";
    let (pages, b) = place(body, "b");
    assert_eq!(pages, 1);
    assert_at(b, (0, 19.0, 40.0), "empty box");
}

/// The block-start margin of the box after a forced break is kept, in a
/// block as it is under the body.
#[test]
fn the_margin_after_a_forced_break_is_kept_in_a_block() {
    let nested = "<div><div style='break-after: page'>first</div>\
                  <div id=b style='height: 40px; margin-top: 10px'></div></div>";
    let flat = "<div style='break-after: page'>first</div>\
                <div id=b style='height: 40px; margin-top: 10px'></div>";
    let (_, nested) = place(nested, "b");
    let (_, flat) = place(flat, "b");
    assert_at(flat, (1, 10.0, 40.0), "flat");
    assert_at(nested, (1, 10.0, 40.0), "nested");
}

/// The empty box lands where a box that holds a line lands, whatever box
/// holds the two: a column container, a clipping box, or a box of a fixed
/// height.
#[test]
fn an_empty_box_lands_where_a_box_with_a_line_lands() {
    let a = "<div style='break-after: page'>first</div>";
    for (name, open, close) in [
        (
            "columns",
            "<div style='columns: 2; column-gap: 0'><div>",
            "</div></div>",
        ),
        (
            "clip",
            "<div style='overflow: hidden; height: 100px'>",
            "</div>",
        ),
        ("fixed height", "<div style='height: 150px'>", "</div>"),
    ] {
        let empty = format!(
            "{open}{a}<div id=b style='height: 40px; background: blue'></div>{close}\
             <div id=p>after</div>"
        );
        let line = empty.replace("blue'></div>", "blue'>x</div>");
        let (pages, got) = place(&empty, "b");
        let (line_pages, expected) = place(&line, "b");
        assert_eq!(pages, line_pages, "{name}: pages");
        assert_eq!(
            expected.0, 1,
            "{name}: the box with a line moves to the next page"
        );
        assert_at(got, expected, name);
        let (_, got) = place(&empty, "p");
        let (_, expected) = place(&line, "p");
        assert_at((got.0, got.1, 0.0), (expected.0, expected.1, 0.0), name);
    }
}

/// A float or an absolutely positioned box between the two boxes is no box
/// to break after, so the break stays between the two.
#[test]
fn a_float_or_an_absolute_box_between_does_not_stop_the_break() {
    for between in [
        "<div style='float: left; width: 10px; height: 10px'></div>",
        "<div style='position: absolute; width: 10px; height: 10px'></div>",
    ] {
        let body = format!(
            "<div><div style='break-after: page'>first</div>{between}\
             <div id=b style='height: 40px; background: blue'></div></div>"
        );
        let (pages, b) = place(&body, "b");
        assert_eq!(pages, 2);
        assert_at(b, (1, 0.0, 40.0), "empty box");
    }
}

/// A box that takes no block space after the break adds no page: it is where
/// the same box is under the body.
#[test]
fn a_box_with_no_height_after_a_forced_break_adds_no_page() {
    let nested = "<div><div style='break-after: page'>first</div>\
                  <div style='clear: both'></div></div>";
    let flat = "<div style='break-after: page'>first</div><div style='clear: both'></div>";
    let (nested_pages, _) = place(&format!("{nested}<div id=b></div>"), "b");
    let (flat_pages, _) = place(&format!("{flat}<div id=b></div>"), "b");
    assert_eq!(nested_pages, flat_pages);
}
