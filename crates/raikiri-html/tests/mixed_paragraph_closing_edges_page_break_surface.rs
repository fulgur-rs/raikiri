//! The bottom padding and border of a paragraph that mixes text with blocks
//! and ends with a block child, around the end of a page. They follow the
//! block as they follow the last box of a block in plain block flow: when the
//! block's lines fit the page but the edges do not, its last lines, or the
//! whole block when `orphans` and `widows` leave no break between them, go
//! to the next page with the edges. Where each line falls is what Chromium
//! prints for the same documents on pages 100px wide and 200px tall, with
//! Ahem lines of 20px.

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
        "<!doctype html><style>@page{{size:100px 200px;margin:0}} body{{margin:0;font:20px/20px Ahem}}</style><body>{body}</body>"
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
fn the_edges_of_a_paragraph_follow_its_last_block() {
    // The edges below the block take its last lines, or the whole block, to the
    // next page; the text before the block stays.
    assert_rows(&[
        (
            "<div style='height:100px'>S</div><div style='padding-bottom:10px'>T<div>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'L', 120),
                (0, 'L', 140),
                (0, 'S', 0),
                (0, 'T', 100),
                (0, 'Z', 170),
            ],
        ),
        (
            "<div style='height:100px'>S</div><div style='padding-bottom:10px;border-bottom:5px solid'>T<br>T<div>L<br>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 100),
                (0, 'T', 120),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'Z', 75),
            ],
        ),
        (
            "<div style='height:100px'>S</div><div style='padding-bottom:10px'><div style='padding-bottom:10px'>T<div>L<br>L</div></div></div><div>Z</div>",
            &[
                (0, 'L', 120),
                (0, 'L', 140),
                (0, 'S', 0),
                (0, 'T', 100),
                (0, 'Z', 180),
            ],
        ),
        (
            "<div style='height:100px'>S</div><div style='padding-bottom:10px'>T<div>L</div></div><div>Z</div>",
            &[(0, 'L', 120), (0, 'S', 0), (0, 'T', 100), (0, 'Z', 150)],
        ),
        (
            "<div style='height:100px'>S</div><div style='padding-bottom:30px'>T<div>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'L', 120),
                (0, 'L', 140),
                (0, 'S', 0),
                (0, 'T', 100),
                (1, 'Z', 0),
            ],
        ),
        (
            "<div style='height:100px'>S</div><div style='padding-bottom:30px;border-bottom:5px solid'>T<br>T<div>L<br>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 100),
                (0, 'T', 120),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'Z', 95),
            ],
        ),
        (
            "<div style='height:100px'>S</div><div style='padding-bottom:30px'><div style='padding-bottom:10px'>T<div>L<br>L</div></div></div><div>Z</div>",
            &[
                (0, 'L', 120),
                (0, 'L', 140),
                (0, 'S', 0),
                (0, 'T', 100),
                (1, 'Z', 0),
            ],
        ),
        (
            "<div style='height:100px'>S</div><div style='padding-bottom:30px'>T<div>L</div></div><div>Z</div>",
            &[(0, 'L', 120), (0, 'S', 0), (0, 'T', 100), (0, 'Z', 170)],
        ),
        (
            "<div style='height:120px'>S</div><div style='padding-bottom:10px'>T<div>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'L', 140),
                (0, 'L', 160),
                (0, 'S', 0),
                (0, 'T', 120),
                (1, 'Z', 0),
            ],
        ),
        (
            "<div style='height:120px'>S</div><div style='padding-bottom:10px;border-bottom:5px solid'>T<br>T<div>L<br>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'L', 160),
                (0, 'L', 180),
                (0, 'S', 0),
                (0, 'T', 120),
                (0, 'T', 140),
                (1, 'L', 0),
                (1, 'Z', 35),
            ],
        ),
        (
            "<div style='height:120px'>S</div><div style='padding-bottom:10px'><div style='padding-bottom:10px'>T<div>L<br>L</div></div></div><div>Z</div>",
            &[
                (0, 'L', 140),
                (0, 'L', 160),
                (0, 'S', 0),
                (0, 'T', 120),
                (1, 'Z', 0),
            ],
        ),
        (
            "<div style='height:120px'>S</div><div style='padding-bottom:10px'>T<div>L</div></div><div>Z</div>",
            &[(0, 'L', 140), (0, 'S', 0), (0, 'T', 120), (0, 'Z', 170)],
        ),
        (
            "<div style='height:120px'>S</div><div style='padding-bottom:30px'>T<div>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 120),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 70),
            ],
        ),
        (
            "<div style='height:120px'>S</div><div style='padding-bottom:30px;border-bottom:5px solid'>T<br>T<div>L<br>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'L', 160),
                (0, 'L', 180),
                (0, 'S', 0),
                (0, 'T', 120),
                (0, 'T', 140),
                (1, 'L', 0),
                (1, 'Z', 55),
            ],
        ),
        (
            "<div style='height:120px'>S</div><div style='padding-bottom:30px'><div style='padding-bottom:10px'>T<div>L<br>L</div></div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 120),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 80),
            ],
        ),
        (
            "<div style='height:120px'>S</div><div style='padding-bottom:30px'>T<div>L</div></div><div>Z</div>",
            &[(0, 'L', 140), (0, 'S', 0), (0, 'T', 120), (1, 'Z', 0)],
        ),
        (
            "<div style='height:140px'>S</div><div style='padding-bottom:10px'>T<div>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 140),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 50),
            ],
        ),
        (
            "<div style='height:140px'>S</div><div style='padding-bottom:10px;border-bottom:5px solid'>T<br>T<div>L<br>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 140),
                (0, 'T', 160),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'Z', 75),
            ],
        ),
        (
            "<div style='height:140px'>S</div><div style='padding-bottom:10px'><div style='padding-bottom:10px'>T<div>L<br>L</div></div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 140),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 60),
            ],
        ),
        (
            "<div style='height:140px'>S</div><div style='padding-bottom:10px'>T<div>L</div></div><div>Z</div>",
            &[(0, 'L', 160), (0, 'S', 0), (0, 'T', 140), (1, 'Z', 0)],
        ),
        (
            "<div style='height:140px'>S</div><div style='padding-bottom:30px'>T<div>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 140),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 70),
            ],
        ),
        (
            "<div style='height:140px'>S</div><div style='padding-bottom:30px;border-bottom:5px solid'>T<br>T<div>L<br>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 140),
                (0, 'T', 160),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'Z', 95),
            ],
        ),
        (
            "<div style='height:140px'>S</div><div style='padding-bottom:30px'><div style='padding-bottom:10px'>T<div>L<br>L</div></div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 140),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 80),
            ],
        ),
        (
            "<div style='height:140px'>S</div><div style='padding-bottom:30px'>T<div>L</div></div><div>Z</div>",
            &[(0, 'S', 0), (0, 'T', 140), (1, 'L', 0), (1, 'Z', 50)],
        ),
        (
            "<div style='height:150px'>S</div><div style='padding-bottom:10px'>T<div>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 150),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 50),
            ],
        ),
        (
            "<div style='height:150px'>S</div><div style='padding-bottom:10px;border-bottom:5px solid'>T<br>T<div>L<br>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 150),
                (0, 'T', 170),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'Z', 75),
            ],
        ),
        (
            "<div style='height:150px'>S</div><div style='padding-bottom:10px'><div style='padding-bottom:10px'>T<div>L<br>L</div></div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 150),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 60),
            ],
        ),
        (
            "<div style='height:150px'>S</div><div style='padding-bottom:10px'>T<div>L</div></div><div>Z</div>",
            &[(0, 'L', 170), (0, 'S', 0), (0, 'T', 150), (1, 'Z', 0)],
        ),
        (
            "<div style='height:150px'>S</div><div style='padding-bottom:30px'>T<div>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 150),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 70),
            ],
        ),
        (
            "<div style='height:150px'>S</div><div style='padding-bottom:30px;border-bottom:5px solid'>T<br>T<div>L<br>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 150),
                (0, 'T', 170),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'Z', 95),
            ],
        ),
        (
            "<div style='height:150px'>S</div><div style='padding-bottom:30px'><div style='padding-bottom:10px'>T<div>L<br>L</div></div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 150),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 80),
            ],
        ),
        (
            "<div style='height:150px'>S</div><div style='padding-bottom:30px'>T<div>L</div></div><div>Z</div>",
            &[(0, 'S', 0), (0, 'T', 150), (1, 'L', 0), (1, 'Z', 50)],
        ),
        (
            "<div style='height:160px'>S</div><div style='padding-bottom:10px'>T<div>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 160),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 50),
            ],
        ),
        (
            "<div style='height:160px'>S</div><div style='padding-bottom:10px;border-bottom:5px solid'>T<br>T<div>L<br>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 160),
                (0, 'T', 180),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'Z', 75),
            ],
        ),
        (
            "<div style='height:160px'>S</div><div style='padding-bottom:10px'><div style='padding-bottom:10px'>T<div>L<br>L</div></div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 160),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 60),
            ],
        ),
        (
            "<div style='height:160px'>S</div><div style='padding-bottom:10px'>T<div>L</div></div><div>Z</div>",
            &[(0, 'S', 0), (0, 'T', 160), (1, 'L', 0), (1, 'Z', 30)],
        ),
        (
            "<div style='height:160px'>S</div><div style='padding-bottom:30px'>T<div>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 160),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 70),
            ],
        ),
        (
            "<div style='height:160px'>S</div><div style='padding-bottom:30px;border-bottom:5px solid'>T<br>T<div>L<br>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 160),
                (0, 'T', 180),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'Z', 95),
            ],
        ),
        (
            "<div style='height:160px'>S</div><div style='padding-bottom:30px'><div style='padding-bottom:10px'>T<div>L<br>L</div></div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 160),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 80),
            ],
        ),
        (
            "<div style='height:160px'>S</div><div style='padding-bottom:30px'>T<div>L</div></div><div>Z</div>",
            &[(0, 'S', 0), (0, 'T', 160), (1, 'L', 0), (1, 'Z', 50)],
        ),
    ]);
}

