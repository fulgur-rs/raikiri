//! A page break inside a table cell whose content is laid out as lines: a
//! cell that starts with text, an anonymous cell that wraps the boxes of a
//! row (CSS 2.1 §17.2.1), or text that a forced break moves. As for a cell
//! that holds only blocks, the row grows to hold the moved content, and the
//! cells beside it keep their places (CSS 2.1 §17.5.3). Where each glyph
//! falls is what Chromium prints for the same documents on pages 200px wide
//! and 100px tall, with Ahem glyphs of 10px on lines of 20px.

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
fn a_break_in_a_cell_that_starts_with_text_leaves_the_cell_beside_it() {
    // The forced break and the box that avoids a break inside it move `B` to
    // the next page alike.
    for second in ["break-before:page", "break-inside:avoid"] {
        assert_eq!(
            glyphs(&format!(
                r#"<table><tr><td>T<div style="height:30px">A</div><div style="height:70px;{second}">B</div></td><td>C</td></tr></table><div>X</div>"#
            )),
            [
                (0, 'A', 0, 25),
                (0, 'C', 10, 5),
                (0, 'T', 0, 5),
                (1, 'B', 0, 5),
                (1, 'X', 0, 75)
            ],
            "{second}"
        );
    }
}

#[test]
fn a_break_in_an_anonymous_cell_leaves_the_cell_beside_it() {
    assert_eq!(
        glyphs(
            r#"<div style="display:table"><div style="display:table-row"><div style="height:50px">A</div><div style="height:70px;break-before:page">B</div><div style="display:table-cell">C</div></div><div style="display:table-row"><div style="display:table-cell">R</div></div></div><div>X</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (0, 'C', 10, 5),
            (1, 'B', 0, 5),
            (1, 'R', 0, 75),
            (2, 'X', 0, 5)
        ]
    );
    assert_eq!(
        glyphs(
            r#"<div style="display:table"><div style="display:table-row">T<div style="height:50px">A</div><div style="height:70px;break-inside:avoid">B</div><div style="display:table-cell">C</div></div><div style="display:table-row"><div style="display:table-cell">R</div></div></div><div>X</div>"#
        ),
        [
            (0, 'A', 0, 25),
            (0, 'C', 10, 5),
            (0, 'T', 0, 5),
            (1, 'B', 0, 5),
            (1, 'R', 0, 75),
            (2, 'X', 0, 5)
        ]
    );
}

#[test]
fn the_content_after_the_moved_box_in_an_anonymous_cell_follows_it() {
    // A block after it, and the cell beside them.
    assert_eq!(
        glyphs(
            r#"<div style="display:table"><div style="display:table-row"><div style="height:30px">A</div><div style="height:70px;break-before:page">B</div><div>D</div><div style="display:table-cell">C</div></div><div style="display:table-row"><div style="display:table-cell">R</div></div></div><div>X</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (0, 'C', 10, 5),
            (1, 'B', 0, 5),
            (1, 'D', 0, 75),
            (2, 'R', 0, 5),
            (2, 'X', 0, 25)
        ]
    );
    // Text after it, which is the first text of the cell: it moves once.
    assert_eq!(
        glyphs(
            r#"<div style="display:table"><div style="display:table-row"><div style="height:30px">A</div><div style="height:70px;break-before:page">B</div>U</div><div style="display:table-row"><div style="display:table-cell">R</div></div></div><div>X</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (1, 'B', 0, 5),
            (1, 'U', 0, 75),
            (2, 'R', 0, 5),
            (2, 'X', 0, 25)
        ]
    );
}

#[test]
fn text_that_a_forced_break_moves_grows_the_row() {
    // Text after the break, in the cell's own lines.
    assert_eq!(
        glyphs(
            r#"<table><tr><td>T<div style="height:30px;break-after:page">A</div>U<div>D</div></td><td>C</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'A', 0, 25),
            (0, 'C', 10, 5),
            (0, 'T', 0, 5),
            (1, 'D', 0, 25),
            (1, 'U', 0, 5),
            (1, 'X', 0, 45)
        ]
    );
    // Text in an inline box, and text in a paragraph nested in the cell.
    for content in [
        r#"<div style="break-after:page">A</div><span>U</span><div>D</div>"#,
        r#"<div><div style="break-after:page">A</div>U</div><div>D</div>"#,
    ] {
        assert_eq!(
            glyphs(&format!(
                r#"<table><tr><td style="width:100px">{content}</td><td>C</td></tr><tr><td>R</td></tr></table><div>X</div>"#
            )),
            [
                (0, 'A', 0, 5),
                (0, 'C', 100, 5),
                (1, 'D', 0, 25),
                (1, 'R', 0, 45),
                (1, 'U', 0, 5),
                (1, 'X', 0, 65)
            ],
            "{content}"
        );
    }
}
