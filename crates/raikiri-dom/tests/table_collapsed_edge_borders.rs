//! In the collapsing border model the border of a cell at the edge of the
//! table is the border of the table there: the width the two have together is
//! the larger of them, not their sum (CSS 2.1 §17.6.2). The table takes room
//! outside its edge cells only for the part of its own border that reaches
//! beyond theirs, so a table of bordered cells is as wide and as high as its
//! cells with their borders, and their content sits one border width inside it.
//!
//! The expected boxes are the ones Chromium computes for the same markup in an
//! 800px by 600px viewport, with the body margin of 8px and a font of 10px
//! Ahem: each is an element's left, top, width and height. The boxes of the
//! cells themselves are not compared: Chromium gives each cell half of the
//! border on every side, where a cell here contains the whole of it.

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

/// A label, the markup, and the boxes Chromium gives for some of its elements.
struct Case {
    label: &'static str,
    body: String,
    expected: &'static [(&'static str, [f32; 4])],
}

fn check(cases: &[Case]) {
    for case in cases {
        for (id, want) in case.expected {
            let found = border_box(&case.body, id);
            assert!(
                found
                    .iter()
                    .zip(want)
                    .all(|(got, want)| (got - want).abs() <= TOLERANCE),
                "{}: #{id} is {found:?}, expected {want:?} (left, top, width, height)",
                case.label
            );
        }
    }
}

/// A collapsing table in a row, whose `cells` each hold a `20px` by `10px`
/// box `i0`, `i1`, ...
fn row_of_cells(table_style: &str, cell_style: &str, cells: usize) -> String {
    let mut body = format!(
        "<div id=t style=\"display:table;border-spacing:0;border-collapse:collapse;\
         box-sizing:content-box;{table_style}\"><div style=\"display:table-row\">"
    );
    for cell in 0..cells {
        body.push_str(&format!(
            "<div id=c{cell} style=\"display:table-cell;{cell_style}\">\
             <div id=i{cell} style=\"width:20px;height:10px\"></div></div>"
        ));
    }
    body.push_str("</div></div>");
    body
}

#[test]
fn the_border_of_an_edge_cell_is_not_counted_again_outside_it() {
    check(&[
        Case {
            label: "a 4px border on the cell",
            body: row_of_cells("", "border:4px solid #000", 1),
            expected: &[
                ("t", [8.0, 8.0, 28.0, 18.0]),
                ("i0", [12.0, 12.0, 20.0, 10.0]),
            ],
        },
        Case {
            label: "a 1px border on the cell",
            body: row_of_cells("", "border:1px solid #000", 1),
            expected: &[
                ("t", [8.0, 8.0, 22.0, 12.0]),
                ("i0", [9.0, 9.0, 20.0, 10.0]),
            ],
        },
        Case {
            label: "a 1px border and 3px of padding on the cell",
            body: row_of_cells("", "border:1px solid #000;padding:3px", 1),
            expected: &[
                ("t", [8.0, 8.0, 28.0, 18.0]),
                ("i0", [12.0, 12.0, 20.0, 10.0]),
            ],
        },
        Case {
            label: "a 3px border and 3px of padding on the cell",
            body: row_of_cells("", "border:3px solid #000;padding:3px", 1),
            expected: &[
                ("t", [8.0, 8.0, 32.0, 22.0]),
                ("i0", [14.0, 14.0, 20.0, 10.0]),
            ],
        },
        Case {
            label: "two cells with a 4px border share the line between them",
            body: row_of_cells("", "border:4px solid #000", 2),
            expected: &[
                ("t", [8.0, 8.0, 52.0, 18.0]),
                ("i0", [12.0, 12.0, 20.0, 10.0]),
                ("i1", [36.0, 12.0, 20.0, 10.0]),
            ],
        },
    ]);
}

#[test]
fn the_border_of_the_table_is_outside_the_cells_only_beyond_theirs() {
    check(&[
        Case {
            label: "a 4px border on the table",
            body: row_of_cells("border:4px solid #00f", "", 1),
            expected: &[
                ("t", [8.0, 8.0, 28.0, 18.0]),
                ("i0", [12.0, 12.0, 20.0, 10.0]),
            ],
        },
        Case {
            label: "4px on the table and 4px on the cell",
            body: row_of_cells("border:4px solid #00f", "border:4px solid #000", 1),
            expected: &[
                ("t", [8.0, 8.0, 28.0, 18.0]),
                ("i0", [12.0, 12.0, 20.0, 10.0]),
            ],
        },
        Case {
            label: "2px on the table and 4px on the cell, which is the wider",
            body: row_of_cells("border:2px solid #00f", "border:4px solid #000", 1),
            expected: &[
                ("t", [8.0, 8.0, 28.0, 18.0]),
                ("i0", [12.0, 12.0, 20.0, 10.0]),
            ],
        },
        Case {
            label: "4px on the table and 2px on the cell, which is the narrower",
            body: row_of_cells("border:4px solid #00f", "border:2px solid #000", 1),
            expected: &[
                ("t", [8.0, 8.0, 28.0, 18.0]),
                ("i0", [12.0, 12.0, 20.0, 10.0]),
            ],
        },
        Case {
            label: "a 4px border on the table, two cells without one",
            body: row_of_cells("border:4px solid #00f", "", 2),
            expected: &[
                ("t", [8.0, 8.0, 48.0, 18.0]),
                ("i0", [12.0, 12.0, 20.0, 10.0]),
                ("i1", [32.0, 12.0, 20.0, 10.0]),
            ],
        },
    ]);
}

