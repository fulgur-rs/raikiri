//! A block-level table keeps its border box clear of the margin boxes of the
//! floats in its formatting context (CSS 2.1 section 9.5): it sits beside a
//! float when it fits in the room the float leaves, and below it when it
//! does not. The root of a vertical writing mode follows the same rule along
//! its inline axis, and a plain block child of such a root shares the root's
//! context, so only its lines make room for a float.
//!
//! The expected border boxes are the ones Chromium computes for the same
//! markup in an 800px by 600px viewport, with no margin on the body. The
//! tables carry no spacing, and their cells no padding, unless a case says
//! otherwise.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A border box `(x, y, width, height)`.
type Rect = (f32, f32, f32, f32);

/// The border boxes of the elements of `body` with an `id` on the first page.
fn border_boxes(body: &str) -> Vec<(String, Rect)> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
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
            Some((id.to_owned(), (rect.x, rect.y, rect.width, rect.height)))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.02;

/// Compares the border boxes of the elements named in `expected`.
fn compare(body: &str, expected: &[(&str, Rect)]) {
    let actual = border_boxes(body);
    for &(id, want) in expected {
        let found = actual
            .iter()
            .find(|(name, _)| name == id)
            .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"))
            .1;
        let close = |a: f32, b: f32| (a - b).abs() <= TOLERANCE;
        assert!(
            close(found.0, want.0)
                && close(found.1, want.1)
                && close(found.2, want.2)
                && close(found.3, want.3),
            "{body}: #{id} is {found:?}, expected {want:?}"
        );
    }
}

/// One case of a vertical root: the size of the child `k` and the border
/// boxes of the float `f` and of `k`.
struct Case {
    width: u32,
    height: u32,
    float: Rect,
    child: Rect,
}

/// Lays out a vertical root of 200px by 200px with a float of 100px by 100px
/// on `side`, then a child with the `style` and the size of each case, and
/// compares the boxes of both.
fn check_vertical(writing_mode: &str, side: &str, style: &str, cases: &[Case]) {
    for case in cases {
        let body = format!(
            "<div id=o style=\"writing-mode:{writing_mode};width:200px;height:200px\">\
             <div id=f style=\"float:{side};width:100px;height:100px\"></div>\
             <div id=k style=\"{style}width:{}px;height:{}px\"></div></div>",
            case.width, case.height
        );
        compare(&body, &[("f", case.float), ("k", case.child)]);
    }
}

/// Lays out a container of 200px with a float of 100px by 100px on `side`,
/// then `element` and a block of 5px, and compares the boxes of the float,
/// the element `k`, the block `after` and the container `o`.
fn check_horizontal(side: &str, element: &str, expected: [Rect; 4]) {
    let body = format!(
        "<div id=o style=\"width:200px\">\
         <div id=f style=\"float:{side};width:100px;height:100px\"></div>{element}\
         <div id=after style=\"height:5px\"></div></div>"
    );
    let [float, k, after, o] = expected;
    compare(&body, &[("f", float), ("k", k), ("after", after), ("o", o)]);
}

#[test]
fn a_table_in_a_vertical_rl_root_with_a_left_float() {
    check_vertical(
        "vertical-rl",
        "left",
        "display:table;",
        &[
            Case {
                width: 50,
                height: 20,
                float: (100.0, 0.0, 100.0, 100.0),
                child: (150.0, 100.0, 50.0, 20.0),
            },
            Case {
                width: 150,
                height: 20,
                float: (100.0, 0.0, 100.0, 100.0),
                child: (50.0, 100.0, 150.0, 20.0),
            },
            Case {
                width: 20,
                height: 150,
                float: (100.0, 0.0, 100.0, 100.0),
                child: (80.0, 0.0, 20.0, 150.0),
            },
        ],
    );
}

#[test]
fn a_table_in_a_vertical_rl_root_with_a_right_float() {
    check_vertical(
        "vertical-rl",
        "right",
        "display:table;",
        &[
            Case {
                width: 50,
                height: 20,
                float: (100.0, 100.0, 100.0, 100.0),
                child: (150.0, 0.0, 50.0, 20.0),
            },
            Case {
                width: 150,
                height: 20,
                float: (100.0, 100.0, 100.0, 100.0),
                child: (50.0, 0.0, 150.0, 20.0),
            },
            Case {
                width: 20,
                height: 150,
                float: (100.0, 100.0, 100.0, 100.0),
                child: (80.0, 0.0, 20.0, 150.0),
            },
        ],
    );
}

#[test]
fn a_table_in_a_vertical_lr_root_with_a_left_float() {
    check_vertical(
        "vertical-lr",
        "left",
        "display:table;",
        &[
            Case {
                width: 50,
                height: 20,
                float: (0.0, 0.0, 100.0, 100.0),
                child: (0.0, 100.0, 50.0, 20.0),
            },
            Case {
                width: 150,
                height: 20,
                float: (0.0, 0.0, 100.0, 100.0),
                child: (0.0, 100.0, 150.0, 20.0),
            },
            Case {
                width: 20,
                height: 150,
                float: (0.0, 0.0, 100.0, 100.0),
                child: (100.0, 0.0, 20.0, 150.0),
            },
        ],
    );
}

#[test]
fn a_table_in_a_vertical_lr_root_with_a_right_float() {
    check_vertical(
        "vertical-lr",
        "right",
        "display:table;",
        &[
            Case {
                width: 50,
                height: 20,
                float: (0.0, 100.0, 100.0, 100.0),
                child: (0.0, 0.0, 50.0, 20.0),
            },
            Case {
                width: 150,
                height: 20,
                float: (0.0, 100.0, 100.0, 100.0),
                child: (0.0, 0.0, 150.0, 20.0),
            },
            Case {
                width: 20,
                height: 150,
                float: (0.0, 100.0, 100.0, 100.0),
                child: (100.0, 0.0, 20.0, 150.0),
            },
        ],
    );
}

