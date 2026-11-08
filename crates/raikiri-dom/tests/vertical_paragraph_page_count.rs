//! A vertical paragraph in a horizontal flow paginated along y stacks its line
//! boxes across the page, along x. Their logical block offsets are left and
//! right edges there, not tops and bottoms, so a paragraph whose lines reach
//! past the page height is still one piece on one page: no blank page goes in
//! front of it, and no blank pages follow it. The paragraph is an orthogonal
//! flow, which no page break goes through, so one that would cross the page
//! end moves to the next page whole (CSS Writing Modes 4 section 7.3; CSS
//! Fragmentation 3 sections 2 and 5.1.1).
//!
//! The pages are 800px by 600px with no margin, and the text is 10px Ahem, so
//! a line box of the paragraph is 10px wide. The expected boxes are the ones
//! Chromium gives the same markup on screen.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A box of an element with an `id`: its page, from 0, and its left, top,
/// width and height on that page.
type Placed = (usize, f32, f32, f32, f32);

/// The number of pages and the boxes of each element with an `id`.
fn pages_of(body: &str) -> (usize, Vec<(String, Placed)>) {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }}</style>{body}"
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
    let mut boxes = Vec::new();
    let mut pages = 0;
    for (index, page) in result.pages().enumerate() {
        pages = index + 1;
        let dom = page.dom();
        for fragment in page.fragments() {
            if let Some(id) = dom.attr(fragment.node(), "id") {
                let rect = fragment.rect();
                boxes.push((
                    id.to_owned(),
                    (index, rect.x, rect.y, rect.width, rect.height),
                ));
            }
        }
    }
    (pages, boxes)
}

/// Checks that `body` lays out on `page_count` pages with exactly the
/// `expected` boxes, each given with its page: a box that the page cuts is
/// one entry per page.
fn check(label: &str, body: &str, page_count: usize, expected: &[(&str, Placed)]) {
    let (pages, boxes) = pages_of(body);
    assert_eq!(pages, page_count, "{label}: page count, got {boxes:?}");
    assert_eq!(boxes.len(), expected.len(), "{label}: boxes {boxes:?}");
    let near = |a: f32, b: f32| (a - b).abs() <= 0.04;
    for (id, (page, x, y, width, height)) in expected {
        let (_, got) = boxes
            .iter()
            .find(|(name, placed)| name == id && placed.0 == *page)
            .unwrap_or_else(|| {
                panic!("{label}: no fragment for #{id} on page {page}, got {boxes:?}")
            });
        assert!(
            near(got.1, *x) && near(got.2, *y) && near(got.3, *width) && near(got.4, *height),
            "{label}: #{id} got {got:?}, expected {:?}",
            (page, x, y, width, height)
        );
    }
}

/// Checks that `body` lays out on one page with exactly the `expected` boxes.
fn check_one_page(label: &str, body: &str, expected: &[(&str, [f32; 4])]) {
    let expected: Vec<(&str, Placed)> = expected
        .iter()
        .map(|&(id, [x, y, width, height])| (id, (0, x, y, width, height)))
        .collect();
    check(label, body, 1, &expected);
}

/// A horizontal inline-block whose 800px line box makes the paragraph's
/// lines reach x = 800, past the 600px page height.
const WIDE_ATOMIC: &str = "<div id=o style=\"display:inline-block;writing-mode:horizontal-tb\">\
    wwwwwwwww wwwwwwwww wwwwwwwww wwwwwwwww wwwwwwwww wwwwwwwww \
    wwwwwwwww wwwwwwwww wwwwwwwww wwwwwwwww wwwwwwwww wwwwwwwww</div>";

#[test]
fn text_then_wide_atomic_inline_stays_on_one_page() {
    check_one_page(
        "vertical-lr",
        &format!("<div id=p style=\"writing-mode:vertical-lr\">aa bb cc{WIDE_ATOMIC}</div>"),
        &[
            ("p", [0.0, 0.0, 800.0, 100.0]),
            ("o", [0.0, 80.0, 800.0, 20.0]),
        ],
    );
    check_one_page(
        "vertical-rl",
        &format!("<div id=p style=\"writing-mode:vertical-rl\">aa bb cc{WIDE_ATOMIC}</div>"),
        &[
            ("p", [0.0, 0.0, 800.0, 100.0]),
            ("o", [0.0, 80.0, 800.0, 20.0]),
        ],
    );
}

#[test]
fn wide_atomic_inline_then_text_stays_on_one_page() {
    check_one_page(
        "atomic first",
        &format!("<div id=p style=\"writing-mode:vertical-lr\">{WIDE_ATOMIC} dd ee</div>"),
        &[
            ("p", [0.0, 0.0, 800.0, 80.0]),
            ("o", [0.0, 0.0, 800.0, 20.0]),
        ],
    );
}

