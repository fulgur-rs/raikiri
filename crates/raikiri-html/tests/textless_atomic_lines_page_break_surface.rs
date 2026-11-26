//! The lines of a paragraph that no text places, such as a paragraph of
//! inline-blocks alone, break at a page end as lines of text do: a line that
//! crosses the page end starts the next page, the `orphans` and `widows` of
//! the paragraph count its lines, blank ones left by `<br>` included, and a
//! paragraph that moves whole takes its box along. Where each glyph falls is
//! what Chromium prints for the same documents on pages 100px square, with
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
        "<!doctype html><style>@page{{size:100px 100px;margin:0}} body{{margin:0;font:20px/30px Ahem}} i{{display:inline-block;width:40px;font-style:normal}}</style><body>{body}</body>"
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
fn widows_take_lines_of_inline_blocks_to_the_next_page() {
    // The fourth line crosses the page end; the widows take the third along.
    assert_eq!(
        glyphs(
            r#"<div style="widows:3;"><i>A</i> <i>B</i> <i>C</i> <i>D</i> <i>E</i> <i>F</i> <i>G</i> <i>H</i></div><div>Z</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (0, 'B', 60, 5),
            (0, 'C', 0, 35),
            (0, 'D', 60, 35),
            (1, 'E', 0, 5),
            (1, 'F', 60, 5),
            (1, 'G', 0, 35),
            (1, 'H', 60, 35),
            (1, 'Z', 0, 65),
        ]
    );
}

#[test]
fn orphans_win_over_widows_in_a_paragraph_of_inline_blocks() {
    // Two lines fit: they keep the orphans, and the last line goes
    // alone, as the widows give way.
    assert_eq!(
        glyphs(
            r#"<div style="height:10px"></div><div style="border-top:3px solid;border-bottom:3px solid;"><i>A</i> <i>B</i> <i>C</i> <i>D</i> <i>E</i> <i>F</i></div><div>Z</div>"#
        ),
        [
            (0, 'A', 0, 18),
            (0, 'B', 60, 18),
            (0, 'C', 0, 48),
            (0, 'D', 60, 48),
            (1, 'E', 0, 5),
            (1, 'F', 60, 5),
            (1, 'Z', 0, 38),
        ]
    );
}

#[test]
fn a_paragraph_short_of_its_orphans_moves_with_its_border() {
    // Two lines fit, fewer than the orphans: the paragraph and its
    // border start the next page.
    assert_eq!(
        glyphs(
            r#"<div style="height:10px"></div><div style="orphans:3;border-top:3px solid;border-bottom:3px solid;"><i>A</i> <i>B</i> <i>C</i> <i>D</i> <i>E</i> <i>F</i> <i>G</i> <i>H</i></div><div>Z</div>"#
        ),
        [
            (1, 'A', 0, 8),
            (1, 'B', 60, 8),
            (1, 'C', 0, 38),
            (1, 'D', 60, 38),
            (1, 'E', 0, 68),
            (1, 'F', 60, 68),
            (2, 'G', 0, 5),
            (2, 'H', 60, 5),
            (2, 'Z', 0, 38),
        ]
    );
}

#[test]
fn a_first_line_that_crosses_the_page_end_takes_the_padding_along() {
    // The first line crosses the page end below a spacer: the break
    // falls before the box, with its padding.
    assert_eq!(
        glyphs(
            r#"<div style="height:70px"></div><div style="padding:5px 0;"><i>A</i> <i>B</i> <i>C</i> <i>D</i> <i>E</i> <i>F</i> <i>G</i> <i>H</i></div><div>Z</div>"#
        ),
        [
            (1, 'A', 0, 10),
            (1, 'B', 60, 10),
            (1, 'C', 0, 40),
            (1, 'D', 60, 40),
            (2, 'E', 0, 5),
            (2, 'F', 60, 5),
            (2, 'G', 0, 35),
            (2, 'H', 60, 35),
            (2, 'Z', 0, 70),
        ]
    );
}

#[test]
fn a_paragraph_moves_with_the_box_it_begins() {
    // The paragraph begins a box with top padding; both move to keep the
    // orphans.
    assert_eq!(
        glyphs(
            r#"<div style="height:10px"></div><div style="padding-top:4px"><div style="orphans:3;"><i>A</i> <i>B</i> <i>C</i> <i>D</i> <i>E</i> <i>F</i> <i>G</i> <i>H</i></div></div><div>Z</div>"#
        ),
        [
            (1, 'A', 0, 9),
            (1, 'B', 60, 9),
            (1, 'C', 0, 39),
            (1, 'D', 60, 39),
            (1, 'E', 0, 69),
            (1, 'F', 60, 69),
            (2, 'G', 0, 5),
            (2, 'H', 60, 5),
            (2, 'Z', 0, 35),
        ]
    );
}

#[test]
fn a_paragraph_that_starts_the_page_keeps_its_natural_break() {
    // Nothing comes before the paragraph on the page, so there is no
    // break before it: it breaks where its lines reach the page end.
    assert_eq!(
        glyphs(
            r#"<div style="orphans:3;padding:5px 0;"><i>A</i> <i>B</i> <i>C</i> <i>D</i> <i>E</i> <i>F</i> <i>G</i> <i>H</i> <i>I</i> <i>J</i></div><div>Z</div>"#
        ),
        [
            (0, 'A', 0, 10),
            (0, 'B', 60, 10),
            (0, 'C', 0, 40),
            (0, 'D', 60, 40),
            (0, 'E', 0, 70),
            (0, 'F', 60, 70),
            (1, 'G', 0, 5),
            (1, 'H', 60, 5),
            (1, 'I', 0, 35),
            (1, 'J', 60, 35),
            (1, 'Z', 0, 70),
        ]
    );
}

#[test]
fn blank_lines_count_for_the_widows() {
    // Lines left by br count as lines: the widows take the line of B along.
    assert_eq!(
        glyphs(
            r#"<div style="widows:3;"><i>A</i><br><br><i>B</i><br><br><i>C</i></div><div>Z</div>"#
        ),
        [
            (0, 'A', 0, 5),
            (1, 'B', 0, 5),
            (1, 'C', 0, 65),
            (2, 'Z', 0, 5),
        ]
    );
}

#[test]
fn a_blank_line_that_starts_a_page_moves_the_lines_before_it() {
    // The blank line after A starts the next page, leaving A alone under
    // the orphans: the paragraph moves.
    assert_eq!(
        glyphs(
            r#"<div style="height:70px"></div><div><i>A</i><br><br><i>B</i><br><br><i>C</i></div><div>Z</div>"#
        ),
        [
            (1, 'A', 0, 5),
            (1, 'B', 0, 65),
            (2, 'C', 0, 35),
            (2, 'Z', 0, 65),
        ]
    );
}

#[test]
fn lines_that_moved_whole_break_again_on_the_next_page() {
    // The paragraph moves whole for its orphans, then breaks again where
    // its third line crosses the next page end.
    assert_eq!(
        glyphs(
            r#"<div style="height:10px"></div><div style="orphans:3;"><i style="height:45px">A</i> <i>B</i> <i>C</i> <i style="height:45px">D</i> <i>E</i> <i>F</i></div><div>Z</div>"#
        ),
        [
            (1, 'A', 0, 5),
            (1, 'B', 60, 5),
            (1, 'C', 0, 50),
            (1, 'D', 60, 50),
            (2, 'E', 0, 5),
            (2, 'F', 60, 5),
            (2, 'Z', 0, 35),
        ]
    );
}
