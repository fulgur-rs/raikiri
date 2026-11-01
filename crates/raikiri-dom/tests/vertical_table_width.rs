//! The width of a table in a vertical writing mode is its extent along the
//! block axis, which its rows set, so a table that states no width shrinks to
//! it instead of filling the room around it (CSS 2.1 section 17.5.2.2). A block
//! in one of its cells starts at the block-start side of the cell.
//!
//! The expected boxes are the ones Chromium computes for the same markup in an
//! 800px by 600px viewport, with the body margin of 8px and a font of 10px
//! Ahem: each is an element's left, top, width and height. Tables of more than
//! one row are not covered: their rows stack across the table here, not along it.

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

/// The boxes Chromium gives for some elements of a case: `[vertical-rl,
/// vertical-lr]`.
type Expected<'a> = [&'a [(&'a str, [f32; 4])]; 2];

const MODES: [&str; 2] = ["vertical-rl", "vertical-lr"];

/// Lays out the markup `body` makes for each writing mode and compares the
/// elements the expectation names, reporting every mismatch at once.
fn check(label: &str, body: impl Fn(&str) -> String, expected: Expected<'_>) {
    let mut mismatches = Vec::new();
    for (mode, expected) in MODES.into_iter().zip(expected) {
        let markup = body(mode);
        for (id, want) in expected {
            let found = border_box(&markup, id);
            if !found
                .iter()
                .zip(want)
                .all(|(got, want)| (got - want).abs() <= TOLERANCE)
            {
                mismatches.push(format!(
                    "{label} in {mode}: #{id} is {found:?}, expected {want:?} \
                     (left, top, width, height)"
                ));
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

/// A vertical table `t` of one row `r` in the writing mode `mode`, whose style
/// is `style`, holding `cells`.
fn table(mode: &str, style: &str, cells: &str) -> String {
    format!(
        "<div id=t style='display: table; writing-mode: {mode}; {style}'>\
         <div id=r style='display: table-row'>{cells}</div></div>"
    )
}

/// A cell `c` of `style` holding `content`.
fn cell(style: &str, content: &str) -> String {
    format!("<div id=c style='display: table-cell; {style}'>{content}</div>")
}

#[test]
fn a_table_that_states_no_width_is_as_wide_as_its_cells() {
    let one_cell = || cell("width: 60px; height: 80px", "<div id=p>x</div>");
    check(
        "a cell of 60px by 80px",
        |mode| table(mode, "", &one_cell()),
        [
            &[
                ("t", [8.0, 8.0, 60.0, 80.0]),
                ("r", [8.0, 8.0, 60.0, 80.0]),
                ("c", [8.0, 8.0, 60.0, 80.0]),
                ("p", [58.0, 8.0, 10.0, 80.0]),
            ],
            &[
                ("t", [8.0, 8.0, 60.0, 80.0]),
                ("c", [8.0, 8.0, 60.0, 80.0]),
                ("p", [8.0, 8.0, 10.0, 80.0]),
            ],
        ],
    );
    check(
        "text that wraps at the height of the cell",
        |mode| {
            table(
                mode,
                "",
                &cell("height: 80px", "<div id=p>xxxxxxxxxxxx</div>"),
            )
        },
        [
            &[
                ("t", [8.0, 8.0, 10.0, 120.0]),
                ("p", [8.0, 8.0, 10.0, 120.0]),
            ],
            &[
                ("t", [8.0, 8.0, 10.0, 120.0]),
                ("p", [8.0, 8.0, 10.0, 120.0]),
            ],
        ],
    );
    check(
        "a border and padding on the table",
        |mode| table(mode, "border: 2px solid #00f; padding: 3px", &one_cell()),
        [
            &[
                ("t", [8.0, 8.0, 70.0, 90.0]),
                ("c", [13.0, 13.0, 60.0, 80.0]),
                ("p", [63.0, 13.0, 10.0, 80.0]),
            ],
            &[
                ("t", [8.0, 8.0, 70.0, 90.0]),
                ("c", [13.0, 13.0, 60.0, 80.0]),
                ("p", [13.0, 13.0, 10.0, 80.0]),
            ],
        ],
    );
    check(
        "a block narrower than the cell: wider than its container",
        |mode| {
            format!(
                "<div style='width: 40px'>{}</div>",
                table(mode, "", &one_cell())
            )
        },
        [
            &[
                ("t", [8.0, 8.0, 60.0, 80.0]),
                ("p", [58.0, 8.0, 10.0, 80.0]),
            ],
            &[("t", [8.0, 8.0, 60.0, 80.0]), ("p", [8.0, 8.0, 10.0, 80.0])],
        ],
    );
    check(
        "a stated height",
        |mode| {
            table(
                mode,
                "height: 120px",
                &cell("width: 60px", "<div id=p>x</div>"),
            )
        },
        [
            &[
                ("t", [8.0, 8.0, 60.0, 120.0]),
                ("p", [58.0, 8.0, 10.0, 120.0]),
            ],
            &[
                ("t", [8.0, 8.0, 60.0, 120.0]),
                ("p", [8.0, 8.0, 10.0, 120.0]),
            ],
        ],
    );
    check(
        "a cell without a height",
        |mode| table(mode, "", &cell("width: 60px", "<div id=p>xxxx</div>")),
        [
            &[
                ("t", [8.0, 8.0, 60.0, 40.0]),
                ("p", [58.0, 8.0, 10.0, 40.0]),
            ],
            &[("t", [8.0, 8.0, 60.0, 40.0]), ("p", [8.0, 8.0, 10.0, 40.0])],
        ],
    );
}

#[test]
fn an_inline_or_floated_table_is_as_wide_as_its_cells() {
    let one_cell = || cell("width: 60px; height: 80px", "<div id=p>x</div>");
    let expected: Expected<'_> = [
        &[
            ("t", [8.0, 8.0, 60.0, 80.0]),
            ("p", [58.0, 8.0, 10.0, 80.0]),
        ],
        &[("t", [8.0, 8.0, 60.0, 80.0]), ("p", [8.0, 8.0, 10.0, 80.0])],
    ];
    check(
        "an inline table",
        |mode| table(mode, "display: inline-table", &one_cell()),
        expected,
    );
    check(
        "a floated table",
        |mode| table(mode, "float: left", &one_cell()),
        expected,
    );
}

#[test]
fn a_table_that_states_a_width_keeps_it() {
    check(
        "a width of 200px",
        |mode| {
            table(
                mode,
                "width: 200px",
                &cell("width: 60px; height: 80px", "<div id=p>x</div>"),
            )
        },
        [
            &[
                ("t", [8.0, 8.0, 200.0, 80.0]),
                ("p", [198.0, 8.0, 10.0, 80.0]),
            ],
            &[
                ("t", [8.0, 8.0, 200.0, 80.0]),
                ("p", [8.0, 8.0, 10.0, 80.0]),
            ],
        ],
    );
}

#[test]
fn the_cells_of_a_vertical_row_share_the_extent_of_the_row() {
    check(
        "two cells of 30px by 50px",
        |mode| {
            let cells = format!(
                "{}<div id=d style='display: table-cell; width: 30px; height: 50px'>\
                 <div id=q>y</div></div>",
                cell("width: 30px; height: 50px", "<div id=p>x</div>")
            );
            table(mode, "", &cells)
        },
        [
            &[
                ("t", [8.0, 8.0, 30.0, 100.0]),
                ("c", [8.0, 8.0, 30.0, 50.0]),
                ("p", [28.0, 8.0, 10.0, 50.0]),
                ("d", [8.0, 58.0, 30.0, 50.0]),
                ("q", [28.0, 58.0, 10.0, 50.0]),
            ],
            &[
                ("t", [8.0, 8.0, 30.0, 100.0]),
                ("c", [8.0, 8.0, 30.0, 50.0]),
                ("p", [8.0, 8.0, 10.0, 50.0]),
                ("d", [8.0, 58.0, 30.0, 50.0]),
                ("q", [8.0, 58.0, 10.0, 50.0]),
            ],
        ],
    );
}

#[test]
fn a_block_in_a_vertical_cell_starts_at_the_block_start_of_the_cell() {
    let block = "<div id=p style='width: 30px; height: 20px'></div>";
    let in_cell = |align: &'static str| {
        move |mode: &str| {
            table(
                mode,
                "",
                &cell(&format!("width: 60px; height: 80px; {align}"), block),
            )
        }
    };
    // The block-start side is on the right in vertical-rl and on the left in
    // vertical-lr.
    for (align, rl, lr) in [
        ("vertical-align: baseline", 38.0, 8.0),
        ("vertical-align: top", 38.0, 8.0),
        ("vertical-align: middle", 23.0, 23.0),
        ("vertical-align: bottom", 8.0, 38.0),
    ] {
        check(
            align,
            in_cell(align),
            [
                &[("t", [8.0, 8.0, 60.0, 80.0]), ("p", [rl, 8.0, 30.0, 20.0])],
                &[("t", [8.0, 8.0, 60.0, 80.0]), ("p", [lr, 8.0, 30.0, 20.0])],
            ],
        );
    }
    // A table wider than its cell leaves the room after the block, not before it.
    for align in ["vertical-align: baseline", "vertical-align: top"] {
        check(
            &format!("a table of 200px, {align}"),
            |mode| {
                table(
                    mode,
                    "width: 200px",
                    &cell(&format!("width: 60px; height: 80px; {align}"), block),
                )
            },
            [
                &[
                    ("t", [8.0, 8.0, 200.0, 80.0]),
                    ("p", [178.0, 8.0, 30.0, 20.0]),
                ],
                &[
                    ("t", [8.0, 8.0, 200.0, 80.0]),
                    ("p", [8.0, 8.0, 30.0, 20.0]),
                ],
            ],
        );
    }
    // Half of the 60px of the cell, whatever it is that the table shrinks to.
    check(
        "a percentage width",
        |mode| {
            table(
                mode,
                "",
                &cell(
                    "width: 60px; height: 80px",
                    "<div id=p style='width: 50%; height: 20px'></div>",
                ),
            )
        },
        [
            &[
                ("t", [8.0, 8.0, 60.0, 80.0]),
                ("p", [38.0, 8.0, 30.0, 20.0]),
            ],
            &[("t", [8.0, 8.0, 60.0, 80.0]), ("p", [8.0, 8.0, 30.0, 20.0])],
        ],
    );
}
