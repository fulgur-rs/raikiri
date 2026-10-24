//! A table cell that spans rows asks them for its height once every cell
//! that sits in a single row has set the height of that row. What the spanned
//! rows lack is shared out the way browsers do it (CSS Tables 3 leaves the
//! split to the user agent): in proportion to the heights of the rows, the
//! rows with an authored height after the others, and equally among the rows
//! where another spanning cell starts.
//!
//! The expected rows are the ones Chromium computes for the same markup in an
//! 800px by 600px viewport, with the body margin of 8px: each is the row's
//! top and height.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The top and height of the elements of `body` that have an `id`, on the
/// first page.
fn boxes(body: &str) -> Vec<(String, f32, f32)> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px Ahem }} div {{ box-sizing: border-box }} \
         td {{ padding: 0 }} table {{ border-spacing: 0 }}</style>{body}"
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
            Some((id.to_owned(), rect.y, rect.height))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.04;

/// A label, the markup of a case and the `(id, top, height)` it expects.
type Case<'a> = (&'a str, &'a str, &'a [(&'a str, f32, f32)]);

/// Lays out each case and compares the rows it names with the expected top
/// and height.
fn check_rows(cases: &[Case<'_>]) {
    for &(label, body, expected) in cases {
        let actual = boxes(body);
        for &(id, top, height) in expected {
            let (_, y, h) = actual
                .iter()
                .find(|(name, ..)| name == id)
                .unwrap_or_else(|| panic!("{label}: no fragment for #{id}"));
            assert!(
                (y - top).abs() <= TOLERANCE && (h - height).abs() <= TOLERANCE,
                "{label}: #{id} is at {y} and {h} tall, expected {top} and {height}"
            );
        }
    }
}

/// A cell spanning rows that are shorter than it grows them in proportion to their heights, and the last row takes all of it when every spanned row is empty.
#[test]
fn a_spanning_cell_shares_its_missing_height_in_proportion_to_the_rows() {
    let cases: &[Case<'_>] = &[
        (
            "rows 10 and 10, cell 100",
            "<table><tr id=r1><td id=c rowspan=2><div style=height:100px></div></td><td id=a><div style=height:10px></div></td></tr><tr id=r2><td id=b><div style=height:10px></div></td></tr></table>",
            &[("r1", 8.0, 50.0), ("r2", 58.0, 50.0)],
        ),
        (
            "rows 10 and 30, cell 100",
            "<table><tr id=r1><td id=c rowspan=2><div style=height:100px></div></td><td id=a><div style=height:10px></div></td></tr><tr id=r2><td id=b><div style=height:30px></div></td></tr></table>",
            &[("r1", 8.0, 25.0), ("r2", 33.0, 75.0)],
        ),
        (
            "rows 30 and 10, cell 100",
            "<table><tr id=r1><td id=c rowspan=2><div style=height:100px></div></td><td id=a><div style=height:30px></div></td></tr><tr id=r2><td id=b><div style=height:10px></div></td></tr></table>",
            &[("r1", 8.0, 75.0), ("r2", 83.0, 25.0)],
        ),
        (
            "rows 40 and 0, cell 100",
            "<table><tr id=r1><td id=c rowspan=2><div style=height:100px></div></td><td id=a><div style=height:40px></div></td></tr><tr id=r2><td id=b><div style=height:0px></div></td></tr></table>",
            &[("r1", 8.0, 100.0), ("r2", 108.0, 0.0)],
        ),
        (
            "rows 0 and 40, cell 100",
            "<table><tr id=r1><td id=c rowspan=2><div style=height:100px></div></td><td id=a><div style=height:0px></div></td></tr><tr id=r2><td id=b><div style=height:40px></div></td></tr></table>",
            &[("r1", 8.0, 0.0), ("r2", 8.0, 100.0)],
        ),
        (
            "rows 0 and 0, cell 100",
            "<table><tr id=r1><td id=c rowspan=2><div style=height:100px></div></td><td id=a><div style=height:0px></div></td></tr><tr id=r2><td id=b><div style=height:0px></div></td></tr></table>",
            &[("r1", 8.0, 0.0), ("r2", 8.0, 100.0)],
        ),
        (
            "three rows 10, 30 and 0, cell 120",
            "<table><tr id=r1><td id=c rowspan=3><div style=height:120px></div></td><td id=a><div style=height:10px></div></td></tr><tr id=r2><td id=b><div style=height:30px></div></td></tr><tr id=r3><td id=e><div style=height:0px></div></td></tr></table>",
            &[("r1", 8.0, 30.0), ("r2", 38.0, 90.0), ("r3", 128.0, 0.0)],
        ),
        (
            "an empty row in the middle",
            "<table><tr id=r1><td id=c rowspan=3><div style=height:100px></div></td><td id=a><div style=height:10px></div></td></tr><tr id=r2></tr><tr id=r3><td id=e><div style=height:30px></div></td></tr></table>",
            &[("r1", 8.0, 25.0), ("r2", 33.0, 0.0), ("r3", 33.0, 75.0)],
        ),
        (
            "a span past the last row",
            "<table><tr id=r1><td id=c rowspan=5><div style=height:100px></div></td><td id=a><div style=height:10px></div></td></tr><tr id=r2><td id=b><div style=height:30px></div></td></tr></table>",
            &[("r1", 8.0, 25.0), ("r2", 33.0, 75.0)],
        ),
    ];
    check_rows(cases);
}

/// A spanning cell stretches the rows without an authored height first. The rows that have one, on the row or on a cell in it, share the height only when there is no such row with any height.
#[test]
fn rows_with_an_authored_height_take_a_share_after_the_rows_without_one() {
    let cases: &[Case<'_>] = &[
        (
            "row 1 has a height",
            "<table><tr id=r1 style=\"height:30px\"><td id=c rowspan=3><div style=height:150px></div></td><td id=a><div style=height:10px></div></td></tr><tr id=r2><td id=b><div style=height:20px></div></td></tr><tr id=r3><td id=e><div style=height:40px></div></td></tr></table>",
            &[("r1", 8.0, 30.0), ("r2", 38.0, 40.0), ("r3", 78.0, 80.0)],
        ),
        (
            "row 2 has a height",
            "<table><tr id=r1><td id=c rowspan=3><div style=height:150px></div></td><td id=a><div style=height:10px></div></td></tr><tr id=r2 style=\"height:30px\"><td id=b><div style=height:20px></div></td></tr><tr id=r3><td id=e><div style=height:40px></div></td></tr></table>",
            &[("r1", 8.0, 24.0), ("r2", 32.0, 30.0), ("r3", 62.0, 96.0)],
        ),
        (
            "rows 1 and 3 have a height",
            "<table><tr id=r1 style=\"height:30px\"><td id=c rowspan=3><div style=height:150px></div></td><td id=a><div style=height:10px></div></td></tr><tr id=r2><td id=b><div style=height:20px></div></td></tr><tr id=r3 style=\"height:30px\"><td id=e><div style=height:40px></div></td></tr></table>",
            &[("r1", 8.0, 30.0), ("r2", 38.0, 80.0), ("r3", 118.0, 40.0)],
        ),
        (
            "rows 1 and 2 have a height",
            "<table><tr id=r1 style=\"height:30px\"><td id=c rowspan=3><div style=height:150px></div></td><td id=a><div style=height:10px></div></td></tr><tr id=r2 style=\"height:30px\"><td id=b><div style=height:20px></div></td></tr><tr id=r3><td id=e><div style=height:40px></div></td></tr></table>",
            &[("r1", 8.0, 30.0), ("r2", 38.0, 30.0), ("r3", 68.0, 90.0)],
        ),
        (
            "every row has a height",
            "<table><tr id=r1 style=\"height:20px\"><td id=c rowspan=2><div style=height:100px></div></td><td id=a><div style=height:0px></div></td></tr><tr id=r2 style=\"height:30px\"><td id=b><div style=height:0px></div></td></tr></table>",
            &[("r1", 8.0, 40.0), ("r2", 48.0, 60.0)],
        ),
        (
            "a fixed row beside an empty row",
            "<table><tr id=r1 style=\"height:30px\"><td id=c rowspan=2><div style=height:100px></div></td><td id=a><div style=height:0px></div></td></tr><tr id=r2><td id=b><div style=height:0px></div></td></tr></table>",
            &[("r1", 8.0, 100.0), ("r2", 108.0, 0.0)],
        ),
        (
            "a cell with a height fixes its row",
            "<table><tr id=r1><td id=c rowspan=2><div style=height:100px></div></td><td id=a style=\"height:20px\"></td></tr><tr id=r2><td id=b><div style=height:40px></div></td></tr></table>",
            &[("r1", 8.0, 20.0), ("r2", 28.0, 80.0)],
        ),
        (
            "a cell with a height in each row",
            "<table><tr id=r1><td id=c rowspan=2><div style=height:100px></div></td><td id=a style=\"height:20px\"></td></tr><tr id=r2><td id=b style=\"height:40px\"></td></tr></table>",
            &[("r1", 8.0, 33.33), ("r2", 41.33, 66.67)],
        ),
    ];
    check_rows(cases);
}

/// Every single-row cell sets its row before any spanning cell asks for more, wherever it is in the source.
#[test]
fn single_row_cells_are_counted_before_a_spanning_cell_shares_out_its_height() {
    let cases: &[Case<'_>] = &[
        (
            "the spanning cell comes first in the source",
            "<table><tr id=r1><td id=c rowspan=2><div style=height:100px></div></td><td id=a><div style=height:1px></div></td></tr><tr id=r2><td id=b><div style=height:99px></div></td></tr></table>",
            &[("r1", 8.0, 1.0), ("r2", 9.0, 99.0)],
        ),
        (
            "the same cell after the others",
            "<table><tr id=r1><td id=a><div style=height:1px></div></td><td id=c rowspan=2><div style=height:100px></div></td></tr><tr id=r2><td id=b><div style=height:99px></div></td></tr></table>",
            &[("r1", 8.0, 1.0), ("r2", 9.0, 99.0)],
        ),
        (
            "a taller cell below",
            "<table><tr id=r1><td id=c rowspan=2><div style=height:60px></div></td><td id=a><div style=height:10px></div></td></tr><tr id=r2><td id=b><div style=height:70px></div></td></tr></table>",
            &[("r1", 8.0, 10.0), ("r2", 18.0, 70.0)],
        ),
    ];
    check_rows(cases);
}

/// When another spanning cell starts in a later row of the span, the rows where such cells start share the missing height equally.
#[test]
fn a_span_starting_in_a_later_row_takes_the_missing_height_of_the_spans_over_it() {
    let cases: &[Case<'_>] = &[
        (
            "a span starts in the second row",
            "<table><tr id=r1><td id=c rowspan=2><div style=height:60px></div></td><td id=a><div style=height:10px></div></td></tr><tr id=r2><td id=b rowspan=2><div style=height:60px></div></td></tr><tr id=r3><td id=e><div style=height:10px></div></td></tr></table>",
            &[("r1", 8.0, 10.0), ("r2", 18.0, 50.0), ("r3", 68.0, 10.0)],
        ),
        (
            "the same with the second span taller",
            "<table><tr id=r1><td id=c rowspan=2><div style=height:60px></div></td><td id=a><div style=height:10px></div></td></tr><tr id=r2><td id=b rowspan=2><div style=height:100px></div></td></tr><tr id=r3><td id=e><div style=height:10px></div></td></tr></table>",
            &[
                ("r1", 8.0, 10.0),
                ("r2", 18.0, 83.33),
                ("r3", 101.33, 16.67),
            ],
        ),
        (
            "the same with the first row taller",
            "<table><tr id=r1><td id=c rowspan=2><div style=height:60px></div></td><td id=a><div style=height:30px></div></td></tr><tr id=r2><td id=b rowspan=2><div style=height:60px></div></td></tr><tr id=r3><td id=e><div style=height:10px></div></td></tr></table>",
            &[("r1", 8.0, 30.0), ("r2", 38.0, 45.0), ("r3", 83.0, 15.0)],
        ),
        (
            "a shorter span inside a longer one",
            "<table><tr id=r1><td id=c rowspan=3><div style=height:100px></div></td><td id=a><div style=height:10px></div></td></tr><tr id=r2><td id=b rowspan=2><div style=height:20px></div></td></tr><tr id=r3><td id=e><div style=height:10px></div></td></tr><tr id=r4><td id=f><div style=height:10px></div></td></tr></table>",
            &[
                ("r1", 8.0, 10.0),
                ("r2", 18.0, 70.0),
                ("r3", 88.0, 20.0),
                ("r4", 108.0, 10.0),
            ],
        ),
        (
            "two spans that end in the same row",
            "<table><tr id=r1><td id=c rowspan=3><div style=height:100px></div></td><td id=a><div style=height:10px></div></td></tr><tr id=r2><td id=b rowspan=2><div style=height:60px></div></td></tr><tr id=r3><td id=e><div style=height:10px></div></td></tr></table>",
            &[("r1", 8.0, 10.0), ("r2", 18.0, 30.0), ("r3", 48.0, 60.0)],
        ),
        (
            "spans that start in the first row",
            "<table><tr id=r1><td id=c rowspan=2><div style=height:60px></div></td><td id=a rowspan=3><div style=height:80px></div></td><td id=z><div style=height:0px></div></td></tr><tr id=r2><td id=b><div style=height:10px></div></td></tr><tr id=r3><td id=e><div style=height:10px></div></td></tr></table>",
            &[("r1", 8.0, 0.0), ("r2", 8.0, 68.56), ("r3", 76.56, 11.44)],
        ),
        (
            "two starts in one row",
            "<table><tr id=r1><td id=c rowspan=3><div style=height:150px></div></td><td id=a><div style=height:10px></div></td></tr><tr id=r2><td id=b rowspan=2><div style=height:20px></div></td><td id=g rowspan=2><div style=height:20px></div></td></tr><tr id=r3></tr><tr id=r4><td id=e><div style=height:10px></div></td></tr></table>",
            &[
                ("r1", 8.0, 10.0),
                ("r2", 18.0, 120.0),
                ("r3", 138.0, 20.0),
                ("r4", 158.0, 10.0),
            ],
        ),
    ];
    check_rows(cases);
}

/// Border spacing between the spanned rows counts toward the cell, a collapsed row shares in the sizing before it disappears, and the baseline floors the first row before anything is shared.
#[test]
fn a_span_counts_the_spacing_the_collapsed_rows_and_the_baseline() {
    let cases: &[Case<'_>] = &[
        (
            "border spacing counts toward the span",
            "<table style=\"border-spacing:4px\"><tr id=r1><td id=c rowspan=2><div style=height:100px></div></td><td id=a><div style=height:10px></div></td></tr><tr id=r2><td id=b><div style=height:30px></div></td></tr></table>",
            &[("r1", 12.0, 24.0), ("r2", 40.0, 72.0)],
        ),
        (
            "three rows with border spacing",
            "<table style=\"border-spacing:4px\"><tr id=r1><td id=c rowspan=3><div style=height:100px></div></td><td id=a><div style=height:10px></div></td></tr><tr id=r2><td id=b><div style=height:30px></div></td></tr><tr id=r3><td id=e><div style=height:0px></div></td></tr></table>",
            &[("r1", 12.0, 23.0), ("r2", 39.0, 69.0), ("r3", 112.0, 0.0)],
        ),
        (
            "a collapsed row takes a share and then none of the height",
            "<table><tr id=r1 style=\"height:5px\"><td id=c rowspan=2><div style=height:60px></div></td></tr><tr id=r2 style=\"height:5px;visibility:collapse\"><td id=b><div style=height:0px></div></td></tr></table>",
            &[("r1", 8.0, 30.0), ("r2", 38.0, 0.0)],
        ),
        (
            "a baseline row",
            "<table><tr id=r1><td id=c rowspan=2 style=\"vertical-align:baseline;font:40px/40px Ahem;width:40px\">X</td></tr><tr id=r2></tr></table>",
            &[("r1", 8.0, 40.0), ("r2", 48.0, 0.0)],
        ),
        (
            "a baseline span past the last row",
            "<table><tr id=r1><td id=a><div style=height:10px></div></td></tr><tr id=r2><td id=c rowspan=5 style=\"vertical-align:baseline;font:40px/40px Ahem\">X</td><td id=d style=\"vertical-align:baseline;font:10px/10px Ahem\">Y</td></tr></table>",
            &[("r1", 8.0, 10.0), ("r2", 18.0, 40.0)],
        ),
        (
            "a baseline span with a height past the last row",
            "<table><tr id=r1><td id=a><div style=height:10px></div></td></tr><tr id=r2><td id=c rowspan=5 style=\"vertical-align:baseline;font:40px/40px Ahem;height:10px\">X</td><td id=d style=\"vertical-align:baseline;font:10px/10px Ahem\">Y</td><td id=e style=\"vertical-align:baseline;font:20px/20px Ahem\">Z</td></tr></table>",
            &[("r1", 8.0, 10.0), ("r2", 18.0, 40.0)],
        ),
        (
            "a baseline and a shorter row",
            "<table><tr id=r1><td id=c rowspan=2 style=\"vertical-align:baseline;font:40px/40px Ahem;width:40px\">X</td><td id=a><div style=height:10px></div></td></tr><tr id=r2><td id=b><div style=height:30px></div></td></tr></table>",
            &[("r1", 8.0, 32.0), ("r2", 40.0, 30.0)],
        ),
    ];
    check_rows(cases);
}
