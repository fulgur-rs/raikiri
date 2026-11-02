//! The baseline a box with layout containment gives to the flex container or
//! the line that aligns it by a baseline: none of its own, so the container
//! uses the bottom of the box (its border edge in a flex row, its margin edge
//! in a line), not the baseline of its first line (CSS Contain 2 §3.2).
//!
//! Each case lays out a container `#o` of three items `#a`, `#c` and `#z`
//! that align by their baselines, `#c` being the item under test between a
//! 10px line and a 20px one. The expected border boxes are the ones Chromium
//! computes for the same markup in an 800px by 600px viewport, with the body
//! margin of 8px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A border box `(x, y, width, height)`.
type Rect = (f32, f32, f32, f32);

/// The border boxes of the elements of `body` with an `id` on the first page.
fn border_boxes(body: &str) -> Vec<(String, Rect)> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
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
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    page.fragments()
        .filter_map(|fragment| {
            let id = dom.attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((id.to_owned(), (rect.x, rect.y, rect.width, rect.height)))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.02;

/// Check the border boxes of `#o`, `#a`, `#c` and `#z`, in that order.
fn check(body: &str, expected: [Rect; 4]) {
    let actual = border_boxes(body);
    for (id, (x, y, width, height)) in ["o", "a", "c", "z"].into_iter().zip(expected) {
        let found = actual
            .iter()
            .find(|(name, _)| name == id)
            .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"))
            .1;
        let close = |a: f32, b: f32| (a - b).abs() <= TOLERANCE;
        assert!(
            close(found.0, x)
                && close(found.1, y)
                && close(found.2, width)
                && close(found.3, height),
            "{body}: #{id} is {found:?}, expected {:?}",
            (x, y, width, height)
        );
    }
}

/// A flex row that aligns its items by `baseline`, holding the item `#c`
/// styled by `item` with `content` in it.
fn flex_row(baseline: &str, item: &str, content: &str) -> String {
    format!(
        "<div id=o style=\"display:flex;align-items:{baseline};width:300px\">\
         <div id=a style=\"line-height:10px\">X</div>\
         <div id=c style=\"{item}\">{content}</div>\
         <div id=z style=\"line-height:20px\">X</div></div>"
    )
}

/// A line of inline-blocks, the item `#c` among them.
fn line(item: &str, content: &str) -> String {
    let atomic = "display:inline-block";
    format!(
        "<div id=o style=\"width:300px;line-height:20px\">\
         <div id=a style=\"{atomic};line-height:10px\">X</div>\
         <div id=c style=\"{atomic};{item}\">{content}</div>\
         <div id=z style=\"{atomic};line-height:20px\">X</div></div>"
    )
}

/// The item `#c` in a flex row and in a line, each with its expected boxes.
fn check_both(item: &str, content: &str, in_row: [Rect; 4], in_line: [Rect; 4]) {
    check(&flex_row("baseline", item, content), in_row);
    check(&line(item, content), in_line);
}

/// An item whose height of 50px does not come from its 30px line.
const TALL: &str = "height:50px;line-height:30px";

/// Blocks with the paragraph `Y` in them, one after the other.
fn blocks(styles: &[&str]) -> String {
    styles
        .iter()
        .map(|style| format!("<div style=\"{style}\">Y</div>"))
        .collect()
}

const CONTAINED: &str = "contain:layout;height:50px;line-height:30px";
const PLAIN: &str = "line-height:30px";

#[test]
fn an_item_without_containment_gives_the_baseline_of_its_first_line() {
    // The 30px line has its baseline 23px down in the 50px item.
    check_both(
        TALL,
        "X",
        [
            (8.0, 8.0, 300.0, 50.0),
            (8.0, 18.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 50.0),
            (28.0, 13.0, 10.0, 20.0),
        ],
        [
            (8.0, 8.0, 300.0, 50.0),
            (8.0, 18.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 50.0),
            (28.0, 13.0, 10.0, 20.0),
        ],
    );
}

#[test]
fn contain_layout_gives_the_bottom_of_the_item_as_its_baseline() {
    // The bottom of the item is 50px down: the paragraphs sit 8px above it, so
    // the container grows 7px for the descent of the 20px paragraph.
    check_both(
        &format!("contain:layout;{TALL}"),
        "X",
        [
            (8.0, 8.0, 300.0, 57.0),
            (8.0, 50.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 50.0),
            (28.0, 45.0, 10.0, 20.0),
        ],
        [
            (8.0, 8.0, 300.0, 57.0),
            (8.0, 50.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 50.0),
            (28.0, 45.0, 10.0, 20.0),
        ],
    );
}

#[test]
fn contain_content_gives_the_bottom_of_the_item_as_its_baseline() {
    check_both(
        &format!("contain:content;{TALL}"),
        "X",
        [
            (8.0, 8.0, 300.0, 57.0),
            (8.0, 50.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 50.0),
            (28.0, 45.0, 10.0, 20.0),
        ],
        [
            (8.0, 8.0, 300.0, 57.0),
            (8.0, 50.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 50.0),
            (28.0, 45.0, 10.0, 20.0),
        ],
    );
}

#[test]
fn contain_strict_gives_the_bottom_of_the_item_as_its_baseline() {
    check_both(
        &format!("contain:strict;width:30px;{TALL}"),
        "X",
        [
            (8.0, 8.0, 300.0, 57.0),
            (8.0, 50.0, 10.0, 10.0),
            (18.0, 8.0, 30.0, 50.0),
            (48.0, 45.0, 10.0, 20.0),
        ],
        [
            (8.0, 8.0, 300.0, 57.0),
            (8.0, 50.0, 10.0, 10.0),
            (18.0, 8.0, 30.0, 50.0),
            (48.0, 45.0, 10.0, 20.0),
        ],
    );
}

#[test]
fn contain_paint_leaves_the_baseline_of_the_first_line() {
    check_both(
        &format!("contain:paint;{TALL}"),
        "X",
        [
            (8.0, 8.0, 300.0, 50.0),
            (8.0, 18.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 50.0),
            (28.0, 13.0, 10.0, 20.0),
        ],
        [
            (8.0, 8.0, 300.0, 50.0),
            (8.0, 18.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 50.0),
            (28.0, 13.0, 10.0, 20.0),
        ],
    );
}

#[test]
fn contain_size_leaves_the_baseline_of_the_first_line() {
    check_both(
        &format!("contain:size;width:30px;{TALL}"),
        "X",
        [
            (8.0, 8.0, 300.0, 50.0),
            (8.0, 18.0, 10.0, 10.0),
            (18.0, 8.0, 30.0, 50.0),
            (48.0, 13.0, 10.0, 20.0),
        ],
        [
            (8.0, 8.0, 300.0, 50.0),
            (8.0, 18.0, 10.0, 10.0),
            (18.0, 8.0, 30.0, 50.0),
            (48.0, 13.0, 10.0, 20.0),
        ],
    );
}

#[test]
fn an_item_whose_height_comes_from_its_line_gives_the_bottom_of_that_line() {
    // The 30px line makes the item 30px tall, and its bottom is the baseline.
    check_both(
        "contain:layout;line-height:30px",
        "X",
        [
            (8.0, 8.0, 300.0, 37.0),
            (8.0, 30.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 30.0),
            (28.0, 25.0, 10.0, 20.0),
        ],
        [
            (8.0, 8.0, 300.0, 37.0),
            (8.0, 30.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 30.0),
            (28.0, 25.0, 10.0, 20.0),
        ],
    );
}

#[test]
fn an_item_without_a_line_gives_the_bottom_of_its_border_box() {
    check_both(
        "contain:layout;height:50px",
        "",
        [
            (8.0, 8.0, 300.0, 57.0),
            (8.0, 50.0, 10.0, 10.0),
            (18.0, 8.0, 0.0, 50.0),
            (18.0, 45.0, 10.0, 20.0),
        ],
        [
            (8.0, 8.0, 300.0, 57.0),
            (8.0, 50.0, 10.0, 10.0),
            (18.0, 8.0, 0.0, 50.0),
            (18.0, 45.0, 10.0, 20.0),
        ],
    );
}

#[test]
fn the_margin_edge_is_the_baseline_in_a_line_and_the_border_edge_in_a_flex_row() {
    // The border box is 50px tall with a 7px margin below it.
    check_both(
        &format!(
            "contain:layout;{TALL};margin-bottom:7px;border-bottom:3px solid;padding-bottom:4px"
        ),
        "X",
        [
            (8.0, 8.0, 300.0, 57.0),
            (8.0, 50.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 50.0),
            (28.0, 45.0, 10.0, 20.0),
        ],
        [
            (8.0, 8.0, 300.0, 64.0),
            (8.0, 57.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 50.0),
            (28.0, 52.0, 10.0, 20.0),
        ],
    );
}

#[test]
fn a_scroll_container_keeps_the_baseline_of_its_first_line_clamped_to_its_border_box() {
    // In a flex row that is the baseline of the first line; in a line the
    // bottom margin edge is used instead (CSS 2.1 10.8.1).
    check_both(
        &format!("overflow:hidden;{TALL}"),
        "X",
        [
            (8.0, 8.0, 300.0, 50.0),
            (8.0, 18.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 50.0),
            (28.0, 13.0, 10.0, 20.0),
        ],
        [
            (8.0, 8.0, 300.0, 57.0),
            (8.0, 50.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 50.0),
            (28.0, 45.0, 10.0, 20.0),
        ],
    );
}

#[test]
fn an_item_that_aligns_by_its_last_baseline_gives_the_bottom_too() {
    check(
        &flex_row("last baseline", &format!("contain:layout;{TALL}"), "X"),
        [
            (8.0, 8.0, 300.0, 57.0),
            (8.0, 50.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 50.0),
            (28.0, 45.0, 10.0, 20.0),
        ],
    );
}

#[test]
fn a_box_around_a_contained_box_gives_the_bottom_of_it() {
    // The box, 10px wide like its content, has the baseline of the first line
    // or block it holds that exports one, and the contained box exports none.
    check_both(
        "width:10px",
        &blocks(&[CONTAINED]),
        [
            (8.0, 8.0, 300.0, 57.0),
            (8.0, 50.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 50.0),
            (28.0, 45.0, 10.0, 20.0),
        ],
        [
            (8.0, 8.0, 300.0, 57.0),
            (8.0, 50.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 50.0),
            (28.0, 45.0, 10.0, 20.0),
        ],
    );
}

#[test]
fn a_box_around_a_contained_box_and_a_line_gives_the_baseline_of_the_line() {
    // The box, 10px wide like its content, has the baseline of the first line
    // or block it holds that exports one, and the contained box exports none.
    check_both(
        "width:10px",
        &blocks(&[CONTAINED, PLAIN]),
        [
            (8.0, 8.0, 300.0, 80.0),
            (8.0, 68.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 80.0),
            (28.0, 63.0, 10.0, 20.0),
        ],
        [
            (8.0, 8.0, 300.0, 80.0),
            (8.0, 68.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 80.0),
            (28.0, 63.0, 10.0, 20.0),
        ],
    );
}

#[test]
fn a_box_around_a_line_and_a_contained_box_gives_the_baseline_of_the_line() {
    // The box, 10px wide like its content, has the baseline of the first line
    // or block it holds that exports one, and the contained box exports none.
    check_both(
        "width:10px",
        &blocks(&[PLAIN, CONTAINED]),
        [
            (8.0, 8.0, 300.0, 80.0),
            (8.0, 18.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 80.0),
            (28.0, 13.0, 10.0, 20.0),
        ],
        [
            (8.0, 8.0, 300.0, 80.0),
            (8.0, 18.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 80.0),
            (28.0, 13.0, 10.0, 20.0),
        ],
    );
}

#[test]
fn layout_containment_has_no_effect_on_display_contents() {
    // The wrapper generates no box, so the line inside it still gives the
    // baseline.
    check_both(
        "width:10px",
        "<div style=\"display:contents;contain:layout\"><div style=\"line-height:30px\">Y</div></div>",
        [
            (8.0, 8.0, 300.0, 30.0),
            (8.0, 18.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 30.0),
            (28.0, 13.0, 10.0, 20.0),
        ],
        [
            (8.0, 8.0, 300.0, 30.0),
            (8.0, 18.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 30.0),
            (28.0, 13.0, 10.0, 20.0),
        ],
    );
}

#[test]
fn a_flex_item_that_is_an_inline_element_gives_the_bottom_with_layout_containment() {
    // The item is blockified, so containment applies to it.
    check(
        "<div id=o style=\"display:flex;align-items:baseline;width:300px\">\
         <div id=a style=\"line-height:10px\">X</div>\
         <span id=c style=\"contain:layout;height:50px;line-height:30px\">X</span>\
         <div id=z style=\"line-height:20px\">X</div></div>",
        [
            (8.0, 8.0, 300.0, 57.0),
            (8.0, 50.0, 10.0, 10.0),
            (18.0, 8.0, 10.0, 50.0),
            (28.0, 45.0, 10.0, 20.0),
        ],
    );
}
