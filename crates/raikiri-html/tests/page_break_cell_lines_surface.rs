//! The lines of a paragraph in a table cell that cross the page end, or that
//! its orphans and widows move, start the next page, and the cell's row
//! grows to hold them while the cells beside it keep their places. Where
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
fn a_line_crossing_the_page_end_in_a_cell_starts_the_next_page() {
    // The lines of the cell that cross the end of each page start the next one, and the row grows with them.
    assert_eq!(
        glyphs(
            "<div style=\"height:50px\">W</div><table><tr><td>A<br>B<br>C<br>D<br>E<br>F<br>G<br>H</td></tr></table><div>X</div>"
        ),
        [
            (0, 'A', 0, 55),
            (0, 'B', 0, 75),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'D', 0, 25),
            (1, 'E', 0, 45),
            (1, 'F', 0, 65),
            (2, 'G', 0, 5),
            (2, 'H', 0, 25),
            (2, 'X', 0, 45),
        ]
    );
}

#[test]
fn the_widows_of_a_cell_move_a_line_to_the_next_page() {
    // Two widows take the third line to the next page with the fourth.
    assert_eq!(
        glyphs(
            "<div style=\"height:40px\">W</div><table><tr><td>A<br>B<br>C<br>D</td></tr></table><div>X</div>"
        ),
        [
            (0, 'A', 0, 45),
            (0, 'B', 0, 65),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'D', 0, 25),
            (1, 'X', 0, 45),
        ]
    );
}

#[test]
fn lines_of_a_block_in_a_cell_move_down_inside_it() {
    // The block in the cell grows with its moved lines, and so does the row.
    assert_eq!(
        glyphs(
            "<div style=\"height:50px\">W</div><table><tr><td><div>A<br>B<br>C<br>D<br>E<br>F<br>G<br>H</div></td></tr></table><div>X</div>"
        ),
        [
            (0, 'A', 0, 55),
            (0, 'B', 0, 75),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'D', 0, 25),
            (1, 'E', 0, 45),
            (1, 'F', 0, 65),
            (2, 'G', 0, 5),
            (2, 'H', 0, 25),
            (2, 'X', 0, 45),
        ]
    );
}

#[test]
fn the_cells_beside_keep_their_own_lines() {
    // Each cell moves its own lines; the row grows as far as the cell that grows most.
    assert_eq!(
        glyphs(
            "<div style=\"height:50px\">W</div><table><tr><td>A<br>B<br>C<br>D<br>E<br>F<br>G<br>H</td><td>K<br>L<br>M<br>N<br>O<br>P<br>Q<br>R<br>S</td></tr></table><div>X</div>"
        ),
        [
            (0, 'A', 0, 55),
            (0, 'B', 0, 75),
            (0, 'K', 10, 55),
            (0, 'L', 10, 75),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'D', 0, 25),
            (1, 'E', 0, 45),
            (1, 'F', 0, 65),
            (1, 'M', 10, 5),
            (1, 'N', 10, 25),
            (1, 'O', 10, 45),
            (1, 'P', 10, 65),
            (1, 'Q', 10, 85),
            (2, 'G', 0, 5),
            (2, 'H', 0, 25),
            (2, 'R', 10, 5),
            (2, 'S', 10, 25),
            (2, 'X', 0, 45),
        ]
    );
}

#[test]
fn a_middle_aligned_cell_beside_moved_lines_stays_at_the_top() {
    // The cell beside the one that breaks is laid out as if aligned to its top.
    assert_eq!(
        glyphs(
            "<div style=\"height:50px\">W</div><table><tr><td>A<br>B<br>C<br>D<br>E<br>F<br>G<br>H</td><td style=\"vertical-align:middle\">K</td></tr></table><div>X</div>"
        ),
        [
            (0, 'A', 0, 55),
            (0, 'B', 0, 75),
            (0, 'K', 10, 55),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'D', 0, 25),
            (1, 'E', 0, 45),
            (1, 'F', 0, 65),
            (2, 'G', 0, 5),
            (2, 'H', 0, 25),
            (2, 'X', 0, 45),
        ]
    );
}

#[test]
fn the_next_row_follows_the_moved_lines() {
    // The second row starts where the grown first row ends.
    assert_eq!(
        glyphs(
            "<div style=\"height:50px\">W</div><table><tr><td>A<br>B<br>C<br>D<br>E<br>F<br>G<br>H</td></tr><tr><td>I<br>J<br>K<br>L<br>M<br>N</td></tr></table><div>X</div>"
        ),
        [
            (0, 'A', 0, 55),
            (0, 'B', 0, 75),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'D', 0, 25),
            (1, 'E', 0, 45),
            (1, 'F', 0, 65),
            (2, 'G', 0, 5),
            (2, 'H', 0, 25),
            (2, 'I', 0, 45),
            (2, 'J', 0, 65),
            (2, 'K', 0, 85),
            (3, 'L', 0, 5),
            (3, 'M', 0, 25),
            (3, 'N', 0, 45),
            (3, 'X', 0, 65),
        ]
    );
}

