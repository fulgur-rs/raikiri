//! A forced break before a block nested at the start of a box that follows
//! a paragraph's lines, inside a block with bottom padding, border or margin,
//! opens one page. When the bottom edges of the outer block would not fit
//! below the box on the first page, the box moves to the next page for them
//! first. The forced break inside it then falls at that page's start, so it
//! opens no further page, as in Chromium (CSS Fragmentation 3, section 3.1).
//!
//! The pages are 100px square with no margin, and the text is 20px Ahem on
//! 30px lines, so a glyph's top sits 5px below its line's top.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The page, from 0, and the top on that page of the first fragment of each
/// element whose id is in `ids`.
fn place(body: &str, ids: &[&str]) -> Vec<(usize, f32)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 100px 100px; margin: 0 }} \
         body {{ margin: 0; font: 20px/30px Ahem }}</style>\
         <div style='width: 20px'>{body}</div>"
    );
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
    let mut found = vec![None; ids.len()];
    for (index, page) in result.pages().enumerate() {
        let dom = page.dom();
        for fragment in page.fragments() {
            let Some(id) = dom.attr(fragment.node(), "id") else {
                continue;
            };
            if let Some(slot) = ids.iter().position(|&wanted| wanted == id) {
                found[slot].get_or_insert((index, fragment.rect().y));
            }
        }
    }
    found
        .into_iter()
        .zip(ids)
        .map(|(place, id)| place.unwrap_or_else(|| panic!("no fragment for #{id}")))
        .collect()
}

fn assert_places(body: &str, expected: &[(&str, (usize, f32))]) {
    let ids: Vec<&str> = expected.iter().map(|(id, _)| *id).collect();
    let got = place(body, &ids);
    for ((id, want), got) in expected.iter().zip(got) {
        assert!(
            got.0 == want.0 && (got.1 - want.1).abs() <= 0.04,
            "#{id}: placed {got:?}, expected {want:?}"
        );
    }
}

/// The paragraph `edges` holds `before`, then a box holding `T` nested
/// `depth` boxes deep with a break before it, after a box `above` high; `Z`
/// follows the paragraph.
fn nested_after_lines(above: u32, edges: &str, before: &str, height: u32, depth: usize) -> String {
    let mut block =
        format!("<div style='height: {height}px; break-before: page'><span id=t>T</span></div>");
    for _ in 0..depth {
        block = format!("<div>{block}</div>");
    }
    format!(
        "<div style='height: {above}px'></div>\
         <div style='{edges}'>{before}{block}</div>\
         <div><span id=z>Z</span></div>"
    )
}

/// The box after the lines fills the first page to its end, so the padding
/// below it would cross the page end: the box starts the second page once.
#[test]
fn a_nested_break_after_lines_opens_one_page_past_the_bottom_padding() {
    assert_places(
        &nested_after_lines(0, "padding-bottom: 1px", "A<br>A", 40, 1),
        &[("t", (1, 5.0)), ("z", (1, 46.0))],
    );
}

/// The same holds for a bottom border and for a box nested two deep.
#[test]
fn a_nested_break_after_lines_opens_one_page_past_the_bottom_border() {
    assert_places(
        &nested_after_lines(40, "border-bottom: 3px solid", "A<br>A", 20, 2),
        &[("t", (1, 5.0)), ("z", (1, 28.0))],
    );
}

/// A block between the lines and the box comes before the box on the first
/// page, so the break before the box still opens just one page.
#[test]
fn a_nested_break_after_lines_and_a_block_opens_one_page() {
    assert_places(
        &nested_after_lines(
            0,
            "padding-bottom: 20px",
            "A<br>A<div style='height: 10px'></div>",
            20,
            1,
        ),
        &[("t", (1, 5.0)), ("z", (1, 45.0))],
    );
}

#[test]
fn a_nested_break_before_the_lines_opens_one_page() {
    assert_places(
        "<div style='height: 50px'></div>\
         <div style='padding-bottom: 20px'>\
         <div><div style='height: 40px; break-before: page'><span id=t>T</span></div></div>\
         <span id=a>A</span></div>\
         <div><span id=z>Z</span></div>",
        &[("t", (1, 5.0)), ("a", (1, 45.0)), ("z", (2, 5.0))],
    );
}
