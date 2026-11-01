//! The `max-width` of an absolutely positioned table never makes the table
//! narrower than its content (CSS Tables 3 section 3.1, csswg-drafts#5336), as it
//! does not for a table in flow.
//!
//! The expected boxes are the ones Chromium computes for the same markup, with
//! no body margin and `10px Ahem` text: each is the element's left, top, width
//! and height.

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
/// style is `style` and whose only cell `d` holds `content`.
fn abspos_table(style: &str, content: &str) -> String {
    format!(
        "<div style='position: relative; width: 200px; height: 100px'>\
         <div id=t style='display: table; position: absolute; top: 0; left: 0; {style}'>\
         <div style='display: table-row'>\
         <div id=d style='display: table-cell'>{content}</div></div></div></div>"
    )
}

/// A 10px square.
const BOX: &str = "<div style='width: 10px; height: 10px'></div>";

#[test]
fn a_max_width_smaller_than_the_content_leaves_the_table_as_wide_as_its_content() {
    check_boxes(&[
        (
            "border",
            &abspos_table("border: 2px solid #00f; max-width: 5px", BOX),
            &[("t", 0.0, 0.0, 14.0, 14.0), ("d", 2.0, 2.0, 10.0, 10.0)],
        ),
        (
            "border and padding",
            &abspos_table("border: 2px solid #00f; padding: 3px; max-width: 5px", BOX),
            &[("t", 0.0, 0.0, 20.0, 20.0), ("d", 5.0, 5.0, 10.0, 10.0)],
        ),
        (
            "text: the table is as wide as the longest word",
            &abspos_table("max-width: 5px", "aaaa bbbb"),
            &[("t", 0.0, 0.0, 40.0, 20.0)],
        ),
        (
            "a width smaller than the content",
            &abspos_table("width: 20px; max-width: 5px", BOX),
            &[("t", 0.0, 0.0, 10.0, 10.0)],
        ),
        (
            "a percentage max-width",
            &abspos_table("max-width: 5%", "aaaa bbbb"),
            &[("t", 0.0, 0.0, 40.0, 20.0)],
        ),
    ]);
}

#[test]
fn a_max_width_the_content_can_fit_in_still_limits_the_table() {
    check_boxes(&[
        (
            "wrapped text",
            &abspos_table("max-width: 50px", "aaaa bbbb"),
            &[("t", 0.0, 0.0, 50.0, 20.0), ("d", 0.0, 0.0, 50.0, 20.0)],
        ),
        (
            "an explicit width",
            &abspos_table("border: 2px solid #00f; width: 100px; max-width: 50px", BOX),
            &[("t", 0.0, 0.0, 54.0, 14.0), ("d", 2.0, 2.0, 50.0, 10.0)],
        ),
        (
            "a max-width above the content",
            &abspos_table("max-width: 30px", BOX),
            &[("t", 0.0, 0.0, 10.0, 10.0)],
        ),
        (
            "a min-width above the max-width",
            &abspos_table("min-width: 40px; max-width: 5px", BOX),
            &[("t", 0.0, 0.0, 40.0, 10.0)],
        ),
        (
            "anchored to the right edge",
            &abspos_table("max-width: 50px; left: auto; right: 0", "aaaa bbbb"),
            &[("t", 150.0, 0.0, 50.0, 20.0)],
        ),
        (
            "stretched between both insets",
            &abspos_table("max-width: 50px; left: 0; right: 0", "aaaa bbbb"),
            &[("t", 0.0, 0.0, 50.0, 20.0)],
        ),
    ]);
}

/// The table `t` of [`abspos_table`] with `style`, placed by `outer`, whose
/// `{}` holds it.
fn table_in(outer: &str, style: &str) -> String {
    outer.replace(
        "{}",
        &format!(
            "<div id=t style='display: table; position: absolute; top: 0; left: 0; {style}'>\
             <div style='display: table-row'>\
             <div id=d style='display: table-cell'>{BOX}</div></div></div>"
        ),
    )
}

