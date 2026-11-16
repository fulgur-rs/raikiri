//! A column end inside an absolutely positioned box whose containing block
//! lies in the columns fragments the content of the box (CSS Position 3,
//! 3.3.4; CSS Fragmentation 3, 4.4): a line that the end would cut starts the
//! next column, with the blocks after it, a block whose first line or first
//! unbreakable box the end crosses goes there whole, and the boxes around
//! them grow to hold the room this leaves, as Chromium does.
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

/// Lay out `body` and check that each piece in `expected` is one of the
/// pieces of its `id`.
fn check_has(body: &str, expected: &[(&str, Rect)]) {
    let pieces = pieces_of(body);
    for &(id, want) in expected {
        assert!(
            pieces
                .iter()
                .any(|(seen, rect)| seen == id && same(*rect, want)),
            "#{id} has no piece {want:?} among {pieces:?}, in {body}"
        );
    }
}

#[test]
fn a_line_a_column_end_crosses_starts_the_next_column() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><span id=l1>xxx</span> <span id=l2>xxx</span> <span id=l3>xxx</span> <span id=l4>xxx</span></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 30.0)),
            ("l1", (8.0, 33.0, 30.0, 10.0)),
            ("l2", (38.0, 8.0, 30.0, 10.0)),
            ("l3", (38.0, 18.0, 30.0, 10.0)),
            ("l4", (38.0, 28.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn a_box_on_the_lines_moves_its_crossed_line_to_the_next_column() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative">x x x x x x x<div id=a style="position:absolute;width:30px;top:25px"><span id=l1>xxx</span> <span id=l2>xxx</span> <span id=l3>xxx</span> <span id=l4>xxx</span></div> x</div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 30.0)),
            ("l1", (8.0, 33.0, 30.0, 10.0)),
            ("l2", (38.0, 8.0, 30.0, 10.0)),
            ("l4", (38.0, 28.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn the_moved_line_leaves_room_at_the_end_of_its_column() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:22px"><span id=l1>xxx</span> <span id=l2>xxx</span> <span id=l3>xxx</span> <span id=l4>xxx</span></div></div></div>"#,
        &[
            ("a", (8.0, 30.0, 30.0, 18.0)),
            ("a", (38.0, 8.0, 30.0, 30.0)),
            ("l2", (38.0, 8.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn every_column_end_moves_the_line_it_crosses() {
    check(
        r#"<div id=m style="columns:3;column-gap:0;width:90px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><span id=l1>xxx</span> <span id=l2>xxx</span> <span id=l3>xxx</span> <span id=l4>xxx</span> <span id=l5>xxx</span> <span id=l6>xxx</span> <span id=l7>xxx</span> <span id=l8>xxx</span> <span id=l9>xxx</span></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 40.0)),
            ("a", (68.0, 8.0, 30.0, 40.0)),
            ("l2", (38.0, 8.0, 30.0, 10.0)),
            ("l5", (38.0, 38.0, 30.0, 10.0)),
            ("l6", (68.0, 8.0, 30.0, 10.0)),
            ("l9", (68.0, 38.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn lines_move_into_overflow_columns() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><span id=l1>xxx</span> <span id=l2>xxx</span> <span id=l3>xxx</span> <span id=l4>xxx</span> <span id=l5>xxx</span> <span id=l6>xxx</span> <span id=l7>xxx</span> <span id=l8>xxx</span> <span id=l9>xxx</span></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 40.0)),
            ("a", (68.0, 8.0, 30.0, 40.0)),
            ("l6", (68.0, 8.0, 30.0, 10.0)),
            ("l9", (68.0, 38.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn the_next_column_is_after_the_gap() {
    check(
        r#"<div id=m style="columns:2;column-gap:10px;width:70px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><span id=l1>xxx</span> <span id=l2>xxx</span> <span id=l3>xxx</span> <span id=l4>xxx</span></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (48.0, 8.0, 30.0, 30.0)),
            ("l2", (48.0, 8.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn a_box_whose_first_line_is_crossed_starts_empty() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:35px"><span id=l1>xxx</span> <span id=l2>xxx</span> <span id=l3>xxx</span></div></div></div>"#,
        &[
            ("a", (8.0, 43.0, 30.0, 5.0)),
            ("a", (38.0, 8.0, 30.0, 30.0)),
            ("l1", (38.0, 8.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn a_box_of_definite_height_keeps_it_and_its_lines_overflow() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px;height:40px"><span id=l1>xxx</span> <span id=l2>xxx</span> <span id=l3>xxx</span> <span id=l4>xxx</span></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 25.0)),
            ("l4", (38.0, 28.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn a_box_with_a_minimum_height_grows_past_it() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px;min-height:30px"><span id=l1>xxx</span> <span id=l2>xxx</span> <span id=l3>xxx</span> <span id=l4>xxx</span></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 30.0)),
        ],
    );
}

#[test]
fn a_box_with_a_maximum_height_grows_up_to_it() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px;max-height:42px"><span id=l1>xxx</span> <span id=l2>xxx</span> <span id=l3>xxx</span> <span id=l4>xxx</span></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 27.0)),
            ("l4", (38.0, 28.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn padding_stays_below_the_moved_lines() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px;padding-top:3px;padding-bottom:4px"><span id=l1>xxx</span> <span id=l2>xxx</span> <span id=l3>xxx</span> <span id=l4>xxx</span></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 34.0)),
            ("l1", (8.0, 36.0, 30.0, 10.0)),
            ("l2", (38.0, 8.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn a_border_and_padding_around_blocks_grow_with_them() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:22px;padding-bottom:5px;border-top:2px solid"><div id=c1>xxx xxx xxx</div></div></div></div>"#,
        &[
            ("a", (8.0, 30.0, 30.0, 18.0)),
            ("a", (38.0, 8.0, 30.0, 25.0)),
            ("c1", (8.0, 32.0, 30.0, 16.0)),
            ("c1", (38.0, 8.0, 30.0, 20.0)),
        ],
    );
}

#[test]
fn an_atomic_inline_moves_with_its_line() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px">xxx <span id=b style="display:inline-block;width:20px;height:12px"></span> xxx</div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 24.0)),
            ("b", (38.0, 8.0, 20.0, 12.0)),
        ],
    );
}

#[test]
fn a_block_after_a_moved_line_moves_with_it() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><div id=c1>xxx xxx</div><div id=c2>xxx xxx</div></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 30.0)),
            ("c1", (8.0, 33.0, 30.0, 15.0)),
            ("c1", (38.0, 8.0, 30.0, 10.0)),
            ("c2", (38.0, 18.0, 30.0, 20.0)),
        ],
    );
}

#[test]
fn nested_blocks_grow_and_move() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><div id=c1><div id=c2>xxx xxx</div><div id=c3>xxx</div></div><div id=c4>xxx</div></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 30.0)),
            ("c1", (8.0, 33.0, 30.0, 15.0)),
            ("c1", (38.0, 8.0, 30.0, 20.0)),
            ("c2", (8.0, 33.0, 30.0, 15.0)),
            ("c2", (38.0, 8.0, 30.0, 10.0)),
            ("c3", (38.0, 18.0, 30.0, 10.0)),
            ("c4", (38.0, 28.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn blocks_move_at_every_column_end() {
    check(
        r#"<div id=m style="columns:3;column-gap:0;width:90px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><div id=c1>xxx xxx xxx</div><div id=c2>xxx xxx xxx xxx</div><div id=c3>xxx xxx</div></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 40.0)),
            ("a", (68.0, 8.0, 30.0, 40.0)),
            ("c1", (8.0, 33.0, 30.0, 15.0)),
            ("c1", (38.0, 8.0, 30.0, 20.0)),
            ("c2", (38.0, 28.0, 30.0, 20.0)),
            ("c2", (68.0, 8.0, 30.0, 20.0)),
            ("c3", (68.0, 28.0, 30.0, 20.0)),
        ],
    );
}

#[test]
fn a_block_whose_first_line_is_crossed_goes_to_the_next_column() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:20px"><div id=c1 style="margin-bottom:3px">xxx</div><div id=c2 style="margin-top:6px">xxx xxx xxx</div></div></div></div>"#,
        &[
            ("a", (8.0, 28.0, 30.0, 20.0)),
            ("a", (38.0, 8.0, 30.0, 30.0)),
            ("c1", (8.0, 28.0, 30.0, 10.0)),
            ("c2", (38.0, 8.0, 30.0, 30.0)),
        ],
    );
}

#[test]
fn a_wrapper_whose_first_line_is_crossed_goes_with_it() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><div id=c0>xxx</div><div id=c1><div id=c2>xxx</div><div id=c3>xxx</div></div></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 20.0)),
            ("c1", (38.0, 8.0, 30.0, 20.0)),
            ("c2", (38.0, 8.0, 30.0, 10.0)),
            ("c3", (38.0, 18.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn a_block_goes_with_its_first_line_padding_and_all() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><div id=c0>xxx</div><div id=c1 style="padding-top:2px"><div id=c2>xxx</div></div></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 12.0)),
            ("c1", (38.0, 8.0, 30.0, 12.0)),
            ("c2", (38.0, 10.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn a_block_of_definite_height_overflows_and_the_next_block_moves() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><div id=c1 style="height:12px">xxx xxx</div><div id=c2>xxx xxx</div></div></div></div>"#,
        &[
            ("c1", (8.0, 33.0, 30.0, 12.0)),
            ("c2", (38.0, 8.0, 30.0, 20.0)),
        ],
    );
}

#[test]
fn a_scroll_container_starts_the_next_column_whole() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><div id=c1>xxx</div><div id=c2 style="height:12px;overflow:hidden">x</div><div id=c3>xxx</div></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 22.0)),
            ("c2", (38.0, 8.0, 30.0, 12.0)),
            ("c3", (38.0, 20.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn a_table_that_fits_a_column_starts_the_next_one_whole() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><div id=c1>xxx</div><div id=c2 style="display:table;width:30px"><div style="display:table-cell">xxx</div></div><div id=c3>xxx</div></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 20.0)),
            ("c2", (38.0, 8.0, 30.0, 10.0)),
            ("c3", (38.0, 18.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn a_flex_container_after_moved_lines_moves_with_them() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><div id=c1>xxx xxx</div><div id=c2 style="display:flex">xxx</div></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 20.0)),
            ("c1", (8.0, 33.0, 30.0, 15.0)),
            ("c1", (38.0, 8.0, 30.0, 10.0)),
            ("c2", (38.0, 18.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn a_wrapper_whose_first_box_is_a_crossed_flex_container_goes_with_it() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><div id=c0>xxx</div><div id=c1><div id=c2 style="display:flex">xxx</div></div><div id=c3>xxx</div></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 20.0)),
            ("c1", (38.0, 8.0, 30.0, 10.0)),
            ("c3", (38.0, 18.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn a_list_item_moves_like_a_block() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><div id=c1>xxx</div><div id=c2 style="display:list-item;list-style:none">xxx xxx</div></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 20.0)),
            ("c2", (38.0, 8.0, 30.0, 20.0)),
        ],
    );
}

#[test]
fn an_empty_block_moves_with_the_lines_before_it() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><div id=c1>xxx xxx</div><div id=e style="height:4px"></div><div id=c2>xxx</div></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 24.0)),
            ("c1", (8.0, 33.0, 30.0, 15.0)),
            ("c1", (38.0, 8.0, 30.0, 10.0)),
            ("e", (38.0, 18.0, 30.0, 4.0)),
            ("c2", (38.0, 22.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn lines_of_a_block_after_a_block_move() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><div id=c1>xxx</div><div id=c2><span>xxx</span> <span>xxx</span></div></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 20.0)),
            ("c2", (38.0, 8.0, 30.0, 20.0)),
        ],
    );
}

