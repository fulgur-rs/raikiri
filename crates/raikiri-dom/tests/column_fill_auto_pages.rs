//! The columns of a container with `column-fill: auto` and an auto height fill
//! in turn on each page that cuts the container, and the container goes on at
//! the top of the next page (CSS Multi-column 1 sections 8.1 and 8.2; CSS
//! Fragmentation 3 section 2).
//!
//! The expected places are the ones Chromium gives the same markup when it
//! prints a 480px by 288px page with 48px margins: the page area is 384px wide
//! and 192px high, and `columns: 2` leaves 184px columns and a 16px gap.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

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

/// A container that a forced break or a box kept whole moves down fills the
/// columns of each page from where it lands, as one that starts there does.
#[test]
fn a_container_the_pages_move_fills_the_columns_where_it_lands() {
    let lands_on_the_second_page: &[(&str, &[Piece])] = &[
        ("g", &[(0, 0.0, 0.0, 60.0)]),
        ("a", &[(1, 0.0, 0.0, 192.0), (1, 200.0, 0.0, 108.0)]),
        ("k", &[(2, 0.0, 0.0, 16.0)]),
    ];
    let container = format!("<div style='{COLUMNS}'><div id=a style='height: 300px'></div></div>");
    for (label, body) in [
        (
            "break-before",
            format!(
                "<div id=g style='height: 60px'></div>\
                 <div style='{COLUMNS}; break-before: page'><div id=a style='height: 300px'></div></div>"
            ),
        ),
        (
            "break-before on a wrapper",
            format!(
                "<div id=g style='height: 60px'></div><div style='break-before: page'>{container}</div>"
            ),
        ),
        (
            "break-after",
            format!("<div id=g style='height: 60px; break-after: page'></div>{container}"),
        ),
    ] {
        check(
            label,
            &format!("{body}{AFTER}"),
            3,
            lands_on_the_second_page,
        );
    }
    check(
        "kept whole",
        &format!(
            "<div id=g style='height: 150px'></div>\
             <div id=h style='break-inside: avoid; height: 60px'></div>\
             {container}{AFTER}"
        ),
        3,
        &[
            ("h", &[(1, 0.0, 0.0, 60.0)]),
            (
                "a",
                &[
                    (1, 0.0, 60.0, 132.0),
                    (1, 200.0, 60.0, 132.0),
                    (2, 0.0, 0.0, 36.0),
                ],
            ),
            ("k", &[(2, 0.0, 36.0, 16.0)]),
        ],
    );
}

/// Two containers that a forced break moves together each fill the columns
/// of their own pages: the first fills both columns of the page it lands on,
/// which leaves nothing of that page for the second.
#[test]
fn containers_that_a_forced_break_moves_together_fill_their_own_pages() {
    for (label, second) in [("one break", ""), ("two breaks", "; break-before: page")] {
        check(
            label,
            &format!(
                "<div id=g style='height: 60px'></div>\
                 <div style='{COLUMNS}; break-before: page'><div id=a style='height: 300px'></div></div>\
                 <div style='{COLUMNS}{second}'><div id=b style='height: 300px'></div></div>{AFTER}"
            ),
            4,
            &[
                ("a", &[(1, 0.0, 0.0, 192.0), (1, 200.0, 0.0, 108.0)]),
                ("b", &[(2, 0.0, 0.0, 192.0), (2, 200.0, 0.0, 108.0)]),
                ("k", &[(3, 0.0, 0.0, 16.0)]),
            ],
        );
    }
}