#[test]
fn the_floor_holds_whatever_contains_the_table() {
    check_boxes(&[
        (
            "the initial containing block",
            &table_in("{}", "border: 2px solid #00f; max-width: 5px"),
            &[("t", 0.0, 0.0, 14.0, 14.0), ("d", 2.0, 2.0, 10.0, 10.0)],
        ),
        (
            "a grid container",
            &table_in(
                "<div style='display: grid; position: relative; width: 200px; height: 100px'>{}</div>",
                "border: 2px solid #00f; max-width: 5px",
            ),
            &[("t", 0.0, 0.0, 14.0, 14.0), ("d", 2.0, 2.0, 10.0, 10.0)],
        ),
        (
            "a flex container",
            &table_in(
                "<div style='display: flex; position: relative; width: 200px; height: 100px'>{}</div>",
                "border: 2px solid #00f; max-width: 5px",
            ),
            &[("t", 0.0, 0.0, 14.0, 14.0), ("d", 2.0, 2.0, 10.0, 10.0)],
        ),
        (
            "an inline-table with border-box sizing",
            &table_in(
                "<div style='position: relative; width: 200px; height: 100px'>{}</div>",
                "display: inline-table; box-sizing: border-box; border: 2px solid #00f; \
                 padding: 3px; max-width: 5px",
            ),
            &[("t", 0.0, 0.0, 20.0, 20.0), ("d", 5.0, 5.0, 10.0, 10.0)],
        ),
        (
            "an inline containing block",
            &table_in(
                "<div style='position: relative; width: 200px; height: 100px'>\
                 <span style='position: relative'>x{}</span></div>",
                "max-width: 5px",
            ),
            &[("t", 0.0, 0.0, 10.0, 10.0)],
        ),
        (
            "a fixed table",
            &table_in(
                "<div style='height: 100px'>{}</div>",
                "position: fixed; border: 2px solid #00f; max-width: 5px",
            ),
            &[("t", 0.0, 0.0, 14.0, 14.0), ("d", 2.0, 2.0, 10.0, 10.0)],
        ),
        (
            "a vertical writing mode",
            &table_in(
                "<div style='position: relative; width: 200px; height: 100px; \
                 writing-mode: vertical-rl'>{}</div>",
                "left: auto; right: 0; max-width: 5px",
            ),
            &[("t", 190.0, 0.0, 10.0, 10.0)],
        ),
        (
            "a vertical grid container",
            &table_in(
                "<div style='display: grid; position: relative; width: 200px; height: 100px; \
                 writing-mode: vertical-rl'>{}</div>",
                "left: auto; right: 0; border: 2px solid #00f; max-width: 5px",
            ),
            &[("t", 186.0, 0.0, 14.0, 14.0), ("d", 188.0, 2.0, 10.0, 10.0)],
        ),
        (
            "a grid area the table names, with room for its content",
            "<div style='display: grid; grid-template-columns: 80px 120px; position: relative; \
             width: 200px; height: 100px'><div id=t style='display: table; position: absolute; \
             top: 0; grid-column: 2; max-width: 40px'><div style='display: table-row'>\
             <div id=d style='display: table-cell'>aaaa bbbb</div></div></div></div>",
            &[("t", 80.0, 0.0, 40.0, 20.0)],
        ),
        (
            "a calc() max-width",
            &table_in(
                "<div style='position: relative; width: 200px; height: 100px'>{}</div>",
                "max-width: calc(2px + 3px)",
            ),
            &[("t", 0.0, 0.0, 10.0, 10.0)],
        ),
    ]);
}

/// The absolutely positioned table `t` with `style`, whose only cell `d` holds
/// `content`, in a grid container of 200px by 100px styled by `grid`.
fn table_in_grid(grid: &str, style: &str, content: &str) -> String {
    format!(
        "<div style='display: grid; position: relative; width: 200px; height: 100px; {grid}'>\
         <div id=t style='display: table; position: absolute; top: 0; {style}'>\
         <div style='display: table-row'>\
         <div id=d style='display: table-cell'>{content}</div></div></div></div>"
    )
}

