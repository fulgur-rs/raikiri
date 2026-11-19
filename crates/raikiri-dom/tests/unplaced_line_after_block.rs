//! A paragraph whose single line holds no text that places it, such as a
//! `<br>`, atomic inlines alone, generated content or a preserved newline,
//! breaks between a block child above that line and the line, as before a
//! line of text there. Chromium leaves the block on its page and moves the
//! line, with the padding and border that close the boxes after it, to the
//! next page, or starts the page with it when a page edge falls in the
//! block-end margin of the block. The pages and positions are the ones
//! Chromium prints for the same markup: 100px pages without margins, Ahem at
//! 20px on 30px lines.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The page index and the top on that page of the first fragment of each of
/// the elements `#t` and `#z`.
fn places(body: &str) -> [(usize, f32); 2] {
    let html = format!(
        "<!DOCTYPE html><style>@page{{size:100px 100px;margin:0}}\
         body{{margin:0;font:20px/30px Ahem}}</style>{body}"
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
    let mut found = [None, None];
    for (index, page) in result.pages().enumerate() {
        for fragment in page.fragments() {
            let slot = match page.dom().attr(fragment.node(), "id") {
                Some("t") => 0,
                Some("z") => 1,
                _ => continue,
            };
            found[slot].get_or_insert((index, fragment.rect().y));
        }
    }
    found.map(|place| place.unwrap_or_else(|| panic!("no fragment for #t or #z in {body}")))
}

/// Lays out every `(body, [#t, #z])` case and reports all that differ.
fn assert_places(cases: &[(String, [(usize, f32); 2])]) {
    let wrong: Vec<String> = cases
        .iter()
        .filter_map(|(body, expected)| {
            let got = places(body);
            got.iter()
                .zip(expected)
                .any(|(got, expected)| got.0 != expected.0 || (got.1 - expected.1).abs() >= 0.01)
                .then(|| format!("{body}: expected {expected:?}, got {got:?}"))
        })
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

const Z: &str = "<div><span id=z>Z</span></div>";
const BR: &str = "<br>";
const IMG30: &str = "<img style=\"height:30px;width:10px;vertical-align:top\">";
const IMG50: &str = "<img style=\"height:50px;width:10px;vertical-align:top\">";
const IMG120: &str = "<img style=\"height:120px;width:10px;vertical-align:top\">";
const INLINE_BLOCK: &str = "<span style=\"display:inline-block;width:10px;height:20px\"></span>";

/// A block `above` px tall, then a paragraph with the style `style` whose
/// 20px block child `T`, with the margin `margin_bottom` below it, is
/// followed by `line`, then `#z`.
fn paragraph(above: u32, style: &str, margin_bottom: u32, line: &str) -> String {
    format!(
        "<div style=\"width:20px\"><div style=\"height:{above}px\"></div>\
         <div style=\"{style}\"><div style=\"height:20px;margin-bottom:{margin_bottom}px\">\
         <span id=t>T</span></div>{line}</div>{Z}</div>"
    )
}

#[test]
fn line_crossing_the_page_end_moves_without_the_block_before_it() {
    assert_places(&[
        (paragraph(60, "", 0, BR), [(0, 65.0), (1, 35.0)]),
        (paragraph(70, "", 0, BR), [(0, 75.0), (1, 35.0)]),
        (paragraph(60, "", 10, IMG30), [(0, 65.0), (1, 35.0)]),
        (paragraph(40, "", 0, IMG50), [(0, 45.0), (1, 55.0)]),
        (paragraph(70, "", 20, INLINE_BLOCK), [(0, 75.0), (1, 35.0)]),
        // A line that fits stays.
        (paragraph(40, "", 0, BR), [(0, 45.0), (1, 5.0)]),
    ]);
}

#[test]
fn line_taller_than_a_page_moves_too() {
    assert_places(&[(paragraph(50, "", 10, IMG120), [(0, 55.0), (2, 25.0)])]);
}

#[test]
fn page_edge_in_the_margin_of_the_block_starts_the_page_with_the_line() {
    assert_places(&[
        (paragraph(60, "", 40, BR), [(0, 65.0), (1, 35.0)]),
        (paragraph(40, "", 40, BR), [(0, 45.0), (1, 35.0)]),
        (paragraph(70, "", 40, IMG30), [(0, 75.0), (1, 35.0)]),
        (paragraph(40, "", 20, BR), [(0, 45.0), (1, 35.0)]),
    ]);
}

#[test]
fn closing_edges_go_with_the_line() {
    assert_places(&[
        (
            paragraph(40, "padding-bottom:10px", 10, BR),
            [(0, 45.0), (1, 45.0)],
        ),
        (
            paragraph(60, "border-bottom:5px solid", 10, IMG30),
            [(0, 65.0), (1, 40.0)],
        ),
        (
            paragraph(60, "padding-bottom:10px", 40, BR),
            [(0, 65.0), (1, 45.0)],
        ),
        // The padding of a box that the paragraph ends.
        (
            format!(
                "<div style=\"width:20px\"><div style=\"height:40px\"></div>\
                 <div style=\"padding-bottom:10px\"><div><div style=\"height:20px;margin-bottom:10px\">\
                 <span id=t>T</span></div><br></div></div>{Z}</div>"
            ),
            [(0, 45.0), (1, 45.0)],
        ),
    ]);
}

#[test]
fn generated_and_preserved_lines_break_from_the_block() {
    assert_places(&[
        (
            format!(
                "<style>.g::after{{content:\"\";display:inline-block;width:10px;height:20px}}</style>\
                 {}",
                paragraph(60, "", 10, "").replace("<div style=\"\">", "<div class=g>")
            ),
            [(0, 65.0), (1, 35.0)],
        ),
        (
            "<div style=\"width:20px\"><div style=\"height:60px\"></div><div style=\"white-space:pre\">\
             <div style=\"height:20px\"><span id=t>T</span></div>&#10;<div><span id=z>Z</span></div></div></div>"
                .to_owned(),
            [(0, 65.0), (1, 35.0)],
        ),
        (
            "<div style=\"width:20px\"><div style=\"height:70px\"></div><div style=\"white-space:pre\">\
             <div style=\"height:20px;margin-bottom:40px\"><span id=t>T</span></div>&#10;<div><span id=z>Z</span></div></div></div>"
                .to_owned(),
            [(0, 75.0), (1, 35.0)],
        ),
    ]);
}

#[test]
fn line_between_blocks_relative_paragraph_and_body_child() {
    assert_places(&[
        // A block after the line follows it.
        (
            paragraph(60, "", 0, "<br><div>U</div>"),
            [(0, 65.0), (1, 65.0)],
        ),
        (
            paragraph(60, "position:relative;top:5px", 10, BR),
            [(0, 70.0), (1, 35.0)],
        ),
        (
            format!(
                "<div style=\"height:60px\"></div><div style=\"width:20px\">\
                 <div style=\"height:20px;margin-bottom:30px\"><span id=t>T</span></div>{IMG30}</div>{Z}"
            ),
            [(0, 65.0), (1, 35.0)],
        ),
    ]);
}

#[test]
fn block_inside_an_inline_box_moves_with_the_line() {
    assert_places(&[(
        format!(
            "<div style=\"width:20px\"><div style=\"height:60px\"></div><div><span>\
             <div style=\"height:20px\"><span id=t>T</span></div><br></span></div>{Z}</div>"
        ),
        [(1, 5.0), (1, 55.0)],
    )]);
}

#[test]
fn block_that_avoids_a_break_after_it_moves_with_the_line() {
    let avoiding = |above: u32| {
        format!(
            "<div style=\"width:20px\"><div style=\"height:{above}px\"></div><div>\
             <div style=\"height:20px;break-after:avoid\"><span id=t>T</span></div><br></div>{Z}</div>"
        )
    };
    assert_places(&[
        (avoiding(60), [(1, 5.0), (1, 55.0)]),
        (avoiding(50), [(0, 55.0), (1, 5.0)]),
    ]);
}

#[test]
fn line_in_a_flex_or_grid_item_breaks_from_the_block_too() {
    let item = |above: u32, container: &str, margin_bottom: u32| {
        format!(
            "<div style=\"width:20px\"><div style=\"height:{above}px\"></div>\
             <div style=\"{container}\"><div><div style=\"height:20px;margin-bottom:{margin_bottom}px\">\
             <span id=t>T</span></div><br></div></div>{Z}</div>"
        )
    };
    assert_places(&[
        (item(60, "display:flex", 10), [(0, 65.0), (1, 35.0)]),
        (item(70, "display:flex", 0), [(0, 75.0), (1, 35.0)]),
        (
            item(60, "display:flex;flex-direction:column", 0),
            [(0, 65.0), (1, 35.0)],
        ),
        (item(70, "display:grid", 0), [(0, 75.0), (1, 35.0)]),
        // A paragraph nested in a row flex item.
        (
            format!(
                "<div style=\"width:40px\"><div style=\"height:50px\"></div><div style=\"display:flex\">\
                 <div style=\"width:20px\"><div><div style=\"height:20px;margin-bottom:10px\">\
                 <span id=t>T</span></div><br></div></div></div>{Z}</div>"
            ),
            [(0, 55.0), (1, 35.0)],
        ),
    ]);
}
