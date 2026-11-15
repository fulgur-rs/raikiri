//! The last box of a block whose `height`, `min-height` or `max-height` is
//! set, around the end of a page, and the bottom padding and border of that
//! block. When the content box ends where the last box ends, or above its
//! end, the box moves to the next page with the edges, as it does in a block
//! of auto height, and the block keeps the height its constraints give it.
//! When the block leaves room below its content, the edges start the next
//! page whole instead. The lines of a paragraph before a block child that
//! ends it stay where they are, and the edges of the blocks around a block of
//! set height follow its border box. Where each line falls is what Chromium
//! prints for the same documents on pages 100px square, with Ahem glyphs of
//! 20px on lines of 30px.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Lays out `body` on the pages and with the font of the Chromium documents.
fn lay_out(body: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}} body{{margin:0;font:20px/30px Ahem}}</style><body>{body}</body>"
    );
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    result
}

/// The page, glyph and glyph top of every word of text in `body`.
fn glyph_tops(body: &str) -> Vec<(u32, char, i32)> {
    let result = lay_out(body);
    let mut tops = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            for word in run.text.split_whitespace() {
                let glyph = word.chars().next().unwrap();
                tops.push((index, glyph, (run.origin.1 - run.ascent).round() as i32));
            }
        }
    }
    tops.sort_unstable();
    tops
}

/// A document and the page, glyph and top of each of its words.
type Row = (&'static str, &'static [(u32, char, i32)]);

fn assert_rows(rows: &[Row]) {
    for &(body, expected) in rows {
        assert_eq!(glyph_tops(body), expected, "{body}");
    }
}

#[test]
fn the_last_box_moves_with_the_edges_of_a_box_of_set_height() {
    // A relative offset of the last box moves its paint only: the box is
    // measured where the flow puts it.
    assert_rows(&[
        (
            "<div style='height:20px'></div><div style='width:20px;height:80px;padding-bottom:15px'>X X<div style='height:20px;line-height:20px'>B</div></div><div>Z</div>",
            &[(0, 'X', 25), (0, 'X', 55), (1, 'B', 0), (1, 'Z', 20)],
        ),
        (
            "<div style='height:20px'></div><div style='width:20px;max-height:60px;padding-bottom:30px'>X X<div style='height:20px;line-height:20px'>B</div></div><div>Z</div>",
            &[(0, 'X', 25), (0, 'X', 55), (1, 'B', 0), (1, 'Z', 15)],
        ),
        (
            "<div style='width:20px;min-height:60px;padding-bottom:30px'>X X<div style='height:20px;line-height:20px'>B</div></div><div>Z</div>",
            &[(0, 'X', 5), (0, 'X', 35), (1, 'B', 0), (1, 'Z', 55)],
        ),
        (
            "<div style='width:20px;max-height:80px;padding-bottom:30px'>X X<div style='height:20px;line-height:20px'>B</div></div><div>Z</div>",
            &[(0, 'X', 5), (0, 'X', 35), (1, 'B', 0), (1, 'Z', 15)],
        ),
        (
            "<div style='height:40px'></div><div style='width:20px;max-height:60px;padding-bottom:15px'><div style='height:30px'></div><div style='height:20px;line-height:20px'>B</div></div><div>Z</div>",
            &[(1, 'B', 0), (1, 'Z', 20)],
        ),
        (
            "<div style='height:40px'></div><div style='width:20px;max-height:100px;padding-bottom:30px'><div style='height:30px'></div><div style='height:20px;line-height:20px'>B</div></div><div>Z</div>",
            &[(1, 'B', 0), (1, 'Z', 55)],
        ),
        (
            "<div style='height:20px'></div><div style='width:20px;height:80px;border-bottom:15px solid'>X X<div style='height:20px;line-height:20px'>B</div></div><div>Z</div>",
            &[(0, 'X', 25), (0, 'X', 55), (1, 'B', 0), (1, 'Z', 20)],
        ),
        (
            "<div style='height:20px'></div><div style='width:20px;box-sizing:border-box;max-height:90px;padding-bottom:30px'>X X<div style='height:20px;line-height:20px'>B</div></div><div>Z</div>",
            &[(0, 'X', 25), (0, 'X', 55), (1, 'B', 0), (1, 'Z', 15)],
        ),
        (
            "<div style='height:20px'></div><div style='height:60px;padding-bottom:30px'><div style='height:40px'>A</div><div style='position:relative;top:10px;height:20px;line-height:20px'>B</div></div><div>Z</div>",
            &[(0, 'A', 25), (1, 'B', 10), (1, 'Z', 15)],
        ),
        (
            "<div style='height:40px'></div><div style='height:60px;padding-bottom:15px'><div style='height:40px'>A</div><div style='position:relative;top:-10px;height:20px;line-height:20px'>B</div></div><div>Z</div>",
            &[(0, 'A', 45), (1, 'B', -10), (1, 'Z', 20)],
        ),
    ]);
}