#[test]
fn a_plain_block_in_a_vertical_rl_root_with_a_left_float() {
    check_vertical(
        "vertical-rl",
        "left",
        "",
        &[
            Case {
                width: 50,
                height: 20,
                float: (100.0, 0.0, 100.0, 100.0),
                child: (150.0, 0.0, 50.0, 20.0),
            },
            Case {
                width: 150,
                height: 20,
                float: (100.0, 0.0, 100.0, 100.0),
                child: (50.0, 0.0, 150.0, 20.0),
            },
            Case {
                width: 20,
                height: 150,
                float: (100.0, 0.0, 100.0, 100.0),
                child: (180.0, 0.0, 20.0, 150.0),
            },
        ],
    );
}

#[test]
fn a_plain_block_in_a_vertical_rl_root_with_a_right_float() {
    check_vertical(
        "vertical-rl",
        "right",
        "",
        &[
            Case {
                width: 50,
                height: 20,
                float: (100.0, 100.0, 100.0, 100.0),
                child: (150.0, 0.0, 50.0, 20.0),
            },
            Case {
                width: 150,
                height: 20,
                float: (100.0, 100.0, 100.0, 100.0),
                child: (50.0, 0.0, 150.0, 20.0),
            },
            Case {
                width: 20,
                height: 150,
                float: (100.0, 100.0, 100.0, 100.0),
                child: (180.0, 0.0, 20.0, 150.0),
            },
        ],
    );
}

#[test]
fn a_plain_block_in_a_vertical_lr_root_with_a_left_float() {
    check_vertical(
        "vertical-lr",
        "left",
        "",
        &[
            Case {
                width: 50,
                height: 20,
                float: (0.0, 0.0, 100.0, 100.0),
                child: (0.0, 0.0, 50.0, 20.0),
            },
            Case {
                width: 150,
                height: 20,
                float: (0.0, 0.0, 100.0, 100.0),
                child: (0.0, 0.0, 150.0, 20.0),
            },
            Case {
                width: 20,
                height: 150,
                float: (0.0, 0.0, 100.0, 100.0),
                child: (0.0, 0.0, 20.0, 150.0),
            },
        ],
    );
}

#[test]
fn a_plain_block_in_a_vertical_lr_root_with_a_right_float() {
    check_vertical(
        "vertical-lr",
        "right",
        "",
        &[
            Case {
                width: 50,
                height: 20,
                float: (0.0, 100.0, 100.0, 100.0),
                child: (0.0, 0.0, 50.0, 20.0),
            },
            Case {
                width: 150,
                height: 20,
                float: (0.0, 100.0, 100.0, 100.0),
                child: (0.0, 0.0, 150.0, 20.0),
            },
            Case {
                width: 20,
                height: 150,
                float: (0.0, 100.0, 100.0, 100.0),
                child: (0.0, 0.0, 20.0, 150.0),
            },
        ],
    );
}

#[test]
fn an_auto_width_table_that_fits_beside_the_float_with_a_left_float() {
    check_horizontal(
        "left",
        "<table id=k style=\"border-spacing:0\"><tr><td style=\"padding:0;width:50px;height:10px\"></td></tr></table>",
        [
            (0.0, 0.0, 100.0, 100.0),
            (100.0, 0.0, 50.0, 10.0),
            (0.0, 10.0, 200.0, 5.0),
            (0.0, 0.0, 200.0, 15.0),
        ],
    );
}

#[test]
fn an_auto_width_table_shrinks_a_cells_preferred_width_to_the_float_slot() {
    for side in ["left", "right"] {
        for content in ["", "<div style=\"width:80px;height:10px\"></div>"] {
            let float_x = if side == "left" { 0.0 } else { 100.0 };
            let table_x = if side == "left" { 100.0 } else { 0.0 };
            let table = format!(
                "<table id=k style=\"border-spacing:0\"><tr><td style=\"padding:0;width:150px;height:10px\">{content}</td></tr></table>"
            );
            check_horizontal(
                side,
                &table,
                [
                    (float_x, 0.0, 100.0, 100.0),
                    (table_x, 0.0, 100.0, 10.0),
                    (0.0, 10.0, 200.0, 5.0),
                    (0.0, 0.0, 200.0, 15.0),
                ],
            );
        }
    }
}

#[test]
fn a_cells_unbreakable_content_keeps_the_table_below_a_narrow_float_slot() {
    for side in ["left", "right"] {
        let float_x = if side == "left" { 0.0 } else { 100.0 };
        check_horizontal(
            side,
            "<table id=k style=\"border-spacing:0\"><tr><td style=\"padding:0;width:150px\"><div style=\"width:150px;height:10px\"></div></td></tr></table>",
            [
                (float_x, 0.0, 100.0, 100.0),
                (0.0, 100.0, 150.0, 10.0),
                (0.0, 110.0, 200.0, 5.0),
                (0.0, 0.0, 200.0, 115.0),
            ],
        );
    }
}