/// Boxes that plain wrappers hold fill the columns of each page in turn, as
/// boxes the container holds itself do, and the wrappers break with them.
#[test]
fn boxes_inside_wrappers_fill_the_columns_of_each_page() {
    let tall_in_columns: &[Piece] = &[
        (0, 0.0, 0.0, 192.0),
        (0, 200.0, 0.0, 192.0),
        (1, 0.0, 0.0, 96.0),
    ];
    check(
        "one wrapper",
        &format!(
            "<div style='{COLUMNS}'><div id=w>\
             <div id=a style='height: 480px'></div></div></div>{AFTER}"
        ),
        2,
        &[
            ("w", tall_in_columns),
            ("a", tall_in_columns),
            ("k", &[(1, 0.0, 96.0, 16.0)]),
        ],
    );
    check(
        "nested wrappers",
        &format!(
            "<div style='{COLUMNS}'><div id=w><div id=v>\
             <div id=a style='height: 480px'></div></div></div></div>{AFTER}"
        ),
        2,
        &[
            ("w", tall_in_columns),
            ("v", tall_in_columns),
            ("a", tall_in_columns),
            ("k", &[(1, 0.0, 96.0, 16.0)]),
        ],
    );
    let continued: &[Piece] = &[
        (0, 0.0, 100.0, 92.0),
        (0, 200.0, 0.0, 192.0),
        (1, 0.0, 0.0, 16.0),
    ];
    check(
        "two boxes in a wrapper",
        &format!(
            "<div style='{COLUMNS}'><div id=w>\
             <div id=a style='height: 100px'></div>\
             <div id=b style='height: 300px'></div></div></div>{AFTER}"
        ),
        2,
        &[
            ("a", &[(0, 0.0, 0.0, 100.0)]),
            ("b", continued),
            ("k", &[(1, 0.0, 16.0, 16.0)]),
        ],
    );
    check(
        "a box and a wrapper",
        &format!(
            "<div style='{COLUMNS}'><div id=a style='height: 100px'></div>\
             <div id=w><div id=b style='height: 300px'></div></div></div>{AFTER}"
        ),
        2,
        &[
            ("a", &[(0, 0.0, 0.0, 100.0)]),
            ("w", continued),
            ("b", continued),
            ("k", &[(1, 0.0, 16.0, 16.0)]),
        ],
    );
    check(
        "a box with a top margin",
        &format!(
            "<div style='{COLUMNS}'><div id=w>\
             <div id=a style='margin-top: 20px; height: 400px'></div></div></div>{AFTER}"
        ),
        2,
        &[
            (
                "a",
                &[
                    (0, 0.0, 20.0, 172.0),
                    (0, 200.0, 0.0, 192.0),
                    (1, 0.0, 0.0, 36.0),
                ],
            ),
            ("k", &[(1, 0.0, 36.0, 16.0)]),
        ],
    );
    check(
        "a padded wrapper",
        &format!(
            "<div style='{COLUMNS}'><div id=w style='padding: 10px 0'>\
             <div id=a style='height: 400px'></div></div></div>{AFTER}"
        ),
        2,
        // The wrapper slices its padding, so each of its pieces is the whole
        // box moved up, which the pages show in parts; the box it holds
        // shows where the padding leaves it.
        &[
            (
                "a",
                &[
                    (0, 0.0, 10.0, 182.0),
                    (0, 200.0, 0.0, 192.0),
                    (1, 0.0, 0.0, 26.0),
                ],
            ),
            ("k", &[(1, 0.0, 36.0, 16.0)]),
        ],
    );
    check(
        "boxes split by an empty box",
        &format!(
            "<div style='{COLUMNS}'><div id=w>\
             <div id=a style='height: 200px'></div><div style='height: 20px'></div>\
             <div id=b style='height: 200px'></div></div></div>{AFTER}"
        ),
        2,
        &[
            (
                "w",
                &[
                    (0, 0.0, 0.0, 192.0),
                    (0, 200.0, 0.0, 192.0),
                    (1, 0.0, 0.0, 36.0),
                ],
            ),
            ("a", &[(0, 0.0, 0.0, 192.0), (0, 200.0, 0.0, 8.0)]),
            ("b", &[(0, 200.0, 28.0, 164.0), (1, 0.0, 0.0, 36.0)]),
            ("k", &[(1, 0.0, 36.0, 16.0)]),
        ],
    );
    check(
        "a wrapper partway down a page",
        &format!(
            "<div style='height: 60px'></div><div style='{COLUMNS}'><div id=w>\
             <div id=a style='height: 480px'></div></div></div>{AFTER}"
        ),
        3,
        &[
            (
                "a",
                &[
                    (0, 0.0, 60.0, 132.0),
                    (0, 200.0, 60.0, 132.0),
                    (1, 0.0, 0.0, 192.0),
                    (1, 200.0, 0.0, 24.0),
                ],
            ),
            ("k", &[(2, 0.0, 0.0, 16.0)]),
        ],
    );
}

