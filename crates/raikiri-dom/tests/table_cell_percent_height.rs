//! A `height: N%` on a child of a table cell resolves against the height of the
//! cell only when that height is definite: the table states a `height` (a
//! `min-height` or a height a flex or grid parent gives it does not count), or
//! the cell states one of its own (CSS 2.1 §17.5.3, §10.5). Otherwise the
//! percentage is `auto`, and the row height the cells share, which follows from
//! their content, is not a basis. The cell is still as high as its row.
//!
//! The expected boxes are the ones Chromium computes for the same markup in an
//! 800px by 600px viewport, with the body margin of 8px and a font of 10px
//! Ahem: each is an element's left, top, width and height.

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

/// A table of `rows` rows whose first row holds a cell `c` with a child `p`
/// of `height: 50%` and one line of text, and then `beside`. The `table`, `row`
/// and `cell` styles are added to those elements.
fn table(table: &str, row: &str, cell: &str, beside: &str, rows: usize) -> String {
    let more_rows = "<div style=\"display:table-row\"><div style=\"display:table-cell\">\
                     <div style=\"height:10px\"></div></div></div>"
        .repeat(rows.saturating_sub(1));
    format!(
        "<div id=t style=\"display:table;{table}\">\
         <div id=r style=\"display:table-row;{row}\">\
         <div id=c style=\"display:table-cell;{cell}\"><div id=p style=\"height:50%\">x</div></div>\
         {beside}</div>{more_rows}</div>"
    )
}

fn plain() -> String {
    table("", "", "", "", 1)
}

#[test]
fn a_percentage_height_is_auto_when_no_height_is_stated() {
    check(&[
        Case {
            label: "an auto table",
            body: plain(),
            expected: &[("c", [8.0, 8.0, 10.0, 10.0]), ("p", [8.0, 8.0, 10.0, 10.0])],
        },
        Case {
            label: "two rows",
            body: table("", "", "", "", 2),
            expected: &[("t", [8.0, 8.0, 10.0, 20.0]), ("p", [8.0, 8.0, 10.0, 10.0])],
        },
        // The row is as high as it says, and the cell fills it, but that
        // height follows from the row and not from the table or the cell.
        Case {
            label: "a row with a height",
            body: table("", "height:40px", "", "", 1),
            expected: &[("c", [8.0, 8.0, 10.0, 40.0]), ("p", [8.0, 8.0, 10.0, 10.0])],
        },
        Case {
            label: "a table with a min-height",
            body: table("min-height:100px", "", "", "", 1),
            expected: &[
                ("t", [8.0, 8.0, 10.0, 100.0]),
                ("c", [8.0, 8.0, 10.0, 100.0]),
                ("p", [8.0, 8.0, 10.0, 10.0]),
            ],
        },
        // A percentage height of the cell has no table height to resolve
        // against, so it is `auto` as well.
        Case {
            label: "a cell with a percentage height",
            body: table("", "", "height:50%", "", 1),
            expected: &[("c", [8.0, 8.0, 10.0, 10.0]), ("p", [8.0, 8.0, 10.0, 10.0])],
        },
        Case {
            label: "a table with a percentage height and no room to resolve it",
            body: table("height:100%", "", "", "", 1),
            expected: &[("t", [8.0, 8.0, 10.0, 10.0]), ("p", [8.0, 8.0, 10.0, 10.0])],
        },
        Case {
            label: "a floated table",
            body: table("float:left", "", "", "", 1),
            expected: &[("p", [8.0, 8.0, 10.0, 10.0])],
        },
        Case {
            label: "an inline table",
            body: table("display:inline-table", "", "", "", 1),
            expected: &[("p", [8.0, 8.0, 10.0, 10.0])],
        },
        Case {
            label: "an absolutely positioned table",
            body: table("position:absolute;top:0;left:0", "", "", "", 1),
            expected: &[("p", [0.0, 0.0, 10.0, 10.0])],
        },
        // The cell next to one that states a height does not get one.
        Case {
            label: "the cell beside a cell with a height",
            body: table(
                "",
                "",
                "height:40px",
                "<div id=d style=\"display:table-cell\"><div id=q style=\"height:50%\">y</div></div>",
                1,
            ),
            expected: &[
                ("c", [8.0, 8.0, 10.0, 40.0]),
                ("d", [18.0, 8.0, 10.0, 40.0]),
                ("q", [18.0, 8.0, 10.0, 10.0]),
            ],
        },
    ]);
}

