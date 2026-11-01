//! The columns of a table in a vertical writing mode stack along its physical
//! height. Each column is as high as the cells that start in it ask; a cell
//! that spans several columns shares its height among them instead of asking
//! it of each one; a table with a height of its own shares that height among
//! its columns in proportion to what they ask.
//!
//! The expected boxes are the ones Chromium computes for the same markup in an
//! 800px by 600px viewport, with the body margin of 8px and a font of 10px
//! Ahem: each is an element's left, top, width and height. Where the rows of a
//! table are several, only the top and the height are compared: how wide the
//! rows are is a separate matter.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The left, top, width and height of the element `id` in `body`, on the first
/// page of 800px by 600px.
fn border_box(body: &str, id: &str) -> [f32; 4] {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
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
            [rect.x, rect.y, rect.width, rect.height]
        })
        .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"))
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.04;

fn close(found: f32, want: f32) -> bool {
    (found - want).abs() <= TOLERANCE
}

/// Compares the boxes of the elements `expected` names, reporting every
/// mismatch at once.
fn check(label: &str, markup: &str, expected: &[(&str, [f32; 4])]) -> Vec<String> {
    let mut mismatches = Vec::new();
    for (id, want) in expected {
        let found = border_box(markup, id);
        if !found.iter().zip(want).all(|(&got, &want)| close(got, want)) {
            mismatches.push(format!(
                "{label}: #{id} is {found:?}, expected {want:?} (left, top, width, height)"
            ));
        }
    }
    mismatches
}

/// Compares the top and the height of the elements `expected` names.
fn check_extent(label: &str, markup: &str, expected: &[(&str, f32, f32)]) -> Vec<String> {
    let mut mismatches = Vec::new();
    for &(id, top, height) in expected {
        let found = border_box(markup, id);
        if !close(found[1], top) || !close(found[3], height) {
            mismatches.push(format!(
                "{label}: #{id} is at {} and {} high, expected {top} and {height}",
                found[1], found[3]
            ));
        }
    }
    mismatches
}

const MODES: [&str; 2] = ["vertical-rl", "vertical-lr"];

/// A `<table>` in `mode` with the rows `rows`, whose cells have no padding
/// unless they say so.
fn table_element(mode: &str, spacing: &str, rows: &str) -> String {
    format!("<table id=t style='writing-mode: {mode}; border-spacing: {spacing}'>{rows}</table>")
}

/// A cell `id` of the width and height, holding `text`.
fn td(id: &str, attrs: &str, size: [u32; 2], padding: &str, text: &str) -> String {
    format!(
        "<td id={id} {attrs} style='width: {}px; height: {}px; padding: {padding}'>{text}</td>",
        size[0], size[1]
    )
}

/// A table of `display: table` boxes in `mode` with the style `style`, holding
/// one row of cells of the sizes `cells`: the cells `c` and `d`, each with
/// a block `pc` and `pd` of text.
fn table_boxes(mode: &str, style: &str, cells: &[(&str, [u32; 2])]) -> String {
    let cells: String = cells
        .iter()
        .map(|(id, size)| {
            format!(
                "<div id={id} style='display: table-cell; width: {}px; height: {}px'>\
                 <div id=p{id}>x</div></div>",
                size[0], size[1]
            )
        })
        .collect();
    format!(
        "<div id=t style='display: table; writing-mode: {mode}; {style}'>\
         <div id=r style='display: table-row'>{cells}</div></div>"
    )
}

