//! The columns of a container with `column-fill: auto` and an auto height fill
//! in turn on each page that cuts the container, and the container goes on at
//! the top of the next page (CSS Multi-column 1 sections 8.1 and 8.2; CSS
//! Fragmentation 3 section 2).
//!
//! The expected places are the ones Chromium gives the same markup when it
//! prints a 480px by 288px page with 48px margins: the page area is 384px wide
//! and 192px high, and `columns: 2` leaves 184px columns and a 16px gap.

use raikiri_html::{
    LayoutOptions, LayoutStatus, RenderResources, layout, parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The margin of the page, which fragments are placed inside.
const MARGIN: f32 = 48.0;

/// One piece of a box on a page: its page, from 0, and its left, top and
/// height on that page's area.
type Piece = (usize, f32, f32, f32);

/// The number of pages and the pieces of each element with an `id`, in the
/// order the pages show them.
fn pieces_of(body: &str) -> (usize, Vec<(String, Vec<Piece>)>) {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 480px 288px; margin: 48px }} \
         body {{ margin: 0 }}</style>{body}"
    );
    let resources = RenderResources::new();
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
    let mut pieces: Vec<(String, Vec<Piece>)> = Vec::new();
    let mut pages = 0;
    for (index, page) in result.pages().enumerate() {
        pages = index + 1;
        let dom = page.dom();
        for fragment in page.fragments() {
            let Some(id) = dom.attr(fragment.node(), "id") else {
                continue;
            };
            let rect = fragment.rect();
            let piece = (index, rect.x - MARGIN, rect.y - MARGIN, rect.height);
            match pieces.iter_mut().find(|(seen, _)| seen == id) {
                Some((_, own)) => own.push(piece),
                None => pieces.push((id.to_owned(), vec![piece])),
            }
        }
    }
    (pages, pieces)
}

/// Lays out `body` and compares the page count and the pieces of each box
/// with the expected ones.
fn check(label: &str, body: &str, expected_pages: usize, expected: &[(&str, &[Piece])]) {
    let (pages, pieces) = pieces_of(body);
    assert_eq!(pages, expected_pages, "{label}: page count, got {pieces:?}");
    for &(id, expected) in expected {
        let (_, got) = pieces
            .iter()
            .find(|(name, _)| name == id)
            .unwrap_or_else(|| panic!("{label}: no fragment for #{id}"));
        assert_eq!(got.len(), expected.len(), "{label}: #{id} pieces {got:?}");
        for (got, expected) in got.iter().zip(expected) {
            assert!(
                got.0 == expected.0
                    && (got.1 - expected.1).abs() <= 0.04
                    && (got.2 - expected.2).abs() <= 0.04
                    && (got.3 - expected.3).abs() <= 0.04,
                "{label}: #{id} piece {got:?}, expected {expected:?}"
            );
        }
    }
}

const COLUMNS: &str = "columns: 2; column-fill: auto";
const AFTER: &str = "<div id=k style='height: 16px'></div>";

/// A box that is taller than a column fills both columns of the first page,
/// and goes on in the first column of the next page, where the container ends
/// with its content.
#[test]
fn a_tall_box_fills_both_columns_of_each_page() {
    check(
        "480px box",
        &format!("<div style='{COLUMNS}'><div id=a style='height: 480px'></div></div>{AFTER}"),
        2,
        &[
            (
                "a",
                &[
                    (0, 0.0, 0.0, 192.0),
                    (0, 200.0, 0.0, 192.0),
                    (1, 0.0, 0.0, 96.0),
                ],
            ),
            ("k", &[(1, 0.0, 96.0, 16.0)]),
        ],
    );
    // The last page is full, so the box after the container starts a page.
    check(
        "1000px box",
        &format!("<div style='{COLUMNS}'><div id=a style='height: 1000px'></div></div>{AFTER}"),
        4,
        &[
            (
                "a",
                &[
                    (0, 0.0, 0.0, 192.0),
                    (0, 200.0, 0.0, 192.0),
                    (1, 0.0, 0.0, 192.0),
                    (1, 200.0, 0.0, 192.0),
                    (2, 0.0, 0.0, 192.0),
                    (2, 200.0, 0.0, 40.0),
                ],
            ),
            ("k", &[(3, 0.0, 0.0, 16.0)]),
        ],
    );
}

