//! The `height` of an absolutely positioned table is the height of its grid
//! box, and its caption comes on top of that, as it does for a table in flow
//! (CSS Tables 3 section 3). The layout library places the whole wrapper box,
//! so an inset from the bottom edge anchors the end of the caption or the grid,
//! not the end of the grid alone.
//!
//! The expected boxes are the ones Chromium computes for the same markup, with
//! no body margin and `10px Ahem` text: each is the element's left, top, width
//! and height. The box of the table `t` is its grid box, which is what a table
//! reports here; Chromium reports the wrapper, caption included.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The left, top, width and height of the elements of `body` that have an
/// `id`, on the first page.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         html, body {{ margin: 0 }} body {{ font: 10px/10px Ahem }}</style>{body}"
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
            Some((id.to_owned(), [rect.x, rect.y, rect.width, rect.height]))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.04;

/// A label, the markup of a case and the `(id, left, top, width, height)` it
/// expects.
type Case<'a> = (&'a str, &'a str, &'a [(&'a str, f32, f32, f32, f32)]);

/// Lays out each case and compares the elements it names with the expected
/// boxes, reporting every mismatch at once.
fn check_boxes(cases: &[Case<'_>]) {
    let mut mismatches = Vec::new();
    for &(label, body, expected) in cases {
        let actual = boxes(body);
        for &(id, left, top, width, height) in expected {
            let (_, rect) = actual
                .iter()
                .find(|(name, _)| name == id)
                .unwrap_or_else(|| panic!("{label}: no fragment for #{id}"));
            let want = [left, top, width, height];
            if !rect
                .iter()
                .zip(want)
                .all(|(got, want)| (got - want).abs() <= TOLERANCE)
            {
                mismatches.push(format!(
                    "{label}: #{id} is {rect:?}, expected {want:?} (left, top, width, height)"
                ));
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

/// A 200x100 positioned box holding an absolutely positioned table `t` whose
/// style is `style`, with a 30x10 caption `c` styled by `caption` and a single
/// cell `d` that holds a 10px square.
fn captioned_table(style: &str, caption: &str) -> String {
    format!(
        "<div style='position: relative; width: 200px; height: 100px'>\
         <div id=t style='display: table; position: absolute; {style}'>\
         <div id=c style='display: table-caption; height: 10px; width: 30px; {caption}'></div>\
         <div style='display: table-row'><div id=d style='display: table-cell'>\
         <div style='width: 10px; height: 10px'></div></div></div></div></div>"
    )
}

#[test]
fn a_height_is_the_height_of_the_grid_box_and_the_caption_comes_on_top() {
    check_boxes(&[
        (
            "caption above",
            &captioned_table("top: 0; left: 0; height: 40px", ""),
            &[
                ("t", 0.0, 10.0, 30.0, 40.0),
                ("c", 0.0, 0.0, 30.0, 10.0),
                ("d", 0.0, 10.0, 30.0, 40.0),
            ],
        ),
        (
            "caption below",
            &captioned_table("top: 0; left: 0; height: 40px; caption-side: bottom", ""),
            &[
                ("t", 0.0, 0.0, 30.0, 40.0),
                ("c", 0.0, 40.0, 30.0, 10.0),
                ("d", 0.0, 0.0, 30.0, 40.0),
            ],
        ),
        (
            "padding and border",
            &captioned_table(
                "top: 0; left: 0; height: 40px; border: 2px solid #00f; padding: 3px",
                "",
            ),
            &[
                ("t", 0.0, 10.0, 30.0, 50.0),
                ("c", 0.0, 0.0, 30.0, 10.0),
                ("d", 5.0, 15.0, 20.0, 40.0),
            ],
        ),
        (
            "border-box sizing",
            &captioned_table(
                "top: 0; left: 0; height: 40px; box-sizing: border-box; border: 2px solid #00f",
                "",
            ),
            &[
                ("t", 0.0, 10.0, 30.0, 40.0),
                ("c", 0.0, 0.0, 30.0, 10.0),
                ("d", 2.0, 12.0, 26.0, 36.0),
            ],
        ),
        (
            "a percentage height",
            &captioned_table("top: 0; left: 0; height: 50%", ""),
            &[("t", 0.0, 10.0, 30.0, 50.0), ("d", 0.0, 10.0, 30.0, 50.0)],
        ),
        (
            "caption margins",
            &captioned_table("top: 0; left: 0; height: 40px", "margin: 4px 0"),
            &[
                ("t", 0.0, 18.0, 30.0, 40.0),
                ("c", 0.0, 4.0, 30.0, 10.0),
                ("d", 0.0, 18.0, 30.0, 40.0),
            ],
        ),
        (
            "a height below the content",
            &captioned_table("top: 0; left: 0; height: 5px", ""),
            &[("t", 0.0, 10.0, 30.0, 10.0), ("d", 0.0, 10.0, 30.0, 10.0)],
        ),
        (
            "both vertical insets",
            &captioned_table("top: 0; bottom: 0; left: 0; height: 40px", ""),
            &[("t", 0.0, 10.0, 30.0, 40.0), ("d", 0.0, 10.0, 30.0, 40.0)],
        ),
        (
            "a fixed table",
            &captioned_table("top: 0; left: 0; height: 40px; position: fixed", ""),
            &[("t", 0.0, 10.0, 30.0, 40.0), ("d", 0.0, 10.0, 30.0, 40.0)],
        ),
        (
            "an inline-table",
            &captioned_table("top: 0; left: 0; height: 40px; display: inline-table", ""),
            &[("t", 0.0, 10.0, 30.0, 40.0), ("d", 0.0, 10.0, 30.0, 40.0)],
        ),
    ]);
}

#[test]
fn the_box_is_placed_by_its_whole_height() {
    check_boxes(&[
        (
            "anchored to the bottom edge",
            &captioned_table("bottom: 0; left: 0; height: 40px", ""),
            &[
                ("t", 0.0, 60.0, 30.0, 40.0),
                ("c", 0.0, 50.0, 30.0, 10.0),
                ("d", 0.0, 60.0, 30.0, 40.0),
            ],
        ),
        (
            "padding and border",
            &captioned_table(
                "bottom: 0; left: 0; height: 40px; border: 2px solid #00f; padding: 3px",
                "",
            ),
            &[
                ("t", 0.0, 50.0, 30.0, 50.0),
                ("c", 0.0, 40.0, 30.0, 10.0),
                ("d", 5.0, 55.0, 20.0, 40.0),
            ],
        ),
        (
            "border-box sizing",
            &captioned_table(
                "bottom: 0; left: 0; height: 40px; box-sizing: border-box; border: 2px solid #00f",
                "",
            ),
            &[("c", 0.0, 50.0, 30.0, 10.0), ("d", 2.0, 62.0, 26.0, 36.0)],
        ),
        (
            "caption below the grid",
            &captioned_table("bottom: 0; left: 0; height: 40px; caption-side: bottom", ""),
            &[
                ("t", 0.0, 50.0, 30.0, 40.0),
                ("c", 0.0, 90.0, 30.0, 10.0),
                ("d", 0.0, 50.0, 30.0, 40.0),
            ],
        ),
        (
            "caption margins",
            &captioned_table("bottom: 0; left: 0; height: 40px", "margin: 4px 0"),
            &[("c", 0.0, 46.0, 30.0, 10.0), ("d", 0.0, 60.0, 30.0, 40.0)],
        ),
        (
            "a height below the content",
            &captioned_table("bottom: 0; left: 0; height: 5px", ""),
            &[("c", 0.0, 80.0, 30.0, 10.0), ("d", 0.0, 90.0, 30.0, 10.0)],
        ),
        (
            "a percentage height and inset",
            &captioned_table("bottom: 10%; left: 0; height: 50%", ""),
            &[("c", 0.0, 30.0, 30.0, 10.0), ("d", 0.0, 40.0, 30.0, 50.0)],
        ),
        (
            "a bottom margin",
            &captioned_table("bottom: 0; left: 0; height: 40px; margin-bottom: 7px", ""),
            &[("c", 0.0, 43.0, 30.0, 10.0), ("d", 0.0, 53.0, 30.0, 40.0)],
        ),
        (
            "a top inset with a top margin",
            &captioned_table("top: 20px; left: 0; height: 40px; margin-top: 3px", ""),
            &[("c", 0.0, 23.0, 30.0, 10.0), ("d", 0.0, 33.0, 30.0, 40.0)],
        ),
        (
            "an inline-table",
            &captioned_table(
                "bottom: 0; left: 0; height: 40px; display: inline-table",
                "",
            ),
            &[("c", 0.0, 50.0, 30.0, 10.0), ("d", 0.0, 60.0, 30.0, 40.0)],
        ),
        (
            "a fixed table, placed against the page",
            &captioned_table("bottom: 0; left: 0; height: 40px; position: fixed", ""),
            &[("c", 0.0, 550.0, 30.0, 10.0), ("d", 0.0, 560.0, 30.0, 40.0)],
        ),
        (
            "a table with no cells",
            "<div style='position: relative; width: 200px; height: 100px'>\
             <div id=t style='display: table; position: absolute; bottom: 0; left: 0; height: 40px'>\
             <div id=c style='display: table-caption; height: 10px; width: 30px'></div></div></div>",
            &[("c", 0.0, 50.0, 30.0, 10.0)],
        ),
        (
            "a table with no cells, from the top",
            "<div style='position: relative; width: 200px; height: 100px'>\
             <div id=t style='display: table; position: absolute; top: 0; left: 0; height: 40px'>\
             <div id=c style='display: table-caption; height: 10px; width: 30px'></div></div></div>",
            &[("c", 0.0, 0.0, 30.0, 10.0)],
        ),
    ]);
}

#[test]
fn limits_padding_and_borders_combine_with_the_caption_from_the_bottom() {
    check_boxes(&[
        (
            "max-height under the height, with padding and border",
            &captioned_table(
                "bottom: 0; left: 0; height: 40px; max-height: 30px; border: 2px solid #00f; \
                 padding: 3px",
                "",
            ),
            &[("c", 0.0, 50.0, 30.0, 10.0), ("d", 5.0, 65.0, 20.0, 30.0)],
        ),
        (
            "min-height over the height, with padding and border",
            &captioned_table(
                "bottom: 0; left: 0; height: 40px; min-height: 60px; border: 2px solid #00f; \
                 padding: 3px",
                "",
            ),
            &[("c", 0.0, 20.0, 30.0, 10.0), ("d", 5.0, 35.0, 20.0, 60.0)],
        ),
        (
            "min-height over the height, border-box",
            &captioned_table(
                "bottom: 0; left: 0; height: 40px; min-height: 60px; box-sizing: border-box; \
                 border: 2px solid #00f; padding: 3px",
                "",
            ),
            &[("c", 0.0, 30.0, 30.0, 10.0), ("d", 5.0, 45.0, 20.0, 50.0)],
        ),
        (
            "max-height under the height, border-box",
            &captioned_table(
                "bottom: 0; left: 0; height: 60px; max-height: 40px; box-sizing: border-box; \
                 border: 2px solid #00f; padding: 3px",
                "",
            ),
            &[("c", 0.0, 50.0, 30.0, 10.0), ("d", 5.0, 65.0, 20.0, 30.0)],
        ),
    ]);
}

#[test]
fn a_table_with_no_cells_follows_the_same_limits_from_the_bottom() {
    let empty = |style: &str| {
        format!(
            "<div style='position: relative; width: 200px; height: 100px'>\
             <div id=t style='display: table; position: absolute; bottom: 0; left: 0; {style}'>\
             <div id=c style='display: table-caption; height: 10px; width: 30px'></div>\
             </div></div>"
        )
    };
    let (min, max, border_box, border) = (
        empty("height: 20px; min-height: 40px"),
        empty("height: 40px; max-height: 30px"),
        empty("height: 40px; box-sizing: border-box; border: 2px solid #00f"),
        empty("height: 40px; border: 2px solid #00f; padding: 3px"),
    );
    check_boxes(&[
        ("min-height", &min, &[("c", 0.0, 50.0, 30.0, 10.0)]),
        ("max-height", &max, &[("c", 0.0, 60.0, 30.0, 10.0)]),
        ("border-box", &border_box, &[("c", 0.0, 50.0, 30.0, 10.0)]),
        (
            "padding and border",
            &border,
            &[("c", 0.0, 40.0, 30.0, 10.0)],
        ),
    ]);
}

#[test]
fn a_horizontal_table_in_a_vertical_block_reads_its_physical_insets() {
    let table = |style: &str| {
        format!(
            "<div style='position: relative; width: 200px; height: 100px; \
             writing-mode: vertical-rl'><div id=t style='display: table; position: absolute; \
             writing-mode: horizontal-tb; height: 40px; {style}'>\
             <div id=c style='display: table-caption; height: 10px; width: 30px'></div>\
             <div style='display: table-row'><div id=d style='display: table-cell'>\
             <div style='width: 10px; height: 10px'></div></div></div></div></div>"
        )
    };
    // The vertical insets are `auto` and `bottom`: the whole wrapper is placed.
    let from_the_bottom = table("width: 30px; left: 0; right: 0; bottom: 0");
    // Both vertical insets are stated: the box stays at the top.
    let between = table("top: 0; bottom: 0; left: 0");
    check_boxes(&[
        (
            "bottom inset",
            &from_the_bottom,
            &[
                ("c", 170.0, 50.0, 30.0, 10.0),
                ("d", 170.0, 60.0, 30.0, 40.0),
            ],
        ),
        (
            "both vertical insets",
            &between,
            &[("c", 0.0, 0.0, 30.0, 10.0), ("d", 0.0, 10.0, 30.0, 40.0)],
        ),
    ]);
}

#[test]
fn an_intrinsic_keyword_or_calc_height_is_resolved_as_ever() {
    check_boxes(&[
        (
            "min-content",
            &captioned_table("top: 0; left: 0; height: min-content", ""),
            &[("c", 0.0, 0.0, 30.0, 10.0), ("d", 0.0, 10.0, 30.0, 10.0)],
        ),
        (
            "calc",
            &captioned_table("top: 0; left: 0; height: calc(50% + 10px)", ""),
            &[("c", 0.0, 0.0, 30.0, 10.0), ("d", 0.0, 10.0, 30.0, 60.0)],
        ),
        (
            "calc from the bottom",
            &captioned_table("bottom: 0; left: 0; height: calc(50% + 10px)", ""),
            &[("c", 0.0, 30.0, 30.0, 10.0), ("d", 0.0, 40.0, 30.0, 60.0)],
        ),
    ]);
}

#[test]
fn the_containing_block_may_be_any_kind_of_box() {
    let table = "<div id=t style='display: table; position: absolute; bottom: 0; left: 0; \
                 height: 40px'><div id=c style='display: table-caption; height: 10px; \
                 width: 30px'></div><div style='display: table-row'><div id=d \
                 style='display: table-cell'><div style='width: 10px; height: 10px'></div>\
                 </div></div></div>";
    let expected: &[(&str, f32, f32, f32, f32)] =
        &[("c", 0.0, 50.0, 30.0, 10.0), ("d", 0.0, 60.0, 30.0, 40.0)];
    let (flex, grid, vertical) = (
        format!(
            "<div style='position: relative; width: 200px; height: 100px; display: flex'>{table}</div>"
        ),
        format!(
            "<div style='position: relative; width: 200px; height: 100px; display: grid'>{table}</div>"
        ),
        format!(
            "<div style='position: relative; width: 200px; height: 100px; \
             writing-mode: vertical-rl'>{}</div>",
            table.replace("height: 40px", "height: 40px; writing-mode: horizontal-tb")
        ),
    );
    check_boxes(&[
        ("a flex container", &flex, expected),
        ("a grid container", &grid, expected),
        ("a vertical container", &vertical, expected),
    ]);
}

#[test]
fn a_table_without_a_caption_is_placed_by_its_height() {
    check_boxes(&[(
        "no caption",
        "<div style='position: relative; width: 200px; height: 100px'>\
         <div id=t style='display: table; position: absolute; bottom: 0; left: 0; height: 40px'>\
         <div id=d style='display: table-cell'><div style='width: 10px; height: 10px'></div>\
         </div></div></div>",
        &[("t", 0.0, 60.0, 10.0, 40.0), ("d", 0.0, 60.0, 10.0, 40.0)],
    )]);
}

#[test]
fn a_minimum_or_maximum_height_still_applies_to_the_grid_box() {
    check_boxes(&[
        (
            "min-height",
            &captioned_table("top: 0; left: 0; min-height: 40px", ""),
            &[("t", 0.0, 10.0, 30.0, 40.0), ("d", 0.0, 10.0, 30.0, 40.0)],
        ),
        (
            "height above max-height",
            &captioned_table("top: 0; left: 0; height: 40px; max-height: 45px", ""),
            &[("t", 0.0, 10.0, 30.0, 40.0), ("d", 0.0, 10.0, 30.0, 40.0)],
        ),
        (
            "max-height below the height, from the bottom",
            &captioned_table("bottom: 0; left: 0; height: 40px; max-height: 30px", ""),
            &[("c", 0.0, 60.0, 30.0, 10.0), ("d", 0.0, 70.0, 30.0, 30.0)],
        ),
        (
            "min-height above the height, from the bottom",
            &captioned_table("bottom: 0; left: 0; height: 20px; min-height: 40px", ""),
            &[("c", 0.0, 50.0, 30.0, 10.0), ("d", 0.0, 60.0, 30.0, 40.0)],
        ),
    ]);
}