#[test]
fn the_edges_follow_a_block_after_text_and_other_blocks() {
    // Text after the block keeps the edges with its own line; a block before the
    // last one, or text lines before it, stay on the first page.
    assert_rows(&[
        (
            "<div style='height:120px'>S</div><div style='padding-bottom:10px'>T<div>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'L', 140),
                (0, 'L', 160),
                (0, 'S', 0),
                (0, 'T', 120),
                (1, 'J', 0),
                (1, 'Z', 30),
            ],
        ),
        (
            "<div style='height:120px'>S</div><div style='padding-bottom:10px'>T<div>M</div><div>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'M', 140),
                (0, 'S', 0),
                (0, 'T', 120),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 50),
            ],
        ),
        (
            "<div style='height:120px'>S</div><div style='padding-bottom:30px'>T<div>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'L', 140),
                (0, 'L', 160),
                (0, 'S', 0),
                (0, 'T', 120),
                (1, 'J', 0),
                (1, 'Z', 50),
            ],
        ),
        (
            "<div style='height:120px'>S</div><div style='padding-bottom:30px'>T<div>M</div><div>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'M', 140),
                (0, 'S', 0),
                (0, 'T', 120),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 70),
            ],
        ),
        (
            "<div style='height:140px'>S</div><div style='padding-bottom:10px'>T<div>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'L', 160),
                (0, 'L', 180),
                (0, 'S', 0),
                (0, 'T', 140),
                (1, 'J', 0),
                (1, 'Z', 30),
            ],
        ),
        (
            "<div style='height:140px'>S</div><div style='padding-bottom:10px'>T<div>M</div><div>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'M', 160),
                (0, 'S', 0),
                (0, 'T', 140),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 50),
            ],
        ),
        (
            "<div style='height:140px'>S</div><div style='padding-bottom:30px'>T<div>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'L', 160),
                (0, 'L', 180),
                (0, 'S', 0),
                (0, 'T', 140),
                (1, 'J', 0),
                (1, 'Z', 50),
            ],
        ),
        (
            "<div style='height:140px'>S</div><div style='padding-bottom:30px'>T<div>M</div><div>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'M', 160),
                (0, 'S', 0),
                (0, 'T', 140),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 70),
            ],
        ),
        (
            "<div style='height:150px'>S</div><div style='padding-bottom:10px'>T<div>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 150),
                (1, 'J', 40),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 70),
            ],
        ),
        (
            "<div style='height:150px'>S</div><div style='padding-bottom:10px'>T<div>M</div><div>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'M', 170),
                (0, 'S', 0),
                (0, 'T', 150),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 50),
            ],
        ),
        (
            "<div style='height:150px'>S</div><div style='padding-bottom:30px'>T<div>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 150),
                (1, 'J', 40),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 90),
            ],
        ),
        (
            "<div style='height:150px'>S</div><div style='padding-bottom:30px'>T<div>M</div><div>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'M', 170),
                (0, 'S', 0),
                (0, 'T', 150),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 70),
            ],
        ),
        (
            "<div style='height:160px'>S</div><div style='padding-bottom:10px'>T<div>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 160),
                (1, 'J', 40),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 70),
            ],
        ),
        (
            "<div style='height:160px'>S</div><div style='padding-bottom:10px'>T<div>M</div><div>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'M', 180),
                (0, 'S', 0),
                (0, 'T', 160),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 50),
            ],
        ),
        (
            "<div style='height:160px'>S</div><div style='padding-bottom:30px'>T<div>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 160),
                (1, 'J', 40),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 90),
            ],
        ),
        (
            "<div style='height:160px'>S</div><div style='padding-bottom:30px'>T<div>M</div><div>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'M', 180),
                (0, 'S', 0),
                (0, 'T', 160),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 70),
            ],
        ),
    ]);
}