/// A wrapper with 10px of padding above and below the boxes it holds.
fn padded(inner: &str) -> String {
    format!("<div id=w style='padding: 10px 0'>{inner}</div>")
}

/// A box inside a wrapper that slices its padding at a column end goes on at
/// the top of the next column, though the wrapper's piece there is the whole
/// wrapper moved up (CSS Fragmentation 3, 5.4), whether the columns fill in
/// turn or balance.
#[test]
fn boxes_in_a_sliced_wrapper_go_on_in_the_next_column() {
    let one = padded("<div id=a style='height: 200px'></div>");
    check(
        "column-fill: auto",
        &format!("<div style='columns: 2; column-fill: auto; height: 150px'>{one}</div>"),
        1,
        &[("a", &[(0, 0.0, 10.0, 140.0), (0, 200.0, 0.0, 60.0)])],
    );
    check(
        "balanced",
        &format!("<div style='columns: 2; height: 150px'>{one}</div>"),
        1,
        &[("a", &[(0, 0.0, 10.0, 100.0), (0, 200.0, 0.0, 100.0)])],
    );
    let two =
        padded("<div id=a style='height: 100px'></div><div id=b style='height: 100px'></div>");
    check(
        "two boxes",
        &format!("<div style='columns: 2; column-fill: auto; height: 150px'>{two}</div>"),
        1,
        &[
            ("a", &[(0, 0.0, 10.0, 100.0)]),
            ("b", &[(0, 0.0, 110.0, 40.0), (0, 200.0, 0.0, 60.0)]),
        ],
    );
}

/// Columns that break before the content ends on a page reach the end of
/// that page, even on the last page the container fills, so what follows the
/// container starts the next page. Content that ends in the first column of
/// its page ends the container there.
#[test]
fn columns_that_break_on_the_last_page_reach_its_end() {
    let avoid =
        |height: u32| format!("<div id=b style='height: {height}px; break-inside: avoid'></div>");
    check(
        "a box that avoids breaks moves to the second column",
        &format!(
            "<div style='{COLUMNS}'><div id=a style='height: 150px'></div>{}</div>{AFTER}",
            avoid(150)
        ),
        2,
        &[
            ("a", &[(0, 0.0, 0.0, 150.0)]),
            ("b", &[(0, 200.0, 0.0, 150.0)]),
            ("k", &[(1, 0.0, 0.0, 16.0)]),
        ],
    );
    check(
        "a shorter first column",
        &format!(
            "<div style='{COLUMNS}'><div id=a style='height: 100px'></div>{}</div>{AFTER}",
            avoid(150)
        ),
        2,
        &[
            ("b", &[(0, 200.0, 0.0, 150.0)]),
            ("k", &[(1, 0.0, 0.0, 16.0)]),
        ],
    );
    check(
        "on a later page",
        &format!(
            "<div style='{COLUMNS}'><div id=a style='height: 500px'></div>{}</div>{AFTER}",
            avoid(150)
        ),
        3,
        &[
            ("b", &[(1, 200.0, 0.0, 150.0)]),
            ("k", &[(2, 0.0, 0.0, 16.0)]),
        ],
    );
    check(
        "partway down a page",
        &format!(
            "<div style='height: 60px'></div><div style='{COLUMNS}'>\
             <div id=a style='height: 100px'></div>{}</div>{AFTER}",
            avoid(100)
        ),
        2,
        &[
            ("b", &[(0, 200.0, 60.0, 100.0)]),
            ("k", &[(1, 0.0, 0.0, 16.0)]),
        ],
    );
    check(
        "the last box in the first column of the next page",
        &format!(
            "<div style='{COLUMNS}'><div id=a style='height: 150px'></div>{}\
             <div id=c style='height: 100px; break-inside: avoid'></div></div>{AFTER}",
            avoid(150)
        ),
        2,
        &[
            ("c", &[(1, 0.0, 0.0, 100.0)]),
            ("k", &[(1, 0.0, 100.0, 16.0)]),
        ],
    );
    // The block-end padding and border of the container follow on the next
    // page.
    check(
        "block-end padding and border",
        &format!(
            "<div style='{COLUMNS}; padding-bottom: 10px; border-bottom: 4px solid'>\
             <div id=a style='height: 150px'></div>{}</div>{AFTER}",
            avoid(150)
        ),
        2,
        &[
            ("b", &[(0, 200.0, 0.0, 150.0)]),
            ("k", &[(1, 0.0, 14.0, 16.0)]),
        ],
    );
    check(
        "content that ends in the first column",
        &format!(
            "<div style='{COLUMNS}'><div id=a style='height: 150px'></div>{}</div>{AFTER}",
            avoid(30)
        ),
        2,
        &[
            ("b", &[(0, 0.0, 150.0, 30.0)]),
            ("k", &[(0, 0.0, 180.0, 12.0), (1, 0.0, 0.0, 4.0)]),
        ],
    );
}