#[test]
fn spanning_content_preserves_the_preferred_column_distribution() {
    for side in ["left", "right"] {
        for (second_style, first_width, second_width) in
            [("", 150.0, 50.0), ("width:150px", 100.0, 100.0)]
        {
            let body = format!(
                "<div id=o style=\"width:200px\">\
                 <div style=\"float:{side};width:100px;height:100px\"></div>\
                 <table id=k style=\"border-spacing:0\"><tr>\
                 <td id=a style=\"padding:0;width:150px;height:10px\"></td>\
                 <td id=b style=\"padding:0;height:10px;{second_style}\"></td></tr>\
                 <tr><td colspan=2 style=\"padding:0\">\
                 <div style=\"width:200px;height:10px\"></div></td></tr></table>\
                 <div id=after style=\"height:5px\"></div></div>"
            );
            compare(
                &body,
                &[
                    ("k", (0.0, 100.0, 200.0, 20.0)),
                    ("a", (0.0, 100.0, first_width, 10.0)),
                    ("b", (first_width, 100.0, second_width, 10.0)),
                    ("after", (0.0, 120.0, 200.0, 5.0)),
                    ("o", (0.0, 0.0, 200.0, 125.0)),
                ],
            );
        }
    }
}

#[test]
fn a_spanning_minimum_fills_length_preferences_before_auto_columns() {
    for side in ["left", "right"] {
        for table_style in ["", "width:min-content"] {
            for (text, row_height) in [("X X X X X", 20.0), ("X X X X X X X X X X", 40.0)] {
                let body = format!(
                    "<div id=o style=\"width:200px\">\
                     <div style=\"float:{side};width:100px;height:100px\"></div>\
                     <table id=k style=\"border-spacing:0;{table_style}\"><tr>\
                     <td id=a style=\"padding:0;height:10px;width:150px\"></td>\
                     <td id=b style=\"padding:0;height:10px\">{text}</td></tr>\
                     <tr><td colspan=2 style=\"padding:0\">\
                     <div style=\"width:200px;height:10px\"></div></td></tr></table>\
                     <div id=after style=\"height:5px\"></div></div>"
                );
                compare(
                    &body,
                    &[
                        ("k", (0.0, 100.0, 200.0, row_height + 10.0)),
                        ("a", (0.0, 100.0, 150.0, row_height)),
                        ("b", (150.0, 100.0, 50.0, row_height)),
                        ("after", (0.0, 110.0 + row_height, 200.0, 5.0)),
                        ("o", (0.0, 0.0, 200.0, 115.0 + row_height)),
                    ],
                );
            }
        }
    }
}

#[test]
fn an_empty_spanning_cell_can_shrink_its_preferred_width() {
    for (side, table_x) in [("left", 100.0), ("right", 0.0)] {
        for (first_style, first_width, second_width) in
            [("width:150px", 100.0, 0.0), ("", 50.0, 50.0)]
        {
            let body = format!(
                "<div id=o style=\"width:200px\">\
                 <div style=\"float:{side};width:100px;height:100px\"></div>\
                 <table id=k style=\"border-spacing:0\"><tr>\
                 <td id=a style=\"padding:0;height:10px;{first_style}\"></td>\
                 <td id=b style=\"padding:0;height:10px\"></td></tr>\
                 <tr><td colspan=2 style=\"padding:0;width:150px;height:10px\"></td></tr></table>\
                 <div id=after style=\"height:5px\"></div></div>"
            );
            compare(
                &body,
                &[
                    ("k", (table_x, 0.0, 100.0, 20.0)),
                    ("a", (table_x, 0.0, first_width, 10.0)),
                    ("b", (table_x + first_width, 0.0, second_width, 10.0)),
                    ("after", (0.0, 20.0, 200.0, 5.0)),
                    ("o", (0.0, 0.0, 200.0, 25.0)),
                ],
            );
        }
    }
}

#[test]
fn cell_lengths_and_percentages_share_a_narrow_float_slot() {
    for (side, table_x) in [("left", 100.0), ("right", 0.0)] {
        let body = format!(
            "<div id=o style=\"width:200px\">\
             <div style=\"float:{side};width:100px;height:100px\"></div>\
             <table id=k style=\"border-spacing:0\"><tr>\
             <td id=a style=\"padding:0;height:10px;width:150px\"></td>\
             <td id=b style=\"padding:0;height:10px;width:50%\"></td></tr></table>\
             <div id=after style=\"height:5px\"></div></div>"
        );
        compare(
            &body,
            &[
                ("k", (table_x, 0.0, 100.0, 10.0)),
                ("a", (table_x, 0.0, 50.0, 10.0)),
                ("b", (table_x + 50.0, 0.0, 50.0, 10.0)),
                ("after", (0.0, 10.0, 200.0, 5.0)),
                ("o", (0.0, 0.0, 200.0, 15.0)),
            ],
        );
    }
}

#[test]
fn a_column_width_and_fixed_layout_cell_width_remain_minimums() {
    for side in ["left", "right"] {
        for table in [
            "<table id=k style=\"border-spacing:0\"><col style=\"width:150px\"><tr><td style=\"padding:0;height:10px\"></td></tr></table>",
            "<table id=k style=\"border-spacing:0;table-layout:fixed;width:50px\"><tr><td style=\"padding:0;width:150px;height:10px\"></td></tr></table>",
            "<table id=k style=\"border-spacing:0\"><tr><td style=\"padding:0;width:150px;min-width:120px;height:10px\"></td></tr></table>",
        ] {
            let float_x = if side == "left" { 0.0 } else { 100.0 };
            check_horizontal(
                side,
                table,
                [
                    (float_x, 0.0, 100.0, 100.0),
                    (0.0, 100.0, 150.0, 10.0),
                    (0.0, 110.0, 200.0, 5.0),
                    (0.0, 0.0, 200.0, 115.0),
                ],
            );
        }
    }
}