#[test]
fn lines_past_the_page_height_add_no_trailing_page() {
    check_one_page(
        "sized atomic, no text",
        "<div id=p style=\"writing-mode:vertical-lr\"><div id=o style=\"display:inline-block;\
         writing-mode:horizontal-tb;width:800px;height:10px\"></div></div>",
        &[
            ("p", [0.0, 0.0, 800.0, 10.0]),
            ("o", [0.0, 0.0, 800.0, 10.0]),
        ],
    );
    check_one_page(
        "vertical-lr, text before a 610px atomic",
        "<div id=p style=\"writing-mode:vertical-lr\">aa<div id=o style=\"display:inline-block;\
         writing-mode:horizontal-tb;width:610px;height:10px\"></div></div>",
        &[
            ("p", [0.0, 0.0, 610.0, 30.0]),
            ("o", [0.0, 20.0, 610.0, 10.0]),
        ],
    );
    check_one_page(
        "vertical-rl, text before a 610px atomic",
        "<div id=p style=\"writing-mode:vertical-rl\">aa<div id=o style=\"display:inline-block;\
         writing-mode:horizontal-tb;width:610px;height:10px\"></div></div>",
        &[
            ("p", [0.0, 0.0, 610.0, 30.0]),
            ("o", [0.0, 20.0, 610.0, 10.0]),
        ],
    );
    check_one_page(
        "610px line height",
        "<div id=p style=\"writing-mode:vertical-lr;line-height:610px\">aa</div>",
        &[("p", [0.0, 0.0, 610.0, 20.0])],
    );
    check_one_page(
        "610px line height, block after",
        "<div id=p style=\"writing-mode:vertical-lr;line-height:610px\">aa</div>\
         <div id=q>after</div>",
        &[
            ("p", [0.0, 0.0, 610.0, 20.0]),
            ("q", [0.0, 20.0, 800.0, 10.0]),
        ],
    );
}

#[test]
fn many_lines_across_the_page_add_no_trailing_pages() {
    let words = "aaaa ".repeat(900);
    check_one_page(
        "auto height",
        &format!("<div id=p style=\"writing-mode:vertical-lr\">{words}</div>"),
        &[("p", [0.0, 0.0, 750.0, 600.0])],
    );
    check_one_page(
        "100px height",
        &format!("<div id=p style=\"writing-mode:vertical-lr;height:100px\">{words}</div>"),
        &[("p", [0.0, 0.0, 4500.0, 100.0])],
    );
}

/// The paragraph is an orthogonal flow, which no page break goes through: one
/// that would cross the page end moves to the next page whole, as Chromium
/// prints it, rather than leaving a slice of each of its columns behind.
#[test]
fn paragraph_crossing_the_page_end_moves_whole() {
    check(
        "auto height",
        "<div style=\"height:590px\"></div>\
         <div id=p style=\"writing-mode:vertical-lr\">aa bb</div>",
        2,
        &[("p", (1, 0.0, 0.0, 10.0, 50.0))],
    );
    check(
        "fixed height",
        "<div style=\"height:590px\"></div>\
         <div id=p style=\"writing-mode:vertical-lr;height:20px\">aa bb cc</div>",
        2,
        &[("p", (1, 0.0, 0.0, 30.0, 20.0))],
    );
    check(
        "lines past the page height",
        "<div style=\"height:590px\"></div>\
         <div id=p style=\"writing-mode:vertical-lr;line-height:610px\">aa bb</div>",
        2,
        &[("p", (1, 0.0, 0.0, 610.0, 50.0))],
    );
}

/// A relative offset moves the paragraph after fragmentation: whether it
/// overflows its parent and crosses the page end is decided at its normal-flow
/// place, so it moves whole, and the offset then moves it down on the new page.
#[test]
fn a_relative_offset_does_not_make_the_paragraph_overflow_its_parent() {
    check(
        "relative",
        "<div style=\"height:550px\"></div><div><div id=p style=\"writing-mode:vertical-rl;\
         height:100px;position:relative;top:100px\">aa</div></div>",
        2,
        &[("p", (1, 0.0, 100.0, 10.0, 100.0))],
    );
}

/// A paragraph that runs out past the bottom of a parent whose height ends on
/// the page is overflow of that parent: Chromium cuts it at the page end
/// rather than moving it on.
#[test]
fn paragraph_overflowing_a_parent_that_ends_on_the_page_is_cut() {
    for (label, parent) in [
        ("height", "height:250px;padding-top:240px"),
        ("max-height", "max-height:250px;padding-top:240px"),
    ] {
        check(
            label,
            &format!(
                "<div style=\"height:300px\"></div><div style=\"{parent};box-sizing:border-box\">\
                 <div id=p style=\"writing-mode:vertical-rl;height:100px\">aa</div></div>"
            ),
            2,
            &[
                ("p", (0, 0.0, 540.0, 10.0, 60.0)),
                ("p", (1, 0.0, 0.0, 10.0, 40.0)),
            ],
        );
    }
}

/// The text of a paragraph of several text nodes, or one placed in positioned
/// coordinates, stays in its columns.
#[test]
fn text_stays_in_its_columns() {
    // The inline boxes are checked along y only: the place of an inline box
    // across its column is not what this test is about.
    let (pages, boxes) = pages_of(
        "<div style=\"height:300px\"></div><div id=p style=\"writing-mode:vertical-lr;\
         height:20px\">aa bb <b id=b>cc</b> dd <i id=i>ee</i></div>",
    );
    assert_eq!(pages, 1, "several text nodes: page count, got {boxes:?}");
    for (id, (page, x, y, width, height)) in &boxes {
        let (expected_x, expected_width) = match id.as_str() {
            "p" => (Some(0.0), 50.0),
            _ => (None, 10.0),
        };
        assert!(
            *page == 0
                && expected_x.is_none_or(|expected: f32| (x - expected).abs() <= 0.04)
                && (0.0..=40.0).contains(x)
                && (y - 300.0).abs() <= 0.04
                && (width - expected_width).abs() <= 0.04
                && (height - 20.0).abs() <= 0.04,
            "several text nodes: #{id} got {:?}",
            (page, x, y, width, height)
        );
    }
    check_one_page(
        "positioned coordinates",
        "<div style=\"position:absolute;top:0\"><div id=p style=\"writing-mode:vertical-lr;\
         height:20px\"><span id=s style=\"display:inline-block;width:10px;height:20px\">\
         </span>aa bb</div></div>",
        &[("p", [0.0, 0.0, 30.0, 20.0]), ("s", [0.0, 0.0, 10.0, 20.0])],
    );
}