/// A float in the flow of the box is not moved: the box is cut at the column end as before, and its first piece is where Chromium puts it.
#[test]
fn a_box_holding_a_float_still_starts_in_its_first_column() {
    check_has(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><div id=c1 style="float:left;width:10px">x x</div><div id=c2>xxx xxx xxx</div></div></div></div>"#,
        &[("a", (8.0, 33.0, 30.0, 15.0))],
    );
}

/// A positioned box on the lines of the box is not moved, so neither are the lines.
#[test]
fn a_box_holding_a_positioned_box_still_starts_in_its_first_column() {
    check_has(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px">xxx xxx <span style="position:absolute">y</span>xxx</div></div></div>"#,
        &[("a", (8.0, 33.0, 30.0, 15.0))],
    );
}

/// A float in a block of the box stops the moves the same way.
#[test]
fn a_block_holding_a_float_still_starts_in_its_first_column() {
    check_has(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><div id=c0>xxx</div><div id=c1><div style="float:left;width:10px">x</div>xxx xxx</div></div></div></div>"#,
        &[("a", (8.0, 33.0, 30.0, 15.0))],
    );
}

/// A grid taller than a column is not kept whole, so the box is cut as before.
#[test]
fn a_grid_taller_than_a_column_still_starts_in_its_first_column() {
    check_has(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><div id=c1>xxx</div><div id=c2 style="display:grid">xxx xxx xxx xxx xxx</div></div></div></div>"#,
        &[("a", (8.0, 33.0, 30.0, 15.0))],
    );
}