/// A table that names a grid area is as wide as its content whatever its
/// `max-width` says, as the percentages of the box resolve against the width of
/// that area rather than against the whole grid container.
#[test]
fn a_table_that_names_a_grid_area_is_not_narrower_than_its_content() {
    const COLUMNS: &str = "grid-template-columns: 80px 120px";
    let boxed = "border: 2px solid #00f; max-width: 5px";
    check_boxes(&[
        (
            "the second column",
            &table_in_grid(COLUMNS, &format!("grid-column: 2; {boxed}"), BOX),
            &[("t", 80.0, 0.0, 14.0, 14.0), ("d", 82.0, 2.0, 10.0, 10.0)],
        ),
        (
            "the first column",
            &table_in_grid(COLUMNS, &format!("grid-column: 1; {boxed}"), BOX),
            &[("t", 0.0, 0.0, 14.0, 14.0)],
        ),
        (
            "both columns",
            &table_in_grid(COLUMNS, &format!("grid-column: 1 / 3; {boxed}"), BOX),
            &[("t", 0.0, 0.0, 14.0, 14.0)],
        ),
        (
            "a row after the first",
            &table_in_grid(
                "grid-template-columns: 80px 120px; grid-template-rows: 30px 60px",
                &format!("grid-column: 2; grid-row: 2; {boxed}"),
                BOX,
            ),
            &[("t", 80.0, 30.0, 14.0, 14.0)],
        ),
        (
            "a named line",
            &table_in_grid(
                "grid-template-columns: [a] 80px [b] 120px [c]",
                &format!("grid-column: b / c; {boxed}"),
                BOX,
            ),
            &[("t", 80.0, 0.0, 14.0, 14.0)],
        ),
        (
            "text: as wide as the longest word",
            &table_in_grid(COLUMNS, "grid-column: 2; max-width: 5px", "aaaa bbbb"),
            &[("t", 80.0, 0.0, 40.0, 20.0)],
        ),
        (
            "anchored to the right edge of the area",
            &table_in_grid(
                COLUMNS,
                &format!("grid-column: 2; left: auto; right: 0; {boxed}"),
                BOX,
            ),
            &[("t", 186.0, 0.0, 14.0, 14.0), ("d", 188.0, 2.0, 10.0, 10.0)],
        ),
        (
            "centered in the area",
            &table_in_grid(
                COLUMNS,
                &format!("grid-column: 2; justify-self: center; {boxed}"),
                BOX,
            ),
            &[("t", 133.0, 0.0, 14.0, 14.0)],
        ),
        (
            "a percentage max-width of the area: 9.6px of 120px",
            &table_in_grid(
                COLUMNS,
                "grid-column: 2; border: 2px solid #00f; max-width: 8%",
                BOX,
            ),
            &[("t", 80.0, 0.0, 14.0, 14.0)],
        ),
        (
            "a percentage padding of the area: 12px of 120px",
            &table_in_grid(COLUMNS, "grid-column: 2; padding: 10%; max-width: 5px", BOX),
            &[("t", 80.0, 0.0, 34.0, 34.0), ("d", 92.0, 12.0, 10.0, 10.0)],
        ),
    ]);
}

#[test]
fn a_table_that_names_a_grid_area_of_a_vertical_grid_is_not_narrower_than_its_content() {
    const ROWS: &str = "grid-template-rows: 80px 120px; writing-mode: vertical-rl";
    let boxed = "border: 2px solid #00f; max-width: 5px";
    check_boxes(&[
        (
            "the second row, which is on the left",
            &table_in_grid(ROWS, &format!("grid-row: 2; {boxed}"), BOX),
            &[("t", 106.0, 0.0, 14.0, 14.0), ("d", 108.0, 2.0, 10.0, 10.0)],
        ),
        (
            "the first row, which is on the right",
            &table_in_grid(ROWS, &format!("grid-row: 1; {boxed}"), BOX),
            &[("t", 186.0, 0.0, 14.0, 14.0)],
        ),
        (
            "a percentage max-width of the area",
            &table_in_grid(
                ROWS,
                "grid-row: 2; border: 2px solid #00f; max-width: 8%",
                BOX,
            ),
            &[("t", 106.0, 0.0, 14.0, 14.0)],
        ),
    ]);
}

/// The column of a fixed table with a `width` is sized from the `width`, so
/// its content says nothing about the narrowest it can be, in a grid area too.
#[test]
fn a_fixed_table_with_a_width_in_a_grid_area_ignores_its_content_for_the_floor() {
    check_boxes(&[(
        "the second column",
        &table_in_grid(
            "grid-template-columns: 80px 120px",
            "grid-column: 2; table-layout: fixed; width: 100px; max-width: 5px; \
             border: 2px solid #00f",
            "<div style='width: 200px; height: 10px'></div>",
        ),
        &[("t", 80.0, 0.0, 9.0, 14.0)],
    )]);
}

