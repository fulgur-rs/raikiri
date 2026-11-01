//! A row removed by `visibility: collapse` still takes its share when a table
//! with a `height` or `min-height` of its own shares the extra height among its
//! rows, with the spacing around it, and gives that share up afterwards, so the
//! table ends up lower than the height it states.
//!
//! The expected boxes are the ones Chromium computes for the same markup in an
//! 800px by 600px viewport, with the body margin of 8px, a font of 10px Ahem,
//! cells without padding and tables without spacing unless a test says
//! otherwise: each is an element's top and height. Layout units are 1/64px, so
//! a share that Chromium rounds (14.31 for 14.29) differs a little.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The top and the height of the element `id` in `body`, on the first page of
/// 800px by 600px.
fn extent(body: &str, id: &str) -> [f32; 2] {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px Ahem }} td {{ padding: 0 }} \
         table {{ border-spacing: 0 }}</style>{body}"
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
        .find(|fragment| dom.attr(fragment.node(), "id") == Some(id))
        .map(|fragment| {
            let rect = fragment.rect();
            [rect.y, rect.height]
        })
        .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"))
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.04;

/// Compares the top and the height of the elements `expected` names, reporting
/// every mismatch at once.
fn check(label: &str, markup: &str, expected: &[(&str, f32, f32)]) {
    let mismatches: Vec<String> = expected
        .iter()
        .filter_map(|&(id, top, height)| {
            let found = extent(markup, id);
            let close =
                (found[0] - top).abs() <= TOLERANCE && (found[1] - height).abs() <= TOLERANCE;
            (!close).then(|| {
                format!(
                    "{label}: #{id} is at {} and {} high, expected {top} and {height}",
                    found[0], found[1]
                )
            })
        })
        .collect();
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

/// A row `id` whose cell is `height` high, collapsed or not.
fn row(id: u32, height: u32, collapsed: bool) -> String {
    let style = if collapsed {
        "visibility: collapse"
    } else {
        ""
    };
    format!("<tr id=r{id} style='{style}'><td id=c{id} style='height: {height}px'></td></tr>")
}

/// A table `t` with the style `style` and the rows `rows`.
fn table(style: &str, rows: &[String]) -> String {
    format!("<table id=t style='{style}'>{}</table>", rows.concat())
}

#[test]
fn a_collapsed_row_takes_its_share_and_gives_it_up() {
    // The 40px row is collapsed: the 100px table shares 40px among rows of 40
    // and 20, so the row that remains is 20 / 60 of 100 high, and the table is
    // as high as that row.
    check(
        "40 collapsed, 20",
        &table("height: 100px", &[row(1, 40, true), row(2, 20, false)]),
        &[("t", 8.0, 33.34), ("r1", 8.0, 0.0), ("r2", 8.0, 33.34)],
    );
    // The same table with nothing collapsed is as high as it states.
    check(
        "40, 20",
        &table("height: 100px", &[row(1, 40, false), row(2, 20, false)]),
        &[("t", 8.0, 100.0), ("r1", 8.0, 66.66), ("r2", 74.66, 33.34)],
    );
    check(
        "20 collapsed, 20",
        &table("height: 100px", &[row(1, 20, true), row(2, 20, false)]),
        &[("t", 8.0, 50.0), ("r2", 8.0, 50.0)],
    );
    check(
        "40 collapsed, 20, 10",
        &table(
            "height: 100px",
            &[row(1, 40, true), row(2, 20, false), row(3, 10, false)],
        ),
        &[("t", 8.0, 42.86), ("r2", 8.0, 28.56), ("r3", 36.56, 14.3)],
    );
}

#[test]
fn a_collapsed_row_anywhere_among_the_rows() {
    check(
        "last",
        &table("height: 100px", &[row(1, 20, false), row(2, 40, true)]),
        &[("t", 8.0, 33.33), ("r1", 8.0, 33.33), ("r2", 41.33, 0.0)],
    );
    check(
        "middle",
        &table(
            "height: 100px",
            &[row(1, 20, false), row(2, 40, true), row(3, 10, false)],
        ),
        &[
            ("t", 8.0, 42.86),
            ("r1", 8.0, 28.56),
            ("r2", 36.56, 0.0),
            ("r3", 36.56, 14.3),
        ],
    );
    check(
        "two",
        &table(
            "height: 100px",
            &[
                row(1, 30, true),
                row(2, 20, false),
                row(3, 10, true),
                row(4, 10, false),
            ],
        ),
        &[
            ("t", 8.0, 42.88),
            ("r2", 8.0, 28.56),
            ("r3", 36.56, 0.0),
            ("r4", 36.56, 14.31),
        ],
    );
    check(
        "equal rows",
        &table(
            "height: 100px",
            &[row(1, 20, false), row(2, 20, true), row(3, 20, false)],
        ),
        &[("t", 8.0, 66.67), ("r1", 8.0, 33.33), ("r3", 41.33, 33.34)],
    );
}

#[test]
fn rows_that_are_all_collapsed_leave_no_height() {
    check(
        "all collapsed",
        &table("height: 100px", &[row(1, 40, true), row(2, 20, true)]),
        &[("t", 8.0, 0.0)],
    );
}

#[test]
fn nothing_is_shared_when_the_rows_fill_the_height() {
    // The table states 100px and the rows ask 100px: the collapsed row just
    // leaves.
    check(
        "80 collapsed, 20",
        &table("height: 100px", &[row(1, 80, true), row(2, 20, false)]),
        &[("t", 8.0, 20.0), ("r2", 8.0, 20.0)],
    );
    // The rows ask more than the table states.
    check(
        "80 collapsed, 30",
        &table("height: 100px", &[row(1, 80, true), row(2, 30, false)]),
        &[("t", 8.0, 30.0), ("r2", 8.0, 30.0)],
    );
    check(
        "height 50",
        &table("height: 50px", &[row(1, 40, true), row(2, 30, false)]),
        &[("t", 8.0, 30.0)],
    );
}

#[test]
fn the_spacing_of_the_collapsed_row_is_shared_around_too() {
    // The spacing between and around all the rows comes out of the room the
    // rows share, collapsed or not; the spacing of the row that collapsed goes
    // with it.
    check(
        "spacing 4",
        &table(
            "height: 100px; border-spacing: 4px",
            &[row(1, 40, true), row(2, 20, false)],
        ),
        &[("t", 8.0, 37.34), ("r1", 12.0, 0.0), ("r2", 12.0, 29.34)],
    );
    check(
        "spacing 4, nothing collapsed",
        &table(
            "height: 100px; border-spacing: 4px",
            &[row(1, 40, false), row(2, 20, false)],
        ),
        &[("t", 8.0, 100.0), ("r1", 12.0, 58.66), ("r2", 74.66, 29.34)],
    );
    check(
        "spacing 4, three rows",
        &table(
            "height: 100px; border-spacing: 4px",
            &[row(1, 40, true), row(2, 20, false), row(3, 10, false)],
        ),
        &[("t", 8.0, 48.0), ("r2", 12.0, 24.0), ("r3", 40.0, 12.0)],
    );
    // Only the vertical spacing counts.
    check(
        "spacing 6px 2px",
        &table(
            "height: 100px; border-spacing: 6px 2px",
            &[row(1, 40, true), row(2, 20, false)],
        ),
        &[("t", 8.0, 35.34), ("r2", 10.0, 31.34)],
    );
}

#[test]
fn a_min_height_is_shared_the_same_way() {
    check(
        "min-height 100",
        &table("min-height: 100px", &[row(1, 40, true), row(2, 20, false)]),
        &[("t", 8.0, 33.34), ("r2", 8.0, 33.34)],
    );
    check(
        "min-height 80",
        &table("min-height: 80px", &[row(1, 40, true), row(2, 20, false)]),
        &[("t", 8.0, 26.67)],
    );
    // The taller of the height and the `min-height` is shared.
    check(
        "height 100, min-height 150",
        &table(
            "height: 100px; min-height: 150px",
            &[row(1, 40, true), row(2, 20, false)],
        ),
        &[("t", 8.0, 50.0)],
    );
    // A `max-height` caps the room the rows share.
    check(
        "height 100, max-height 30",
        &table(
            "height: 100px; max-height: 30px",
            &[row(1, 40, true), row(2, 20, false)],
        ),
        &[("t", 8.0, 20.0), ("r2", 8.0, 20.0)],
    );
}

#[test]
fn the_height_of_the_table_is_its_border_box() {
    check(
        "padding and border",
        &table(
            "height: 100px; padding: 5px; border: 3px solid",
            &[row(1, 40, true), row(2, 20, false)],
        ),
        &[("t", 8.0, 44.0), ("r2", 16.0, 28.0)],
    );
    check(
        "collapsed borders",
        &table(
            "height: 100px; border-collapse: collapse",
            &[row(1, 40, true), row(2, 20, false)],
        ),
        &[("t", 8.0, 33.34), ("r2", 8.0, 33.34)],
    );
    check(
        "borders on the cells and spacing",
        &format!(
            "<style>td {{ border: 2px solid }}</style>{}",
            table(
                "height: 100px; border-spacing: 3px",
                &[row(1, 40, true), row(2, 20, false), row(3, 10, false)],
            )
        ),
        &[("t", 8.0, 49.78), ("r2", 11.0, 25.75), ("r3", 39.75, 15.03)],
    );
}

#[test]
fn a_percentage_height_of_the_containing_block_is_shared_too() {
    check(
        "height 100%",
        &format!(
            "<div style='height: 100px'>{}</div>",
            table("height: 100%", &[row(1, 40, true), row(2, 20, false)])
        ),
        &[("t", 8.0, 33.34), ("r2", 8.0, 33.34)],
    );
}

#[test]
fn row_groups_share_the_extra_with_the_collapsed_row() {
    // The header and the footer keep the height of their rows while the body
    // can take the extra, as when nothing collapses; the collapsed row of the
    // header is one of those rows.
    check(
        "thead collapsed",
        "<table id=t style='height: 100px'>\
         <thead><tr id=r1 style='visibility: collapse'><td style='height: 30px'></td></tr></thead>\
         <tbody><tr id=r2><td style='height: 20px'></td></tr></tbody>\
         <tfoot><tr id=r3><td style='height: 10px'></td></tr></tfoot></table>",
        &[("t", 8.0, 70.0), ("r2", 8.0, 60.0), ("r3", 68.0, 10.0)],
    );
    check(
        "tbody collapsed",
        "<table id=t style='height: 100px'>\
         <thead><tr id=r1><td style='height: 10px'></td></tr></thead>\
         <tbody><tr id=r2 style='visibility: collapse'><td style='height: 30px'></td></tr>\
         <tr id=r3><td style='height: 20px'></td></tr></tbody></table>",
        &[("t", 8.0, 46.0), ("r1", 8.0, 10.0), ("r3", 18.0, 36.0)],
    );
    // The bodies share it in proportion to their heights, a collapsed one
    // included.
    check(
        "three bodies",
        "<table id=t style='height: 100px'>\
         <tbody><tr id=r1 style='visibility: collapse'><td style='height: 30px'></td></tr></tbody>\
         <tbody><tr id=r2><td style='height: 20px'></td></tr></tbody>\
         <tbody><tr id=r3><td style='height: 10px'></td></tr></tbody></table>",
        &[("t", 8.0, 50.0), ("r2", 8.0, 33.33), ("r3", 41.33, 16.67)],
    );
    check(
        "collapsed body",
        "<table id=t style='height: 100px'>\
         <tbody style='visibility: collapse'><tr><td style='height: 30px'></td></tr>\
         <tr><td style='height: 30px'></td></tr></tbody>\
         <tbody><tr id=r3><td style='height: 20px'></td></tr></tbody></table>",
        &[("t", 8.0, 25.0), ("r3", 8.0, 25.0)],
    );
    // The spacing inside a body counts the collapsed row.
    check(
        "spacing and two bodies",
        "<table id=t style='height: 100px; border-spacing: 4px'>\
         <tbody><tr id=r1 style='visibility: collapse'><td style='height: 30px'></td></tr>\
         <tr id=r2><td style='height: 20px'></td></tr>\
         <tr id=r3><td style='height: 10px'></td></tr></tbody>\
         <tbody><tr id=r4><td style='height: 10px'></td></tr></tbody></table>",
        &[
            ("t", 8.0, 61.66),
            ("r2", 12.0, 22.89),
            ("r3", 38.89, 11.47),
            ("r4", 54.36, 11.3),
        ],
    );
}

#[test]
fn percentage_and_stated_heights_of_rows_take_part() {
    // The row of 50% grows to half of the 100px the table states, and nothing
    // is left for the rest.
    check(
        "50% row",
        "<table id=t style='height: 100px'>\
         <tr id=r1 style='visibility: collapse'><td style='height: 40px'></td></tr>\
         <tr id=r2 style='height: 50%'><td></td></tr>\
         <tr id=r3><td style='height: 10px'></td></tr></table>",
        &[("t", 8.0, 60.0), ("r2", 8.0, 50.0), ("r3", 58.0, 10.0)],
    );
    // The collapsed row is the one of 50%: it grows to 50, and the rest is
    // shared by all the rows in proportion.
    check(
        "50% collapsed row",
        "<table id=t style='height: 100px'>\
         <tr id=r1 style='visibility: collapse; height: 50%'><td style='height: 10px'></td></tr>\
         <tr id=r2><td style='height: 20px'></td></tr>\
         <tr id=r3><td style='height: 10px'></td></tr></table>",
        &[("t", 8.0, 37.5), ("r2", 8.0, 25.0), ("r3", 33.0, 12.5)],
    );
    // Rows with a stated height keep it while another row can take the extra.
    check(
        "stated heights",
        "<table id=t style='height: 100px'>\
         <tr id=r1 style='visibility: collapse'><td style='height: 40px'></td></tr>\
         <tr id=r2><td>X</td></tr>\
         <tr id=r3><td style='height: 20px'></td></tr></table>",
        &[("t", 8.0, 60.0), ("r2", 8.0, 40.0), ("r3", 48.0, 20.0)],
    );
    // The collapsed row is the only one that can take it.
    check(
        "collapsed row takes it",
        "<table id=t style='height: 100px'>\
         <tr id=r1 style='visibility: collapse'><td>XX</td></tr>\
         <tr id=r2><td style='height: 20px'></td></tr></table>",
        &[("t", 8.0, 20.0), ("r2", 8.0, 20.0)],
    );
    check(
        "text",
        "<table id=t style='height: 100px'>\
         <tr id=r1 style='visibility: collapse'><td>XXX<br>XXX<br>XXX</td></tr>\
         <tr id=r2><td>X</td></tr></table>",
        &[("t", 8.0, 25.0), ("r2", 8.0, 25.0)],
    );
}

#[test]
fn a_cell_that_spans_the_collapsed_row_counts_in_its_rows() {
    // The cell of 30px spans a collapsed row of 10 and a row of 20: the rows
    // share the 70px that remain in proportion to 10 and 20.
    check(
        "rowspan",
        "<table id=t style='height: 100px'>\
         <tr id=r1 style='visibility: collapse'>\
         <td rowspan=2 style='height: 30px'></td><td style='height: 10px'></td></tr>\
         <tr id=r2><td style='height: 20px'></td></tr></table>",
        &[("t", 8.0, 66.67), ("r2", 8.0, 66.67)],
    );
}

#[test]
fn a_flex_item_keeps_the_height_its_container_gives_it() {
    // A column flex container grows the table to 100px, and the rows share
    // that with the collapsed one as they do in a table that states it.
    check(
        "flex-grow",
        &format!(
            "<div style='display: flex; flex-direction: column; height: 100px'>{}</div>",
            table("flex: 1", &[row(1, 40, true), row(2, 20, false)])
        ),
        &[("t", 8.0, 100.0), ("r2", 8.0, 33.34)],
    );
    check(
        "flex-grow, nothing collapsed",
        &format!(
            "<div style='display: flex; flex-direction: column; height: 100px'>{}</div>",
            table("flex: 1", &[row(1, 40, false), row(2, 20, false)])
        ),
        &[("t", 8.0, 100.0), ("r1", 8.0, 66.66), ("r2", 74.66, 33.34)],
    );
    // Without a height to share the collapsed row just leaves.
    check(
        "no growing",
        &format!(
            "<div style='display: flex; flex-direction: column; height: 100px'>{}</div>",
            table("", &[row(1, 40, true), row(2, 20, false)])
        ),
        &[("t", 8.0, 20.0)],
    );
}

#[test]
fn the_rows_of_a_stretched_table_share_with_the_collapsed_row() {
    // `height: stretch` hands the table the 200px of its container, which the
    // rows share with the collapsed one before it leaves. Only the rows are
    // checked: the box of a stretched table keeps its height.
    check(
        "stretch",
        &format!(
            "<div style='height: 200px'>{}</div>",
            table("height: stretch", &[row(1, 40, true), row(2, 20, false)])
        ),
        &[("r1", 8.0, 0.0), ("r2", 8.0, 66.67)],
    );
}
