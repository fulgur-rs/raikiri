//! A page break avoided before a box is no better than one avoided inside
//! it. Chromium rates the two alike and takes the later one: a box that
//! avoids breaks inside it, with a break avoided before it, is split where
//! it stands rather than moved on, once nothing before it can take a better
//! break. A box whose first piece no break goes through, such as an image,
//! still moves on: cutting that piece is worse than either.
//!
//! A run of boxes tied by avoided breaks moves to the next page when it
//! crosses the page end, though it does not fit there, as long as no break
//! inside it is better than the one before it. Content that runs out past a
//! box that avoids breaks inside it counts as part of that box.
//!
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

const AVOID_BOTH: &str = "break-before:avoid;break-inside:avoid";

/// In a 20px wide block: a block `above` px tall, then a paragraph of
/// `lines` lines of `A` and then `block`, then `#z`.
fn paragraph(above: u32, lines: usize, block: &str) -> String {
    format!(
        "<div style=\"width:20px\"><div style=\"height:{above}px\"></div>\
         <div>{}{block}</div><div><span id=z>Z</span></div></div>",
        vec!["A"; lines].join("<br>")
    )
}

/// In a 20px wide block: a block `above` px tall, then `boxes` blocks of
/// one line each, then `block`, then `#z`.
fn boxes(above: u32, boxes: usize, block: &str) -> String {
    format!(
        "<div style=\"width:20px\"><div style=\"height:{above}px\"></div>{}{block}\
         <div><span id=z>Z</span></div></div>",
        "<div>B</div>".repeat(boxes)
    )
}

/// A block `height` px tall with the style `style` that holds `#t`.
fn block(height: u32, style: &str) -> String {
    format!("<div style=\"height:{height}px;{style}\"><span id=t>T</span></div>")
}

/// A block with the style `style` that holds an 80px image, then a block
/// that holds `#t`.
fn image_box(style: &str) -> String {
    format!(
        "<div style=\"{style}\"><img style=\"width:10px;height:80px\"></div>\
         <div><span id=t>T</span></div>"
    )
}

#[test]
fn a_box_with_both_breaks_avoided_splits_where_it_stands() {
    assert_places(&[
        // The last line starts the second page and cannot move: the block
        // after it splits there.
        (
            paragraph(30, 3, &block(80, AVOID_BOTH)),
            [(1, 35.0), (2, 15.0)],
        ),
        // The paragraph of one line moves whole with the block first, and the
        // last line goes with it, then the block splits.
        (
            paragraph(10, 1, &block(80, AVOID_BOTH)),
            [(1, 35.0), (2, 15.0)],
        ),
        (
            paragraph(10, 3, &block(120, AVOID_BOTH)),
            [(1, 35.0), (2, 55.0)],
        ),
        (
            paragraph(60, 1, &block(100, AVOID_BOTH)),
            [(1, 35.0), (2, 35.0)],
        ),
        // A block that fits stays where it is.
        (
            paragraph(10, 1, &block(60, AVOID_BOTH)),
            [(0, 45.0), (1, 5.0)],
        ),
        // Between blocks, the box before moves on with the block.
        (boxes(30, 1, &block(80, AVOID_BOTH)), [(1, 35.0), (2, 15.0)]),
        (
            boxes(70, 1, &block(120, AVOID_BOTH)),
            [(1, 35.0), (2, 55.0)],
        ),
        // The break after the box before is avoided instead.
        (
            "<div style=\"width:20px\"><div style=\"height:10px\"></div>\
             <div style=\"break-after:avoid\">B</div>"
                .to_owned()
                + &block(80, "break-inside:avoid")
                + "<div><span id=z>Z</span></div></div>",
            [(1, 35.0), (2, 15.0)],
        ),
        // Without the avoided break before it, the block moves on alone.
        (
            boxes(30, 1, &block(80, "break-inside:avoid")),
            [(1, 5.0), (2, 5.0)],
        ),
    ]);
}

#[test]
fn a_box_with_both_breaks_avoided_splits_among_the_body_children() {
    assert_places(&[(
        "<div style=\"height:30px\"></div><div>B</div>".to_owned()
            + &block(80, AVOID_BOTH)
            + "<div><span id=z>Z</span></div>",
        [(1, 35.0), (2, 15.0)],
    )]);
}