#[test]
fn the_edges_of_nested_paragraphs_add_up() {
    // A paragraph inside the paragraph adds its own edges below the last block.
    assert_rows(&[
        (
            "<div style='height:120px'>S</div><div style='padding-bottom:10px'>T<div style='padding-bottom:10px'>U<div>L<br>L</div></div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 120),
                (0, 'U', 140),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 60),
            ],
        ),
        (
            "<div style='height:120px'>S</div><div style='padding-bottom:30px'>T<div style='padding-bottom:10px'>U<div>L<br>L</div></div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 120),
                (0, 'U', 140),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 80),
            ],
        ),
        (
            "<div style='height:140px'>S</div><div style='padding-bottom:10px'>T<div style='padding-bottom:10px'>U<div>L<br>L</div></div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 140),
                (0, 'U', 160),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 60),
            ],
        ),
        (
            "<div style='height:140px'>S</div><div style='padding-bottom:30px'>T<div style='padding-bottom:10px'>U<div>L<br>L</div></div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 140),
                (0, 'U', 160),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 80),
            ],
        ),
        (
            "<div style='height:150px'>S</div><div style='padding-bottom:10px'>T<div style='padding-bottom:10px'>U<div>L<br>L</div></div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 150),
                (0, 'U', 170),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 60),
            ],
        ),
        (
            "<div style='height:150px'>S</div><div style='padding-bottom:30px'>T<div style='padding-bottom:10px'>U<div>L<br>L</div></div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 150),
                (0, 'U', 170),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 80),
            ],
        ),
        (
            "<div style='height:160px'>S</div><div style='padding-bottom:10px'>T<div style='padding-bottom:10px'>U<div>L<br>L</div></div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 160),
                (0, 'U', 180),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 60),
            ],
        ),
        (
            "<div style='height:160px'>S</div><div style='padding-bottom:30px'>T<div style='padding-bottom:10px'>U<div>L<br>L</div></div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 160),
                (0, 'U', 180),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 80),
            ],
        ),
    ]);
}