#[test]
fn a_cell_spanning_columns_keeps_the_height_it_states() {
    let mut mismatches = Vec::new();
    for mode in MODES {
        // The height of 80px is shared by two columns, and 60px by three.
        let two = table_element(
            mode,
            "0",
            &format!(
                "<tr id=r>{}</tr>",
                td("c", "colspan=2", [60, 80], "0", "<div id=p>x</div>")
            ),
        );
        mismatches.extend(check(
            &format!("{mode}, two columns"),
            &two,
            &[
                ("t", [8.0, 8.0, 60.0, 80.0]),
                ("c", [8.0, 8.0, 60.0, 80.0]),
                ("p", [33.0, 8.0, 10.0, 80.0]),
            ],
        ));
        let three = table_element(
            mode,
            "0",
            &format!(
                "<tr id=r>{}</tr>",
                td("c", "colspan=3", [90, 60], "0", "<div id=p>x</div>")
            ),
        );
        mismatches.extend(check(
            &format!("{mode}, three columns"),
            &three,
            &[
                ("t", [8.0, 8.0, 90.0, 60.0]),
                ("c", [8.0, 8.0, 90.0, 60.0]),
                ("p", [48.0, 8.0, 10.0, 60.0]),
            ],
        ));
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn a_cell_spanning_columns_leaves_out_the_spacing_padding_and_neighbours() {
    let mut mismatches = Vec::new();
    for mode in MODES {
        // A cell of 30px beside the spanned columns adds a column of its own.
        let beside = table_element(
            mode,
            "0",
            &format!(
                "<tr id=r>{}{}</tr>",
                td("c", "colspan=2", [60, 80], "0", "x"),
                td("d", "", [40, 30], "0", "y")
            ),
        );
        mismatches.extend(check(
            &format!("{mode}, a cell beside it"),
            &beside,
            &[
                ("t", [8.0, 8.0, 60.0, 110.0]),
                ("c", [8.0, 8.0, 60.0, 80.0]),
                ("d", [8.0, 88.0, 60.0, 30.0]),
            ],
        ));
        // The spacing between the two columns is not part of the 80px, and
        // the spacing around them is.
        let spaced = table_element(
            mode,
            "6px",
            &format!("<tr id=r>{}</tr>", td("c", "colspan=2", [60, 80], "0", "x")),
        );
        mismatches.extend(check(
            &format!("{mode}, spacing"),
            &spaced,
            &[
                ("t", [8.0, 8.0, 72.0, 92.0]),
                ("c", [14.0, 14.0, 60.0, 80.0]),
            ],
        ));
        // The first of two spacings is along the inline axis, the height of
        // the table: it is the gap between the columns and around them.
        let uneven = table_element(
            mode,
            "6px 4px",
            &format!(
                "<tr id=r>{}{}</tr>",
                td("c", "colspan=2", [60, 80], "0", "x"),
                td("d", "", [40, 30], "0", "y")
            ),
        );
        mismatches.extend(check(
            &format!("{mode}, uneven spacing"),
            &uneven,
            &[
                ("t", [8.0, 8.0, 68.0, 128.0]),
                ("c", [12.0, 14.0, 60.0, 80.0]),
                ("d", [12.0, 100.0, 60.0, 30.0]),
            ],
        ));
        // The padding of the cell is added to its size once.
        let padded = table_element(
            mode,
            "0",
            &format!(
                "<tr id=r>{}</tr>",
                td("c", "colspan=2", [60, 80], "3px 5px", "x")
            ),
        );
        mismatches.extend(check(
            &format!("{mode}, padding"),
            &padded,
            &[("t", [8.0, 8.0, 70.0, 86.0]), ("c", [8.0, 8.0, 70.0, 86.0])],
        ));
        // A cell with no size of its own is as high as its text.
        let text = table_element(
            mode,
            "0",
            "<tr id=r><td id=c colspan=2 style='padding: 0'>xxxxxxxx</td></tr>",
        );
        mismatches.extend(check(
            &format!("{mode}, text only"),
            &text,
            &[("t", [8.0, 8.0, 10.0, 80.0]), ("c", [8.0, 8.0, 10.0, 80.0])],
        ));
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn a_table_with_several_rows_stacks_the_tallest_cell_of_each_column() {
    let mut mismatches = Vec::new();
    for mode in MODES {
        // Two columns: 60px by 50px and 40px by 50px in the first, 30px by
        // 20px in the second.
        let columns = format!(
            "<div id=t style='display: table; writing-mode: {mode}'>\
             <div id=r1 style='display: table-row'>\
             <div id=c1 style='display: table-cell; width: 60px; height: 50px'>\
             <div id=p>x</div></div>\
             <div id=c3 style='display: table-cell; width: 30px; height: 20px'>\
             <div id=p3>z</div></div></div>\
             <div id=r2 style='display: table-row'>\
             <div id=c2 style='display: table-cell; width: 40px; height: 50px'>\
             <div id=q>y</div></div></div></div>"
        );
        mismatches.extend(check_extent(
            &format!("{mode}, columns"),
            &columns,
            &[
                ("t", 8.0, 70.0),
                ("c1", 8.0, 50.0),
                ("c2", 8.0, 50.0),
                ("c3", 58.0, 20.0),
            ],
        ));
        // The cell of the second row spans both columns, which are 20px and
        // 30px high without it: it takes the 40px it lacks in proportion.
        let grown = table_element(
            mode,
            "0",
            &format!(
                "<tr id=r>{}{}</tr><tr id=r2>{}</tr>",
                td("c", "", [60, 20], "0", "x"),
                td("e", "", [60, 30], "0", "z"),
                td("d", "colspan=2", [40, 90], "0", "y")
            ),
        );
        mismatches.extend(check_extent(
            &format!("{mode}, a spanning cell asks more"),
            &grown,
            &[
                ("t", 8.0, 90.0),
                ("c", 8.0, 36.0),
                ("e", 44.0, 54.0),
                ("d", 8.0, 90.0),
            ],
        ));
        // It asks no more than they have: the columns stay as they are.
        let fit = table_element(
            mode,
            "0",
            &format!(
                "<tr id=r>{}{}</tr><tr id=r2>{}</tr>",
                td("c", "", [60, 50], "0", "x"),
                td("e", "", [60, 50], "0", "z"),
                td("d", "colspan=2", [40, 30], "0", "y")
            ),
        );
        mismatches.extend(check_extent(
            &format!("{mode}, a spanning cell asks less"),
            &fit,
            &[
                ("t", 8.0, 100.0),
                ("c", 8.0, 50.0),
                ("e", 58.0, 50.0),
                ("d", 8.0, 100.0),
            ],
        ));
        // A cell that spans two rows stays in its column: the columns are
        // as high as the cells in them, whatever the rows are.
        for (label, tall, wide, expected) in [
            (
                "a cell spanning rows",
                [60, 80],
                [40, 30],
                [("t", 8.0, 110.0), ("c", 8.0, 80.0), ("d", 88.0, 30.0)],
            ),
            (
                "a short cell spanning rows",
                [60, 20],
                [40, 50],
                [("t", 8.0, 70.0), ("c", 8.0, 20.0), ("d", 28.0, 50.0)],
            ),
        ] {
            let rows = table_element(
                mode,
                "0",
                &format!(
                    "<tr id=r>{}{}</tr><tr>{}</tr>",
                    td("c", "rowspan=2", tall, "0", "x"),
                    td("d", "", wide, "0", "y"),
                    td("e", "", wide, "0", "z")
                ),
            );
            let mut expected = expected.to_vec();
            expected.push(("e", expected[2].1, expected[2].2));
            mismatches.extend(check_extent(&format!("{mode}, {label}"), &rows, &expected));
        }
        // A spanning cell inside another: the narrower one settles first.
        let nested = table_element(
            mode,
            "0",
            &format!(
                "<tr id=r>{}{}{}</tr><tr id=r2>{}{}</tr><tr id=r3>{}</tr>",
                td("c", "", [60, 10], "0", "x"),
                td("e", "", [60, 10], "0", "z"),
                td("f", "", [60, 10], "0", "w"),
                td("d", "colspan=2", [40, 50], "0", "y"),
                td("g", "", [40, 10], "0", "v"),
                td("h", "colspan=3", [40, 90], "0", "u")
            ),
        );
        mismatches.extend(check_extent(
            &format!("{mode}, spanning cells inside each other"),
            &nested,
            &[
                ("t", 8.0, 90.0),
                ("c", 8.0, 37.5),
                ("e", 45.5, 37.5),
                ("f", 83.0, 15.0),
                ("d", 8.0, 75.0),
                ("g", 83.0, 15.0),
                ("h", 8.0, 90.0),
            ],
        ));
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn a_table_height_is_shared_by_the_columns_in_proportion_to_their_heights() {
    let cells = [("c", [60, 50]), ("d", [60, 30])];
    let mut mismatches = Vec::new();
    for mode in MODES {
        // 200px for columns of 50px and 30px: 125px and 75px.
        for (label, style) in [
            ("height", "height: 200px"),
            ("min-height", "min-height: 200px"),
        ] {
            mismatches.extend(check(
                &format!("{mode}, {label}"),
                &table_boxes(mode, style, &cells),
                &[
                    ("t", [8.0, 8.0, 60.0, 200.0]),
                    ("c", [8.0, 8.0, 60.0, 125.0]),
                    ("d", [8.0, 133.0, 60.0, 75.0]),
                ],
            ));
        }
        // The spacing around and between the columns is part of the 200px.
        mismatches.extend(check(
            &format!("{mode}, height with spacing"),
            &table_boxes(mode, "height: 200px; border-spacing: 6px", &cells),
            &[
                ("t", [8.0, 8.0, 72.0, 200.0]),
                ("c", [14.0, 14.0, 60.0, 113.75]),
                ("d", [14.0, 133.75, 60.0, 68.25]),
            ],
        ));
        // Three columns: the spacing between them and around them is part of
        // the 200px, and the padding and the border of the table are not.
        let three = [("c", [60, 50]), ("d", [60, 30]), ("e", [60, 20])];
        mismatches.extend(check(
            &format!("{mode}, three columns with spacing"),
            &table_boxes(mode, "height: 200px; border-spacing: 6px", &three),
            &[
                ("t", [8.0, 8.0, 72.0, 200.0]),
                ("c", [14.0, 14.0, 60.0, 88.0]),
                ("d", [14.0, 108.0, 60.0, 52.8]),
                ("e", [14.0, 166.8, 60.0, 35.2]),
            ],
        ));
        mismatches.extend(check(
            &format!("{mode}, three columns with spacing, padding and border"),
            &table_boxes(
                mode,
                "height: 200px; border-spacing: 6px; padding: 3px; border: 2px solid black",
                &three,
            ),
            &[
                ("t", [8.0, 8.0, 82.0, 200.0]),
                ("c", [19.0, 19.0, 60.0, 83.0]),
                ("d", [19.0, 108.0, 60.0, 49.8]),
                ("e", [19.0, 163.8, 60.0, 33.2]),
            ],
        ));
        // The padding and the border of the table are outside the columns.
        mismatches.extend(check(
            &format!("{mode}, padding and border"),
            &table_boxes(mode, "padding: 3px; border: 2px solid black", &cells),
            &[
                ("t", [8.0, 8.0, 70.0, 90.0]),
                ("c", [13.0, 13.0, 60.0, 50.0]),
                ("d", [13.0, 63.0, 60.0, 30.0]),
            ],
        ));
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn the_height_of_a_row_does_not_stretch_the_columns() {
    let mut mismatches = Vec::new();
    for mode in MODES {
        let one = table_boxes(mode, "", &[("c", [60, 50])]).replace(
            "id=r style='display: table-row'",
            "id=r style='display: table-row; height: 100px'",
        );
        mismatches.extend(check(
            &format!("{mode}, one column"),
            &one,
            &[("t", [8.0, 8.0, 60.0, 50.0]), ("c", [8.0, 8.0, 60.0, 50.0])],
        ));
        let two = table_boxes(mode, "", &[("c", [60, 50]), ("d", [60, 30])]).replace(
            "id=r style='display: table-row'",
            "id=r style='display: table-row; height: 100px'",
        );
        mismatches.extend(check(
            &format!("{mode}, two columns"),
            &two,
            &[
                ("t", [8.0, 8.0, 60.0, 80.0]),
                ("c", [8.0, 8.0, 60.0, 50.0]),
                ("d", [8.0, 58.0, 60.0, 30.0]),
            ],
        ));
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

/// A table in `mode` with one row of a vertical cell `c` (60px by 50px) and a
/// horizontal cell `d` (60px wide, `height` high) aligned by `align` and holding
/// `content`, in the order the cells are in `first`.
fn mixed_row(mode: &str, first: &str, height: u32, align: &str, content: &str) -> String {
    let vertical = "<div id=c style='display: table-cell; width: 60px; height: 50px'>\
                    <div id=pc>x</div></div>";
    let horizontal = format!(
        "<div id=d style='display: table-cell; writing-mode: horizontal-tb; width: 60px; \
         height: {height}px; vertical-align: {align}'>{content}</div>"
    );
    let (one, two) = if first == "d" {
        (horizontal.as_str(), vertical)
    } else {
        (vertical, horizontal.as_str())
    };
    format!(
        "<div id=t style='display: table; writing-mode: {mode}'>\
         <div id=r style='display: table-row'>{one}{two}</div></div>"
    )
}

#[test]
fn a_horizontal_cell_in_a_vertical_table_aligns_in_the_height_of_its_column() {
    let square = "<div id=pd style='width: 20px; height: 20px'></div>";
    let mut mismatches = Vec::new();
    for mode in MODES {
        // The column of `d` is 30px high next to a column of 50px: the room
        // around a block of 20px is 10px, not the 30px of the row.
        for (align, top) in [
            ("baseline", 58.0),
            ("top", 58.0),
            ("middle", 63.0),
            ("bottom", 68.0),
        ] {
            mismatches.extend(check(
                &format!("{mode}, {align}"),
                &mixed_row(mode, "c", 30, align, square),
                &[
                    ("t", [8.0, 8.0, 60.0, 80.0]),
                    ("d", [8.0, 58.0, 60.0, 30.0]),
                    ("pd", [8.0, top, 20.0, 20.0]),
                ],
            ));
        }
        // Content higher than the cell makes the column as high as itself: no
        // room is left to spread, whichever side it is aligned to.
        let tall = "<div id=pd style='width: 20px; height: 40px'></div>";
        for align in ["middle", "bottom"] {
            mismatches.extend(check(
                &format!("{mode}, content of 40px, {align}"),
                &mixed_row(mode, "c", 30, align, tall),
                &[
                    ("t", [8.0, 8.0, 60.0, 90.0]),
                    ("d", [8.0, 58.0, 60.0, 40.0]),
                    ("pd", [8.0, 58.0, 20.0, 40.0]),
                ],
            ));
        }
        // A cell with nothing in it keeps its column, aligned by any side.
        for align in ["baseline", "middle"] {
            mismatches.extend(check(
                &format!("{mode}, empty, {align}"),
                &mixed_row(mode, "c", 30, align, ""),
                &[
                    ("t", [8.0, 8.0, 60.0, 80.0]),
                    ("d", [8.0, 58.0, 60.0, 30.0]),
                ],
            ));
        }
        // It comes first: its column is the first 30px.
        mismatches.extend(check(
            &format!("{mode}, first"),
            &mixed_row(mode, "d", 30, "middle", square),
            &[
                ("d", [8.0, 8.0, 60.0, 30.0]),
                ("pd", [8.0, 13.0, 20.0, 20.0]),
                ("c", [8.0, 38.0, 60.0, 50.0]),
            ],
        ));
        // Text: its line is 10px high.
        mismatches.extend(check(
            &format!("{mode}, text"),
            &mixed_row(mode, "c", 30, "middle", "<div id=pd>x</div>"),
            &[("pd", [8.0, 68.0, 60.0, 10.0])],
        ));
        // Alone, its column is as high as the row and nothing changes.
        let alone = format!(
            "<div id=t style='display: table; writing-mode: {mode}'>\
             <div id=r style='display: table-row'>\
             <div id=d style='display: table-cell; writing-mode: horizontal-tb; width: 60px; \
             height: 50px; vertical-align: middle'>{square}</div></div></div>"
        );
        mismatches.extend(check(
            &format!("{mode}, alone"),
            &alone,
            &[
                ("d", [8.0, 8.0, 60.0, 50.0]),
                ("pd", [8.0, 23.0, 20.0, 20.0]),
            ],
        ));
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

/// Like [`mixed_row`], with the horizontal cell holding a block `pd` of 20px
/// that is `percent` high, and the table styled by `style`.
fn mixed_row_with_percent(
    mode: &str,
    first: &str,
    height: u32,
    align: &str,
    style: &str,
    percent: &str,
) -> String {
    mixed_row(
        mode,
        first,
        height,
        align,
        &format!("<div id=pd style='width: 20px; height: {percent}'></div>"),
    )
    .replacen("display: table;", &format!("display: table; {style};"), 1)
}

#[test]
fn a_horizontal_cell_lays_out_percentage_heights_at_the_height_of_its_column() {
    let mut mismatches = Vec::new();
    for mode in MODES {
        // 50% of the 30px of the column, not of the 50px of the row.
        for (align, top) in [("top", 58.0), ("middle", 65.5), ("bottom", 73.0)] {
            mismatches.extend(check(
                &format!("{mode}, 50%, {align}"),
                &mixed_row_with_percent(mode, "c", 30, align, "", "50%"),
                &[
                    ("d", [8.0, 58.0, 60.0, 30.0]),
                    ("pd", [8.0, top, 20.0, 15.0]),
                ],
            ));
        }
        mismatches.extend(check(
            &format!("{mode}, 100%"),
            &mixed_row_with_percent(mode, "c", 30, "middle", "", "100%"),
            &[("pd", [8.0, 58.0, 20.0, 30.0])],
        ));
        mismatches.extend(check(
            &format!("{mode}, 50%, first"),
            &mixed_row_with_percent(mode, "d", 30, "middle", "", "50%"),
            &[
                ("d", [8.0, 8.0, 60.0, 30.0]),
                ("pd", [8.0, 15.5, 20.0, 15.0]),
                ("c", [8.0, 38.0, 60.0, 50.0]),
            ],
        ));
        // A table with a height shares it, 125px and 75px for the columns.
        for (align, top) in [("top", 133.0), ("middle", 151.75)] {
            mismatches.extend(check(
                &format!("{mode}, 50% in a table of 200px, {align}"),
                &mixed_row_with_percent(mode, "c", 30, align, "height: 200px", "50%"),
                &[
                    ("d", [8.0, 133.0, 60.0, 75.0]),
                    ("pd", [8.0, top, 20.0, 37.5]),
                ],
            ));
        }
        // A cell with no height of its own gives its child nothing to resolve
        // the percentage against.
        mismatches.extend(check(
            &format!("{mode}, 50% of a cell with no height"),
            &mixed_row_with_percent(mode, "c", 0, "middle", "", "50%"),
            &[
                ("t", [8.0, 8.0, 60.0, 50.0]),
                ("d", [8.0, 58.0, 60.0, 0.0]),
                ("pd", [8.0, 58.0, 20.0, 0.0]),
            ],
        ));
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}
