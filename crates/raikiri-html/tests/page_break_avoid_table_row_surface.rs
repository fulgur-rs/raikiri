//! A box that avoids a break after it, followed by a table, stays with the
//! part of the table's first row that comes before the first break that
//! splits the row, as in Chromium; it moves with the row only when the row
//! cannot split. Where each glyph falls is what Chromium prints for the same
//! documents on pages 200px wide and 100px tall, with Ahem glyphs of 10px on
//! lines of 20px.

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
fn a_heading_stays_with_the_line_of_a_tall_first_row() {
    // A row taller than the room left breaks after its line, so the heading
    // that avoids a break after it stays with that line.
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><div style="break-after:avoid">H</div><table><tr><td style="height:50px">A</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 85),
            (0, 'H', 0, 65),
            (0, 'W', 0, 5),
            (1, 'X', 0, 35),
        ]
    );
}

#[test]
fn a_heading_stays_with_a_first_row_that_breaks_below_its_line() {
    // The row breaks inside its set height, below its only line.
    assert_eq!(
        glyphs(
            r#"<div style="height:50px">W</div><div style="break-after:avoid">H</div><table><tr><td style="height:50px">A</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 75),
            (0, 'H', 0, 55),
            (0, 'W', 0, 5),
            (1, 'X', 0, 25),
        ]
    );
}

#[test]
fn a_heading_stays_with_the_lines_of_a_first_row_that_fit() {
    // The first two lines fit with the heading; the third starts the next
    // page.
    assert_eq!(
        glyphs(
            r#"<div style="height:40px">W</div><div style="break-after:avoid">H</div><table><tr><td>A<br>B<br>C</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 65),
            (0, 'B', 0, 85),
            (0, 'H', 0, 45),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'X', 0, 25),
        ]
    );
}

#[test]
fn a_heading_stays_with_the_first_block_of_a_cell() {
    // The cell breaks between its blocks.
    assert_eq!(
        glyphs(
            r#"<div style="height:50px">W</div><div style="break-after:avoid">H</div><table><tr><td><div>A</div><div>B</div><div>C</div></td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 75),
            (0, 'H', 0, 55),
            (0, 'W', 0, 5),
            (1, 'B', 0, 5),
            (1, 'C', 0, 25),
            (1, 'X', 0, 45),
        ]
    );
}

#[test]
fn a_heading_stays_when_every_cell_of_the_first_row_breaks() {
    // Each cell puts content before the break.
    assert_eq!(
        glyphs(
            r#"<div style="height:40px">W</div><div style="break-after:avoid">H</div><table><tr><td>A<br>B<br>C</td><td>D</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 65),
            (0, 'B', 0, 85),
            (0, 'D', 10, 65),
            (0, 'H', 0, 45),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'X', 0, 25),
        ]
    );
}

#[test]
fn a_heading_stays_when_one_cell_fits_and_the_other_breaks() {
    // One cell fits whole; the other breaks between its blocks.
    assert_eq!(
        glyphs(
            r#"<div style="height:50px">W</div><div style="break-after:avoid">H</div><table><tr><td><div>A</div><div>B</div><div>C</div></td><td>D</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 75),
            (0, 'D', 10, 75),
            (0, 'H', 0, 55),
            (0, 'W', 0, 5),
            (1, 'B', 0, 5),
            (1, 'C', 0, 25),
            (1, 'X', 0, 45),
        ]
    );
}

#[test]
fn a_heading_moves_with_a_first_row_whose_orphans_do_not_fit() {
    // Only one line fits, fewer than the orphans: the row moves whole, and
    // the heading with it.
    assert_eq!(
        glyphs(
            r#"<div style="height:50px">W</div><div style="break-after:avoid">H</div><table><tr><td>A<br>B<br>C</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'A', 0, 25),
            (1, 'B', 0, 45),
            (1, 'C', 0, 65),
            (1, 'H', 0, 5),
            (1, 'X', 0, 85),
        ]
    );
}

