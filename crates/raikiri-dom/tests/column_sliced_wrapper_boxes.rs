//! The boxes a wrapper holds inside a multicolumn container stay where the
//! columns put them when the wrapper slices its padding at a column end: each
//! piece of the wrapper paints the whole box moved up by what its earlier
//! pieces took, and the boxes inside it make up for that (CSS Fragmentation 3,
//! 5.4).
//!
//! The expected places are the ones Chromium gives the same markup when it
//! prints a 480px by 288px page with 48px margins: `columns: 2` leaves 184px
//! columns and a 16px gap.

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

/// A wrapper with 10px of padding above and below the boxes it holds.
fn padded(inner: &str) -> String {
    format!("<div id=w style='padding: 10px 0'>{inner}</div>")
}

/// A box the column end cuts goes on at the top of the next column, under the
/// piece of the wrapper there, whether the columns fill in turn or balance.
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
