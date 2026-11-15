//! The lines of a block child of a paragraph that mixes text with blocks,
//! around the end of a page. They break across pages as the lines of a block
//! in plain block flow do: the line that crosses the page end starts the next
//! page, and the block moves whole when the page would keep fewer lines than
//! `orphans` asks. The paragraph's lines before the block break apart from
//! the block's, and its content after the block follows the moved lines.
//! Where each line falls is what Chromium prints for the same documents on
//! pages 100px wide and 200px tall, with Ahem lines of 20px.

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
fn the_lines_of_a_block_in_a_paragraph_start_the_next_page() {
    // A line of the block that crosses the page end starts the next page, and
    // the block moves whole when fewer than two of its lines fit the page.
    assert_rows(&[
        (
            "<div style='height:110px'>A</div><div>I<div>L<br>L<br>L<br>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 110),
                (0, 'L', 130),
                (0, 'L', 150),
                (0, 'L', 170),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 40),
            ],
        ),
        (
            "<div style='height:130px'>A</div><div>I<div>L<br>L<br>L<br>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 130),
                (0, 'L', 150),
                (0, 'L', 170),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'Z', 60),
            ],
        ),
        (
            "<div style='height:150px'>A</div><div>I<div>L<br>L<br>L<br>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 150),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'Z', 100),
            ],
        ),
        (
            "<div style='height:170px'>A</div><div>I<div>L<br>L<br>L<br>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 170),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'Z', 100),
            ],
        ),
        (
            "<div style='height:90px'>A</div><div>I<br>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 90),
                (0, 'I', 110),
                (0, 'L', 130),
                (0, 'L', 150),
                (0, 'L', 170),
                (1, 'J', 40),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 60),
            ],
        ),
        (
            "<div style='height:110px'>A</div><div>I<br>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 110),
                (0, 'I', 130),
                (0, 'L', 150),
                (0, 'L', 170),
                (1, 'J', 60),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'Z', 80),
            ],
        ),
        (
            "<div style='height:130px'>A</div><div>I<br>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 130),
                (0, 'I', 150),
                (1, 'J', 100),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'Z', 120),
            ],
        ),
        (
            "<div style='height:150px'>A</div><div>I<br>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 150),
                (0, 'I', 170),
                (1, 'J', 100),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'Z', 120),
            ],
        ),
    ]);
}

#[test]
fn the_content_after_the_block_follows_its_moved_lines() {
    // The text and blocks after the block follow its moved lines; the line
    // before it stays on the first page.
    assert_rows(&[
        (
            "<div style='height:110px'>A</div><div>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 110),
                (0, 'L', 130),
                (0, 'L', 150),
                (0, 'L', 170),
                (1, 'J', 40),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 60),
            ],
        ),
        (
            "<div style='height:110px'>A</div><div>I<div>L<br>L<br>L<br>L<br>L</div><div>M<br>M</div>J</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 110),
                (0, 'L', 130),
                (0, 'L', 150),
                (0, 'L', 170),
                (1, 'J', 80),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'M', 40),
                (1, 'M', 60),
            ],
        ),
        (
            "<div style='height:130px'>A</div><div>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 130),
                (0, 'L', 150),
                (0, 'L', 170),
                (1, 'J', 60),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'Z', 80),
            ],
        ),
        (
            "<div style='height:130px'>A</div><div>I<div>L<br>L<br>L<br>L<br>L</div><div>M<br>M</div>J</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 130),
                (0, 'L', 150),
                (0, 'L', 170),
                (1, 'J', 100),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'M', 60),
                (1, 'M', 80),
            ],
        ),
        (
            "<div style='height:150px'>A</div><div>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 150),
                (1, 'J', 100),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'Z', 120),
            ],
        ),
        (
            "<div style='height:150px'>A</div><div>I<div>L<br>L<br>L<br>L<br>L</div><div>M<br>M</div>J</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 150),
                (1, 'J', 140),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'M', 100),
                (1, 'M', 120),
            ],
        ),
        (
            "<div style='height:170px'>A</div><div>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 170),
                (1, 'J', 100),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'Z', 120),
            ],
        ),
        (
            "<div style='height:170px'>A</div><div>I<div>L<br>L<br>L<br>L<br>L</div><div>M<br>M</div>J</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 170),
                (1, 'J', 140),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'M', 100),
                (1, 'M', 120),
            ],
        ),
    ]);
}