#[test]
fn the_floor_is_the_min_content_width_of_every_column_and_the_caption() {
    check_boxes(&[
        (
            "two columns",
            "<div style='position: relative; width: 200px; height: 100px'>\
             <div id=t style='display: table; position: absolute; top: 0; left: 0; \
             width: 60px; max-width: 5px'><div style='display: table-row'>\
             <div id=d style='display: table-cell'><div style='width: 10px; height: 10px'></div></div>\
             <div id=e style='display: table-cell'><div style='width: 10px; height: 10px'></div></div>\
             </div></div></div>",
            &[
                ("t", 0.0, 0.0, 20.0, 10.0),
                ("d", 0.0, 0.0, 10.0, 10.0),
                ("e", 10.0, 0.0, 10.0, 10.0),
            ],
        ),
        (
            "a caption wider than the cell",
            "<div style='position: relative; width: 200px; height: 100px'>\
             <div id=t style='display: table; position: absolute; top: 0; left: 0; \
             max-width: 5px'><div id=c style='display: table-caption'>aaaaaaaaaa</div>\
             <div style='display: table-row'><div id=d style='display: table-cell'>\
             <div style='width: 10px; height: 10px'></div></div></div></div></div>",
            &[("c", 0.0, 0.0, 100.0, 10.0), ("d", 0.0, 10.0, 100.0, 10.0)],
        ),
    ]);
}

/// The width of the table box only: the column of a fixed table is still
/// sized from its `width`, not from the `max-width` that cuts the box.
#[test]
fn a_fixed_table_with_a_width_ignores_its_content_for_the_floor() {
    check_boxes(&[
        (
            "a content wider than both limits",
            &abspos_table(
                "table-layout: fixed; width: 100px; max-width: 50px",
                "<div style='width: 200px; height: 10px'></div>",
            ),
            &[("t", 0.0, 0.0, 50.0, 10.0)],
        ),
        (
            "a content wider than the width and the limit",
            &abspos_table(
                "table-layout: fixed; width: 40px; max-width: 5px",
                "<div style='width: 200px; height: 10px'></div>",
            ),
            &[("t", 0.0, 0.0, 5.0, 10.0)],
        ),
        (
            "borders",
            &abspos_table(
                "table-layout: fixed; border: 2px solid #00f; width: 100px; max-width: 50px",
                "<div style='width: 200px; height: 10px'></div>",
            ),
            &[("t", 0.0, 0.0, 54.0, 14.0)],
        ),
        (
            "no width: the automatic algorithm applies",
            &abspos_table(
                "table-layout: fixed; max-width: 50px",
                "<div style='width: 200px; height: 10px'></div>",
            ),
            &[("t", 0.0, 0.0, 200.0, 10.0)],
        ),
    ]);
}

#[test]
fn percentage_padding_of_an_absolute_table_uses_the_containing_inline_size() {
    for mode in ["vertical-rl", "vertical-lr"] {
        let outer = format!(
            "<div style='position:relative;width:200px;height:100px;writing-mode:{mode}'>{{}}</div>"
        );
        check_boxes(&[(
            mode,
            &table_in(&outer, "box-sizing:content-box;padding:10%;max-width:5px"),
            &[("t", 0.0, 0.0, 30.0, 30.0), ("d", 10.0, 10.0, 10.0, 10.0)],
        )]);
    }
}

#[test]
fn percentage_padding_and_width_limits_keep_their_own_containing_bases() {
    let oracle: serde_json::Value =
        serde_json::from_str(include_str!("data/table-absolute-padding.json"))
            .expect("Chromium oracle");
    let source = oracle["cases"].as_array().expect("cases");
    let expected: Vec<Vec<_>> = source
        .iter()
        .map(|case| {
            case["boxes"]
                .as_array()
                .expect("boxes")
                .iter()
                .map(|entry| {
                    let values = entry.as_array().expect("box");
                    let number = |index: usize| values[index].as_f64().expect("coordinate") as f32;
                    (
                        values[0].as_str().expect("id"),
                        number(1),
                        number(2),
                        number(3),
                        number(4),
                    )
                })
                .collect()
        })
        .collect();
    let cases: Vec<Case<'_>> = source
        .iter()
        .zip(&expected)
        .map(|(case, expected)| {
            (
                case["label"].as_str().expect("label"),
                case["body"].as_str().expect("markup"),
                expected.as_slice(),
            )
        })
        .collect();
    check_boxes(&cases);
}