#[test]
fn a_cell_minimum_wins_over_a_conflicting_maximum() {
    for side in ["left", "right"] {
        let float_x = if side == "left" { 0.0 } else { 100.0 };
        check_horizontal(
            side,
            "<table id=k style=\"border-spacing:0\"><tr><td style=\"padding:0;height:10px;width:150px;min-width:120px;max-width:80px\"></td></tr></table>",
            [
                (float_x, 0.0, 100.0, 100.0),
                (0.0, 100.0, 120.0, 10.0),
                (0.0, 110.0, 200.0, 5.0),
                (0.0, 0.0, 200.0, 115.0),
            ],
        );
    }
}

#[test]
fn cell_edges_survive_while_a_preferred_width_shrinks() {
    for side in ["left", "right"] {
        for (box_sizing, height) in [("content-box", 40.0), ("border-box", 30.0)] {
            let float_x = if side == "left" { 0.0 } else { 100.0 };
            let table_x = if side == "left" { 100.0 } else { 0.0 };
            let table = format!(
                "<table id=k style=\"border-spacing:0\"><tr><td style=\"width:150px;height:10px;padding:10px;border:5px solid;box-sizing:{box_sizing}\"></td></tr></table>"
            );
            check_horizontal(
                side,
                &table,
                [
                    (float_x, 0.0, 100.0, 100.0),
                    (table_x, 0.0, 100.0, height),
                    (0.0, height, 200.0, 5.0),
                    (0.0, 0.0, 200.0, height + 5.0),
                ],
            );
        }
    }
}

#[test]
fn a_numeric_cell_minimum_uses_its_box_sizing_and_edges() {
    for (side, table_x) in [("left", 100.0), ("right", 0.0)] {
        for (box_sizing, width, height) in
            [("content-box", 100.0, 40.0), ("border-box", 70.0, 30.0)]
        {
            let float_x = if side == "left" { 0.0 } else { 100.0 };
            let table = format!(
                "<table id=k style=\"border-spacing:0;width:min-content\"><tr>\
                 <td style=\"width:150px;min-width:70px;height:10px;padding:10px;border:5px solid;box-sizing:{box_sizing}\"></td></tr></table>"
            );
            check_horizontal(
                side,
                &table,
                [
                    (float_x, 0.0, 100.0, 100.0),
                    (table_x, 0.0, width, height),
                    (0.0, height, 200.0, 5.0),
                    (0.0, 0.0, 200.0, height + 5.0),
                ],
            );
        }
    }
}

#[test]
fn a_cell_maximum_can_limit_its_contribution_while_content_overflows() {
    for (side, table_x) in [("left", 100.0), ("right", 0.0)] {
        let float_x = if side == "left" { 0.0 } else { 100.0 };
        check_horizontal(
            side,
            "<table id=k style=\"border-spacing:0\"><tr>\
             <td style=\"padding:0;height:10px;width:150px;max-width:80px\">\
             <div style=\"width:150px;height:10px\"></div></td></tr></table>",
            [
                (float_x, 0.0, 100.0, 100.0),
                (table_x, 0.0, 80.0, 10.0),
                (0.0, 10.0, 200.0, 5.0),
                (0.0, 0.0, 200.0, 15.0),
            ],
        );
    }
}

#[test]
fn cell_preferred_widths_can_shrink_without_a_float() {
    for (style, width) in [("", 100.0), ("width:min-content", 0.0)] {
        let body = format!(
            "<div id=o style=\"width:100px\">\
             <table id=k style=\"border-spacing:0;{style}\"><tr>\
             <td style=\"padding:0;width:150px;height:10px\"></td></tr></table>\
             <div id=after style=\"height:5px\"></div></div>"
        );
        compare(
            &body,
            &[
                ("k", (0.0, 0.0, width, 10.0)),
                ("after", (0.0, 10.0, 100.0, 5.0)),
                ("o", (0.0, 0.0, 100.0, 15.0)),
            ],
        );
    }
}

#[test]
fn a_tables_intrinsic_floor_is_used_before_choosing_a_slot_beside_a_float() {
    for side in ["left", "right"] {
        for limit in ["width:50px", "max-width:50px"] {
            for element in [
                format!(
                    "<div id=k style=\"display:table;{limit}\"><div style=\"width:150px;height:10px\"></div></div>"
                ),
                format!(
                    "<table id=k style=\"border-spacing:0;{limit}\"><tr><td style=\"padding:0\"><div style=\"width:150px;height:10px\"></div></td></tr></table>"
                ),
            ] {
                let float_x = if side == "left" { 0.0 } else { 100.0 };
                check_horizontal(
                    side,
                    &element,
                    [
                        (float_x, 0.0, 100.0, 100.0),
                        (0.0, 100.0, 150.0, 10.0),
                        (0.0, 110.0, 200.0, 5.0),
                        (0.0, 0.0, 200.0, 115.0),
                    ],
                );
            }
        }
    }
}

#[test]
fn a_table_with_a_small_intrinsic_floor_still_fits_beside_the_float() {
    for side in ["left", "right"] {
        let float_x = if side == "left" { 0.0 } else { 100.0 };
        let table_x = if side == "left" { 100.0 } else { 0.0 };
        check_horizontal(
            side,
            "<div id=k style=\"display:table;width:50px\"><div style=\"width:80px;height:10px\"></div></div>",
            [
                (float_x, 0.0, 100.0, 100.0),
                (table_x, 0.0, 80.0, 10.0),
                (0.0, 10.0, 200.0, 5.0),
                (0.0, 0.0, 200.0, 15.0),
            ],
        );
    }
}

