//! A page whose size differs from the page before it, because its page
//! name differs, lays the content after its start out again at its own
//! width. The paragraphs that start on it, or after a box that starts it,
//! keep their `orphans` and `widows` at the breaks after its start, as in
//! Chromium. Where each glyph falls is what Chromium prints for the same
//! documents, on default pages 100px square, pages named `foo` 200px wide,
//! and Ahem glyphs of 20px on lines of 30px.

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
        "<!doctype html><style>@page{{size:100px 100px;margin:0}} @page foo{{size:200px 100px}} body{{margin:0;font:20px/30px Ahem}}</style><body>{body}</body>"
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
fn widows_hold_in_a_paragraph_that_starts_a_page_after_a_page_name_change() {
    // The fourth line crosses the end of the second page; the widows take
    // the third along.
    assert_eq!(
        glyphs(r#"<div style="page:foo">A</div><div>Z<br>Y<br>X<br>W</div>"#),
        [
            (0, 'A', 0, 5),
            (1, 'Y', 0, 35),
            (1, 'Z', 0, 5),
            (2, 'W', 0, 35),
            (2, 'X', 0, 5),
        ]
    );
}

#[test]
fn larger_widows_hold_after_a_page_name_change() {
    assert_eq!(
        glyphs(r#"<div style="page:foo">A</div><div style="widows:3">Z<br>Y<br>X<br>W<br>V</div>"#),
        [
            (0, 'A', 0, 5),
            (1, 'Y', 0, 35),
            (1, 'Z', 0, 5),
            (2, 'V', 0, 65),
            (2, 'W', 0, 35),
            (2, 'X', 0, 5),
        ]
    );
}

#[test]
fn widows_hold_in_lines_of_the_body_after_a_box_that_starts_the_page() {
    // The spacer starts the second page, and the lines that follow it in the
    // body break twice: the widows hold at the second break.
    assert_eq!(
        glyphs(
            r#"<div style="page:foo">A</div><div style="height:40px"></div>Z<br>Y<br>X<br>W<br>V<br>U"#
        ),
        [
            (0, 'A', 0, 5),
            (1, 'Y', 0, 75),
            (1, 'Z', 0, 45),
            (2, 'W', 0, 35),
            (2, 'X', 0, 5),
            (3, 'U', 0, 35),
            (3, 'V', 0, 5),
        ]
    );
}

#[test]
fn widows_hold_in_lines_beside_a_float_that_starts_the_page() {
    assert_eq!(
        glyphs(
            r#"<div style="page:foo">A</div><div style="float:left;width:10px;height:10px"></div>Z<br>Y<br>X<br>W"#
        ),
        [
            (0, 'A', 0, 5),
            (1, 'Y', 0, 35),
            (1, 'Z', 10, 5),
            (2, 'W', 0, 35),
            (2, 'X', 0, 5),
        ]
    );
}

#[test]
fn orphans_hold_in_lines_of_the_body_after_a_page_name_change() {
    // Two lines fit below the spacer; the orphans take them to the next page.
    assert_eq!(
        glyphs(
            r#"<style>body{orphans:3}</style><div style="page:foo">A</div><div style="height:40px"></div>Z<br>Y<br>X<br>W"#
        ),
        [
            (0, 'A', 0, 5),
            (2, 'X', 0, 65),
            (2, 'Y', 0, 35),
            (2, 'Z', 0, 5),
            (3, 'W', 0, 5),
        ]
    );
}
