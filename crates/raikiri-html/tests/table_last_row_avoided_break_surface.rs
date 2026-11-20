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

#[test]
fn a_table_that_avoids_breaks_inside_it_moves_whole() {
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:50px">S</div><table style="break-inside:avoid">{ROWS}</table><div style="break-before:avoid">Z</div>"#
        )),
        [(0, 'S', 0), (1, 'L', 0), (1, 'X', 20), (1, 'Z', 40)]
    );
}

#[test]
fn a_table_of_one_row_ties_the_boxes_on_both_sides() {
    // The box before the table, its row and the box after it move together.
    assert_eq!(
        glyphs(
            r#"<div style="height:50px">S</div><div style="break-after:avoid">B</div><table><tr><td>X</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [(0, 'S', 0), (1, 'B', 0), (1, 'X', 20), (1, 'Z', 40)]
    );
}

#[test]
fn a_forced_break_inside_the_last_row_group_keeps_the_rows_before_it() {
    assert_eq!(
        glyphs(
            r#"<div style="height:30px">S</div><table><tbody><tr><td>K</td></tr></tbody><tbody style="break-inside:avoid"><tr style="break-after:page"><td>A</td></tr><tr><td>B</td></tr></tbody></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'A', 50),
            (0, 'K', 30),
            (0, 'S', 0),
            (1, 'B', 0),
            (1, 'Z', 20)
        ]
    );
}

#[test]
fn an_empty_last_row_is_the_part_that_moves() {
    // The empty row ends where the row before it does, and moves alone with
    // the box after it.
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:50px">S</div><table>{ROWS}<tr></tr></table><div style="break-before:avoid">Z</div>"#
        )),
        [(0, 'L', 50), (0, 'S', 0), (0, 'X', 70), (1, 'Z', 0)]
    );
}

#[test]
fn the_last_row_of_an_anonymous_table_moves_with_the_box_after_it() {
    assert_eq!(
        glyphs(
            r#"<div style="height:50px">S</div><div style="display:table-row"><div style="display:table-cell">L</div></div><div style="display:table-row"><div style="display:table-cell">X</div></div><div style="break-before:avoid">Z</div>"#
        ),
        [(0, 'L', 50), (0, 'S', 0), (1, 'X', 0), (1, 'Z', 20)]
    );
}

#[test]
fn tables_inside_a_block_move_as_they_do_in_the_body() {
    // One that avoids breaks inside it moves whole.
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:50px">S</div><div><table style="break-inside:avoid">{ROWS}</table><div style="break-before:avoid">Z</div></div>"#
        )),
        [(0, 'S', 0), (1, 'L', 0), (1, 'X', 20), (1, 'Z', 40)]
    );
    // One of one row ties the boxes on both sides.
    assert_eq!(
        glyphs(
            r#"<div style="height:50px">S</div><div><div style="break-after:avoid">B</div><table><tr><td>X</td></tr></table><div style="break-before:avoid">Z</div></div>"#
        ),
        [(0, 'S', 0), (1, 'B', 0), (1, 'X', 20), (1, 'Z', 40)]
    );
    // Rows in a row group, after a column group: the last row moves.
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:50px">S</div><div><table><colgroup><col></colgroup><tbody>{ROWS}</tbody></table><div style="break-before:avoid">Z</div></div>"#
        )),
        [(0, 'L', 50), (0, 'S', 0), (1, 'X', 0), (1, 'Z', 20)]
    );
    // A caption below the rows moves alone.
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:50px">S</div><div><table style="caption-side:bottom"><caption>C</caption>{ROWS}</table><div style="break-before:avoid">Z</div></div>"#
        )),
        [
            (0, 'L', 50),
            (0, 'S', 0),
            (0, 'X', 70),
            (1, 'C', 0),
            (1, 'Z', 20)
        ]
    );
    // Rows that are children of the table box itself.
    assert_eq!(
        glyphs(
            r#"<div style="height:50px">S</div><div><div style="display:table"><div style="display:table-row"><div style="display:table-cell">L</div></div><div style="display:table-row"><div style="display:table-cell">X</div></div></div><div style="break-before:avoid">Z</div></div>"#
        ),
        [(0, 'L', 50), (0, 'S', 0), (1, 'X', 0), (1, 'Z', 20)]
    );
    // One with a footer group that fits stays.
    assert_eq!(
        glyphs(
            r#"<div style="height:10px">S</div><div><table><tr><td>L</td></tr><tfoot><tr><td>F</td></tr></tfoot></table><div style="break-before:avoid">Z</div></div>"#
        ),
        [(0, 'F', 30), (0, 'L', 10), (0, 'S', 0), (0, 'Z', 50)]
    );
}

