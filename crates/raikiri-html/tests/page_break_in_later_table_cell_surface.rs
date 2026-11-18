//! A break in a table cell after an earlier cell of its row already ran on
//! to the next page. The cells of a row start on the page their row starts
//! on (CSS 2.1 §17.5), so a box in a later cell that crosses the end of that
//! page moves to the next one as in any block flow, and a forced break in it
//! opens the page after the row's first one, not a page after the one the
//! earlier cell reached. Where each glyph falls is what Chromium prints for
//! the same documents on pages 200px wide and 100px tall, with Ahem glyphs
//! of 10px on lines of 20px.

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

/// The first cell: its third box starts the second page.
const RUNS_ON: &str = r#"<div style="height:30px">A</div><div style="height:70px">B</div><div style="height:70px">D</div>"#;

#[test]
fn a_box_in_a_later_cell_breaks_on_the_page_of_its_row() {
    // The box that avoids a break inside it and the forced break move `Y` to
    // the second page alike.
    for second in ["break-inside:avoid", "break-before:page"] {
        assert_eq!(
            glyphs(&format!(
                r#"<table><tr><td>{RUNS_ON}</td><td><div style="height:50px">W</div><div style="height:70px;{second}">Y</div></td></tr></table><div>X</div>"#
            )),
            [
                (0, 'A', 0, 5),
                (0, 'B', 0, 35),
                (0, 'W', 10, 5),
                (1, 'D', 0, 5),
                (1, 'X', 0, 75),
                (1, 'Y', 10, 5)
            ],
            "{second}"
        );
    }
}

#[test]
fn every_later_cell_starts_on_the_page_of_its_row() {
    // A third cell, after the second.
    assert_eq!(
        glyphs(&format!(
            r#"<table><tr><td>{RUNS_ON}</td><td><div style="height:50px">W</div><div style="height:70px;break-inside:avoid">Y</div></td><td><div style="height:60px">V</div><div style="height:70px;break-inside:avoid">Z</div></td></tr></table><div>X</div>"#
        )),
        [
            (0, 'A', 0, 5),
            (0, 'B', 0, 35),
            (0, 'V', 20, 5),
            (0, 'W', 10, 5),
            (1, 'D', 0, 5),
            (1, 'X', 0, 75),
            (1, 'Y', 10, 5),
            (1, 'Z', 20, 5)
        ]
    );
    // A cell that an anonymous cell before it leaves behind.
    assert_eq!(
        glyphs(&format!(
            r#"<div style="display:table">{RUNS_ON}<div style="display:table-cell"><div style="height:50px">W</div><div style="height:70px;break-inside:avoid">Y</div></div></div><div>X</div>"#
        )),
        [
            (0, 'A', 0, 5),
            (0, 'B', 0, 35),
            (0, 'W', 10, 5),
            (1, 'D', 0, 5),
            (1, 'X', 0, 75),
            (1, 'Y', 10, 5)
        ]
    );
    // Cells inside `display: contents` boxes, which put them in the row.
    for (first, second) in [
        ("", "display:contents"),
        ("display:contents", "display:contents"),
    ] {
        assert_eq!(
            glyphs(&format!(
                r#"<div style="display:table"><div style="display:table-row"><div style="{first}"><div style="display:table-cell">{RUNS_ON}</div></div><div style="{second}"><div style="display:table-cell"><div style="height:50px">W</div><div style="height:70px;break-inside:avoid">Y</div></div></div></div></div><div>X</div>"#
            )),
            [
                (0, 'A', 0, 5),
                (0, 'B', 0, 35),
                (0, 'W', 10, 5),
                (1, 'D', 0, 5),
                (1, 'X', 0, 75),
                (1, 'Y', 10, 5)
            ],
            "{first} {second}"
        );
    }
    // A cell beside a nested table whose first cell runs on.
    assert_eq!(
        glyphs(&format!(
            r#"<table><tr><td><table><tr><td>{RUNS_ON}</td><td><div style="height:50px">W</div></td></tr></table></td><td><div style="height:50px">W</div><div style="height:70px;break-inside:avoid">Y</div></td></tr></table><div>X</div>"#
        )),
        [
            (0, 'A', 0, 5),
            (0, 'B', 0, 35),
            (0, 'W', 10, 5),
            (0, 'W', 20, 5),
            (1, 'D', 0, 5),
            (1, 'X', 0, 75),
            (1, 'Y', 20, 5)
        ]
    );
}

#[test]
fn the_flow_after_the_row_goes_on_from_its_last_page() {
    // The moved box grows the row, and the next row follows it.
    assert_eq!(
        glyphs(&format!(
            r#"<table><tr><td>{RUNS_ON}</td><td><div style="height:50px">W</div><div style="height:70px;break-inside:avoid">Y</div></td></tr><tr><td>R</td><td>S</td></tr></table><div>X</div>"#
        )),
        [
            (0, 'A', 0, 5),
            (0, 'B', 0, 35),
            (0, 'W', 10, 5),
            (1, 'D', 0, 5),
            (1, 'R', 0, 75),
            (1, 'S', 10, 75),
            (1, 'Y', 10, 5),
            (2, 'X', 0, 5)
        ]
    );
    // The first cell runs on to the third page, past the moved box; so it
    // does when it comes after the cell with the moved box.
    for (first, second) in [("A", "W"), ("W", "A")] {
        let runs_on = r#"<div style="height:30px">A</div><div style="height:70px">B</div><div style="height:100px">D</div><div style="height:70px">E</div>"#;
        let moved = r#"<div style="height:50px">W</div><div style="height:70px;break-inside:avoid">Y</div>"#;
        let (left, right) = if first == "A" {
            (runs_on, moved)
        } else {
            (moved, runs_on)
        };
        let glyphs = glyphs(&format!(
            r#"<table><tr><td>{left}</td><td>{right}</td></tr><tr><td>R</td></tr></table><div>X</div>"#
        ));
        let (a, w) = if first == "A" { (0, 10) } else { (10, 0) };
        let mut expected = vec![
            (0, 'A', a, 5),
            (0, 'B', a, 35),
            (0, 'W', w, 5),
            (1, 'D', a, 5),
            (1, 'Y', w, 5),
            (2, 'E', a, 5),
            (2, 'R', 0, 75),
            (3, 'X', 0, 5),
        ];
        expected.sort_unstable();
        assert_eq!(glyphs, expected, "{first} {second}");
    }
}

#[test]
fn a_cell_of_a_row_beside_a_spanning_cell_starts_on_its_row_page() {
    // The second row starts on the first page, beside the cell that spans
    // both rows and runs on to the second page.
    let glyphs = glyphs(&format!(
        r#"<table><tr><td rowspan="2">{RUNS_ON}</td><td><div style="height:50px">W</div></td></tr><tr><td><div style="height:70px;break-inside:avoid">Y</div></td></tr></table><div>X</div>"#
    ));
    assert!(glyphs.contains(&(1, 'Y', 10, 5)), "{glyphs:?}");
}
