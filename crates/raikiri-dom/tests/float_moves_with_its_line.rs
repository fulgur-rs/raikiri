//! A float placed on a line of a paragraph goes with that line when the line
//! moves to the next page at the page end: it starts the next page beside the
//! line rather than staying where the layout put it and being cut there
//! (as Chromium prints it). A float placed beside an
//! earlier line stays with that line, and a float with no line beside it is
//! cut at the page end like any box.
//!
//! The pages are 100px by 100px with no margin, and the text is 20px Ahem on
//! 30px lines. The expected boxes are the ones Chromium 153 prints for the
//! same markup.

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
        "<!DOCTYPE html><style>@page {{ size: 100px 100px; margin: 0 }} \
         body {{ margin: 0; font: 20px/30px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
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

/// Checks the block-axis extents of `#o` on each page: `(page, top, bottom)`.
fn check(label: &str, body: &str, page_count: usize, expected: &[(usize, f32, f32)]) {
    let (pages, boxes) = pages_of(body);
    let got: Vec<(usize, f32, f32)> = boxes
        .iter()
        .filter(|(id, _)| id == "o")
        .map(|(_, (page, _, y, _, height))| (*page, *y, y + height))
        .collect();
    assert_eq!(pages, page_count, "{label}: page count, got {got:?}");
    let near = |a: f32, b: f32| (a - b).abs() <= 0.04;
    assert!(
        got.len() == expected.len()
            && got
                .iter()
                .zip(expected)
                .all(|(g, e)| g.0 == e.0 && near(g.1, e.1) && near(g.2, e.2)),
        "{label}: #o got {got:?}, expected {expected:?}"
    );
}

/// A 20px square float.
const FLOAT: &str = "<div id=o style=\"float:left;width:20px;height:20px\"></div>";

/// The line beside the float would cross the page end after the 90px block,
/// so it starts the next page, and the float goes with it.
#[test]
fn a_float_goes_to_the_next_page_with_its_line() {
    let moved = [(1, 0.0, 20.0)];
    for (label, body) in [
        (
            "after a block",
            format!("<div style=\"height:90px\"></div>{FLOAT}z z z"),
        ),
        (
            "after the first word",
            format!("<div style=\"height:90px\"></div>z{FLOAT} z"),
        ),
        (
            "right float",
            "<div style=\"height:90px\"></div><div id=o style=\"float:right;width:20px;\
             height:20px\"></div>z z z"
                .to_owned(),
        ),
        (
            "inside an inline box",
            format!("<div style=\"height:90px\"></div><span>{FLOAT}z z z</span>"),
        ),
        (
            "in a flow root",
            format!(
                "<div style=\"display:flow-root\"><div style=\"height:90px\"></div>{FLOAT}z z z\
                 </div>w"
            ),
        ),
        (
            "between two blocks",
            format!(
                "<div style=\"height:50px\"></div>z<div style=\"height:10px\"></div>{FLOAT}z z z"
            ),
        ),
        (
            "with a block after the lines",
            format!(
                "<div style=\"height:90px\"></div>{FLOAT}z z z<div style=\"height:10px\"></div>"
            ),
        ),
        (
            "own paragraph",
            format!("<div style=\"height:90px\"></div><div>{FLOAT}z z z</div>"),
        ),
        (
            "own paragraph that moves to keep its orphans",
            format!("<div style=\"height:60px\"></div><div>{FLOAT}z z z z z z z</div>"),
        ),
    ] {
        check(label, &body, 2, &moved);
    }
    check(
        "top margin",
        "<div style=\"height:90px\"></div><div id=o style=\"float:left;width:20px;height:20px;\
         margin-top:10px\"></div>z z z",
        2,
        &[(1, 10.0, 30.0)],
    );
}

/// The float fits on the page beside its line, but the lines move whole to
/// the next page to keep their orphans: the float goes along.
#[test]
fn a_float_that_fits_goes_with_the_lines_that_keep_their_orphans() {
    check(
        "tall float",
        "<div style=\"height:55px\"></div><div id=o style=\"float:left;width:20px;height:40px\">\
         </div>z z z z z z",
        2,
        &[(1, 0.0, 40.0)],
    );
}

/// A float beside lines that stay on the page stays too, and one with no line
/// beside it is cut at the page end.
#[test]
fn a_float_without_a_moved_line_stays() {
    check(
        "beside lines that stay",
        "<div style=\"height:30px\"></div><div id=o style=\"float:left;width:20px;height:60px\">\
         </div>z z z z z z z z",
        2,
        &[(0, 30.0, 90.0)],
    );
    check(
        "no line beside it",
        &format!("<div style=\"height:90px\"></div>{FLOAT}"),
        2,
        &[(0, 90.0, 100.0), (1, 0.0, 10.0)],
    );
}

/// A float that cannot be split and does not fit goes to the next page by
/// itself, and its line follows it there: the float starts the page beside
/// the line and does not move down again with it.
#[test]
fn a_float_that_went_ahead_stays_at_the_top_of_the_page() {
    check(
        "image",
        "<div style=\"height:80px\"></div><img id=o style=\"float:left;width:20px;height:40px\">\
         z z z",
        2,
        &[(1, 0.0, 40.0)],
    );
}

/// A paragraph with a definite height keeps it when its last line moves to
/// the next page; the float placed after it goes there too, and the
/// formatting context around them still holds the float.
#[test]
fn a_float_past_a_paragraph_of_fixed_height_stays_in_its_context() {
    let paragraph = |root: &str, float: &str| {
        format!(
            "<div{root} style=\"display:flow-root\"><div style=\"height:60px\"></div>\
             <div style=\"height:30px\">z<br><div{float} style=\"float:left;width:20px;\
             height:50px\"></div>z</div></div>"
        )
    };
    check("float", &paragraph("", " id=o"), 2, &[(1, 30.0, 80.0)]);
    check(
        "formatting context",
        &paragraph(" id=o", ""),
        2,
        &[(0, 0.0, 100.0), (1, 0.0, 80.0)],
    );
}

/// A relatively positioned inline around the float shifts the float but not
/// the line it sits on: the line decides whether the float moves.
#[test]
fn a_float_in_a_relatively_positioned_inline_goes_by_its_line() {
    check(
        "line moves",
        "<div style=\"height:90px\"></div><span style=\"position:relative;top:5px\">\
         <div id=o style=\"float:left;width:20px;height:20px\"></div></span>z z z",
        2,
        &[(1, 5.0, 25.0)],
    );
    check(
        "line stays",
        "<div style=\"height:30px\"></div><span style=\"position:relative;top:60px\">\
         <div id=o style=\"float:left;width:20px;height:20px\"></div></span>z z z z z z z z",
        2,
        &[(0, 90.0, 100.0), (1, 0.0, 10.0)],
    );
}