#[test]
fn a_last_row_that_crosses_the_page_end_breaks_inside_it() {
    let rows = "<tr><td>L</td></tr><tr><td>X<br>X<br>X</td></tr>";
    let expected = [
        (0, 'L', 30),
        (0, 'S', 0),
        (0, 'X', 50),
        (0, 'X', 70),
        (1, 'X', 0),
        (1, 'Z', 20),
    ];
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:30px">S</div><table>{rows}</table><div style="break-before:avoid">Z</div>"#
        )),
        expected
    );
    assert_eq!(
        glyphs(&format!(
            r#"<div><div style="height:30px">S</div><table>{rows}</table><div style="break-before:avoid">Z</div></div>"#
        )),
        expected
    );
}

#[test]
fn a_last_row_of_several_lines_that_fits_moves_whole() {
    assert_eq!(
        glyphs(
            r#"<div style="height:20px">S</div><table><tr><td>L</td></tr><tr><td>X<br>Y<br>W</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'L', 20),
            (0, 'S', 0),
            (1, 'W', 40),
            (1, 'X', 0),
            (1, 'Y', 20),
            (1, 'Z', 60)
        ]
    );
}

#[test]
fn a_box_after_the_last_row_too_tall_for_a_page_breaks_inside_on_the_next() {
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:30px">S</div><table>{ROWS}</table><div style="break-before:avoid">Z<br>Z<br>Z<br>Z<br>Z<br>Z</div>"#
        )),
        [
            (0, 'L', 30),
            (0, 'S', 0),
            (1, 'X', 0),
            (1, 'Z', 20),
            (1, 'Z', 40),
            (1, 'Z', 60),
            (1, 'Z', 80),
            (2, 'Z', 0),
            (2, 'Z', 20)
        ]
    );
}

#[test]
fn a_break_forced_after_the_last_row_keeps_the_table_on_its_page() {
    let expected = [(0, 'L', 50), (0, 'S', 0), (0, 'X', 70), (1, 'Z', 0)];
    for rows in [
        r#"<tr><td>L</td></tr><tr style="break-after:page"><td>X</td></tr>"#,
        r#"<tr><td>L</td></tr><tr><td><div style="break-after:page">X</div></td></tr>"#,
        r#"<tr><td>L</td></tr><tbody style="break-after:page"><tr><td>X</td></tr></tbody>"#,
    ] {
        assert_eq!(
            glyphs(&format!(
                r#"<div style="height:50px">S</div><table>{rows}</table><div style="break-before:avoid">Z</div>"#
            )),
            expected,
            "{rows}"
        );
    }
    assert_eq!(
        glyphs(
            r#"<div><div style="height:50px">S</div><table><tr><td>L</td></tr><tr style="break-after:page"><td>X</td></tr></table><div style="break-before:avoid">Z</div></div>"#
        ),
        expected
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:50px">S</div><table><tr style="break-after:page"><td>X</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [(0, 'S', 0), (0, 'X', 50), (1, 'Z', 0)]
    );
}

