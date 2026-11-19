//! A forced break before the first content of a table cell, at any depth of
//! blocks inside it, moves the cell's row to the next page, and the content
//! starts that page without breaking again. Where each glyph falls is what
//! Chromium prints for the same documents on pages 200px wide and 100px tall,
//! with Ahem glyphs of 10px on lines of 20px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The page, glyph, left edge and glyph top of every glyph of text in
/// `body`, laid out on the pages and with the font of the Chromium documents.
fn glyphs(body: &str) -> Vec<(u32, char, i32, i32)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:200px 100px;margin:0}} body{{margin:0;font:10px/20px Ahem}} table{{border-spacing:0}} td{{padding:0;vertical-align:top}}</style><body>{body}</body>"
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
            // Every Ahem glyph is 10px wide.
            for (offset, glyph) in run.text.chars().enumerate() {
                if !glyph.is_whitespace() {
                    glyphs.push((
                        index,
                        glyph,
                        (run.origin.0 + 10.0 * offset as f32).round() as i32,
                        (run.origin.1 - run.ascent).round() as i32,
                    ));
                }
            }
        }
    }
    glyphs.sort_unstable();
    glyphs
}

#[test]
fn a_forced_break_in_a_nested_block_at_a_cell_start_breaks_once() {
    // A forced break in a block nested at the start of a cell moves the row
    // to the next page, and the content starts that page instead of breaking
    // a second time.
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><table><tr><td><div><div style="break-before:page">A</div></div></td><td><div style="height:60px">B</div></td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'A', 0, 5),
            (1, 'B', 10, 5),
            (1, 'R', 0, 65),
            (1, 'X', 0, 85),
        ]
    );
}

#[test]
fn a_forced_break_in_a_nested_block_moves_a_fitting_row() {
    // The row fits the page, but the forced break at its cell start still
    // moves it.
    assert_eq!(
        glyphs(
            r#"<div style="height:20px">W</div><table><tr><td><div><div style="break-before:page">A</div></div></td><td>B</td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'A', 0, 5),
            (1, 'B', 10, 5),
            (1, 'R', 0, 25),
            (1, 'X', 0, 45),
        ]
    );
}

#[test]
fn a_forced_break_deep_in_nested_blocks_moves_the_row_once() {
    // Several levels of blocks between the cell and the forced break.
    assert_eq!(
        glyphs(
            r#"<div style="height:20px">W</div><table><tr><td><div><div><div style="break-before:page">A</div></div></div></td><td>B</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'A', 0, 5),
            (1, 'B', 10, 5),
            (1, 'X', 0, 25),
        ]
    );
}

#[test]
fn a_forced_break_after_a_wrapper_padding_moves_the_row_with_it() {
    // The wrapper's top padding comes before the break and moves with the
    // row.
    assert_eq!(
        glyphs(
            r#"<div style="height:20px">W</div><table><tr><td><div style="padding-top:10px"><div style="break-before:page">A</div></div></td><td>B</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'A', 0, 15),
            (1, 'B', 10, 5),
            (1, 'X', 0, 35),
        ]
    );
}

#[test]
fn a_forced_break_in_a_later_row_moves_only_that_row() {
    // The row before stays on the first page.
    assert_eq!(
        glyphs(
            r#"<div style="height:20px">W</div><table><tr><td>Q</td></tr><tr><td><div><div style="break-before:page">A</div></div></td><td>B</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'Q', 0, 25),
            (0, 'W', 0, 5),
            (1, 'A', 0, 5),
            (1, 'B', 10, 5),
            (1, 'X', 0, 25),
        ]
    );
}

#[test]
fn each_row_with_a_nested_forced_break_starts_a_page() {
    // Two consecutive rows each start a page of their own.
    assert_eq!(
        glyphs(
            r#"<div style="height:20px">W</div><table><tr><td><div><div style="break-before:page">A</div></div></td><td>B</td></tr><tr><td><div><div style="break-before:page">C</div></div></td><td>D</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'A', 0, 5),
            (1, 'B', 10, 5),
            (2, 'C', 0, 5),
            (2, 'D', 10, 5),
            (2, 'X', 0, 25),
        ]
    );
}

#[test]
fn a_nested_forced_break_at_the_document_start_adds_no_page() {
    // A table that opens the document: the break before the first page is
    // dropped.
    assert_eq!(
        glyphs(
            r#"<table><tr><td><div><div style="break-before:page">A</div></div></td><td>B</td></tr></table><div>X</div>"#
        ),
        [(0, 'A', 0, 5), (0, 'B', 10, 5), (0, 'X', 0, 25),]
    );
}

#[test]
fn a_forced_break_on_the_cell_itself_moves_the_row() {
    // The cell itself carries the forced break.
    assert_eq!(
        glyphs(
            r#"<div style="height:20px">W</div><table><tr><td style="break-before:page">A</td><td>B</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'A', 0, 5),
            (1, 'B', 10, 5),
            (1, 'X', 0, 25),
        ]
    );
}

#[test]
fn a_forced_break_in_a_second_cell_moves_the_row() {
    // The forced break is nested in the second cell of the row.
    assert_eq!(
        glyphs(
            r#"<div style="height:20px">W</div><table><tr><td>B</td><td><div><div style="break-before:page">A</div></div></td></tr></table><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'A', 10, 5),
            (1, 'B', 0, 5),
            (1, 'X', 0, 25),
        ]
    );
}

#[test]
fn a_forced_break_in_a_second_cell_of_a_crossing_row_moves_it() {
    // The forced break is nested in the second cell, and the row crosses the
    // page end.
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><table><tr><td><div style="height:60px">B</div></td><td><div><div style="break-before:page">A</div></div></td></tr></table><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'A', 10, 5),
            (1, 'B', 0, 5),
            (1, 'X', 0, 65),
        ]
    );
}