/// A collapsing table of `cells` columns and `rows` rows, in a block of 300px,
/// with a `200px` box in each cell, in the border-box sizing a `table` element
/// has.
fn grid_of_cells(table_style: &str, cell_style: &str, cells: usize, rows: usize) -> String {
    let mut body = format!(
        "<div style=\"width:300px\"><div id=t style=\"display:table;border-spacing:0;\
         border-collapse:collapse;{table_style}\">"
    );
    for _ in 0..rows {
        body.push_str("<div style=\"display:table-row\">");
        for _ in 0..cells {
            body.push_str(&format!(
                "<div style=\"display:table-cell;{cell_style}\">\
                 <div style=\"width:200px;height:10px\"></div></div>"
            ));
        }
        body.push_str("</div>");
    }
    body.push_str("</div></div>");
    body
}

#[test]
fn a_table_that_states_its_size_keeps_it_with_the_borders_inside() {
    check(&[
        Case {
            label: "width 100, a 2px border on the table",
            body: grid_of_cells(
                "table-layout:fixed;width:100px;border:2px solid #00f",
                "",
                2,
                1,
            ),
            expected: &[("t", [8.0, 8.0, 100.0, 14.0])],
        },
        Case {
            label: "width 100, a 3px border on the table",
            body: grid_of_cells(
                "table-layout:fixed;width:100px;border:3px solid #00f",
                "",
                2,
                1,
            ),
            expected: &[("t", [8.0, 8.0, 100.0, 16.0])],
        },
        Case {
            label: "width 100, borders of 2px on the table and the cells",
            body: grid_of_cells(
                "table-layout:fixed;width:100px;border:2px solid #00f",
                "border:2px solid #000",
                2,
                1,
            ),
            expected: &[("t", [8.0, 8.0, 100.0, 14.0])],
        },
        Case {
            label: "width 100, 6px on the table and 2px on the cells",
            body: grid_of_cells(
                "table-layout:fixed;width:100px;border:6px solid #00f",
                "border:2px solid #000",
                2,
                1,
            ),
            expected: &[("t", [8.0, 8.0, 100.0, 22.0])],
        },
        Case {
            label: "width 100, 2px on the table and 6px on the cells",
            body: grid_of_cells(
                "table-layout:fixed;width:100px;border:2px solid #00f",
                "border:6px solid #000",
                2,
                1,
            ),
            expected: &[("t", [8.0, 8.0, 100.0, 22.0])],
        },
        Case {
            label: "width 100, 5px on the table and 3px on the cells",
            body: grid_of_cells(
                "table-layout:fixed;width:100px;border:5px solid #00f",
                "border:3px solid #000",
                2,
                1,
            ),
            expected: &[("t", [8.0, 8.0, 100.0, 20.0])],
        },
        Case {
            label: "width 100, a different border on every side of the table",
            body: grid_of_cells(
                "table-layout:fixed;width:100px;border-width:2px 4px 6px 8px;\
                 border-style:solid;border-color:#00f",
                "",
                2,
                1,
            ),
            expected: &[("t", [8.0, 8.0, 100.0, 18.0])],
        },
        Case {
            label: "width and height 100, a 2px border on the table",
            body: grid_of_cells(
                "table-layout:fixed;width:100px;height:100px;border:2px solid #00f",
                "",
                2,
                1,
            ),
            expected: &[("t", [8.0, 8.0, 100.0, 100.0])],
        },
        Case {
            label: "width 80, 4px borders on two rows of two cells",
            body: grid_of_cells(
                "table-layout:fixed;width:80px",
                "border:4px solid #000;height:10px",
                2,
                2,
            ),
            expected: &[("t", [8.0, 8.0, 80.0, 32.0])],
        },
        Case {
            label: "automatic layout, a 2px border on the table",
            body: grid_of_cells("width:100px;border:2px solid #00f", "", 2, 1),
            expected: &[("t", [8.0, 8.0, 404.0, 14.0])],
        },
        Case {
            label: "automatic layout, 2px on the table and on the cells",
            body: grid_of_cells("border:2px solid #00f", "border:2px solid #000", 2, 1),
            expected: &[("t", [8.0, 8.0, 406.0, 14.0])],
        },
    ]);
}
