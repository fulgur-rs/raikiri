//! A block-level box that establishes a block formatting context keeps its
//! border box clear of the margin boxes of the floats beside it: an `auto`
//! width shrinks to the room the float leaves (CSS 2.1 section 9.5). That holds
//! for every box that makes a context of its own, whichever property does it:
//! `display: flow-root`, a scroll container, or layout or paint containment.
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

/// The left, top, width and height of the elements of `body` that have an
/// `id`, on the first page.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         html, body {{ margin: 0 }} body {{ font: 10px/10px Ahem }}</style>{body}"
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

/// A 200px wide container, a 100px square float on `float`'s side, and a
/// 20px tall block `c` whose style is `style` and whose content is `content`.
fn container(float: &str, style: &str, content: &str) -> String {
    format!(
        "<div id=o style='width: 200px'>\
         <div style='float: {float}; width: 100px; height: 100px'></div>\
         <div id=c style='height: 20px; {style}'>{content}</div></div>"
    )
}

/// Every way to make a box a block formatting context root.
const ROOTS: [&str; 11] = [
    "display: flow-root",
    "overflow: hidden",
    "overflow: auto",
    "overflow: scroll",
    "overflow-x: hidden",
    "overflow-y: auto",
    "contain: layout",
    "contain: paint",
    "contain: content",
    "align-content: center",
    "column-count: 2",
];

#[test]
fn an_auto_width_context_root_shrinks_to_the_room_beside_a_float() {
    for style in ROOTS {
        check_boxes(&[
            (
                style,
                &container("left", style, ""),
                &[("o", 0.0, 0.0, 200.0, 20.0), ("c", 100.0, 0.0, 100.0, 20.0)],
            ),
            (
                style,
                &container("right", style, ""),
                &[("o", 0.0, 0.0, 200.0, 20.0), ("c", 0.0, 0.0, 100.0, 20.0)],
            ),
        ]);
    }
}

#[test]
fn text_in_a_context_root_wraps_in_the_room_beside_a_float() {
    check_boxes(&[(
        "overflow: hidden with text",
        "<div id=o style='width: 200px'>\
         <div style='float: left; width: 100px; height: 100px'></div>\
         <div id=c style='overflow: hidden'>aaaaa bbbbb</div></div>",
        &[("o", 0.0, 0.0, 200.0, 20.0), ("c", 100.0, 0.0, 100.0, 20.0)],
    )]);
}

/// A 200px wide container `o` with a float of `float_style` and then `content`.
fn beside(float_style: &str, content: &str) -> String {
    format!(
        "<div id=o style='width: 200px'>\
         <div style='float: left; {float_style}'></div>{content}</div>"
    )
}

#[test]
fn the_room_beside_a_float_is_cut_by_the_boxes_own_edges_and_limits() {
    const FLOAT: &str = "width: 100px; height: 100px";
    check_boxes(&[
        (
            "padding and border",
            &beside(
                FLOAT,
                "<div id=c style='overflow: hidden; padding: 5px 7px; border: 3px solid; \
                 height: 20px'></div>",
            ),
            &[("o", 0.0, 0.0, 200.0, 36.0), ("c", 100.0, 0.0, 100.0, 36.0)],
        ),
        (
            "margin",
            &beside(
                FLOAT,
                "<div id=c style='overflow: hidden; margin: 0 10px; height: 20px'></div>",
            ),
            &[("c", 100.0, 0.0, 90.0, 20.0)],
        ),
        (
            "max-width",
            &beside(
                FLOAT,
                "<div id=c style='overflow: hidden; max-width: 60px; height: 20px'></div>",
            ),
            &[("c", 100.0, 0.0, 60.0, 20.0)],
        ),
        (
            "min-width that does not fit beside the float",
            &beside(
                FLOAT,
                "<div id=c style='overflow: hidden; min-width: 150px; height: 20px'></div>",
            ),
            &[
                ("o", 0.0, 0.0, 200.0, 120.0),
                ("c", 0.0, 100.0, 200.0, 20.0),
            ],
        ),
        (
            "floats on both sides",
            "<div id=o style='width: 200px'>\
             <div style='float: left; width: 60px; height: 100px'></div>\
             <div style='float: right; width: 60px; height: 100px'></div>\
             <div id=c style='overflow: hidden; height: 20px'></div></div>",
            &[("c", 60.0, 0.0, 80.0, 20.0)],
        ),
        (
            "a float shorter than the box, text wrapping beside it",
            &beside(
                "width: 60px; height: 60px",
                "<div id=c style='overflow: hidden'>aaaa bbbb cccc dddd eeee</div>",
            ),
            &[("o", 0.0, 0.0, 200.0, 20.0), ("c", 60.0, 0.0, 140.0, 20.0)],
        ),
        (
            "clear puts the box below the float at the full width",
            &beside(
                "width: 100px; height: 10px",
                "<div id=c style='overflow: hidden; clear: both; height: 20px'></div>",
            ),
            &[("o", 0.0, 0.0, 200.0, 30.0), ("c", 0.0, 10.0, 200.0, 20.0)],
        ),
    ]);
}