#[test]
fn a_height_a_flex_or_grid_container_gives_the_table_is_not_a_basis() {
    let in_container = |container: &str, table_style: &str| {
        format!(
            "<div id=w style=\"{container};height:100px\">{}</div>",
            table(table_style, "", "", "", 1)
        )
    };
    check(&[
        Case {
            label: "a table stretched by a row flex container",
            body: in_container("display:flex", ""),
            expected: &[
                ("t", [8.0, 8.0, 10.0, 100.0]),
                ("c", [8.0, 8.0, 10.0, 100.0]),
                ("p", [8.0, 8.0, 10.0, 10.0]),
            ],
        },
        Case {
            label: "a table grown by a column flex container",
            body: in_container("display:flex;flex-direction:column", "flex:1 1 auto"),
            expected: &[
                ("t", [8.0, 8.0, 784.0, 100.0]),
                ("p", [8.0, 8.0, 784.0, 10.0]),
            ],
        },
        Case {
            label: "a table stretched by a grid container",
            body: in_container("display:grid", ""),
            expected: &[
                ("t", [8.0, 8.0, 784.0, 100.0]),
                ("p", [8.0, 8.0, 784.0, 10.0]),
            ],
        },
        // A height the table states itself is a basis, wherever it sits.
        Case {
            label: "a flex item with a height",
            body: in_container("display:flex", "height:80px"),
            expected: &[("t", [8.0, 8.0, 10.0, 80.0]), ("p", [8.0, 8.0, 10.0, 40.0])],
        },
    ]);
}

#[test]
fn a_percentage_height_resolves_against_a_stated_height() {
    check(&[
        Case {
            label: "a table with a height",
            body: table("height:100px", "", "", "", 1),
            expected: &[
                ("c", [8.0, 8.0, 10.0, 100.0]),
                ("p", [8.0, 8.0, 10.0, 50.0]),
            ],
        },
        Case {
            label: "a table with a calc height",
            body: table("height:calc(50px + 50px)", "", "", "", 1),
            expected: &[("p", [8.0, 8.0, 10.0, 50.0])],
        },
        Case {
            label: "a table with a percentage height that resolves",
            body: format!(
                "<div style=\"height:100px\">{}</div>",
                table("height:100%", "", "", "", 1)
            ),
            expected: &[
                ("t", [8.0, 8.0, 10.0, 100.0]),
                ("p", [8.0, 8.0, 10.0, 50.0]),
            ],
        },
        Case {
            label: "an absolutely positioned table with a height",
            body: table("position:absolute;top:0;left:0;height:100px", "", "", "", 1),
            expected: &[("p", [0.0, 0.0, 10.0, 50.0])],
        },
        // Every cell of the table has the height of its own row to resolve
        // against.
        Case {
            label: "two rows share the height",
            body: table("height:100px", "", "", "", 2),
            expected: &[("r", [8.0, 8.0, 10.0, 50.0]), ("p", [8.0, 8.0, 10.0, 25.0])],
        },
        Case {
            label: "a cell with a percentage height in a table with a height",
            body: table("height:100px", "", "height:80%", "", 1),
            expected: &[("p", [8.0, 8.0, 10.0, 50.0])],
        },
        Case {
            label: "a cell next to a taller cell",
            body: table(
                "height:100px",
                "",
                "",
                "<div id=d style=\"display:table-cell\"><div style=\"width:10px;height:60px\"></div></div>",
                1,
            ),
            expected: &[
                ("c", [8.0, 8.0, 10.0, 100.0]),
                ("p", [8.0, 60.0, 10.0, 50.0]),
            ],
        },
        Case {
            label: "a cell with a height of its own",
            body: table("", "", "height:40px", "", 1),
            expected: &[("c", [8.0, 8.0, 10.0, 40.0]), ("p", [8.0, 8.0, 10.0, 20.0])],
        },
        Case {
            label: "a cell with a calc height",
            body: table("", "", "height:calc(20px + 20px)", "", 1),
            expected: &[("p", [8.0, 8.0, 10.0, 20.0])],
        },
        // The percentage is of the cell's content box.
        Case {
            label: "a cell with a height and padding",
            body: table("", "", "height:40px;padding:5px", "", 1),
            expected: &[
                ("c", [8.0, 8.0, 20.0, 40.0]),
                ("p", [13.0, 13.0, 10.0, 15.0]),
            ],
        },
        // The cell's own height makes it definite, and the row it ends up in
        // is what the percentage resolves against, even when the row is
        // taller than the height the cell states.
        Case {
            label: "a cell with a height next to a taller cell",
            body: table(
                "",
                "",
                "height:40px",
                "<div id=d style=\"display:table-cell\"><div style=\"width:10px;height:60px\"></div></div>",
                1,
            ),
            expected: &[
                ("c", [8.0, 8.0, 10.0, 62.0]),
                ("p", [8.0, 60.0, 10.0, 31.0]),
            ],
        },
        Case {
            label: "a cell with a height in a taller row",
            body: table("height:100px", "", "height:40px", "", 2),
            expected: &[("r", [8.0, 8.0, 10.0, 40.0]), ("p", [8.0, 8.0, 10.0, 20.0])],
        },
    ]);
}