/// Lines of a paragraph do the same: the lines that go on in the second
/// column of the last page break the columns there, so the box after the
/// container starts the next page.
#[test]
fn lines_that_go_on_in_the_second_column_of_the_last_page_reach_its_end() {
    let lines = vec!["x"; 30].join("<br>");
    check(
        "30 lines of 10px",
        &format!(
            "<div style='{COLUMNS}'><p id=p style='margin: 0; font: 10px/10px Ahem'>{lines}</p></div>{AFTER}"
        ),
        2,
        &[
            ("p", &[(0, 0.0, 0.0, 192.0), (0, 200.0, 0.0, 110.0)]),
            ("k", &[(1, 0.0, 0.0, 16.0)]),
        ],
    );
}

/// A container that clones its decorations has its block-axis padding and
/// border on each page it is cut on, so the columns of a page are that much
/// shorter, those of a later page start below the top ones, and the container
/// ends with its bottom ones after the content (CSS Fragmentation 3, 5.4).
#[test]
fn a_container_that_clones_its_decorations_keeps_them_on_each_page() {
    let red = "<div id=a style='height: 480px'></div>";
    for (label, edges) in [
        ("10px padding", "padding: 10px"),
        ("padding and border", "padding: 4px; border: 6px solid blue"),
    ] {
        check(
            label,
            &format!(
                "<div id=c style='{COLUMNS}; {edges}; box-decoration-break: clone'>{red}</div>{AFTER}"
            ),
            2,
            &[
                ("c", &[(0, 0.0, 0.0, 192.0), (1, 0.0, 0.0, 156.0)]),
                (
                    "a",
                    &[
                        (0, 10.0, 10.0, 172.0),
                        (0, 200.0, 10.0, 172.0),
                        (1, 10.0, 10.0, 136.0),
                    ],
                ),
                ("k", &[(1, 0.0, 156.0, 16.0)]),
            ],
        );
    }
    check(
        "3px top, 12px bottom",
        &format!(
            "<div id=c style='{COLUMNS}; padding: 3px 0 12px; box-decoration-break: clone'>\
             {red}</div>{AFTER}"
        ),
        2,
        &[
            ("c", &[(0, 0.0, 0.0, 192.0), (1, 0.0, 0.0, 141.0)]),
            (
                "a",
                &[
                    (0, 0.0, 3.0, 177.0),
                    (0, 200.0, 3.0, 177.0),
                    (1, 0.0, 3.0, 126.0),
                ],
            ),
            ("k", &[(1, 0.0, 141.0, 16.0)]),
        ],
    );
    // Without clone the padding is sliced: the columns of the first page reach
    // its end, and the second page has no top padding.
    check(
        "sliced",
        &format!("<div id=c style='{COLUMNS}; padding: 10px'>{red}</div>{AFTER}"),
        2,
        &[
            (
                "a",
                &[
                    (0, 10.0, 10.0, 182.0),
                    (0, 200.0, 10.0, 182.0),
                    (1, 10.0, 0.0, 116.0),
                ],
            ),
            ("k", &[(1, 0.0, 126.0, 16.0)]),
        ],
    );
}

