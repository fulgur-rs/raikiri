//! A table taller than its rows (a `height` or `min-height`, or a height a
//! parent imposes) gives the extra to the rows the way browsers do (CSS 2.1
//! 17.5.3 leaves the split to the user agent): the rows with no authored
//! height take it, in proportion to their heights, and a row whose own
//! `height`, or whose single-row cell's `height`, is a length keeps that
//! height. Only when every shown row has an authored height do all of them
//! share the extra, again in proportion. A row with a percentage `height`
//! takes that share of the height of the rows' group first, when the table's
//! own height is authored. A table with several row groups shares the extra
//! among the groups first, in proportion to their heights, except that the
//! header group on top and the footer group at the bottom keep theirs while a
//! body group can grow; each group then shares what it got among its rows.
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

#[test]
fn cell_min_height_does_not_floor_the_row() {
    check_rows(&[
        (
            "cell minimum with a percentage-height child",
            "<div id=t style='display:table'><div id=r style='display:table-row'><div id=c style='display:table-cell;min-height:40px'><div style='height:50%'>x</div></div></div></div>",
            &[("t", 8.0, 10.0), ("r", 8.0, 10.0), ("c", 8.0, 10.0)],
        ),
        (
            "cell height still floors the row",
            "<table id=t><tr id=r><td id=c style='height:30px;min-height:80px'>x</td></tr></table>",
            &[("t", 8.0, 30.0), ("r", 8.0, 30.0), ("c", 8.0, 30.0)],
        ),
        (
            "row height still floors the row",
            "<table id=t><tr id=r style='height:30px'><td id=c style='min-height:80px'>x</td></tr></table>",
            &[("t", 8.0, 30.0), ("r", 8.0, 30.0), ("c", 8.0, 30.0)],
        ),
        (
            "table minimum still floors the grid",
            "<table id=t style='min-height:30px'><tr id=r><td id=c style='min-height:80px'>x</td></tr></table>",
            &[("t", 8.0, 30.0), ("r", 8.0, 30.0), ("c", 8.0, 30.0)],
        ),
    ]);
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

/// Rows removed by `display: none` take no part.
#[test]
fn removed_rows_take_no_part_in_the_sharing() {
    let cases: &[Case<'_>] = &[
        (
            "the last row is removed",
            "<table id=t style='height:200px'><tr id=r0><td id=c0><div style='height:20px'></div></td></tr><tr id=r1 style='height:40px'><td id=c1><div style='height:10px'></div></td></tr><tr id=r2 style='display:none'><td id=c2><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 160.0), ("r1", 168.0, 40.0)],
        ),
        (
            "the middle row is removed",
            "<table id=t style='height:200px'><tr id=r0><td id=c0><div style='height:20px'></div></td></tr><tr id=r1 style='display:none'><td id=c1><div style='height:10px'></div></td></tr><tr id=r2 style='height:40px'><td id=c2><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 160.0), ("r2", 168.0, 40.0)],
        ),
        (
            "a removed row with an authored height",
            "<table id=t style='height:200px'><tr id=r0><td id=c0><div style='height:20px'></div></td></tr><tr id=r1 style='height:40px;display:none'><td id=c1><div style='height:10px'></div></td></tr><tr id=r2><td id=c2><div style='height:10px'></div></td></tr></table>",
            &[
                ("t", 8.0, 200.0),
                ("r0", 8.0, 133.33),
                ("r2", 141.33, 66.67),
            ],
        ),
        (
            "a removed row beside two free rows",
            "<table id=t style='height:200px'><tr id=r0><td id=c0><div style='height:20px'></div></td></tr><tr id=r1 style='display:none'><td id=c1><div style='height:10px'></div></td></tr><tr id=r2><td id=c2><div style='height:10px'></div></td></tr></table>",
            &[
                ("t", 8.0, 200.0),
                ("r0", 8.0, 133.33),
                ("r2", 141.33, 66.67),
            ],
        ),
    ];
    check_rows(cases);
}

/// Rows removed by `visibility: collapse` take their share first, as the rows
/// that stay do, and give it up: the table ends up lower than the height it
/// states, as much lower as the collapsed rows took.
#[test]
fn collapsed_rows_share_and_then_leave() {
    let cases: &[Case<'_>] = &[
        (
            "the last row is collapsed",
            "<table id=t style='height:200px'><tr id=r0><td id=c0><div style='height:20px'></div></td></tr><tr id=r1 style='height:40px'><td id=c1><div style='height:10px'></div></td></tr><tr id=r2 style='visibility:collapse'><td id=c2><div style='height:10px'></div></td></tr></table>",
            &[
                ("t", 8.0, 146.66),
                ("r0", 8.0, 106.66),
                ("r1", 114.66, 40.0),
                ("r2", 154.66, 0.0),
            ],
        ),
        (
            "the middle row is collapsed",
            "<table id=t style='height:200px'><tr id=r0><td id=c0><div style='height:20px'></div></td></tr><tr id=r1 style='visibility:collapse'><td id=c1><div style='height:10px'></div></td></tr><tr id=r2 style='height:40px'><td id=c2><div style='height:10px'></div></td></tr></table>",
            &[
                ("t", 8.0, 146.66),
                ("r0", 8.0, 106.66),
                ("r1", 114.66, 0.0),
                ("r2", 114.66, 40.0),
            ],
        ),
        (
            "a collapsed row with an authored height",
            "<table id=t style='height:200px'><tr id=r0><td id=c0><div style='height:20px'></div></td></tr><tr id=r1 style='height:40px;visibility:collapse'><td id=c1><div style='height:10px'></div></td></tr><tr id=r2><td id=c2><div style='height:10px'></div></td></tr></table>",
            &[
                ("t", 8.0, 160.0),
                ("r0", 8.0, 106.66),
                ("r2", 114.66, 53.34),
            ],
        ),
        (
            "a collapsed row beside two free rows",
            "<table id=t style='height:200px'><tr id=r0><td id=c0><div style='height:20px'></div></td></tr><tr id=r1 style='visibility:collapse'><td id=c1><div style='height:10px'></div></td></tr><tr id=r2><td id=c2><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 150.0), ("r0", 8.0, 100.0), ("r2", 108.0, 50.0)],
        ),
    ];
    check_rows(cases);
}

/// A row with a percentage `height` takes that share of the rows' height, and the rows without an authored height take the rest.
#[test]
fn percentage_rows_take_their_share_of_the_table_height() {
    let cases: &[Case<'_>] = &[
        (
            "a 50% row after a free one",
            "<table id='t' style='height:200px'><tr id='r0'><td><div style='height:20px'></div></td></tr><tr id='r1' style='height:50%'><td><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 100.0), ("r1", 108.0, 100.0)],
        ),
        (
            "a 50% row before a free one",
            "<table id='t' style='height:200px'><tr id='r0' style='height:50%'><td><div style='height:10px'></div></td></tr><tr id='r1'><td><div style='height:20px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 100.0), ("r1", 108.0, 100.0)],
        ),
        (
            "two 50% rows",
            "<table id='t' style='height:200px'><tr id='r0' style='height:50%'><td><div style='height:10px'></div></td></tr><tr id='r1' style='height:50%'><td><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 100.0), ("r1", 108.0, 100.0)],
        ),
        (
            "a 25% row and two free rows",
            "<table id='t' style='height:200px'><tr id='r0' style='height:25%'><td><div style='height:10px'></div></td></tr><tr id='r1'><td><div style='height:10px'></div></td></tr><tr id='r2'><td><div style='height:30px'></div></td></tr></table>",
            &[
                ("t", 8.0, 200.0),
                ("r0", 8.0, 50.0),
                ("r1", 58.0, 37.5),
                ("r2", 95.5, 112.5),
            ],
        ),
        (
            "two 30% rows and a free one",
            "<table id='t' style='height:200px'><tr id='r0' style='height:30%'><td><div style='height:10px'></div></td></tr><tr id='r1' style='height:30%'><td><div style='height:10px'></div></td></tr><tr id='r2'><td><div style='height:10px'></div></td></tr></table>",
            &[
                ("t", 8.0, 200.0),
                ("r0", 8.0, 60.0),
                ("r1", 68.0, 60.0),
                ("r2", 128.0, 80.0),
            ],
        ),
        (
            "a row with a length, a 50% row and a free one",
            "<table id='t' style='height:200px'><tr id='r0' style='height:30px'><td><div style='height:10px'></div></td></tr><tr id='r1' style='height:50%'><td><div style='height:10px'></div></td></tr><tr id='r2'><td><div style='height:10px'></div></td></tr></table>",
            &[
                ("t", 8.0, 200.0),
                ("r0", 8.0, 30.0),
                ("r1", 38.0, 100.0),
                ("r2", 138.0, 70.0),
            ],
        ),
        (
            "a 50% row without content beside a free one",
            "<table id='t' style='height:200px'><tr id='r0' style='height:50%'><td></td></tr><tr id='r1'><td></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 100.0), ("r1", 108.0, 100.0)],
        ),
        (
            "a 0% row keeps its content",
            "<table id='t' style='height:200px'><tr id='r0'><td><div style='height:20px'></div></td></tr><tr id='r1' style='height:0%'><td><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 190.0), ("r1", 198.0, 10.0)],
        ),
        (
            "free rows without content share what a 20% row leaves",
            "<table id='t' style='height:300px'><tr id='r0'><td><div style='height:0px'></div></td></tr><tr id='r1' style='height:20%'><td><div style='height:10px'></div></td></tr><tr id='r2'><td><div style='height:0px'></div></td></tr></table>",
            &[
                ("t", 8.0, 300.0),
                ("r0", 8.0, 120.0),
                ("r1", 128.0, 60.0),
                ("r2", 188.0, 120.0),
            ],
        ),
    ];
    check_rows(cases);
}

/// The percentage rows grow in order while the table has height to give; a row never gets less than its content.
#[test]
fn a_percentage_row_gets_no_more_than_the_extra_that_remains() {
    let cases: &[Case<'_>] = &[
        (
            "60% and 60%",
            "<table id='t' style='height:200px'><tr id='r0' style='height:60%'><td><div style='height:10px'></div></td></tr><tr id='r1' style='height:60%'><td><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 120.0), ("r1", 128.0, 80.0)],
        ),
        (
            "a free row and a 100% row",
            "<table id='t' style='height:200px'><tr id='r0'><td><div style='height:10px'></div></td></tr><tr id='r1' style='height:100%'><td><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 10.0), ("r1", 18.0, 190.0)],
        ),
        (
            "two 100% rows",
            "<table id='t' style='height:200px'><tr id='r0' style='height:100%'><td><div style='height:10px'></div></td></tr><tr id='r1' style='height:100%'><td><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 190.0), ("r1", 198.0, 10.0)],
        ),
        (
            "content taller than the share",
            "<table id='t' style='height:200px'><tr id='r0' style='height:50%'><td><div style='height:150px'></div></td></tr><tr id='r1'><td><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 150.0), ("r1", 158.0, 50.0)],
        ),
        (
            "a share smaller than the content of the row",
            "<table id='t' style='height:200px'><tr id='r0'><td><div style='height:20px'></div></td></tr><tr id='r1' style='height:5%'><td><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 190.0), ("r1", 198.0, 10.0)],
        ),
    ];
    check_rows(cases);
}

/// With no free row left, the rows with a percentage or an authored height share what remains in proportion to their heights.
#[test]
fn rows_that_all_have_a_percentage_or_an_authored_height_share_the_rest_in_proportion() {
    let cases: &[Case<'_>] = &[
        (
            "a row with a length and a 50% row",
            "<table id='t' style='height:200px'><tr id='r0' style='height:40px'><td><div style='height:10px'></div></td></tr><tr id='r1' style='height:50%'><td><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 57.14), ("r1", 65.14, 142.86)],
        ),
        (
            "20%, 30% and 10% rows",
            "<table id='t' style='height:200px'><tr id='r0' style='height:20%'><td><div style='height:10px'></div></td></tr><tr id='r1' style='height:30%'><td><div style='height:10px'></div></td></tr><tr id='r2' style='height:10%'><td><div style='height:10px'></div></td></tr></table>",
            &[
                ("t", 8.0, 200.0),
                ("r0", 8.0, 66.66),
                ("r1", 74.66, 100.0),
                ("r2", 174.66, 33.34),
            ],
        ),
        (
            "a length, a 50% and a length row",
            "<table id='t' style='height:300px'><tr id='r0' style='height:20px'><td><div style='height:10px'></div></td></tr><tr id='r1' style='height:50%'><td><div style='height:10px'></div></td></tr><tr id='r2' style='height:40px'><td><div style='height:10px'></div></td></tr></table>",
            &[
                ("t", 8.0, 300.0),
                ("r0", 8.0, 28.56),
                ("r1", 36.56, 214.28),
                ("r2", 250.84, 57.16),
            ],
        ),
        (
            "a row with a length and a 5% row",
            "<table id='t' style='height:200px'><tr id='r0' style='height:30px'><td><div style='height:20px'></div></td></tr><tr id='r1' style='height:5%'><td><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 150.0), ("r1", 158.0, 50.0)],
        ),
        (
            "10% rows with taller content",
            "<table id='t' style='height:200px'><tr id='r0' style='height:10%'><td><div style='height:30px'></div></td></tr><tr id='r1' style='height:10%'><td><div style='height:30px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 100.0), ("r1", 108.0, 100.0)],
        ),
    ];
    check_rows(cases);
}

/// The percentage of a row is the largest of its own and those of the cells confined to it; a cell that spans rows does not count.
#[test]
fn a_percentage_on_a_cell_counts_for_its_row() {
    let cases: &[Case<'_>] = &[
        (
            "a 50% cell beside a free row",
            "<table id='t' style='height:200px'><tr id='r0'><td style='height:50%'><div style='height:10px'></div></td></tr><tr id='r1'><td><div style='height:20px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 100.0), ("r1", 108.0, 100.0)],
        ),
        (
            "20% row with a 50% cell",
            "<table id='t' style='height:200px'><tr id='r0' style='height:20%'><td style='height:50%'><div style='height:10px'></div></td></tr><tr id='r1'><td><div style='height:20px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 100.0), ("r1", 108.0, 100.0)],
        ),
        (
            "50% row with a 20% cell",
            "<table id='t' style='height:200px'><tr id='r0' style='height:50%'><td style='height:20%'><div style='height:10px'></div></td></tr><tr id='r1'><td><div style='height:20px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 100.0), ("r1", 108.0, 100.0)],
        ),
        (
            "cells of 20% and 40% in one row",
            "<table id='t' style='height:200px'><tr id='r0'><td style='height:20%'><div style='height:10px'></div></td><td style='height:40%'><div style='height:10px'></div></td></tr><tr id='r1'><td><div style='height:20px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 80.0), ("r1", 88.0, 120.0)],
        ),
        (
            "a row with a length and a 50% cell",
            "<table id='t' style='height:200px'><tr id='r0' style='height:30px'><td style='height:50%'><div style='height:10px'></div></td></tr><tr id='r1'><td><div style='height:20px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 100.0), ("r1", 108.0, 100.0)],
        ),
        (
            "a 50% row with a cell with a length",
            "<table id='t' style='height:200px'><tr id='r0'><td><div style='height:20px'></div></td></tr><tr id='r1' style='height:50%'><td style='height:30px'><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 100.0), ("r1", 108.0, 100.0)],
        ),
        (
            "a spanning cell with a percentage does not count",
            "<table id='t' style='height:200px'><tr id='r0' style='height:25%'><td id='c' rowspan='2' style='height:80%'><div style='height:10px'></div></td><td><div style='height:10px'></div></td></tr><tr id='r1'><td><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 50.0), ("r1", 58.0, 150.0)],
        ),
        (
            "a 50% row with a spanning cell",
            "<table id='t' style='height:200px'><tr id='r0' style='height:50%'><td rowspan='2'><div style='height:40px'></div></td><td><div style='height:10px'></div></td></tr><tr id='r1'><td><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 100.0), ("r1", 108.0, 100.0)],
        ),
    ];
    check_rows(cases);
}

/// The percentage is of the height of the rows' group: the table's height after `min-height`, without its padding, border and the border-spacing at its edges.
#[test]
fn the_percentage_is_of_the_height_the_table_ends_up_with() {
    let cases: &[Case<'_>] = &[
        (
            "min-height",
            "<table id='t' style='min-height:200px'><tr id='r0'><td><div style='height:20px'></div></td></tr><tr id='r1' style='height:50%'><td><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 100.0), ("r1", 108.0, 100.0)],
        ),
        (
            "min-height above height",
            "<table id='t' style='height:100px;min-height:200px'><tr id='r0'><td><div style='height:20px'></div></td></tr><tr id='r1' style='height:50%'><td><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 100.0), ("r1", 108.0, 100.0)],
        ),
        (
            "height above min-height",
            "<table id='t' style='height:300px;min-height:200px'><tr id='r0'><td><div style='height:20px'></div></td></tr><tr id='r1' style='height:50%'><td><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 300.0), ("r0", 8.0, 150.0), ("r1", 158.0, 150.0)],
        ),
        (
            "a table height in a percentage",
            "<div style='height:300px'><table id='t' style='height:50%'><tr id='r0'><td><div style='height:20px'></div></td></tr><tr id='r1' style='height:50%'><td><div style='height:10px'></div></td></tr></table></div>",
            &[("t", 8.0, 150.0), ("r0", 8.0, 75.0), ("r1", 83.0, 75.0)],
        ),
        (
            "a table of the height of its container",
            "<div style='display:inline-block;height:300px'><table id='t' style='height:100%'><tr id='r0'><td><div style='height:10px'></div></td></tr><tr id='r1' style='height:50%'><td><div style='height:10px'></div></td></tr></table></div>",
            &[("t", 8.0, 300.0), ("r0", 8.0, 150.0), ("r1", 158.0, 150.0)],
        ),
        (
            "a border-box table with borders",
            "<table id='t' style='height:200px;box-sizing:border-box;border:10px solid'><tr id='r0'><td><div style='height:10px'></div></td></tr><tr id='r1' style='height:50%'><td><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 18.0, 90.0), ("r1", 108.0, 90.0)],
        ),
        (
            "padding",
            "<table id='t' style='height:300px;padding:10px'><tr id='r0'><td><div style='height:20px'></div></td></tr><tr id='r1' style='height:50%'><td><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 300.0), ("r0", 18.0, 140.0), ("r1", 158.0, 140.0)],
        ),
        (
            "border-spacing 4px, two rows",
            "<table id='t' style='height:300px;border-spacing:4px'><tr id='r0'><td><div style='height:20px'></div></td></tr><tr id='r1' style='height:50%'><td><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 300.0), ("r0", 12.0, 142.0), ("r1", 158.0, 146.0)],
        ),
        (
            "border-spacing 10px, four rows",
            "<table id='t' style='height:300px;border-spacing:10px'><tr id='r0'><td><div style='height:20px'></div></td></tr><tr id='r1'><td><div style='height:10px'></div></td></tr><tr id='r2'><td><div style='height:10px'></div></td></tr><tr id='r3' style='height:50%'><td><div style='height:10px'></div></td></tr></table>",
            &[
                ("t", 8.0, 300.0),
                ("r0", 18.0, 55.0),
                ("r1", 83.0, 27.5),
                ("r2", 120.5, 27.5),
                ("r3", 158.0, 140.0),
            ],
        ),
        (
            "border-spacing, border and padding",
            "<table id='t' style='height:200px;border-spacing:4px;border:5px solid;padding:3px'><tr id='r0'><td><div style='height:20px'></div></td></tr><tr id='r1' style='height:50%'><td><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 20.0, 84.0), ("r1", 108.0, 88.0)],
        ),
        (
            "border-spacing with two percentage rows",
            "<table id='t' style='height:300px;border-spacing:4px'><tr id='r0' style='height:20%'><td><div style='height:10px'></div></td></tr><tr id='r1' style='height:30%'><td><div style='height:10px'></div></td></tr><tr id='r2'><td><div style='height:10px'></div></td></tr></table>",
            &[
                ("t", 8.0, 300.0),
                ("r0", 12.0, 58.39),
                ("r1", 74.39, 87.59),
                ("r2", 165.98, 138.02),
            ],
        ),
        (
            "table-layout: fixed",
            "<table id='t' style='height:200px;table-layout:fixed;width:100px'><tr id='r0' style='height:50%'><td><div style='height:10px'></div></td></tr><tr id='r1'><td><div style='height:20px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 100.0), ("r1", 108.0, 100.0)],
        ),
        (
            "in a tbody",
            "<table id='t' style='height:200px'><tbody id='s0'><tr id='r0' style='height:50%'><td><div style='height:10px'></div></td></tr><tr id='r1'><td><div style='height:10px'></div></td></tr></tbody></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 100.0), ("r1", 108.0, 100.0)],
        ),
    ];
    check_rows(cases);
}

/// A percentage row of a table with no definite height, a `calc()` that mixes in a length, and a row that takes no part are treated as before; rows from several groups are too, since Chromium sizes the groups first.
#[test]
fn a_percentage_that_cannot_resolve_is_left_to_the_rows() {
    let cases: &[Case<'_>] = &[
        (
            "a table without a height",
            "<table id='t'><tr id='r0'><td><div style='height:20px'></div></td></tr><tr id='r1' style='height:50%'><td><div style='height:10px'></div></td></tr></table>",
            &[("t", 8.0, 30.0), ("r0", 8.0, 20.0), ("r1", 28.0, 10.0)],
        ),
        (
            "a table without a height, a tall row",
            "<table id='t'><tr id='r0' style='height:50%'><td><div style='height:60px'></div></td></tr><tr id='r1'><td><div style='height:20px'></div></td></tr></table>",
            &[("t", 8.0, 80.0), ("r0", 8.0, 60.0), ("r1", 68.0, 20.0)],
        ),
        (
            "a percentage table height in an auto container",
            "<div><table id='t' style='height:50%'><tr id='r0'><td><div style='height:20px'></div></td></tr><tr id='r1' style='height:50%'><td><div style='height:10px'></div></td></tr></table></div>",
            &[("t", 8.0, 30.0), ("r0", 8.0, 20.0), ("r1", 28.0, 10.0)],
        ),
        (
            "a calc() with a length",
            "<table id='t' style='height:200px'><tr id='r0' style='height:calc(50% + 10px)'><td><div style='height:10px'></div></td></tr><tr id='r1'><td><div style='height:20px'></div></td></tr></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 66.66), ("r1", 74.66, 133.34)],
        ),
        (
            "a hidden 50% row",
            "<table id='t' style='height:200px'><tr id='r0'><td><div style='height:20px'></div></td></tr><tr id='r1' style='height:50%;display:none'><td></td></tr><tr id='r2'><td><div style='height:10px'></div></td></tr></table>",
            &[
                ("t", 8.0, 200.0),
                ("r0", 8.0, 133.33),
                ("r2", 141.33, 66.67),
            ],
        ),
        (
            "rows in two groups",
            "<table id='t' style='height:200px'><tbody id='s0'><tr id='r0' style='height:50%'><td><div style='height:10px'></div></td></tr></tbody><tbody id='s1'><tr id='r1'><td><div style='height:20px'></div></td></tr></tbody></table>",
            &[("t", 8.0, 200.0), ("r0", 8.0, 66.66), ("r1", 74.66, 133.34)],
        ),
    ];
    check_rows(cases);
}

/// Two or more row groups share the extra in proportion to their heights (their rows and the spacing between them) before the rows in each group share what it got.
#[test]
fn row_groups_share_the_extra_before_their_rows_do() {
    let cases: &[Case<'_>] = &[
        (
            "two groups of free rows",
            "<table id=t style='height:200px'><tbody id=b0><tr id=r0><td id=c0><div style='height:20px'></div></td></tr></tbody><tbody id=b1><tr id=r1><td id=c1><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 133.33),
                ("r0", 8.0, 133.33),
                ("b1", 141.33, 66.67),
                ("r1", 141.33, 66.67),
            ],
        ),
        (
            "a group with an authored row and a group of one row",
            "<table id=t style='height:200px'><tbody id=b0><tr id=r0><td id=c0><div style='height:20px'></div></td></tr><tr id=r1 style='height:40px'><td id=c1><div style='height:10px'></div></td></tr></tbody><tbody id=b1><tr id=r2><td id=c2><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 171.42),
                ("r0", 8.0, 131.42),
                ("r1", 139.42, 40.0),
                ("b1", 179.42, 28.58),
                ("r2", 179.42, 28.58),
            ],
        ),
        (
            "a group whose only row is authored grows too",
            "<table id=t style='height:200px'><tbody id=b0><tr id=r0 style='height:30px'><td id=c0><div style='height:20px'></div></td></tr></tbody><tbody id=b1><tr id=r1><td id=c1><div style='height:10px'></div></td></tr><tr id=r2><td id=c2><div style='height:30px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 85.7),
                ("r0", 8.0, 85.7),
                ("b1", 93.7, 114.3),
                ("r1", 93.7, 28.56),
                ("r2", 122.27, 85.73),
            ],
        ),
        (
            "all rows authored",
            "<table id=t style='height:200px'><tbody id=b0><tr id=r0 style='height:30px'><td id=c0><div style='height:10px'></div></td></tr></tbody><tbody id=b1><tr id=r1 style='height:20px'><td id=c1><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 120.0),
                ("r0", 8.0, 120.0),
                ("b1", 128.0, 80.0),
                ("r1", 128.0, 80.0),
            ],
        ),
        (
            "three groups, the middle one authored",
            "<table id=t style='height:200px'><tbody id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr></tbody><tbody id=b1><tr id=r1 style='height:20px'><td id=c1><div style='height:10px'></div></td></tr></tbody><tbody id=b2><tr id=r2><td id=c2><div style='height:30px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 33.33),
                ("r0", 8.0, 33.33),
                ("b1", 41.33, 66.66),
                ("r1", 41.33, 66.66),
                ("b2", 107.98, 100.02),
                ("r2", 107.98, 100.02),
            ],
        ),
        (
            "an authored row in each group",
            "<table id=t style='height:200px'><tbody id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr><tr id=r1 style='height:50px'><td id=c1><div style='height:50px'></div></td></tr></tbody><tbody id=b1><tr id=r2 style='height:20px'><td id=c2><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 150.0),
                ("r0", 8.0, 100.0),
                ("r1", 108.0, 50.0),
                ("b1", 158.0, 50.0),
                ("r2", 158.0, 50.0),
            ],
        ),
        (
            "free and authored rows in two groups",
            "<table id=t style='height:200px'><tbody id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr><tr id=r1 style='height:30px'><td id=c1><div style='height:10px'></div></td></tr></tbody><tbody id=b1><tr id=r2 style='height:30px'><td id=c2><div style='height:10px'></div></td></tr><tr id=r3><td id=c3><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 100.0),
                ("r0", 8.0, 70.0),
                ("r1", 78.0, 30.0),
                ("b1", 108.0, 100.0),
                ("r2", 108.0, 30.0),
                ("r3", 138.0, 70.0),
            ],
        ),
        (
            "three groups of several rows",
            "<table id=t style='height:200px'><tbody id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr><tr id=r1><td id=c1><div style='height:20px'></div></td></tr></tbody><tbody id=b1><tr id=r2><td id=c2><div style='height:30px'></div></td></tr></tbody><tbody id=b2><tr id=r3><td id=c3><div style='height:10px'></div></td></tr><tr id=r4><td id=c4><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 75.0),
                ("r0", 8.0, 25.0),
                ("r1", 33.0, 50.0),
                ("b1", 83.0, 75.0),
                ("r2", 83.0, 75.0),
                ("b2", 158.0, 50.0),
                ("r3", 158.0, 25.0),
                ("r4", 183.0, 25.0),
            ],
        ),
        (
            "empty groups share equally",
            "<table id=t style='height:200px'><tbody id=b0><tr id=r0><td id=c0><div style='height:0px'></div></td></tr></tbody><tbody id=b1><tr id=r1><td id=c1><div style='height:0px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 100.0),
                ("r0", 8.0, 100.0),
                ("b1", 108.0, 100.0),
                ("r1", 108.0, 100.0),
            ],
        ),
        (
            "an empty group next to one with content",
            "<table id=t style='height:200px'><tbody id=b0><tr id=r0><td id=c0><div style='height:0px'></div></td></tr></tbody><tbody id=b1><tr id=r1><td id=c1><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 0.0),
                ("r0", 8.0, 0.0),
                ("b1", 8.0, 200.0),
                ("r1", 8.0, 200.0),
            ],
        ),
        (
            "a min-height shares like a height",
            "<table id=t style='min-height:200px'><tbody id=b0><tr id=r0><td id=c0><div style='height:20px'></div></td></tr><tr id=r1 style='height:40px'><td id=c1><div style='height:10px'></div></td></tr></tbody><tbody id=b1><tr id=r2><td id=c2><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 171.42),
                ("r0", 8.0, 131.42),
                ("r1", 139.42, 40.0),
                ("b1", 179.42, 28.58),
                ("r2", 179.42, 28.58),
            ],
        ),
        (
            "border spacing between the rows and the groups",
            "<table id=t style='height:200px;border-spacing:5px'><tbody id=b0><tr id=r0><td id=c0><div style='height:20px'></div></td></tr><tr id=r1 style='height:40px'><td id=c1><div style='height:10px'></div></td></tr></tbody><tbody id=b1><tr id=r2><td id=c2><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 13.0, 160.33),
                ("r0", 13.0, 115.33),
                ("r1", 133.33, 40.0),
                ("b1", 178.33, 24.67),
                ("r2", 178.33, 24.67),
            ],
        ),
        (
            "spacing inside a group counts toward its height",
            "<table id=t style='height:400px;border-spacing:10px'><tbody id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr><tr id=r1 style='height:25%'><td id=c1><div style='height:10px'></div></td></tr></tbody><tbody id=b1><tr id=r2><td id=c2><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 400.0),
                ("b0", 18.0, 277.5),
                ("r0", 18.0, 198.13),
                ("r1", 226.13, 69.38),
                ("b1", 305.5, 92.5),
                ("r2", 305.5, 92.5),
            ],
        ),
        (
            "a group of one big authored row next to a small free one",
            "<table id=t style='height:400px'><tbody id=b0><tr id=r0 style='height:100px'><td id=c0><div style='height:10px'></div></td></tr></tbody><tbody id=b1><tr id=r1><td id=c1><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 400.0),
                ("b0", 8.0, 363.63),
                ("r0", 8.0, 363.63),
                ("b1", 371.63, 36.38),
                ("r1", 371.63, 36.38),
            ],
        ),
        (
            "the same with a header on top",
            "<table id=t style='height:400px'><thead id=b0><tr id=r0 style='height:100px'><td id=c0><div style='height:10px'></div></td></tr></thead><tbody id=b1><tr id=r1><td id=c1><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 400.0),
                ("b0", 8.0, 100.0),
                ("r0", 8.0, 100.0),
                ("b1", 108.0, 300.0),
                ("r1", 108.0, 300.0),
            ],
        ),
    ];
    check_rows(cases);
}

