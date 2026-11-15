//! The lines that follow a block child of a paragraph that mixes text with
//! blocks, around the end of a page. They break apart from the block before
//! them, as a paragraph of their own: when too few of them fit the page, they
//! move to the next page and the block stays where it is, rather than the
//! whole paragraph moving. Where each line falls is what Chromium prints for
//! the same documents on pages 100px square, with Ahem glyphs of 20px on lines
//! of 30px.

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

/// The page, glyph and glyph top of every line of text in `body`.
fn glyph_tops(body: &str) -> Vec<(u32, char, i32)> {
    let result = lay_out(body);
    let mut tops = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            let Some(glyph) = run.text.trim().chars().next() else {
                continue;
            };
            tops.push((index, glyph, (run.origin.1 - run.ascent).round() as i32));
        }
    }
    tops.sort_unstable();
    tops.dedup();
    tops
}

/// A document and the page, glyph and top of each of its lines.
type Row = (&'static str, &'static [(u32, char, i32)]);

fn assert_rows(rows: &[Row]) {
    for &(body, expected) in rows {
        assert_eq!(glyph_tops(body), expected, "{body}");
    }
}

#[test]
fn the_lines_after_a_leading_block_break_apart_from_it() {
    // The lines after a block that starts the paragraph break on their own: when
    // too few of them fit the page, they go to the next page and the block
    // stays where it is.
    assert_rows(&[
        (
            "<div style='width:20px'><div style='height:0px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 5), (0, 'X', 35), (0, 'Z', 65)],
        ),
        (
            "<div style='width:20px'><div style='height:10px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 15), (0, 'X', 45), (0, 'Z', 75)],
        ),
        (
            "<div style='width:20px'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 25), (0, 'X', 55), (1, 'Z', 5)],
        ),
        (
            "<div style='width:20px'><div style='height:40px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 45), (0, 'X', 75), (1, 'Z', 5)],
        ),
        (
            "<div style='height:10px'></div><div style='width:20px'><div style='height:0px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 15), (0, 'X', 45), (0, 'Z', 75)],
        ),
        (
            "<div style='height:10px'></div><div style='width:20px'><div style='height:10px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 25), (0, 'X', 55), (1, 'Z', 5)],
        ),
        (
            "<div style='height:10px'></div><div style='width:20px'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (1, 'Z', 5)],
        ),
        (
            "<div style='height:10px'></div><div style='width:20px'><div style='height:40px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:20px'></div><div style='width:20px'><div style='height:0px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 25), (0, 'X', 55), (1, 'Z', 5)],
        ),
        (
            "<div style='height:20px'></div><div style='width:20px'><div style='height:10px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (1, 'Z', 5)],
        ),
        (
            "<div style='height:20px'></div><div style='width:20px'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 45), (0, 'X', 75), (1, 'Z', 5)],
        ),
        (
            "<div style='height:20px'></div><div style='width:20px'><div style='height:40px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px'><div style='height:0px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (1, 'Z', 5)],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px'><div style='height:10px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 45), (0, 'X', 75), (1, 'Z', 5)],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px'><div style='height:40px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:40px'></div><div style='width:20px'><div style='height:0px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 45), (0, 'X', 75), (1, 'Z', 5)],
        ),
        (
            "<div style='height:40px'></div><div style='width:20px'><div style='height:10px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:40px'></div><div style='width:20px'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:40px'></div><div style='width:20px'><div style='height:40px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:45px'></div><div style='width:20px'><div style='height:0px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:45px'></div><div style='width:20px'><div style='height:10px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:45px'></div><div style='width:20px'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:45px'></div><div style='width:20px'><div style='height:40px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:50px'></div><div style='width:20px'><div style='height:0px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:50px'></div><div style='width:20px'><div style='height:10px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:50px'></div><div style='width:20px'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:50px'></div><div style='width:20px'><div style='height:40px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:60px'></div><div style='width:20px'><div style='height:0px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:60px'></div><div style='width:20px'><div style='height:10px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:60px'></div><div style='width:20px'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:60px'></div><div style='width:20px'><div style='height:40px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:70px'></div><div style='width:20px'><div style='height:0px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:70px'></div><div style='width:20px'><div style='height:10px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:70px'></div><div style='width:20px'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:70px'></div><div style='width:20px'><div style='height:40px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 15), (1, 'X', 45), (1, 'Z', 75)],
        ),
    ]);
}