/// The cloned edges shorten the columns of every page, and content that goes
/// on in the second column of a page ends the container there.
#[test]
fn cloned_decorations_shorten_the_columns_of_every_page() {
    let clone = format!("{COLUMNS}; padding: 10px; box-decoration-break: clone");
    check(
        "900px box",
        &format!("<div id=c style='{clone}'><div id=a style='height: 900px'></div></div>{AFTER}"),
        4,
        &[
            (
                "a",
                &[
                    (0, 10.0, 10.0, 172.0),
                    (0, 200.0, 10.0, 172.0),
                    (1, 10.0, 10.0, 172.0),
                    (1, 200.0, 10.0, 172.0),
                    (2, 10.0, 10.0, 172.0),
                    (2, 200.0, 10.0, 40.0),
                ],
            ),
            ("k", &[(3, 0.0, 0.0, 16.0)]),
        ],
    );
    check(
        "250px box",
        &format!("<div id=c style='{clone}'><div id=a style='height: 250px'></div></div>{AFTER}"),
        2,
        &[
            ("c", &[(0, 0.0, 0.0, 192.0)]),
            ("a", &[(0, 10.0, 10.0, 172.0), (0, 200.0, 10.0, 78.0)]),
            ("k", &[(1, 0.0, 0.0, 16.0)]),
        ],
    );
    // A container that starts partway down a page has what is left of it.
    check(
        "after 100px",
        &format!(
            "<div style='height: 100px'></div><div id=c style='{clone}'>\
             <div id=a style='height: 480px'></div></div>{AFTER}"
        ),
        3,
        &[
            (
                "a",
                &[
                    (0, 10.0, 110.0, 72.0),
                    (0, 200.0, 110.0, 72.0),
                    (1, 10.0, 10.0, 172.0),
                    (1, 200.0, 10.0, 164.0),
                ],
            ),
            ("k", &[(2, 0.0, 0.0, 16.0)]),
        ],
    );
}

/// The boxes inside a wrapper and the lines of a paragraph fill the shorter
/// columns too.
#[test]
fn wrapped_boxes_and_lines_fill_the_columns_a_clone_leaves() {
    let clone = format!("{COLUMNS}; padding: 10px; box-decoration-break: clone");
    check(
        "padded wrapper",
        &format!(
            "<div id=c style='{clone}'><div style='padding: 5px'>\
             <div id=a style='height: 500px'></div></div></div>{AFTER}"
        ),
        3,
        &[
            (
                "a",
                &[
                    (0, 15.0, 15.0, 167.0),
                    (0, 205.0, 10.0, 172.0),
                    (1, 15.0, 10.0, 161.0),
                ],
            ),
            ("c", &[(0, 0.0, 0.0, 192.0), (1, 0.0, 0.0, 186.0)]),
            ("k", &[(1, 0.0, 186.0, 6.0), (2, 0.0, 0.0, 10.0)]),
        ],
    );
    // Thirty one-word lines: eight in each of the three columns of the first
    // page, and six on the second below its top padding.
    let words = vec!["XXXX"; 30].join(" ");
    check(
        "paragraph",
        &format!(
            "<div id=c style='columns: 3; column-gap: 0; column-fill: auto; padding: 10px; \
             box-decoration-break: clone; font: 20px/20px Ahem'><p id=p style='margin: 0'>\
             {words}</p></div>{AFTER}"
        ),
        2,
        &[
            ("c", &[(0, 0.0, 0.0, 192.0), (1, 0.0, 0.0, 140.0)]),
            ("k", &[(1, 0.0, 140.0, 16.0)]),
        ],
    );
}

/// Cloned edges as tall as the page leave no room for columns; the layout
/// still completes, with the content going on over the pages.
#[test]
fn cloned_edges_as_tall_as_the_page_still_lay_out() {
    let (pages, pieces) = pieces_of(&format!(
        "<div id=c style='{COLUMNS}; padding: 96px 0; box-decoration-break: clone'>\
         <div id=a style='height: 480px'></div></div>{AFTER}"
    ));
    assert!(pages > 1, "{pieces:?}");
    assert!(pieces.iter().any(|(id, own)| id == "a" && !own.is_empty()));
}

