//! The lines of a paragraph that mixes text with atomic inlines keep whole on
//! a page: a line that crosses the page end starts the next page, and the
//! `widows` and `orphans` of the paragraph count the lines of atomic inlines
//! as they count the lines of text. Where each glyph falls is what Chromium
//! prints for the same documents on pages 200px wide and 100px tall, with
//! Ahem glyphs of 20px on lines of 30px.

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
        "<!doctype html><style>@page{{size:200px 100px;margin:0}} body{{margin:0;font:20px/30px Ahem}} table{{border-spacing:0}} td{{padding:0;vertical-align:top}} i{{display:inline-block;width:20px;height:30px;font-style:normal}}</style><body>{body}</body>"
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
            // Every Ahem glyph is 20px wide.
            for (offset, glyph) in run.text.chars().enumerate() {
                if !glyph.is_whitespace() {
                    glyphs.push((
                        index,
                        glyph,
                        (run.origin.0 + 20.0 * offset as f32).round() as i32,
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
fn widows_move_a_line_of_atomic_inlines_after_text() {
    // Four lines from 10px: the third ends at the page end, but the widows
    // take it to the next page with the last.
    assert_eq!(
        glyphs(
            r#"<div style="height:10px"></div><div style="width:20px;widows:2">T T <i>B</i><i>B</i></div><div>Z</div>"#
        ),
        [
            (0, 'T', 0, 15),
            (0, 'T', 0, 45),
            (1, 'B', 0, 5),
            (1, 'B', 0, 35),
            (1, 'Z', 0, 65),
        ]
    );
}

#[test]
fn a_line_of_atomic_inlines_between_text_lines_moves_down() {
    // The fourth line crosses the page end and starts the next page, as the
    // widows ask for the line before it too.
    assert_eq!(
        glyphs(
            r#"<div style="height:10px"></div><div style="width:20px">T <i>B</i> T <i>B</i></div><div>Z</div>"#
        ),
        [
            (0, 'B', 0, 45),
            (0, 'T', 0, 15),
            (1, 'B', 0, 35),
            (1, 'T', 0, 5),
            (1, 'Z', 0, 65),
        ]
    );
}

#[test]
fn a_text_line_after_a_tall_atomic_inline_starts_the_next_page() {
    // The 50px inline-block makes the second line end at 70px; the third
    // line crosses the page end and moves.
    assert_eq!(
        glyphs(
            r#"<div style="height:10px"></div><div style="width:20px">T <i style="height:50px">B</i> T</div><div>Z</div>"#
        ),
        [
            (0, 'B', 0, 45),
            (0, 'T', 0, 15),
            (1, 'T', 0, 5),
            (1, 'Z', 0, 35),
        ]
    );
}

#[test]
fn lines_of_atomic_inlines_before_text_keep_their_place() {
    // The orphans keep both lines of inline-blocks on the first page.
    assert_eq!(
        glyphs(
            r#"<div style="height:40px"></div><div style="width:20px"><i>B</i><i>B</i> T T</div><div>Z</div>"#
        ),
        [
            (0, 'B', 0, 45),
            (0, 'B', 0, 75),
            (1, 'T', 0, 5),
            (1, 'T', 0, 35),
            (1, 'Z', 0, 65),
        ]
    );
}

#[test]
fn lines_of_atomic_inlines_in_a_contents_wrapper_before_text_keep_their_place() {
    // A `display: contents` wrapper lays the inline-blocks out in its place,
    // as in the test above.
    assert_eq!(
        glyphs(
            r#"<div style="height:40px"></div><div style="width:20px"><span style="display:contents"><i>B</i><i>B</i></span> T T</div><div>Z</div>"#
        ),
        [
            (0, 'B', 0, 45),
            (0, 'B', 0, 75),
            (1, 'T', 0, 5),
            (1, 'T', 0, 35),
            (1, 'Z', 0, 65),
        ]
    );
}

#[test]
fn a_script_with_an_author_display_before_the_atomic_inlines_is_their_text() {
    // Chromium prints the text of `<script style="display:inline">`: it
    // shares the first line with the inline-blocks, which then follow it
    // down the page.
    assert_eq!(
        glyphs(
            r#"<div style="height:40px"></div><div style="width:20px"><script style="display:inline">x</script><i>B</i><i>B</i> T T</div><div>Z</div>"#
        ),
        [
            (0, 'B', 0, 75),
            (0, 'x', 0, 45),
            (1, 'B', 0, 5),
            (1, 'T', 0, 35),
            (1, 'T', 0, 65),
            (2, 'Z', 0, 5),
        ]
    );
}

#[test]
fn a_line_with_an_image_moves_with_the_text() {
    assert_eq!(
        glyphs(
            r#"<div style="height:40px"></div><div style="width:20px">T <img style="width:20px;height:30px;vertical-align:top"> T</div><div>Z</div>"#
        ),
        [(0, 'T', 0, 45), (1, 'T', 0, 5), (1, 'Z', 0, 35)]
    );
}

#[test]
fn a_tall_atomic_inline_in_a_cell_moves_to_the_next_page() {
    // The second line of the cell holds a 90px inline-block that crosses the
    // page end: it starts the next page and grows the row.
    assert_eq!(
        glyphs(
            r#"<table><tr><td>T<br><i style="height:90px">B</i></td><td>C</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'C', 20, 5),
            (0, 'T', 0, 5),
            (1, 'B', 0, 5),
            (2, 'X', 0, 5),
        ]
    );
}

#[test]
fn lines_of_atomic_inlines_in_a_cell_follow_the_widows() {
    // Lines of 20px, with glyphs of 10px: the last two lines, both of
    // inline-blocks, start the next page together.
    assert_eq!(
        glyphs(
            r#"<table style="font:10px/20px Ahem"><tr><td style="width:20px"><div style="height:40px"></div>T T <i style="height:20px">B</i> <i style="height:20px">B</i></td><td>C</td></tr></table><div>X</div>"#
        ),
        [
            (0, 'C', 20, 5),
            (0, 'T', 0, 45),
            (0, 'T', 0, 65),
            (1, 'B', 0, 5),
            (1, 'B', 0, 25),
            (1, 'X', 0, 45),
        ]
    );
}

#[test]
fn an_absolute_box_of_atomic_inlines_breaks_between_its_lines() {
    // The third line crosses the page end and starts the next page, with the
    // fourth after it.
    for content in ["<i>A</i>", "T "] {
        let glyphs = glyphs(&format!(
            r#"<div style="position:absolute;top:20px;width:20px">{content}<i>B</i><i>C</i><i>D</i></div>"#
        ));
        let first = if content == "T " { 'T' } else { 'A' };
        let mut expected = vec![
            (0, first, 0, 25),
            (0, 'B', 0, 55),
            (1, 'C', 0, 5),
            (1, 'D', 0, 35),
        ];
        expected.sort_unstable();
        assert_eq!(glyphs, expected, "{content}");
    }
}

#[test]
fn an_atomic_inline_taller_than_a_page_starts_a_page_of_its_own() {
    assert_eq!(
        glyphs(
            r#"<div style="height:10px"></div><div style="width:20px">T <i style="height:150px">A</i> U</div><div>Z</div>"#
        ),
        [
            (1, 'T', 0, 5),
            (2, 'A', 0, 5),
            (3, 'U', 0, 55),
            (4, 'Z', 0, 5),
        ]
    );
}

#[test]
fn an_atomic_inline_raised_above_its_line_stays_where_it_is_drawn() {
    // The inline-block's negative margin raises it into the line before; the
    // widows move only the last line.
    assert_eq!(
        glyphs(
            r#"<div style="height:10px"></div><div style="width:20px">T T <i style="margin-top:-25px;vertical-align:top">A</i> U</div><div>Z</div>"#
        ),
        [
            (0, 'A', 0, 80),
            (0, 'T', 0, 15),
            (0, 'T', 0, 45),
            (1, 'U', 0, 35),
            (1, 'Z', 0, 65),
        ]
    );
}

#[test]
fn text_before_an_atomic_inline_in_source_order_moves_the_lines_under_bidi() {
    // The bidi override puts the inline-block before the text on the line,
    // but the text comes first in the document: the lines still move as in
    // the paragraph without the override.
    assert_eq!(
        glyphs(
            r#"<div style="height:10px"></div><div style="width:20px;widows:2"><bdo dir="rtl">T T <i>B</i><i>B</i></bdo></div><div>Z</div>"#
        ),
        glyphs(
            r#"<div style="height:10px"></div><div style="width:20px;widows:2">T T <i>B</i><i>B</i></div><div>Z</div>"#
        )
    );
}

#[test]
fn an_atomic_inline_in_a_ruby_annotation_moves_with_its_line() {
    // Chromium prints the ruby's line on the next page with the annotation's
    // glyph 25px above the base's. Where the moved line itself starts
    // depends on how far the annotation reaches above the line, which is
    // checked elsewhere, so only the page and the gap are compared here.
    let ruby = r#"<style>rt{font-size:20px;line-height:30px}</style>"#;
    for top in [30, 40] {
        let glyphs = glyphs(&format!(
            r#"{ruby}<div style="position:absolute;top:{top}px;width:20px">T <ruby>R<rt><i>C</i></rt></ruby></div>"#
        ));
        let [
            (0, 'T', 0, text),
            (1, 'C', 0, annotation),
            (1, 'R', 0, base),
        ] = glyphs[..]
        else {
            panic!("{top}: {glyphs:?}");
        };
        assert_eq!((text, base - annotation), (top + 5, 25), "{top}");
    }
    assert_eq!(
        glyphs(&format!(
            r#"{ruby}<div style="height:30px"></div><div style="width:20px">T <ruby>R<rt><i>C</i></rt></ruby></div>"#
        )),
        [(1, 'C', 0, 30), (1, 'R', 0, 55), (1, 'T', 0, 5)]
    );
}

#[test]
fn the_summary_of_a_details_is_text_before_the_atomic_inlines_after_it() {
    // The first summary is laid out first wherever it is among the children
    // of its details, so its text comes before the inline-blocks and the
    // lines of inline-blocks move as in a paragraph that starts with the text.
    let expected = [
        (0, 'T', 0, 45),
        (0, 'T', 0, 75),
        (1, 'B', 0, 5),
        (1, 'B', 0, 35),
        (1, 'Z', 0, 65),
    ];
    assert_eq!(
        glyphs(
            r#"<div style="height:40px"></div><details open style="width:20px"><i>B</i><i>B</i><summary style="display:inline">T T</summary></details><div>Z</div>"#
        ),
        expected
    );
    assert_eq!(
        glyphs(
            r#"<div style="height:40px"></div><div style="width:20px">T T<i>B</i><i>B</i></div><div>Z</div>"#
        ),
        expected
    );
}
