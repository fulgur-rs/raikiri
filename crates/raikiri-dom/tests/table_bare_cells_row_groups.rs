//! Cells placed straight inside a table form anonymous rows in source order, so
//! a row group between them splits the run into separate rows (CSS 2 §17.2.1).
//! The first header group is then drawn above every row and the first footer
//! group below every row (CSS 2 §17.2), the other groups keep their place.
//!
//! Table parts outside any table get an anonymous table around each run of
//! them, also when a `display: contents` element sits between the parent and the
//! parts (CSS 2 §17.2.1, CSS Display 3 §2.5).
//!
//! The expected boxes are the ones Chromium computes for the same markup in an
//! 800px by 600px viewport, with the body margin of 8px: each is an element's
//! left, top, width and height.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The left, top, width and height of each element in `ids`, on the first page
/// of 800px by 600px.
fn border_boxes(body: &str, ids: &[&str]) -> Vec<[f32; 4]> {
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
    ids.iter()
        .map(|id| {
            page.fragments()
                .find(|fragment| dom.attr(fragment.node(), "id") == Some(id))
                .map(|fragment| {
                    let rect = fragment.rect();
                    [rect.x, rect.y, rect.width, rect.height]
                })
                .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.04;

fn cell(id: &str, text: &str) -> String {
    format!("<div id={id} style=\"display:table-cell\">{text}</div>")
}

fn group(kind: &str, id: &str, inner: &str) -> String {
    format!("<div id={id} style=\"display:table-{kind}\">{inner}</div>")
}

fn table(inner: &str) -> String {
    format!("<div id=t style=\"display:table\">{inner}</div>")
}

/// Checks the boxes of the elements in `expected`, given as an id and the left,
/// top, width and height Chromium gives it.
fn check(label: &str, body: &str, expected: &[(&str, [f32; 4])]) {
    let ids: Vec<&str> = expected.iter().map(|&(id, _)| id).collect();
    let found = border_boxes(body, &ids);
    for ((id, want), got) in expected.iter().zip(found) {
        assert!(
            got.iter()
                .zip(want)
                .all(|(got, want)| (got - want).abs() <= TOLERANCE),
            "{label}: #{id} is {got:?}, expected {want:?} (left, top, width, height)"
        );
    }
}

#[test]
fn bare_cells_on_both_sides_of_a_header_group_form_two_rows() {
    let body = table(&format!(
        "{}{}{}{}{}",
        cell("a", "a"),
        cell("b", "b"),
        group("header-group", "H", &cell("h", "h")),
        cell("e", "e"),
        cell("f", "f"),
    ));
    check(
        "header between cell pairs",
        &body,
        &[
            ("t", [8.0, 8.0, 20.0, 30.0]),
            ("H", [8.0, 8.0, 20.0, 10.0]),
            ("h", [8.0, 8.0, 10.0, 10.0]),
            ("a", [8.0, 18.0, 10.0, 10.0]),
            ("b", [18.0, 18.0, 10.0, 10.0]),
            ("e", [8.0, 28.0, 10.0, 10.0]),
            ("f", [18.0, 28.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn bare_cells_on_both_sides_of_a_footer_group_form_two_rows() {
    let body = table(&format!(
        "{}{}{}{}{}",
        cell("a", "a"),
        cell("b", "b"),
        group("footer-group", "F", &cell("g", "g")),
        cell("e", "e"),
        cell("f", "f"),
    ));
    check(
        "footer between cell pairs",
        &body,
        &[
            ("t", [8.0, 8.0, 20.0, 30.0]),
            ("a", [8.0, 8.0, 10.0, 10.0]),
            ("b", [18.0, 8.0, 10.0, 10.0]),
            ("e", [8.0, 18.0, 10.0, 10.0]),
            ("f", [18.0, 18.0, 10.0, 10.0]),
            ("F", [8.0, 28.0, 20.0, 10.0]),
            ("g", [8.0, 28.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn a_header_and_a_footer_group_each_split_the_cells_around_them() {
    let body = table(&format!(
        "{}{}{}{}{}",
        cell("a", "a"),
        group("footer-group", "F", &cell("g", "g")),
        cell("b", "b"),
        group("header-group", "H", &cell("h", "h")),
        cell("e", "e"),
    ));
    check(
        "footer then header between single cells",
        &body,
        &[
            ("t", [8.0, 8.0, 10.0, 50.0]),
            ("H", [8.0, 8.0, 10.0, 10.0]),
            ("h", [8.0, 8.0, 10.0, 10.0]),
            ("a", [8.0, 18.0, 10.0, 10.0]),
            ("b", [8.0, 28.0, 10.0, 10.0]),
            ("e", [8.0, 38.0, 10.0, 10.0]),
            ("F", [8.0, 48.0, 10.0, 10.0]),
            ("g", [8.0, 48.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn only_the_first_header_group_moves_to_the_top() {
    let body = table(&format!(
        "{}{}{}{}{}",
        cell("a", "a"),
        group("header-group", "H", &cell("h", "h")),
        cell("b", "b"),
        group("header-group", "H2", &cell("i", "i")),
        cell("e", "e"),
    ));
    check(
        "two header groups",
        &body,
        &[
            ("t", [8.0, 8.0, 10.0, 50.0]),
            ("H", [8.0, 8.0, 10.0, 10.0]),
            ("a", [8.0, 18.0, 10.0, 10.0]),
            ("b", [8.0, 28.0, 10.0, 10.0]),
            ("H2", [8.0, 38.0, 10.0, 10.0]),
            ("i", [8.0, 38.0, 10.0, 10.0]),
            ("e", [8.0, 48.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn display_contents_wrappers_do_not_join_the_rows_around_a_group() {
    let wrapped_group = format!(
        "<div style=\"display:contents\">{}</div>",
        group("header-group", "H", &cell("h", "h"))
    );
    check(
        "wrapped header group",
        &table(&format!(
            "{}{}{}{}{}",
            cell("a", "a"),
            cell("b", "b"),
            wrapped_group,
            cell("e", "e"),
            cell("f", "f"),
        )),
        &[
            ("t", [8.0, 8.0, 20.0, 30.0]),
            ("h", [8.0, 8.0, 10.0, 10.0]),
            ("a", [8.0, 18.0, 10.0, 10.0]),
            ("b", [18.0, 18.0, 10.0, 10.0]),
            ("e", [8.0, 28.0, 10.0, 10.0]),
            ("f", [18.0, 28.0, 10.0, 10.0]),
        ],
    );
    check(
        "cells and the header group inside one wrapper",
        &table(&format!(
            "{}<div style=\"display:contents\">{}{}{}</div>{}",
            cell("a", "a"),
            cell("b", "b"),
            group("header-group", "H", &cell("h", "h")),
            cell("e", "e"),
            cell("f", "f"),
        )),
        &[
            ("t", [8.0, 8.0, 20.0, 30.0]),
            ("h", [8.0, 8.0, 10.0, 10.0]),
            ("a", [8.0, 18.0, 10.0, 10.0]),
            ("b", [18.0, 18.0, 10.0, 10.0]),
            ("e", [8.0, 28.0, 10.0, 10.0]),
            ("f", [18.0, 28.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn text_on_both_sides_of_a_header_group_forms_two_rows() {
    // The two letters become anonymous cells in rows of their own, below the
    // header group: three rows of one 10px cell each.
    let body = table(&format!(
        "x{}y",
        group("header-group", "H", &cell("h", "h"))
    ));
    check(
        "text around a header group",
        &body,
        &[
            ("t", [8.0, 8.0, 10.0, 30.0]),
            ("H", [8.0, 8.0, 10.0, 10.0]),
            ("h", [8.0, 8.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn column_widths_are_shared_by_the_rows_either_side_of_a_group() {
    let body = table(&format!(
        "{}{}{}",
        cell("a", "a"),
        group("header-group", "H", &cell("h", "hh")),
        cell("e", "eee"),
    ));
    check(
        "unequal cell widths",
        &body,
        &[
            ("t", [8.0, 8.0, 30.0, 30.0]),
            ("H", [8.0, 8.0, 30.0, 10.0]),
            ("h", [8.0, 8.0, 30.0, 10.0]),
            ("a", [8.0, 18.0, 30.0, 10.0]),
            ("e", [8.0, 28.0, 30.0, 10.0]),
        ],
    );
}

#[test]
fn a_body_group_or_a_row_between_bare_cells_keeps_its_place() {
    check(
        "row group",
        &table(&format!(
            "{}{}{}",
            cell("a", "a"),
            group("row-group", "B", &cell("g", "g")),
            cell("e", "e"),
        )),
        &[
            ("t", [8.0, 8.0, 10.0, 30.0]),
            ("a", [8.0, 8.0, 10.0, 10.0]),
            ("B", [8.0, 18.0, 10.0, 10.0]),
            ("e", [8.0, 28.0, 10.0, 10.0]),
        ],
    );
    check(
        "row",
        &table(&format!(
            "{}{}{}",
            cell("a", "a"),
            group("row", "R", &cell("g", "g")),
            cell("e", "e"),
        )),
        &[
            ("t", [8.0, 8.0, 10.0, 30.0]),
            ("a", [8.0, 8.0, 10.0, 10.0]),
            ("R", [8.0, 18.0, 10.0, 10.0]),
            ("e", [8.0, 28.0, 10.0, 10.0]),
        ],
    );
}

fn wrapper(inner: &str) -> String {
    format!("<div style=\"display:contents\">{inner}</div>")
}

fn plain(inner: &str) -> String {
    format!("<div id=p>{inner}</div>")
}

#[test]
fn a_row_outside_a_table_gets_an_anonymous_table_also_through_display_contents() {
    // The row's text becomes an anonymous cell of 20px; the cell after it adds 10px.
    let row = group("row", "a", &format!("2a{}", cell("b", "2")));
    let expected: &[(&str, [f32; 4])] = &[
        ("a", [8.0, 8.0, 30.0, 10.0]),
        ("b", [28.0, 8.0, 10.0, 10.0]),
    ];
    check("row", &plain(&row), expected);
    check("row in a wrapper", &plain(&wrapper(&row)), expected);
}

#[test]
fn a_cell_outside_a_table_is_as_wide_as_its_content_also_through_display_contents() {
    let expected: &[(&str, [f32; 4])] = &[("a", [8.0, 8.0, 10.0, 10.0])];
    check("cell", &plain(&cell("a", "3")), expected);
    check(
        "cell in a wrapper",
        &plain(&wrapper(&cell("a", "3"))),
        expected,
    );
}

#[test]
fn cells_outside_a_table_share_one_anonymous_row() {
    let expected: &[(&str, [f32; 4])] = &[
        ("a", [8.0, 8.0, 20.0, 10.0]),
        ("b", [28.0, 8.0, 20.0, 10.0]),
    ];
    let both = format!("{}{}", cell("a", "5a"), cell("b", "5b"));
    check("two cells", &plain(&both), expected);
    check(
        "the first cell in a wrapper",
        &plain(&format!("{}{}", wrapper(&cell("a", "5a")), cell("b", "5b"))),
        expected,
    );
    check(
        "wrappers nested around the first cell",
        &plain(&format!(
            "{}{}",
            wrapper(&wrapper(&cell("a", "5a"))),
            cell("b", "5b")
        )),
        expected,
    );
    check(
        "whitespace between the cells",
        &plain(&format!(" {} {} ", cell("a", "5a"), cell("b", "5b"))),
        expected,
    );
}

#[test]
fn a_caption_outside_a_table_is_as_wide_as_its_content() {
    check(
        "caption",
        &plain(&group("caption", "a", "1")),
        &[("a", [8.0, 8.0, 10.0, 10.0])],
    );
}

#[test]
fn a_caption_before_a_row_outside_a_table_sits_above_it_also_through_display_contents() {
    let caption = group("caption", "a", "1");
    let row = group("row", "b", &format!("2a{}", cell("c", "2")));
    let expected: &[(&str, [f32; 4])] = &[
        ("a", [8.0, 8.0, 30.0, 10.0]),
        ("b", [8.0, 18.0, 30.0, 10.0]),
        ("c", [28.0, 18.0, 10.0, 10.0]),
    ];
    check(
        "caption and row",
        &plain(&format!("{caption}{row}")),
        expected,
    );
    check(
        "caption and row in separate wrappers",
        &plain(&format!("{}{}", wrapper(&caption), wrapper(&row))),
        expected,
    );
}

#[test]
fn a_caption_after_a_cell_outside_a_table_is_drawn_above_it() {
    check(
        "cell then caption",
        &plain(&format!(
            "{}{}",
            cell("a", "5a"),
            group("caption", "b", "5b")
        )),
        &[
            ("a", [8.0, 18.0, 20.0, 10.0]),
            ("b", [8.0, 8.0, 20.0, 10.0]),
        ],
    );
}

#[test]
fn a_cell_followed_by_a_row_outside_a_table_forms_a_row_above_it() {
    check(
        "cell then row",
        &plain(&format!(
            "{}{}",
            cell("a", "5a"),
            group("row", "b", &cell("c", "5c"))
        )),
        &[
            ("a", [8.0, 8.0, 20.0, 10.0]),
            ("b", [8.0, 18.0, 20.0, 10.0]),
            ("c", [8.0, 18.0, 20.0, 10.0]),
        ],
    );
}

#[test]
fn a_row_group_outside_a_table_gets_an_anonymous_table() {
    let inner = group("row", "b", &cell("c", "5c"));
    check(
        "row group",
        &plain(&group("row-group", "g", &inner)),
        &[
            ("g", [8.0, 8.0, 20.0, 10.0]),
            ("b", [8.0, 8.0, 20.0, 10.0]),
            ("c", [8.0, 8.0, 20.0, 10.0]),
        ],
    );
}

#[test]
fn a_block_between_cells_outside_a_table_splits_them_into_two_anonymous_tables() {
    check(
        "block after a cell",
        &plain(&format!("<div id=x>xx</div>{}", cell("a", "5a"))),
        &[
            ("x", [8.0, 8.0, 784.0, 10.0]),
            ("a", [8.0, 18.0, 20.0, 10.0]),
        ],
    );
    check(
        "block between two cells",
        &plain(&format!(
            "{}<div id=x>xx</div>{}",
            cell("a", "5a"),
            cell("b", "5b")
        )),
        &[
            ("a", [8.0, 8.0, 20.0, 10.0]),
            ("x", [8.0, 18.0, 784.0, 10.0]),
            ("b", [8.0, 28.0, 20.0, 10.0]),
        ],
    );
}

#[test]
fn a_caption_alone_outside_a_table_sits_below_the_block_before_it() {
    let caption = "<div id=cap style=\"display:table-caption;width:100px;height:30px\"></div>";
    check(
        "block then caption",
        &plain(&format!("<div id=x>xx</div>{caption}")),
        &[
            ("x", [8.0, 8.0, 784.0, 10.0]),
            ("cap", [8.0, 18.0, 100.0, 30.0]),
        ],
    );
    check(
        "block, caption, block",
        &plain(&format!("<div id=x>xx</div>{caption}<div id=y>yy</div>")),
        &[
            ("x", [8.0, 8.0, 784.0, 10.0]),
            ("cap", [8.0, 18.0, 100.0, 30.0]),
            ("y", [8.0, 48.0, 784.0, 10.0]),
        ],
    );
}

#[test]
fn a_caption_with_layout_containment_outside_a_table_holds_its_positioned_content() {
    // The paragraph's margin collapses with the body's, so it starts at 10px.
    check(
        "caption with layout containment",
        "<p id=p>Test passes</p><div id=cap style=\"display:table-caption;contain:layout;\
         width:100px;height:100px\"><div id=abs style=\"position:absolute;bottom:0;right:0;\
         width:100px;height:100px\"></div></div>",
        &[
            ("p", [8.0, 10.0, 784.0, 10.0]),
            ("cap", [8.0, 30.0, 100.0, 100.0]),
            ("abs", [8.0, 30.0, 100.0, 100.0]),
        ],
    );
}

fn vertical(mode: &str, inner: &str) -> String {
    format!("<div id=p style=\"writing-mode:{mode}\">{inner}</div>")
}

fn two_cell_row(id: &str) -> String {
    group(
        "row",
        id,
        &format!("{}{}", cell("c1", "aa"), cell("c2", "b")),
    )
}

#[test]
fn a_row_in_a_vertical_block_gets_an_anonymous_table() {
    // The cells follow each other down the block, and the row runs across it.
    let expected: &[(&str, [f32; 4])] = &[
        ("p", [8.0, 8.0, 10.0, 30.0]),
        ("r", [8.0, 8.0, 10.0, 30.0]),
        ("c1", [8.0, 8.0, 10.0, 20.0]),
        ("c2", [8.0, 28.0, 10.0, 10.0]),
    ];
    for mode in ["vertical-rl", "vertical-lr"] {
        check(mode, &vertical(mode, &two_cell_row("r")), expected);
    }
    // The block it sits in is vertical as well.
    check(
        "nested vertical block",
        &format!(
            "<div id=o style=\"writing-mode:vertical-rl\">{}</div>",
            vertical("vertical-rl", &two_cell_row("r"))
        ),
        &[
            ("o", [8.0, 8.0, 10.0, 30.0]),
            ("p", [8.0, 8.0, 10.0, 30.0]),
            ("r", [8.0, 8.0, 10.0, 30.0]),
            ("c1", [8.0, 8.0, 10.0, 20.0]),
            ("c2", [8.0, 28.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn rows_in_a_vertical_block_share_the_columns_of_one_table() {
    // The second row has one cell, which takes the 20px of the first column.
    let rows = format!(
        "{}{}",
        two_cell_row("r1"),
        group("row", "r2", &cell("c3", "c")),
    );
    check(
        "two rows",
        &vertical("vertical-rl", &rows),
        &[
            ("p", [8.0, 8.0, 20.0, 30.0]),
            ("r1", [18.0, 8.0, 10.0, 30.0]),
            ("c1", [18.0, 8.0, 10.0, 20.0]),
            ("c2", [18.0, 28.0, 10.0, 10.0]),
            ("r2", [8.0, 8.0, 10.0, 30.0]),
            ("c3", [8.0, 8.0, 10.0, 20.0]),
        ],
    );
}

#[test]
fn cells_in_a_vertical_block_share_one_anonymous_row() {
    let cells = format!("{}{}", cell("c1", "aa"), cell("c2", "b"));
    check(
        "cells in a vertical block",
        &vertical("vertical-rl", &cells),
        &[
            ("p", [8.0, 8.0, 10.0, 30.0]),
            ("c1", [8.0, 8.0, 10.0, 20.0]),
            ("c2", [8.0, 28.0, 10.0, 10.0]),
        ],
    );
    // Beside a horizontal block, which is not part of the table.
    check(
        "vertical block in a horizontal block",
        &format!(
            "<div id=o>{}<div id=q>x</div></div>",
            vertical("vertical-lr", &cells)
        ),
        &[
            ("o", [8.0, 8.0, 784.0, 40.0]),
            ("p", [8.0, 8.0, 10.0, 30.0]),
            ("c1", [8.0, 8.0, 10.0, 20.0]),
            ("c2", [8.0, 28.0, 10.0, 10.0]),
            ("q", [8.0, 38.0, 784.0, 10.0]),
        ],
    );
}

#[test]
fn an_anonymous_table_in_a_vertical_block_is_placed_among_its_sibling_blocks() {
    let block = |id: &str| format!("<div id={id} style=\"width:5px;height:15px\"></div>");
    check(
        "blocks either side of a row",
        &vertical(
            "vertical-rl",
            &format!("{}{}{}", block("b1"), two_cell_row("r"), block("b2")),
        ),
        &[
            ("p", [8.0, 8.0, 20.0, 30.0]),
            ("b1", [23.0, 8.0, 5.0, 15.0]),
            ("r", [13.0, 8.0, 10.0, 30.0]),
            ("c1", [13.0, 8.0, 10.0, 20.0]),
            ("c2", [13.0, 28.0, 10.0, 10.0]),
            ("b2", [8.0, 8.0, 5.0, 15.0]),
        ],
    );
    // A block between two cells makes two tables.
    check(
        "block between cells",
        &vertical(
            "vertical-rl",
            &format!(
                "{}<div id=blk style=\"width:5px;height:5px\"></div>{}",
                cell("c1", "aa"),
                cell("c2", "b")
            ),
        ),
        &[
            ("p", [8.0, 8.0, 25.0, 20.0]),
            ("c1", [23.0, 8.0, 10.0, 20.0]),
            ("blk", [18.0, 8.0, 5.0, 5.0]),
            ("c2", [8.0, 8.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn a_caption_and_a_row_group_in_a_vertical_block_join_the_anonymous_table() {
    check(
        "caption beside a row",
        &vertical(
            "vertical-rl",
            &format!(
                "<div id=cap style=\"display:table-caption\">aaa</div>{}",
                two_cell_row("r")
            ),
        ),
        &[
            ("p", [8.0, 8.0, 20.0, 30.0]),
            ("cap", [18.0, 8.0, 10.0, 30.0]),
            ("r", [8.0, 8.0, 10.0, 30.0]),
            ("c1", [8.0, 8.0, 10.0, 20.0]),
            ("c2", [8.0, 28.0, 10.0, 10.0]),
        ],
    );
    check(
        "row group in a wrapper",
        &vertical(
            "vertical-rl",
            &wrapper(&group("row-group", "rg", &two_cell_row("r"))),
        ),
        &[
            ("p", [8.0, 8.0, 10.0, 30.0]),
            ("rg", [8.0, 8.0, 10.0, 30.0]),
            ("r", [8.0, 8.0, 10.0, 30.0]),
            ("c1", [8.0, 8.0, 10.0, 20.0]),
            ("c2", [8.0, 28.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn rows_in_a_list_item_with_an_outside_marker_get_an_anonymous_table() {
    let item = |inner: &str| {
        format!(
            "<div id=p style=\"display:list-item;list-style:outside disc;margin-left:30px\">{inner}</div>"
        )
    };
    check(
        "one row",
        &item(&two_cell_row("r1")),
        &[
            ("p", [38.0, 8.0, 754.0, 10.0]),
            ("r1", [38.0, 8.0, 30.0, 10.0]),
            ("c1", [38.0, 8.0, 20.0, 10.0]),
            ("c2", [58.0, 8.0, 10.0, 10.0]),
        ],
    );
    check(
        "two rows",
        &item(&format!(
            "{}{}",
            two_cell_row("r1"),
            group("row", "r2", &cell("c3", "c"))
        )),
        &[
            ("p", [38.0, 8.0, 754.0, 20.0]),
            ("r1", [38.0, 8.0, 30.0, 10.0]),
            ("c1", [38.0, 8.0, 20.0, 10.0]),
            ("c2", [58.0, 8.0, 10.0, 10.0]),
            ("r2", [38.0, 18.0, 30.0, 10.0]),
            ("c3", [38.0, 18.0, 20.0, 10.0]),
        ],
    );
    // An inside marker that draws nothing leaves the row alone on its line.
    check(
        "inside marker of type none",
        "<div id=p style=\"display:list-item;list-style:none inside\">\
         <div id=r1 style=\"display:table-row\"><div id=c1 style=\"display:table-cell\">aa</div>\
         <div id=c2 style=\"display:table-cell\">b</div></div></div>",
        &[
            ("p", [8.0, 8.0, 784.0, 10.0]),
            ("r1", [8.0, 8.0, 30.0, 10.0]),
            ("c1", [8.0, 8.0, 20.0, 10.0]),
            ("c2", [28.0, 8.0, 10.0, 10.0]),
        ],
    );
    check(
        "cells in a list element",
        &format!(
            "<ul style=\"margin:0;padding-left:30px\"><li id=p>{}{}</li></ul>",
            cell("c1", "aa"),
            cell("c2", "b")
        ),
        &[
            ("p", [38.0, 8.0, 754.0, 10.0]),
            ("c1", [38.0, 8.0, 20.0, 10.0]),
            ("c2", [58.0, 8.0, 10.0, 10.0]),
        ],
    );
}

#[test]
fn an_anonymous_table_in_a_vertical_block_keeps_the_content_of_its_parts() {
    // The row's `::before` text takes an anonymous cell of its own, 20px
    // along the block, in front of the cell the row holds.
    check(
        "generated text of a row",
        "<style>.row::before { content: \"xx\" }</style><div id=p style=\"writing-mode:vertical-rl\">\
         <div id=r class=row style=\"display:table-row\"><div id=c1 style=\"display:table-cell\">b</div></div></div>",
        &[
            ("p", [8.0, 8.0, 10.0, 30.0]),
            ("r", [8.0, 8.0, 10.0, 30.0]),
            ("c1", [8.0, 28.0, 10.0, 10.0]),
        ],
    );
    // A stated height of the block leaves the table its own size.
    check(
        "stated block height",
        "<div id=p style=\"writing-mode:vertical-rl;height:100px\">\
         <div id=r style=\"display:table-row\"><div id=c1 style=\"display:table-cell\">aa</div>\
         <div id=c2 style=\"display:table-cell\">b</div></div></div>",
        &[
            ("p", [8.0, 8.0, 10.0, 100.0]),
            ("r", [8.0, 8.0, 10.0, 30.0]),
            ("c1", [8.0, 8.0, 10.0, 20.0]),
            ("c2", [8.0, 28.0, 10.0, 10.0]),
        ],
    );
    // Cells that state a size stretch their row and column.
    check(
        "stated cell sizes",
        "<div id=p style=\"writing-mode:vertical-lr\"><div id=rg style=\"display:table-row-group\">\
         <div id=r1 style=\"display:table-row\"><div id=c1 style=\"display:table-cell\">a</div>\
         <div id=c2 style=\"display:table-cell;height:25px\">b</div></div>\
         <div id=r2 style=\"display:table-row\"><div id=c3 style=\"display:table-cell;width:15px\">c</div></div>\
         </div></div>",
        &[
            ("p", [8.0, 8.0, 25.0, 35.0]),
            ("rg", [8.0, 8.0, 25.0, 35.0]),
            ("r1", [8.0, 8.0, 10.0, 35.0]),
            ("c1", [8.0, 8.0, 10.0, 10.0]),
            ("c2", [8.0, 18.0, 10.0, 25.0]),
            ("r2", [18.0, 8.0, 15.0, 35.0]),
            ("c3", [18.0, 8.0, 15.0, 10.0]),
        ],
    );
}

#[test]
fn a_row_beside_a_block_with_text_in_a_vertical_block_keeps_its_own_text() {
    // The text of the block makes the vertical parent lay its children out as
    // lines, where the row stays a block that lays out its own text.
    check(
        "block with text",
        &vertical(
            "vertical-rl",
            "<div id=b>x</div><div id=r style=\"display:table-row\">row text</div>",
        ),
        &[
            ("p", [8.0, 8.0, 20.0, 80.0]),
            ("b", [18.0, 8.0, 10.0, 80.0]),
            ("r", [8.0, 8.0, 10.0, 80.0]),
        ],
    );
    check(
        "nested block with text",
        &vertical(
            "vertical-rl",
            &format!(
                "<div id=b><div id=d>x</div></div>{}",
                group("row", "r", &cell("c1", "aa"))
            ),
        ),
        &[
            ("p", [8.0, 8.0, 20.0, 20.0]),
            ("b", [18.0, 8.0, 10.0, 20.0]),
            ("d", [18.0, 8.0, 10.0, 20.0]),
            ("r", [8.0, 8.0, 10.0, 20.0]),
            ("c1", [8.0, 8.0, 10.0, 20.0]),
        ],
    );
}