/// A cloned container whose content starts near the end of a page goes on
/// below the repeated top edges of the next page.
#[test]
fn cloned_columns_that_start_late_go_on_below_the_next_top_edges() {
    let clone = format!("{COLUMNS}; padding: 10px; box-decoration-break: clone");
    let body = |before: u32| {
        format!(
            "<div style='height: {before}px'></div><div id=c style='{clone}'>\
             <div id=a style='height: 300px'></div></div>{AFTER}"
        )
    };
    check(
        "after 150px",
        &body(150),
        3,
        &[
            (
                "a",
                &[
                    (0, 10.0, 160.0, 22.0),
                    (0, 200.0, 160.0, 22.0),
                    (1, 10.0, 10.0, 172.0),
                    (1, 200.0, 10.0, 84.0),
                ],
            ),
            ("k", &[(2, 0.0, 0.0, 16.0)]),
        ],
    );
    // The content starts inside the bottom padding of the first page, where
    // it keeps the rest of that page; the next page has its top padding.
    let (pages, pieces) = pieces_of(&body(175));
    assert_eq!(pages, 3, "{pieces:?}");
    let (_, a) = pieces.iter().find(|(id, _)| id == "a").expect("#a");
    assert_eq!(
        a.iter().filter(|piece| piece.0 == 1).collect::<Vec<_>>(),
        [&(1, 10.0, 10.0, 172.0), &(1, 200.0, 10.0, 114.0)],
        "{a:?}"
    );
}

/// The boxes after the last spanner of a container that fills its columns in
/// turn fill the columns of each page they reach, as those of a container
/// without spanners do; the rows before a spanner stay balanced.
#[test]
fn columns_after_the_last_spanner_fill_in_turn_on_each_page() {
    let columns = "columns: 2; column-gap: 0; column-fill: auto";
    let before =
        "<div style='height: 200px'></div><div style='column-span: all; height: 30px'></div>";
    let body = |after: &str| format!("<div id=c style='{columns}'>{before}{after}</div>{AFTER}");
    check(
        "tall box",
        &body("<div id=j style='height: 300px'></div>"),
        2,
        &[
            (
                "j",
                &[
                    (0, 0.0, 130.0, 62.0),
                    (0, 192.0, 130.0, 62.0),
                    (1, 0.0, 0.0, 176.0),
                ],
            ),
            ("k", &[(1, 0.0, 176.0, 16.0)]),
        ],
    );
    check(
        "two boxes",
        &body("<div id=j style='height: 150px'></div><div id=m style='height: 200px'></div>"),
        3,
        &[
            (
                "j",
                &[
                    (0, 0.0, 130.0, 62.0),
                    (0, 192.0, 130.0, 62.0),
                    (1, 0.0, 0.0, 26.0),
                ],
            ),
            ("m", &[(1, 0.0, 26.0, 166.0), (1, 192.0, 0.0, 34.0)]),
            ("k", &[(2, 0.0, 0.0, 16.0)]),
        ],
    );
    check(
        "second spanner",
        &body(
            "<div id=m style='height: 40px'></div>\
             <div id=s style='column-span: all; height: 10px'></div>\
             <div id=j style='height: 300px'></div>",
        ),
        3,
        &[
            ("m", &[(0, 0.0, 130.0, 20.0), (0, 192.0, 130.0, 20.0)]),
            ("s", &[(0, 0.0, 150.0, 10.0)]),
            (
                "j",
                &[
                    (0, 0.0, 160.0, 32.0),
                    (0, 192.0, 160.0, 32.0),
                    (1, 0.0, 0.0, 192.0),
                    (1, 192.0, 0.0, 44.0),
                ],
            ),
            ("k", &[(2, 0.0, 0.0, 16.0)]),
        ],
    );
    // A box that fits below the spanner fills the first column alone.
    check(
        "short box",
        &body("<div id=j style='height: 50px'></div>"),
        2,
        &[
            ("c", &[(0, 0.0, 0.0, 180.0)]),
            ("j", &[(0, 0.0, 130.0, 50.0)]),
        ],
    );
}