/// The header group on top and the footer group at the bottom keep the height of their rows while a body group can take the extra; with no body group they share it like any other.
#[test]
fn the_header_and_footer_groups_keep_their_height_while_a_body_group_can_grow() {
    let cases: &[Case<'_>] = &[
        (
            "an authored header",
            "<table id=t style='height:200px'><thead id=b0><tr id=r0 style='height:30px'><td id=c0><div style='height:20px'></div></td></tr></thead><tbody id=b1><tr id=r1><td id=c1><div style='height:10px'></div></td></tr><tr id=r2><td id=c2><div style='height:30px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 30.0),
                ("r0", 8.0, 30.0),
                ("b1", 38.0, 170.0),
                ("r1", 38.0, 42.5),
                ("r2", 80.5, 127.5),
            ],
        ),
        (
            "a free header",
            "<table id=t style='height:200px'><thead id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr></thead><tbody id=b1><tr id=r1><td id=c1><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 10.0),
                ("r0", 8.0, 10.0),
                ("b1", 18.0, 190.0),
                ("r1", 18.0, 190.0),
            ],
        ),
        (
            "a footer",
            "<table id=t style='height:200px'><tbody id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr></tbody><tfoot id=b1><tr id=r1 style='height:30px'><td id=c1><div style='height:10px'></div></td></tr></tfoot></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 170.0),
                ("r0", 8.0, 170.0),
                ("b1", 178.0, 30.0),
                ("r1", 178.0, 30.0),
            ],
        ),
        (
            "a header, a body and a footer",
            "<table id=t style='height:200px'><thead id=b0><tr id=r0 style='height:30px'><td id=c0><div style='height:10px'></div></td></tr></thead><tbody id=b1><tr id=r1><td id=c1><div style='height:10px'></div></td></tr></tbody><tfoot id=b2><tr id=r2 style='height:30px'><td id=c2><div style='height:10px'></div></td></tr></tfoot></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 30.0),
                ("r0", 8.0, 30.0),
                ("b1", 38.0, 140.0),
                ("r1", 38.0, 140.0),
                ("b2", 178.0, 30.0),
                ("r2", 178.0, 30.0),
            ],
        ),
        (
            "an authored header and an authored body",
            "<table id=t style='height:200px'><thead id=b0><tr id=r0 style='height:30px'><td id=c0><div style='height:10px'></div></td></tr></thead><tbody id=b1><tr id=r1 style='height:20px'><td id=c1><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 30.0),
                ("r0", 8.0, 30.0),
                ("b1", 38.0, 170.0),
                ("r1", 38.0, 170.0),
            ],
        ),
        (
            "an authored body and a footer",
            "<table id=t style='height:200px'><tbody id=b0><tr id=r0 style='height:30px'><td id=c0><div style='height:10px'></div></td></tr></tbody><tfoot id=b1><tr id=r1 style='height:20px'><td id=c1><div style='height:10px'></div></td></tr></tfoot></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 180.0),
                ("r0", 8.0, 180.0),
                ("b1", 188.0, 20.0),
                ("r1", 188.0, 20.0),
            ],
        ),
        (
            "a header, two bodies and a footer",
            "<table id=t style='height:200px'><thead id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr></thead><tbody id=b1><tr id=r1><td id=c1><div style='height:10px'></div></td></tr></tbody><tbody id=b2><tr id=r2><td id=c2><div style='height:30px'></div></td></tr></tbody><tfoot id=b3><tr id=r3><td id=c3><div style='height:10px'></div></td></tr></tfoot></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 10.0),
                ("r0", 8.0, 10.0),
                ("b1", 18.0, 45.0),
                ("r1", 18.0, 45.0),
                ("b2", 63.0, 135.0),
                ("r2", 63.0, 135.0),
                ("b3", 198.0, 10.0),
                ("r3", 198.0, 10.0),
            ],
        ),
        (
            "an empty body between them",
            "<table id=t style='height:200px'><thead id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr></thead><tbody id=b1><tr id=r1><td id=c1><div style='height:0px'></div></td></tr></tbody><tfoot id=b2><tr id=r2><td id=c2><div style='height:10px'></div></td></tr></tfoot></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 10.0),
                ("r0", 8.0, 10.0),
                ("b1", 18.0, 180.0),
                ("r1", 18.0, 180.0),
                ("b2", 198.0, 10.0),
                ("r2", 198.0, 10.0),
            ],
        ),
        (
            "only the first header group stays",
            "<table id=t style='height:200px'><thead id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr></thead><thead id=b1><tr id=r1><td id=c1><div style='height:10px'></div></td></tr></thead><tbody id=b2><tr id=r2><td id=c2><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 10.0),
                ("r0", 8.0, 10.0),
                ("b1", 18.0, 95.0),
                ("r1", 18.0, 95.0),
                ("b2", 113.0, 95.0),
                ("r2", 113.0, 95.0),
            ],
        ),
        (
            "only the first footer group stays",
            "<table id=t style='height:200px'><tbody id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr></tbody><tfoot id=b1><tr id=r1><td id=c1><div style='height:10px'></div></td></tr></tfoot><tfoot id=b2><tr id=r2><td id=c2><div style='height:10px'></div></td></tr></tfoot></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 95.0),
                ("r0", 8.0, 95.0),
                ("b1", 198.0, 10.0),
                ("r1", 198.0, 10.0),
                ("b2", 103.0, 95.0),
                ("r2", 103.0, 95.0),
            ],
        ),
        (
            "a header and a footer without a body",
            "<table id=t style='height:200px'><thead id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr></thead><tfoot id=b1><tr id=r1><td id=c1><div style='height:20px'></div></td></tr></tfoot></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 66.66),
                ("r0", 8.0, 66.66),
                ("b1", 74.66, 133.34),
                ("r1", 74.66, 133.34),
            ],
        ),
        (
            "a header alone",
            "<table id=t style='height:200px'><thead id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr></thead></table>",
            &[("t", 8.0, 200.0), ("b0", 8.0, 200.0), ("r0", 8.0, 200.0)],
        ),
        (
            "a footer alone",
            "<table id=t style='height:200px'><tfoot id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr></tfoot></table>",
            &[("t", 8.0, 200.0), ("b0", 8.0, 200.0), ("r0", 8.0, 200.0)],
        ),
        (
            "a header with a hidden body",
            "<table id=t style='height:200px'><thead id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr></thead><tbody id=b1 style='display:none'><tr id=r1><td id=c1><div style='height:10px'></div></td></tr></tbody></table>",
            &[("t", 8.0, 200.0), ("b0", 8.0, 200.0), ("r0", 8.0, 200.0)],
        ),
        (
            "a header and a footer with the footer hidden",
            "<table id=t style='height:200px'><thead id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr></thead><tfoot id=b1 style='display:none'><tr id=r1><td id=c1><div style='height:10px'></div></td></tr></tfoot></table>",
            &[("t", 8.0, 200.0), ("b0", 8.0, 200.0), ("r0", 8.0, 200.0)],
        ),
        (
            "a percentage row in the header does not grow it",
            "<table id=t style='height:400px'><thead id=b0><tr id=r0 style='height:50%'><td id=c0><div style='height:10px'></div></td></tr></thead><tbody id=b1><tr id=r1><td id=c1><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 400.0),
                ("b0", 8.0, 10.0),
                ("r0", 8.0, 10.0),
                ("b1", 18.0, 390.0),
                ("r1", 18.0, 390.0),
            ],
        ),
        (
            "a percentage row in the footer does not grow it",
            "<table id=t style='height:400px'><tbody id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr></tbody><tfoot id=b1><tr id=r1 style='height:50%'><td id=c1><div style='height:10px'></div></td></tr></tfoot></table>",
            &[
                ("t", 8.0, 400.0),
                ("b0", 8.0, 390.0),
                ("r0", 8.0, 390.0),
                ("b1", 398.0, 10.0),
                ("r1", 398.0, 10.0),
            ],
        ),
        (
            "a percentage row in the body takes its share of the body",
            "<table id=t style='height:400px'><thead id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr></thead><tbody id=b1><tr id=r1><td id=c1><div style='height:10px'></div></td></tr><tr id=r2 style='height:50%'><td id=c2><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 400.0),
                ("b0", 8.0, 10.0),
                ("r0", 8.0, 10.0),
                ("b1", 18.0, 390.0),
                ("r1", 18.0, 195.0),
                ("r2", 213.0, 195.0),
            ],
        ),
        (
            "an empty first header leaves the second one a body group",
            "<table id=t style='height:200px'><thead id=b0></thead><thead id=b1><tr id=r0><td id=c0><div style='height:10px'></div></td></tr></thead><tbody id=b2><tr id=r1><td id=c1><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 200.0),
                ("b1", 8.0, 100.0),
                ("r0", 8.0, 100.0),
                ("b2", 108.0, 100.0),
                ("r1", 108.0, 100.0),
            ],
        ),
        (
            "an empty first footer leaves the second one a body group",
            "<table id=t style='height:200px'><tbody id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr></tbody><tfoot id=b1></tfoot><tfoot id=b2><tr id=r1><td id=c1><div style='height:10px'></div></td></tr></tfoot></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 100.0),
                ("r0", 8.0, 100.0),
                ("b2", 108.0, 100.0),
                ("r1", 108.0, 100.0),
            ],
        ),
        (
            "a header and a body",
            "<table id=t style='height:200px'><thead id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr></thead><tbody id=b1><tr id=r1><td id=c1><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 10.0),
                ("r0", 8.0, 10.0),
                ("b1", 18.0, 190.0),
                ("r1", 18.0, 190.0),
            ],
        ),
    ];
    check_rows(cases);
}

