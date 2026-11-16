//! Balanced columns hold the absolutely positioned boxes whose containing
//! block lies in them (CSS Position 3, 3.3.4; CSS Multi-column 1, 7): the
//! columns grow until such a box ends in the last one, or, for a monolithic
//! box, fits whole in its column, and the content in flow then fills them in
//! turn, as Chromium's balancer does.
//!
//! The expected rectangles are the ones Chromium gives the same markup in an
//! 800px by 600px viewport with a body margin of 8px, `div` boxes sized by
//! their border box and the Ahem font at 10px with a 10px line height: the
//! left, top, width and height of each piece of the border box of an element
//! with an `id`, as `getClientRects` lists them.

use raikiri_dom::PaintEvent;
use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The left, top, width and height of a box.
type Rect = (f32, f32, f32, f32);

const AHEM: &[u8] = include_bytes!("data/text-autospace/Ahem.ttf");

/// What the painter shows of each piece of the box of each element with an
/// `id` on the first page of `body`, laid out on 800px by 600px pages: each
/// piece cut by the clips around it, as a piece of a box broken between
/// columns paints the whole box clipped to its column.
fn pieces_of(body: &str) -> Vec<(String, Rect)> {
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
    let (document, cascade, _, _) = page.paint_inputs();
    let mut clips: Vec<(f32, f32, f32, f32)> = Vec::new();
    let mut pieces = Vec::new();
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
                    pieces.push((id.to_owned(), (x0, y0, x1 - x0, y1 - y0)));
                }
            }
            _ => {}
        }
    }
    pieces
}

/// Whether two rectangles agree to within 0.02px.
fn same(a: Rect, b: Rect) -> bool {
    [(a.0, b.0), (a.1, b.1), (a.2, b.2), (a.3, b.3)]
        .iter()
        .all(|(a, b)| (a - b).abs() <= 0.02)
}

/// Lay out `body` and check that the pieces of each `id` in `expected` are
/// exactly the ones listed for it.
fn check(body: &str, expected: &[(&str, Rect)]) {
    let pieces = pieces_of(body);
    let mut ids: Vec<&str> = expected.iter().map(|&(id, _)| id).collect();
    ids.dedup();
    for id in ids {
        let mut got: Vec<Rect> = pieces
            .iter()
            .filter(|(seen, _)| seen == id)
            .map(|(_, rect)| *rect)
            .collect();
        for &(_, want) in expected.iter().filter(|&&(seen, _)| seen == id) {
            let found = got.iter().position(|&rect| same(rect, want));
            let index = found
                .unwrap_or_else(|| panic!("#{id} has no piece {want:?} among {got:?}, in {body}"));
            got.remove(index);
        }
        assert!(got.is_empty(), "#{id} has more pieces {got:?}, in {body}");
    }
}

