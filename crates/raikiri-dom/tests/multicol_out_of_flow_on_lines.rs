//! An absolutely or fixed positioned box whose static position is on a line of
//! a paragraph in a multicol container goes with that line to its column: the
//! paragraph is broken at the column width and split between the columns as
//! without the box, and the box takes its static position where the column
//! puts its line (CSS Multi-column 1, 8.1; CSS 2, 10.3.7 and 10.6.4). This
//! holds for a box whose containing block is the container, one outside it,
//! and one inside a positioned inline element of the line, for a paragraph
//! that is a child of the container, inside a plain block of it, or the
//! container itself.
//!
//! The expected rectangles are the ones Chromium gives the same markup in an
//! 800px by 600px viewport with a body margin of 8px, `div` boxes sized by
//! their border box and the Ahem font at 10px with a 10px line height. Each is
//! the left, top, width and height of the border box of an element with an
//! `id`; the one of a box in several columns is the union of its pieces.

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

/// Lay out `body` and compare the box of each `id` with `expected`, to within
/// the 1/64px that Chromium rounds its column widths to.
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
        .all(|(a, b)| (a - b).abs() <= 0.02);
        assert!(close, "#{id} is {got:?}, not {want:?}, in {body}");
    }
}

#[test]
fn a_box_after_the_text_goes_to_the_column_of_its_line() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;position:relative"><div id=p>x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px"></span> x x x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 30.0)),
            ("p", (8.0, 8.0, 200.0, 30.0)),
            ("s", (198.0, 8.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;height:60px;position:relative"><div id=p>x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px"></span> x x x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 60.0)),
            ("p", (8.0, 8.0, 200.0, 30.0)),
            ("s", (198.0, 8.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;height:60px;column-fill:auto;position:relative"><div id=p>x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px"></span> x x x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 60.0)),
            ("p", (8.0, 8.0, 100.0, 50.0)),
            ("s", (98.0, 38.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px"><div id=p>x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px"></span> x x x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 30.0)),
            ("p", (8.0, 8.0, 200.0, 30.0)),
            ("s", (198.0, 8.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:3;column-gap:10px;width:220px;position:relative"><div id=p>x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px"></span> x x x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 220.0, 30.0)),
            ("p", (8.0, 8.0, 219.99, 30.0)),
            ("s", (191.33, 8.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn a_box_in_the_middle_of_a_line_keeps_its_place_on_it() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;position:relative"><div id=p>x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px"></span>x x x x x x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 30.0)),
            ("p", (8.0, 8.0, 200.0, 30.0)),
            ("s", (138.0, 8.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;height:60px;column-fill:auto;position:relative"><div id=p>x x x x x x x x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px"></span>x x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 60.0)),
            ("p", (8.0, 8.0, 100.0, 60.0)),
            ("s", (38.0, 58.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn insets_on_one_axis_keep_the_static_position_on_the_other() {
    // A box that the insets put outside every column is still placed once.
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;position:relative"><div id=p>x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px;left:-20px"></span> x x x x</div></div>"#,
        &[
            ("p", (8.0, 8.0, 200.0, 30.0)),
            ("s", (-12.0, 8.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;position:relative"><div id=p>x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px;top:0"></span> x x x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 30.0)),
            ("p", (8.0, 8.0, 200.0, 30.0)),
            ("s", (198.0, 8.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;position:relative"><div id=p>x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px;left:5px"></span> x x x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 30.0)),
            ("p", (8.0, 8.0, 200.0, 30.0)),
            ("s", (13.0, 8.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;height:60px;position:relative"><div id=p>x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px;bottom:0"></span> x x x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 60.0)),
            ("p", (8.0, 8.0, 200.0, 30.0)),
            ("s", (198.0, 58.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;height:60px;column-fill:auto;position:relative"><div id=p>x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px;top:3px;left:5px"></span> x x x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 60.0)),
            ("p", (8.0, 8.0, 100.0, 50.0)),
            ("s", (13.0, 11.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn a_box_at_the_start_or_end_of_the_paragraph_goes_with_its_line() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;position:relative"><div id=p><span id=s style="position:absolute;width:10px;height:10px"></span>x x x x x x x x x x x x x x x x x x x x x x x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 30.0)),
            ("p", (8.0, 8.0, 200.0, 30.0)),
            ("s", (8.0, 8.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;height:60px;column-fill:auto;position:relative"><div id=p><span id=s style="position:absolute;width:10px;height:10px"></span>x x x x x x x x x x x x x x x x x x x x x x x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 60.0)),
            ("p", (8.0, 8.0, 100.0, 50.0)),
            ("s", (8.0, 8.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;position:relative"><div id=p>x x x x x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px"></span></div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 30.0)),
            ("p", (8.0, 8.0, 200.0, 30.0)),
            ("s", (178.0, 18.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;height:60px;column-fill:auto;position:relative"><div id=p>x x x x x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px"></span></div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 60.0)),
            ("p", (8.0, 8.0, 100.0, 50.0)),
            ("s", (78.0, 48.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn a_box_in_a_later_paragraph_follows_that_paragraph_into_its_column() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;height:60px;position:relative"><div id=p>x x x x x x</div><div id=q>x x x x <span id=s style="position:absolute;width:10px;height:10px"></span>x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 60.0)),
            ("p", (8.0, 8.0, 100.0, 20.0)),
            ("q", (108.0, 8.0, 100.0, 20.0)),
            ("s", (188.0, 8.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;height:60px;column-fill:auto;position:relative"><div id=p>x x x x x x x x x x x x x x x x x x x x x x</div><div id=q>x x x x <span id=s style="position:absolute;width:10px;height:10px"></span>x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 60.0)),
            ("p", (8.0, 8.0, 100.0, 50.0)),
            ("q", (108.0, 8.0, 100.0, 20.0)),
            ("s", (188.0, 8.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;position:relative"><div id=p>x x x x x x</div><div id=q>x x x x <span id=s style="position:absolute;width:10px;height:10px;display:inline-block"></span>x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 20.0)),
            ("p", (8.0, 8.0, 100.0, 20.0)),
            ("q", (108.0, 8.0, 100.0, 20.0)),
            ("s", (188.0, 8.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn a_box_that_was_block_level_or_inline_block_sits_by_its_line() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;position:relative"><div id=p>x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;display:block;width:10px;height:10px"></span> x x x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 30.0)),
            ("p", (8.0, 8.0, 200.0, 30.0)),
            ("s", (108.0, 18.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;height:60px;column-fill:auto;position:relative"><div id=p>x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px;display:inline-block"></span> x x x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 60.0)),
            ("p", (8.0, 8.0, 100.0, 50.0)),
            ("s", (98.0, 38.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn a_box_inside_a_positioned_inline_element_goes_with_the_element() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;height:60px;position:relative"><div id=p>x x x x x x x x x x x x x x x x x x x x <span id=w style="position:relative">x <span id=s style="position:absolute;width:10px;height:10px"></span>x</span> x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 60.0)),
            ("p", (8.0, 8.0, 200.0, 30.0)),
            ("w", (108.0, 18.0, 30.0, 10.0)),
            ("s", (128.0, 18.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;position:relative"><div id=p>x x x x x x x x x x x x x x x x x x x x <span id=w style="position:relative">x <span id=s style="position:absolute;width:10px;height:10px"></span>x</span> x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 30.0)),
            ("p", (8.0, 8.0, 200.0, 30.0)),
            ("w", (108.0, 18.0, 30.0, 10.0)),
            ("s", (128.0, 18.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;position:relative"><div id=p>x x x x x x x x x x x x x x x x x x x x <span id=w>x <span id=s style="position:absolute;width:10px;height:10px"></span>x</span> x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 30.0)),
            ("p", (8.0, 8.0, 200.0, 30.0)),
            ("w", (108.0, 18.0, 30.0, 10.0)),
            ("s", (128.0, 18.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn an_aligned_line_keeps_its_aligned_point_in_the_column() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;position:relative"><div id=p style="text-align:center">x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px"></span> x x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 30.0)),
            ("p", (8.0, 8.0, 200.0, 30.0)),
            ("s", (203.0, 8.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;height:60px;column-fill:auto;position:relative"><div id=p style="text-align:center">x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px"></span> x x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 60.0)),
            ("p", (8.0, 8.0, 100.0, 50.0)),
            ("s", (103.0, 38.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn the_lines_of_the_container_itself_carry_their_boxes() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;position:relative">x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px"></span> x x x x</div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 30.0)),
            ("s", (198.0, 8.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;height:60px;column-fill:auto;position:relative">x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px"></span> x x x x</div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 60.0)),
            ("s", (98.0, 38.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:3;column-gap:10px;width:220px;position:relative">x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px"></span> x x x x</div>"#,
        &[
            ("m", (8.0, 8.0, 220.0, 30.0)),
            ("s", (191.33, 8.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;position:relative">x x x <span id=i style="display:inline-block;width:20px;height:10px"></span> x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px"></span> x x x x</div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 30.0)),
            ("i", (68.0, 8.0, 20.0, 10.0)),
            ("s", (138.0, 18.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn a_paragraph_inside_a_plain_wrapper_carries_its_boxes() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;height:60px;column-fill:auto;position:relative"><div id=wr><div id=p>x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px"></span> x x x x</div></div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 60.0)),
            ("wr", (8.0, 8.0, 100.0, 50.0)),
            ("p", (8.0, 8.0, 100.0, 50.0)),
            ("s", (98.0, 38.0, 10.0, 10.0)),
        ],
    );
}

#[test]
fn fixed_boxes_text_inside_and_atomic_neighbours_go_with_the_line() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;position:relative"><div id=p>x x x x x x x x x x x x x x x x x x x x<span id=s style="position:fixed;width:10px;height:10px"></span> x x x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 30.0)),
            ("p", (8.0, 8.0, 200.0, 30.0)),
            ("s", (198.0, 8.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;height:60px;column-fill:auto;position:relative"><div id=p>x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:30px">ab cd</span> x x x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 60.0)),
            ("p", (8.0, 8.0, 100.0, 50.0)),
            ("s", (98.0, 38.0, 30.0, 20.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;position:relative"><div id=p>x x x <span id=i style="display:inline-block;width:20px;height:10px"></span> x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px"></span> x x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 30.0)),
            ("p", (8.0, 8.0, 200.0, 30.0)),
            ("i", (68.0, 8.0, 20.0, 10.0)),
            ("s", (178.0, 18.0, 10.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;height:60px;position:relative"><div id=p>x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px"></span> x x x x</div><div id=o style="position:absolute;top:0;right:0;width:5px;height:5px"></div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 60.0)),
            ("p", (8.0, 8.0, 200.0, 30.0)),
            ("s", (198.0, 8.0, 10.0, 10.0)),
            ("o", (203.0, 8.0, 5.0, 5.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:200px;position:relative"><div id=p>x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px"></span> x x</div><div id=sp style="column-span:all;height:10px"></div><div id=q>x x x x x x</div></div>"#,
        &[
            ("m", (8.0, 8.0, 200.0, 50.0)),
            ("p", (8.0, 8.0, 100.0, 20.0)),
            ("s", (58.0, 18.0, 10.0, 10.0)),
            ("sp", (8.0, 28.0, 200.0, 10.0)),
            ("q", (8.0, 38.0, 100.0, 20.0)),
        ],
    );
}
