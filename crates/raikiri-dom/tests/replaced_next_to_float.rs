//! A block-level replaced element keeps its border box clear of the margin
//! boxes of the floats in its block formatting context (CSS 2.1 section 9.5):
//! it sits beside a float that leaves it room, and below the floats when it does
//! not fit.
//!
//! The expected boxes are the ones Chromium computes for the same markup, with
//! no body margin and `10px Ahem` text: each is the element's left, top, width
//! and height.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A 1x1 image.
const GIF: &str = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

/// The left, top, width and height of the elements of `body` that have an
/// `id`, on the first page.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         html, body {{ margin: 0 }} body {{ font: 10px/10px Ahem }} \
         div {{ box-sizing: border-box }}</style>{body}"
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
            Some((id.to_owned(), [rect.x, rect.y, rect.width, rect.height]))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.04;

/// A label, the markup of a case and the `(id, left, top, width, height)` it
/// expects.
type Case<'a> = (&'a str, &'a str, &'a [(&'a str, f32, f32, f32, f32)]);

/// Lays out each case and compares the elements it names with the expected
/// boxes.
fn check_boxes(cases: &[Case<'_>]) {
    for &(label, body, expected) in cases {
        let actual = boxes(body);
        for &(id, left, top, width, height) in expected {
            let (_, rect) = actual
                .iter()
                .find(|(name, _)| name == id)
                .unwrap_or_else(|| panic!("{label}: no fragment for #{id}"));
            let want = [left, top, width, height];
            assert!(
                rect.iter()
                    .zip(want)
                    .all(|(got, want)| (got - want).abs() <= TOLERANCE),
                "{label}: #{id} is {rect:?}, expected {want:?} (left, top, width, height)"
            );
        }
    }
}

/// A 200px wide container, a 100px square float on the left, and then
/// `content`; `style` adds to the container's style.
fn container(style: &str, float: &str, content: &str) -> String {
    format!(
        "<div id=o style='width: 200px; {style}'>\
         <div style='float: {float}; width: 100px; height: 100px'></div>{content}</div>"
    )
}

/// A block-level canvas whose style is `style`.
fn canvas(style: &str) -> String {
    format!("<canvas id=c width=1 height=1 style='display: block; {style}'></canvas>")
}

/// A block-level image whose style is `style`.
fn image(style: &str) -> String {
    format!("<img id=c src='{GIF}' style='display: block; {style}'>")
}

const SIZE: &str = "width: 50px; height: 20px;";

/// A replaced element that fits in the room the float leaves is beside it, and
/// one that does not is below the float.
#[test]
fn a_block_level_replaced_element_sits_beside_a_float_or_below_it() {
    check_boxes(&[
        (
            "canvas that fits",
            &container("", "left", &canvas(SIZE)),
            &[("o", 0.0, 0.0, 200.0, 20.0), ("c", 100.0, 0.0, 50.0, 20.0)],
        ),
        (
            "image that fits",
            &container("", "left", &image(SIZE)),
            &[("o", 0.0, 0.0, 200.0, 20.0), ("c", 100.0, 0.0, 50.0, 20.0)],
        ),
        (
            "canvas that is too wide",
            &container("", "left", &canvas("width: 150px; height: 20px")),
            &[
                ("o", 0.0, 0.0, 200.0, 120.0),
                ("c", 0.0, 100.0, 150.0, 20.0),
            ],
        ),
        (
            "image that is too wide",
            &container("", "left", &image("width: 150px; height: 20px")),
            &[
                ("o", 0.0, 0.0, 200.0, 120.0),
                ("c", 0.0, 100.0, 150.0, 20.0),
            ],
        ),
        (
            "width as a percentage of the container",
            &container("", "left", &canvas("width: 100%; height: 20px")),
            &[("c", 0.0, 100.0, 200.0, 20.0)],
        ),
        (
            "float on the right",
            &container("", "right", &canvas(SIZE)),
            &[("c", 0.0, 0.0, 50.0, 20.0)],
        ),
        (
            "right to left",
            &container("direction: rtl", "right", &canvas(SIZE)),
            &[("c", 50.0, 0.0, 50.0, 20.0)],
        ),
        (
            "natural size",
            &container("", "left", &canvas("")),
            &[("o", 0.0, 0.0, 200.0, 1.0), ("c", 100.0, 0.0, 1.0, 1.0)],
        ),
        (
            "taller than the float",
            &container(
                "",
                "left",
                &format!(
                    "{}<div id=p style='height: 10px'></div>",
                    canvas("width: 50px; height: 150px")
                ),
            ),
            &[
                ("c", 100.0, 0.0, 50.0, 150.0),
                ("p", 0.0, 150.0, 200.0, 10.0),
            ],
        ),
        (
            "followed by a block",
            &container(
                "",
                "left",
                &format!("{}<div id=p style='height: 10px'></div>", canvas(SIZE)),
            ),
            &[
                ("o", 0.0, 0.0, 200.0, 30.0),
                ("c", 100.0, 0.0, 50.0, 20.0),
                ("p", 0.0, 20.0, 200.0, 10.0),
            ],
        ),
        (
            "after a block",
            &format!(
                "<div id=o style='width: 200px'><div style='height: 30px'></div>\
                 <div style='float: left; width: 100px; height: 100px'></div>{}</div>",
                canvas(SIZE)
            ),
            &[("o", 0.0, 0.0, 200.0, 50.0), ("c", 100.0, 30.0, 50.0, 20.0)],
        ),
    ]);
}

/// The margins, the sizes and the clearance of the replaced element decide where
/// it goes beside the float.
#[test]
fn the_margins_and_the_size_of_a_replaced_element_place_it_beside_a_float() {
    check_boxes(&[
        (
            "a left margin lies under the float",
            &container(
                "",
                "left",
                &canvas("width: 50px; height: 20px; margin-left: 10px"),
            ),
            &[("c", 100.0, 0.0, 50.0, 20.0)],
        ),
        (
            "a top margin",
            &container(
                "",
                "left",
                &canvas("width: 50px; height: 20px; margin-top: 10px"),
            ),
            &[
                ("o", 0.0, 10.0, 200.0, 20.0),
                ("c", 100.0, 10.0, 50.0, 20.0),
            ],
        ),
        (
            "auto margins centre it in the room",
            &container(
                "",
                "left",
                &canvas("width: 50px; height: 20px; margin: 0 auto"),
            ),
            &[("c", 125.0, 0.0, 50.0, 20.0)],
        ),
        (
            "stretch fills the room",
            &container("", "left", &canvas("width: stretch; height: 20px")),
            &[("c", 100.0, 0.0, 100.0, 20.0)],
        ),
        (
            "clear",
            &container(
                "",
                "left",
                &canvas("width: 50px; height: 20px; clear: both"),
            ),
            &[("o", 0.0, 0.0, 200.0, 120.0), ("c", 0.0, 100.0, 50.0, 20.0)],
        ),
        (
            "floats on both sides",
            &format!(
                "<div id=o style='width: 200px'>\
                 <div style='float: left; width: 100px; height: 100px'></div>\
                 <div style='float: right; width: 100px; height: 100px'></div>{}</div>",
                canvas(SIZE)
            ),
            &[("o", 0.0, 0.0, 200.0, 120.0), ("c", 0.0, 100.0, 50.0, 20.0)],
        ),
        (
            "a margin that collapses through the container",
            &format!(
                "<div id=o style='width: 200px'><div style='height: 10px'></div>\
                 <div style='float: left; width: 100px; height: 100px'></div>{}</div>",
                canvas("width: 50px; height: 20px; margin-top: 30px")
            ),
            &[("o", 0.0, 0.0, 200.0, 60.0), ("c", 100.0, 40.0, 50.0, 20.0)],
        ),
        (
            "a flow root next to the float, as before",
            &container("display: flow-root", "left", &canvas(SIZE)),
            &[("o", 0.0, 0.0, 200.0, 100.0), ("c", 100.0, 0.0, 50.0, 20.0)],
        ),
    ]);
}
