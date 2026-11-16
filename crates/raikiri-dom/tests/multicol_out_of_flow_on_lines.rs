//! An absolutely or fixed positioned box whose static position is on a line of
//! a paragraph in a multicol container goes with that line to its column: the
//! paragraph is broken at the column width and split between the columns as
//! without the box, and the box takes its static position where the column
//! puts its line (CSS Multi-column 1, 8.1; CSS 2, 10.3.7 and 10.6.4). This
//! holds for a box whose containing block is the container, one outside it,
//! and one inside a positioned inline element of the line, for a paragraph
//! that is a child of the container, inside a plain block of it, or the
//! container itself. A box whose containing block lies inside the columns
//! is broken between them like the content in flow: the part past the end of
//! its column goes on at the top of the next one, overflow columns included
//! (CSS Fragmentation 3, 4.1).
//!
//! The expected rectangles are the ones Chromium gives the same markup in an
//! 800px by 600px viewport with a body margin of 8px, `div` boxes sized by
//! their border box and the Ahem font at 10px with a 10px line height. Each is
//! the left, top, width and height of the border box of an element with an
//! `id`; the one of a box in several columns is the union of its pieces.

use raikiri_dom::PaintEvent;
use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The left, top, width and height of a box.
type Rect = (f32, f32, f32, f32);

const AHEM: &[u8] = include_bytes!("data/text-autospace/Ahem.ttf");

/// Lay `body` out on 800px by 600px pages.
fn first_page(body: &str) -> raikiri_html::DocumentLayout {
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
    result
}

/// Add `rect` to the union kept for `id` in `rects`.
fn unite(rects: &mut Vec<(String, Rect)>, id: &str, rect: Rect) {
    let (right, bottom) = (rect.0 + rect.2, rect.1 + rect.3);
    match rects.iter_mut().find(|(seen, _)| seen == id) {
        Some((_, own)) => {
            let (x0, y0) = (own.0.min(rect.0), own.1.min(rect.1));
            let (x1, y1) = ((own.0 + own.2).max(right), (own.1 + own.3).max(bottom));
            *own = (x0, y0, x1 - x0, y1 - y0);
        }
        None => rects.push((id.to_owned(), rect)),
    }
}

/// The union of the pieces of each element with an `id` on the first page.
fn rects_of(body: &str) -> Vec<(String, Rect)> {
    let result = first_page(body);
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    let mut rects: Vec<(String, Rect)> = Vec::new();
    for fragment in page.fragments() {
        let Some(id) = dom.attr(fragment.node(), "id") else {
            continue;
        };
        let rect = fragment.rect();
        unite(&mut rects, id, (rect.x, rect.y, rect.width, rect.height));
    }
    rects
}

/// The union of what the painter shows of the box of each element with an
/// `id` on the first page: each piece is cut by the clips around it, as the
/// piece of a box broken between columns paints the whole box clipped to its
/// column.
fn painted_rects_of(body: &str) -> Vec<(String, Rect)> {
    let result = first_page(body);
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    let (document, cascade, _, _) = page.paint_inputs();
    let mut clips: Vec<(f32, f32, f32, f32)> = Vec::new();
    let mut rects: Vec<(String, Rect)> = Vec::new();
    for event in document.page_paint_order(cascade, page.index(), page.name()) {
        match event {
            PaintEvent::PushClip(clip, _) => {
                let r = clip.rect;
                let (x0, x1) = if clip.clip_x {
                    (r.x, r.x + r.width)
                } else {
                    (f32::NEG_INFINITY, f32::INFINITY)
                };
                let (y0, y1) = if clip.clip_y {
                    (r.y, r.y + r.height)
                } else {
                    (f32::NEG_INFINITY, f32::INFINITY)
                };
                clips.push((x0, y0, x1, y1));
            }
            PaintEvent::PopClip => {
                clips.pop();
            }
            PaintEvent::Box(fragment) => {
                let Some(id) = dom.attr(fragment.node(), "id") else {
                    continue;
                };
                let r = fragment.rect();
                let (mut x0, mut y0, mut x1, mut y1) = (r.x, r.y, r.x + r.width, r.y + r.height);
                for clip in &clips {
                    (x0, y0) = (x0.max(clip.0), y0.max(clip.1));
                    (x1, y1) = (x1.min(clip.2), y1.min(clip.3));
                }
                if x1 > x0 && y1 > y0 {
                    unite(&mut rects, id, (x0, y0, x1 - x0, y1 - y0));
                }
            }
            _ => {}
        }
    }
    rects
}

