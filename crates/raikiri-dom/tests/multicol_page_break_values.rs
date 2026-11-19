//! A page break value (`page`, `left`, `right`, `recto`, `verso`) on a box
//! inside a multicol container. Chromium ignores it when it prints: the box
//! stays in the columns where it would be without the value, and the content
//! after the container stays on the page.
//!
//! The expected places are the ones Chromium gives the same markup when it
//! prints a 100px square page with no margin and `10px Ahem` text.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The page count and the `(id, page, x, y)` of the first fragment of each
/// element with an `id`.
fn places(body: &str) -> (usize, Vec<(String, usize, f32, f32)>) {
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
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("expected a complete layout");
    };
    let mut places: Vec<(String, usize, f32, f32)> = Vec::new();
    let mut pages = 0;
    for (index, page) in result.pages().enumerate() {
        pages = index + 1;
        let dom = page.dom();
        for fragment in page.fragments() {
            let Some(id) = dom.attr(fragment.node(), "id") else {
                continue;
            };
            if places.iter().all(|(seen, ..)| seen != id) {
                let rect = fragment.rect();
                places.push((id.to_owned(), index, rect.x, rect.y));
            }
        }
    }
    (pages, places)
}

fn check(body: &str, expected_pages: usize, expected: &[(&str, usize, f32, f32)]) {
    let (pages, places) = places(body);
    assert_eq!(pages, expected_pages, "{body}: page count");
    for &(id, page, x, y) in expected {
        let (_, got_page, got_x, got_y) = places
            .iter()
            .find(|(name, ..)| name == id)
            .unwrap_or_else(|| panic!("{body}: no fragment for #{id}"));
        assert!(
            *got_page == page && (got_x - x).abs() <= 0.04 && (got_y - y).abs() <= 0.04,
            "{body}: #{id} is on page {got_page} at ({got_x}, {got_y}), \
             expected page {page} at ({x}, {y})"
        );
    }
}

fn boxes(ids: &str, styled: char, style: &str) -> String {
    ids.chars()
        .map(|id| {
            let css = if id == styled { style } else { "" };
            format!(
                "<div id={} style='{css}'>{id}</div>",
                id.to_ascii_lowercase()
            )
        })
        .collect()
}

const VALUES: [&str; 5] = ["page", "left", "right", "recto", "verso"];

#[test]
fn a_page_value_in_balanced_columns_changes_nothing() {
    for value in VALUES {
        let body = format!(
            "<div style='columns:2;column-gap:0;width:100px'>{}</div><div id=x>X</div>",
            boxes("ABCD", 'B', &format!("break-before:{value}"))
        );
        check(
            &body,
            1,
            &[
                ("a", 0, 0.0, 0.0),
                ("b", 0, 0.0, 10.0),
                ("c", 0, 50.0, 0.0),
                ("d", 0, 50.0, 10.0),
                ("x", 0, 0.0, 20.0),
            ],
        );
    }
}

#[test]
fn a_page_value_in_filled_columns_changes_nothing() {
    for value in VALUES {
        let body = format!(
            "<div style='columns:2;column-gap:0;width:100px;column-fill:auto;height:40px'>{}</div>\
             <div id=x>X</div>",
            boxes("ABC", 'B', &format!("break-after:{value}"))
        );
        check(
            &body,
            1,
            &[
                ("a", 0, 0.0, 0.0),
                ("b", 0, 0.0, 10.0),
                ("c", 0, 0.0, 20.0),
                ("x", 0, 0.0, 40.0),
            ],
        );
    }
}

#[test]
fn a_page_value_in_columns_across_pages_changes_nothing() {
    for value in ["page", "right"] {
        let body = format!(
            "<div style='columns:2;column-gap:0;width:100px'>{}</div><div id=x>X</div>",
            boxes(
                "ABCDEFGHIJKLMNOPQRSTUVW",
                'M',
                &format!("break-before:{value}")
            )
        );
        check(
            &body,
            2,
            &[
                ("a", 0, 0.0, 0.0),
                ("j", 0, 0.0, 90.0),
                ("k", 0, 50.0, 0.0),
                ("m", 0, 50.0, 20.0),
                ("t", 0, 50.0, 90.0),
                ("u", 1, 0.0, 0.0),
                ("v", 1, 0.0, 10.0),
                ("w", 1, 50.0, 0.0),
                ("x", 1, 0.0, 20.0),
            ],
        );
    }
}

const COLUMNS: &str = "columns:2;column-gap:0;width:100px";

/// A spanner leaves the columns of its container, so a page value on its
/// edge with the page context still breaks the page: before it, between it
/// and another spanner, and after the container's last spanner.
#[test]
fn a_page_value_on_a_spanner_breaks_the_page() {
    check(
        &format!(
            "<div style='{COLUMNS}'>A<div id=s style='column-span:all;break-before:page'>S</div>\
             C</div><div id=x>X</div>"
        ),
        2,
        &[("s", 1, 0.0, 0.0)],
    );
    check(
        &format!(
            "<div style='{COLUMNS}'>A<div id=s style='column-span:all;break-after:page'>S</div>\
             <div id=t style='column-span:all'>T</div>C</div>"
        ),
        2,
        &[("s", 0, 0.0, 10.0)],
    );
    check(
        &format!(
            "<div style='{COLUMNS}'>A<br>B<div id=s style='column-span:all;break-after:page'>S</div>\
             </div><div id=x>X</div>"
        ),
        2,
        &[("s", 0, 0.0, 20.0), ("x", 1, 0.0, 0.0)],
    );
}

/// A page value between a spanner and the column content after it falls in
/// the columns, and so does one on a spanner of a multicol container that
/// itself sits in columns.
#[test]
fn a_page_value_from_a_spanner_into_columns_changes_nothing() {
    check(
        &format!(
            "<div style='{COLUMNS}'>A<div id=s style='column-span:all;break-after:page'>S</div>\
             C</div><div id=x>X</div>"
        ),
        1,
        &[("s", 0, 0.0, 10.0), ("x", 0, 0.0, 30.0)],
    );
    check(
        &format!(
            "<div style='{COLUMNS}'>A<div style='{COLUMNS}'>B\
             <div id=s style='column-span:all;break-before:page'>S</div>C</div></div>\
             <div id=x>X</div>"
        ),
        1,
        &[],
    );
}

/// An empty box with a page value inside columns leaves the content after
/// it on the page too.
#[test]
fn a_page_value_on_an_empty_box_in_columns_changes_nothing() {
    for edge in ["before", "after"] {
        check(
            &format!(
                "<div style='{COLUMNS}'><div style='break-{edge}:page'></div>C<br>D</div>\
                 <div id=x>X</div>"
            ),
            1,
            &[],
        );
    }
}