#[test]
fn table_captions_edges_and_stretch_limits_are_resolved_before_float_avoidance() {
    for side in ["left", "right"] {
        for (element, width, height) in [
            (
                "<table id=k style=\"border-spacing:0;max-width:stretch\"><tr><td style=\"padding:0\"><div style=\"width:150px;height:10px\"></div></td></tr></table>",
                150.0,
                10.0,
            ),
            (
                "<table id=k style=\"border-spacing:0;width:stretch\"><tr><td style=\"padding:0\"><div style=\"width:150px;height:10px\"></div></td></tr></table>",
                200.0,
                10.0,
            ),
            (
                "<table id=k style=\"border-spacing:10px;width:50px\"><tr><td style=\"padding:0\"><div style=\"width:140px;height:10px\"></div></td></tr></table>",
                160.0,
                30.0,
            ),
            (
                "<table id=k style=\"border-collapse:collapse;border:4px solid;width:50px\"><tr><td style=\"padding:0\"><div style=\"width:150px;height:10px\"></div></td></tr></table>",
                158.0,
                18.0,
            ),
        ] {
            let float_x = if side == "left" { 0.0 } else { 100.0 };
            check_horizontal(
                side,
                element,
                [
                    (float_x, 0.0, 100.0, 100.0),
                    (0.0, 100.0, width, height),
                    (0.0, 100.0 + height, 200.0, 5.0),
                    (0.0, 0.0, 200.0, 105.0 + height),
                ],
            );
        }
    }
}

#[test]
fn a_caption_minimum_places_the_whole_table_wrapper_below_the_float() {
    for side in ["left", "right"] {
        let body = format!(
            "<div id=o style=\"width:200px\">\
             <div style=\"float:{side};width:100px;height:100px\"></div>\
             <table id=k style=\"border-spacing:0;width:50px\">\
             <caption id=cap><div style=\"width:150px;height:10px\"></div></caption>\
             <tr><td style=\"padding:0;height:10px\"></td></tr></table>\
             <div id=after style=\"height:5px\"></div></div>"
        );
        // The table fragment describes the grid; the caption has its own fragment.
        compare(
            &body,
            &[
                ("cap", (0.0, 100.0, 150.0, 10.0)),
                ("k", (0.0, 110.0, 150.0, 10.0)),
                ("after", (0.0, 120.0, 200.0, 5.0)),
                ("o", (0.0, 0.0, 200.0, 125.0)),
            ],
        );
    }
}

#[test]
fn a_fixed_layout_table_keeps_its_specified_width_when_its_content_overflows() {
    for side in ["left", "right"] {
        let float_x = if side == "left" { 0.0 } else { 100.0 };
        let table_x = if side == "left" { 100.0 } else { 0.0 };
        check_horizontal(
            side,
            "<table id=k style=\"border-spacing:0;table-layout:fixed;width:50px\"><tr><td style=\"padding:0\"><div style=\"width:150px;height:10px\"></div></td></tr></table>",
            [
                (float_x, 0.0, 100.0, 100.0),
                (table_x, 0.0, 50.0, 10.0),
                (0.0, 10.0, 200.0, 5.0),
                (0.0, 0.0, 200.0, 15.0),
            ],
        );
    }
}

#[test]
fn a_specified_table_keeps_its_column_allocation_after_collapse() {
    for side in ["left", "right"] {
        let float_x = if side == "left" { 0.0 } else { 100.0 };
        check_horizontal(
            side,
            "<table id=k style=\"border-spacing:0;table-layout:fixed;width:180px\"><col><col style=\"visibility:collapse\"><col><tr><td style=\"padding:0;height:10px\"></td><td style=\"padding:0;height:10px\"></td><td style=\"padding:0;height:10px\"></td></tr></table>",
            [
                (float_x, 0.0, 100.0, 100.0),
                (0.0, 100.0, 120.0, 10.0),
                (0.0, 110.0, 200.0, 5.0),
                (0.0, 0.0, 200.0, 115.0),
            ],
        );
    }
}

#[test]
fn a_stretched_table_is_measured_and_laid_out_in_the_same_slot_room() {
    for side in ["left", "right"] {
        let float_x = if side == "left" { 0.0 } else { 100.0 };
        let table_x = if side == "left" { 100.0 } else { 0.0 };
        check_horizontal(
            side,
            "<table id=k style=\"border-spacing:0;table-layout:fixed;width:stretch\"><col><col style=\"visibility:collapse\"><tr><td style=\"padding:0;height:10px\"></td><td style=\"padding:0;height:10px\"></td></tr></table>",
            [
                (float_x, 0.0, 100.0, 100.0),
                (table_x, 0.0, 50.0, 10.0),
                (0.0, 10.0, 200.0, 5.0),
                (0.0, 0.0, 200.0, 15.0),
            ],
        );
    }
}

#[test]
fn a_table_made_of_table_display_boxes_with_a_left_float() {
    check_horizontal(
        "left",
        "<div id=k style=\"display:table\"><div style=\"display:table-row\"><div style=\"display:table-cell;width:50px;height:10px\"></div></div></div>",
        [
            (0.0, 0.0, 100.0, 100.0),
            (100.0, 0.0, 50.0, 10.0),
            (0.0, 10.0, 200.0, 5.0),
            (0.0, 0.0, 200.0, 15.0),
        ],
    );
}

#[test]
fn a_left_margin_may_reach_under_the_float_with_a_left_float() {
    check_horizontal(
        "left",
        "<div id=k style=\"display:table;width:60px;height:20px;margin-left:20px\"></div>",
        [
            (0.0, 0.0, 100.0, 100.0),
            (100.0, 0.0, 60.0, 20.0),
            (0.0, 20.0, 200.0, 5.0),
            (0.0, 0.0, 200.0, 25.0),
        ],
    );
}

