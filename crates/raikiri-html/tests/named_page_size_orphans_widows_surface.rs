//! A page whose size differs from the page before it, because its page
//! name or its `:first` rule differs, lays the content after its start out
//! again at its own width. The paragraphs that start on it, after a box
//! that starts it, or before it with a line that starts it, keep their
//! `orphans` and `widows` at the breaks after its start, as in Chromium. Where each glyph falls is what Chromium prints for the same
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

#[test]
fn lines_after_a_block_that_starts_a_page_of_another_size_keep_their_widows() {
    // The block among the lines of the paragraph starts the second page, of
    // another size than the wider first; the lines after it break at the end
    // of that page with the widows of the paragraph.
    assert_eq!(
        glyphs(
            r#"<style>@page :first{size:200px 100px}</style><div style="orphans:3;widows:3">Z<br>Y<br>X<div style="height:40px">B</div>W<br>V<br>U</div>"#
        ),
        [
            (0, 'X', 0, 65),
            (0, 'Y', 0, 35),
            (0, 'Z', 0, 5),
            (1, 'B', 0, 5),
            (2, 'U', 0, 65),
            (2, 'V', 0, 35),
            (2, 'W', 0, 5),
        ]
    );
}

/// The style that makes the first page 200px wide, so the page after it,
/// which a line of the paragraph starts, is laid out again at its own width.
const WIDE_FIRST_PAGE: &str = "<style>@page :first{size:200px 100px}</style>";

#[test]
fn widows_hold_after_a_page_of_another_size_that_a_line_starts() {
    // The fifth line would end the second page alone over the last; the
    // widows take it down with the last.
    let lines = "Z<br>Y<br>X<br>W<br>V<br>U<br>T";
    let expected = [
        (0, 'X', 0, 65),
        (0, 'Y', 0, 35),
        (0, 'Z', 0, 5),
        (1, 'V', 0, 35),
        (1, 'W', 0, 5),
        (2, 'T', 0, 35),
        (2, 'U', 0, 5),
    ];
    assert_eq!(glyphs(&format!("{WIDE_FIRST_PAGE}{lines}")), expected);
    assert_eq!(
        glyphs(&format!("{WIDE_FIRST_PAGE}<div>{lines}</div>")),
        expected
    );
    // Three widows would leave one line on the second page, under the two
    // orphans: the page gives up as many as it can spare.
    assert_eq!(
        glyphs(&format!(
            "{WIDE_FIRST_PAGE}<div style=\"widows:3\">{lines}</div>"
        )),
        expected
    );
}

#[test]
fn orphans_that_no_page_holds_keep_the_lines_after_a_page_of_another_size() {
    // Neither the first page nor the second holds four lines: both break
    // where they end, and the second keeps the line the widows would take.
    assert_eq!(
        glyphs(&format!(
            "{WIDE_FIRST_PAGE}<div style=\"orphans:4\">Z<br>Y<br>X<br>W<br>V<br>U<br>T</div>"
        )),
        [
            (0, 'X', 0, 65),
            (0, 'Y', 0, 35),
            (0, 'Z', 0, 5),
            (1, 'U', 0, 65),
            (1, 'V', 0, 35),
            (1, 'W', 0, 5),
            (2, 'T', 0, 5),
        ]
    );
}

#[test]
fn the_breaks_after_a_paragraph_that_a_page_of_another_size_splits_keep_their_widows() {
    let glyphs = glyphs(&format!(
        "{WIDE_FIRST_PAGE}<div>Z<br>Y<br>X<br>W<br>V<br>U<br>T</div><div>Q<br>P<br>O<br>N</div>"
    ));
    assert_eq!(
        glyphs[7..],
        [
            (3, 'P', 0, 35),
            (3, 'Q', 0, 5),
            (4, 'N', 0, 35),
            (4, 'O', 0, 5),
        ]
    );
}

#[test]
fn widows_hold_in_text_wrapped_again_after_a_page_of_another_size() {
    // Two words fit on a line of the first page, one on the pages after it.
    let line_starts: Vec<_> = glyphs(&format!(
        "{WIDE_FIRST_PAGE}<div>AAAA BBBB CCCC DDDD EEEE FFFF GGGG HHHH IIII JJ</div>"
    ))
    .into_iter()
    .filter(|&(_, _, left, _)| left == 0)
    .collect();
    assert_eq!(
        line_starts,
        [
            (0, 'A', 0, 5),
            (0, 'C', 0, 35),
            (0, 'E', 0, 65),
            (1, 'G', 0, 5),
            (1, 'H', 0, 35),
            (2, 'I', 0, 5),
            (2, 'J', 0, 35),
        ]
    );
}
