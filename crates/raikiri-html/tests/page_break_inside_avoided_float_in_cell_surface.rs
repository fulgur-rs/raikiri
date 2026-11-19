//! A float in a table cell that avoids a break inside it and moves to the
//! next page. The cell grows to hold the float (CSS 2.1 §10.6.7), and the
//! row grows with it only as far as the cell's new content reaches below
//! the row: the room that the vertical alignment of a cell leaves in its
//! padding is not content, a taller cell beside it keeps the row's height,
//! and the cells beside it stay where they are. Where each glyph falls is
//! what Chromium prints for the same documents on pages 200px wide and
//! 100px tall, with Ahem glyphs of 10px on lines of 20px.

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

/// The first cell: 50px of the first page, then a float 70px tall that
/// avoids a break inside it, so it moves to the second page.
const MOVED: &str = r#"<td><div style="height:50px">W</div><div style="float:left;height:70px;break-inside:avoid">Y</div></td>"#;

#[test]
fn a_taller_cell_beside_the_float_keeps_the_row_height() {
    // The row ends at 170px whichever cell comes first, with the float
    // nested in a block too.
    for row in [
        format!(r#"{MOVED}<td><div style="height:170px">D</div></td>"#),
        r#"<td><div style="height:50px">W</div><div><div style="float:left;height:70px;break-inside:avoid">Y</div></div></td><td><div style="height:170px">D</div></td>"#.to_string(),
        // A cell aligned to the middle or the bottom of the row.
        r#"<td style="vertical-align:middle"><div style="height:50px">W</div><div style="float:left;height:70px;break-inside:avoid">Y</div></td><td><div style="height:170px">D</div></td>"#.to_string(),
        r#"<td style="vertical-align:bottom"><div style="height:50px">W</div><div style="float:left;height:70px;break-inside:avoid">Y</div></td><td><div style="height:170px">D</div></td>"#.to_string(),
    ] {
        assert_eq!(
            glyphs(&format!("<table><tr>{row}</tr></table><div>X</div>")),
            [
                (0, 'D', 10, 5),
                (0, 'W', 0, 5),
                (1, 'X', 0, 75),
                (1, 'Y', 0, 5)
            ],
            "{row}"
        );
    }
    assert_eq!(
        glyphs(
            r#"<table><tr><td><div style="height:170px">D</div></td><td><div style="height:50px">W</div><div style="float:left;height:70px;break-inside:avoid">Y</div></td></tr></table><div>X</div>"#
        ),
        [
            (0, 'D', 0, 5),
            (0, 'W', 10, 5),
            (1, 'X', 0, 75),
            (1, 'Y', 10, 5)
        ]
    );
}

#[test]
fn a_shorter_cell_beside_the_float_stays_where_it_is() {
    // The row grows to the float's new bottom; the cell beside it keeps
    // its content at the top of the row.
    for beside in [60, 140] {
        assert_eq!(
            glyphs(&format!(
                r#"<table><tr>{MOVED}<td><div style="height:{beside}px">D</div></td></tr></table><div>X</div>"#
            )),
            [
                (0, 'D', 10, 5),
                (0, 'W', 0, 5),
                (1, 'X', 0, 75),
                (1, 'Y', 0, 5)
            ],
            "cell beside {beside}px tall"
        );
    }
    // Cells in an anonymous row.
    assert_eq!(
        glyphs(
            r#"<div style="display:table"><div style="display:table-cell"><div style="height:50px">W</div><div style="float:left;height:70px;break-inside:avoid">Y</div></div><div style="display:table-cell"><div style="height:60px">D</div></div></div><div>X</div>"#
        ),
        [
            (0, 'D', 10, 5),
            (0, 'W', 0, 5),
            (1, 'X', 0, 75),
            (1, 'Y', 0, 5)
        ]
    );
    // The cell's own padding stays below the float.
    assert_eq!(
        glyphs(
            r#"<table><tr><td style="padding-bottom:10px"><div style="height:50px">W</div><div style="float:left;height:70px;break-inside:avoid">Y</div></td><td><div style="height:150px">D</div></td></tr></table><div>X</div>"#
        ),
        [
            (0, 'D', 10, 5),
            (0, 'W', 0, 5),
            (1, 'X', 0, 85),
            (1, 'Y', 0, 5)
        ]
    );
}

#[test]
fn the_next_row_follows_the_row() {
    assert_eq!(
        glyphs(&format!(
            r#"<table><tr>{MOVED}<td><div style="height:170px">D</div></td></tr><tr><td>R</td></tr></table><div>X</div>"#
        )),
        [
            (0, 'D', 10, 5),
            (0, 'W', 0, 5),
            (1, 'R', 0, 75),
            (1, 'Y', 0, 5),
            (2, 'X', 0, 5)
        ]
    );
}

#[test]
fn floats_that_move_in_two_cells_grow_the_row_once() {
    // The row grows as far as the cell that grows most.
    for (z, x) in [(70, (1, 75)), (90, (2, 5))] {
        let mut expected = vec![
            (0, 'V', 10, 5),
            (0, 'W', 0, 5),
            (x.0, 'X', 0, x.1),
            (1, 'Y', 0, 5),
            (1, 'Z', 10, 5),
        ];
        expected.sort_unstable();
        assert_eq!(
            glyphs(&format!(
                r#"<table><tr>{MOVED}<td><div style="height:50px">V</div><div style="float:left;height:{z}px;break-inside:avoid">Z</div></td></tr></table><div>X</div>"#
            )),
            expected,
            "second float {z}px tall"
        );
    }
}

#[test]
fn a_cell_with_a_height_grows_too() {
    // A cell's `height` is a minimum (CSS 2.1 §17.5.3).
    assert_eq!(
        glyphs(
            r#"<table><tr><td style="height:120px"><div style="height:50px">W</div><div style="float:left;height:70px;break-inside:avoid">Y</div></td><td><div style="height:60px">D</div></td></tr></table><div>X</div>"#
        ),
        [
            (0, 'D', 10, 5),
            (0, 'W', 0, 5),
            (1, 'X', 0, 75),
            (1, 'Y', 0, 5)
        ]
    );
}

#[test]
fn floats_that_move_one_after_another_in_a_cell_grow_the_row_to_the_last() {
    // The second float clears the first, which moved, and moves on to the
    // third page; a cell beside them that ends below both keeps the row's
    // end there.
    for beside in [60, 250] {
        assert_eq!(
            glyphs(&format!(
                r#"<table><tr><td><div style="height:50px">W</div><div style="float:left;height:70px;break-inside:avoid">Y</div><div style="float:left;clear:left;height:50px;break-inside:avoid">Z</div></td><td><div style="height:{beside}px">D</div></td></tr></table><div>X</div>"#
            )),
            [
                (0, 'D', 10, 5),
                (0, 'W', 0, 5),
                (1, 'Y', 0, 5),
                (2, 'X', 0, 55),
                (2, 'Z', 0, 5)
            ],
            "cell beside {beside}px tall"
        );
    }
}

#[test]
fn a_row_that_starts_down_the_page_breaks_where_the_float_moves() {
    // The row starts below other content and the float alone crosses the
    // page end: the row breaks there, its first fragment keeps the content
    // above the float and the cell beside it, and the float starts the next
    // page.
    // The cell's bottom padding goes below the float.
    for (cell, beside, end) in [
        ("<td>", "<td>D</td>", 45),
        ("<td>", r#"<td style="vertical-align:bottom">D</td>"#, 45),
        (r#"<td style="padding-bottom:10px">"#, "<td>D</td>", 55),
    ] {
        assert_eq!(
            glyphs(&format!(
                r#"<div style="height:30px">H</div><table><tr>{cell}<div style="height:40px">W</div><div style="float:left;width:20px;height:40px;break-inside:avoid">F</div></td>{beside}</tr></table><div>X</div>"#
            )),
            [
                (0, 'D', 20, 35),
                (0, 'H', 0, 5),
                (0, 'W', 0, 35),
                (1, 'F', 0, 5),
                (1, 'X', 0, end),
            ],
            "{cell}{beside}"
        );
    }
}

#[test]
fn a_row_down_the_page_breaks_for_floats_in_wrappers_and_rows() {
    // The float sits in a wrapper, in an anonymous table, or among later
    // floats and boxes that do not clear it.
    for (body, expected) in [
        (
            r#"<table><tr><td><div style="height:40px">W</div><div><div style="float:left;width:20px;height:40px;break-inside:avoid">F</div></div></td><td>D</td></tr></table>"#,
            vec![(0, 'D', 20, 35), (1, 'F', 0, 5)],
        ),
        (
            r#"<div style="display:table-row"><div style="display:table-cell"><div style="height:40px">W</div><div style="float:left;width:20px;height:40px;break-inside:avoid">F</div></div><div style="display:table-cell;vertical-align:top">D</div></div>"#,
            vec![(0, 'D', 20, 35), (1, 'F', 0, 5)],
        ),
        (
            r#"<table><tr><td><div style="height:40px">W</div><div style="float:left;width:20px;height:40px;break-inside:avoid">F</div><div style="float:left;width:20px;height:40px;break-inside:avoid">G</div></td><td>D</td></tr></table>"#,
            vec![(0, 'D', 40, 35), (1, 'F', 0, 5), (1, 'G', 20, 5)],
        ),
        (
            r#"<table><tr><td><div style="height:40px">W</div><div style="float:left;display:flex;width:20px;height:40px;break-inside:avoid">F</div></td><td>D</td></tr></table>"#,
            vec![(0, 'D', 20, 35), (1, 'F', 0, 5)],
        ),
        (
            r#"<table><tr><td style="width:60px"><div style="height:40px">W</div><div style="float:right;width:20px;height:40px;break-inside:avoid">F</div></td><td>D</td></tr></table>"#,
            vec![(0, 'D', 60, 35), (1, 'F', 40, 5)],
        ),
    ] {
        let mut expected = expected;
        expected.extend([(0, 'H', 0, 5), (0, 'W', 0, 35), (1, 'X', 0, 45)]);
        expected.sort_unstable();
        assert_eq!(
            glyphs(&format!(
                r#"<div style="height:30px">H</div>{body}<div>X</div>"#
            )),
            expected,
            "{body}"
        );
    }
}

#[test]
fn a_box_after_the_float_stays_before_the_break() {
    // A box after the float that does not clear it stays on the first page,
    // and a box that clears the float only from inside an independent
    // formatting context still lets the row break.
    assert_eq!(
        glyphs(
            r#"<div style="height:30px">H</div><table><tr><td><div style="height:40px">W</div><div style="float:left;width:20px;height:40px;break-inside:avoid">F</div><div style="height:20px">Y</div></td><td>D</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'D', 20, 35),
            (0, 'H', 0, 5),
            (0, 'W', 0, 35),
            (0, 'Y', 0, 75),
            (1, 'F', 0, 5),
            (1, 'X', 0, 45),
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:30px">H</div><table><tr><td><div style="height:40px">W</div><div style="display:flow-root"><div style="float:left;width:20px;height:40px;break-inside:avoid">F</div></div><div style="clear:left">Y</div></td><td>D</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'D', 20, 35),
            (0, 'H', 0, 5),
            (0, 'W', 0, 35),
            (1, 'F', 0, 5),
            (1, 'X', 0, 65),
            (1, 'Y', 0, 45),
        ]
    );
}

#[test]
fn a_row_down_the_page_with_a_following_row_breaks_once() {
    // The next row follows the grown row on the next page.
    assert_eq!(
        glyphs(
            r#"<div style="height:30px">H</div><table><tr><td><div style="height:40px">W</div><div style="float:left;width:20px;height:40px;break-inside:avoid">F</div></td><td>D</td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'D', 20, 35),
            (0, 'H', 0, 5),
            (0, 'W', 0, 35),
            (1, 'F', 0, 5),
            (1, 'R', 0, 45),
            (1, 'X', 0, 65),
        ]
    );
}
