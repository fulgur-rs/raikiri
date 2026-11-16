//! A paragraph with nothing but margins before it on its page, whose first
//! line crosses the page end below the paragraph's own padding-top or
//! border-top. The page has no break before such a paragraph: it stays on the
//! page with its edges, and the page breaks between the edges and the line,
//! which starts the next page. That holds for the margin of the first page,
//! one collapsed through the body or a wrapper, and one kept after a forced
//! break, below the padding of a wrapper too. A box before the paragraph on
//! the page, even an empty one, gives the page a break before it, and so do
//! edges that cross the page end themselves: the paragraph then moves whole.
//! A line with no edges above it in the paragraph moves to the next page.
//! Lines of text, of generated text and of inline-blocks alike. Where each
//! line falls is what Chromium prints for the same documents on pages 100px
//! square, with Ahem glyphs of 20px on lines of 30px.

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

/// The content of a paragraph of one line: of text, of generated text and of
/// an inline-block.
const LINES: [(&str, &str); 3] = [
    ("", "A"),
    (" class=\"g\"", ""),
    (
        "",
        r#"<span style="display:inline-block;height:30px">A</span>"#,
    ),
];

/// What comes before and after a paragraph with nothing but margins above
/// it on its page: the body's own margin, a wrapper whose margin collapses
/// with the paragraph's, a wrapper with padding, and a wrapper with a margin
/// after a forced break, which the page keeps.
const OPENINGS: [(&str, &str, u32); 4] = [
    ("<style>body{margin-top:20px!important}</style>", "", 1),
    (r#"<div style="margin-top:10px">"#, "</div>", 1),
    (r#"<div style="padding-top:10px">"#, "</div>", 1),
    (
        r#"<div style="height:10px"></div><div style="break-before:page;margin-top:20px">"#,
        "</div>",
        2,
    ),
];

/// `before`, then a paragraph with the margin and padding `style`, holding
/// `content`, then `after` and `Z`.
fn document(before: &str, after: &str, class: &str, style: &str, content: &str) -> String {
    format!(r#"{before}<p{class} style="width:20px;{style}">{content}</p>{after}<div>Z</div>"#)
}

#[test]
fn a_paragraph_below_only_margins_leaves_its_edges_and_moves_its_line() {
    for (class, content) in LINES {
        for edge in ["padding-top:60px", "border-top:55px solid"] {
            let body = document("", "", class, &format!("margin:20px 0 0;{edge}"), content);
            assert_eq!(glyph_tops(&body), [(1, 'A', 5), (1, 'Z', 35)], "{body}");
        }
        for (before, after, page) in OPENINGS {
            let body = document(
                before,
                after,
                class,
                "margin:20px 0 0;padding-top:60px",
                content,
            );
            assert_eq!(
                glyph_tops(&body),
                [(page, 'A', 5), (page, 'Z', 35)],
                "{body}"
            );
        }
    }
}

#[test]
fn the_lines_after_the_first_stay_with_it() {
    for (before, after) in [("", ""), (r#"<div style="padding-top:10px">"#, "</div>")] {
        let body = document(before, after, "", "margin:50px 0 0;padding-top:40px", "A B");
        assert_eq!(
            glyph_tops(&body),
            [(1, 'A', 5), (1, 'B', 35), (1, 'Z', 65)],
            "{body}"
        );
    }
}

#[test]
fn a_line_taller_than_a_page_also_leaves_the_edges() {
    let tall = r#"<span style="display:inline-block;height:120px;vertical-align:top">A</span>"#;
    for (before, after) in [("", ""), (r#"<div style="padding-top:10px">"#, "</div>")] {
        let body = document(before, after, "", "margin:20px 0 0;padding-top:60px", tall);
        assert_eq!(glyph_tops(&body), [(1, 'A', 5), (2, 'Z', 25)], "{body}");
    }
}

#[test]
fn a_box_before_the_paragraph_on_its_page_lets_it_move_whole() {
    for (class, content) in LINES {
        let body = document(
            "<div></div>",
            "",
            class,
            "margin:20px 0 0;padding-top:60px",
            content,
        );
        assert_eq!(glyph_tops(&body), [(1, 'A', 65), (2, 'Z', 5)], "{body}");
    }
}

#[test]
fn edges_that_cross_the_page_end_move_the_paragraph_whole() {
    for (class, content) in LINES {
        for (before, after) in [("", ""), (r#"<div style="padding-top:10px">"#, "</div>")] {
            let body = document(
                before,
                after,
                class,
                "margin:50px 0 0;padding-top:60px",
                content,
            );
            assert_eq!(glyph_tops(&body), [(1, 'A', 65), (2, 'Z', 5)], "{body}");
        }
    }
}

#[test]
fn a_line_with_no_edges_above_it_moves_to_the_next_page() {
    for (class, content) in LINES {
        for (before, after, page) in [
            ("", "", 1),
            (r#"<div style="padding-top:10px">"#, "</div>", 1),
            OPENINGS[3],
        ] {
            let body = document(before, after, class, "margin:80px 0 0", content);
            assert_eq!(
                glyph_tops(&body),
                [(page, 'A', 5), (page, 'Z', 35)],
                "{body}"
            );
        }
    }
}

#[test]
fn text_or_a_generated_box_before_the_paragraph_lets_it_move_whole() {
    for (class, content) in LINES {
        let body = document(
            r#"<div style="width:20px;font-size:10px;line-height:10px">Y<div style="font:20px/30px Ahem">"#,
            "</div></div>",
            class,
            "margin:10px 0 0;padding-top:60px",
            content,
        );
        assert_eq!(
            glyph_tops(&body),
            [(0, 'Y', 0), (1, 'A', 65), (2, 'Z', 5)],
            "{body}"
        );
        let body = document(
            "Y Y Y",
            "",
            class,
            "margin:10px 0 0;padding-top:40px",
            content,
        );
        assert_eq!(
            glyph_tops(&body),
            [
                (0, 'Y', 5),
                (0, 'Y', 5),
                (0, 'Y', 5),
                (1, 'A', 45),
                (1, 'Z', 75)
            ],
            "{body}"
        );
        let body = document(
            r#"<style>.w::before{content:"x";display:block}</style><div class="w">"#,
            "</div>",
            class,
            "margin:10px 0 0;padding-top:40px",
            content,
        );
        assert_eq!(
            glyph_tops(&body),
            [(0, 'x', 5), (1, 'A', 45), (1, 'Z', 75)],
            "{body}"
        );
    }
}

#[test]
fn generated_content_of_the_body_before_the_paragraph_lets_it_move_whole() {
    for (class, content) in LINES {
        for display in [";display:block", ""] {
            let body = document(
                &format!(r#"<style>body::before{{content:"x"{display}}}</style>"#),
                "",
                class,
                "margin:10px 0 0;padding-top:40px",
                content,
            );
            assert_eq!(
                glyph_tops(&body),
                [(0, 'x', 5), (1, 'A', 45), (1, 'Z', 75)],
                "{body}"
            );
        }
    }
}

#[test]
fn a_box_before_the_paragraph_counts_where_the_flow_puts_it() {
    // The box before the paragraph is on its page in the flow, though its
    // relative offset draws it on the page before.
    let body = document(
        r#"<div style="height:100px">Y</div><div style="height:10px;position:relative;top:-150px"></div>"#,
        "",
        "",
        "margin:20px 0 0;padding-top:60px",
        "A",
    );
    assert_eq!(
        glyph_tops(&body),
        [(0, 'Y', 5), (2, 'A', 65), (3, 'Z', 5)],
        "{body}"
    );
}

#[test]
fn a_generated_box_counts_where_it_ends() {
    // The generated box starts on the page before the paragraph's, and ends
    // on the paragraph's page: the paragraph moves whole.
    for (before, after) in [
        (
            r#"<style>body::before{content:"";display:block;height:140px}</style>"#,
            "",
        ),
        (
            r#"<style>.w::before{content:"";display:block;height:140px}</style><div class="w">"#,
            "</div>",
        ),
        (
            r#"<style>.s::before{content:"";display:block;height:140px}</style><span class="s"></span>"#,
            "",
        ),
    ] {
        let body = document(before, after, "", "margin:10px 0 0;padding-top:40px", "A");
        assert_eq!(glyph_tops(&body), [(2, 'A', 45), (2, 'Z', 75)], "{body}");
    }
    for (class, content) in LINES {
        let body = document(
            r#"<style>.w::before{content:"";display:block;height:140px}</style><div class="w" style="padding-top:10px">"#,
            "</div>",
            class,
            "margin:10px 0 0;padding-top:40px",
            content,
        );
        assert_eq!(glyph_tops(&body), [(2, 'A', 45), (2, 'Z', 75)], "{body}");
    }
}