#[test]
fn a_box_of_set_height_keeps_its_last_box_when_the_edges_fit_or_room_is_left() {
    assert_rows(&[
        (
            "<div style='height:20px'></div><div style='width:20px;height:60px;padding-bottom:15px'>X X<div style='height:20px;line-height:20px'>B</div></div><div>Z</div>",
            &[(0, 'B', 80), (0, 'X', 25), (0, 'X', 55), (1, 'Z', 5)],
        ),
        (
            "<div style='height:40px'></div><div style='width:20px;height:60px;padding-bottom:15px'><div style='height:30px'></div><div style='height:20px;line-height:20px'>B</div></div><div>Z</div>",
            &[(0, 'B', 70), (1, 'Z', 20)],
        ),
        (
            "<div style='height:40px'></div><div style='width:20px;min-height:60px;padding-bottom:15px'><div style='height:30px'></div><div style='height:20px;line-height:20px'>B</div></div><div>Z</div>",
            &[(0, 'B', 70), (1, 'Z', 20)],
        ),
        (
            "<div style='width:20px;height:80px;padding-bottom:15px'>X X<div style='height:20px;line-height:20px;margin-bottom:10px'>B</div></div><div>Z</div>",
            &[(0, 'B', 60), (0, 'X', 5), (0, 'X', 35), (1, 'Z', 5)],
        ),
        (
            "<div style='width:20px;min-height:60px;padding-bottom:15px'>X X<div style='height:20px;line-height:20px;margin-bottom:10px'>B</div></div><div>Z</div>",
            &[(0, 'X', 5), (0, 'X', 35), (1, 'B', 0), (1, 'Z', 50)],
        ),
    ]);
}

#[test]
fn the_edges_below_room_in_a_box_start_the_next_page_whole() {
    assert_rows(&[
        (
            "<div style='height:20px'></div><div style='width:20px;height:60px;padding-bottom:30px'>X<div style='height:20px;line-height:20px'>B</div></div><div>Z</div>",
            &[(0, 'B', 50), (0, 'X', 25), (1, 'Z', 35)],
        ),
        (
            "<div style='width:20px;height:80px;padding-bottom:30px'><div style='height:30px'></div><div style='height:20px;line-height:20px'>B</div></div><div>Z</div>",
            &[(0, 'B', 30), (1, 'Z', 35)],
        ),
        (
            "<div style='width:20px;min-height:80px;padding-bottom:30px'><div style='height:30px'></div><div style='height:20px;line-height:20px'>B</div></div><div>Z</div>",
            &[(0, 'B', 30), (1, 'Z', 35)],
        ),
        (
            "<div style='width:20px;min-height:90px;padding-bottom:15px'>X X<div style='height:20px'></div></div><div>Z</div>",
            &[(0, 'X', 5), (0, 'X', 35), (1, 'Z', 20)],
        ),
        (
            "<div style='height:20px'></div><div style='width:20px;height:60px;padding-bottom:30px'>X X</div><div>Z</div>",
            &[(0, 'X', 25), (1, 'X', 5), (1, 'Z', 15)],
        ),
        (
            "<div style='height:20px'></div><div style='width:20px;min-height:60px;padding-bottom:30px'><div style='height:30px'>A</div><div style='height:20px;line-height:20px'>B</div></div><div>Z</div>",
            &[(0, 'A', 25), (0, 'B', 50), (1, 'Z', 35)],
        ),
    ]);
}

