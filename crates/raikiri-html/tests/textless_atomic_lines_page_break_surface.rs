//! The lines of a paragraph that no text places, such as a paragraph of
//! inline-blocks alone, break at a page end as lines of text do: a line that
//! crosses the page end starts the next page, the `orphans` and `widows` of
//! the paragraph count its lines, blank ones left by `<br>` included, and a
//! paragraph that moves whole takes its box along. A paragraph of `<br>`
//! line breaks alone breaks between its blank lines the same way. Where each
//! glyph and box falls is what Chromium prints for the same documents on
//! pages 100px square, with Ahem glyphs of 20px on lines of 30px.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// `body` laid out on the pages and with the font of the Chromium documents.
fn lay_out(body: &str) -> DocumentLayout {
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
    result
}

/// The page, glyph, left edge and glyph top of every glyph of text in
/// `body`.
fn glyphs(body: &str) -> Vec<(u32, char, i32, i32)> {
    let result = lay_out(body);
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

#[test]
fn a_long_paragraph_breaks_again_on_each_page() {
    // Twelve lines of one inline-block each run over four pages; each page
    // after the first holds three of them, as their orphans and widows allow.
    let boxes: Vec<String> = ('A'..='L')
        .map(|glyph| format!(r#"<i style="width:60px">{glyph}</i>"#))
        .collect();
    let expected: Vec<(u32, char, i32, i32)> = ('A'..='L')
        .enumerate()
        .map(|(index, glyph)| {
            let index = index as i32;
            if index < 3 {
                (0, glyph, 0, 15 + 30 * index)
            } else {
                ((index / 3) as u32, glyph, 0, 5 + 30 * (index % 3))
            }
        })
        .chain(std::iter::once((4, 'Z', 0, 5)))
        .collect();
    assert_eq!(
        glyphs(&format!(
            r#"<div style="height:10px"></div><div style="orphans:3;widows:3">{}</div><div>Z</div>"#,
            boxes.join(" ")
        )),
        expected
    );
}

#[test]
fn widows_move_lines_that_do_not_fit_one_page_together() {
    // Lines of 40px from 10px: the third crosses the page end, and the four
    // widows take the second along, although the four lines need more than
    // a page; they break again there.
    assert_eq!(
        glyphs(
            r#"<div style="height:10px"></div><div style="orphans:1;widows:4"><i style="width:60px;height:40px">A</i> <i style="width:60px;height:40px">B</i> <i style="width:60px;height:40px">C</i> <i style="width:60px;height:40px">D</i> <i style="width:60px;height:40px">E</i></div><div>Z</div>"#
        ),
        [
            (0, 'A', 0, 15),
            (1, 'B', 0, 5),
            (1, 'C', 0, 45),
            (2, 'D', 0, 5),
            (2, 'E', 0, 45),
            (3, 'Z', 0, 5),
        ]
    );
}

#[test]
fn a_ruby_annotation_is_text_on_its_line() {
    // The annotation over the first inline-block is text, so the paragraph
    // breaks as one with text: it starts the next page whole for its orphans.
    let glyphs: Vec<_> = glyphs(
        r#"<div style="height:70px"></div><div><ruby><i>A</i><rt>rr</rt></ruby> <i>B</i> <i>C</i> <i>D</i> <i>E</i> <i>F</i></div><div>Z</div>"#,
    )
    .into_iter()
    .filter(|&(_, glyph, _, _)| glyph != 'r')
    .collect();
    assert_eq!(
        glyphs,
        [
            (1, 'A', 0, 15),
            (1, 'B', 60, 15),
            (1, 'C', 0, 45),
            (1, 'D', 60, 45),
            (1, 'E', 0, 75),
            (1, 'F', 60, 75),
            (2, 'Z', 0, 5),
        ]
    );
}

#[test]
fn a_line_moved_down_with_its_widows_breaks_again_at_the_next_page_end() {
    // The widows take the second line along to the second page, and the
    // third line, below it, then crosses the end of that page in turn.
    assert_eq!(
        glyphs(
            r#"<style>i{vertical-align:top}</style><div style="font:0/0 Ahem;orphans:1;widows:2"><i style="height:30px;font:20px/30px Ahem">A</i><br><i style="height:60px;font:20px/30px Ahem">B</i><br><i style="height:60px;font:20px/30px Ahem">C</i></div>"#
        ),
        [(0, 'A', 0, 5), (1, 'B', 0, 5), (2, 'C', 0, 5)]
    );
}

#[test]
fn a_line_moved_down_with_a_blank_line_breaks_again_at_the_next_page_end() {
    // The blank line crosses the end of the first page and starts the
    // second; the line after it then crosses the end of that page in turn.
    assert_eq!(
        glyphs(
            r#"<style>i{vertical-align:top;height:60px;font:20px/30px Ahem}</style><div style="font:0/60px Ahem;orphans:1;widows:1"><i>A</i><br><br><i>C</i></div>"#
        ),
        [(0, 'A', 0, 5), (2, 'C', 0, 5)]
    );
}

/// The page, top and bottom of each fragment of the box with `id` in `body`.
fn extents(body: &str, id: &str) -> Vec<(u32, f32, f32)> {
    let result = lay_out(body);
    let mut extents = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        let dom = page.dom();
        for fragment in page.fragments() {
            if dom.attr(fragment.node(), "id") == Some(id) {
                let rect = fragment.rect();
                extents.push((index, rect.y, rect.y + rect.height));
            }
        }
    }
    extents
}

/// A box of `height` above a paragraph `p` of `breaks` line breaks alone,
/// styled with `style`, and a box `z` of 10px below it.
fn blank_lines(height: u32, style: &str, breaks: usize) -> String {
    format!(
        "<div style='height:{height}px'></div><div id=p style='{style}'>{}</div>\
         <div id=z style='height:10px'></div>",
        "<br>".repeat(breaks)
    )
}

#[test]
fn a_paragraph_of_blank_lines_short_of_its_orphans_and_widows_moves_whole() {
    // Two lines leave no break with two lines on each side.
    let body = blank_lines(80, "", 2);
    assert_eq!(extents(&body, "p"), [(1, 0.0, 60.0)]);
    assert_eq!(extents(&body, "z"), [(1, 60.0, 70.0)]);
    // With one line on each side, the first line crosses the page end and
    // takes the paragraph along.
    assert_eq!(
        extents(&blank_lines(80, "orphans:1;widows:1", 2), "p"),
        [(1, 0.0, 60.0)]
    );
}

#[test]
fn blank_lines_break_between_them_at_the_page_end() {
    // The third line crosses the page end and starts the next page.
    let body = blank_lines(20, "", 4);
    assert_eq!(extents(&body, "p"), [(0, 20.0, 100.0), (1, 0.0, 60.0)]);
    assert_eq!(extents(&body, "z"), [(1, 60.0, 70.0)]);
    // Three widows take the second line along.
    assert_eq!(
        extents(&blank_lines(20, "widows:3", 5), "z"),
        [(1, 90.0, 100.0)]
    );
}

#[test]
fn a_long_paragraph_of_blank_lines_breaks_again_on_each_page() {
    // The first two lines cross the end of the first page, so the paragraph
    // moves whole; its widows then take a line along to the last page.
    let body = blank_lines(50, "", 7);
    assert_eq!(
        extents(&body, "p"),
        [(1, 0.0, 100.0), (2, 0.0, 100.0), (3, 0.0, 60.0)]
    );
    assert_eq!(extents(&body, "z"), [(3, 60.0, 70.0)]);
    assert_eq!(extents(&blank_lines(20, "", 7), "z"), [(2, 60.0, 70.0)]);
}
