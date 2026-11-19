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