/// Lay out `body` and compare the box of each `id` with `expected`, to within
/// the 1/64px that Chromium rounds its column widths to.
fn check(body: &str, expected: &[(&str, Rect)]) {
    compare(body, &rects_of(body), expected);
}

/// Lay out `body` and compare what the painter shows of the box of each `id`
/// with `expected`.
fn check_painted(body: &str, expected: &[(&str, Rect)]) {
    compare(body, &painted_rects_of(body), expected);
}

/// Compare the box of each `id` in `rects` with `expected`, to within the
/// 1/64px that Chromium rounds its column widths to.
fn compare(body: &str, rects: &[(String, Rect)], expected: &[(&str, Rect)]) {
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

#[test]
fn a_positioned_paragraph_places_its_boxes_where_the_columns_put_their_lines() {
    for container in ["", "height:60px;"] {
        check(
            &format!(
                r#"<div id=m style="columns:2;column-gap:0;width:200px;{container}"><div id=p style="position:relative">x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px"></span> x x x x</div></div>"#
            ),
            &[
                ("p", (8.0, 8.0, 200.0, 30.0)),
                ("s", (198.0, 8.0, 10.0, 10.0)),
            ],
        );
        check(
            &format!(
                r#"<div id=m style="columns:2;column-gap:0;width:200px;{container}"><div id=p style="position:relative">x x x x x x x x x x x x x x x x x x x x<span id=s style="position:absolute;width:10px;height:10px;left:5px;top:2px"></span> x x x x</div></div>"#
            ),
            &[
                ("p", (8.0, 8.0, 200.0, 30.0)),
                ("s", (13.0, 10.0, 10.0, 10.0)),
            ],
        );
    }
}

/// The content of a box positioned against an inline element is broken
/// between the columns with the line it sits on: what reaches past the end of
/// the column goes on at the top of the next one, in the overflow columns
/// past the last, and a box positioned against the container stays whole.
#[test]
fn the_content_of_a_box_inside_a_positioned_inline_element_breaks_with_the_columns() {
    let paragraph = |content: &str| {
        format!(
            r#"<div id=p>x x x x <span id=r style="position:relative">y<span id=a style="position:absolute;width:30px;left:0;top:0"><div id=d>{content}</div></span></span> x x x x</div>"#
        )
    };
    let six = "xxx xxx xxx xxx xxx xxx";
    check_painted(
        &format!(
            r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto">{}</div>"#,
            paragraph(six)
        ),
        &[
            ("r", (8.0, 28.0, 10.0, 10.0)),
            ("a", (8.0, 8.0, 60.0, 40.0)),
            ("d", (8.0, 8.0, 60.0, 40.0)),
        ],
    );
    // Ten lines fill the second column and go on in an overflow column.
    check_painted(
        &format!(
            r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto">{}</div>"#,
            paragraph("xxx xxx xxx xxx xxx xxx xxx xxx xxx xxx")
        ),
        &[("a", (8.0, 8.0, 90.0, 40.0)), ("d", (8.0, 8.0, 90.0, 40.0))],
    );
    // In a plain block of the container, with gaps, and from the right.
    check_painted(
        &format!(
            r#"<div id=m style="columns:2;column-gap:10px;width:70px;height:40px;column-fill:auto"><div id=w>{}</div></div>"#,
            paragraph(six)
        ),
        &[("a", (8.0, 8.0, 70.0, 40.0)), ("d", (8.0, 8.0, 70.0, 40.0))],
    );
    check_painted(
        &format!(
            r#"<div id=m style="direction:rtl;columns:2;column-gap:10px;width:70px;height:40px;column-fill:auto"><div id=p>x x x x <span id=r style="position:relative">y<span id=a style="position:absolute;width:30px;right:0;top:0"><div id=d>{six}</div></span></span> x x x x</div></div>"#
        ),
        &[
            ("a", (-12.0, 8.0, 70.0, 40.0)),
            ("d", (-12.0, 8.0, 70.0, 40.0)),
        ],
    );
    // A box that ends in its column of balanced columns stays in it.
    check_painted(
        &format!(
            r#"<div id=m style="columns:2;column-gap:0;width:60px">{}</div>"#,
            paragraph("xxx")
        ),
        &[
            ("m", (8.0, 8.0, 60.0, 30.0)),
            ("a", (8.0, 28.0, 30.0, 10.0)),
            ("d", (8.0, 28.0, 30.0, 10.0)),
        ],
    );
    // A box against the container is laid out outside the columns.
    check_painted(
        r#"<div id=m style="position:relative;columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div id=p>x x x x <span id=r>y<span id=a style="position:absolute;width:30px;left:0;top:20px"><div id=d>xxx xxx xxx xxx xxx xxx</div></span></span> x x x x</div></div>"#,
        &[
            ("a", (8.0, 28.0, 30.0, 60.0)),
            ("d", (8.0, 28.0, 30.0, 60.0)),
        ],
    );
    // Columns that fill in turn under an auto height grow with their content.
    check_painted(
        &format!(
            r#"<div id=m style="columns:2;column-gap:0;width:60px;column-fill:auto">{}</div>"#,
            paragraph("xxx")
        ),
        &[
            ("m", (8.0, 8.0, 60.0, 50.0)),
            ("a", (8.0, 28.0, 30.0, 10.0)),
            ("d", (8.0, 28.0, 30.0, 10.0)),
        ],
    );
    // Below a spanner, the columns that fill in turn start under it.
    check_painted(
        &format!(
            r#"<div id=m style="columns:2;column-gap:0;width:60px;height:60px;column-fill:auto"><div id=s style="column-span:all;height:10px"></div>{}</div>"#,
            paragraph(six)
        ),
        &[
            ("p", (8.0, 18.0, 30.0, 50.0)),
            ("a", (8.0, 18.0, 60.0, 50.0)),
            ("d", (8.0, 18.0, 60.0, 50.0)),
        ],
    );
}

/// A box against the paragraph breaks with it too, and one with borders and
/// padding is cut where its column ends.
#[test]
fn a_box_inside_a_positioned_paragraph_breaks_with_the_columns() {
    for (insets, left) in [("", 18.0), ("left:0;top:20px;", 8.0)] {
        check_painted(
            &format!(
                r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div id=p style="position:relative">x x x x <span id=r>y<span id=a style="position:absolute;width:30px;{insets}"><div id=d>xxx xxx xxx xxx xxx xxx</div></span></span> x x x x</div></div>"#
            ),
            &[
                ("a", (left, 8.0, 60.0, 40.0)),
                ("d", (left, 8.0, 60.0, 40.0)),
            ],
        );
    }
    check_painted(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div id=p>x x x x <span id=r style="position:relative">y<span id=a style="position:absolute;width:30px;left:0;top:0;border:2px solid;padding:3px"><div id=d>xxx xxx xxx xxx</div></span></span> x x x x</div></div>"#,
        &[("a", (8.0, 8.0, 70.0, 40.0))],
    );
}

/// A box on a line that the columns put in a later column of a container
/// with a definite height goes to the top of that column with the line,
/// whatever its containing block.
#[test]
fn a_box_on_a_line_of_a_later_column_goes_up_with_the_line() {
    let paragraph = |inline: &str, positioned: &str| {
        format!(
            r#"<div id=p{positioned}>x x x x x x x x <span id=r{inline}>y<span id=a style="position:absolute;width:30px"><div id=d>xxx</div></span></span> x x</div>"#
        )
    };
    let relative = r#" style="position:relative""#;
    for (container, inner) in [
        ("", paragraph(relative, "")),
        ("", paragraph("", "")),
        ("position:relative;", paragraph("", "")),
        ("", paragraph("", relative)),
        ("", format!("<div id=w>{}</div>", paragraph("", ""))),
    ] {
        check(
            &format!(
                r#"<div id=m style="{container}columns:2;column-gap:0;width:60px;height:40px;column-fill:auto">{inner}</div>"#
            ),
            &[
                ("r", (38.0, 8.0, 10.0, 10.0)),
                ("a", (48.0, 8.0, 30.0, 10.0)),
            ],
        );
    }
    // With an inset against the inline element, and broken in turn.
    check_painted(
        r#"<div id=m style="columns:3;column-gap:0;width:90px;height:40px;column-fill:auto"><div id=p>x x x x x x x x <span id=r style="position:relative">y<span id=a style="position:absolute;width:30px;left:0;top:0"><div id=d>xxx xxx xxx xxx xxx xxx</div></span></span> x x x x</div></div>"#,
        &[
            ("a", (38.0, 8.0, 60.0, 40.0)),
            ("d", (38.0, 8.0, 60.0, 40.0)),
        ],
    );
}