#[test]
fn a_margin_that_does_not_fit_sends_the_table_below_with_a_left_float() {
    check_horizontal(
        "left",
        "<div id=k style=\"display:table;width:60px;height:20px;margin-left:50px\"></div>",
        [
            (0.0, 0.0, 100.0, 100.0),
            (100.0, 0.0, 60.0, 20.0),
            (0.0, 20.0, 200.0, 5.0),
            (0.0, 0.0, 200.0, 25.0),
        ],
    );
}

#[test]
fn a_percentage_width_resolves_against_the_container_with_a_left_float() {
    check_horizontal(
        "left",
        "<div id=k style=\"display:table;width:40%;height:20px\"></div>",
        [
            (0.0, 0.0, 100.0, 100.0),
            (100.0, 0.0, 80.0, 20.0),
            (0.0, 20.0, 200.0, 5.0),
            (0.0, 0.0, 200.0, 25.0),
        ],
    );
}

#[test]
fn a_min_width_wider_than_the_room_sends_the_table_below_with_a_left_float() {
    check_horizontal(
        "left",
        "<div id=k style=\"display:table;min-width:120px;height:20px\"></div>",
        [
            (0.0, 0.0, 100.0, 100.0),
            (0.0, 100.0, 120.0, 20.0),
            (0.0, 120.0, 200.0, 5.0),
            (0.0, 0.0, 200.0, 125.0),
        ],
    );
}

#[test]
fn auto_margins_centre_the_table_in_the_room_with_a_left_float() {
    check_horizontal(
        "left",
        "<div id=k style=\"display:table;width:50px;height:20px;margin:0 auto\"></div>",
        [
            (0.0, 0.0, 100.0, 100.0),
            (125.0, 0.0, 50.0, 20.0),
            (0.0, 20.0, 200.0, 5.0),
            (0.0, 0.0, 200.0, 25.0),
        ],
    );
}

#[test]
fn clear_moves_the_table_below_the_float_with_a_left_float() {
    check_horizontal(
        "left",
        "<div id=k style=\"display:table;width:50px;height:20px;clear:both\"></div>",
        [
            (0.0, 0.0, 100.0, 100.0),
            (0.0, 100.0, 50.0, 20.0),
            (0.0, 120.0, 200.0, 5.0),
            (0.0, 0.0, 200.0, 125.0),
        ],
    );
}

#[test]
fn a_wide_table_sits_below_the_float_with_a_left_float() {
    check_horizontal(
        "left",
        "<div id=k style=\"display:table;width:150px;height:20px\"></div>",
        [
            (0.0, 0.0, 100.0, 100.0),
            (0.0, 100.0, 150.0, 20.0),
            (0.0, 120.0, 200.0, 5.0),
            (0.0, 0.0, 200.0, 125.0),
        ],
    );
}

#[test]
fn a_narrow_table_sits_beside_the_float_with_a_left_float() {
    check_horizontal(
        "left",
        "<div id=k style=\"display:table;width:50px;height:20px\"></div>",
        [
            (0.0, 0.0, 100.0, 100.0),
            (100.0, 0.0, 50.0, 20.0),
            (0.0, 20.0, 200.0, 5.0),
            (0.0, 0.0, 200.0, 25.0),
        ],
    );
}

#[test]
fn an_auto_width_table_that_fits_beside_the_float_with_a_right_float() {
    check_horizontal(
        "right",
        "<table id=k style=\"border-spacing:0\"><tr><td style=\"padding:0;width:50px;height:10px\"></td></tr></table>",
        [
            (100.0, 0.0, 100.0, 100.0),
            (0.0, 0.0, 50.0, 10.0),
            (0.0, 10.0, 200.0, 5.0),
            (0.0, 0.0, 200.0, 15.0),
        ],
    );
}

#[test]
fn a_table_made_of_table_display_boxes_with_a_right_float() {
    check_horizontal(
        "right",
        "<div id=k style=\"display:table\"><div style=\"display:table-row\"><div style=\"display:table-cell;width:50px;height:10px\"></div></div></div>",
        [
            (100.0, 0.0, 100.0, 100.0),
            (0.0, 0.0, 50.0, 10.0),
            (0.0, 10.0, 200.0, 5.0),
            (0.0, 0.0, 200.0, 15.0),
        ],
    );
}

#[test]
fn a_left_margin_may_reach_under_the_float_with_a_right_float() {
    check_horizontal(
        "right",
        "<div id=k style=\"display:table;width:60px;height:20px;margin-left:20px\"></div>",
        [
            (100.0, 0.0, 100.0, 100.0),
            (20.0, 0.0, 60.0, 20.0),
            (0.0, 20.0, 200.0, 5.0),
            (0.0, 0.0, 200.0, 25.0),
        ],
    );
}

#[test]
fn a_margin_that_does_not_fit_sends_the_table_below_with_a_right_float() {
    check_horizontal(
        "right",
        "<div id=k style=\"display:table;width:60px;height:20px;margin-left:50px\"></div>",
        [
            (100.0, 0.0, 100.0, 100.0),
            (50.0, 100.0, 60.0, 20.0),
            (0.0, 120.0, 200.0, 5.0),
            (0.0, 0.0, 200.0, 125.0),
        ],
    );
}

#[test]
fn a_percentage_width_resolves_against_the_container_with_a_right_float() {
    check_horizontal(
        "right",
        "<div id=k style=\"display:table;width:40%;height:20px\"></div>",
        [
            (100.0, 0.0, 100.0, 100.0),
            (0.0, 0.0, 80.0, 20.0),
            (0.0, 20.0, 200.0, 5.0),
            (0.0, 0.0, 200.0, 25.0),
        ],
    );
}