#[test]
fn a_table_that_avoids_breaks_inside_it_moves_only_after_a_forced_break() {
    let expected = [(0, 'L', 50), (0, 'S', 0), (1, 'X', 0), (1, 'Z', 20)];
    let table = r#"<table style="break-inside:avoid"><tr style="break-after:page"><td>L</td></tr><tr><td>X</td></tr></table>"#;
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:50px">S</div>{table}<div style="break-before:avoid">Z</div>"#
        )),
        expected
    );
    assert_eq!(
        glyphs(&format!(
            r#"<div><div style="height:50px">S</div>{table}<div style="break-before:avoid">Z</div></div>"#
        )),
        expected
    );
}

#[test]
fn a_break_forced_between_row_groups_of_a_table_that_avoids_breaks_inside_it_counts() {
    let expected = [(0, 'L', 50), (0, 'S', 0), (1, 'X', 0), (1, 'Z', 20)];
    for groups in [
        r#"<tbody style="break-after:page"><tr><td>L</td></tr></tbody><tbody><tr><td>X</td></tr></tbody>"#,
        r#"<tbody><tr><td>L</td></tr></tbody><tbody style="break-before:page"><tr><td>X</td></tr></tbody>"#,
    ] {
        assert_eq!(
            glyphs(&format!(
                r#"<div style="height:50px">S</div><table style="break-inside:avoid">{groups}</table><div style="break-before:avoid">Z</div>"#
            )),
            expected,
            "{groups}"
        );
    }
}

