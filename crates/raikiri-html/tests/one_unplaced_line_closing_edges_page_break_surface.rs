//! A paragraph whose only line is generated text or an inline-block, and
//! whose own bottom padding crosses the end of a page. Nothing breaks between
//! the line and those edges, so the paragraph moves to the next page whole
//! when content comes before it, together with the padding or border that the
//! blocks it begins keep above it. A paragraph whose blocks start the page
//! stays, and its edges continue on the next page. Where each line falls is
//! what Chromium prints for the same documents on pages 100px square, with
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

const GENERATED: &str = r#"<p class="g" style="margin:0;width:20px;padding-bottom:PBpx"></p>"#;
const INLINE_BLOCK: &str = r#"<p style="margin:0;width:20px;padding-bottom:PBpx"><span style="display:inline-block;height:30px">A</span></p>"#;

/// `before`, then the paragraph `paragraph` with a bottom padding of
/// `padding` inside a block with the top edge `edge`, then a strip and `Z`.
fn document(before: &str, edge: &str, paragraph: &str, padding: u32) -> String {
    let paragraph = paragraph.replace("PB", &padding.to_string());
    format!(
        r#"{before}<div style="{edge}">{paragraph}</div><div style="height:10px"></div><div>Z</div>"#
    )
}

#[test]
fn a_paragraph_after_content_moves_with_the_top_edge_of_its_block() {
    for paragraph in [GENERATED, INLINE_BLOCK] {
        for edge in ["padding-top:10px", "border-top:10px solid"] {
            let body = document(r#"<div style="height:10px">Y</div>"#, edge, paragraph, 60);
            assert_eq!(
                glyph_tops(&body),
                [(0, 'Y', 5), (1, 'A', 15), (2, 'Z', 15)],
                "{body}"
            );
            let body = document(r#"<div style="height:10px"></div>"#, edge, paragraph, 60);
            assert_eq!(glyph_tops(&body), [(1, 'A', 15), (2, 'Z', 15)], "{body}");
        }
    }
}

#[test]
fn a_paragraph_whose_block_starts_the_page_stays() {
    for paragraph in [GENERATED, INLINE_BLOCK] {
        for edge in ["padding-top:10px", "border-top:10px solid"] {
            let body = document("", edge, paragraph, 60);
            assert_eq!(glyph_tops(&body), [(0, 'A', 15), (1, 'Z', 15)], "{body}");
        }
    }
}

#[test]
fn a_paragraph_whose_edges_fit_stays() {
    for paragraph in [GENERATED, INLINE_BLOCK] {
        for edge in ["padding-top:10px", "border-top:10px solid"] {
            let body = document(r#"<div style="height:10px">Y</div>"#, edge, paragraph, 40);
            assert_eq!(
                glyph_tops(&body),
                [(0, 'A', 25), (0, 'Y', 5), (1, 'Z', 5)],
                "{body}"
            );
        }
    }
}
