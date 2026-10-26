//! The baseline of a table is the baseline of its first row (CSS 2.1
//! §10.8.1): an inline-level table sits on it in a line, and a flex or grid
//! container aligns the table by it. The first row is the one the table
//! starts with whether or not `visibility: collapse` removes it. The cells
//! that start in it and align by their baseline supply it, the tallest ascent
//! winning, and with none the baseline is the row's bottom edge, which is its
//! top once the row is collapsed.
//!
//! Every table follows a `<span id=y>Y</span>` in the same line, so the top of
//! the `y` span, the `Y` glyph's 8px ascent above the line's baseline, says
//! where the table's baseline is. The expected boxes are the ones Chromium
//! computes for the same markup in an 800px by 600px viewport with the body
//! margin of 8px: each is the top and height of the element.

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

/// Lays out each case and compares the elements it names with the expected top
/// and height.
fn check(cases: &[Case<'_>]) {
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

/// An inline-table sits on the baseline of its first row, not on its bottom edge: a baseline-aligned 40px cell gives 32px, a middle-aligned one leaves the row's bottom edge at 40px, and a collapsed second row changes neither.
#[test]
fn an_inline_table_sits_on_the_baseline_of_its_first_row() {
    let cases: &[Case<'_>] = &[
        (
            "baseline-aligned first row",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0><td id=c  style=\"vertical-align:baseline;font-size:40px;line-height:40px\"><span id=sc>X</span></td></tr><tr id=r1><td id=d  style=\"vertical-align:baseline;font-size:20px;line-height:20px\"><span id=sd>X</span></td></tr></table>",
            &[("y", 32.0, 10.0), ("t", 8.0, 60.0)],
        ),
        (
            "middle-aligned first row",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0><td id=c  style=\"font-size:40px;line-height:40px\"><span id=sc>X</span></td></tr><tr id=r1><td id=d  style=\"vertical-align:baseline;font-size:20px;line-height:20px\"><span id=sd>X</span></td></tr></table>",
            &[("y", 40.0, 10.0), ("t", 8.0, 60.0)],
        ),
        (
            "second row collapsed",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0><td id=c  style=\"vertical-align:baseline;font-size:20px;line-height:20px\"><span id=sc>X</span></td></tr><tr id=r1 style=\"visibility:collapse\"><td id=d  style=\"vertical-align:baseline;font-size:40px;line-height:40px\"><span id=sd>X</span></td></tr></table>",
            &[("y", 16.0, 10.0), ("t", 8.0, 20.0)],
        ),
    ];
    check(cases);
}

/// A collapsed first row still gives the baseline: from its tallest baseline-aligned cell, and from its own top when no cell aligns by its baseline. A row that is shown below it, or after it, does not.
#[test]
fn the_first_row_gives_the_baseline_even_when_it_collapses() {
    let cases: &[Case<'_>] = &[
        (
            "baseline-aligned cell",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0 style=\"visibility:collapse\"><td id=c  style=\"vertical-align:baseline;font-size:40px;line-height:40px\"><span id=sc>X</span></td></tr><tr id=r1><td id=d  style=\"vertical-align:baseline;font-size:20px;line-height:20px\"><span id=sd>X</span></td></tr></table>",
            &[("y", 32.0, 10.0), ("t", 8.0, 20.0)],
        ),
        (
            "middle-aligned cell",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0 style=\"visibility:collapse\"><td id=c  style=\"font-size:40px;line-height:40px\"><span id=sc>X</span></td></tr><tr id=r1><td id=d  style=\"font-size:20px;line-height:20px\"><span id=sd>X</span></td></tr></table>",
            &[("y", 8.0, 10.0), ("t", 16.0, 20.0)],
        ),
        (
            "middle-aligned collapsed cell, baseline-aligned row below",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0 style=\"visibility:collapse\"><td id=c  style=\"vertical-align:middle;font-size:40px;line-height:40px\"><span id=sc>X</span></td></tr><tr id=r1><td id=d  style=\"vertical-align:baseline;font-size:20px;line-height:20px\"><span id=sd>X</span></td></tr></table>",
            &[("y", 8.0, 10.0), ("t", 16.0, 20.0)],
        ),
        (
            "spanning cell",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0 style=\"visibility:collapse\"><td id=c rowspan=2 style=\"vertical-align:baseline;font-size:40px;line-height:40px\"><span id=sc>X</span></td></tr><tr id=r1><td id=d  style=\"vertical-align:baseline;font-size:20px;line-height:20px\"><span id=sd>X</span></td></tr></table>",
            &[("y", 32.0, 10.0), ("t", 8.0, 20.0)],
        ),
        (
            "spanning cell beside a taller cell",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0 style=\"visibility:collapse\"><td id=c rowspan=2 style=\"vertical-align:baseline;font-size:30px;line-height:30px\"><span id=sc>X</span></td><td id=a  style=\"vertical-align:baseline;font-size:40px;line-height:40px\"><span id=sa>X</span></td></tr><tr id=r1><td id=d  style=\"vertical-align:baseline;font-size:20px;line-height:20px\"><span id=sd>X</span></td></tr></table>",
            &[("y", 32.0, 10.0), ("t", 8.0, 20.0)],
        ),
        (
            "tallest of two cells",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0 style=\"visibility:collapse\"><td id=c  style=\"vertical-align:baseline;font-size:40px;line-height:40px\"><span id=sc>X</span></td><td id=a  style=\"vertical-align:baseline;font-size:30px;line-height:30px\"><span id=sa>X</span></td></tr><tr id=r1><td id=d  style=\"vertical-align:baseline;font-size:20px;line-height:20px\"><span id=sd>X</span></td><td id=e  style=\"vertical-align:baseline;font-size:10px;line-height:10px\"><span id=se>X</span></td></tr></table>",
            &[("y", 32.0, 10.0), ("t", 8.0, 20.0)],
        ),
        (
            "two collapsed rows then a shown one",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0 style=\"visibility:collapse\"><td id=c  style=\"vertical-align:baseline;font-size:40px;line-height:40px\"><span id=sc>X</span></td></tr><tr id=r1 style=\"visibility:collapse\"><td id=d  style=\"vertical-align:baseline;font-size:30px;line-height:30px\"><span id=sd>X</span></td></tr><tr id=r2><td id=e  style=\"vertical-align:baseline;font-size:20px;line-height:20px\"><span id=se>X</span></td></tr></table>",
            &[("y", 32.0, 10.0), ("t", 8.0, 20.0)],
        ),
        (
            "collapsed header group",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><thead><tr id=r0 style=\"visibility:collapse\"><td id=c  style=\"vertical-align:baseline;font-size:40px;line-height:40px\"><span id=sc>X</span></td></tr></thead><tbody><tr id=r1><td id=d  style=\"vertical-align:baseline;font-size:20px;line-height:20px\"><span id=sd>X</span></td></tr></tbody></table>",
            &[("y", 32.0, 10.0), ("t", 8.0, 20.0)],
        ),
        (
            "collapsed footer group before the body",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tfoot><tr id=r0 style=\"visibility:collapse\"><td id=c  style=\"vertical-align:baseline;font-size:40px;line-height:40px\"><span id=sc>X</span></td></tr></tfoot><tbody><tr id=r1><td id=d  style=\"vertical-align:baseline;font-size:20px;line-height:20px\"><span id=sd>X</span></td></tr></tbody></table>",
            &[("y", 16.0, 10.0), ("t", 8.0, 20.0)],
        ),
        (
            "cell padding",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0 style=\"visibility:collapse\"><td id=c  style=\"vertical-align:baseline;font-size:40px;line-height:40px;padding-top:7px\"><span id=sc>X</span></td></tr><tr id=r1><td id=d  style=\"vertical-align:baseline;font-size:20px;line-height:20px\"><span id=sd>X</span></td></tr></table>",
            &[("y", 39.0, 10.0), ("t", 8.0, 20.0)],
        ),
        (
            "border spacing",
            "<span id=y>Y</span><table id=t style=\"display:inline-table;border-spacing:4px\"><tr id=r0 style=\"visibility:collapse\"><td id=c  style=\"vertical-align:baseline;font-size:40px;line-height:40px\"><span id=sc>X</span></td></tr><tr id=r1><td id=d  style=\"vertical-align:baseline;font-size:20px;line-height:20px\"><span id=sd>X</span></td></tr></table>",
            &[("y", 36.0, 10.0), ("t", 8.0, 28.0)],
        ),
        (
            "table border and padding",
            "<span id=y>Y</span><table id=t style=\"display:inline-table;border:3px solid;padding:5px\"><tr id=r0 style=\"visibility:collapse\"><td id=c  style=\"vertical-align:baseline;font-size:40px;line-height:40px\"><span id=sc>X</span></td></tr><tr id=r1><td id=d  style=\"vertical-align:baseline;font-size:20px;line-height:20px\"><span id=sd>X</span></td></tr></table>",
            &[("y", 40.0, 10.0), ("t", 8.0, 36.0)],
        ),
        (
            "authored cell height",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0 style=\"visibility:collapse\"><td id=c  style=\"vertical-align:baseline;font-size:40px;line-height:40px;height:60px\"><span id=sc>X</span></td></tr><tr id=r1><td id=d  style=\"vertical-align:baseline;font-size:20px;line-height:20px\"><span id=sd>X</span></td></tr></table>",
            &[("y", 32.0, 10.0), ("t", 8.0, 20.0)],
        ),
        (
            "spanning middle cell",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0 style=\"visibility:collapse\"><td id=c rowspan=2 style=\"vertical-align:middle;font-size:40px;line-height:40px\"><span id=sc>X</span></td><td id=a  style=\"vertical-align:middle;font-size:10px;line-height:10px\"><span id=sa>X</span></td></tr><tr id=r1><td id=d  style=\"vertical-align:baseline;font-size:20px;line-height:20px\"><span id=sd>X</span></td></tr></table>",
            &[("y", 8.0, 10.0), ("t", 16.0, 26.67)],
        ),
    ];
    check(cases);
}

/// With every row collapsed the table has no height, but the baseline is still the first row's.
#[test]
fn a_table_whose_rows_all_collapse_keeps_the_baseline_of_its_first_row() {
    let cases: &[Case<'_>] = &[(
        "every row collapsed",
        "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0 style=\"visibility:collapse\"><td id=c  style=\"vertical-align:baseline;font-size:40px;line-height:40px\"><span id=sc>X</span></td></tr><tr id=r1 style=\"visibility:collapse\"><td id=d  style=\"vertical-align:baseline;font-size:20px;line-height:20px\"><span id=sd>X</span></td></tr></table>",
        &[("y", 32.0, 10.0), ("t", 8.0, 0.0)],
    )];
    check(cases);
}

/// A flex container with `align-items: baseline` aligns a table by the same baseline.
#[test]
fn a_flex_container_aligns_a_table_by_the_baseline_of_its_first_row() {
    let cases: &[Case<'_>] = &[(
        "flex item",
        "<div style=\"display:flex;align-items:baseline\"><span id=y>Y</span><table id=t style=\"\"><tr id=r0 style=\"visibility:collapse\"><td id=c  style=\"vertical-align:baseline;font-size:40px;line-height:40px\"><span id=sc>X</span></td></tr><tr id=r1><td id=d  style=\"vertical-align:baseline;font-size:20px;line-height:20px\"><span id=sd>X</span></td></tr></table></div>",
        &[("y", 32.0, 10.0), ("t", 8.0, 20.0)],
    )];
    check(cases);
}