#[test]
fn the_edges_below_the_lines_follow_them() {
    // The padding and border of the paragraph, and of the block, stay below
    // the moved lines.
    assert_rows(&[
        (
            "<div style='height:110px'>A</div><div style='padding-bottom:10px;border-bottom:5px solid'>I<div>L<br>L<br>L<br>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 110),
                (0, 'L', 130),
                (0, 'L', 150),
                (0, 'L', 170),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 55),
            ],
        ),
        (
            "<div style='height:110px'>A</div><div>I<div style='padding-bottom:10px'>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 110),
                (0, 'L', 130),
                (0, 'L', 150),
                (0, 'L', 170),
                (1, 'J', 50),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 70),
            ],
        ),
        (
            "<div style='height:130px'>A</div><div style='padding-bottom:10px;border-bottom:5px solid'>I<div>L<br>L<br>L<br>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 130),
                (0, 'L', 150),
                (0, 'L', 170),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'Z', 75),
            ],
        ),
        (
            "<div style='height:130px'>A</div><div>I<div style='padding-bottom:10px'>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 130),
                (0, 'L', 150),
                (0, 'L', 170),
                (1, 'J', 70),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'Z', 90),
            ],
        ),
        (
            "<div style='height:150px'>A</div><div style='padding-bottom:10px;border-bottom:5px solid'>I<div>L<br>L<br>L<br>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 150),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'Z', 115),
            ],
        ),
        (
            "<div style='height:150px'>A</div><div>I<div style='padding-bottom:10px'>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 150),
                (1, 'J', 110),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'Z', 130),
            ],
        ),
        (
            "<div style='height:170px'>A</div><div style='padding-bottom:10px;border-bottom:5px solid'>I<div>L<br>L<br>L<br>L<br>L</div></div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 170),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'Z', 115),
            ],
        ),
        (
            "<div style='height:170px'>A</div><div>I<div style='padding-bottom:10px'>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 170),
                (1, 'J', 110),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'Z', 130),
            ],
        ),
        (
            "<div style='height:90px'>A</div><div style='padding-bottom:10px;break-inside:avoid'>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (1, 'I', 0),
                (1, 'J', 120),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'L', 100),
                (1, 'Z', 150),
            ],
        ),
        (
            "<div style='height:110px'>A</div><div style='padding-bottom:10px;break-inside:avoid'>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (1, 'I', 0),
                (1, 'J', 120),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'L', 100),
                (1, 'Z', 150),
            ],
        ),
        (
            "<div style='height:130px'>A</div><div style='padding-bottom:10px;break-inside:avoid'>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (1, 'I', 0),
                (1, 'J', 120),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'L', 100),
                (1, 'Z', 150),
            ],
        ),
        (
            "<div style='height:150px'>A</div><div style='padding-bottom:10px;break-inside:avoid'>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (1, 'I', 0),
                (1, 'J', 120),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'L', 100),
                (1, 'Z', 150),
            ],
        ),
    ]);
}

