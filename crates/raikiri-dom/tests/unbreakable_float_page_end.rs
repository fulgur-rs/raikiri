//! A float that cannot be split, such as an image, and does not fit in the
//! rest of the page starts the next page whole, also when it sits inside a
//! block: it is not cut as overflow of a block that ends above its bottom,
//! and the block that moves to the next page after it does not move it down
//! a second time (as Chromium prints it).
//!
//! The pages are 100px by 100px with no margin, and the text is 20px Ahem on
//! 30px lines. The expected boxes are the ones Chromium prints for the same
//! markup.

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

/// An image float `height` tall after a block `before` tall, in a block that
/// holds `rest` after the float.
fn image_float(before: u32, height: u32, rest: &str) -> String {
    format!(
        "<div style=\"height:{before}px\"></div><div><img id=o style=\"float:left;width:20px;\
         height:{height}px\">{rest}</div>"
    )
}

/// A block that holds nothing but the float ends above the float's bottom;
/// the float still starts the next page whole.
#[test]
fn a_float_alone_in_its_block_is_not_cut() {
    check("image", &image_float(80, 40, ""), 2, &[(1, 0.0, 40.0)]);
    check(
        "size containment",
        "<div style=\"height:80px\"></div><div><div id=o style=\"float:left;width:20px;\
         height:40px;contain:size\"></div></div>",
        2,
        &[(1, 0.0, 40.0)],
    );
}

/// The lines after the float move to the next page too, and take their
/// block along: the float stays at the top of the page.
#[test]
fn a_float_that_went_ahead_is_not_moved_again_by_its_block() {
    check("short", &image_float(80, 40, "z z z"), 2, &[(1, 0.0, 40.0)]);
    check(
        "taller",
        &image_float(60, 50, "z z z"),
        2,
        &[(1, 0.0, 50.0)],
    );
    check(
        "tallest",
        &image_float(50, 60, "z z z"),
        2,
        &[(1, 0.0, 60.0)],
    );
    check(
        "in an inline box",
        "<div style=\"height:80px\"></div><div><span><img id=o style=\"float:left;\
         width:20px;height:40px\"></span>z z z</div>",
        2,
        &[(1, 0.0, 40.0)],
    );
}

/// Only the padding of its block comes before the float on the page: the
/// float still starts the next page, even when it is as tall as the page.
#[test]
fn a_float_below_padding_alone_starts_the_next_page() {
    for height in [80.0, 100.0] {
        check(
            &format!("{height}px"),
            &format!(
                "<div style=\"padding-top:30px\"><img id=o style=\"float:left;width:20px;\
                 height:{height}px\"></div>"
            ),
            2,
            &[(1, 0.0, height)],
        );
    }
}