/// A table of one row whose cell `c` has `cell` as its style and `inside` as
/// its content, in a table styled `table`.
fn aligned(table: &str, cell: &str, inside: &str) -> String {
    format!(
        "<div id=t style=\"display:table;{table}\"><div id=r style=\"display:table-row\">\
         <div id=c style=\"display:table-cell;{cell}\">{inside}</div></div></div>"
    )
}

const HALF: &str = "<div id=p style=\"height:50%\">x</div>";

#[test]
fn a_cell_aligns_its_content_by_the_height_the_percentages_give_it() {
    let middle = "vertical-align:middle;height:40px";
    check(&[
        // The child is 20px once its percentage resolves, not the 10px of the
        // line it holds before.
        Case {
            label: "middle",
            body: aligned("", middle, HALF),
            expected: &[("p", [8.0, 18.0, 10.0, 20.0])],
        },
        Case {
            label: "bottom",
            body: aligned("", "vertical-align:bottom;height:40px", HALF),
            expected: &[("p", [8.0, 28.0, 10.0, 20.0])],
        },
        Case {
            label: "top and baseline do not move it",
            body: aligned("", "vertical-align:top;height:40px", HALF),
            expected: &[("p", [8.0, 8.0, 10.0, 20.0])],
        },
        Case {
            label: "baseline",
            body: aligned("", "vertical-align:baseline;height:40px", HALF),
            expected: &[("p", [8.0, 8.0, 10.0, 20.0])],
        },
        // The height comes from the table, which states one.
        Case {
            label: "middle in a table with a height",
            body: aligned("height:100px", "vertical-align:middle", HALF),
            expected: &[("p", [8.0, 33.0, 10.0, 50.0])],
        },
        Case {
            label: "bottom in a table with a height",
            body: aligned("height:100px", "vertical-align:bottom", HALF),
            expected: &[("p", [8.0, 58.0, 10.0, 50.0])],
        },
        // It fills the cell: nothing is left to align.
        Case {
            label: "a child that fills the cell",
            body: aligned("", middle, "<div id=p style=\"height:100%\">x</div>"),
            expected: &[("p", [8.0, 8.0, 10.0, 40.0])],
        },
        Case {
            label: "a grandchild under a child that fills the cell",
            body: aligned(
                "",
                middle,
                "<div id=q style=\"height:100%\"><div id=p style=\"height:50%\">x</div></div>",
            ),
            expected: &[("q", [8.0, 8.0, 10.0, 40.0]), ("p", [8.0, 8.0, 10.0, 20.0])],
        },
        // The margins of the child count in the height of the content.
        Case {
            label: "margins around the child",
            body: aligned(
                "",
                middle,
                "<div id=p style=\"height:50%;margin:4px 0 6px\">x</div>",
            ),
            expected: &[("p", [8.0, 17.0, 10.0, 20.0])],
        },
        Case {
            label: "another child below it",
            body: aligned(
                "",
                middle,
                &format!("{HALF}<div id=q style=\"height:10px\">y</div>"),
            ),
            expected: &[
                ("p", [8.0, 13.0, 10.0, 20.0]),
                ("q", [8.0, 33.0, 10.0, 10.0]),
            ],
        },
        Case {
            label: "a float",
            body: aligned(
                "",
                middle,
                "<div id=p style=\"height:50%;float:left;width:10px\">x</div>",
            ),
            expected: &[("p", [8.0, 18.0, 10.0, 20.0])],
        },
        // The percentage is of the content box, so the padding stays out.
        Case {
            label: "middle with padding",
            body: aligned("", "vertical-align:middle;height:40px;padding:4px", HALF),
            expected: &[("p", [12.0, 20.0, 10.0, 16.0])],
        },
        Case {
            label: "bottom with padding",
            body: aligned("", "vertical-align:bottom;height:40px;padding:4px", HALF),
            expected: &[("p", [12.0, 28.0, 10.0, 16.0])],
        },
        // The child is shorter than its three lines, which overflow it.
        Case {
            label: "content that overflows the child",
            body: aligned(
                "width:10px",
                middle,
                "<div id=p style=\"height:50%\">x x x</div>",
            ),
            expected: &[("p", [8.0, 18.0, 10.0, 20.0])],
        },
        // The percentage is of the row the cell ends up in.
        Case {
            label: "a taller cell beside it",
            body: "<div id=t style=\"display:table\"><div id=r style=\"display:table-row\">\
                   <div id=c style=\"display:table-cell;vertical-align:middle;height:40px\">\
                   <div id=p style=\"height:50%\">x</div></div>\
                   <div id=d style=\"display:table-cell;height:60px\">\
                   <div style=\"height:60px;width:10px\"></div></div></div></div>"
                .to_string(),
            expected: &[
                ("c", [8.0, 8.0, 10.0, 60.0]),
                ("p", [8.0, 23.0, 10.0, 30.0]),
            ],
        },
        // Without a percentage the alignment is what it was.
        Case {
            label: "a child without a percentage",
            body: aligned("", middle, "<div id=p style=\"height:10px\">x</div>"),
            expected: &[("p", [8.0, 23.0, 10.0, 10.0])],
        },
        Case {
            label: "a child without a percentage in a table with a height",
            body: aligned(
                "height:100px",
                "vertical-align:middle",
                "<div id=p style=\"height:10px\">x</div>",
            ),
            expected: &[("p", [8.0, 53.0, 10.0, 10.0])],
        },
    ]);
}