#[test]
fn a_block_inside_another_box_of_the_paragraph_breaks_its_lines_too() {
    // A block nested in a block child, or in an inline element, of the
    // paragraph breaks the same way.
    assert_rows(&[
        (
            "<div style='height:110px'>A</div><div>I<div><div>L<br>L<br>L<br>L<br>L</div></div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 110),
                (0, 'L', 130),
                (0, 'L', 150),
                (0, 'L', 170),
                (1, 'J', 40),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 60),
            ],
        ),
        (
            "<div style='height:110px'>A</div><div>I<span><div>L<br>L<br>L<br>L<br>L</div></span>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 110),
                (0, 'L', 130),
                (0, 'L', 150),
                (0, 'L', 170),
                (1, 'J', 40),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 60),
            ],
        ),
        (
            "<div style='height:130px'>A</div><div>I<div><div>L<br>L<br>L<br>L<br>L</div></div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 130),
                (0, 'L', 150),
                (0, 'L', 170),
                (1, 'J', 60),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'Z', 80),
            ],
        ),
        (
            "<div style='height:130px'>A</div><div>I<span><div>L<br>L<br>L<br>L<br>L</div></span>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 130),
                (0, 'L', 150),
                (0, 'L', 170),
                (1, 'J', 60),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'Z', 80),
            ],
        ),
        (
            "<div style='height:150px'>A</div><div>I<div><div>L<br>L<br>L<br>L<br>L</div></div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 150),
                (1, 'J', 100),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'Z', 120),
            ],
        ),
        (
            "<div style='height:170px'>A</div><div>I<div><div>L<br>L<br>L<br>L<br>L</div></div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 170),
                (1, 'J', 100),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'Z', 120),
            ],
        ),
        (
            "<div style='height:90px'>A</div><div>I<span style='position:relative;top:10px'><div>L<br>L<br>L<br>L<br>L</div></span>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 90),
                (0, 'L', 120),
                (0, 'L', 140),
                (0, 'L', 160),
                (1, 'J', 40),
                (1, 'L', 10),
                (1, 'L', 30),
                (1, 'Z', 60),
            ],
        ),
        (
            "<div style='height:110px'>A</div><div>I<span style='position:relative;top:10px'><div>L<br>L<br>L<br>L<br>L</div></span>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 110),
                (0, 'L', 140),
                (0, 'L', 160),
                (0, 'L', 180),
                (1, 'J', 40),
                (1, 'L', 10),
                (1, 'L', 30),
                (1, 'Z', 60),
            ],
        ),
        (
            "<div style='height:130px'>A</div><div>I<span style='position:relative;top:10px'><div>L<br>L<br>L<br>L<br>L</div></span>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 130),
                (0, 'L', 160),
                (0, 'L', 180),
                (1, 'J', 60),
                (1, 'L', 10),
                (1, 'L', 30),
                (1, 'L', 50),
                (1, 'Z', 80),
            ],
        ),
    ]);
}

#[test]
fn a_paragraph_with_a_set_height_lets_the_lines_overflow_it() {
    // A paragraph with a set height keeps it: the moved lines and the text after
    // them overflow it, and the box after the paragraph stays below its end.
    assert_rows(&[
        (
            "<div style='height:110px'>A</div><div style='height:150px'>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 110),
                (0, 'L', 130),
                (0, 'L', 150),
                (0, 'L', 170),
                (1, 'J', 40),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 60),
            ],
        ),
        (
            "<div style='height:130px'>A</div><div style='height:150px'>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 130),
                (0, 'L', 150),
                (0, 'L', 170),
                (1, 'J', 60),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'Z', 80),
            ],
        ),
        (
            "<div style='height:150px'>A</div><div style='height:150px'>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 150),
                (1, 'J', 100),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'Z', 100),
            ],
        ),
        (
            "<div style='height:170px'>A</div><div style='height:150px'>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 170),
                (1, 'J', 100),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'Z', 120),
            ],
        ),
    ]);
}

#[test]
fn the_lines_after_a_leading_block_break_apart_from_the_next_block() {
    // A block before the text leaves the line between the two blocks to break
    // on its own: it stays on the first page, and the next block breaks.
    assert_rows(&[
        (
            "<div style='height:90px'>A</div><div><div>X</div>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 110),
                (0, 'L', 130),
                (0, 'L', 150),
                (0, 'L', 170),
                (0, 'X', 90),
                (1, 'J', 40),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 60),
            ],
        ),
        (
            "<div style='height:110px'>A</div><div><div>X</div>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 130),
                (0, 'L', 150),
                (0, 'L', 170),
                (0, 'X', 110),
                (1, 'J', 60),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'Z', 80),
            ],
        ),
        (
            "<div style='height:130px'>A</div><div><div>X</div>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 150),
                (0, 'X', 130),
                (1, 'J', 100),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'Z', 120),
            ],
        ),
        (
            "<div style='height:150px'>A</div><div><div>X</div>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 170),
                (0, 'X', 150),
                (1, 'J', 100),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'Z', 120),
            ],
        ),
    ]);
}

