//! A table taller than its rows (a `height` or `min-height`, or a height a
//! parent imposes) gives the extra to the rows the way browsers do (CSS 2.1
//! 17.5.3 leaves the split to the user agent): the rows with no authored
//! height take it, in proportion to their heights, and a row whose own
//! `height`, or whose single-row cell's `height`, is a length keeps that
//! height. Only when every shown row has an authored height do all of them
//! share the extra, again in proportion.
//!
//! The expected boxes are the ones Chromium computes for the same markup in an
//! 800px by 600px viewport, with the body margin of 8px: each is the element's
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

/// The extra height of a table goes to the rows that have no authored height, in proportion to their heights; a row with its own `height`, or whose single-row cell has one, keeps it.
#[test]
fn a_table_height_goes_to_the_rows_without_an_authored_height() {
    let cases: &[Case<'_>] = &[
        (
            "row 2 has a height, row 1 is 20px",
            "<table id=t style='height:200px'><tr id=r0><td id=c0><div style='height:20px'></div></td></tr><tr id=r1 style='height:40px'><td id=c1><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 160.0), ("r1", 168.0, 40.0)],
        ),
        (
            "row 1 has a height, row 2 is 10px",
            "<table id=t style='height:200px'><tr id=r0 style='height:40px'><td id=c0><div style='height:20px'></div></td></tr><tr id=r1><td id=c1><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 40.0), ("r1", 48.0, 160.0)],
        ),
        (
            "a cell of row 2 has a height",
            "<table id=t style='height:200px'><tr id=r0><td id=c0><div style='height:20px'></div></td></tr><tr id=r1><td id=c1 style='height:40px'><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 160.0), ("r1", 168.0, 40.0)],
        ),
        (
            "rows 20px, 40px (authored) and 10px",
            "<table id=t style='height:200px'><tr id=r0><td id=c0><div style='height:20px'></div></td></tr><tr id=r1 style='height:40px'><td id=c1><div style='height:10px'></div></td></tr><tr id=r2><td id=c2><div style='height:10px'></div></td></tr></table>",
            &[
                ("t", 8.0, 200.0),
                ("r0", 8.0, 106.66),
                ("r1", 114.66, 40.0),
                ("r2", 154.66, 53.34),
            ],
        ),
        (
            "a row height and a cell height around a free row",
            "<table id=t style='height:200px'><tr id=r0 style='height:30px'><td id=c0><div style='height:20px'></div></td></tr><tr id=r1><td id=c1><div style='height:10px'></div></td></tr><tr id=r2><td id=c2 style='height:30px'><div style='height:30px'></div></td></tr></table>",
            &[
                ("t", 8.0, 200.0),
                ("r0", 8.0, 30.0),
                ("r1", 38.0, 140.0),
                ("r2", 178.0, 30.0),
            ],
        ),
        (
            "the row and its cell both have heights",
            "<table id=t style='height:200px'><tr id=r0 style='height:30px'><td id=c0 style='height:50px'><div style='height:20px'></div></td></tr><tr id=r1><td id=c1><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 50.0), ("r1", 58.0, 150.0)],
        ),
        (
            "a cell with a height shares its row with a cell without",
            "<table id=t style='height:200px'><tr id=r0><td id=c0 style='height:40px'><div style='height:10px'></div></td><td id=e0><div style='height:20px'></div></td></tr><tr id=r1><td id=c1><div style='height:10px'></div></td><td id=e1><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 40.0), ("r1", 48.0, 160.0)],
        ),
        (
            "a spanning cell does not fix the rows it spans",
            "<table id=t style='height:200px'><tr id=r0><td id=c0 rowspan=2><div style='height:60px'></div></td><td id=c1><div style='height:10px'></div></td></tr><tr id=r1 style='height:40px'><td id=c2><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 160.0), ("r1", 168.0, 40.0)],
        ),
        (
            "border-spacing stays out of the rows",
            "<table id=t style='height:200px;border-spacing:4px'><tr id=r0><td id=c0><div style='height:20px'></div></td></tr><tr id=r1 style='height:40px'><td id=c1><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 12.0, 148.0), ("r1", 164.0, 40.0)],
        ),
    ];
    check_rows(cases);
}

/// When every row has an authored height, all of them share the extra in proportion to their heights, as do rows without one.
#[test]
fn rows_that_all_have_an_authored_height_share_the_extra_in_proportion() {
    let cases: &[Case<'_>] = &[
        (
            "two rows with a row height",
            "<table id=t style='height:200px'><tr id=r0 style='height:40px'><td id=c0><div style='height:10px'></div></td></tr><tr id=r1 style='height:20px'><td id=c1><div style='height:10px'></div></td></tr></table>",
            &[
                ("t", 8.0, 200.0),
                ("r0", 8.0, 133.33),
                ("r1", 141.33, 66.67),
            ],
        ),
        (
            "two rows with a cell height",
            "<table id=t style='height:200px'><tr id=r0><td id=c0 style='height:40px'><div style='height:10px'></div></td></tr><tr id=r1><td id=c1 style='height:20px'><div style='height:10px'></div></td></tr></table>",
            &[
                ("t", 8.0, 200.0),
                ("r0", 8.0, 133.33),
                ("r1", 141.33, 66.67),
            ],
        ),
        (
            "no authored height",
            "<table id=t style='height:200px'><tr id=r0><td id=c0><div style='height:20px'></div></td></tr><tr id=r1><td id=c1><div style='height:10px'></div></td></tr></table>",
            &[
                ("t", 8.0, 200.0),
                ("r0", 8.0, 133.33),
                ("r1", 141.33, 66.67),
            ],
        ),
        (
            "min-height with two row heights",
            "<table id=t style='min-height:200px'><tr id=r0 style='height:30px'><td id=c0><div style='height:20px'></div></td></tr><tr id=r1 style='height:40px'><td id=c1><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 85.7), ("r1", 93.7, 114.3)],
        ),
        (
            "a table height below the rows changes nothing",
            "<table id=t style='height:30px'><tr id=r0><td id=c0><div style='height:20px'></div></td></tr><tr id=r1 style='height:40px'><td id=c1><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 60.0), ("r0", 8.0, 20.0), ("r1", 28.0, 40.0)],
        ),
    ];
    check_rows(cases);
}

