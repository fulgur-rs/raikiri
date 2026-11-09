//! The padding and border that close a box and do not fit under its last line
//! or box on the page that content ends on: they start the next page, even
//! when they are taller than a page, and split from there. Boxes that nest each
//! judge their own edges, the innermost first. In an item of a flex or grid
//! container, the edges move the item but not the items beside it. A block
//! with nothing in it has its edges pushed the same way, the edges a clip
//! hides stay where they are, and an image keeps its own. Where each line of
//! text falls is what Chromium prints for the same documents.
//!
//! Each row is a document and the page and top of every line of text in it (a
//! marker `Z` in a block after the box shows where the box ends), as Chromium
//! prints it on pages 100px tall and 100px wide. The first table has Ahem
//! lines of 20px with a line height of 30px, the second of 10px with a line
//! height of 10px, and so are those of the table of items beside others. The
//! last is boxes with nothing in them but edges.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The page and top of each line of text of `body` laid out in `font`, a
/// `font` shorthand without the family. `baseline` is how far below the top of
/// a line the text's baseline sits: Ahem's ascent is 0.8em, and its glyphs sit
/// in the middle of a line taller than the font size.
fn line_tops(body: &str, font: &str, baseline: f32) -> Vec<(usize, i32)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}}         body{{margin:0;font:{font} Ahem}}</style><body>{body}</body>"
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
    let mut tops = Vec::new();
    for index in 0..30 {
        let Some(page) = result.page(index) else {
            continue;
        };
        for run in page.text_runs() {
            // The words of a line are runs of their own in some paragraphs.
            tops.push((index as usize, (run.origin.1 - baseline).round() as i32));
        }
    }
    tops.sort_unstable();
    tops.dedup();
    tops
}

type Row = (&'static str, &'static [(usize, i32)]);

fn check(rows: &[Row], font: &str, baseline: f32) {
    for &(body, expected) in rows {
        assert_eq!(line_tops(body, font, baseline), expected, "{body}");
    }
}

#[test]
fn edges_that_do_not_fit_under_the_content_start_the_next_page_with_30px_lines() {
    check(ROWS_30PX_LINES, "20px/30px", 21.0);
}

#[test]
fn edges_that_do_not_fit_under_the_content_start_the_next_page_with_10px_lines() {
    check(ROWS_10PX_LINES, "10px/10px", 8.0);
}

#[test]
fn edges_in_an_item_move_its_row_but_not_the_items_beside_it() {
    check(ROWS_BESIDE_OTHERS, "10px/10px", 8.0);
}

#[test]
fn a_box_with_edges_above_nothing_and_no_height_moves_whole() {
    check(ROWS_EMPTY_BOXES, "10px/10px", 8.0);
}

/// Not Chromium's output: Chromium keeps an inline-block whole, but a tall
/// inline-block with room for content fragments across the pages, which the
/// css/printing fragmented-inline-block-002-print pair requires, so its
/// border-top does not take it to the next page.
#[test]
fn an_inline_block_with_room_for_content_keeps_fragmenting() {
    let rows: &[Row] = &[(
        r#"<div style="height:80px">S</div><div style="display:inline-block;width:20px;height:150px;border-top:5px solid"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (2, 37)],
    )];
    check(rows, "10px/10px", 8.0);
}