#[test]
fn an_avoiding_block_moves_whole_with_the_edges() {
    // A block that avoids a break inside it moves whole with the edges.
    assert_rows(&[
        (
            "<div style='height:120px'>S</div><div style='padding-bottom:10px'>T<div style='break-inside:avoid'>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'L', 140),
                (0, 'L', 160),
                (0, 'S', 0),
                (0, 'T', 120),
                (1, 'Z', 0),
            ],
        ),
        (
            "<div style='height:120px'>S</div><div style='padding-bottom:30px'>T<div style='break-inside:avoid'>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 120),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 70),
            ],
        ),
        (
            "<div style='height:140px'>S</div><div style='padding-bottom:10px'>T<div style='break-inside:avoid'>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 140),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 50),
            ],
        ),
        (
            "<div style='height:140px'>S</div><div style='padding-bottom:30px'>T<div style='break-inside:avoid'>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 140),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 70),
            ],
        ),
        (
            "<div style='height:150px'>S</div><div style='padding-bottom:10px'>T<div style='break-inside:avoid'>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 150),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 50),
            ],
        ),
        (
            "<div style='height:150px'>S</div><div style='padding-bottom:30px'>T<div style='break-inside:avoid'>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 150),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 70),
            ],
        ),
        (
            "<div style='height:160px'>S</div><div style='padding-bottom:10px'>T<div style='break-inside:avoid'>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 160),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 50),
            ],
        ),
        (
            "<div style='height:160px'>S</div><div style='padding-bottom:30px'>T<div style='break-inside:avoid'>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 160),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 70),
            ],
        ),
    ]);
}