#[test]
fn a_paragraph_in_an_inline_block_or_a_flex_item_breaks_the_same_way() {
    // The paragraph that holds the block can sit in an inline-block, or be a
    // flex item alone on its line.
    assert_rows(&[
        (
            "<div style='height:90px'>A</div><div style='display:inline-block'>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (1, 'I', 0),
                (1, 'J', 120),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'L', 100),
                (1, 'Z', 140),
            ],
        ),
        (
            "<div style='height:90px'>A</div><div style='display:flex'><div>I<div>L<br>L<br>L<br>L<br>L</div>J</div></div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 90),
                (0, 'L', 110),
                (0, 'L', 130),
                (0, 'L', 150),
                (1, 'J', 40),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 60),
            ],
        ),
        (
            "<div style='height:110px'>A</div><div style='display:inline-block'>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (1, 'I', 0),
                (1, 'J', 120),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'L', 100),
                (1, 'Z', 140),
            ],
        ),
        (
            "<div style='height:110px'>A</div><div style='display:flex'><div>I<div>L<br>L<br>L<br>L<br>L</div>J</div></div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 110),
                (0, 'L', 130),
                (0, 'L', 150),
                (0, 'L', 170),
                (1, 'J', 40),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'Z', 60),
            ],
        ),
        (
            "<div style='height:130px'>A</div><div style='display:inline-block'>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (1, 'I', 0),
                (1, 'J', 120),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'L', 100),
                (1, 'Z', 140),
            ],
        ),
        (
            "<div style='height:130px'>A</div><div style='display:flex'><div>I<div>L<br>L<br>L<br>L<br>L</div>J</div></div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 130),
                (0, 'L', 150),
                (0, 'L', 170),
                (1, 'J', 60),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'Z', 80),
            ],
        ),
        (
            "<div style='height:150px'>A</div><div style='display:inline-block'>I<div>L<br>L<br>L<br>L<br>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (1, 'I', 0),
                (1, 'J', 120),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'L', 100),
                (1, 'Z', 140),
            ],
        ),
        (
            "<div style='height:150px'>A</div><div style='display:flex'><div>I<div>L<br>L<br>L<br>L<br>L</div>J</div></div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 150),
                (1, 'J', 100),
                (1, 'L', 0),
                (1, 'L', 20),
                (1, 'L', 40),
                (1, 'L', 60),
                (1, 'L', 80),
                (1, 'Z', 120),
            ],
        ),
    ]);
}

