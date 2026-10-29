//! A relative offset only paints a box away from its place in the flow, so the
//! page a box starts on, and the page the boxes after it start on, are found
//! from the flow (CSS Fragmentation 3 section 5.5; CSS 2.1 section 9.4.3).
//!
//! The expected places are the ones Chromium gives the same markup when it
//! prints a 100px square page with no margin and `10px Ahem` text: each box
//! is the page it is painted on and its top on that page.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The page count and the `(id, page, top)` of each element with an `id`: the
/// page it is first painted on, from 0, and its top on that page.
fn places(boxes: &[&str]) -> (usize, Vec<(String, usize, f32)>) {
    let body: String = boxes
        .iter()
        .enumerate()
        .map(|(index, style)| {
            let id = char::from(b'a' + u8::try_from(index).expect("few boxes"));
            let label = id.to_ascii_uppercase();
            format!("<div id={id} style='width: 100px; {style}'>{label}</div>")
        })
        .collect();
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 100px 100px; margin: 0 }} \
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
    let mut places: Vec<(String, usize, f32)> = Vec::new();
    let mut pages = 0;
    for (index, page) in result.pages().enumerate() {
        pages = index + 1;
        let dom = page.dom();
        for fragment in page.fragments() {
            let Some(id) = dom.attr(fragment.node(), "id") else {
                continue;
            };
            if places.iter().all(|(seen, ..)| seen != id) {
                places.push((id.to_owned(), index, fragment.rect().y));
            }
        }
    }
    (pages, places)
}

/// Lays out `boxes` and compares the pages and the places with the expected
/// ones.
fn check(label: &str, boxes: &[&str], expected_pages: usize, expected: &[(&str, usize, f32)]) {
    let (pages, places) = places(boxes);
    assert_eq!(pages, expected_pages, "{label}: page count");
    for &(id, page, top) in expected {
        let (_, got_page, got_top) = places
            .iter()
            .find(|(name, ..)| name == id)
            .unwrap_or_else(|| panic!("{label}: no fragment for #{id}"));
        assert!(
            *got_page == page && (got_top - top).abs() <= 0.04,
            "{label}: #{id} is on page {got_page} at {got_top}, expected page {page} at {top}"
        );
    }
}

/// A box that is painted on the second page, though the flow puts it on the
/// first, does not move the boxes after it to the second page.
#[test]
fn a_box_painted_on_a_later_page_does_not_move_the_boxes_after_it() {
    check(
        "break after the offset box",
        &[
            "height: 20px",
            "position: relative; top: 100px; height: 20px; break-after: page",
            "height: 20px",
        ],
        2,
        &[("a", 0, 0.0), ("b", 1, 20.0), ("c", 1, 0.0)],
    );
    check(
        "break before the next box",
        &[
            "height: 20px",
            "position: relative; top: 100px; height: 20px",
            "height: 20px; break-before: page",
        ],
        2,
        &[("a", 0, 0.0), ("b", 1, 20.0), ("c", 1, 0.0)],
    );
    check(
        "next box avoids a break inside it",
        &[
            "height: 20px",
            "position: relative; top: 100px; height: 20px",
            "height: 90px; break-inside: avoid",
        ],
        2,
        &[("a", 0, 0.0), ("b", 1, 20.0), ("c", 1, 0.0)],
    );
    check(
        "no break",
        &[
            "height: 20px",
            "position: relative; top: 100px; height: 20px",
            "height: 20px",
        ],
        2,
        &[("a", 0, 0.0), ("b", 1, 20.0), ("c", 0, 40.0)],
    );
}

/// A box painted above the page it starts on in the flow stays in the flow
/// of that page.
#[test]
fn a_box_painted_on_an_earlier_page_does_not_move_the_boxes_after_it() {
    check(
        "negative offset",
        &[
            "height: 100px",
            "position: relative; top: -30px; height: 20px",
            "height: 10px",
        ],
        2,
        &[("a", 0, 0.0), ("b", 0, 70.0), ("c", 1, 20.0)],
    );
}
