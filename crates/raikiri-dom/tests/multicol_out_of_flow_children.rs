//! An absolutely positioned (or fixed) box whose containing block is a
//! multicol container, or a box outside it, is laid out against that
//! containing block, outside the fragmentation context of the columns (CSS
//! Multi-column 1, 8.1; CSS Fragmentation 3, 4.1). It takes no room in the
//! columns, so the boxes in flow still go to the columns around it, and its
//! insets resolve against the container's final size. A box with a static
//! position stands where the box in flow before it went.
//!
//! The expected rectangles are the ones Chromium gives the same markup in an
//! 800px by 600px viewport with a body margin of 8px, `div` boxes sized by
//! their border box and the Ahem font at 10px. Each is the left, top, width
//! and height of the border box of an element with an `id`; the one of a box
//! in several columns is the union of its pieces.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The left, top, width and height of a box.
type Rect = (f32, f32, f32, f32);

const AHEM: &[u8] = include_bytes!("data/text-autospace/Ahem.ttf");

/// The union of the pieces of each element with an `id` on the first page.
fn rects_of(body: &str) -> Vec<(String, Rect)> {
    let html = format!(
        "<!DOCTYPE html><html><head><style>@page{{size:800px 600px;margin:0}}\
         body{{margin:8px;font:10px/10px Ahem}}div{{box-sizing:border-box}}</style>\
         </head><body>{body}</body></html>"
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
    let mut rects: Vec<(String, Rect)> = Vec::new();
    for fragment in page.fragments() {
        let Some(id) = dom.attr(fragment.node(), "id") else {
            continue;
        };
        let rect = fragment.rect();
        let (right, bottom) = (rect.x + rect.width, rect.y + rect.height);
        match rects.iter_mut().find(|(seen, _)| seen == id) {
            Some((_, own)) => {
                let (x0, y0) = (own.0.min(rect.x), own.1.min(rect.y));
                let (x1, y1) = ((own.0 + own.2).max(right), (own.1 + own.3).max(bottom));
                *own = (x0, y0, x1 - x0, y1 - y0);
            }
            None => rects.push((id.to_owned(), (rect.x, rect.y, rect.width, rect.height))),
        }
    }
    rects
}

/// Lay out `body` and compare the box of each `id` with `expected`.
fn check(body: &str, expected: &[(&str, Rect)]) {
    let rects = rects_of(body);
    for &(id, want) in expected {
        let got = rects
            .iter()
            .find(|(seen, _)| seen == id)
            .map(|(_, rect)| *rect)
            .unwrap_or_else(|| panic!("no box for #{id} in {rects:?}"));
        let close = [
            (got.0, want.0),
            (got.1, want.1),
            (got.2, want.2),
            (got.3, want.3),
        ]
        .iter()
        .all(|(a, b)| (a - b).abs() <= 0.01);
        assert!(
            close,
            "#{id}: got {got:?}, expected {want:?}; all {rects:?}"
        );
    }
}

#[test]
fn definite_height_relative_container() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;height:100px;position:relative\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div><div id=z style=\"position:absolute;top:0;left:0;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 100.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
            ("z", (8.0, 8.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn column_fill_auto() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;height:100px;position:relative;column-fill:auto\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div><div id=c style=\"height:40px\"></div><div id=d style=\"height:40px\"></div><div id=z style=\"position:absolute;top:0;left:0;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 100.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (8.0, 48.0, 100.0, 40.0)),
            ("c", (8.0, 8.0, 200.0, 100.0)),
            ("d", (108.0, 28.0, 100.0, 40.0)),
            ("z", (8.0, 8.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn auto_height_relative_container() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div><div id=z style=\"position:absolute;top:0;left:0;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 40.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
            ("z", (8.0, 8.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn containing_block_outside_the_container() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div><div id=z style=\"position:absolute;top:0;left:0;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 40.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
            ("z", (0.0, 0.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn first_child_out_of_flow() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;height:100px;position:relative\"><div id=z style=\"position:absolute;top:0;left:0;height:10px;width:10px\"></div><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 100.0)),
            ("z", (8.0, 8.0, 10.0, 10.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
        ],
    );
}

#[test]
fn static_position_between_blocks() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative\"><div id=a style=\"height:40px\"></div><div id=z style=\"position:absolute;height:10px;width:10px\"></div><div id=b style=\"height:40px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 40.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("z", (8.0, 48.0, 10.0, 10.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
        ],
    );
}

#[test]
fn out_of_flow_box_inside_a_column_child() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"><div id=z style=\"position:absolute;top:5px;left:5px;height:10px;width:10px\"></div></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 40.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
            ("z", (13.0, 13.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn paragraphs_in_definite_height() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;height:60px;position:relative\"><div id=p style=\"width:100%\">x x x x x x x x x x x x </div><div id=q>x x x x x x x x x x x x </div><div id=z style=\"position:absolute;top:0;left:0;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 60.0)),
            ("p", (8.0, 8.0, 100.0, 30.0)),
            ("q", (108.0, 8.0, 100.0, 30.0)),
            ("z", (8.0, 8.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn balanced_paragraphs() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative\"><div id=p>x x x x x x x x x x x x </div><div id=q>x x x x x x x x x x x x </div><div id=z style=\"position:absolute;top:0;left:0;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 30.0)),
            ("p", (8.0, 8.0, 100.0, 30.0)),
            ("q", (108.0, 8.0, 100.0, 30.0)),
            ("z", (8.0, 8.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn percentage_insets_and_sizes() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;height:100px;position:relative\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div><div id=z style=\"position:absolute;bottom:0;right:0;height:10%;width:25%\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 100.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
            ("z", (158.0, 98.0, 50.0, 10.0)),
        ],
    );
}

#[test]
fn fixed_box() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;height:100px\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div><div id=z style=\"position:fixed;top:300px;left:300px;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 100.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
            ("z", (300.0, 300.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn auto_fill_box_across_columns() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;height:60px;position:relative;column-fill:auto\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div><div id=c style=\"height:40px\"></div><div id=z style=\"position:absolute;top:0;left:0;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 60.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (8.0, 8.0, 200.0, 60.0)),
            ("c", (108.0, 28.0, 100.0, 40.0)),
            ("z", (8.0, 8.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn wrapper_beside_out_of_flow_box() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative\"><div id=w><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div></div><div id=z style=\"position:absolute;top:0;left:0;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 40.0)),
            ("w", (8.0, 8.0, 200.0, 40.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
            ("z", (8.0, 8.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn following_sibling() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div><div id=z style=\"position:absolute;top:0;left:0;height:10px;width:10px\"></div></div><div id=n style=\"height:10px\"></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 40.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
            ("z", (8.0, 8.0, 10.0, 10.0)),
            ("n", (8.0, 48.0, 784.0, 10.0)),
        ],
    );
}

#[test]
fn out_of_flow_box_taller_than_the_columns() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div><div id=z style=\"position:absolute;top:0;left:0;height:300px;width:10px\"></div></div><div id=n style=\"height:10px\"></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 40.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
            ("z", (8.0, 8.0, 10.0, 300.0)),
            ("n", (8.0, 48.0, 784.0, 10.0)),
        ],
    );
}

#[test]
fn bottom_inset_against_balanced_height() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div><div id=z style=\"position:absolute;bottom:0;right:0;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 40.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
            ("z", (198.0, 38.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn percentage_height_against_balanced_height() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div><div id=z style=\"position:absolute;top:0;left:0;height:50%;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 40.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
            ("z", (8.0, 8.0, 10.0, 20.0)),
        ],
    );
}

#[test]
fn static_position_in_second_column() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div><div id=z style=\"position:absolute;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 40.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
            ("z", (108.0, 48.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn static_position_between_columns() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div><div id=z style=\"position:absolute;height:10px;width:10px\"></div><div id=c style=\"height:40px\"></div><div id=d style=\"height:40px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 80.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (8.0, 48.0, 100.0, 40.0)),
            ("z", (8.0, 88.0, 10.0, 10.0)),
            ("c", (108.0, 8.0, 100.0, 40.0)),
            ("d", (108.0, 48.0, 100.0, 40.0)),
        ],
    );
}

#[test]
fn display_contents_parent() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative\"><div style=\"display:contents\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div><div id=z style=\"position:absolute;top:0;left:0;height:10px;width:10px\"></div></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 40.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
            ("z", (8.0, 8.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn float_before_blocks() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative\"><div id=f style=\"float:left;width:50px;height:30px\"></div><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div><div id=z style=\"position:absolute;top:0;left:0;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 40.0)),
            ("f", (8.0, 8.0, 50.0, 30.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
            ("z", (8.0, 8.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn auto_fill_auto_height() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;column-fill:auto;position:relative\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div><div id=z style=\"position:absolute;top:0;left:0;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 80.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (8.0, 48.0, 100.0, 40.0)),
            ("z", (8.0, 8.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn contained_and_escaping_boxes() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative\"><div id=a style=\"height:40px;position:relative\"><div id=y style=\"position:absolute;top:5px;left:5px;height:10px;width:10px\"></div></div><div id=b style=\"height:40px\"></div><div id=z style=\"position:absolute;top:0;left:0;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 40.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("y", (13.0, 13.0, 10.0, 10.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
            ("z", (8.0, 8.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn fixed_box_in_transformed_container() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;height:100px;transform:translateX(0)\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div><div id=z style=\"position:fixed;top:0;left:0;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 100.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
            ("z", (8.0, 8.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn tall_child_in_definite_height() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;height:100px;position:relative\"><div id=a style=\"height:150px\"></div><div id=z style=\"position:absolute;top:0;left:0;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 100.0)),
            ("a", (8.0, 8.0, 200.0, 75.0)),
            ("z", (8.0, 8.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn padded_container() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative;padding:10px;width:220px\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div><div id=z style=\"position:absolute;bottom:0;right:0;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 220.0, 60.0)),
            ("a", (18.0, 18.0, 100.0, 40.0)),
            ("b", (118.0, 18.0, 100.0, 40.0)),
            ("z", (218.0, 58.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn out_of_flow_box_with_children() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div><div id=z style=\"position:absolute;top:0;left:0;width:30px\"><div id=y style=\"height:30px\"></div></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 40.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
            ("z", (8.0, 8.0, 30.0, 30.0)),
            ("y", (8.0, 8.0, 30.0, 30.0)),
        ],
    );
}

#[test]
fn right_to_left_static_position() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative;direction:rtl\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div><div id=z style=\"position:absolute;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 40.0)),
            ("a", (108.0, 8.0, 100.0, 40.0)),
            ("b", (8.0, 8.0, 100.0, 40.0)),
            ("z", (98.0, 48.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn static_position_with_white_space_between_boxes() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative\">\n<div id=a style=\"height:40px\"></div>\n<div id=b style=\"height:40px\"></div>\n<div id=z style=\"position:absolute;height:10px;width:10px\"></div>\n</div>",
        &[
            ("m", (8.0, 8.0, 200.0, 40.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
            ("z", (108.0, 48.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn static_position_after_a_box_broken_across_columns() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative\"><div id=a style=\"height:120px\"></div><div id=z style=\"position:absolute;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 60.0)),
            ("a", (8.0, 8.0, 200.0, 60.0)),
            ("z", (108.0, 68.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn static_position_after_a_broken_box_with_a_gap() {
    check(
        "<div id=m style=\"columns:2;column-gap:20px;width:220px;position:relative\"><div id=a style=\"height:120px\"></div><div id=z style=\"position:absolute;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 220.0, 60.0)),
            ("a", (8.0, 8.0, 220.0, 60.0)),
            ("z", (128.0, 68.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn right_to_left_static_position_after_a_narrow_box() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative;direction:rtl\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px;width:50px\"></div><div id=z style=\"position:absolute;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 40.0)),
            ("a", (108.0, 8.0, 100.0, 40.0)),
            ("b", (58.0, 8.0, 50.0, 40.0)),
            ("z", (98.0, 48.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn static_position_after_a_broken_paragraph() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative\"><div id=p>x x x x x x x x x x x x x x x x x x x x x x x x</div><div id=z style=\"position:absolute;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 30.0)),
            ("p", (8.0, 8.0, 200.0, 30.0)),
            ("z", (108.0, 28.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn centered_inline_static_position_first() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative;text-align:center\"><div id=z style=\"position:absolute;display:inline;height:10px;width:10px\"></div><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 40.0)),
            ("z", (58.0, 8.0, 10.0, 10.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
        ],
    );
}

#[test]
fn centered_inline_static_position_after_blocks() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative;text-align:center\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div><div id=z style=\"position:absolute;display:inline;height:10px;width:10px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 40.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
            ("z", (158.0, 48.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn right_to_left_inline_static_position() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative;direction:rtl\"><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div><span id=z style=\"position:absolute;height:10px;width:10px\"></span></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 40.0)),
            ("a", (108.0, 8.0, 100.0, 40.0)),
            ("b", (8.0, 8.0, 100.0, 40.0)),
            ("z", (98.0, 48.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn block_static_position_first() {
    check(
        "<div id=m style=\"columns:2;column-gap:0;width:200px;position:relative\"><div id=z style=\"position:absolute;height:10px;width:10px\"></div><div id=a style=\"height:40px\"></div><div id=b style=\"height:40px\"></div></div>",
        &[
            ("m", (8.0, 8.0, 200.0, 40.0)),
            ("z", (8.0, 8.0, 10.0, 10.0)),
            ("a", (8.0, 8.0, 100.0, 40.0)),
            ("b", (108.0, 8.0, 100.0, 40.0)),
        ],
    );
}