#[test]
fn a_min_width_wider_than_the_room_sends_the_table_below_with_a_right_float() {
    check_horizontal(
        "right",
        "<div id=k style=\"display:table;min-width:120px;height:20px\"></div>",
        [
            (100.0, 0.0, 100.0, 100.0),
            (0.0, 100.0, 120.0, 20.0),
            (0.0, 120.0, 200.0, 5.0),
            (0.0, 0.0, 200.0, 125.0),
        ],
    );
}

#[test]
fn auto_margins_centre_the_table_in_the_room_with_a_right_float() {
    check_horizontal(
        "right",
        "<div id=k style=\"display:table;width:50px;height:20px;margin:0 auto\"></div>",
        [
            (100.0, 0.0, 100.0, 100.0),
            (25.0, 0.0, 50.0, 20.0),
            (0.0, 20.0, 200.0, 5.0),
            (0.0, 0.0, 200.0, 25.0),
        ],
    );
}

#[test]
fn clear_moves_the_table_below_the_float_with_a_right_float() {
    check_horizontal(
        "right",
        "<div id=k style=\"display:table;width:50px;height:20px;clear:both\"></div>",
        [
            (100.0, 0.0, 100.0, 100.0),
            (0.0, 100.0, 50.0, 20.0),
            (0.0, 120.0, 200.0, 5.0),
            (0.0, 0.0, 200.0, 125.0),
        ],
    );
}

#[test]
fn a_wide_table_sits_below_the_float_with_a_right_float() {
    check_horizontal(
        "right",
        "<div id=k style=\"display:table;width:150px;height:20px\"></div>",
        [
            (100.0, 0.0, 100.0, 100.0),
            (0.0, 100.0, 150.0, 20.0),
            (0.0, 120.0, 200.0, 5.0),
            (0.0, 0.0, 200.0, 125.0),
        ],
    );
}

#[test]
fn a_narrow_table_sits_beside_the_float_with_a_right_float() {
    check_horizontal(
        "right",
        "<div id=k style=\"display:table;width:50px;height:20px\"></div>",
        [
            (100.0, 0.0, 100.0, 100.0),
            (0.0, 0.0, 50.0, 20.0),
            (0.0, 20.0, 200.0, 5.0),
            (0.0, 0.0, 200.0, 25.0),
        ],
    );
}

/// With no float in the way, a table is as wide as its columns make it and
/// the limits on its width allow. Each case is the markup of the table `k`
/// and the border boxes of `k`, of the container `o` and of the block that
/// follows it.
#[test]
fn a_table_with_no_float_around_keeps_the_width_its_columns_give_it() {
    const CASES: &[(&str, Rect, Rect, Rect)] = &[
        (
            "<table id=k style=\"border-spacing:0\"><tr><td style=\"padding:0;width:50px;height:10px\"></td></tr></table>",
            (0.0, 0.0, 50.0, 10.0),
            (0.0, 0.0, 200.0, 15.0),
            (0.0, 10.0, 200.0, 5.0),
        ),
        (
            "<table id=k style=\"border-spacing:0;margin:0 auto\"><tr><td style=\"padding:0;width:50px;height:10px\"></td></tr></table>",
            (75.0, 0.0, 50.0, 10.0),
            (0.0, 0.0, 200.0, 15.0),
            (0.0, 10.0, 200.0, 5.0),
        ),
        (
            "<table id=k style=\"border-spacing:0;margin-left:30px\"><tr><td style=\"padding:0;width:50px;height:10px\"></td></tr></table>",
            (30.0, 0.0, 50.0, 10.0),
            (0.0, 0.0, 200.0, 15.0),
            (0.0, 10.0, 200.0, 5.0),
        ),
        (
            "<table id=k style=\"border-spacing:0;margin-left:auto\"><tr><td style=\"padding:0;width:50px;height:10px\"></td></tr></table>",
            (150.0, 0.0, 50.0, 10.0),
            (0.0, 0.0, 200.0, 15.0),
            (0.0, 10.0, 200.0, 5.0),
        ),
        (
            "<table id=k style=\"border-spacing:0;min-width:120px\"><tr><td style=\"padding:0;width:50px;height:10px\"></td></tr></table>",
            (0.0, 0.0, 120.0, 10.0),
            (0.0, 0.0, 200.0, 15.0),
            (0.0, 10.0, 200.0, 5.0),
        ),
        (
            "<table id=k style=\"border-spacing:0;width:50%\"><tr><td style=\"padding:0;height:10px\"></td></tr></table>",
            (0.0, 0.0, 100.0, 10.0),
            (0.0, 0.0, 200.0, 15.0),
            (0.0, 10.0, 200.0, 5.0),
        ),
        (
            "<div dir=rtl style=\"width:200px\"><table id=k style=\"border-spacing:0\"><tr><td style=\"padding:0;width:50px;height:10px\"></td></tr></table></div>",
            (150.0, 0.0, 50.0, 10.0),
            (0.0, 0.0, 200.0, 15.0),
            (0.0, 10.0, 200.0, 5.0),
        ),
        (
            "<table id=k style=\"border-spacing:0;box-sizing:border-box;width:100px;border:5px solid\"><tr><td style=\"padding:0;height:10px\"></td></tr></table>",
            (0.0, 0.0, 100.0, 20.0),
            (0.0, 0.0, 200.0, 25.0),
            (0.0, 20.0, 200.0, 5.0),
        ),
        (
            "<table id=k style=\"border-spacing:0;table-layout:fixed;width:150px\"><tr><td style=\"padding:0;height:10px\"></td></tr></table>",
            (0.0, 0.0, 150.0, 10.0),
            (0.0, 0.0, 200.0, 15.0),
            (0.0, 10.0, 200.0, 5.0),
        ),
        (
            "<table id=k style=\"border-spacing:0;margin:0 auto;width:50px\"><tr><td style=\"padding:0;height:10px\"></td></tr></table>",
            (75.0, 0.0, 50.0, 10.0),
            (0.0, 0.0, 200.0, 15.0),
            (0.0, 10.0, 200.0, 5.0),
        ),
        (
            "<table id=k style=\"border-spacing:0\"><tr><td style=\"padding:0\"><div style=\"width:300px;height:10px\"></div></td></tr></table>",
            (0.0, 0.0, 300.0, 10.0),
            (0.0, 0.0, 200.0, 15.0),
            (0.0, 10.0, 200.0, 5.0),
        ),
        (
            "<table id=k style=\"border-collapse:collapse;border:4px solid;margin:0 auto\"><tr><td style=\"padding:0;width:50px;height:10px\"></td></tr></table>",
            (71.0, 0.0, 58.0, 18.0),
            (0.0, 0.0, 200.0, 23.0),
            (0.0, 18.0, 200.0, 5.0),
        ),
        (
            "<table id=k style=\"border-spacing:0;margin:0 auto\"><tr><td style=\"padding:0;width:50px;height:10px\"></td><td style=\"padding:0;width:70px\"></td></tr></table>",
            (40.0, 0.0, 120.0, 10.0),
            (0.0, 0.0, 200.0, 15.0),
            (0.0, 10.0, 200.0, 5.0),
        ),
        (
            "<table id=k style=\"border-collapse:collapse;font:20px/1 Ahem;overflow-wrap:anywhere;max-width:0\"><tr><td style=\"padding:0\">XXXX</td></tr></table>",
            (0.0, 0.0, 20.0, 80.0),
            (0.0, 0.0, 200.0, 85.0),
            (0.0, 80.0, 200.0, 5.0),
        ),
        (
            "<div id=k style=\"display:table;margin:0 auto\"><div style=\"width:60px;height:10px\"></div></div>",
            (70.0, 0.0, 60.0, 10.0),
            (0.0, 0.0, 200.0, 15.0),
            (0.0, 10.0, 200.0, 5.0),
        ),
        (
            "<table id=k style=\"border-spacing:0;margin:5px 20px 7px 30px\"><tr><td style=\"padding:0;width:50px;height:10px\"></td></tr></table>",
            (30.0, 5.0, 50.0, 10.0),
            (0.0, 5.0, 200.0, 22.0),
            (0.0, 22.0, 200.0, 5.0),
        ),
    ];
    for &(element, k, o, after) in CASES {
        let body = format!(
            "<div id=o style=\"width:200px\">{element}\
             <div id=after style=\"height:5px\"></div></div>"
        );
        compare(&body, &[("k", k), ("o", o), ("after", after)]);
    }
}