#[test]
fn a_run_too_tall_for_a_page_moves_on_when_nothing_inside_it_breaks_better() {
    let chain = |above: u32, count: usize| {
        format!(
            "<div style=\"width:20px\"><div style=\"height:{above}px\"></div>\
             <div><span id=t>T</span></div>{}<div><span id=z>Z</span></div></div>",
            "<div style=\"break-before:avoid\">B</div>".repeat(count - 1)
        )
    };
    let tied = |above: u32, last: &str| {
        format!(
            "<div style=\"width:20px\"><div style=\"height:{above}px\"></div>\
             <div><span id=t>T</span></div>{last}<div><span id=z>Z</span></div></div>"
        )
    };
    assert_places(&[
        // One-line blocks hold no break inside them.
        (chain(30, 4), [(1, 5.0), (2, 35.0)]),
        // A run that fits its page stays.
        (chain(10, 3), [(0, 15.0), (1, 5.0)]),
        (
            tied(
                30,
                "<div style=\"break-before:avoid\"><img style=\"display:block;\
                 width:10px;height:80px\"></div>",
            ),
            [(1, 5.0), (3, 5.0)],
        ),
        (
            tied(
                10,
                "<div style=\"break-before:avoid\">B</div>\
                 <div style=\"break-before:avoid;break-inside:avoid;height:120px\">X</div>",
            ),
            [(1, 5.0), (3, 5.0)],
        ),
    ]);
    // Among the children of the body.
    assert_places(&[(
        "<div style=\"height:50px\"></div><div><span id=t>T</span></div>".to_owned()
            + &"<div style=\"break-before:avoid\">B</div>".repeat(4)
            + "<div><span id=z>Z</span></div>",
        [(1, 5.0), (2, 65.0)],
    )]);
}

#[test]
fn content_running_out_of_a_box_that_avoids_breaks_inside_counts_as_the_box() {
    // The 20px block fits the page, but its 30px line runs past the end.
    assert_places(&[
        (
            paragraph(50, 1, &block(20, AVOID_BOTH)),
            [(1, 35.0), (1, 55.0)],
        ),
        (
            paragraph(50, 1, &block(20, "break-inside:avoid")),
            [(1, 5.0), (1, 25.0)],
        ),
        (boxes(50, 1, &block(20, AVOID_BOTH)), [(1, 35.0), (1, 55.0)]),
        (
            boxes(50, 1, &block(20, "break-inside:avoid")),
            [(1, 5.0), (1, 25.0)],
        ),
        // The line ends at the page end.
        (
            paragraph(40, 1, &block(20, AVOID_BOTH)),
            [(0, 75.0), (1, 5.0)],
        ),
    ]);
}

#[test]
fn a_box_whose_first_piece_is_an_image_moves_past_the_avoided_break() {
    let avoided = "break-before:avoid";
    assert_places(&[
        (
            paragraph(10, 1, &image_box(&format!("display:flex;{avoided}"))),
            [(3, 5.0), (3, 35.0)],
        ),
        (
            paragraph(
                10,
                1,
                &image_box(&format!("display:flex;flex-direction:column;{avoided}")),
            ),
            [(3, 5.0), (3, 35.0)],
        ),
        (
            paragraph(60, 1, &image_box(&format!("display:grid;{avoided}"))),
            [(3, 5.0), (3, 35.0)],
        ),
        (paragraph(10, 1, &image_box(avoided)), [(3, 5.0), (3, 35.0)]),
        // The box before moves on with the flex container first.
        (
            boxes(30, 1, &image_box(&format!("display:flex;{avoided}"))),
            [(3, 5.0), (3, 35.0)],
        ),
        // Text beside the image in the row.
        (
            paragraph(
                30,
                1,
                &format!(
                    "<div style=\"display:flex;{avoided}\"><img style=\"width:10px;\
                     height:80px\"><div>B<br>B<br>B</div></div><div><span id=t>T</span></div>"
                ),
            ),
            [(3, 5.0), (3, 35.0)],
        ),
        // Without the avoided break, the line stays and the container moves.
        (
            paragraph(30, 1, &image_box("display:flex")),
            [(2, 5.0), (2, 35.0)],
        ),
    ]);
}
