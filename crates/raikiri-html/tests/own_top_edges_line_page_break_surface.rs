//! The first line of a paragraph below its own padding-top or border-top, so
//! tall that the line does not fit after them on a page. When content comes
//! before the paragraph, the page breaks before it; on the page it then
//! starts, or on the page it starts anyway, the page breaks between its top
//! edges and the line, which starts the page after. Lines of text, of
//! generated text, of inline-blocks and of a `<br>` alone alike, and the
//! lines after the first break again on the pages after it. So does a line
//! below the floats that put it down, and a line taller than a page. A
//! paragraph that ends on its page, or that no break goes through, keeps its
//! line. Where each line falls is what Chromium prints for the same documents
//! on pages 100px square, with Ahem glyphs of 20px on lines of 30px.

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

/// The content of a paragraph of one line: of text, of generated text, of an
/// inline-block and of a `<br>` alone, with whether the line has a glyph.
const LINES: [(&str, &str, bool); 4] = [
    ("", "A", true),
    (" class=\"g\"", "", true),
    (
        "",
        r#"<span style="display:inline-block;height:30px">A</span>"#,
        true,
    ),
    ("", "<br>", false),
];

const EDGES: [&str; 2] = ["padding-top:80px", "border-top:75px solid"];

const BEFORE: &str = r#"<div style="height:10px">Y</div>"#;

/// `before`, then a paragraph with the top `edge` and 20px of padding below,
/// holding `content`, then `Z`.
fn document(before: &str, class: &str, edge: &str, content: &str) -> String {
    format!(
        r#"{before}<p{class} style="margin:0;width:20px;{edge};padding-bottom:20px">{content}</p><div>Z</div>"#
    )
}

#[test]
fn a_paragraph_after_content_starts_the_next_page_and_its_line_the_one_after() {
    for (class, content, glyph) in LINES {
        for edge in EDGES {
            let body = document(BEFORE, class, edge, content);
            let expected: Vec<_> = [(0, 'Y', 5), (2, 'A', 5), (2, 'Z', 55)]
                .into_iter()
                .filter(|&(_, c, _)| glyph || c != 'A')
                .collect();
            assert_eq!(glyph_tops(&body), expected, "{body}");
        }
    }
}

#[test]
fn a_paragraph_that_starts_the_page_leaves_its_edges_there() {
    for (class, content, glyph) in LINES {
        for edge in EDGES {
            let body = document("", class, edge, content);
            let expected: Vec<_> = [(1, 'A', 5), (1, 'Z', 55)]
                .into_iter()
                .filter(|&(_, c, _)| glyph || c != 'A')
                .collect();
            assert_eq!(glyph_tops(&body), expected, "{body}");
        }
    }
}

#[test]
fn the_lines_after_the_first_break_again_on_the_pages_after_it() {
    for edge in EDGES {
        let body = document(BEFORE, "", edge, "A B C");
        assert_eq!(
            glyph_tops(&body),
            [
                (0, 'Y', 5),
                (2, 'A', 5),
                (2, 'B', 35),
                (3, 'C', 5),
                (3, 'Z', 55)
            ],
            "{body}"
        );
        let body = document("", "", edge, "A B C");
        assert_eq!(
            glyph_tops(&body),
            [(1, 'A', 5), (1, 'B', 35), (2, 'C', 5), (2, 'Z', 55)],
            "{body}"
        );
    }
}

const TALL_INLINE_BLOCK: &str =
    r#"<span style="display:inline-block;height:120px;width:20px">A</span>"#;

#[test]
fn a_line_taller_than_a_page_also_leaves_the_edges_or_floats_above_it() {
    let paragraph = format!(
        r#"<p style="margin:0;width:40px;padding-top:50px">{TALL_INLINE_BLOCK}</p><div>Z</div>"#
    );
    assert_eq!(
        glyph_tops(&format!("{BEFORE}{paragraph}")),
        [(0, 'Y', 5), (2, 'A', 5), (3, 'Z', 25)]
    );
    assert_eq!(glyph_tops(&paragraph), [(1, 'A', 5), (2, 'Z', 25)]);
    let below_float = format!(
        r#"<div style="float:left;width:100%;height:60px"></div><p style="margin:0;width:20px">{TALL_INLINE_BLOCK}</p><div>Z</div>"#
    );
    assert_eq!(glyph_tops(&below_float), [(1, 'A', 5), (2, 'Z', 25)]);
    let text_below_float = r#"<div style="float:left;width:100%;height:80px"></div><p style="margin:0;width:20px">A</p><div>Z</div>"#;
    assert_eq!(glyph_tops(text_below_float), [(1, 'A', 5), (1, 'Z', 35)]);
}

#[test]
fn a_paragraph_that_ends_on_its_page_keeps_the_line_that_overflows_it() {
    let fixed = "padding-top:75px;height:100px;box-sizing:border-box";
    let body = format!(
        r#"<p style="margin:0;width:20px;{fixed}"><span style="display:inline-block;height:30px">A</span></p><div>Z</div>"#
    );
    assert_eq!(glyph_tops(&body), [(0, 'A', 80), (1, 'Z', 5)], "{body}");
    let body =
        format!(r#"{BEFORE}<p class="g" style="margin:0;width:20px;{fixed}"></p><div>Z</div>"#);
    assert_eq!(
        glyph_tops(&body),
        [(0, 'Y', 5), (1, 'A', 80), (2, 'Z', 5)],
        "{body}"
    );
    // A box taller than the page breaks below its edges, even with its line
    // running past its own end.
    let body = format!(
        r#"<p style="margin:0;width:20px;padding-top:75px;max-height:150px;box-sizing:border-box">{TALL_INLINE_BLOCK}</p><div>Z</div>"#
    );
    assert_eq!(glyph_tops(&body), [(1, 'A', 5), (1, 'Z', 55)], "{body}");
}

#[test]
fn a_box_with_size_containment_keeps_its_line() {
    // Chromium leaves the line below the edges, where its glyph crosses the
    // page end.
    let body = r#"<p class="g" style="margin:0;width:20px;contain:size;height:30px;padding-top:80px"></p><div>Z</div>"#;
    assert_eq!(glyph_tops(body), [(0, 'A', 85), (1, 'Z', 15)], "{body}");
}

#[test]
fn a_box_around_the_paragraph_that_ends_on_its_page_keeps_the_line() {
    for content in [
        "A",
        r#"<span style="display:inline-block;height:30px">A</span>"#,
    ] {
        let body = format!(
            r#"<div style="max-height:100px"><p style="margin:0;width:20px;padding-top:75px">{content}</p></div><div>Z</div>"#
        );
        assert_eq!(glyph_tops(&body), [(0, 'A', 80), (1, 'Z', 5)], "{body}");
    }
}