#[test]
fn a_heading_moves_with_a_first_row_whose_first_line_crosses() {
    // The first line would cross the page end.
    assert_eq!(
        glyphs(
            r#"<div style="height:70px">W</div><div style="break-after:avoid">H</div><table><tr><td>A<br>B</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'A', 0, 25),
            (1, 'B', 0, 45),
            (1, 'H', 0, 5),
            (1, 'X', 0, 65),
        ]
    );
}

#[test]
fn a_heading_moves_with_a_first_row_that_starts_with_an_avoid_box() {
    // The cell starts with a box that avoids breaks inside it and crosses the
    // page end.
    assert_eq!(
        glyphs(
            r#"<div style="height:50px">W</div><div style="break-after:avoid">H</div><table><tr><td><div style="break-inside:avoid">A<br>B</div></td></tr></table><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'A', 0, 25),
            (1, 'B', 0, 45),
            (1, 'H', 0, 5),
            (1, 'X', 0, 65),
        ]
    );
}

#[test]
fn a_heading_moves_with_a_first_row_whose_cell_avoids_breaks() {
    // The cell itself avoids breaks inside it.
    assert_eq!(
        glyphs(
            r#"<div style="height:40px">W</div><div style="break-after:avoid">H</div><table><tr><td style="break-inside:avoid">A<br>B<br>C</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'A', 0, 25),
            (1, 'B', 0, 45),
            (1, 'C', 0, 65),
            (1, 'H', 0, 5),
            (1, 'X', 0, 85),
        ]
    );
}

#[test]
fn a_heading_stays_with_a_first_row_that_fits_before_the_next() {
    // The first row fits; the second one starts the next page.
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><div style="break-after:avoid">H</div><table><tr><td>A</td></tr><tr><td>B<br>C</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 85),
            (0, 'H', 0, 65),
            (0, 'W', 0, 5),
            (1, 'B', 0, 5),
            (1, 'C', 0, 25),
            (1, 'X', 0, 45),
        ]
    );
}

#[test]
fn a_table_that_avoids_a_break_before_it_keeps_the_lines_that_fit() {
    // `break-before: avoid` on the table ties it to the box before it the same
    // way.
    assert_eq!(
        glyphs(
            r#"<div style="height:40px">W</div><div>H</div><table style="break-before:avoid"><tr><td>A<br>B<br>C</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 65),
            (0, 'B', 0, 85),
            (0, 'H', 0, 45),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'X', 0, 25),
        ]
    );
}

#[test]
fn two_headings_stay_with_the_lines_of_the_first_row() {
    // Two headings chained by avoided breaks stay with the lines that fit.
    assert_eq!(
        glyphs(
            r#"<div style="height:20px">W</div><div style="break-after:avoid">H</div><div style="break-after:avoid">H</div><table><tr><td>A<br>B<br>C<br>D</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 65),
            (0, 'B', 0, 85),
            (0, 'H', 0, 25),
            (0, 'H', 0, 45),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'D', 0, 25),
            (1, 'X', 0, 45),
        ]
    );
}

#[test]
fn a_heading_in_a_wrapper_stays_with_the_lines_that_fit() {
    // The heading and the table share a wrapper.
    assert_eq!(
        glyphs(
            r#"<div style="height:40px">W</div><div><div style="break-after:avoid">H</div><table><tr><td>A<br>B<br>C</td></tr></table></div><div>X</div>"#
        ),
        [
            (0, 'A', 0, 65),
            (0, 'B', 0, 85),
            (0, 'H', 0, 45),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'X', 0, 25),
        ]
    );
}

#[test]
fn a_heading_moves_when_border_spacing_leaves_room_for_one_line() {
    // The border spacing above the row leaves room for one line, fewer than
    // the orphans, so the row moves whole with the heading.
    assert_eq!(
        glyphs(
            r#"<div style="height:40px">W</div><div style="break-after:avoid">H</div><table style="border-spacing:5px"><tr><td>A<br>B<br>C</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'A', 5, 30),
            (1, 'B', 5, 50),
            (1, 'C', 5, 70),
            (1, 'H', 0, 5),
            (2, 'X', 0, 5),
        ]
    );
}