#[test]
fn the_run_after_a_block_keeps_its_orphans_and_widows() {
    // The run after the block breaks by the orphans and widows of the paragraph,
    // as a paragraph of its own.
    assert_rows(&[
        (
            "<div style='width:20px'><div style='height:20px'></div>X X X</div><div>Z</div>",
            &[(0, 'X', 25), (0, 'X', 55), (1, 'X', 5), (1, 'Z', 35)],
        ),
        (
            "<div style='width:20px'><div style='height:20px'></div>X X X X</div>",
            &[(0, 'X', 25), (0, 'X', 55), (1, 'X', 5), (1, 'X', 35)],
        ),
        (
            "<div style='width:20px;orphans:1;widows:1'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 25), (0, 'X', 55), (1, 'Z', 5)],
        ),
        (
            "<div style='height:10px'></div><div style='width:20px'><div style='height:20px'></div>X X X</div><div>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (1, 'X', 5), (1, 'Z', 35)],
        ),
        (
            "<div style='height:10px'></div><div style='width:20px'><div style='height:20px'></div>X X X X</div>",
            &[(0, 'X', 35), (0, 'X', 65), (1, 'X', 5), (1, 'X', 35)],
        ),
        (
            "<div style='height:10px'></div><div style='width:20px;orphans:1;widows:1'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (1, 'Z', 5)],
        ),
        (
            "<div style='height:20px'></div><div style='width:20px'><div style='height:20px'></div>X X X</div><div>Z</div>",
            &[(0, 'X', 45), (0, 'X', 75), (1, 'X', 5), (1, 'Z', 35)],
        ),
        (
            "<div style='height:20px'></div><div style='width:20px'><div style='height:20px'></div>X X X X</div>",
            &[(0, 'X', 45), (0, 'X', 75), (1, 'X', 5), (1, 'X', 35)],
        ),
        (
            "<div style='height:20px'></div><div style='width:20px;orphans:1;widows:1'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 45), (0, 'X', 75), (1, 'Z', 5)],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px'><div style='height:20px'></div>X X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'X', 65), (2, 'Z', 5)],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px'><div style='height:20px'></div>X X X X</div>",
            &[(1, 'X', 5), (1, 'X', 35), (2, 'X', 5), (2, 'X', 35)],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px;orphans:1;widows:1'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 55), (1, 'X', 5), (1, 'Z', 35)],
        ),
        (
            "<div style='height:40px'></div><div style='width:20px'><div style='height:20px'></div>X X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'X', 65), (2, 'Z', 5)],
        ),
        (
            "<div style='height:40px'></div><div style='width:20px'><div style='height:20px'></div>X X X X</div>",
            &[(1, 'X', 5), (1, 'X', 35), (2, 'X', 5), (2, 'X', 35)],
        ),
        (
            "<div style='height:40px'></div><div style='width:20px;orphans:1;widows:1'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 65), (1, 'X', 5), (1, 'Z', 35)],
        ),
        (
            "<div style='height:45px'></div><div style='width:20px'><div style='height:20px'></div>X X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'X', 65), (2, 'Z', 5)],
        ),
        (
            "<div style='height:45px'></div><div style='width:20px'><div style='height:20px'></div>X X X X</div>",
            &[(1, 'X', 5), (1, 'X', 35), (2, 'X', 5), (2, 'X', 35)],
        ),
        (
            "<div style='height:45px'></div><div style='width:20px;orphans:1;widows:1'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 70), (1, 'X', 5), (1, 'Z', 35)],
        ),
        (
            "<div style='height:50px'></div><div style='width:20px'><div style='height:20px'></div>X X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'X', 65), (2, 'Z', 5)],
        ),
        (
            "<div style='height:50px'></div><div style='width:20px'><div style='height:20px'></div>X X X X</div>",
            &[(1, 'X', 5), (1, 'X', 35), (2, 'X', 5), (2, 'X', 35)],
        ),
        (
            "<div style='height:50px'></div><div style='width:20px;orphans:1;widows:1'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 75), (1, 'X', 5), (1, 'Z', 35)],
        ),
        (
            "<div style='height:60px'></div><div style='width:20px'><div style='height:20px'></div>X X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'X', 65), (2, 'Z', 5)],
        ),
        (
            "<div style='height:60px'></div><div style='width:20px'><div style='height:20px'></div>X X X X</div>",
            &[(1, 'X', 5), (1, 'X', 35), (2, 'X', 5), (2, 'X', 35)],
        ),
        (
            "<div style='height:60px'></div><div style='width:20px;orphans:1;widows:1'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:70px'></div><div style='width:20px'><div style='height:20px'></div>X X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'X', 65), (2, 'Z', 5)],
        ),
        (
            "<div style='height:70px'></div><div style='width:20px'><div style='height:20px'></div>X X X X</div>",
            &[(1, 'X', 5), (1, 'X', 35), (2, 'X', 5), (2, 'X', 35)],
        ),
        (
            "<div style='height:70px'></div><div style='width:20px;orphans:1;widows:1'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
    ]);
}

#[test]
fn the_run_after_a_block_keeps_the_edges_and_nesting() {
    // The run keeps the paragraph's bottom padding, and breaks the same way after
    // a block with text, after text and a block, and inside a nested paragraph.
    assert_rows(&[
        (
            "<div style='width:20px;padding-bottom:10px'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 25), (0, 'X', 55), (1, 'Z', 5)],
        ),
        (
            "<div style='width:20px'>W<div style='height:10px'></div>X X</div><div>Z</div>",
            &[(0, 'W', 5), (0, 'X', 45), (0, 'X', 75), (1, 'Z', 5)],
        ),
        (
            "<div style='width:20px'><div style='height:20px'>Q</div>X X</div><div>Z</div>",
            &[(0, 'Q', 5), (0, 'X', 25), (0, 'X', 55), (1, 'Z', 5)],
        ),
        (
            "<div style='width:20px'><div><div style='height:20px'></div>X X</div></div><div>Z</div>",
            &[(0, 'X', 25), (0, 'X', 55), (1, 'Z', 5)],
        ),
        (
            "<div style='height:10px'></div><div style='width:20px;padding-bottom:10px'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (1, 'Z', 5)],
        ),
        (
            "<div style='height:10px'></div><div style='width:20px'>W<div style='height:10px'></div>X X</div><div>Z</div>",
            &[(0, 'W', 15), (1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:10px'></div><div style='width:20px'><div style='height:20px'>Q</div>X X</div><div>Z</div>",
            &[(0, 'Q', 15), (0, 'X', 35), (0, 'X', 65), (1, 'Z', 5)],
        ),
        (
            "<div style='height:10px'></div><div style='width:20px'><div><div style='height:20px'></div>X X</div></div><div>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (1, 'Z', 5)],
        ),
        (
            "<div style='height:20px'></div><div style='width:20px;padding-bottom:10px'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 75)],
        ),
        (
            "<div style='height:20px'></div><div style='width:20px'>W<div style='height:10px'></div>X X</div><div>Z</div>",
            &[(0, 'W', 25), (1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:20px'></div><div style='width:20px'><div style='height:20px'>Q</div>X X</div><div>Z</div>",
            &[(0, 'Q', 25), (0, 'X', 45), (0, 'X', 75), (1, 'Z', 5)],
        ),
        (
            "<div style='height:20px'></div><div style='width:20px'><div><div style='height:20px'></div>X X</div></div><div>Z</div>",
            &[(0, 'X', 45), (0, 'X', 75), (1, 'Z', 5)],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px;padding-bottom:10px'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 75)],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px'>W<div style='height:10px'></div>X X</div><div>Z</div>",
            &[(0, 'W', 35), (1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px'><div style='height:20px'>Q</div>X X</div><div>Z</div>",
            &[(0, 'Q', 35), (1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px'><div><div style='height:20px'></div>X X</div></div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:40px'></div><div style='width:20px;padding-bottom:10px'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 75)],
        ),
        (
            "<div style='height:40px'></div><div style='width:20px'>W<div style='height:10px'></div>X X</div><div>Z</div>",
            &[(0, 'W', 45), (1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:40px'></div><div style='width:20px'><div style='height:20px'>Q</div>X X</div><div>Z</div>",
            &[(0, 'Q', 45), (1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:40px'></div><div style='width:20px'><div><div style='height:20px'></div>X X</div></div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:45px'></div><div style='width:20px;padding-bottom:10px'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 75)],
        ),
        (
            "<div style='height:45px'></div><div style='width:20px'>W<div style='height:10px'></div>X X</div><div>Z</div>",
            &[(0, 'W', 50), (1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:45px'></div><div style='width:20px'><div style='height:20px'>Q</div>X X</div><div>Z</div>",
            &[(0, 'Q', 50), (1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:45px'></div><div style='width:20px'><div><div style='height:20px'></div>X X</div></div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:50px'></div><div style='width:20px;padding-bottom:10px'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 75)],
        ),
        (
            "<div style='height:50px'></div><div style='width:20px'>W<div style='height:10px'></div>X X</div><div>Z</div>",
            &[(0, 'W', 55), (1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:50px'></div><div style='width:20px'><div style='height:20px'>Q</div>X X</div><div>Z</div>",
            &[(0, 'Q', 55), (1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:50px'></div><div style='width:20px'><div><div style='height:20px'></div>X X</div></div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:60px'></div><div style='width:20px;padding-bottom:10px'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 75)],
        ),
        (
            "<div style='height:60px'></div><div style='width:20px'>W<div style='height:10px'></div>X X</div><div>Z</div>",
            &[(0, 'W', 65), (1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:60px'></div><div style='width:20px'><div style='height:20px'>Q</div>X X</div><div>Z</div>",
            &[(0, 'Q', 65), (1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:60px'></div><div style='width:20px'><div><div style='height:20px'></div>X X</div></div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:70px'></div><div style='width:20px;padding-bottom:10px'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 75)],
        ),
        (
            "<div style='height:70px'></div><div style='width:20px'>W<div style='height:10px'></div>X X</div><div>Z</div>",
            &[(0, 'W', 75), (1, 'X', 15), (1, 'X', 45), (1, 'Z', 75)],
        ),
        (
            "<div style='height:70px'></div><div style='width:20px'><div style='height:20px'>Q</div>X X</div><div>Z</div>",
            &[(0, 'Q', 75), (1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:70px'></div><div style='width:20px'><div><div style='height:20px'></div>X X</div></div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
    ]);
}

#[test]
fn a_run_between_two_blocks_moves_whole_for_its_orphans() {
    // A run between two block children moves whole to the next page when a break
    // inside it would leave too few orphans; the block after it follows.
    assert_rows(&[
        (
            "<div style='width:20px'><div style='height:20px'></div>X X<div style='height:10px'></div>Y</div><div>Z</div>",
            &[(0, 'X', 25), (0, 'X', 55), (1, 'Y', 5), (1, 'Z', 35)],
        ),
        (
            "<div style='height:10px'></div><div style='width:20px'><div style='height:20px'></div>X X<div style='height:10px'></div>Y</div><div>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (1, 'Y', 5), (1, 'Z', 35)],
        ),
        (
            "<div style='height:20px'></div><div style='width:20px'><div style='height:20px'></div>X X<div style='height:10px'></div>Y</div><div>Z</div>",
            &[(0, 'X', 45), (0, 'X', 75), (1, 'Y', 15), (1, 'Z', 45)],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px'><div style='height:20px'></div>X X<div style='height:10px'></div>Y</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Y', 75), (2, 'Z', 5)],
        ),
        (
            "<div style='height:40px'></div><div style='width:20px'><div style='height:20px'></div>X X<div style='height:10px'></div>Y</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Y', 75), (2, 'Z', 5)],
        ),
        (
            "<div style='height:45px'></div><div style='width:20px'><div style='height:20px'></div>X X<div style='height:10px'></div>Y</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Y', 75), (2, 'Z', 5)],
        ),
        (
            "<div style='height:50px'></div><div style='width:20px'><div style='height:20px'></div>X X<div style='height:10px'></div>Y</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Y', 75), (2, 'Z', 5)],
        ),
        (
            "<div style='height:60px'></div><div style='width:20px'><div style='height:20px'></div>X X<div style='height:10px'></div>Y</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Y', 75), (2, 'Z', 5)],
        ),
        (
            "<div style='height:70px'></div><div style='width:20px'><div style='height:20px'></div>X X<div style='height:10px'></div>Y</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Y', 75), (2, 'Z', 5)],
        ),
    ]);
}

#[test]
fn a_run_inside_a_float_breaks_as_one_in_the_flow() {
    // A paragraph inside a float breaks its run after a block as one in the flow
    // does.
    assert_rows(&[
        (
            "<div><div style='float:left;width:20px'><div style='height:20px'></div>X X</div></div><div style='clear:both'>Z</div>",
            &[(0, 'X', 25), (0, 'X', 55), (1, 'Z', 5)],
        ),
        (
            "<div style='height:10px'></div><div><div style='float:left;width:20px'><div style='height:20px'></div>X X</div></div><div style='clear:both'>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (1, 'Z', 5)],
        ),
        (
            "<div style='height:20px'></div><div><div style='float:left;width:20px'><div style='height:20px'></div>X X</div></div><div style='clear:both'>Z</div>",
            &[(0, 'X', 45), (0, 'X', 75), (1, 'Z', 5)],
        ),
        (
            "<div style='height:30px'></div><div><div style='float:left;width:20px'><div style='height:20px'></div>X X</div></div><div style='clear:both'>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:40px'></div><div><div style='float:left;width:20px'><div style='height:20px'></div>X X</div></div><div style='clear:both'>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:45px'></div><div><div style='float:left;width:20px'><div style='height:20px'></div>X X</div></div><div style='clear:both'>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:50px'></div><div><div style='float:left;width:20px'><div style='height:20px'></div>X X</div></div><div style='clear:both'>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:60px'></div><div><div style='float:left;width:20px'><div style='height:20px'></div>X X</div></div><div style='clear:both'>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:70px'></div><div><div style='float:left;width:20px'><div style='height:20px'></div>X X</div></div><div style='clear:both'>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
    ]);
}

#[test]
fn a_paragraph_that_avoids_a_break_still_moves_whole() {
    // A paragraph that avoids a break inside it moves whole, block and run
    // together.
    assert_rows(&[
        (
            "<div style='width:20px;break-inside:avoid'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 25), (0, 'X', 55), (1, 'Z', 5)],
        ),
        (
            "<div style='height:10px'></div><div style='width:20px;break-inside:avoid'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 35), (0, 'X', 65), (1, 'Z', 5)],
        ),
        (
            "<div style='height:20px'></div><div style='width:20px;break-inside:avoid'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(0, 'X', 45), (0, 'X', 75), (1, 'Z', 5)],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px;break-inside:avoid'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 25), (1, 'X', 55), (2, 'Z', 5)],
        ),
        (
            "<div style='height:40px'></div><div style='width:20px;break-inside:avoid'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 25), (1, 'X', 55), (2, 'Z', 5)],
        ),
        (
            "<div style='height:45px'></div><div style='width:20px;break-inside:avoid'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 25), (1, 'X', 55), (2, 'Z', 5)],
        ),
        (
            "<div style='height:50px'></div><div style='width:20px;break-inside:avoid'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 25), (1, 'X', 55), (2, 'Z', 5)],
        ),
        (
            "<div style='height:60px'></div><div style='width:20px;break-inside:avoid'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 25), (1, 'X', 55), (2, 'Z', 5)],
        ),
        (
            "<div style='height:70px'></div><div style='width:20px;break-inside:avoid'><div style='height:20px'></div>X X</div><div>Z</div>",
            &[(1, 'X', 25), (1, 'X', 55), (2, 'Z', 5)],
        ),
    ]);
}

#[test]
fn the_run_after_an_empty_block_breaks_below_the_top_edges() {
    // Top padding or border that ends the page stays there with the empty
    // block after it, and the run starts the next page; a relatively offset
    // run moves as one in the flow does.
    assert_rows(&[
        (
            "<div style='height:90px'></div><div style='padding-top:10px;width:20px'><div style='height:0'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:90px'></div><div style='border-top:10px solid;width:20px'><div style='height:0'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:90px'></div><div style='padding-top:10px;width:20px'><div style='height:0'></div>X X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'X', 65), (2, 'Z', 5)],
        ),
        (
            "<div style='height:80px'></div><div style='padding-top:20px;width:20px'><div style='height:0'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:80px'></div><div style='border-top:20px solid;width:20px'><div style='height:0'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:80px'></div><div style='padding-top:20px;width:20px'><div style='height:0'></div>X X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'X', 65), (2, 'Z', 5)],
        ),
        (
            "<div style='height:85px'></div><div style='padding-top:15px;width:20px'><div style='height:0'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:85px'></div><div style='border-top:15px solid;width:20px'><div style='height:0'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:85px'></div><div style='padding-top:15px;width:20px'><div style='height:0'></div>X X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'X', 65), (2, 'Z', 5)],
        ),
        (
            "<div style='height:70px'></div><div style='padding-top:30px;width:20px'><div style='height:0'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:70px'></div><div style='border-top:30px solid;width:20px'><div style='height:0'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:70px'></div><div style='padding-top:30px;width:20px'><div style='height:0'></div>X X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'X', 65), (2, 'Z', 5)],
        ),
        (
            "<div style='height:60px'></div><div style='padding-top:40px;width:20px'><div style='height:0'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:60px'></div><div style='border-top:40px solid;width:20px'><div style='height:0'></div>X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'Z', 65)],
        ),
        (
            "<div style='height:60px'></div><div style='padding-top:40px;width:20px'><div style='height:0'></div>X X X</div><div>Z</div>",
            &[(1, 'X', 5), (1, 'X', 35), (1, 'X', 65), (2, 'Z', 5)],
        ),
        (
            "<div style='height:30px'></div><div style='width:20px'><div style='height:20px'></div><span style='position:relative;top:5px'>X X</span></div><div>Z</div>",
            &[(1, 'X', 10), (1, 'X', 40), (1, 'Z', 65)],
        ),
        (
            "<div style='height:40px'></div><div style='width:20px'><div style='height:20px'></div><span style='position:relative;top:5px'>X X</span></div><div>Z</div>",
            &[(1, 'X', 10), (1, 'X', 40), (1, 'Z', 65)],
        ),
        (
            "<div style='height:50px'></div><div style='width:20px'><div style='height:20px'></div><span style='position:relative;top:5px'>X X</span></div><div>Z</div>",
            &[(1, 'X', 10), (1, 'X', 40), (1, 'Z', 65)],
        ),
    ]);
}