/// A cell with nothing in flow, such as an empty one with an authored height, takes no part in aligning the row, and with no other cell to supply it the table sits on the bottom of the cells' content boxes as the row stretched them, so an authored height or padding moves it. A block gives the bottom edge of its own box, as a line gives its baseline. A caption below the row does not change any of it.
#[test]
fn only_a_cell_with_something_in_flow_gives_a_baseline() {
    let cases: &[Case<'_>] = &[
        (
            "empty baseline-aligned cell with an authored height",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0><td id=c style=\"vertical-align:baseline;height:96px\"></td></tr></table>",
            &[("y", 96.0, 10.0), ("t", 8.0, 96.0)],
        ),
        (
            "empty middle-aligned cell with an authored height",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0><td id=c style=\"vertical-align:middle;height:96px\"></td></tr></table>",
            &[("y", 96.0, 10.0), ("t", 8.0, 96.0)],
        ),
        (
            "baseline-aligned cell holding a 96px block",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0><td id=c style=\"vertical-align:baseline\"><div style=\"height:96px\"></div></td></tr></table>",
            &[("y", 96.0, 10.0), ("t", 8.0, 96.0)],
        ),
        (
            "authored height and padding below the cell content",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0><td id=c style=\"vertical-align:baseline;height:96px;padding-bottom:7px\"></td></tr></table>",
            &[("y", 96.0, 10.0), ("t", 8.0, 103.0)],
        ),
        (
            "empty cell beside a cell with a line of its own",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0><td id=c style=\"vertical-align:baseline;height:96px\"></td><td id=d style=\"vertical-align:baseline;font:20px/20px Ahem\">X</td></tr></table>",
            &[("y", 16.0, 10.0), ("t", 8.0, 96.0)],
        ),
        (
            "empty cell with top padding beside text: the text keeps the top of the row",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0><td id=c style=\"vertical-align:baseline;padding-top:96px\"></td><td id=d style=\"vertical-align:baseline;font:20px/20px Ahem\"><span id=sd>X</span></td></tr></table>",
            &[
                ("y", 16.0, 10.0),
                ("t", 8.0, 96.0),
                ("c", 8.0, 96.0),
                ("sd", 8.0, 20.0),
            ],
        ),
        (
            "empty cell with bottom padding beside text",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0><td id=c style=\"vertical-align:baseline;padding-bottom:96px\"></td><td id=d style=\"vertical-align:baseline;font:20px/20px Ahem\"><span id=sd>X</span></td></tr></table>",
            &[
                ("y", 16.0, 10.0),
                ("t", 8.0, 96.0),
                ("c", 8.0, 96.0),
                ("sd", 8.0, 20.0),
            ],
        ),
        (
            "empty cell with a top border shorter than the text",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0><td id=c style=\"vertical-align:baseline;border-top:30px solid\"></td><td id=d style=\"vertical-align:baseline;font:20px/20px Ahem\"><span id=sd>X</span></td></tr></table>",
            &[
                ("y", 16.0, 10.0),
                ("t", 8.0, 30.0),
                ("c", 8.0, 30.0),
                ("sd", 8.0, 20.0),
            ],
        ),
        (
            "empty cell with an authored height and top padding beside text",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0><td id=c style=\"vertical-align:baseline;height:96px;padding-top:10px\"></td><td id=d style=\"vertical-align:baseline;font:20px/20px Ahem\"><span id=sd>X</span></td></tr></table>",
            &[
                ("y", 16.0, 10.0),
                ("t", 8.0, 106.0),
                ("c", 8.0, 106.0),
                ("sd", 8.0, 20.0),
            ],
        ),
        (
            "empty cell in a row with an authored height",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0 style=\"height:50px\"><td id=c style=\"vertical-align:baseline\"></td></tr></table>",
            &[("y", 50.0, 10.0), ("t", 8.0, 50.0)],
        ),
        (
            "inline-table with a caption below its row",
            "<div id=table style=\"background:blue;caption-side:bottom;display:inline-table\"><div id=caption style=\"display:table-caption\">Filler Text</div><div id=row style=\"display:table-row\"><div id=cell style=\"background:blue;display:table-cell;height:1in;width:2in\"></div></div></div>",
            &[("caption", 104.0, 10.0), ("cell", 8.0, 96.0)],
        ),
        (
            "a baseline-aligned block beside text: the block gives its bottom edge",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0><td id=c style=\"vertical-align:baseline\"><div style=\"height:96px\"></div></td><td id=d style=\"vertical-align:baseline;font:20px/20px Ahem\"><span id=sd>X</span></td></tr></table>",
            &[("y", 96.0, 10.0), ("t", 8.0, 100.0), ("sd", 88.0, 20.0)],
        ),
        (
            "the same block, middle-aligned: only the text gives a baseline",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0><td id=c style=\"vertical-align:middle\"><div style=\"height:96px\"></div></td><td id=d style=\"vertical-align:baseline;font:20px/20px Ahem\"><span id=sd>X</span></td></tr></table>",
            &[("y", 16.0, 10.0), ("t", 8.0, 96.0), ("sd", 8.0, 20.0)],
        ),
        (
            "a shorter block beside the text",
            "<span id=y>Y</span><table id=t style=\"display:inline-table\"><tr id=r0><td id=c style=\"vertical-align:baseline\"><div style=\"height:30px\"></div></td><td id=d style=\"vertical-align:baseline;font:20px/20px Ahem\"><span id=sd>X</span></td></tr></table>",
            &[("y", 30.0, 10.0), ("t", 8.0, 34.0), ("sd", 22.0, 20.0)],
        ),
    ];
    check(cases);
}
