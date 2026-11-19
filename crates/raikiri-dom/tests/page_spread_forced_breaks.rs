//! The page-spread values of `break-before` and `break-after` (`left`,
//! `right`, `recto`, `verso`), and `left` / `right` of the legacy
//! `page-break-before` / `page-break-after`, force a page break (CSS
//! Fragmentation 3 sections 3.1 and 3.4).
//!
//! Chromium does not add a blank page to make the next page a left or right
//! one when it prints: the content after the break starts on the next page,
//! as after `page`. The expected places are the ones Chromium gives the same
//! markup when it prints a 100px square page with no margin and `10px Ahem`
//! text: each box is the page it is painted on and its top on that page.

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

#[test]
fn page_spread_break_before_starts_the_next_page() {
    for value in [
        "break-before: left",
        "break-before: right",
        "break-before: recto",
        "break-before: verso",
        "page-break-before: left",
        "page-break-before: right",
    ] {
        check(
            value,
            &["", value, ""],
            2,
            &[("a", 0, 0.0), ("b", 1, 0.0), ("c", 1, 10.0)],
        );
    }
}

#[test]
fn page_spread_break_after_starts_the_next_page() {
    for value in [
        "break-after: left",
        "break-after: right",
        "break-after: recto",
        "break-after: verso",
        "page-break-after: left",
        "page-break-after: right",
    ] {
        check(
            value,
            &["", value, ""],
            2,
            &[("a", 0, 0.0), ("b", 0, 10.0), ("c", 1, 0.0)],
        );
    }
}

/// Two page-spread breaks in a row each open one page, with no blank page
/// between them.
#[test]
fn consecutive_page_spread_breaks_add_no_blank_page() {
    check(
        "verso then recto",
        &["", "break-before: verso", "break-after: recto", ""],
        3,
        &[("a", 0, 0.0), ("b", 1, 0.0), ("c", 1, 10.0), ("d", 2, 0.0)],
    );
}

/// A page-spread break before the first box of the document opens no page.
#[test]
fn page_spread_break_before_the_first_box_opens_no_page() {
    check(
        "first box",
        &["break-before: right", ""],
        1,
        &[("a", 0, 0.0), ("b", 0, 10.0)],
    );
}