const ROWS_30PX_LINES: &[Row] = &[
    (
        r#"<div style="width:20px;padding-bottom:15px"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 50), (1, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:15px"><div style="height:50px"></div><div style="margin:0">X</div></div><div style="width:20px">Z</div>"#,
        &[(0, 50), (1, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:40px"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 70)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:40px"><div style="height:50px"></div><div style="margin:0">X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 70)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><div style="height:50px"></div><div style="margin:0">X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="height:50px"></div><div style="margin:0">X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:100px"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:100px"><div style="height:50px"></div><div style="margin:0">X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:50px"></div><div style="margin:0">X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:15px"><div style="height:50px"></div><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:15px"><div style="height:50px"></div><div style="margin:0">X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:40px"><div style="height:50px"></div><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:40px"><div style="height:50px"></div><div style="margin:0">X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><div style="height:50px"></div><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><div style="height:50px"></div><div style="margin:0">X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="height:50px"></div><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0), (4, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="height:50px"></div><div style="margin:0">X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0), (4, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:100px"><div style="height:50px"></div><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0), (4, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:100px"><div style="height:50px"></div><div style="margin:0">X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0), (4, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:50px"></div><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0), (4, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:50px"></div><div style="margin:0">X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0), (4, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:15px"><div style="height:50px"></div><p style="margin:0">X X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (2, 45)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:15px"><div style="height:50px"></div><div style="margin:0">X X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (2, 45)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:40px"><div style="height:50px"></div><p style="margin:0">X X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (2, 70)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:40px"><div style="height:50px"></div><div style="margin:0">X X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (2, 70)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><div style="height:50px"></div><p style="margin:0">X X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><div style="height:50px"></div><div style="margin:0">X X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="height:50px"></div><p style="margin:0">X X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (4, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="height:50px"></div><div style="margin:0">X X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (4, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:100px"><div style="height:50px"></div><p style="margin:0">X X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (4, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:100px"><div style="height:50px"></div><div style="margin:0">X X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (4, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:50px"></div><p style="margin:0">X X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (4, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:50px"></div><div style="margin:0">X X X</div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (1, 30), (2, 0), (4, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:15px"><div style="height:50px"></div><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(1, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><div style="height:50px"></div><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="height:50px"></div><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:100px"><div style="height:50px"></div><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:50px"></div><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(3, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:250px"><div style="height:50px"></div><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(4, 50)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:15px"><div style="height:50px"></div><div style="height:40px"></div></div><div style="width:20px">Z</div>"#,
        &[(1, 55)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><div style="height:50px"></div><div style="height:40px"></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="height:50px"></div><div style="height:40px"></div></div><div style="width:20px">Z</div>"#,
        &[(3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:100px"><div style="height:50px"></div><div style="height:40px"></div></div><div style="width:20px">Z</div>"#,
        &[(3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:50px"></div><div style="height:40px"></div></div><div style="width:20px">Z</div>"#,
        &[(3, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:250px"><div style="height:50px"></div><div style="height:40px"></div></div><div style="width:20px">Z</div>"#,
        &[(4, 50)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:250px"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (4, 50)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:300px"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (5, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:50px"></div><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0), (4, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:250px"><div style="height:50px"></div><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0), (5, 50)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:300px"><div style="height:50px"></div><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0), (6, 0)],
    ),
    (
        r#"<div style="width:20px;border-bottom:95px solid"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:47px;border-bottom:48px solid"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;border-bottom:130px solid"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:65px;border-bottom:65px solid"><div style="height:50px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="height:20px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:20px"></div><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (1, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (1, 0), (2, 0)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:60px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:60px"><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-top:20px;padding-bottom:60px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 20), (1, 60)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(1, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><div style="padding-bottom:20px"><p style="margin:0">X</p></div></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (1, 60)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:60px"><div style="height:50px"></div><div style="margin-bottom:10px"><p style="margin:0">X</p></div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (1, 0), (3, 0)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:95px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 0)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:95px"><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-top:20px;padding-bottom:95px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 20), (2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="padding-bottom:20px"><p style="margin:0">X</p></div></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (2, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:95px"><div style="height:50px"></div><div style="margin-bottom:10px"><p style="margin:0">X</p></div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 0)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (2, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (1, 0), (3, 30)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:130px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 30)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:130px"><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(3, 30)],
    ),
    (
        r#"<div style="width:20px;padding-top:20px;padding-bottom:130px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 20), (2, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(2, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="padding-bottom:20px"><p style="margin:0">X</p></div></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (2, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:130px"><div style="height:50px"></div><div style="margin-bottom:10px"><p style="margin:0">X</p></div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (3, 30)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:250px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (3, 50)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:250px"><p style="margin:0">X X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (1, 0), (4, 50)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:250px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (4, 50)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:250px"><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(4, 50)],
    ),
    (
        r#"<div style="width:20px;padding-top:20px;padding-bottom:250px"><p style="margin:0">X</p></div><div style="width:20px">Z</div>"#,
        &[(0, 20), (3, 50)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:250px"><div style="height:30px"></div></div><div style="width:20px">Z</div>"#,
        &[(3, 50)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:250px"><div style="padding-bottom:20px"><p style="margin:0">X</p></div></div><div style="width:20px">Z</div>"#,
        &[(0, 0), (3, 50)],
    ),
    (
        r#"<div style="width:20px;padding-bottom:250px"><div style="height:50px"></div><div style="margin-bottom:10px"><p style="margin:0">X</p></div></div><div style="width:20px">Z</div>"#,
        &[(1, 0), (4, 50)],
    ),
    (
        r#"<div style="height:0px"></div><div style="width:20px;padding-bottom:40px"><div style="padding-bottom:40px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(1, 0)],
    ),
    (
        r#"<div style="height:0px"></div><div style="width:20px;padding-bottom:60px"><div style="padding-bottom:20px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(1, 0)],
    ),
    (
        r#"<div style="height:0px"></div><div style="width:20px;padding-bottom:20px"><div style="padding-bottom:60px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(1, 0)],
    ),
    (
        r#"<div style="height:0px"></div><div style="width:20px;padding-bottom:80px"><div style="padding-bottom:30px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="height:0px"></div><div style="width:20px;padding-bottom:10px"><div style="padding-bottom:95px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(2, 10)],
    ),
    (
        r#"<div style="height:0px"></div><div style="width:20px;padding-bottom:95px"><div style="padding-bottom:10px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="height:20px"></div><div style="width:20px;padding-bottom:40px"><div style="padding-bottom:40px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="height:20px"></div><div style="width:20px;padding-bottom:60px"><div style="padding-bottom:20px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="height:20px"></div><div style="width:20px;padding-bottom:20px"><div style="padding-bottom:60px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="height:20px"></div><div style="width:20px;padding-bottom:80px"><div style="padding-bottom:30px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(3, 0)],
    ),
    (
        r#"<div style="height:20px"></div><div style="width:20px;padding-bottom:10px"><div style="padding-bottom:95px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(3, 10)],
    ),
    (
        r#"<div style="height:20px"></div><div style="width:20px;padding-bottom:95px"><div style="padding-bottom:10px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(3, 0)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:40px"><div style="padding-bottom:40px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:60px"><div style="padding-bottom:20px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:20px"><div style="padding-bottom:60px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(2, 0)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:80px"><div style="padding-bottom:30px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(3, 0)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:10px"><div style="padding-bottom:95px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(3, 10)],
    ),
    (
        r#"<div style="height:50px"></div><div style="width:20px;padding-bottom:95px"><div style="padding-bottom:10px"><div style="height:20px"></div></div></div><div style="width:20px">Z</div>"#,
        &[(3, 0)],
    ),
];

const ROWS_10PX_LINES: &[Row] = &[
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:30px"><div style="height:40px">A</div><div style="height:30px">B</div><div style="height:20px">C</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 40), (1, 0), (1, 50)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:90px"><div style="height:40px">A</div><div style="height:30px">B</div><div style="height:20px">C</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 40), (1, 0), (2, 90)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:15px"><div style="height:10px">A</div><div style="height:90px">B</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0), (2, 15)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:100px"><div style="height:10px">A</div><div style="height:90px">B</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0), (3, 0)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:250px"><div style="height:10px">A</div><div style="height:90px">B</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0), (4, 50)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:30px"><div style="height:80px">A</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:50px">S</div><div style="padding-bottom:95px"><div style="width:30px">aaa aaa</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 50), (1, 0), (3, 0)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:50px">S</div><div style="padding-bottom:250px"><div style="width:30px">aaa aaa</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 50), (1, 0), (4, 50)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:90px"><div style="height:10px">S</div><div style="width:30px">aaa aaa aaa aaa aaa aaa</div></div><div style="height:10px">Z</div>"#,
        &[
            (0, 0),
            (0, 10),
            (0, 20),
            (0, 30),
            (0, 40),
            (1, 0),
            (1, 10),
            (2, 90),
        ],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:90px"><div style="height:60px">S</div><div style="width:30px;padding-bottom:5px">aaa</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0), (2, 90)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><div style="padding-bottom:30px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><div style="border-bottom:30px solid"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><div style="padding-bottom:30px"><div style="height:0"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><div style="padding-bottom:30px"> </div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><div style="padding-bottom:20px"><div style="padding-bottom:30px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 50)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><p style="margin:0;padding-bottom:30px"></p><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:60px">S</div><div style="height:40px;overflow:hidden"><div style="padding-bottom:50px"><div style="height:20px">A</div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 60), (1, 0)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><img style="display:block;width:20px;height:90px;padding-bottom:30px"><div style="height:10px">Z</div>"#,
        &[(1, 20)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:20px">S</div><img style="display:block;width:20px;height:90px;padding-bottom:30px"><div style="height:10px">Z</div>"#,
        &[(0, 0), (2, 20)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><div style="padding-bottom:30px"><div style="float:left;width:10px;height:10px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><div style="padding-bottom:30px"><div style="position:absolute;width:10px;height:10px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:100px">S</div><div style="padding-bottom:30px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><div style="padding-bottom:250px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (3, 50)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:30px"></div><div style="height:10px">Z</div>"#,
        &[(0, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:70px">S</div><div style="padding-bottom:30px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><div style="display:flow-root;padding-bottom:30px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:60px">S</div><div style="padding-bottom:20px"><div style="height:30px"></div><div style="display:inline-block;width:60px;padding-top:6px"><div style="height:20px;break-inside:avoid">z</div></div><div style="height:10px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 6), (1, 56)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:30px"><div style="height:60px">A</div><div style="height:50px"><div style="height:60px">B</div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 60), (1, 40)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:30px"><div style="height:60px">A</div><div style="height:50px"><div style="height:20px">B</div><div style="padding-bottom:30px"><div style="height:10px">C</div></div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 60), (1, 0), (1, 40)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:60px">S</div><div style="padding-bottom:70px"><div style="height:30px"></div><div style="display:inline-block;width:60px;padding-top:6px"><div style="height:20px;break-inside:avoid">z</div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 6), (2, 0)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:60px">S</div><div style="padding-bottom:80px"><div style="height:30px"></div><div style="display:inline-block;width:60px;padding-top:6px"><div style="height:20px;break-inside:avoid">z</div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 6), (2, 80)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:60px">S</div><div style="padding-bottom:120px"><div style="height:30px"></div><div style="display:inline-block;width:60px;padding-top:6px"><div style="height:20px;break-inside:avoid">z</div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 6), (3, 20)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:90px">S</div><div style="display:inline-block;width:40px;height:26px"></div><div style="display:inline-block;width:60px;padding-top:6px"><div style="height:20px;break-inside:avoid">z</div></div><div style="padding-bottom:30px;break-before:page"><div style="height:10px">D</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 18), (2, 0), (2, 40)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:90px">S</div><div style="display:inline-block;vertical-align:top;width:40px;height:10px"></div><div style="display:inline-block;vertical-align:top;width:60px;padding-top:6px"><div style="height:20px;break-inside:avoid">z</div></div><div style="padding-bottom:30px;break-before:page"><div style="height:10px">D</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 6), (2, 0), (2, 40)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:90px">S</div><div style="display:inline-block;width:40px;height:26px"></div><div style="display:inline-block;width:60px;padding-top:6px"><div style="height:20px;break-inside:avoid">z</div></div><div style="padding-bottom:95px;break-before:page"><div style="height:10px">D</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 18), (2, 0), (4, 0)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:90px">S</div><div style="display:inline-block;vertical-align:top;width:40px;height:10px"></div><div style="display:inline-block;vertical-align:top;width:60px;padding-top:6px"><div style="height:20px;break-inside:avoid">z</div></div><div style="padding-bottom:95px;break-before:page"><div style="height:10px">D</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 6), (2, 0), (4, 0)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><div style="padding-bottom:30px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><div style="position:relative;top:-30px;padding-bottom:30px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><div style="position:relative;top:30px;padding-bottom:30px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><div style="position:relative;bottom:30px;padding-bottom:30px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="padding-bottom:30px"><div style="height:80px">S</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="position:relative;top:30px;padding-bottom:30px"><div style="height:80px">S</div></div><div style="height:10px">Z</div>"#,
        &[(0, 30), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="position:relative;top:30px;padding-bottom:30px"><div style="height:80px;width:20px">S</div><div style="width:20px">aa</div></div><div style="height:10px">Z</div>"#,
        &[(0, 30), (1, 30), (1, 40)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="position:relative;top:25px;"><div style="height:60px">S</div><div style="padding-bottom:30px"><div style="height:20px">T</div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 25), (1, 25), (1, 50)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="position:relative;top:25px;"><div style="height:80px">S</div><div style="padding-bottom:30px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 25), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><button style="display:block;width:50px;border:0;margin:0;padding:0 0 30px;font:10px/10px Ahem;background:none">B</button><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0), (1, 40)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><button style="display:block;width:50px;border:0;margin:0;padding:0 0 130px;font:10px/10px Ahem;background:none">B</button><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0), (2, 40)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><button style="display:block;width:50px;border:0;margin:0;padding:0 0 130px;font:10px/10px Ahem;background:none">B</button><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 40)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><button style="display:block;width:50px;border:0;margin:0;padding:0 0 30px;font:10px/10px Ahem;background:none">B</button><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 40)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:80px">S</div><img style="display:block;width:20px;height:10px;padding-bottom:130px"><div style="height:10px">Z</div>"#,
        &[(0, 0), (2, 40)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:50px;overflow:hidden"><div style="padding-bottom:30px"><div style="height:80px">S</div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 50)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="display:flow-root;padding-bottom:30px"><div style="height:40px">A</div><div style="height:50px">B</div><div style="float:left;width:20px;height:5px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0), (1, 85)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="display:flow-root;padding-bottom:30px"><div style="height:40px">A</div><div style="height:50px">B</div><div style="float:left;width:20px;height:0px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0), (1, 80)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="display:flow-root;padding-bottom:30px"><div style="height:40px">A</div><div style="height:50px">B</div><div style="float:left;width:20px;height:5px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0), (1, 85)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="display:flow-root;padding-bottom:30px"><div style="height:40px">A</div><div style="height:50px">B</div><div style="float:left;width:20px;height:20px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 40), (1, 40)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="display:block;padding-bottom:30px"><div style="height:40px">A</div><div style="height:50px">B</div><div style="float:left;width:20px;height:0px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0), (1, 80)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="display:block;padding-bottom:30px"><div style="height:40px">A</div><div style="height:50px">B</div><div style="float:left;width:20px;height:5px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0), (1, 80)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="display:block;padding-bottom:30px"><div style="height:40px">A</div><div style="height:50px">B</div><div style="float:left;width:20px;height:20px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 40), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:50px;overflow:hidden"><div style="padding-bottom:30px"><div style="height:20px">S</div><div style="height:70px">T</div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 20), (0, 50)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:50px;overflow:hidden"><div style="padding-bottom:30px"><div style="height:20px">S</div><div style="height:50px">T</div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 20), (0, 50)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:50px;overflow:hidden"><div style="padding-bottom:30px"><div style="height:20px">S</div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 50)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="height:60px;overflow:hidden"><div style="height:20px">S</div><div style="padding-bottom:30px"><div style="height:30px">T</div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 20), (0, 60)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="position:relative;padding-bottom:30px"><div style="height:80px">S</div><div style="position:absolute;bottom:0;left:0;height:10px;width:10px">A</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="position:relative;padding-bottom:30px"><div style="height:60px">S</div><div style="position:absolute;bottom:0;left:0;height:10px;width:10px">A</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 80), (0, 90)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="position:relative;padding-bottom:30px"><div style="height:80px">S</div><div style="position:absolute;top:0;left:0;height:10px;width:10px">A</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="position:relative;padding-bottom:30px"><div style="height:80px">S</div><div style="position:absolute;bottom:20px;left:0;height:10px;width:10px">A</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 80), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="display:grid"><div style="padding-bottom:30px"><div style="height:80px">S</div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="display:grid;grid-template-columns:50px 50px;align-items:start"><div style="padding-bottom:30px"><div style="height:80px">S</div></div><div style="width:50px">T</div></div>"#,
        &[(0, 0)],
    ),
    (
        r#"<style>body{font:10px/10px Ahem}</style><div style="display:grid;grid-template-columns:50px 50px;align-items:start"><div style="padding-bottom:30px"><div style="height:80px">S</div></div><div style="width:50px"><div style="height:10px">T</div><div style="height:10px">U</div></div></div>"#,
        &[(0, 0), (0, 10)],
    ),
];

/// Edges in an item of a flex container or of a grid of columns, with Ahem
/// lines of 10px. The item grows and the container with it, so the flow after
/// the container follows, but the items beside it keep their places; the items
/// on later rows or lines, and the boxes after the block in its own item, move
/// down with it.
const ROWS_BESIDE_OTHERS: &[Row] = &[
    (
        r#"<div style="display:grid;grid-template-columns:50px 50px;align-items:start"><div style="padding-bottom:30px"><div style="height:80px">S</div></div><div style="width:50px">T</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<div style="display:flex;align-items:start"><div style="padding-bottom:30px;width:50px"><div style="height:80px">S</div></div><div style="width:50px">T</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<div style="display:flex"><div style="padding-bottom:30px;width:50px"><div style="height:80px">S</div></div><div style="width:50px">T</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<div style="display:grid;grid-template-columns:50px 50px"><div style="padding-bottom:30px"><div style="height:80px">S</div></div><div style="width:50px">T</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<div style="display:flex;align-items:start"><div style="padding-bottom:30px;width:50px"><div style="height:80px">S</div></div><div style="width:50px"><div style="height:85px"></div>T</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (0, 85), (1, 30)],
    ),
    (
        r#"<div style="display:flex;align-items:start"><div style="width:50px"><div style="padding-bottom:30px"><div style="height:80px">S</div></div><div style="height:10px">U</div></div><div style="width:50px">T</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30), (1, 40)],
    ),
    (
        r#"<div style="display:flex;align-items:start;padding-bottom:10px"><div style="padding-bottom:30px;width:50px"><div style="height:80px">S</div></div><div style="width:50px">T</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 40)],
    ),
    (
        r#"<div style="display:grid;grid-template-columns:50px 50px;align-items:start"><div style="padding-bottom:30px"><div style="height:80px">S</div></div><div>T</div><div>U</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30), (1, 40)],
    ),
    (
        r#"<div style="display:flex;flex-direction:column"><div style="padding-bottom:30px;width:50px"><div style="height:80px">S</div></div><div style="width:50px">T</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30), (1, 40)],
    ),
    (
        r#"<div style="display:flex;align-items:start"><div style="padding-bottom:30px;width:50px"><div style="height:80px">S</div></div><div style="padding-bottom:30px;width:50px"><div style="height:80px">T</div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<div style="display:flex;flex-wrap:wrap;align-items:start;width:100px"><div style="padding-bottom:30px;width:50px"><div style="height:80px">S</div></div><div style="width:50px">T</div><div style="width:100px">U</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30), (1, 40)],
    ),
    (
        r#"<div style="display:flex;align-items:start"><div style="padding-bottom:30px;width:50px"><div style="height:70px">S</div></div><div style="width:50px;padding-bottom:30px"><div style="height:75px">T</div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<div style="display:flex;align-items:start;height:100px"><div style="padding-bottom:30px;width:50px"><div style="height:80px">S</div></div><div style="width:50px">T</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0)],
    ),
    (
        r#"<div style="display:flex;align-items:start"><div style="padding-bottom:30px;width:50px"><div style="height:80px">S</div></div><div style="display:contents"><div style="height:150px;width:50px">T</div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 50)],
    ),
    (
        r#"<div style="display:flex;align-items:start"><div style="width:50px"><div style="height:100px"><div style="padding-bottom:30px"><div style="height:80px">S</div></div><div style="height:10px">C</div></div><div style="height:10px">D</div></div><div style="width:50px">T</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0), (1, 10), (1, 30)],
    ),
    (
        r#"<div style="display:grid;grid-template-columns:50px 50px;grid-template-rows:auto auto;grid-auto-flow:column;align-items:start"><div style="padding-bottom:30px"><div style="height:80px">S</div></div><div>U</div><div>T</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30), (1, 40)],
    ),
    (
        r#"<div style="display:flex;align-items:start;contain:size"><div style="padding-bottom:30px;width:50px"><div style="height:80px">S</div></div><div style="width:50px">T</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0)],
    ),
    (
        r#"<div style="display:flex;align-items:start"><div style="display:contents"><div style="padding-bottom:30px;width:50px"><div style="height:80px">S</div></div></div><div style="width:50px">T</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<div style="display:flex;align-items:start">
 <div style="padding-bottom:30px;width:50px"><div style="height:80px">S</div></div>
 <div style="width:50px">T</div>
</div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<div style="display:flex"><div style="padding-bottom:30px;width:50px"><div style="height:80px">S</div></div><div style="padding-bottom:30px;width:50px"><div style="height:80px">T</div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<div style="display:flex;align-items:flex-start"><div style="padding-bottom:30px;width:50px"><div style="height:80px">S</div></div><div style="width:50px;height:20px;margin-top:120px">T</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 20), (1, 40)],
    ),
    (
        r#"<div style="display:flex"><div style="padding-bottom:30px;width:50px"><div style="height:80px">S</div></div><div style="padding-bottom:30px;width:50px"><div style="height:50px">T</div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    (
        r#"<div style="display:flex;align-items:flex-start"><div style="padding-bottom:30px;width:50px"><div style="height:80px">S</div></div><div style="width:50px;height:20px;position:relative;top:120px">T</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 20), (1, 30)],
    ),
];

/// Boxes that hold no box of their own, after a spacer of 80px on pages 100px
/// tall. The edges above nothing are all there is to keep together, so a box
/// with no height of its own goes to the next page whole when any of it would
/// cross the page end, bottom edges included; a box with room for content is
/// cut after the edges above it.
const ROWS_EMPTY_BOXES: &[Row] = &[
    // A_pt5_pb30
    (
        r#"<div style="height:80px">S</div><div style="padding-top:5px;padding-bottom:30px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 35)],
    ),
    // B_pt5_pb10
    (
        r#"<div style="height:80px">S</div><div style="padding-top:5px;padding-bottom:10px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0)],
    ),
    // C_pt5_pb15
    (
        r#"<div style="height:80px">S</div><div style="padding-top:5px;padding-bottom:15px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0)],
    ),
    // D_pt5_pb16
    (
        r#"<div style="height:80px">S</div><div style="padding-top:5px;padding-bottom:16px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 21)],
    ),
    // E_pt25
    (
        r#"<div style="height:80px">S</div><div style="padding-top:25px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // F_bt25
    (
        r#"<div style="height:80px">S</div><div style="border-top:25px solid"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // G_pb30
    (
        r#"<div style="height:80px">S</div><div style="padding-bottom:30px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    // H_pt15_pb10
    (
        r#"<div style="height:80px">S</div><div style="padding-top:15px;padding-bottom:10px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // I_h10_pt5
    (
        r#"<div style="height:80px">S</div><div style="height:10px;padding-top:5px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0)],
    ),
    // J_h30_pt5
    (
        r#"<div style="height:80px">S</div><div style="height:30px;padding-top:5px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 15)],
    ),
    // K_pt5_h30_pb5
    (
        r#"<div style="height:80px">S</div><div style="height:30px;padding-top:5px;padding-bottom:5px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 20)],
    ),
    // L_mt5_pt5_pb30
    (
        r#"<div style="height:80px">S</div><div style="padding-top:5px;padding-bottom:30px;margin-top:5px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 35)],
    ),
    // M_pt150
    (
        r#"<div style="height:80px">S</div><div style="padding-top:150px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (2, 50)],
    ),
    // N_pt5_pb150
    (
        r#"<div style="height:80px">S</div><div style="padding-top:5px;padding-bottom:150px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (3, 50)],
    ),
    // O_pt20
    (
        r#"<div style="height:80px">S</div><div style="padding-top:20px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0)],
    ),
    // P_pt21
    (
        r#"<div style="height:80px">S</div><div style="padding-top:21px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 21)],
    ),
    // F1_float_pt25
    (
        r#"<div style="height:80px">S</div><div style="float:left;width:20px;padding-top:25px"></div><div style="height:10px;clear:both">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // F2_float_pt5_pb30
    (
        r#"<div style="height:80px">S</div><div style="float:left;width:20px;padding-top:5px;padding-bottom:30px"></div><div style="height:10px;clear:both">Z</div>"#,
        &[(0, 0), (1, 35)],
    ),
    // F3_float_pb30
    (
        r#"<div style="height:80px">S</div><div style="float:left;width:20px;padding-bottom:30px"></div><div style="height:10px;clear:both">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    // F4_ib_pt25
    (
        r#"<div style="height:80px">S</div><div style="display:inline-block;width:20px;padding-top:25px"></div><div style="height:10px;clear:both">Z</div>"#,
        &[(0, 0), (1, 27)],
    ),
    // F5_flow_pt25
    (
        r#"<div style="height:80px">S</div><div style="display:flow-root;padding-top:25px"></div><div style="height:10px;clear:both">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // F6_flow_pt5_pb30
    (
        r#"<div style="height:80px">S</div><div style="display:flow-root;padding-top:5px;padding-bottom:30px"></div><div style="height:10px;clear:both">Z</div>"#,
        &[(0, 0), (1, 35)],
    ),
    // F7_relative_pt25
    (
        r#"<div style="height:80px">S</div><div style="position:relative;padding-top:25px"></div><div style="height:10px;clear:both">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // F8_overflow_pt25
    (
        r#"<div style="height:80px">S</div><div style="overflow:hidden;padding-top:25px"></div><div style="height:10px;clear:both">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // F9_pt25_h0
    (
        r#"<div style="height:80px">S</div><div style="height:0;padding-top:25px"></div><div style="height:10px;clear:both">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // F10_pt25_mh
    (
        r#"<div style="height:80px">S</div><div style="min-height:5px;padding-top:25px"></div><div style="height:10px;clear:both">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    // N1_wrap_child_pt25
    (
        r#"<div style="height:80px">S</div><div><div style="padding-top:25px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // N2_wrap_pt5_child_pt20
    (
        r#"<div style="height:80px">S</div><div style="padding-top:5px"><div style="padding-top:20px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // N3_wrap_pt5_child_pb20
    (
        r#"<div style="height:80px">S</div><div style="padding-top:5px"><div style="padding-bottom:20px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // N4_two_empties
    (
        r#"<div style="height:80px">S</div><div style="padding-top:5px;padding-bottom:5px"></div><div style="padding-top:10px;padding-bottom:10px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 20)],
    ),
    // N5_bt2_bb30
    (
        r#"<div style="height:80px">S</div><div style="border-top:2px solid;border-bottom:30px solid"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 32)],
    ),
    // N6_mt15_pt10
    (
        r#"<div style="height:80px">S</div><div style="margin-top:15px;padding-top:10px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 10)],
    ),
    // N8_sticky
    (
        r#"<div style="height:80px">S</div><div style="position:sticky;top:0;padding-top:25px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // N9_bb_h30
    (
        r#"<div style="height:80px">S</div><div style="box-sizing:border-box;height:30px;padding-top:5px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 10)],
    ),
    // N10_minh0
    (
        r#"<div style="height:80px">S</div><div style="min-height:0;padding-top:25px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // N11_last_in_padded
    (
        r#"<div style="height:80px">S</div><div style="padding-bottom:5px"><div style="padding-top:25px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    // N12_after_empty_pb
    (
        r#"<div style="height:80px">S</div><div style="padding-bottom:5px"></div><div style="padding-top:25px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // N13_white
    (
        r#"<div style="height:80px">S</div><div style="padding-top:25px">  </div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // N14_nested_empty_child
    (
        r#"<div style="height:80px">S</div><div style="padding-top:5px;padding-bottom:20px"><div></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // N15_avoid
    (
        r#"<div style="height:80px">S</div><div style="break-inside:avoid;padding-top:5px;padding-bottom:20px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // N16_mb
    (
        r#"<div style="height:80px">S</div><div style="padding-top:25px;margin-bottom:30px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 55)],
    ),
    // N17_pt25_pb5
    (
        r#"<div style="height:80px">S</div><div style="padding-top:25px;padding-bottom:5px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    // N18_pt19
    (
        r#"<div style="height:80px">S</div><div style="padding-top:19px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0)],
    ),
    // N19_pt19_pb1
    (
        r#"<div style="height:80px">S</div><div style="padding-top:19px;padding-bottom:1px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0)],
    ),
    // N20_pt19_pb2
    (
        r#"<div style="height:80px">S</div><div style="padding-top:19px;padding-bottom:2px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 21)],
    ),
    // N21_background
    (
        r#"<div style="height:80px">S</div><div style="padding-top:25px;background:red"></div><div style="height:10px">W</div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 25), (1, 35)],
    ),
    // R2_rel_down_empty
    (
        r#"<div style="height:80px">S</div><div style="position:relative;top:10px;padding-top:25px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // R3_rel_up_p
    (
        r#"<div style="height:80px">S</div><div style="position:relative;top:-10px;padding-top:25px"><p style="margin:0">X</p></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 15), (1, 35)],
    ),
    // flex item with bottom padding
    (
        r#"<div style="height:80px">S</div><div style="display:flex"><div style="padding-bottom:30px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    // flex container padding-top, item bottom padding
    (
        r#"<div style="height:80px">S</div><div style="display:flex;padding-top:5px"><div style="padding-bottom:30px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 35)],
    ),
    // flex item with padding-top
    (
        r#"<div style="height:80px">S</div><div style="display:flex"><div style="padding-top:25px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // flex item with both paddings
    (
        r#"<div style="height:80px">S</div><div style="display:flex"><div style="padding-top:5px;padding-bottom:30px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 35)],
    ),
    // two flex items, one of them empty
    (
        r#"<div style="height:80px">S</div><div style="display:flex"><div style="padding-bottom:30px;width:10px"></div><div style="width:10px">x</div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 0), (1, 30)],
    ),
    // grid item with padding-top
    (
        r#"<div style="height:80px">S</div><div style="display:grid"><div style="padding-top:25px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // grid padding-top, item bottom padding
    (
        r#"<div style="height:80px">S</div><div style="display:grid;padding-top:5px"><div style="padding-bottom:30px"></div></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    // list item with padding-top
    (
        r#"<div style="height:80px">S</div><ul style="margin:0;padding:0;list-style:none"><li style="padding-top:25px"></li></ul><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // list item with both paddings
    (
        r#"<div style="height:80px">S</div><ul style="margin:0;padding:0;list-style:none"><li style="padding-top:5px;padding-bottom:30px"></li></ul><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 35)],
    ),
    // empty multicol container with padding-top
    (
        r#"<div style="height:80px">S</div><div style="column-count:2;padding-top:25px"></div><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // block image with padding-top
    (
        r#"<div style="height:80px">S</div><img src="data:image/svg+xml,%3Csvg xmlns=%27http://www.w3.org/2000/svg%27 width=%2720%27 height=%2730%27%3E%3Crect width=%2720%27 height=%2730%27/%3E%3C/svg%3E" style="display:block;padding-top:5px;height:60px;width:20px"><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 65)],
    ),
    // block button with padding-top
    (
        r#"<div style="height:80px">S</div><button style="display:block;padding:25px 0 0 0;border:0;height:25px;width:20px"></button><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // block svg with padding-top
    (
        r#"<div style="height:80px">S</div><svg style="display:block;padding-top:25px;width:20px;height:0"></svg><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 25)],
    ),
    // block image whose padding-top crosses the page end
    (
        r#"<div style="height:80px">S</div><img src="data:image/svg+xml,%3Csvg xmlns=%27http://www.w3.org/2000/svg%27 width=%2720%27 height=%2730%27%3E%3Crect width=%2720%27 height=%2730%27/%3E%3C/svg%3E" style="display:block;padding-top:25px;width:20px;height:5px"><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    // block canvas with padding-top
    (
        r#"<div style="height:80px">S</div><canvas style="display:block;padding-top:25px;width:20px;height:5px"></canvas><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
    // block input with padding-top
    (
        r#"<div style="height:80px">S</div><input style="display:block;padding:25px 0 0 0;border:0;height:5px;width:20px"><div style="height:10px">Z</div>"#,
        &[(0, 0), (1, 30)],
    ),
];