/// Content that fits one column stays where it is.
#[test]
fn a_box_that_fits_the_page_stays_in_the_first_column() {
    check(
        "100px box",
        &format!("<div style='{COLUMNS}'><div id=a style='height: 100px'></div></div>{AFTER}"),
        1,
        &[
            ("a", &[(0, 0.0, 0.0, 100.0)]),
            ("k", &[(0, 0.0, 100.0, 16.0)]),
        ],
    );
}

/// Boxes that follow one another continue where the one before them ended,
/// and the container ends on the page whose columns they fill.
#[test]
fn boxes_share_the_columns_of_a_page_in_turn() {
    check(
        "300px and 300px",
        &format!(
            "<div style='{COLUMNS}'><div id=a style='height: 300px'></div>\
             <div id=b style='height: 300px'></div></div>{AFTER}"
        ),
        3,
        &[
            ("a", &[(0, 0.0, 0.0, 192.0), (0, 200.0, 0.0, 108.0)]),
            (
                "b",
                &[
                    (0, 200.0, 108.0, 84.0),
                    (1, 0.0, 0.0, 192.0),
                    (1, 200.0, 0.0, 24.0),
                ],
            ),
            ("k", &[(2, 0.0, 0.0, 16.0)]),
        ],
    );
}

/// A container that starts partway down a page has what is left of the page
/// for its first columns, and a whole page for the ones after.
#[test]
fn a_container_that_starts_partway_down_a_page_fills_what_is_left() {
    check(
        "after a 100px box",
        &format!(
            "<div style='height: 100px'></div><div style='{COLUMNS}'>\
             <div id=a style='height: 250px'></div></div>{AFTER}"
        ),
        2,
        &[
            (
                "a",
                &[
                    (0, 0.0, 100.0, 92.0),
                    (0, 200.0, 100.0, 92.0),
                    (1, 0.0, 0.0, 66.0),
                ],
            ),
            ("k", &[(1, 0.0, 66.0, 16.0)]),
        ],
    );
}

/// Every column of a page is filled before the container goes on, however
/// many there are.
#[test]
fn every_column_of_a_page_is_filled() {
    check(
        "three columns",
        &format!(
            "<div style='columns: 3; column-fill: auto'>\
             <div id=a style='height: 700px'></div></div>{AFTER}"
        ),
        2,
        &[
            (
                "a",
                &[
                    (0, 0.0, 0.0, 192.0),
                    (0, 133.333, 0.0, 192.0),
                    (0, 266.667, 0.0, 192.0),
                    (1, 0.0, 0.0, 124.0),
                ],
            ),
            ("k", &[(1, 0.0, 124.0, 16.0)]),
        ],
    );
}

/// White space between the boxes of the container is no box, so the columns
/// fill as they do without it.
#[test]
fn white_space_between_boxes_does_not_change_the_columns() {
    check(
        "indented boxes",
        &format!(
            "<div style='{COLUMNS}'>\n  <div id=a style='height: 300px'></div>\n  \
             <div id=b style='height: 300px'></div>\n</div>\n{AFTER}"
        ),
        3,
        &[
            ("a", &[(0, 0.0, 0.0, 192.0), (0, 200.0, 0.0, 108.0)]),
            (
                "b",
                &[
                    (0, 200.0, 108.0, 84.0),
                    (1, 0.0, 0.0, 192.0),
                    (1, 200.0, 0.0, 24.0),
                ],
            ),
            ("k", &[(2, 0.0, 0.0, 16.0)]),
        ],
    );
}

/// Columns are filled from the right in a right-to-left container.
#[test]
fn a_right_to_left_container_fills_its_columns_from_the_right() {
    check(
        "direction: rtl",
        &format!(
            "<div style='{COLUMNS}; direction: rtl'>\
             <div id=a style='height: 480px'></div></div>{AFTER}"
        ),
        2,
        &[(
            "a",
            &[
                (0, 200.0, 0.0, 192.0),
                (0, 0.0, 0.0, 192.0),
                (1, 200.0, 0.0, 96.0),
            ],
        )],
    );
}

/// A box with effects that are sized from the whole box is not cut by a page,
/// so its container keeps the single column the pages cut.
#[test]
fn a_box_with_whole_box_effects_keeps_the_column_the_pages_cut() {
    let (pages, pieces) = pieces_of(&format!(
        "<div style='{COLUMNS}'>\
         <div id=a style='height: 480px; box-shadow: 0 0 4px black'></div></div>{AFTER}"
    ));
    assert_eq!(pages, 3, "{pieces:?}");
    let (_, a) = pieces.iter().find(|(id, _)| id == "a").expect("a");
    assert!(a.iter().all(|piece| piece.1 == 0.0), "one column: {a:?}");
}