#[test]
fn a_spanning_cell_beside_moved_lines_stays() {
    // A cell spanning the next row stays where it is, and the next row follows the grown row.
    assert_eq!(
        glyphs(
            "<div style=\"height:50px\">W</div><table><tr><td>A<br>B<br>C<br>D<br>E<br>F</td><td rowspan=2>K</td></tr><tr><td>Z</td></tr></table><div>X</div>"
        ),
        [
            (0, 'A', 0, 55),
            (0, 'B', 0, 75),
            (0, 'K', 10, 55),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'D', 0, 25),
            (1, 'E', 0, 45),
            (1, 'F', 0, 65),
            (1, 'Z', 0, 85),
            (2, 'X', 0, 5),
        ]
    );
}

#[test]
fn the_boxes_after_the_lines_follow_them() {
    // The block after the moved lines, the next row and the box after the table follow them.
    assert_eq!(
        glyphs(
            "<div style=\"height:50px\">W</div><table><tr><td><div>A<br>B<br>C<br>D</div><div>Y</div></td></tr><tr><td>Z</td></tr></table><div>X</div>"
        ),
        [
            (0, 'A', 0, 55),
            (0, 'B', 0, 75),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'D', 0, 25),
            (1, 'X', 0, 85),
            (1, 'Y', 0, 45),
            (1, 'Z', 0, 65),
        ]
    );
}

#[test]
fn a_row_that_fits_a_page_breaks_between_its_lines() {
    // A row shorter than a page breaks between the lines of its cell rather than moving whole.
    assert_eq!(
        glyphs(
            "<div style=\"height:45px\">W</div><table><tr><td>A<br>B<br>C</td></tr></table><div>X</div>"
        ),
        [
            (0, 'A', 0, 50),
            (0, 'B', 0, 70),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'X', 0, 25),
        ]
    );
}

#[test]
fn lines_after_a_block_keep_their_orphans_on_the_page() {
    // The lines of the second block in the cell break after the orphans they need.
    assert_eq!(
        glyphs(
            "<div style=\"height:30px\">W</div><table><tr><td><div>Q</div><div>A<br>B<br>C</div></td></tr></table><div>X</div>"
        ),
        [
            (0, 'A', 0, 55),
            (0, 'B', 0, 75),
            (0, 'Q', 0, 35),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'X', 0, 25),
        ]
    );
}

#[test]
fn the_padding_of_a_cell_follows_its_last_line() {
    // The cell's bottom padding comes after its last moved line.
    assert_eq!(
        glyphs(
            "<div style=\"height:50px\">W</div><table><tr><td style=\"padding:5px 0\">A<br>B<br>C<br>D<br>E<br>F<br>G<br>H</td></tr></table><div>X</div>"
        ),
        [
            (0, 'A', 0, 60),
            (0, 'B', 0, 80),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'D', 0, 25),
            (1, 'E', 0, 45),
            (1, 'F', 0, 65),
            (2, 'G', 0, 5),
            (2, 'H', 0, 25),
            (2, 'X', 0, 50),
        ]
    );
}

#[test]
fn a_cell_height_keeps_the_row_from_growing() {
    // A cell taller than its moved lines does not grow the row.
    assert_eq!(
        glyphs(
            "<div style=\"height:50px\">W</div><table><tr><td style=\"height:80px\">A<br>B<br>C</td></tr></table><div>X</div>"
        ),
        [
            (0, 'A', 0, 55),
            (0, 'B', 0, 75),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'X', 0, 35),
        ]
    );
}

#[test]
fn a_block_height_keeps_the_row_from_growing() {
    // A block with a set height holds its moved lines as overflow; the row does not grow.
    assert_eq!(
        glyphs(
            "<div style=\"height:50px\">W</div><table><tr><td><div style=\"height:70px\">A<br>B<br>C</div></td></tr></table><div>X</div>"
        ),
        [
            (0, 'A', 0, 55),
            (0, 'B', 0, 75),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'X', 0, 25),
        ]
    );
}

#[test]
fn a_middle_aligned_first_cell_does_not_hold_the_row() {
    // A middle-aligned cell before the one whose lines move stays on the first page with the row.
    assert_eq!(
        glyphs(
            "<div style=\"height:50px\">W</div><table><tr><td style=\"vertical-align:middle\">K</td><td>A<br>B<br>C<br>D<br>E<br>F<br>G<br>H</td></tr></table><div>X</div>"
        ),
        [
            (0, 'A', 10, 55),
            (0, 'B', 10, 75),
            (0, 'K', 0, 55),
            (0, 'W', 0, 5),
            (1, 'C', 10, 5),
            (1, 'D', 10, 25),
            (1, 'E', 10, 45),
            (1, 'F', 10, 65),
            (2, 'G', 10, 5),
            (2, 'H', 10, 25),
            (2, 'X', 0, 45),
        ]
    );
}