/// The columns of a box in flow with no lines grow until the positioned box
/// ends in the last one, wherever it starts and however many boxes there are.
#[test]
fn a_box_in_flow_with_no_lines_makes_room_for_its_positioned_box() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px"><div id=r style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;height:80px;top:0"></div></div></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 40.0)),
            ("r", (8.0, 8.0, 30.0, 40.0)),
            ("r", (38.0, 8.0, 30.0, 20.0)),
            ("a", (8.0, 8.0, 30.0, 40.0)),
            ("a", (38.0, 8.0, 30.0, 40.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px"><div id=r style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;height:80px;top:10px"></div></div></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 45.0)),
            ("r", (8.0, 8.0, 30.0, 45.0)),
            ("r", (38.0, 8.0, 30.0, 15.0)),
            ("a", (8.0, 18.0, 30.0, 35.0)),
            ("a", (38.0, 8.0, 30.0, 45.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px"><div id=r style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;height:150px;top:0"></div></div></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 75.0)),
            ("r", (8.0, 8.0, 30.0, 60.0)),
            ("a", (8.0, 8.0, 30.0, 75.0)),
            ("a", (38.0, 8.0, 30.0, 75.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:3;column-gap:0;width:90px"><div id=r style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;height:120px;top:0"></div></div></div>"#,
        &[
            ("m", (8.0, 8.0, 90.0, 40.0)),
            ("r", (8.0, 8.0, 30.0, 40.0)),
            ("r", (38.0, 8.0, 30.0, 20.0)),
            ("a", (8.0, 8.0, 30.0, 40.0)),
            ("a", (38.0, 8.0, 30.0, 40.0)),
            ("a", (68.0, 8.0, 30.0, 40.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px"><div id=r style="position:relative;height:20px"><div id=a style="position:absolute;width:30px;height:80px;top:0"></div></div><div id=b style="height:40px"></div></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 40.0)),
            ("r", (8.0, 8.0, 30.0, 20.0)),
            ("a", (8.0, 8.0, 30.0, 40.0)),
            ("a", (38.0, 8.0, 30.0, 40.0)),
            ("b", (8.0, 28.0, 30.0, 20.0)),
            ("b", (38.0, 8.0, 30.0, 20.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px"><div id=r style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:0"><span id=l1>xxx</span> <span id=l2>xxx</span> <span id=l3>xxx</span> <span id=l4>xxx</span> <span id=l5>xxx</span> <span id=l6>xxx</span> <span id=l7>xxx</span> <span id=l8>xxx</span></div></div></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 40.0)),
            ("r", (8.0, 8.0, 30.0, 40.0)),
            ("r", (38.0, 8.0, 30.0, 20.0)),
            ("a", (8.0, 8.0, 30.0, 40.0)),
            ("a", (38.0, 8.0, 30.0, 40.0)),
            ("l1", (8.0, 8.0, 30.0, 10.0)),
            ("l2", (8.0, 18.0, 30.0, 10.0)),
            ("l3", (8.0, 28.0, 30.0, 10.0)),
            ("l4", (8.0, 38.0, 30.0, 10.0)),
            ("l5", (38.0, 8.0, 30.0, 10.0)),
            ("l6", (38.0, 18.0, 30.0, 10.0)),
            ("l7", (38.0, 28.0, 30.0, 10.0)),
            ("l8", (38.0, 38.0, 30.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px"><div id=r style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;height:80px;top:0;margin-bottom:20px"></div></div></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 40.0)),
            ("r", (8.0, 8.0, 30.0, 40.0)),
            ("r", (38.0, 8.0, 30.0, 20.0)),
            ("a", (8.0, 8.0, 30.0, 40.0)),
            ("a", (38.0, 8.0, 30.0, 40.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px"><div id=r style="position:relative;height:20px"><div id=a style="position:absolute;width:30px;height:50px;top:0"></div></div><div id=r2 style="position:relative;height:20px"><div id=b style="position:absolute;width:30px;height:70px;top:0"></div></div></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 45.0)),
            ("r", (8.0, 8.0, 30.0, 20.0)),
            ("a", (8.0, 8.0, 30.0, 45.0)),
            ("a", (38.0, 8.0, 30.0, 5.0)),
            ("r2", (8.0, 28.0, 30.0, 20.0)),
            ("b", (8.0, 28.0, 30.0, 25.0)),
            ("b", (38.0, 8.0, 30.0, 45.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;padding:5px"><div id=r style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;height:80px;top:0"></div></div></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 50.0)),
            ("r", (13.0, 13.0, 25.0, 40.0)),
            ("r", (38.0, 13.0, 25.0, 20.0)),
            ("a", (13.0, 13.0, 30.0, 40.0)),
            ("a", (38.0, 13.0, 30.0, 40.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px"><div id=r style="position:relative;height:600px"><div id=a style="position:absolute;width:30px;height:1000px;top:0"></div></div></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 500.0)),
            ("r", (8.0, 8.0, 30.0, 500.0)),
            ("r", (38.0, 8.0, 30.0, 100.0)),
            ("a", (8.0, 8.0, 30.0, 500.0)),
            ("a", (38.0, 8.0, 30.0, 500.0)),
        ],
    );
}

/// A box that fits leaves the columns as they are; a minimum height leaves
/// the columns as tall as their content, a maximum height or a definite one
/// bounds them, and what is left goes on in overflow columns.
#[test]
fn columns_hold_a_positioned_box_within_their_limits() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px"><div id=r style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;height:20px;top:0"></div></div></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 30.0)),
            ("r", (8.0, 8.0, 30.0, 30.0)),
            ("r", (38.0, 8.0, 30.0, 30.0)),
            ("a", (8.0, 8.0, 30.0, 20.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px"><div id=r style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;height:60px;top:0"></div></div></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 30.0)),
            ("r", (8.0, 8.0, 30.0, 30.0)),
            ("r", (38.0, 8.0, 30.0, 30.0)),
            ("a", (8.0, 8.0, 30.0, 30.0)),
            ("a", (38.0, 8.0, 30.0, 30.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px"><p id=p style="margin:0;position:relative">xxx xxx xxx xxx<span id=a style="position:absolute;left:0;top:0;width:30px;height:15px"></span></p></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 20.0)),
            ("p", (8.0, 8.0, 30.0, 20.0)),
            ("p", (38.0, 8.0, 30.0, 20.0)),
            ("a", (8.0, 8.0, 30.0, 15.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;min-height:10px"><div id=r style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;height:80px;top:0"></div></div></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 40.0)),
            ("r", (8.0, 8.0, 30.0, 40.0)),
            ("r", (38.0, 8.0, 30.0, 20.0)),
            ("a", (8.0, 8.0, 30.0, 40.0)),
            ("a", (38.0, 8.0, 30.0, 40.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;min-height:100px"><p id=p style="margin:0;position:relative">xxx xxx xxx xxx xxx xxx<span id=a style="position:absolute;left:0;top:0;width:30px;height:80px"></span></p></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 100.0)),
            ("p", (8.0, 8.0, 30.0, 40.0)),
            ("p", (38.0, 8.0, 30.0, 20.0)),
            ("a", (8.0, 8.0, 30.0, 40.0)),
            ("a", (38.0, 8.0, 30.0, 40.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;max-height:35px"><div id=r style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;height:80px;top:0"></div></div></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 35.0)),
            ("r", (8.0, 8.0, 30.0, 35.0)),
            ("r", (38.0, 8.0, 30.0, 25.0)),
            ("a", (8.0, 8.0, 30.0, 35.0)),
            ("a", (38.0, 8.0, 30.0, 35.0)),
            ("a", (68.0, 8.0, 30.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:100px"><div id=r style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;height:80px;top:0"></div></div></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 100.0)),
            ("r", (8.0, 8.0, 30.0, 40.0)),
            ("r", (38.0, 8.0, 30.0, 20.0)),
            ("a", (8.0, 8.0, 30.0, 40.0)),
            ("a", (38.0, 8.0, 30.0, 40.0)),
        ],
    );
}

/// A positioned box against a paragraph or an inline box of it makes the
/// columns taller, and the lines then fill them in turn.
#[test]
fn a_paragraph_makes_room_for_a_positioned_box_on_its_lines() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px"><p id=p style="margin:0;position:relative">xxx xxx xxx xxx xxx xxx<span id=a style="position:absolute;left:0;top:0;width:30px;height:80px"></span></p></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 40.0)),
            ("p", (8.0, 8.0, 30.0, 40.0)),
            ("p", (38.0, 8.0, 30.0, 20.0)),
            ("a", (8.0, 8.0, 30.0, 40.0)),
            ("a", (38.0, 8.0, 30.0, 40.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px"><p id=p style="margin:0;position:relative"><span id=l1>xxx</span> xxx xxx <span id=l4>xxx</span> xxx <span id=l6>xxx</span><span id=a style="position:absolute;left:0;top:0;width:30px;height:80px"></span></p></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 40.0)),
            ("p", (8.0, 8.0, 30.0, 40.0)),
            ("p", (38.0, 8.0, 30.0, 20.0)),
            ("l1", (8.0, 8.0, 30.0, 10.0)),
            ("l4", (8.0, 38.0, 30.0, 10.0)),
            ("l6", (38.0, 18.0, 30.0, 10.0)),
            ("a", (8.0, 8.0, 30.0, 40.0)),
            ("a", (38.0, 8.0, 30.0, 40.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px"><p style="margin:0">xxx <span id=s style="position:relative">xxx<span id=a style="position:absolute;left:0;top:0;width:30px;height:80px"></span></span> xxx xxx xxx xxx</p></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 45.0)),
            ("s", (8.0, 18.0, 30.0, 10.0)),
            ("a", (8.0, 18.0, 30.0, 35.0)),
            ("a", (38.0, 8.0, 30.0, 45.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px"><p id=p style="margin:0">xxx xxx</p><p id=q style="margin:0">xxx <span id=s style="position:relative">xxx<span id=a style="position:absolute;left:0;top:0;width:30px;height:80px"></span></span></p></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 55.0)),
            ("p", (8.0, 8.0, 30.0, 20.0)),
            ("q", (8.0, 28.0, 30.0, 20.0)),
            ("s", (8.0, 38.0, 30.0, 10.0)),
            ("a", (8.0, 38.0, 30.0, 25.0)),
            ("a", (38.0, 8.0, 30.0, 55.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:3;column-gap:0;width:90px"><p id=p style="margin:0;position:relative">xxx xxx xxx xxx xxx xxx<span id=a style="position:absolute;left:0;top:0;width:30px;height:120px"></span></p></div>"#,
        &[
            ("m", (8.0, 8.0, 90.0, 40.0)),
            ("p", (8.0, 8.0, 30.0, 40.0)),
            ("p", (38.0, 8.0, 30.0, 20.0)),
            ("a", (8.0, 8.0, 30.0, 40.0)),
            ("a", (38.0, 8.0, 30.0, 40.0)),
            ("a", (68.0, 8.0, 30.0, 40.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px"><p id=p style="margin:0;position:relative">xxx xxx<span id=a style="position:absolute;left:0;top:0;width:30px;height:80px"></span></p><p id=q style="margin:0">xxx xxx xxx xxx</p></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 40.0)),
            ("p", (8.0, 8.0, 30.0, 20.0)),
            ("a", (8.0, 8.0, 30.0, 40.0)),
            ("a", (38.0, 8.0, 30.0, 40.0)),
            ("q", (8.0, 28.0, 30.0, 20.0)),
            ("q", (38.0, 8.0, 30.0, 20.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;widows:2;orphans:2"><p id=p style="margin:0;position:relative">xxx xxx xxx xxx xxx<span id=a style="position:absolute;left:0;top:0;width:30px;height:70px"></span></p></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 35.0)),
            ("p", (8.0, 8.0, 30.0, 35.0)),
            ("p", (38.0, 8.0, 30.0, 20.0)),
            ("a", (8.0, 8.0, 30.0, 35.0)),
            ("a", (38.0, 8.0, 30.0, 35.0)),
        ],
    );
}

/// The columns grow until the line a column end would cut fits, rather than
/// only until the box would fit if it could break anywhere.
#[test]
fn lines_inside_a_positioned_box_count_where_they_move() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><span id=l1>xxx</span> <span id=l2>xxx</span> <span id=l3>xxx</span> <span id=l4>xxx</span></div></div></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 40.0)),
            ("a", (8.0, 33.0, 30.0, 10.0)),
            ("a", (38.0, 8.0, 30.0, 30.0)),
            ("l1", (8.0, 33.0, 30.0, 10.0)),
            ("l2", (38.0, 8.0, 30.0, 10.0)),
            ("l3", (38.0, 18.0, 30.0, 10.0)),
            ("l4", (38.0, 28.0, 30.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px"><p id=p style="margin:0;position:relative">xxx xxx xxx xxx<span id=a style="position:absolute;left:0;top:5px;width:30px"><span id=l1>xxx</span> <span id=l2>xxx</span> <span id=l3>xxx</span> <span id=l4>xxx</span> <span id=l5>xxx</span> <span id=l6>xxx</span></span></p></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 35.0)),
            ("p", (8.0, 8.0, 30.0, 35.0)),
            ("p", (38.0, 8.0, 30.0, 20.0)),
            ("a", (8.0, 13.0, 30.0, 30.0)),
            ("a", (38.0, 8.0, 30.0, 30.0)),
            ("l1", (8.0, 13.0, 30.0, 10.0)),
            ("l2", (8.0, 23.0, 30.0, 10.0)),
            ("l3", (8.0, 33.0, 30.0, 10.0)),
            ("l4", (38.0, 8.0, 30.0, 10.0)),
            ("l5", (38.0, 18.0, 30.0, 10.0)),
            ("l6", (38.0, 28.0, 30.0, 10.0)),
        ],
    );
}

/// A positioned scroll container stays whole, so the columns grow to hold it
/// in the one it starts in.
#[test]
fn a_monolithic_positioned_box_stays_whole_in_its_column() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px"><div id=r style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;height:80px;top:0;overflow:hidden"></div></div></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 80.0)),
            ("r", (8.0, 8.0, 30.0, 60.0)),
            ("a", (8.0, 8.0, 30.0, 80.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px"><p style="margin:0">xxx <span id=s style="position:relative">xxx<span id=a style="position:absolute;left:0;top:0;width:30px;height:80px;overflow:hidden"></span></span> xxx xxx</p></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 90.0)),
            ("s", (8.0, 18.0, 30.0, 10.0)),
            ("a", (8.0, 18.0, 30.0, 80.0)),
        ],
    );
}

/// Columns that fill in turn under an auto height are one column that nothing
/// ends, which grows to hold the positioned box.
#[test]
fn one_column_that_nothing_ends_holds_a_positioned_box_whole() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><span id=l1>xxx</span> <span id=l2>xxx</span> <span id=l3>xxx</span> <span id=l4>xxx</span> <span id=l5>xxx</span> <span id=l6>xxx</span> <span id=l7>xxx</span> <span id=l8>xxx</span> <span id=l9>xxx</span></div></div></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 115.0)),
            ("a", (8.0, 33.0, 30.0, 90.0)),
            ("l1", (8.0, 33.0, 30.0, 10.0)),
            ("l2", (8.0, 43.0, 30.0, 10.0)),
            ("l3", (8.0, 53.0, 30.0, 10.0)),
            ("l4", (8.0, 63.0, 30.0, 10.0)),
            ("l5", (8.0, 73.0, 30.0, 10.0)),
            ("l6", (8.0, 83.0, 30.0, 10.0)),
            ("l7", (8.0, 93.0, 30.0, 10.0)),
            ("l8", (8.0, 103.0, 30.0, 10.0)),
            ("l9", (8.0, 113.0, 30.0, 10.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;column-fill:auto"><p id=p style="margin:0;position:relative">xxx xxx<span id=a style="position:absolute;left:0;top:0;width:30px;height:80px"></span></p></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 80.0)),
            ("p", (8.0, 8.0, 30.0, 20.0)),
            ("a", (8.0, 8.0, 30.0, 80.0)),
        ],
    );
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;column-fill:auto"><p id=p style="margin:0">xxx <span id=s style="position:relative">xxx<span id=a style="position:absolute;left:0;top:0;width:30px;height:80px"></span></span></p><p id=q style="margin:0">xxx</p></div>"#,
        &[
            ("m", (8.0, 8.0, 60.0, 90.0)),
            ("p", (8.0, 8.0, 30.0, 20.0)),
            ("s", (8.0, 18.0, 30.0, 10.0)),
            ("a", (8.0, 18.0, 30.0, 80.0)),
            ("q", (8.0, 28.0, 30.0, 10.0)),
        ],
    );
}
