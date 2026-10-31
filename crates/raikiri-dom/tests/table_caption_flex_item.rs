//! A table with a caption that is a flex or grid item takes the room of its
//! caption once: the parent assigns the height of the whole table wrapper box,
//! the caption keeps its share of that height, and the table grid takes what
//! is left (CSS Tables 3 §3). The grid used to take the whole height as well,
//! so the container grew by the caption's height, and a table with padding and
//! a border and no caption came out shorter than its own padding and border.
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

/// The caption `aa bbb cc`, which wraps into three 10px lines at the width of
/// its longest word.
fn caption(style: &str) -> String {
    format!("<div id=c style=\"display:table-caption;{style}\">aa bbb cc</div>")
}

/// A row with one cell `x`, as id `id`.
fn row(id: &str) -> String {
    format!(
        "<div id={id} style=\"display:table-row\"><div style=\"display:table-cell\">x</div></div>"
    )
}

fn table(style: &str, inside: &str) -> String {
    format!("<div id=o style=\"display:table;{style}\">{inside}</div>")
}

fn flex(style: &str, inside: &str) -> String {
    format!("<div id=w style=\"display:flex;{style}\">{inside}</div>")
}

fn grid(style: &str, inside: &str) -> String {
    format!("<div id=w style=\"display:grid;{style}\">{inside}</div>")
}

#[test]
fn a_flex_container_is_as_high_as_the_caption_and_the_rows_of_its_table() {
    let caption_and_row = format!("{}{}", caption(""), row("r"));
    check(&[
        Case {
            label: "a caption and no rows",
            body: flex("", &table("", &caption(""))),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 30.0]),
                ("c", [8.0, 8.0, 30.0, 30.0]),
            ],
        },
        Case {
            label: "a caption above a row",
            body: flex("", &table("", &caption_and_row)),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 40.0]),
                ("c", [8.0, 8.0, 30.0, 30.0]),
                ("r", [8.0, 38.0, 30.0, 10.0]),
            ],
        },
        Case {
            label: "a row before the caption",
            body: flex("", &table("", &format!("{}{}", row("r"), caption("")))),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 40.0]),
                ("c", [8.0, 8.0, 30.0, 30.0]),
                ("r", [8.0, 38.0, 30.0, 10.0]),
            ],
        },
        Case {
            label: "a caption below the row",
            body: flex(
                "",
                &table(
                    "",
                    &format!("{}{}", caption("caption-side:bottom"), row("r")),
                ),
            ),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 40.0]),
                ("c", [8.0, 18.0, 30.0, 30.0]),
                ("r", [8.0, 8.0, 30.0, 10.0]),
            ],
        },
        Case {
            label: "items aligned to the start",
            body: flex("align-items:flex-start", &table("", &caption_and_row)),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 40.0]),
                ("r", [8.0, 38.0, 30.0, 10.0]),
            ],
        },
        Case {
            label: "an inline flex container",
            body: flex("display:inline-flex", &table("", &caption_and_row)),
            expected: &[
                ("w", [8.0, 8.0, 30.0, 40.0]),
                ("r", [8.0, 38.0, 30.0, 10.0]),
            ],
        },
        Case {
            label: "a sibling item",
            body: flex(
                "",
                &format!(
                    "{}<div id=x style=\"width:10px;height:15px\"></div>",
                    table("", &caption_and_row)
                ),
            ),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 40.0]),
                ("x", [38.0, 8.0, 10.0, 15.0]),
            ],
        },
    ]);
}

#[test]
fn the_caption_wraps_at_the_width_the_flex_container_gives_the_table() {
    let caption_and_row = format!("{}{}", caption(""), row("r"));
    check(&[
        // A wider table lets the caption fit on one line.
        Case {
            label: "a table width",
            body: flex("", &table("width:100px", &caption_and_row)),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 20.0]),
                ("c", [8.0, 8.0, 100.0, 10.0]),
                ("r", [8.0, 18.0, 100.0, 10.0]),
            ],
        },
        Case {
            label: "a table that grows",
            body: flex("", &table("flex-grow:1", &caption_and_row)),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 20.0]),
                ("c", [8.0, 8.0, 784.0, 10.0]),
                ("r", [8.0, 18.0, 784.0, 10.0]),
            ],
        },
        Case {
            label: "a column of items",
            body: flex("flex-direction:column", &table("", &caption_and_row)),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 20.0]),
                ("c", [8.0, 8.0, 784.0, 10.0]),
                ("r", [8.0, 18.0, 784.0, 10.0]),
            ],
        },
    ]);
}

#[test]
fn the_padding_and_border_of_the_table_stay_inside_the_height_the_flex_container_gives() {
    let padded = "padding:3px;border:2px solid";
    check(&[
        // The grid of the table is as high as its padding, border and row.
        Case {
            label: "no caption",
            body: flex("", &table(padded, &row("r"))),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 20.0]),
                ("o", [8.0, 8.0, 20.0, 20.0]),
                ("r", [13.0, 13.0, 10.0, 10.0]),
            ],
        },
        Case {
            label: "a caption",
            body: flex("", &table(padded, &format!("{}{}", caption(""), row("r")))),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 50.0]),
                ("c", [8.0, 8.0, 30.0, 30.0]),
                ("r", [13.0, 43.0, 20.0, 10.0]),
            ],
        },
    ]);
}