#[test]
fn blocks_inside_a_context_root_beside_a_float_measure_from_its_shrunk_width() {
    const FLOAT: &str = "width: 100px; height: 100px";
    check_boxes(&[
        (
            "percentage width",
            &beside(
                FLOAT,
                "<div id=c style='overflow: hidden'>\
                 <div id=k style='width: 50%; height: 10px'></div></div>",
            ),
            &[
                ("o", 0.0, 0.0, 200.0, 10.0),
                ("c", 100.0, 0.0, 100.0, 10.0),
                ("k", 100.0, 0.0, 50.0, 10.0),
            ],
        ),
        (
            "nested context roots",
            &beside(
                FLOAT,
                "<div id=c style='overflow: hidden'><div id=d style='overflow: auto'>\
                 <div id=k style='height: 10px'></div></div></div>",
            ),
            &[
                ("c", 100.0, 0.0, 100.0, 10.0),
                ("d", 100.0, 0.0, 100.0, 10.0),
                ("k", 100.0, 0.0, 100.0, 10.0),
            ],
        ),
        (
            "no float: the full width",
            "<div id=o style='width: 200px'>\
             <div id=c style='overflow: hidden; height: 20px'>\
             <div id=k style='width: 50%; height: 10px'></div></div></div>",
            &[("c", 0.0, 0.0, 200.0, 20.0), ("k", 0.0, 0.0, 100.0, 10.0)],
        ),
        (
            "a narrow float",
            &beside(
                "width: 10px; height: 10px",
                "<div id=c style='overflow: hidden; height: 20px'></div>",
            ),
            &[("o", 0.0, 0.0, 200.0, 20.0), ("c", 10.0, 0.0, 190.0, 20.0)],
        ),
    ]);
}

/// With no float around, the text of a context root wraps inside the content
/// box the root's own padding and border leave, not inside the container's.
#[test]
fn text_in_a_context_root_wraps_inside_its_own_padding_and_border() {
    let in_container = |style: &str, content: &str| {
        format!("<div style='width: 200px'><div id=c style='{style}'>{content}</div></div>")
    };
    check_boxes(&[
        (
            "padding",
            &in_container("overflow: hidden; padding: 10px", "aaaaaaaaaaaaaaaaaa b"),
            &[("c", 0.0, 0.0, 200.0, 40.0)],
        ),
        (
            "border and padding",
            &in_container(
                "contain: layout; border: 5px solid; padding: 10px",
                "aaaaaaaaaaaaaaaa b",
            ),
            &[("c", 0.0, 0.0, 200.0, 50.0)],
        ),
        (
            "paragraph",
            &in_container(
                "overflow: hidden; padding: 10px",
                "<p id=p style='margin: 0'>aaaaaaaaaaaaaaaaaa b</p>",
            ),
            &[("c", 0.0, 0.0, 200.0, 40.0), ("p", 10.0, 10.0, 180.0, 20.0)],
        ),
    ]);
}
