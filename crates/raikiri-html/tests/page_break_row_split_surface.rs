//! A table row that crosses the page end breaks there with its cells, each
//! laid out as if it were aligned to its top, unless a cell of it breaks
//! before any of its content: then the row starts the next page whole. Where
//! each glyph falls is what Chromium prints for the same documents on pages
//! 200px wide and 100px tall, with Ahem glyphs of 10px on lines of 20px.

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
fn a_row_beside_a_spanning_cell_breaks_with_its_cells() {
    // The row continues the spanning cell of the row before it; its cell's box
    // breaks at the page end.
    assert_eq!(
        glyphs(
            r#"<table><tr><td><div style="height:60px">A</div></td><td rowspan=2><div>C</div></td></tr><tr><td><div style="height:60px">E</div></td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (0, 'C', 10, 5),
            (0, 'E', 0, 65),
            (1, 'X', 0, 25),
        ]
    );
}

#[test]
fn a_row_breaks_inside_cells_whose_boxes_break() {
    // Blocks, nested blocks, a cell spanning columns or rows, and cells with
    // spacing between them all put content before the page end.
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><table><tr><td><div style="height:60px">A</div></td><td><div>B</div></td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 65),
            (0, 'B', 10, 65),
            (0, 'W', 0, 5),
            (1, 'R', 0, 25),
            (1, 'X', 0, 45),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:70px">W</div><table><tr><td><div><div style="height:60px">A</div></div></td><td>B</td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 75),
            (0, 'B', 10, 75),
            (0, 'W', 0, 5),
            (1, 'R', 0, 35),
            (1, 'X', 0, 55),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><table style="border-spacing:5px"><tr><td><div style="height:60px">A</div></td><td>B</td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 5, 70),
            (0, 'B', 20, 70),
            (0, 'W', 0, 5),
            (1, 'R', 5, 35),
            (1, 'X', 0, 60),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><table><tr><td colspan=2><div style="height:60px">A</div></td></tr><tr><td>R</td><td>S</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 65),
            (0, 'W', 0, 5),
            (1, 'R', 0, 25),
            (1, 'S', 10, 25),
            (1, 'X', 0, 45),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:50px">W</div><table><tr><td rowspan=2><div style="height:70px">A</div></td><td>B</td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 55),
            (0, 'B', 10, 55),
            (0, 'W', 0, 5),
            (1, 'R', 10, 5),
            (1, 'X', 0, 40),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:50px">W</div><table><tr><td><div style="padding-bottom:30px">A</div></td><td><div style="height:60px">B</div></td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 55),
            (0, 'B', 10, 55),
            (0, 'W', 0, 5),
            (1, 'R', 0, 15),
            (1, 'X', 0, 35),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><table><tr><td><div style="float:left;height:20px;width:10px"></div>A</td><td><div style="height:60px">B</div></td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 10, 65),
            (0, 'B', 20, 65),
            (0, 'W', 0, 5),
            (1, 'R', 0, 25),
            (1, 'X', 0, 45),
        ]
    );
}

#[test]
fn a_row_taller_than_its_content_breaks_below_the_lines() {
    // Each cell's single line fits; the rest of the row's height goes on.
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><table><tr style="height:50px"><td>A</td><td>B</td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 65),
            (0, 'B', 10, 65),
            (0, 'W', 0, 5),
            (1, 'R', 0, 15),
            (1, 'X', 0, 35),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><table><tr><td>A</td><td><div style="height:60px">B</div></td></tr><tr><td>R<br>S<br>T</td><td>U</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 65),
            (0, 'B', 10, 65),
            (0, 'W', 0, 5),
            (1, 'R', 0, 25),
            (1, 'S', 0, 45),
            (1, 'T', 0, 65),
            (1, 'U', 10, 25),
            (1, 'X', 0, 85),
        ]
    );
}

#[test]
fn a_cell_aligned_off_its_top_breaks_as_if_aligned_to_its_top() {
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><table><tr><td style="vertical-align:middle">A</td><td><div style="height:50px">B</div></td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 65),
            (0, 'B', 10, 65),
            (0, 'W', 0, 5),
            (1, 'R', 0, 15),
            (1, 'X', 0, 35),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:70px">W</div><table><tr><td style="vertical-align:bottom">A</td><td><div style="height:50px">B</div></td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 75),
            (0, 'B', 10, 75),
            (0, 'W', 0, 5),
            (1, 'R', 0, 25),
            (1, 'X', 0, 45),
        ]
    );
}

#[test]
fn a_row_breaks_between_the_lines_of_a_cell() {
    // Two lines go before the break; with three lines the widows give way to
    // the orphans.
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><table><tr><td>A<br>B<br>C<br>D</td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 65),
            (0, 'B', 0, 85),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'D', 0, 25),
            (1, 'R', 0, 45),
            (1, 'X', 0, 65),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><table><tr><td>A<br>B<br>C</td><td>D</td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 65),
            (0, 'B', 0, 85),
            (0, 'D', 10, 65),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'R', 0, 25),
            (1, 'X', 0, 45),
        ]
    );
}

