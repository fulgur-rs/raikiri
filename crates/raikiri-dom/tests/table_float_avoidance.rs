//! A block-level table keeps its border box clear of the margin boxes of the
//! floats in its formatting context (CSS 2.1 section 9.5): it sits beside a
//! float when it fits in the room the float leaves, and below it when it
//! does not. The root of a vertical writing mode follows the same rule along
//! its inline axis, and a plain block child of such a root shares the root's
//! context, so only its lines make room for a float.
//!
//! The expected border boxes are the ones Chromium computes for the same
//! markup in an 800px by 600px viewport, with no margin on the body. The
//! tables carry no padding, border or spacing of their own.

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
