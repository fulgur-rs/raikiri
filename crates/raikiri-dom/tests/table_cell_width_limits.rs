//! The width a table cell states, within its own `max-width`, and how a fixed
//! layout table shares its width among the columns: a cell's `width` is the
//! content box's (the column takes the border box), the room a table leaves is
//! shared in proportion to the columns that state a width, and a cell's
//! `max-width` caps the `width` it states (CSS 2.1 sections 17.5.2.1 and
//! 17.5.2.2).
//!
//! The expected boxes are the ones Chromium computes for the same markup, with
//! no body margin and `10px Ahem` text: each is the element's left, top, width
//! and height. The long word is 180px wide and cannot be broken.

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

type Expected = &'static [(&'static str, f32, f32, f32, f32)];

/// Lays out `html` and appends to `mismatches` each element of `expected`
/// whose box differs.
fn check_boxes(mismatches: &mut Vec<String>, label: &str, html: &str, expected: Expected) {
    let actual = boxes(html);
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

/// The cell and table styles every case shares: separate borders, 2px
/// spacing, and 5px of padding and a 1px border around each cell.
const STYLE: &str = "<style>table { border-collapse: separate; border-spacing: 2px } \
                     td { padding: 5px; border: 1px solid }</style>";

const WORD: &str = "Supersuperlongword";

/// A one-row table with `table_style` and the cells `cells` (the markup of
/// each `td`, ids and styles included).
fn row(table_style: &str, cells: &[String]) -> String {
    format!(
        "{STYLE}<table id=t style='{table_style}'><tr>{}</tr></table>",
        cells.concat()
    )
}

/// A one-row table like [`row`] with the `<col>` elements `cols` before it.
fn row_with_cols(table_style: &str, cols: &str, cells: &[String]) -> String {
    format!(
        "{STYLE}<table id=t style='{table_style}'>{cols}<tr>{}</tr></table>",
        cells.concat()
    )
}

fn cell(id: &str, style: &str, text: &str) -> String {
    format!("<td id={id} style='{style}'>{text}</td>")
}

const FIXED_300: &str = "table-layout: fixed; width: 300px";

#[test]
fn a_cells_max_width_caps_the_width_it_states_in_a_table_with_automatic_layout() {
    let left = || cell("a", "", "Row 1 Left");
    let cases: Vec<(&str, String, Expected)> = vec![
        (
            "a max-width under the width",
            row(
                "",
                &[left(), cell("c", "width: 100px; max-width: 60px", WORD)],
            ),
            &[
                ("t", 0.0, 0.0, 190.0, 26.0),
                ("a", 2.0, 2.0, 112.0, 22.0),
                ("c", 116.0, 2.0, 72.0, 22.0),
            ],
        ),
        (
            "a max-width over the width",
            row(
                "",
                &[left(), cell("c", "width: 100px; max-width: 150px", WORD)],
            ),
            &[
                ("t", 0.0, 0.0, 280.0, 26.0),
                ("a", 2.0, 2.0, 112.0, 22.0),
                ("c", 116.0, 2.0, 162.0, 22.0),
            ],
        ),
        (
            "a max-width and no width",
            row("", &[left(), cell("c", "max-width: 60px", WORD)]),
            &[
                ("t", 0.0, 0.0, 190.0, 26.0),
                ("a", 2.0, 2.0, 112.0, 22.0),
                ("c", 116.0, 2.0, 72.0, 22.0),
            ],
        ),
        (
            "a max-width in each of two cells",
            row(
                "",
                &[
                    cell("a", "width: 200px; max-width: 50px", "Row 1 Left"),
                    cell("c", "width: 100px; max-width: 60px", WORD),
                ],
            ),
            &[
                ("t", 0.0, 0.0, 140.0, 36.0),
                ("a", 2.0, 2.0, 62.0, 32.0),
                ("c", 66.0, 2.0, 72.0, 32.0),
            ],
        ),
        (
            "a min-width over the max-width",
            row(
                "",
                &[
                    cell(
                        "a",
                        "width: 100px; max-width: 50px; min-width: 80px",
                        "Row 1 Left",
                    ),
                    cell("c", "", "x"),
                ],
            ),
            &[
                ("t", 0.0, 0.0, 120.0, 36.0),
                ("a", 2.0, 2.0, 92.0, 32.0),
                ("c", 96.0, 2.0, 22.0, 32.0),
            ],
        ),
    ];
    let mut mismatches = Vec::new();
    for (label, html, expected) in &cases {
        check_boxes(&mut mismatches, label, html, expected);
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn the_width_a_cell_states_in_a_fixed_table_is_its_content_box() {
    let left = || cell("a", "", "Row 1 Left");
    let cases: Vec<(&str, String, Expected)> = vec![
        (
            "the column takes the padding and border on top of the width",
            row(FIXED_300, &[left(), cell("c", "width: 100px", WORD)]),
            &[
                ("t", 0.0, 0.0, 300.0, 26.0),
                ("a", 2.0, 2.0, 182.0, 22.0),
                ("c", 186.0, 2.0, 112.0, 22.0),
            ],
        ),
        (
            "two columns that state no width share what is left",
            row(
                FIXED_300,
                &[left(), cell("b", "", "x"), cell("c", "width: 100px", WORD)],
            ),
            &[
                ("t", 0.0, 0.0, 300.0, 36.0),
                ("a", 2.0, 2.0, 90.0, 32.0),
                ("b", 94.0, 2.0, 90.0, 32.0),
                ("c", 186.0, 2.0, 112.0, 32.0),
            ],
        ),
        (
            "a border-box cell states its border box",
            row(
                FIXED_300,
                &[
                    left(),
                    cell("c", "box-sizing: border-box; width: 100px", WORD),
                ],
            ),
            &[
                ("t", 0.0, 0.0, 300.0, 26.0),
                ("a", 2.0, 2.0, 194.0, 22.0),
                ("c", 198.0, 2.0, 100.0, 22.0),
            ],
        ),
        (
            "a border-box cell stays as wide as its padding and border",
            row(
                FIXED_300,
                &[
                    cell("a", "box-sizing: border-box; width: 5px", "Row 1 Left"),
                    cell("c", "", "x"),
                ],
            ),
            &[
                ("t", 0.0, 0.0, 300.0, 46.0),
                ("a", 2.0, 2.0, 12.0, 42.0),
                ("c", 16.0, 2.0, 282.0, 42.0),
            ],
        ),
        (
            "a percentage that finds no room beside the lengths takes none",
            row(
                "table-layout: fixed; width: 200px",
                &[
                    cell("a", "width: 150px", "x"),
                    cell("b", "width: 50%", "y"),
                    cell("c", "width: 100px", "z"),
                ],
            ),
            &[
                ("t", 0.0, 0.0, 282.0, 26.0),
                ("a", 2.0, 2.0, 162.0, 22.0),
                ("c", 168.0, 2.0, 112.0, 22.0),
            ],
        ),
        (
            "a max-width caps the width",
            row(
                FIXED_300,
                &[
                    cell("a", "width: 100px; max-width: 40px", "Row 1 Left"),
                    cell("c", "", "x"),
                ],
            ),
            &[
                ("t", 0.0, 0.0, 300.0, 46.0),
                ("a", 2.0, 2.0, 52.0, 42.0),
                ("c", 56.0, 2.0, 242.0, 42.0),
            ],
        ),
        (
            "a percentage is of the room the table has, then the padding and border",
            row(
                FIXED_300,
                &[cell("a", "width: 20%", "Row 1 Left"), cell("c", "", WORD)],
            ),
            &[
                ("t", 0.0, 0.0, 300.0, 36.0),
                ("a", 2.0, 2.0, 70.8, 32.0),
                ("c", 74.8, 2.0, 223.2, 32.0),
            ],
        ),
        (
            "two percentages and a column that states none",
            row(
                FIXED_300,
                &[
                    cell("a", "width: 30%", "Row 1 Left"),
                    cell("b", "width: 20%", "y"),
                    cell("c", "", "x"),
                ],
            ),
            &[
                ("t", 0.0, 0.0, 300.0, 36.0),
                ("a", 2.0, 2.0, 99.6, 32.0),
                ("b", 103.6, 2.0, 70.4, 32.0),
                ("c", 176.0, 2.0, 122.0, 32.0),
            ],
        ),
        (
            "percentages that fill the table take no padding on top",
            row(
                FIXED_300,
                &[
                    cell("a", "width: 50%", "Row 1 Left"),
                    cell("c", "width: 50%", "x"),
                ],
            ),
            &[
                ("t", 0.0, 0.0, 300.0, 26.0),
                ("a", 2.0, 2.0, 147.0, 22.0),
                ("c", 151.0, 2.0, 147.0, 22.0),
            ],
        ),
        (
            "percentages past the table are cut back to it",
            row(
                FIXED_300,
                &[
                    cell("a", "width: 70%", "Row 1 Left"),
                    cell("c", "width: 70%", "x"),
                ],
            ),
            &[
                ("t", 0.0, 0.0, 300.0, 26.0),
                ("a", 2.0, 2.0, 147.0, 22.0),
                ("c", 151.0, 2.0, 147.0, 22.0),
            ],
        ),
        (
            "a percentage gives way to a length beside it",
            row(
                FIXED_300,
                &[
                    cell("a", "width: 60%", "Row 1 Left"),
                    cell("c", "width: 100px", "x"),
                ],
            ),
            &[
                ("t", 0.0, 0.0, 300.0, 26.0),
                ("a", 2.0, 2.0, 182.0, 22.0),
                ("c", 186.0, 2.0, 112.0, 22.0),
            ],
        ),
        (
            "columns that state widths past the table keep them",
            row(
                "table-layout: fixed; width: 100px",
                &[
                    cell("a", "width: 80px", "Row 1 Left"),
                    cell("c", "width: 80px", "x"),
                ],
            ),
            &[
                ("t", 0.0, 0.0, 190.0, 36.0),
                ("a", 2.0, 2.0, 92.0, 32.0),
                ("c", 96.0, 2.0, 92.0, 32.0),
            ],
        ),
    ];
    let mut mismatches = Vec::new();
    for (label, html, expected) in &cases {
        check_boxes(&mut mismatches, label, html, expected);
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn a_fixed_table_shares_the_room_the_columns_leave_in_proportion_to_their_widths() {
    let cases: Vec<(&str, String, Expected)> = vec![
        (
            "every column states a width",
            row(
                FIXED_300,
                &[
                    cell("a", "width: 50px", "Row 1 Left"),
                    cell("c", "width: 100px", WORD),
                ],
            ),
            &[
                ("t", 0.0, 0.0, 300.0, 36.0),
                ("a", 2.0, 2.0, 104.75, 32.0),
                ("c", 108.75, 2.0, 189.25, 32.0),
            ],
        ),
        (
            "a column that states no width takes it instead",
            row(
                FIXED_300,
                &[
                    cell("a", "width: 50px", "Row 1 Left"),
                    cell("b", "", "y"),
                    cell("c", "", "x"),
                ],
            ),
            &[
                ("t", 0.0, 0.0, 300.0, 36.0),
                ("a", 2.0, 2.0, 62.0, 32.0),
                ("b", 66.0, 2.0, 115.0, 32.0),
                ("c", 183.0, 2.0, 115.0, 32.0),
            ],
        ),
    ];
    let mut mismatches = Vec::new();
    for (label, html, expected) in &cases {
        check_boxes(&mut mismatches, label, html, expected);
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn a_percentage_column_keeps_its_share_of_a_fixed_table_while_lengths_take_the_room() {
    let cases: Vec<(&str, String, Expected)> = vec![
        (
            "a length column takes the room beside a percentage",
            row(
                FIXED_300,
                &[
                    cell("a", "width: 20%", "Row 1 Left"),
                    cell("c", "width: 100px", WORD),
                ],
            ),
            &[
                ("t", 0.0, 0.0, 300.0, 36.0),
                ("a", 2.0, 2.0, 70.8, 32.0),
                ("c", 74.8, 2.0, 223.2, 32.0),
            ],
        ),
        (
            "length columns share it in proportion to their widths",
            row(
                FIXED_300,
                &[
                    cell("a", "width: 20%", "Row 1 Left"),
                    cell("b", "width: 100px", "y"),
                    cell("c", "width: 50px", WORD),
                ],
            ),
            &[
                ("t", 0.0, 0.0, 300.0, 36.0),
                ("a", 2.0, 2.0, 70.41, 32.0),
                ("b", 74.41, 2.0, 142.64, 32.0),
                ("c", 219.05, 2.0, 78.95, 32.0),
            ],
        ),
        (
            "percentages alone share it in proportion to their widths",
            row(
                FIXED_300,
                &[
                    cell("a", "width: 20%", "Row 1 Left"),
                    cell("c", "width: 30%", WORD),
                ],
            ),
            &[
                ("t", 0.0, 0.0, 300.0, 26.0),
                ("a", 2.0, 2.0, 121.72, 22.0),
                ("c", 125.72, 2.0, 172.28, 22.0),
            ],
        ),
        (
            "a percentage on a col and one on a cell are both cut back",
            row_with_cols(
                FIXED_300,
                "<col style='width: 70%'><col>",
                &[
                    cell("a", "padding: 0; border: 0", "Row 1 Left"),
                    cell("c", "width: 70%; padding: 0; border: 0", "x"),
                ],
            ),
            &[
                ("t", 0.0, 0.0, 300.0, 14.0),
                ("a", 2.0, 2.0, 147.0, 10.0),
                ("c", 151.0, 2.0, 147.0, 10.0),
            ],
        ),
        (
            "two percentages on cols are cut back",
            row_with_cols(
                FIXED_300,
                "<col style='width: 70%'><col style='width: 70%'>",
                &[cell("a", "", "Row 1 Left"), cell("c", "", "x")],
            ),
            &[
                ("t", 0.0, 0.0, 300.0, 26.0),
                ("a", 2.0, 2.0, 147.0, 22.0),
                ("c", 151.0, 2.0, 147.0, 22.0),
            ],
        ),
        (
            "a percentage on a col gives way to a length on a col",
            row_with_cols(
                FIXED_300,
                "<col style='width: 70%'><col style='width: 100px'>",
                &[cell("a", "", "Row 1 Left"), cell("c", "", "x")],
            ),
            &[
                ("t", 0.0, 0.0, 300.0, 26.0),
                ("a", 2.0, 2.0, 194.0, 22.0),
                ("c", 198.0, 2.0, 100.0, 22.0),
            ],
        ),
    ];
    let mut mismatches = Vec::new();
    for (label, html, expected) in &cases {
        check_boxes(&mut mismatches, label, html, expected);
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}
