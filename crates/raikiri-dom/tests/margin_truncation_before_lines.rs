//! A line right after a block child of a paragraph starts the page whose
//! edge falls in the block-end margin of that block: Chromium truncates the
//! margin there, as before the first box of an anonymous block. A line of
//! white space that the style preserves, or of a no-break space, is a line
//! like any other: the page breaks between it and the block before it,
//! instead of taking the block along to the next page. The pages and
//! positions are the ones Chromium prints for the same markup: 100px pages
//! without margins, Ahem at 20px on 30px lines.

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

const T: &str = "<span id=t>T</span>";
const Z: &str = "<div><span id=z>Z</span></div>";

/// Four lines of text that end the first page after two lines and fill the
/// second page down to 60px, then a paragraph whose block child `T` ends at
/// 80px with the margin `margin_bottom` below it, then `#z`.
fn after_lines(style: &str, margin_bottom: u32, after: &str) -> String {
    format!(
        "<div style=\"width:20px\"><div>X X X X</div><div style=\"{style}\">\
         <div style=\"height:20px;margin-bottom:{margin_bottom}px\">{T}</div>{after}</div>{Z}</div>"
    )
}

/// A 60px block, then a paragraph whose block child `T` with the style
/// `block` ends at 80px, followed by `after`, then `#z`.
fn after_block(style: &str, block: &str, after: &str) -> String {
    format!(
        "<div style=\"width:20px\"><div style=\"height:60px\"></div><div style=\"{style}\">\
         <div style=\"height:20px;{block}\">{T}</div>{after}</div>{Z}</div>"
    )
}

#[test]
fn a_preserved_white_space_line_after_a_cut_margin_starts_the_next_page() {
    let pre = "white-space:pre";
    assert_places(&[
        (
            after_lines(pre, 40, " <div>A</div>"),
            [(1, 65.0), (2, 65.0)],
        ),
        (
            after_lines(pre, 40, " <div style=\"height:0\"></div>"),
            [(1, 65.0), (2, 35.0)],
        ),
        (after_lines(pre, 40, " "), [(1, 65.0), (2, 35.0)]),
        (
            after_lines(pre, 40, "<span> </span><div>A</div>"),
            [(1, 65.0), (2, 65.0)],
        ),
        (
            after_block(pre, "margin-bottom:40px", " "),
            [(0, 65.0), (1, 35.0)],
        ),
        (
            after_block(pre, "margin-bottom:40px", " <div style=\"height:0\"></div>"),
            [(0, 65.0), (1, 35.0)],
        ),
        (
            after_block("white-space:pre-wrap", "margin-bottom:40px", "  "),
            [(0, 65.0), (1, 35.0)],
        ),
        (
            after_block("", "margin-bottom:40px", "&nbsp;"),
            [(0, 65.0), (1, 35.0)],
        ),
    ]);
}

#[test]
fn a_preserved_white_space_line_that_crosses_the_page_end_leaves_the_block_before_it() {
    let pre = "white-space:pre";
    assert_places(&[
        (after_lines(pre, 10, " "), [(1, 65.0), (2, 35.0)]),
        (after_lines(pre, 20, " "), [(1, 65.0), (2, 35.0)]),
        (after_lines(pre, 25, " "), [(1, 65.0), (2, 35.0)]),
        (
            after_block(pre, "margin-bottom:10px", " "),
            [(0, 65.0), (1, 35.0)],
        ),
        (
            after_block("white-space:break-spaces", "margin-bottom:10px", " "),
            [(0, 65.0), (1, 35.0)],
        ),
        (
            after_block("", "margin-bottom:10px", "&nbsp;"),
            [(0, 65.0), (1, 35.0)],
        ),
    ]);
}