#[test]
fn a_heading_stays_when_widows_move_lines_of_the_first_row() {
    // The widows take lines to the next page, but two still fit.
    assert_eq!(
        glyphs(
            r#"<div style="height:40px">W</div><div style="break-after:avoid">H</div><table><tr><td style="widows:3">A<br>B<br>C<br>D</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 65),
            (0, 'B', 0, 85),
            (0, 'H', 0, 45),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'D', 0, 25),
            (1, 'X', 0, 45),
        ]
    );
}

#[test]
fn a_heading_stays_with_the_lines_of_a_table_in_the_block_after_it() {
    // The table is the first box of the block after the heading.
    assert_eq!(
        glyphs(
            r#"<div style="height:40px">W</div><div style="break-after:avoid">H</div><div><table><tr><td>A<br>B<br>C</td></tr></table></div><div>X</div>"#
        ),
        [
            (0, 'A', 0, 65),
            (0, 'B', 0, 85),
            (0, 'H', 0, 45),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'X', 0, 25),
        ]
    );
}

#[test]
fn a_heading_moves_with_a_table_in_the_block_after_it_that_cannot_split() {
    // Only one line of the table in the block after the heading fits.
    assert_eq!(
        glyphs(
            r#"<div style="height:50px">W</div><div style="break-after:avoid">H</div><div><table><tr><td>A<br>B<br>C</td></tr></table></div><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'A', 0, 25),
            (1, 'B', 0, 45),
            (1, 'C', 0, 65),
            (1, 'H', 0, 5),
            (1, 'X', 0, 85),
        ]
    );
}

#[test]
fn a_heading_stays_with_a_middle_aligned_cell_that_breaks() {
    // The middle-aligned cell loses its alignment room when the row breaks,
    // so its line fits with the heading.
    assert_eq!(
        glyphs(
            r#"<div style="height:60px">W</div><div style="break-after:avoid">H</div><table><tr><td style="vertical-align:middle">A</td><td><div style="height:60px">B</div></td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 85),
            (0, 'B', 10, 85),
            (0, 'H', 0, 65),
            (0, 'W', 0, 5),
            (1, 'X', 0, 45),
        ]
    );
}

#[test]
fn a_heading_moves_with_a_first_row_that_avoids_breaks_inside_it() {
    // The row itself avoids breaks inside it.
    assert_eq!(
        glyphs(
            r#"<div style="height:40px">W</div><div style="break-after:avoid">H</div><table><tr style="break-inside:avoid"><td>A<br>B<br>C</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'W', 0, 5),
            (1, 'A', 0, 25),
            (1, 'B', 0, 45),
            (1, 'C', 0, 65),
            (1, 'H', 0, 5),
            (1, 'X', 0, 85),
        ]
    );
}

#[test]
fn a_heading_stays_with_a_first_row_whose_box_avoiding_breaks_crosses_the_page_end() {
    // The page end falls inside a box that avoids a break inside it, which
    // moves down inside the cell, so the row still breaks after its first
    // line.
    assert_eq!(
        glyphs(
            r#"<div style="height:50px">W</div><div style="break-after:avoid">H</div><table><tr><td><div>A</div><div style="break-inside:avoid;height:40px">C</div><div>B</div></td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 75),
            (0, 'H', 0, 55),
            (0, 'W', 0, 5),
            (1, 'B', 0, 45),
            (1, 'C', 0, 5),
            (1, 'X', 0, 65),
        ]
    );
}

#[test]
fn a_heading_is_not_left_behind_when_a_replaced_box_crosses_the_page_end() {
    // The page end falls inside an image of the first row, which then does
    // not break there, so the heading must not stay on the page without the
    // row's first line. Chromium keeps both on the first page.
    let glyphs = glyphs(
        r#"<div style="height:50px">W</div><div style="break-after:avoid">H</div><table><tr><td><div>A</div><img style="display:block;width:20px;height:40px" src="data:image/gif;base64,R0lGODlhAQABAAAAACw="><div>B</div></td></tr></table><div>X</div>"#,
    );
    let page_of = |ch| {
        glyphs
            .iter()
            .find(|glyph| glyph.1 == ch)
            .map(|glyph| glyph.0)
    };
    assert_eq!(page_of('H'), page_of('A'));
}