#[test]
fn a_float_after_the_last_block_keeps_it_in_place() {
    // A float after the last block is what the edges follow: the block stays.
    assert_rows(&[
        (
            "<div style='height:120px'>S</div><div style='padding-bottom:10px'>T<div>L<br>L</div><div style='float:left'>F</div></div><div>Z</div>",
            &[
                (0, 'F', 180),
                (0, 'L', 140),
                (0, 'L', 160),
                (0, 'S', 0),
                (0, 'T', 120),
                (1, 'Z', 0),
            ],
        ),
        (
            "<div style='height:140px'>S</div><div style='padding-bottom:10px'>T<div>L<br>L</div><div style='float:left'>F</div></div><div>Z</div>",
            &[
                (0, 'L', 160),
                (0, 'L', 180),
                (0, 'S', 0),
                (0, 'T', 140),
                (1, 'F', 0),
                (1, 'Z', 10),
            ],
        ),
        (
            "<div style='height:140px'>S</div><div style='padding-bottom:30px'>T<div>L<br>L</div><div style='float:left'>F</div></div><div>Z</div>",
            &[
                (0, 'L', 160),
                (0, 'L', 180),
                (0, 'S', 0),
                (0, 'T', 140),
                (1, 'F', 0),
                (1, 'Z', 30),
            ],
        ),
        (
            "<div style='height:150px'>S</div><div style='padding-bottom:10px'>T<div>L<br>L</div><div style='float:left'>F</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 150),
                (1, 'F', 40),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 50),
            ],
        ),
        (
            "<div style='height:150px'>S</div><div style='padding-bottom:30px'>T<div>L<br>L</div><div style='float:left'>F</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 150),
                (1, 'F', 40),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 70),
            ],
        ),
        (
            "<div style='height:160px'>S</div><div style='padding-bottom:10px'>T<div>L<br>L</div><div style='float:left'>F</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 160),
                (1, 'F', 40),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 50),
            ],
        ),
        (
            "<div style='height:160px'>S</div><div style='padding-bottom:30px'>T<div>L<br>L</div><div style='float:left'>F</div></div><div>Z</div>",
            &[
                (0, 'S', 0),
                (0, 'T', 160),
                (1, 'F', 40),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 70),
            ],
        ),
    ]);
}
