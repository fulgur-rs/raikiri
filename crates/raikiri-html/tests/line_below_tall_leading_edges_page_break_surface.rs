//! The first line of a block below padding or border so tall that the line
//! does not fit after them on a page. Nothing comes between the edges and the
//! line, so the page breaks before the block when content comes before it;
//! on the page the block then starts, the line still runs past the end, and
//! the page breaks between the edges and the line, which starts the page
//! after. A block that already starts its page breaks there at once. Lines of
//! text, of generated text and of inline-blocks alike. Where each line falls
//! is what Chromium prints for the same documents on pages 100px square, with
//! Ahem glyphs of 20px on lines of 30px.

use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Lays out `body` on the pages and with the font of the Chromium documents.
/// The paragraphs of class `g` hold one line of generated text.
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
        "<!doctype html><style>@page{{size:100px 100px;margin:0}} body{{margin:0;font:20px/30px Ahem}} .g::before{{content:\"A\"}}</style><body>{body}</body>"
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

/// The page, glyph and glyph top of every word of text in `body`.
fn glyph_tops(body: &str) -> Vec<(u32, char, i32)> {
    let result = lay_out(body);
    let mut tops = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            for word in run.text.split_whitespace() {
                let glyph = word.chars().next().unwrap();
                tops.push((index, glyph, (run.origin.1 - run.ascent).round() as i32));
            }
        }
    }
    tops.sort_unstable();
    tops
}

const PARAGRAPHS: [&str; 3] = [
    r#"<p style="margin:0;width:20px;padding-bottom:20px">A</p>"#,
    r#"<p class="g" style="margin:0;width:20px;padding-bottom:20px"></p>"#,
    r#"<p style="margin:0;width:20px;padding-bottom:20px"><span style="display:inline-block;height:30px">A</span></p>"#,
];

/// `before`, then `paragraph` below the top edge `edge` of its block, then
/// `Z`.
fn document(before: &str, edge: &str, paragraph: &str) -> String {
    format!(r#"{before}<div style="{edge}">{paragraph}</div><div>Z</div>"#)
}

#[test]
fn a_block_after_content_starts_the_next_page_and_its_line_the_one_after() {
    for paragraph in PARAGRAPHS {
        for edge in ["padding-top:80px", "border-top:75px solid"] {
            let body = document(r#"<div style="height:10px">Y</div>"#, edge, paragraph);
            assert_eq!(
                glyph_tops(&body),
                [(0, 'Y', 5), (2, 'A', 5), (2, 'Z', 55)],
                "{body}"
            );
        }
    }
}

#[test]
fn a_block_that_starts_the_page_leaves_its_edges_there() {
    // The generated paragraph is left out: the style element that generates
    // its text comes before the block in the body.
    for paragraph in [PARAGRAPHS[0], PARAGRAPHS[2]] {
        for edge in ["padding-top:90px", "border-top:75px solid"] {
            let body = document("", edge, paragraph);
            assert_eq!(glyph_tops(&body), [(1, 'A', 5), (1, 'Z', 55)], "{body}");
        }
    }
}

#[test]
fn a_line_that_fits_below_the_edges_on_the_next_page_moves_with_them() {
    for paragraph in PARAGRAPHS {
        let body = document(
            r#"<div style="height:10px">Y</div>"#,
            "padding-top:70px",
            paragraph,
        );
        assert_eq!(
            glyph_tops(&body),
            [(0, 'Y', 5), (1, 'A', 75), (2, 'Z', 25)],
            "{body}"
        );
    }
}
