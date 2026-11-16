//! A first line that crosses the end of the page, inside a box that ends on
//! the page. The page has no break inside such a box, and none before its
//! first line, so the line overflows the box and stays where it is, and the
//! lines after it follow on below it. Only a second line that the break before
//! it would leave alone under `orphans` moves the paragraph, as the break
//! before the paragraph is then the one the page takes. A box that runs past
//! the page end itself moves as before, when the page can break before it.
//! Where each line falls is what Chromium prints for the same documents on
//! pages 100px square, with Ahem glyphs of 20px on lines of 30px.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Lays out `body` on the pages and with the font of the Chromium documents.
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
        "<!doctype html><style>@page{{size:100px 100px;margin:0}} body{{margin:0;font:20px/30px Ahem}}</style><body>{body}</body>"
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

/// The page, glyph and glyph top of every word of text in `body`. A glyph
/// that crosses into a page from the one before it is counted on the page it
/// starts on, as Chromium prints it on both.
fn glyph_tops(body: &str) -> Vec<(u32, char, i32)> {
    let result = lay_out(body);
    let mut tops = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            for word in run.text.split_whitespace() {
                let glyph = word.chars().next().unwrap();
                let top = (run.origin.1 - run.ascent).round() as i32;
                tops.push(if top < 0 && index > 0 {
                    (index - 1, glyph, top + 100)
                } else {
                    (index, glyph, top)
                });
            }
        }
    }
    tops.sort_unstable();
    tops
}

/// A spacer `spacer` tall, then a paragraph 20px wide whose border box is
/// `style` (`box-sizing: border-box`) holding `text`, then a block `Z`.
fn paragraph(spacer: u32, style: &str, text: &str) -> String {
    format!(
        "<div style='height:{spacer}px'></div><p style='margin:0;width:20px;\
         box-sizing:border-box;{style}'>{text}</p><div>Z</div>"
    )
}

#[test]
fn a_line_that_overflows_a_box_ending_on_the_page_stays() {
    for (body, expected) in [
        // The line at 85 crosses the end at 100; the box ends there.
        (
            paragraph(10, "padding-top:75px;height:90px", "A"),
            vec![(0, 'A', 90), (1, 'Z', 5)],
        ),
        (
            paragraph(15, "padding-top:60px;height:75px", "A"),
            vec![(0, 'A', 80), (1, 'Z', 5)],
        ),
        (
            paragraph(10, "padding-top:75px;max-height:90px", "A"),
            vec![(0, 'A', 90), (1, 'Z', 5)],
        ),
        // A box no taller than its padding, and one with a border instead.
        (
            paragraph(20, "padding-top:60px;max-height:20px", "A"),
            vec![(0, 'A', 85), (1, 'Z', 5)],
        ),
        (
            paragraph(20, "border-top:60px solid;height:80px", "A"),
            vec![(0, 'A', 85), (1, 'Z', 5)],
        ),
        // A line box taller than the paragraph, which has no padding.
        (
            paragraph(75, "height:10px", "A"),
            vec![(0, 'A', 80), (1, 'Z', 5)],
        ),
        // A formatting context root, and a relative offset that only paints
        // the line lower.
        (
            paragraph(20, "padding-top:60px;height:80px;display:flow-root", "A"),
            vec![(0, 'A', 85), (1, 'Z', 5)],
        ),
        (
            paragraph(
                20,
                "padding-top:60px;height:80px;position:relative;top:5px",
                "A",
            ),
            vec![(0, 'A', 90), (1, 'Z', 5)],
        ),
    ] {
        assert_eq!(glyph_tops(&body), expected, "{body}");
    }
}

#[test]
fn the_lines_after_it_follow_on_unless_orphans_move_the_paragraph() {
    // With one orphan allowed, the line after the first starts the next page
    // below what the first one overflows into it.
    for widows in [1, 2] {
        let body = paragraph(
            20,
            &format!("padding-top:60px;height:80px;orphans:1;widows:{widows}"),
            "A B",
        );
        assert_eq!(
            glyph_tops(&body),
            [(0, 'A', 85), (1, 'B', 15), (1, 'Z', 5)],
            "{body}"
        );
    }
    let body = paragraph(20, "padding-top:60px;height:80px;orphans:1", "A B C");
    assert_eq!(
        glyph_tops(&body),
        [(0, 'A', 85), (1, 'B', 15), (1, 'C', 45), (1, 'Z', 5)]
    );
    // Two orphans: the break before the second line would leave the first
    // alone, so the page breaks before the paragraph.
    let body = paragraph(20, "padding-top:60px;height:80px", "A B");
    assert_eq!(glyph_tops(&body), [(1, 'A', 65), (2, 'B', 5), (2, 'Z', 5)]);
}

#[test]
fn a_box_that_starts_with_the_paragraph_and_ends_on_the_page_keeps_it() {
    // A paragraph of auto height that crosses the page end, first in a
    // wrapper of set height that ends on the page: the page cannot break
    // before the paragraph, nor inside the wrapper.
    for wrapper in ["height:90px", "height:85px"] {
        for nested in [false, true] {
            let paragraph = "<p style='margin:0;width:20px;padding-top:75px'>A</p>";
            let inner = if nested {
                format!("<div>{paragraph}</div>")
            } else {
                paragraph.to_string()
            };
            let body = format!(
                "<div style='height:5px'></div><div style='{wrapper}'>{inner}</div><div>Z</div>"
            );
            assert_eq!(glyph_tops(&body), [(0, 'A', 85), (1, 'Z', 5)], "{body}");
        }
    }
    // A block before the paragraph in the wrapper gives the page a break
    // before it, so the paragraph, which runs past the page end, moves.
    let body = "<div style='height:5px'></div><div style='height:85px'>\
                <div style='height:10px'></div>\
                <p style='margin:0;width:20px;padding-top:65px'>A</p></div><div>Z</div>";
    assert_eq!(glyph_tops(body), [(1, 'A', 70), (1, 'Z', 5)]);
}

#[test]
fn a_block_after_margins_through_an_empty_block_past_the_page_end_moves() {
    // The margins collapse to 5px, so the block starts at 95 and ends on the
    // page, while its line crosses the end: it stays there, and its glyph
    // starts at the page end, the top of the next page.
    for between in ["", "<div style='height:0'></div>"] {
        let body = format!(
            "<div style='height:90px;margin-bottom:8px'></div>{between}\
             <div style='height:4px;margin-top:-3px'>X</div>"
        );
        assert_eq!(glyph_tops(&body), [(1, 'X', 0)], "{body}");
    }
    // A margin of 12px puts the empty block at 102, past the end: the page
    // breaks before it, and the block after it starts the next page.
    let body = "<div style='height:90px;margin-bottom:12px'></div> \
                <div style='height:0'></div> \
                <div style='height:4px;margin-top:-7px'>X</div>";
    assert_eq!(glyph_tops(body), [(1, 'X', 5)]);
}