#[test]
fn a_cell_box_that_avoids_a_break_moves_down_inside_the_split_row() {
    assert_eq!(
        glyphs(
            r#"<table><tr><td><div style="height:40px">W</div></td><td><div style="height:40px">V</div></td></tr><tr><td><div style="height:10px">P</div><div style="height:70px;break-inside:avoid">Y</div></td><td><div style="height:90px">D</div></td></tr><tr><td>R</td><td>S</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'D', 10, 45),
            (0, 'P', 0, 45),
            (0, 'V', 10, 5),
            (0, 'W', 0, 5),
            (1, 'R', 0, 75),
            (1, 'S', 10, 75),
            (1, 'Y', 0, 5),
            (2, 'X', 0, 5),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:40px">W</div><table><tr><td><div style="height:20px">P</div><div style="height:50px;break-inside:avoid">Y</div><div>Z</div></td><td><div style="height:60px">D</div></td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'D', 10, 45),
            (0, 'P', 0, 45),
            (0, 'W', 0, 5),
            (1, 'R', 0, 75),
            (1, 'Y', 0, 5),
            (1, 'Z', 0, 55),
            (2, 'X', 0, 5),
        ]
    );
}

#[test]
fn rows_of_empty_cells_break_across_the_page_end() {
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><table style="border-spacing:2px"><tr><td style="height:20px"></td></tr><tr><td style="height:30px"></td></tr></table><div>X</div>"#
        ),
        [(0, 'W', 0, 5), (1, 'X', 0, 21),]
    );
}

#[test]
fn a_row_whose_cell_breaks_before_its_content_moves_whole() {
    // The edges that close the cell's only line, and the padding above its
    // content, run past the page end.
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><table><tr><td style="padding-bottom:40px">A</td><td>B</td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'A', 0, 5),
            (1, 'B', 10, 5),
            (1, 'R', 0, 65),
            (1, 'X', 0, 85),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><table><tr><td style="padding-top:30px">A</td><td>B</td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'A', 0, 35),
            (1, 'B', 10, 5),
            (1, 'R', 0, 55),
            (1, 'X', 0, 75),
        ]
    );
}

#[test]
fn a_row_whose_first_box_breaks_before_the_content_moves_whole() {
    // A first box that avoids a break inside it, a replaced one, and one
    // whose margin puts it past the page end.
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><table><tr><td><div style="height:50px;break-inside:avoid">A</div></td><td>B</td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'A', 0, 5),
            (1, 'B', 10, 5),
            (1, 'R', 0, 55),
            (1, 'X', 0, 75),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><table><tr><td><img style="display:block;height:50px;width:10px"><div>A</div></td><td>B</td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'A', 0, 55),
            (1, 'B', 10, 5),
            (1, 'R', 0, 75),
            (2, 'X', 0, 5),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><table><tr><td><div style="margin-top:45px">A</div></td><td><div style="height:50px">B</div></td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'A', 0, 50),
            (1, 'B', 10, 5),
            (1, 'R', 0, 70),
            (2, 'X', 0, 5),
        ]
    );
}

#[test]
fn a_first_box_that_avoids_a_break_and_fits_lets_the_row_break_after_it() {
    assert_eq!(
        glyphs(
            r#"<div style="height:40px">W</div><table><tr><td><div style="height:30px;break-inside:avoid">A</div><div style="height:40px">C</div></td><td>B</td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 45),
            (0, 'B', 10, 45),
            (0, 'C', 0, 75),
            (0, 'W', 0, 5),
            (1, 'R', 0, 15),
            (1, 'X', 0, 35),
        ]
    );
}

#[test]
fn a_row_whose_line_before_a_block_crosses_the_page_end_moves_whole() {
    assert_eq!(
        glyphs(
            r#"<div style="height:85px">W</div><table><tr><td>A<div>B</div></td><td>C</td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'A', 0, 5),
            (1, 'B', 0, 25),
            (1, 'C', 10, 5),
            (1, 'R', 0, 45),
            (1, 'X', 0, 65),
        ]
    );
}

#[test]
fn a_cell_that_avoids_a_break_or_breaks_before_itself_moves_its_row_whole() {
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><table><tr><td style="break-inside:avoid"><div style="height:60px">A</div></td><td>B</td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'A', 0, 5),
            (1, 'B', 10, 5),
            (1, 'R', 0, 65),
            (1, 'X', 0, 85),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><table><tr><td style="break-before:page"><div style="height:60px">A</div></td><td>B</td></tr><tr><td>R</td></tr></table><div>X</div>"#
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
fn a_break_forced_before_the_first_box_of_a_cell_moves_its_row_whole() {
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><table><tr><td><div style="break-before:page">A</div></td><td><div style="height:60px">B</div></td></tr><tr><td>R</td></tr></table><div>X</div>"#
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
fn a_float_that_breaks_lets_its_row_break_with_it() {
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><table><tr><td><div style="float:left;height:60px;width:10px">F</div>A</td><td><div style="height:60px">B</div></td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 10, 65),
            (0, 'B', 20, 65),
            (0, 'F', 0, 65),
            (0, 'W', 0, 5),
            (1, 'R', 0, 25),
            (1, 'X', 0, 45),
        ]
    );
}

#[test]
fn a_float_whose_lines_break_at_the_page_end_lets_the_row_break() {
    // The float's lines break between themselves at the page end, where
    // they are, so the row breaks with them rather than moving whole.
    assert_eq!(
        glyphs(
            "<div style=\"height:40px\">W</div><table><tr><td><div style=\"float:left;width:20px\">F<br>G<br>H<br>I<br>J</div>A</td></tr></table><div>X</div>"
        ),
        [
            (0, 'A', 20, 45),
            (0, 'F', 0, 45),
            (0, 'G', 0, 65),
            (0, 'H', 0, 85),
            (0, 'W', 0, 5),
            (1, 'I', 0, 5),
            (1, 'J', 0, 25),
            (1, 'X', 0, 45),
        ]
    );
}