/// A row with no content and no authored height still takes the extra, equally with the others like it.
#[test]
fn empty_rows_without_a_height_share_the_extra_equally() {
    let cases: &[Case<'_>] = &[
        (
            "an empty row beside an authored one",
            "<table id=t style='height:200px'><tr id=r0><td id=c0><div style='height:0px'></div></td></tr><tr id=r1 style='height:40px'><td id=c1><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 160.0), ("r1", 168.0, 40.0)],
        ),
        (
            "two empty rows beside an authored one",
            "<table id=t style='height:200px'><tr id=r0><td id=c0><div style='height:0px'></div></td></tr><tr id=r1><td id=c1><div style='height:0px'></div></td></tr><tr id=r2 style='height:40px'><td id=c2><div style='height:10px'></div></td></tr></table>",
            &[
                ("t", 8.0, 200.0),
                ("r0", 8.0, 80.0),
                ("r1", 88.0, 80.0),
                ("r2", 168.0, 40.0),
            ],
        ),
        (
            "an empty row beside one with content",
            "<table id=t style='height:200px'><tr id=r0><td id=c0><div style='height:0px'></div></td></tr><tr id=r1><td id=c1><div style='height:20px'></div></td></tr><tr id=r2 style='height:40px'><td id=c2><div style='height:10px'></div></td></tr></table>",
            &[
                ("t", 8.0, 200.0),
                ("r0", 8.0, 0.0),
                ("r1", 8.0, 160.0),
                ("r2", 168.0, 40.0),
            ],
        ),
        (
            "only empty rows",
            "<table id=t style='height:200px'><tr id=r0><td id=c0><div style='height:0px'></div></td></tr><tr id=r1><td id=c1><div style='height:0px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 100.0), ("r1", 108.0, 100.0)],
        ),
        (
            "an empty authored row beside an empty free one",
            "<table id=t style='height:200px'><tr id=r0 style='height:0px'><td id=c0><div style='height:0px'></div></td></tr><tr id=r1><td id=c1><div style='height:0px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 0.0), ("r1", 8.0, 200.0)],
        ),
    ];
    check_rows(cases);
}

/// The extra height that `min-height` asks for is shared the same way.
#[test]
fn min_height_shares_the_extra_like_a_height() {
    let cases: &[Case<'_>] = &[(
        "min-height 200px, row 2 has a height",
        "<table id=t style='min-height:200px'><tr id=r0><td id=c0><div style='height:20px'></div></td></tr><tr id=r1 style='height:40px'><td id=c1><div style='height:10px'></div></td></tr></table>",
        &[("t", 8.0, 200.0), ("r0", 8.0, 160.0), ("r1", 168.0, 40.0)],
    )];
    check_rows(cases);
}

/// Rows removed by `visibility: collapse` take no part. The expected boxes are Chromium's for the same markup with `display: none` rows: CSS Tables 3 removes a collapsed row the same way, though Chromium shares the extra with the collapsed row first and then shrinks the table by it.
#[test]
fn collapsed_rows_take_no_part_in_the_sharing() {
    let cases: &[Case<'_>] = &[
        (
            "the last row is collapsed",
            "<table id=t style='height:200px'><tr id=r0><td id=c0><div style='height:20px'></div></td></tr><tr id=r1 style='height:40px'><td id=c1><div style='height:10px'></div></td></tr><tr id=r2 style='visibility:collapse'><td id=c2><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 160.0), ("r1", 168.0, 40.0)],
        ),
        (
            "the middle row is collapsed",
            "<table id=t style='height:200px'><tr id=r0><td id=c0><div style='height:20px'></div></td></tr><tr id=r1 style='visibility:collapse'><td id=c1><div style='height:10px'></div></td></tr><tr id=r2 style='height:40px'><td id=c2><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 160.0), ("r2", 168.0, 40.0)],
        ),
        (
            "a collapsed row with an authored height",
            "<table id=t style='height:200px'><tr id=r0><td id=c0><div style='height:20px'></div></td></tr><tr id=r1 style='height:40px;visibility:collapse'><td id=c1><div style='height:10px'></div></td></tr><tr id=r2><td id=c2><div style='height:10px'></div></td></tr></table>",
            &[
                ("t", 8.0, 200.0),
                ("r0", 8.0, 133.33),
                ("r2", 141.33, 66.67),
            ],
        ),
        (
            "a collapsed row beside two free rows",
            "<table id=t style='height:200px'><tr id=r0><td id=c0><div style='height:20px'></div></td></tr><tr id=r1 style='visibility:collapse'><td id=c1><div style='height:10px'></div></td></tr><tr id=r2><td id=c2><div style='height:10px'></div></td></tr></table>",
            &[
                ("t", 8.0, 200.0),
                ("r0", 8.0, 133.33),
                ("r2", 141.33, 66.67),
            ],
        ),
    ];
    check_rows(cases);
}
