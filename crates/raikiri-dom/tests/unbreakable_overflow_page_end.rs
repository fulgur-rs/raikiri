//! A box that no page break goes through (a replaced element, a box with
//! `contain: size`, an orthogonal flow) and that would cross the page end
//! moves to the next page whole, taking the blocks it begins with it. When
//! the box runs out past the bottom of a block it begins whose height ends on
//! the page, the part below that block is visible overflow of the block:
//! Chromium leaves the block where it is and cuts the box at the page end. A
//! box with something before it in its parent has a break before it instead,
//! and moves on its own (CSS Fragmentation 3 sections 4.1 and 5.1.1).
//!
//! The pages are 800px by 600px with no margin, and the text is 10px Ahem.
//! The expected boxes are the ones Chromium 153 prints for the same markup.

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
         body {{ margin: 0; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
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

/// A 100px box that `contain: size` keeps from breaking.
const SIZED: &str = "<div id=o style=\"height:100px;width:10px;contain:size\"></div>";
/// A 100px replaced element laid out as a block.
const IMAGE: &str = "<img id=o style=\"display:block;height:100px;width:10px\" \
    src=\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='100'/%3E\">";
/// A 100px vertical paragraph, an orthogonal flow.
const VERTICAL: &str = "<div id=o style=\"writing-mode:vertical-rl;height:100px\">aa</div>";

/// The box begins a block that ends at 550 on the first page and runs past
/// the page end at 600: it is cut there, and the block stays.
#[test]
fn a_box_overflowing_a_block_it_begins_that_ends_on_the_page_is_cut() {
    let cut = [(0, 540.0, 600.0), (1, 0.0, 40.0)];
    for (name, unbreakable) in [("sized", SIZED), ("image", IMAGE), ("vertical", VERTICAL)] {
        for (parent_name, parent) in [
            ("height", "height:250px;padding-top:240px"),
            ("max-height", "max-height:250px;padding-top:240px"),
            ("border", "height:250px;border-top:240px solid"),
        ] {
            check(
                &format!("{name} in {parent_name} parent"),
                &format!(
                    "<div style=\"height:300px\"></div><div style=\"{parent}\">{unbreakable}</div>"
                ),
                2,
                &cut,
            );
        }
    }
    for (label, body) in [
        (
            "no edges",
            format!("<div style=\"height:540px\"></div><div style=\"height:50px\">{SIZED}</div>"),
        ),
        (
            "auto-height wrapper between",
            format!(
                "<div style=\"height:300px\"></div><div style=\"height:250px;padding-top:240px\">\
                 <div>{SIZED}</div></div>"
            ),
        ),
        (
            "fixed-height wrapper in a parent that crosses the page end",
            format!(
                "<div style=\"height:300px\"></div><div style=\"height:350px;padding-top:240px\">\
                 <div style=\"height:50px\">{SIZED}</div></div>"
            ),
        ),
        (
            "white space before",
            format!(
                "<div style=\"height:300px\"></div><div style=\"height:250px;padding-top:240px\">\
                 \n  {SIZED}</div>"
            ),
        ),
        (
            "absolutely positioned sibling before",
            format!(
                "<div style=\"height:300px\"></div><div style=\"height:250px;padding-top:240px;\
                 position:relative\"><div style=\"position:absolute;top:0;width:5px;height:5px\">\
                 </div>{SIZED}</div>"
            ),
        ),
        (
            "block after the box",
            format!(
                "<div style=\"height:300px\"></div><div style=\"height:250px;padding-top:240px\">\
                 {SIZED}<div style=\"height:20px\"></div></div><div style=\"height:30px\"></div>"
            ),
        ),
    ] {
        check(label, &body, 2, &cut);
    }
}

/// A relative offset moves the box after the page cuts it at its normal-flow
/// place, and a box taller than a page is cut on every page it reaches.
#[test]
fn the_cut_box_keeps_its_offset_and_runs_on_over_pages() {
    check(
        "relative offset",
        "<div style=\"height:300px\"></div><div style=\"height:250px;padding-top:240px\">\
         <div id=o style=\"height:100px;width:10px;contain:size;position:relative;top:-20px\">\
         </div></div>",
        2,
        &[(0, 520.0, 600.0), (1, 0.0, 20.0)],
    );
    check(
        "taller than a page",
        "<div style=\"height:300px\"></div><div style=\"height:250px;padding-top:240px\">\
         <div id=o style=\"height:700px;width:10px;contain:size\"></div></div>",
        3,
        &[(0, 540.0, 600.0), (1, 0.0, 600.0), (2, 0.0, 40.0)],
    );
}

/// A box that would cross the page end and that a block of its own sits
/// before moves to the next page alone, even when it overflows its parent.
#[test]
fn a_box_after_a_sibling_moves_alone() {
    let moved = [(1, 0.0, 100.0)];
    for (name, unbreakable) in [("sized", SIZED), ("vertical", VERTICAL)] {
        for (parent_name, parent) in [
            ("height", "height:250px;padding-top:200px"),
            ("max-height", "max-height:250px;padding-top:200px"),
        ] {
            check(
                &format!("{name} in {parent_name} parent"),
                &format!(
                    "<div style=\"height:300px\"></div><div style=\"{parent}\">\
                     <div style=\"height:40px\"></div>{unbreakable}</div>"
                ),
                2,
                &moved,
            );
        }
    }
    for (label, body) in [
        (
            "empty sibling",
            format!(
                "<div style=\"height:300px\"></div><div style=\"height:250px;padding-top:240px\">\
                 <div style=\"height:0px\"></div>{SIZED}</div>"
            ),
        ),
        (
            "wrapper after a sibling",
            format!(
                "<div style=\"height:300px\"></div><div style=\"height:250px;padding-top:200px\">\
                 <div style=\"height:40px\"></div><div>{SIZED}</div></div>"
            ),
        ),
        (
            "text before",
            format!(
                "<div style=\"height:300px\"></div><div style=\"height:250px;padding-top:230px\">\
                 x{SIZED}</div>"
            ),
        ),
    ] {
        check(label, &body, 2, &moved);
    }
}

/// A box that runs past the page end inside a parent that crosses it too
/// takes the parent to the next page, and one that ends inside its parent on
/// the page stays.
#[test]
fn a_box_inside_its_parent_moves_with_it_or_stays() {
    check(
        "parent crosses the page end",
        &format!(
            "<div style=\"height:300px\"></div><div style=\"height:350px;padding-top:240px\">\
             {SIZED}</div>"
        ),
        2,
        &[(1, 240.0, 340.0)],
    );
    check(
        "vertical box in a parent that crosses the page end",
        &format!(
            "<div style=\"height:300px\"></div><div style=\"max-height:400px;padding-top:240px\">\
             {VERTICAL}</div>"
        ),
        2,
        &[(1, 240.0, 340.0)],
    );
    check(
        "box ends on the page",
        "<div style=\"height:300px\"></div><div style=\"height:250px;padding-top:240px\">\
         <div id=o style=\"height:50px;width:10px;contain:size\"></div></div>",
        1,
        &[(0, 540.0, 590.0)],
    );
}