#[test]
fn a_cell_aligns_the_same_in_a_table_some_container_lays_out() {
    let middle = "vertical-align:middle";
    let flex = |style: &str, table: String| {
        format!("<div id=w style=\"display:flex;{style}\">{table}</div>")
    };
    check(&[
        Case {
            label: "a flex row",
            body: flex("", aligned("height:100px", middle, HALF)),
            expected: &[("p", [8.0, 33.0, 10.0, 50.0])],
        },
        Case {
            label: "a flex column",
            body: flex(
                "flex-direction:column",
                aligned("height:100px", middle, HALF),
            ),
            expected: &[("p", [8.0, 33.0, 784.0, 50.0])],
        },
        // The height the column gives the table is not one the table states,
        // so the percentage is `auto` and the line is centred as it is.
        Case {
            label: "a flex column that stretches the table",
            body: flex(
                "flex-direction:column;height:200px",
                aligned("flex-grow:1", middle, HALF),
            ),
            expected: &[("p", [8.0, 103.0, 784.0, 10.0])],
        },
        Case {
            label: "a flex column that stretches a table with a height",
            body: flex(
                "flex-direction:column;height:200px",
                aligned("flex-grow:1;height:60px", middle, HALF),
            ),
            expected: &[("p", [8.0, 58.0, 784.0, 100.0])],
        },
        Case {
            label: "a grid",
            body: format!(
                "<div id=w style=\"display:grid\">{}</div>",
                aligned("height:100px", middle, HALF)
            ),
            expected: &[("p", [8.0, 33.0, 784.0, 50.0])],
        },
        Case {
            label: "a table in a cell that is centred too",
            body: aligned(
                "",
                "vertical-align:middle;height:80px",
                "<div style=\"display:table;height:100px\"><div style=\"display:table-row\">\
                 <div style=\"display:table-cell;vertical-align:middle\">\
                 <div id=p style=\"height:50%\">x</div></div></div></div>",
            ),
            expected: &[("p", [8.0, 33.0, 10.0, 50.0])],
        },
        Case {
            label: "two cells aligned differently in a row with a height",
            body: "<div id=t style=\"display:table\"><div id=r style=\"display:table-row;height:60px\">\
                   <div id=c style=\"display:table-cell;vertical-align:bottom;height:40px\">\
                   <div id=p style=\"height:50%\">x</div></div>\
                   <div id=d style=\"display:table-cell;vertical-align:middle;height:40px\">\
                   <div id=q style=\"height:50%\">y</div></div></div></div>"
                .to_string(),
            expected: &[("p", [8.0, 38.0, 10.0, 30.0]), ("q", [18.0, 23.0, 10.0, 30.0])],
        },
    ]);
}