/// A percentage row takes that share of the height of its group, which is the table's only when the table has just one.
#[test]
fn a_percentage_row_resolves_against_its_group() {
    let cases: &[Case<'_>] = &[
        (
            "a quarter of the first group",
            "<table id=t style='height:400px'><tbody id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr><tr id=r1 style='height:25%'><td id=c1><div style='height:10px'></div></td></tr></tbody><tbody id=b1><tr id=r2><td id=c2><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 400.0),
                ("b0", 8.0, 266.66),
                ("r0", 8.0, 200.0),
                ("r1", 208.0, 66.66),
                ("b1", 274.66, 133.34),
                ("r2", 274.66, 133.34),
            ],
        ),
        (
            "half of the first group",
            "<table id=t style='height:400px'><tbody id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr><tr id=r1 style='height:50%'><td id=c1><div style='height:10px'></div></td></tr></tbody><tbody id=b1><tr id=r2><td id=c2><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 400.0),
                ("b0", 8.0, 266.66),
                ("r0", 8.0, 133.33),
                ("r1", 141.33, 133.33),
                ("b1", 274.66, 133.34),
                ("r2", 274.66, 133.34),
            ],
        ),
        (
            "the only row of a group",
            "<table id=t style='height:400px'><tbody id=b0><tr id=r0 style='height:25%'><td id=c0><div style='height:10px'></div></td></tr></tbody><tbody id=b1><tr id=r1><td id=c1><div style='height:10px'></div></td></tr></tbody><tbody id=b2><tr id=r2><td id=c2><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 400.0),
                ("b0", 8.0, 133.33),
                ("r0", 8.0, 133.33),
                ("b1", 141.33, 133.33),
                ("r1", 141.33, 133.33),
                ("b2", 274.66, 133.34),
                ("r2", 274.66, 133.34),
            ],
        ),
        (
            "a percentage over the group that holds it",
            "<table id=t style='height:400px'><tbody id=b0><tr id=r0 style='height:90%'><td id=c0><div style='height:10px'></div></td></tr></tbody><tbody id=b1><tr id=r1><td id=c1><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 400.0),
                ("b0", 8.0, 200.0),
                ("r0", 8.0, 200.0),
                ("b1", 208.0, 200.0),
                ("r1", 208.0, 200.0),
            ],
        ),
        (
            "a percentage in the second group",
            "<table id=t style='height:400px'><tbody id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr></tbody><tbody id=b1><tr id=r1><td id=c1><div style='height:10px'></div></td></tr><tr id=r2 style='height:25%'><td id=c2><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 400.0),
                ("b0", 8.0, 133.33),
                ("r0", 8.0, 133.33),
                ("b1", 141.33, 266.67),
                ("r1", 141.33, 200.02),
                ("r2", 341.34, 66.66),
            ],
        ),
        (
            "half of a group of two",
            "<table id=t style='height:200px'><tbody id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr><tr id=r1 style='height:50%'><td id=c1><div style='height:10px'></div></td></tr></tbody><tbody id=b1><tr id=r2><td id=c2><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 133.33),
                ("r0", 8.0, 66.67),
                ("r1", 74.67, 66.66),
                ("b1", 141.33, 66.67),
                ("r2", 141.33, 66.67),
            ],
        ),
        (
            "a percentage next to another group",
            "<table id=t style='height:200px'><tbody id=b0><tr id=r0 style='height:50%'><td id=c0><div style='height:10px'></div></td></tr></tbody><tbody id=b1><tr id=r1><td id=c1><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 200.0),
                ("b0", 8.0, 100.0),
                ("r0", 8.0, 100.0),
                ("b1", 108.0, 100.0),
                ("r1", 108.0, 100.0),
            ],
        ),
        (
            "padding and a border around the groups",
            "<table id=t style='height:400px;padding:10px;border:5px solid'><tbody id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr></tbody><tbody id=b1><tr id=r1 style='height:25%'><td id=c1><div style='height:10px'></div></td></tr></tbody></table>",
            &[
                ("t", 8.0, 400.0),
                ("b0", 23.0, 185.0),
                ("r0", 23.0, 185.0),
                ("b1", 208.0, 185.0),
                ("r1", 208.0, 185.0),
            ],
        ),
    ];
    check_rows(cases);
}

/// A group that is not displayed takes no part.
#[test]
fn a_hidden_group_takes_no_part_in_the_sharing() {
    let cases: &[Case<'_>] = &[(
        "a hidden group between two others",
        "<table id=t style='height:200px'><tbody id=b0><tr id=r0><td id=c0><div style='height:10px'></div></td></tr></tbody><tbody id=b1 style='display:none'><tr id=r1><td id=c1><div style='height:10px'></div></td></tr></tbody><tbody id=b2><tr id=r2><td id=c2><div style='height:10px'></div></td></tr></tbody></table>",
        &[
            ("t", 8.0, 200.0),
            ("b0", 8.0, 100.0),
            ("r0", 8.0, 100.0),
            ("b2", 108.0, 100.0),
            ("r2", 108.0, 100.0),
        ],
    )];
    check_rows(cases);
}
