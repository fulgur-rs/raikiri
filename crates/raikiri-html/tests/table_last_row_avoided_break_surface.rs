//! A table followed by a box that it may not break away from keeps its last
//! part with that box: the page breaks between the rows of the table, and the
//! last row, or a caption below the rows, starts the next page with the box.
//! Where each glyph falls is what Chromium prints for the same documents on
//! pages 200px wide and 100px tall, with Ahem glyphs of 20px on lines of
//! 20px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The page, glyph and glyph top of every glyph of text in `body`, laid out
/// on the pages and with the font of the Chromium documents. Every glyph
/// starts its line, at the left edge.
fn glyphs(body: &str) -> Vec<(u32, char, i32)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 100px;margin:0}} body{{margin:0;font:20px/20px Ahem}} table{{border-spacing:0}} td{{padding:0}}</style><body>{body}</body>"
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
    let mut glyphs = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            for glyph in run.text.chars().filter(|glyph| !glyph.is_whitespace()) {
                glyphs.push((index, glyph, (run.origin.1 - run.ascent).round() as i32));
            }
        }
    }
    glyphs.sort_unstable();
    glyphs
}

const ROWS: &str = "<tr><td>L</td></tr><tr><td>X</td></tr>";

#[test]
fn the_last_row_moves_with_a_box_that_avoids_the_break_before_it() {
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:50px">S</div><table>{ROWS}</table><div style="break-before:avoid">Z</div>"#
        )),
        [(0, 'L', 50), (0, 'S', 0), (1, 'X', 0), (1, 'Z', 20)]
    );
}

#[test]
fn the_last_row_moves_when_the_table_avoids_the_break_after_it() {
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:50px">S</div><table style="break-after:avoid">{ROWS}</table><div>Z</div>"#
        )),
        [(0, 'L', 50), (0, 'S', 0), (1, 'X', 0), (1, 'Z', 20)]
    );
}

#[test]
fn the_last_row_moves_with_a_chain_of_avoided_breaks() {
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:50px">S</div><table>{ROWS}</table><div style="break-before:avoid;break-after:avoid">Y</div><div style="break-before:avoid">Z</div>"#
        )),
        [
            (0, 'L', 50),
            (0, 'S', 0),
            (1, 'X', 0),
            (1, 'Y', 20),
            (1, 'Z', 40)
        ]
    );
}

#[test]
fn a_table_of_one_row_moves_whole() {
    assert_eq!(
        glyphs(
            r#"<div style="height:70px">S</div><table><tr><td>X</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [(0, 'S', 0), (1, 'X', 0), (1, 'Z', 20)]
    );
}

#[test]
fn a_last_row_group_that_avoids_breaks_inside_it_moves_whole() {
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:30px">S</div><table><tbody><tr><td>K</td></tr></tbody><tbody style="break-inside:avoid">{ROWS}</tbody></table><div style="break-before:avoid">Z</div>"#
        )),
        [
            (0, 'K', 30),
            (0, 'S', 0),
            (1, 'L', 0),
            (1, 'X', 20),
            (1, 'Z', 40)
        ]
    );
}

#[test]
fn a_caption_below_the_rows_moves_alone() {
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:50px">S</div><table style="caption-side:bottom"><caption>C</caption>{ROWS}</table><div style="break-before:avoid">Z</div>"#
        )),
        [
            (0, 'L', 50),
            (0, 'S', 0),
            (0, 'X', 70),
            (1, 'C', 0),
            (1, 'Z', 20)
        ]
    );
}

#[test]
fn a_table_inside_a_block_moves_its_last_row_with_the_box_after_it() {
    // The table starts the block's boxes, and the page breaks inside it all
    // the same.
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:50px">S</div><div><table>{ROWS}</table><div style="break-before:avoid">Z</div></div>"#
        )),
        [(0, 'L', 50), (0, 'S', 0), (1, 'X', 0), (1, 'Z', 20)]
    );
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:30px">S</div><div><div>Q</div><table>{ROWS}</table><div style="break-before:avoid">Z</div></div>"#
        )),
        [
            (0, 'L', 50),
            (0, 'Q', 30),
            (0, 'S', 0),
            (1, 'X', 0),
            (1, 'Z', 20)
        ]
    );
}

#[test]
fn a_table_that_fits_with_the_box_after_it_stays() {
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:20px">S</div><table>{ROWS}</table><div style="break-before:avoid">Z</div><div style="height:60px">Q</div>"#
        )),
        [
            (0, 'L', 20),
            (0, 'Q', 80),
            (0, 'S', 0),
            (0, 'X', 40),
            (0, 'Z', 60)
        ]
    );
}