#[test]
fn a_table_that_moves_whole_starts_the_run_inside_a_block() {
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:30px">S</div><div><div>K</div><table style="break-inside:avoid">{ROWS}</table></div><div style="break-before:avoid">Z</div>"#
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
fn rows_tied_to_the_last_row_by_avoided_breaks_move_with_it() {
    let cases = [
        // The last row group avoids breaks inside it and the break before it.
        (
            r#"<tbody><tr><td>K</td></tr><tr><td>M</td></tr></tbody><tbody style="break-inside:avoid;break-before:avoid"><tr><td>L</td></tr><tr><td>X</td></tr></tbody>"#,
            &[
                (0, 'K', 30),
                (0, 'S', 0),
                (1, 'L', 20),
                (1, 'M', 0),
                (1, 'X', 40),
                (1, 'Z', 60),
            ][..],
        ),
        // The row before the last row group avoids the break after it.
        (
            r#"<tbody><tr><td>K</td></tr><tr style="break-after:avoid"><td>M</td></tr></tbody><tbody style="break-inside:avoid"><tr><td>L</td></tr><tr><td>X</td></tr></tbody>"#,
            &[
                (0, 'K', 30),
                (0, 'S', 0),
                (1, 'L', 20),
                (1, 'M', 0),
                (1, 'X', 40),
                (1, 'Z', 60),
            ][..],
        ),
        // A row inside the last row group avoids the break after it.
        (
            r#"<tbody><tr><td>K</td></tr></tbody><tbody style="break-inside:avoid"><tr style="break-after:avoid"><td>L</td></tr><tr><td>X</td></tr></tbody>"#,
            &[
                (0, 'K', 30),
                (0, 'S', 0),
                (1, 'L', 0),
                (1, 'X', 20),
                (1, 'Z', 40),
            ][..],
        ),
        // Rows without groups.
        (
            r#"<tr><td>K</td></tr><tr style="break-after:avoid"><td>M</td></tr><tr><td>X</td></tr>"#,
            &[
                (0, 'K', 30),
                (0, 'S', 0),
                (1, 'M', 0),
                (1, 'X', 20),
                (1, 'Z', 40),
            ][..],
        ),
        // A last row group that avoids the break before it.
        (
            r#"<tbody><tr><td>K</td></tr><tr><td>M</td></tr></tbody><tbody style="break-before:avoid"><tr><td>X</td></tr></tbody>"#,
            &[
                (0, 'K', 30),
                (0, 'S', 0),
                (1, 'M', 0),
                (1, 'X', 20),
                (1, 'Z', 40),
            ][..],
        ),
    ];
    for (rows, expected) in cases {
        assert_eq!(
            glyphs(&format!(
                r#"<div style="height:30px">S</div><table>{rows}</table><div style="break-before:avoid">Z</div>"#
            )),
            expected,
            "{rows}"
        );
    }
    // A chain of two avoided breaks.
    assert_eq!(
        glyphs(
            r#"<div style="height:10px">S</div><table><tr><td>K</td></tr><tr><td>J</td></tr><tr style="break-before:avoid"><td>M</td></tr><tr style="break-before:avoid"><td>X</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [
            (0, 'K', 10),
            (0, 'S', 0),
            (1, 'J', 0),
            (1, 'M', 20),
            (1, 'X', 40),
            (1, 'Z', 60)
        ]
    );
}

#[test]
fn a_table_whose_rows_are_all_tied_moves_whole() {
    let expected = [(0, 'S', 0), (1, 'L', 0), (1, 'X', 20), (1, 'Z', 40)];
    for rows in [
        r#"<tbody><tr><td>L</td></tr></tbody><tbody style="break-inside:avoid;break-before:avoid"><tr><td>X</td></tr></tbody>"#,
        r#"<tbody style="break-after:avoid"><tr><td>L</td></tr></tbody><tbody style="break-inside:avoid"><tr><td>X</td></tr></tbody>"#,
    ] {
        assert_eq!(
            glyphs(&format!(
                r#"<div style="height:50px">S</div><table>{rows}</table><div style="break-before:avoid">Z</div>"#
            )),
            expected,
            "{rows}"
        );
    }
}

#[test]
fn rows_of_an_anonymous_table_tied_to_its_last_row_move_with_it() {
    let row = "display:table-row";
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:30px">S</div><div style="{row}">K</div><div style="{row};break-after:avoid">M</div><div style="{row}">X</div><div style="break-before:avoid">Z</div>"#
        )),
        [
            (0, 'K', 30),
            (0, 'S', 0),
            (1, 'M', 0),
            (1, 'X', 20),
            (1, 'Z', 40)
        ]
    );
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:10px">S</div><div style="{row}">K</div><div style="{row}">J</div><div style="{row};break-before:avoid">M</div><div style="{row};break-before:avoid">X</div><div style="break-before:avoid">Z</div>"#
        )),
        [
            (0, 'K', 10),
            (0, 'S', 0),
            (1, 'J', 0),
            (1, 'M', 20),
            (1, 'X', 40),
            (1, 'Z', 60)
        ]
    );
    // All its rows are tied: the table moves whole.
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:50px">S</div><div style="{row};break-after:avoid">M</div><div style="{row}">X</div><div style="break-before:avoid">Z</div>"#
        )),
        [(0, 'S', 0), (1, 'M', 0), (1, 'X', 20), (1, 'Z', 40)]
    );
}

#[test]
fn a_caption_below_the_rows_tied_to_them_moves_with_the_rows_tied_to_the_last() {
    let caption = r#"<caption style="caption-side:bottom;break-before:avoid">C</caption>"#;
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:30px">S</div><table>{caption}<tr><td>K</td></tr><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        )),
        [
            (0, 'K', 30),
            (0, 'S', 0),
            (1, 'C', 20),
            (1, 'L', 0),
            (1, 'Z', 40)
        ]
    );
    // With one row, the whole table moves.
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:50px">S</div><table>{caption}<tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        )),
        [(0, 'S', 0), (1, 'C', 20), (1, 'L', 0), (1, 'Z', 40)]
    );
}

#[test]
fn a_break_forced_after_a_caption_below_the_rows_keeps_the_table_on_its_page() {
    assert_eq!(
        glyphs(
            r#"<div style="height:50px">S</div><table><caption style="caption-side:bottom;break-after:page">C</caption><tr><td>L</td></tr></table><div style="break-before:avoid">Z</div>"#
        ),
        [(0, 'C', 70), (0, 'L', 50), (0, 'S', 0), (1, 'Z', 0)]
    );
}