#[test]
fn the_lines_before_a_block_that_ends_a_box_of_set_height_stay() {
    assert_rows(&[
        (
            "<div style='width:20px;max-height:90px;padding-bottom:15px'>X X X<div style='height:20px'></div></div><div>Z</div>",
            &[(0, 'X', 5), (0, 'X', 35), (0, 'X', 65), (1, 'Z', 10)],
        ),
        (
            "<div style='height:40px'></div><div style='width:20px;height:60px;padding-bottom:15px'>X X<div style='height:20px;line-height:20px'>B</div></div><div>Z</div>",
            &[(0, 'X', 45), (0, 'X', 75), (1, 'B', 0), (1, 'Z', 20)],
        ),
        (
            "<div style='height:20px'></div><div style='width:20px;max-height:150px;padding-bottom:30px'>X X X X X<div style='height:20px'></div></div><div>Z</div>",
            &[
                (0, 'X', 25),
                (0, 'X', 55),
                (1, 'X', 5),
                (1, 'X', 35),
                (1, 'X', 65),
                (2, 'Z', 5),
            ],
        ),
        (
            "<div style='height:20px'></div><div style='width:20px;height:80px;padding-bottom:30px'><div style='padding-bottom:5px'>X X<div style='height:20px;line-height:20px'>B</div></div></div><div>Z</div>",
            &[(0, 'X', 25), (0, 'X', 55), (1, 'B', 0), (1, 'Z', 35)],
        ),
        (
            "<div style='height:40px'></div><div style='width:20px;min-height:60px;padding-bottom:15px'><div style='padding-bottom:5px'>X X<div style='height:20px;line-height:20px'>B</div></div></div><div>Z</div>",
            &[(0, 'X', 45), (0, 'X', 75), (1, 'B', 0), (1, 'Z', 45)],
        ),
    ]);
}

#[test]
fn the_edges_around_a_box_of_set_height_follow_its_border_box() {
    assert_rows(&[
        (
            "<div style='padding-bottom:10px'><div style='width:20px;height:80px;padding-bottom:15px'>X X<div style='height:20px;line-height:20px'>B</div></div></div><div>Z</div>",
            &[(0, 'B', 60), (0, 'X', 5), (0, 'X', 35), (1, 'Z', 15)],
        ),
        (
            "<div style='padding-bottom:10px'><div style='width:20px;max-height:80px;padding-bottom:15px'>X X<div style='height:20px;line-height:20px'>B</div></div></div><div>Z</div>",
            &[(0, 'B', 60), (0, 'X', 5), (0, 'X', 35), (1, 'Z', 15)],
        ),
        (
            "<div style='height:20px'></div><div style='padding-bottom:10px'><div style='width:20px;max-height:100px;padding-bottom:15px'>X X<div style='height:20px;line-height:20px'>B</div></div></div><div>Z</div>",
            &[(0, 'X', 25), (0, 'X', 55), (1, 'B', 0), (1, 'Z', 50)],
        ),
        (
            "<div style='height:20px'></div><div style='padding-bottom:20px'><div style='min-height:40px;padding-bottom:30px'><div style='height:30px'>A</div><div style='height:20px;line-height:20px'>B</div></div></div><div>Z</div>",
            &[(0, 'A', 25), (1, 'B', 0), (1, 'Z', 75)],
        ),
        (
            "<div style='padding-bottom:20px'><div style='width:20px;min-height:40px;padding-bottom:15px'>X X<div style='height:20px;line-height:20px'>B</div></div></div><div>Z</div>",
            &[(0, 'X', 5), (0, 'X', 35), (1, 'B', 0), (1, 'Z', 60)],
        ),
    ]);
}