/// The moves follow the flow only so deep.
#[test]
fn a_box_with_a_very_deep_flow_still_starts_in_its_first_column() {
    check_has(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div><div>xxx xxx</div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div></div>"#,
        &[("a", (8.0, 33.0, 30.0, 15.0))],
    );
}

#[test]
fn lines_overflowing_a_block_of_definite_height_still_move() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><div id=c1 style="height:20px"><span id=l1>xxx</span> <span id=l2>xxx</span> <span id=l3>xxx</span> <span id=l4>xxx</span></div><div id=c2>xxx</div></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 15.0)),
            ("c1", (8.0, 33.0, 30.0, 15.0)),
            ("c1", (38.0, 8.0, 30.0, 5.0)),
            ("l1", (8.0, 33.0, 30.0, 10.0)),
            ("l2", (38.0, 8.0, 30.0, 10.0)),
            ("l3", (38.0, 18.0, 30.0, 10.0)),
            ("l4", (38.0, 28.0, 30.0, 10.0)),
            ("c2", (38.0, 13.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn a_wrapper_with_a_minimum_height_grows_with_its_blocks() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><div id=w style="min-height:10px"><div id=c1>xxx xxx</div><div id=c2>xxx</div></div><div id=n>xxx</div></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 30.0)),
            ("w", (8.0, 33.0, 30.0, 15.0)),
            ("w", (38.0, 8.0, 30.0, 20.0)),
            ("c1", (8.0, 33.0, 30.0, 15.0)),
            ("c1", (38.0, 8.0, 30.0, 10.0)),
            ("c2", (38.0, 18.0, 30.0, 10.0)),
            ("n", (38.0, 28.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn a_wrapper_taller_than_its_content_takes_the_moves_first() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px"><div id=w style="min-height:50px"><div id=c1>xxx xxx</div></div><div id=n>xxx</div></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 40.0)),
            ("a", (68.0, 8.0, 30.0, 10.0)),
            ("w", (8.0, 33.0, 30.0, 15.0)),
            ("w", (38.0, 8.0, 30.0, 35.0)),
            ("c1", (8.0, 33.0, 30.0, 15.0)),
            ("c1", (38.0, 8.0, 30.0, 10.0)),
            ("n", (68.0, 8.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn a_box_its_insets_size_keeps_its_height() {
    check(
        r#"<div id=m style="columns:2;column-gap:0;width:60px;height:40px;column-fill:auto"><div style="position:relative;height:60px"><div id=a style="position:absolute;width:30px;top:25px;bottom:-10px"><span id=l1>xxx</span> <span id=l2>xxx</span> <span id=l3>xxx</span> <span id=l4>xxx</span></div></div></div>"#,
        &[
            ("a", (8.0, 33.0, 30.0, 15.0)),
            ("a", (38.0, 8.0, 30.0, 30.0)),
            ("l1", (8.0, 33.0, 30.0, 10.0)),
            ("l2", (38.0, 8.0, 30.0, 10.0)),
            ("l4", (38.0, 28.0, 30.0, 10.0)),
        ],
    );
}