#[test]
fn a_page_edge_in_the_margin_above_a_line_after_a_block_truncates_it() {
    assert_places(&[
        (
            after_block("white-space:pre", "margin-bottom:40px", "<span>Y</span>"),
            [(0, 65.0), (1, 35.0)],
        ),
        (
            after_block("", "margin-bottom:40px", "<span>Y</span>"),
            [(0, 65.0), (1, 35.0)],
        ),
        (
            after_block("", "margin-bottom:40px", "<span>Y</span><br>Y"),
            [(0, 65.0), (1, 65.0)],
        ),
        // A relative offset only paints the block elsewhere.
        (
            after_block("", "margin-bottom:40px;position:relative;top:5px", "Y"),
            [(0, 70.0), (1, 35.0)],
        ),
        (
            after_block("", "height:40px;margin-bottom:40px", "Y"),
            [(0, 65.0), (1, 35.0)],
        ),
        // The paragraph's own padding stays below its last line.
        (
            after_block("padding-bottom:5px", "margin-bottom:40px", "Y"),
            [(0, 65.0), (1, 40.0)],
        ),
        (
            after_block(
                "",
                "margin-bottom:40px",
                "<img style=\"height:20px;width:10px;display:inline-block\">Y",
            ),
            [(0, 65.0), (1, 65.0)],
        ),
        // The moved lines take the flow after the paragraph up with them, and
        // a line of it that then crosses the page end starts the next page.
        (
            after_block("", "margin-bottom:40px", "Y Y Y"),
            [(0, 65.0), (2, 5.0)],
        ),
    ]);
}

#[test]
fn margins_collapsed_into_the_one_above_a_line_are_truncated_with_it() {
    let wrapper = |inner: &str, after: &str| {
        format!(
            "<div style=\"width:20px\"><div style=\"height:60px\"></div><div>{inner}{after}</div>{Z}</div>"
        )
    };
    assert_places(&[
        (
            wrapper(
                &format!(
                    "<div style=\"margin-bottom:40px\"><div style=\"height:20px;margin-bottom:30px\">{T}</div></div>"
                ),
                "Y",
            ),
            [(0, 65.0), (1, 35.0)],
        ),
        (
            wrapper(
                &format!(
                    "<div style=\"height:20px;margin-bottom:10px\">{T}</div><div style=\"height:0;margin-bottom:40px\"></div>"
                ),
                "Y",
            ),
            [(0, 65.0), (1, 35.0)],
        ),
    ]);
}

#[test]
fn a_margin_that_escapes_the_box_before_a_paragraph_is_truncated_above_its_lines() {
    let case = |style: &str, content: &str| {
        format!(
            "<div style=\"width:20px\"><div>X X X X</div><div style=\"{style}\">\
             <div style=\"height:20px;margin-bottom:40px\">{T}</div></div>\
             <div style=\"{style}\">{content}</div>{Z}</div>"
        )
    };
    let pre = "white-space:pre";
    assert_places(&[
        (case("", "Y"), [(1, 65.0), (2, 35.0)]),
        (case(pre, " "), [(1, 65.0), (2, 35.0)]),
        (
            case(pre, " <div style=\"height:0;margin-bottom:30px\"></div>"),
            [(1, 65.0), (2, 65.0)],
        ),
        (
            case(
                pre,
                "Y<div style=\"height:0;margin-bottom:30px\"><div style=\"margin-bottom:10px\"></div></div>",
            ),
            [(1, 65.0), (2, 65.0)],
        ),
    ]);
}

#[test]
fn margins_that_collapse_through_an_empty_box_are_truncated_before_the_box_after_it() {
    // The empty box sits below the margins collapsed through it: only those
    // margins lie between it and the block with `T`.
    let case = |z: &str| {
        format!(
            "<div style=\"width:20px\"><div>X X X X</div><div style=\"height:20px\">{T}</div>\
             <div style=\"height:0;margin-bottom:30px\"><div style=\"margin-bottom:40px\"></div></div>{z}</div>"
        )
    };
    assert_places(&[
        (
            case(&format!("<div style=\"display:contents\">{Z}</div>")),
            [(1, 65.0), (2, 35.0)],
        ),
        (
            case(
                "<div style=\"display:contents\"><div style=\"display:contents\">\
                 <div style=\"margin-top:20px\"><span id=z>Z</span></div></div></div>",
            ),
            [(1, 65.0), (2, 35.0)],
        ),
    ]);
}