#[test]
fn a_spanning_first_cell_does_not_hold_the_row() {
    // A spanning cell before the one whose lines move stays where it is.
    assert_eq!(
        glyphs(
            "<div style=\"height:50px\">W</div><table><tr><td rowspan=2>K</td><td>A<br>B<br>C<br>D<br>E<br>F</td></tr><tr><td>Z</td></tr></table><div>X</div>"
        ),
        [
            (0, 'A', 10, 55),
            (0, 'B', 10, 75),
            (0, 'K', 0, 55),
            (0, 'W', 0, 5),
            (1, 'C', 10, 5),
            (1, 'D', 10, 25),
            (1, 'E', 10, 45),
            (1, 'F', 10, 65),
            (1, 'Z', 10, 85),
            (2, 'X', 0, 5),
        ]
    );
}

#[test]
fn a_block_after_a_block_in_a_cell_starts_the_next_page_with_its_lines() {
    // Lines of a later block that all cross the page end move down together.
    assert_eq!(
        glyphs(
            "<div style=\"height:50px\">W</div><table><tr><td><div style=\"height:40px\">Q</div><div>A<br>B<br>C</div></td></tr></table><div>X</div>"
        ),
        [
            (0, 'Q', 0, 55),
            (0, 'W', 0, 5),
            (1, 'A', 0, 5),
            (1, 'B', 0, 25),
            (1, 'C', 0, 45),
            (1, 'X', 0, 65),
        ]
    );
}

#[test]
fn a_float_in_the_cell_stays_beside_the_first_lines() {
    // A float beside the first lines stays on the first page; the later lines move.
    assert_eq!(
        glyphs(
            "<div style=\"height:50px\">W</div><table><tr><td><div style=\"float:right;width:10px\">F</div>A<br>B<br>C<br>D</td></tr></table><div>X</div>"
        ),
        [
            (0, 'A', 0, 55),
            (0, 'B', 0, 75),
            (0, 'F', 10, 55),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'D', 0, 25),
            (1, 'X', 0, 45),
        ]
    );
}

#[test]
fn a_full_width_table_breaks_its_row_between_lines() {
    // The cells of a percentage-width table move their lines down too.
    assert_eq!(
        glyphs(
            "<div style=\"height:50px\">W</div><table style=\"width:100%\"><tr><td>A<br>B<br>C<br>D</td><td>K</td></tr><tr><td>Y</td><td>Z</td></tr></table><div>X</div>"
        ),
        [
            (0, 'A', 0, 55),
            (0, 'B', 0, 75),
            (0, 'K', 100, 55),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'D', 0, 25),
            (1, 'X', 0, 65),
            (1, 'Y', 0, 45),
            (1, 'Z', 100, 45),
        ]
    );
}

#[test]
fn too_few_orphans_move_all_lines_of_a_later_block_down() {
    // One line before the page end is too few for the orphans: all the lines of the second block start the next page, and the first block stays.
    assert_eq!(
        glyphs(
            "<div style=\"height:50px\">W</div><table><tr><td><div>Q</div><div>A<br>B<br>C</div></td></tr></table><div>X</div>"
        ),
        [
            (0, 'Q', 0, 55),
            (0, 'W', 0, 5),
            (1, 'A', 0, 5),
            (1, 'B', 0, 25),
            (1, 'C', 0, 45),
            (1, 'X', 0, 65),
        ]
    );
}

#[test]
fn a_block_after_one_that_ends_at_the_page_end_starts_the_next_page() {
    // The second block of the cell starts the next page where the first one ends.
    assert_eq!(
        glyphs(
            "<div style=\"height:40px\">W</div><table><tr><td><div>A<br>B<br>C</div><div>D<br>E<br>F</div></td></tr></table><div>X</div>"
        ),
        [
            (0, 'A', 0, 45),
            (0, 'B', 0, 65),
            (0, 'C', 0, 85),
            (0, 'W', 0, 5),
            (1, 'D', 0, 5),
            (1, 'E', 0, 25),
            (1, 'F', 0, 45),
            (1, 'X', 0, 65),
        ]
    );
}

#[test]
fn a_forced_break_after_moved_lines_starts_another_page() {
    // A forced break later in the cell starts the page after the moved lines, and the middle-aligned cell beside stays at the top.
    assert_eq!(
        glyphs(
            "<div style=\"height:50px\">W</div><table><tr><td>A<br>B<br>C<br>D<br>E<br>F<br>G<div style=\"break-before:page\">Z</div></td><td style=\"vertical-align:middle\">K</td></tr></table><div>X</div>"
        ),
        [
            (0, 'A', 0, 55),
            (0, 'B', 0, 75),
            (0, 'K', 10, 55),
            (0, 'W', 0, 5),
            (1, 'C', 0, 5),
            (1, 'D', 0, 25),
            (1, 'E', 0, 45),
            (1, 'F', 0, 65),
            (1, 'G', 0, 85),
            (2, 'X', 0, 25),
            (2, 'Z', 0, 5),
        ]
    );
}