/// A column removed by `visibility: collapse` still takes part in sizing the
/// rows, which are set in the room the table was given. The table is placed
/// by the height it has there, so it fits beside the first float at the top
/// instead of dropping below it.
#[test]
fn a_collapsed_column_does_not_make_the_table_taller_than_its_room_does() {
    compare(
        "<div id=o style=\"width:300px\">\
         <div id=a style=\"float:left;width:250px;height:20px\"></div>\
         <div id=b style=\"float:right;width:100px;height:100px\"></div>\
         <table id=k style=\"border-spacing:0\"><col><col style=\"visibility:collapse\">\
         <tr><td style=\"padding:0\">X</td><td style=\"padding:0\">X X X</td></tr></table>\
         <div id=after style=\"height:5px\"></div></div>",
        &[
            ("a", (0.0, 0.0, 250.0, 20.0)),
            ("b", (200.0, 20.0, 100.0, 100.0)),
            ("k", (250.0, 0.0, 10.0, 20.0)),
            ("o", (0.0, 0.0, 300.0, 25.0)),
            ("after", (0.0, 20.0, 300.0, 5.0)),
        ],
    );
}

#[test]
fn the_tables_writing_mode_sets_whether_cell_width_can_shrink() {
    for table_mode in ["horizontal-tb", "vertical-rl", "vertical-lr"] {
        for cell_mode in ["horizontal-tb", "vertical-rl", "vertical-lr"] {
            for side in ["none", "left", "right"] {
                let (room, width) = if side == "none" { (40, 60) } else { (200, 150) };
                let float = if side == "none" {
                    String::new()
                } else {
                    format!("<div style='float:{side};width:100px;height:100px'></div>")
                };
                let body = format!(
                    "<div style='width:{room}px'>{float}\
                     <div id=t style='display:table;writing-mode:{table_mode}'>\
                     <div style='display:table-row'>\
                     <div id=c style='display:table-cell;writing-mode:{cell_mode};\
                     width:{width}px;height:80px'></div></div></div></div>"
                );
                let expected = if table_mode == "horizontal-tb" {
                    if side == "none" {
                        (0.0, 0.0, 40.0, 80.0)
                    } else {
                        (if side == "left" { 100.0 } else { 0.0 }, 0.0, 100.0, 80.0)
                    }
                } else if side == "none" {
                    (0.0, 0.0, 60.0, 80.0)
                } else {
                    (0.0, 100.0, 150.0, 80.0)
                };
                compare(&body, &[("t", expected), ("c", expected)]);
            }
        }
    }
}