#[test]
fn a_flex_container_taller_than_the_table_gives_the_rows_what_the_caption_leaves() {
    let two_rows = format!("{}{}{}", caption(""), row("r"), row("r2"));
    check(&[
        Case {
            label: "two rows in a container of 100px",
            body: flex("height:100px", &table("", &two_rows)),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 100.0]),
                ("c", [8.0, 8.0, 30.0, 30.0]),
                ("r", [8.0, 38.0, 30.0, 35.0]),
                ("r2", [8.0, 73.0, 30.0, 35.0]),
            ],
        },
        // Without a caption the rows share the whole height.
        Case {
            label: "two rows and no caption",
            body: flex(
                "height:100px",
                &table("", &format!("{}{}", row("r"), row("r2"))),
            ),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 100.0]),
                ("r", [8.0, 8.0, 10.0, 50.0]),
                ("r2", [8.0, 58.0, 10.0, 50.0]),
            ],
        },
    ]);
}

#[test]
fn a_height_the_table_states_is_the_height_of_its_grid_alone() {
    let caption_and_row = format!("{}{}", caption(""), row("r"));
    check(&[
        Case {
            label: "height: 50px",
            body: flex("", &table("height:50px", &caption_and_row)),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 80.0]),
                ("c", [8.0, 8.0, 30.0, 30.0]),
                ("r", [8.0, 38.0, 30.0, 50.0]),
            ],
        },
        Case {
            label: "height: 50px with the items at the start",
            body: flex(
                "align-items:flex-start",
                &table("height:50px", &caption_and_row),
            ),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 80.0]),
                ("r", [8.0, 38.0, 30.0, 50.0]),
            ],
        },
        // A parent that is taller than the table does not grow a table that
        // states its height.
        Case {
            label: "height: 50px in a flex container of 200px",
            body: flex("height:200px", &table("height:50px", &caption_and_row)),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 200.0]),
                ("c", [8.0, 8.0, 30.0, 30.0]),
                ("r", [8.0, 38.0, 30.0, 50.0]),
            ],
        },
        Case {
            label: "height: 50px in a grid container of 200px",
            body: grid("height:200px", &table("height:50px", &caption_and_row)),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 200.0]),
                ("c", [8.0, 8.0, 784.0, 10.0]),
                ("r", [8.0, 18.0, 784.0, 50.0]),
            ],
        },
    ]);
}

#[test]
fn a_grid_container_is_as_high_as_the_caption_and_the_rows_of_its_table() {
    let caption_and_row = format!("{}{}", caption(""), row("r"));
    check(&[
        Case {
            label: "a caption above a row",
            body: grid("", &table("", &caption_and_row)),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 20.0]),
                ("c", [8.0, 8.0, 784.0, 10.0]),
                ("r", [8.0, 18.0, 784.0, 10.0]),
            ],
        },
        Case {
            label: "padding and a border",
            body: grid("", &table("padding:3px;border:2px solid", &caption_and_row)),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 30.0]),
                ("c", [8.0, 8.0, 784.0, 10.0]),
                ("r", [13.0, 23.0, 774.0, 10.0]),
            ],
        },
    ]);
}

/// A caption without text, `height` high.
fn empty_caption(style: &str) -> String {
    format!("<div id=c style=\"display:table-caption;height:10px;{style}\"></div>")
}

#[test]
fn a_definite_flex_basis_along_a_column_sizes_the_grid_alone() {
    let column = "flex-direction:column";
    let basis_table = "width:80px;flex:0 0 50px";
    check(&[
        // The basis is that of the grid, and the caption comes on top of it.
        Case {
            label: "a caption and no rows",
            body: flex(column, &table(basis_table, &empty_caption(""))),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 60.0]),
                ("c", [8.0, 8.0, 80.0, 10.0]),
            ],
        },
        Case {
            label: "a caption above a row",
            body: flex(
                column,
                &table(basis_table, &format!("{}{}", empty_caption(""), row("r"))),
            ),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 60.0]),
                ("c", [8.0, 8.0, 80.0, 10.0]),
            ],
        },
        Case {
            label: "a border and a caption with a border",
            body: flex(
                column,
                &table(
                    "box-sizing:content-box;border:10px solid;width:80px;flex:0 0 50px",
                    &empty_caption("border:10px solid"),
                ),
            ),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 90.0]),
                ("c", [8.0, 8.0, 100.0, 20.0]),
            ],
        },
        Case {
            label: "a percentage basis",
            body: flex(
                "flex-direction:column;height:200px",
                &table(
                    "width:80px;flex:0 0 50%",
                    &format!("{}{}", empty_caption(""), row("r")),
                ),
            ),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 200.0]),
                ("c", [8.0, 8.0, 80.0, 10.0]),
            ],
        },
        // Along a row the basis is the table's width, and its height is the
        // one the container assigns.
        Case {
            label: "a basis along a row",
            body: flex(
                "",
                &table(
                    "flex:0 0 50px",
                    &format!("{}{}", empty_caption(""), row("r")),
                ),
            ),
            expected: &[
                ("w", [8.0, 8.0, 784.0, 20.0]),
                ("c", [8.0, 8.0, 50.0, 10.0]),
                ("r", [8.0, 18.0, 50.0, 10.0]),
            ],
        },
    ]);
}
