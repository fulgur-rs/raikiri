//! A block inside an inline box of a paragraph lies on a line of its own in
//! that paragraph, and Chromium counts it as one line for `orphans` and
//! `widows`. A page break before such a block that leaves fewer than
//! `orphans` lines of the paragraph before it on the page moves those lines
//! to the next page with the block; otherwise the lines that `widows` asks
//! for beyond the block go along, as far as `orphans` lets them. A block
//! straight in the paragraph is not a line, and the lines before it stay.
//! The pages and positions are the ones Chromium prints for the same markup:
//! 100px pages without margins, Ahem at 20px on 30px lines.

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

/// A block `height` px tall that holds `#t`.
fn block(height: u32) -> String {
    format!("<div style=\"height:{height}px\"><span id=t>T</span></div>")
}

/// A block `above` px tall, then a paragraph with the style `style` that
/// holds `content`, then `#z`.
fn paragraph(above: u32, style: &str, content: &str) -> String {
    format!(
        "<div style=\"width:20px\"><div style=\"height:{above}px\"></div>\
         <div style=\"{style}\">{content}</div><div><span id=z>Z</span></div></div>"
    )
}

/// `lines` lines of `A`, then a block `height` px tall inside the same span.
fn in_span(lines: usize, height: u32) -> String {
    format!(
        "<span>{}{}</span>",
        vec!["A"; lines].join("<br>"),
        block(height)
    )
}

#[test]
fn too_few_orphans_before_the_block_move_with_it() {
    assert_places(&[
        (paragraph(50, "", &in_span(1, 40)), [(1, 35.0), (1, 75.0)]),
        (paragraph(50, "", &in_span(1, 120)), [(1, 35.0), (2, 55.0)]),
        // In a span of their own, the lines still share the paragraph.
        (
            paragraph(50, "", &format!("<span>A</span><span>{}</span>", block(40))),
            [(1, 35.0), (1, 75.0)],
        ),
        // The paragraph goes whole, with its padding and border above the
        // line.
        (
            paragraph(50, "border-top:5px solid;padding-top:5px", &in_span(1, 40)),
            [(1, 45.0), (2, 5.0)],
        ),
        // `orphans: 1` leaves the line on its page.
        (
            paragraph(50, "orphans:1", &in_span(1, 40)),
            [(1, 5.0), (1, 45.0)],
        ),
        // A block straight in the paragraph is no line.
        (
            paragraph(50, "", &format!("A{}", block(40))),
            [(1, 5.0), (1, 45.0)],
        ),
    ]);
}

#[test]
fn a_monolithic_block_takes_the_lines_too() {
    let img = "<span>A<div><img id=t style=\"display:block;width:10px;height:40px\"></div></span>";
    assert_places(&[(paragraph(50, "", img), [(1, 30.0), (1, 75.0)])]);
}

#[test]
fn widows_take_a_line_before_the_block() {
    assert_places(&[
        (paragraph(10, "", &in_span(3, 20)), [(1, 35.0), (1, 55.0)]),
        (paragraph(10, "", &in_span(3, 80)), [(1, 35.0), (2, 15.0)]),
        (
            paragraph(10, "widows:3", &in_span(3, 20)),
            [(1, 35.0), (1, 55.0)],
        ),
        (
            paragraph(10, "widows:1", &in_span(3, 20)),
            [(1, 5.0), (1, 25.0)],
        ),
        // Orphans come first.
        (
            paragraph(10, "orphans:3", &in_span(3, 20)),
            [(1, 5.0), (1, 25.0)],
        ),
        // Lines after the block do not count.
        (
            paragraph(10, "", &format!("<span>A<br>A<br>A{}C</span>", block(20))),
            [(1, 35.0), (2, 5.0)],
        ),
        // The paragraph moved whole for its orphans still keeps its widows.
        (paragraph(50, "", &in_span(3, 20)), [(2, 35.0), (2, 55.0)]),
    ]);
}

#[test]
fn the_block_is_one_of_the_widows_of_a_break_between_the_lines() {
    assert_places(&[
        (
            paragraph(30, "orphans:1", &in_span(3, 20)),
            [(1, 35.0), (1, 55.0)],
        ),
        (
            paragraph(30, "orphans:1", &in_span(3, 80)),
            [(1, 35.0), (2, 15.0)],
        ),
    ]);
}

#[test]
fn a_block_whose_first_piece_fits_breaks_inside() {
    assert_places(&[
        (paragraph(10, "", &in_span(1, 80)), [(0, 45.0), (1, 25.0)]),
        (paragraph(40, "", &in_span(1, 40)), [(0, 75.0), (1, 15.0)]),
    ]);
}