#[test]
fn the_lines_before_the_block_break_between_them() {
    // The lines of the paragraph before its block break between them as a
    // paragraph of their own: the line that crosses the page end starts the
    // next page, and the block and the text after it follow.
    assert_rows(&[
        (
            "<div style='height:90px'>A</div><div>I<br>I<br>I<div>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 90),
                (0, 'I', 110),
                (0, 'I', 130),
                (0, 'J', 170),
                (0, 'L', 150),
                (1, 'Z', 0),
            ],
        ),
        (
            "<div style='height:90px'>A</div><div>I<br>I<br>I<div>L</div></div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 90),
                (0, 'I', 110),
                (0, 'I', 130),
                (0, 'L', 150),
                (0, 'Z', 170),
            ],
        ),
        (
            "<div style='height:90px'>A</div><div>I<br>I<br>I<br>I<br>I<div>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 90),
                (0, 'I', 110),
                (0, 'I', 130),
                (0, 'I', 150),
                (0, 'I', 170),
                (1, 'J', 20),
                (1, 'L', 0),
                (1, 'Z', 40),
            ],
        ),
        (
            "<div style='height:90px'>A</div><div>I<br>I<br>I<br>I<br>I<div>L</div></div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 90),
                (0, 'I', 110),
                (0, 'I', 130),
                (0, 'I', 150),
                (0, 'I', 170),
                (1, 'L', 0),
                (1, 'Z', 20),
            ],
        ),
        (
            "<div style='height:110px'>A</div><div>I<br>I<br>I<div>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 110),
                (0, 'I', 130),
                (0, 'I', 150),
                (0, 'L', 170),
                (1, 'J', 0),
                (1, 'Z', 20),
            ],
        ),
        (
            "<div style='height:110px'>A</div><div>I<br>I<br>I<div>L</div></div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 110),
                (0, 'I', 130),
                (0, 'I', 150),
                (0, 'L', 170),
                (1, 'Z', 0),
            ],
        ),
        (
            "<div style='height:110px'>A</div><div>I<br>I<br>I<br>I<br>I<div>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 110),
                (0, 'I', 130),
                (0, 'I', 150),
                (1, 'I', 0),
                (1, 'I', 20),
                (1, 'J', 60),
                (1, 'L', 40),
                (1, 'Z', 80),
            ],
        ),
        (
            "<div style='height:110px'>A</div><div>I<br>I<br>I<br>I<br>I<div>L</div></div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 110),
                (0, 'I', 130),
                (0, 'I', 150),
                (1, 'I', 0),
                (1, 'I', 20),
                (1, 'L', 40),
                (1, 'Z', 60),
            ],
        ),
        (
            "<div style='height:130px'>A</div><div>I<br>I<br>I<div>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 130),
                (0, 'I', 150),
                (0, 'I', 170),
                (1, 'J', 20),
                (1, 'L', 0),
                (1, 'Z', 40),
            ],
        ),
        (
            "<div style='height:130px'>A</div><div>I<br>I<br>I<div>L</div></div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 130),
                (0, 'I', 150),
                (0, 'I', 170),
                (1, 'L', 0),
                (1, 'Z', 20),
            ],
        ),
        (
            "<div style='height:130px'>A</div><div>I<br>I<br>I<br>I<br>I<div>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 130),
                (0, 'I', 150),
                (0, 'I', 170),
                (1, 'I', 0),
                (1, 'I', 20),
                (1, 'J', 60),
                (1, 'L', 40),
                (1, 'Z', 80),
            ],
        ),
        (
            "<div style='height:130px'>A</div><div>I<br>I<br>I<br>I<br>I<div>L</div></div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 130),
                (0, 'I', 150),
                (0, 'I', 170),
                (1, 'I', 0),
                (1, 'I', 20),
                (1, 'L', 40),
                (1, 'Z', 60),
            ],
        ),
        (
            "<div style='height:150px'>A</div><div>I<br>I<br>I<div>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 150),
                (0, 'I', 170),
                (1, 'I', 0),
                (1, 'J', 40),
                (1, 'L', 20),
                (1, 'Z', 60),
            ],
        ),
        (
            "<div style='height:150px'>A</div><div>I<br>I<br>I<div>L</div></div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 150),
                (0, 'I', 170),
                (1, 'I', 0),
                (1, 'L', 20),
                (1, 'Z', 40),
            ],
        ),
        (
            "<div style='height:150px'>A</div><div>I<br>I<br>I<br>I<br>I<div>L</div>J</div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 150),
                (0, 'I', 170),
                (1, 'I', 0),
                (1, 'I', 20),
                (1, 'I', 40),
                (1, 'J', 80),
                (1, 'L', 60),
                (1, 'Z', 100),
            ],
        ),
        (
            "<div style='height:150px'>A</div><div>I<br>I<br>I<br>I<br>I<div>L</div></div><div>Z</div>",
            &[
                (0, 'A', 0),
                (0, 'I', 150),
                (0, 'I', 170),
                (1, 'I', 0),
                (1, 'I', 20),
                (1, 'I', 40),
                (1, 'L', 60),
                (1, 'Z', 80),
            ],
        ),
    ]);
}
