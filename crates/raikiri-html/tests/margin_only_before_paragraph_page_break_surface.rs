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

#[test]
fn generated_text_of_a_contents_element_before_the_paragraph_lets_it_move_whole() {
    for (class, content) in LINES {
        for pseudo in ["before", "after"] {
            let body = document(
                &format!(
                    r#"<style>.s::{pseudo}{{content:"x"}}</style><span class="s" style="display:contents"></span>"#
                ),
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
fn generated_content_of_a_contents_ancestor_before_the_paragraph_lets_it_move_whole() {
    for (class, content) in LINES {
        for (before, after) in [
            (
                r#"<style>.w{display:contents}.w::before{content:"x"}</style><div class="w">"#,
                "</div>",
            ),
            (
                r#"<style>.w{display:contents}.w::before{content:"x";display:block}</style><div class="w"><div style="display:contents">"#,
                "</div></div>",
            ),
            (
                r#"<style>.s::before{content:"x"}</style><div style="display:contents"><span class="s" style="display:contents"></span>"#,
                "</div>",
            ),
        ] {
            let body = document(
                before,
                after,
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
fn a_phantom_line_before_the_paragraph_lets_it_move_whole() {
    // An empty inline element lays out a line whose anonymous block is a box
    // of no height, as an empty `div` is.
    for (class, content) in LINES {
        for (before, after) in [
            ("<span></span>", ""),
            (r#"<div style="padding-top:10px"><span></span>"#, "</div>"),
            (r#"<div style="display:contents"><b></b></div>"#, ""),
        ] {
            let body = document(
                before,
                after,
                class,
                "margin:20px 0 0;padding-top:60px",
                content,
            );
            assert_eq!(glyph_tops(&body), [(1, 'A', 65), (2, 'Z', 5)], "{body}");
        }
    }
}

#[test]
fn generated_text_inside_a_contents_sibling_lets_the_paragraph_move_whole() {
    for (class, content) in LINES {
        let body = document(
            r#"<style>.s::before{content:"x"}</style><div style="display:contents"><span class="s" style="display:contents"></span></div>"#,
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

#[test]
fn a_generated_box_that_runs_onto_the_page_lets_a_one_line_paragraph_move_whole() {
    // The generated box before the paragraph runs onto the paragraph's page,
    // where no box of the flow is placed before it: the paragraph is
    // measured on that page, and moves whole as its line crosses the page
    // end.
    for (class, content) in LINES {
        for (before, after) in [
            (
                r#"<style>body::before{content:"";display:block;height:140px}</style>"#,
                "",
            ),
            (
                r#"<style>.s::after{content:"";display:block;height:140px}</style><span class="s"></span>"#,
                "",
            ),
            (
                r#"<style>.w{display:contents}.w::before{content:"";display:block;height:140px}</style><div class="w">"#,
                "</div>",
            ),
        ] {
            for (style, top, z) in [
                ("margin:10px 0 0;padding-top:40px", 45, (2, 75)),
                ("margin:10px 0 0;padding-top:60px", 65, (3, 5)),
            ] {
                let body = document(before, after, class, style, content);
                assert_eq!(
                    glyph_tops(&body),
                    [(2, 'A', top), (z.0, 'Z', z.1)],
                    "{body}"
                );
            }
        }
    }
}

#[test]
fn the_page_end_cuts_the_margin_after_a_generated_box() {
    // A generated box that ends at or just above the page end: the page
    // breaks in the margin between it and the paragraph, which is cut, so the
    // paragraph starts the next page with its padding.
    let generated = |height: u32| {
        [
            format!(r#"<style>body::before{{content:"";display:block;height:{height}px}}</style>"#),
            format!(
                r#"<style>.s::before{{content:"";display:block;height:{height}px}}</style><span class="s"></span>"#
            ),
            format!(
                r#"<style>.s::after{{content:"";display:block;height:{height}px}}</style><span class="s"></span>"#
            ),
        ]
    };
    for height in [95, 100] {
        for before in generated(height) {
            for (class, content) in LINES {
                for (padding, tops) in [
                    (40, [(1, 'A', 45), (1, 'Z', 75)]),
                    (60, [(1, 'A', 65), (2, 'Z', 5)]),
                ] {
                    let body = document(
                        &before,
                        "",
                        class,
                        &format!("margin:10px 0 0;padding-top:{padding}px"),
                        content,
                    );
                    assert_eq!(glyph_tops(&body), tops, "{body}");
                }
            }
        }
    }
    // The ::before of a wrapper, before a paragraph of text, of generated
    // text or of an inline-block, with the wrapper's padding above it too.
    for (before, page) in [
        (
            r#"<style>.w::before{content:"";display:block;height:100px}</style><div class="w">"#,
            1,
        ),
        (
            r#"<style>.w::before{content:"";display:block;height:95px}</style><div class="w">"#,
            1,
        ),
        (
            r#"<style>.w::before{content:"";display:block;height:90px}</style><div class="w" style="padding-top:10px">"#,
            1,
        ),
        (
            r#"<style>.w::before{content:"";display:block;height:190px}</style><div class="w" style="padding-top:10px">"#,
            2,
        ),
    ] {
        for (class, content) in LINES {
            let body = document(
                before,
                "</div>",
                class,
                "margin:10px 0 0;padding-top:40px",
                content,
            );
            assert_eq!(
                glyph_tops(&body),
                [(page, 'A', 45), (page, 'Z', 75)],
                "{body}"
            );
        }
    }
}

#[test]
fn the_page_end_cuts_the_margin_after_a_generated_box_through_the_blocks_it_begins() {
    // The paragraph begins blocks after the ::before of a wrapper, and its
    // margin collapses through their tops with theirs: the page end cuts it
    // all the same.
    let before = r#"<style>.w::before{content:"";display:block;height:100px}</style><div class="w">"#;
    for (open, close) in [
        ("<div>", "</div>"),
        ("<div><div>", "</div></div>"),
        (r#"<div style="margin-top:5px">"#, "</div>"),
        (r#"<div style="margin-top:20px">"#, "</div>"),
        (
            r#"<ul style="margin:0;padding:0"><li style="list-style:none">"#,
            "</li></ul>",
        ),
    ] {
        for (class, content) in LINES {
            let body = document(
                &format!("{before}{open}"),
                &format!("{close}</div>"),
                class,
                "margin:10px 0 0;padding-top:40px",
                content,
            );
            assert_eq!(
                glyph_tops(&body),
                [(1, 'A', 45), (1, 'Z', 75)],
                "{body}"
            );
        }
    }
    // A block whose padding, or whose own formatting context, holds the
    // margin of the paragraph keeps it below the page edge.
    for (open, top) in [
        (r#"<div style="padding-top:1px">"#, 56),
        (r#"<div style="display:flow-root">"#, 55),
        (r#"<div style="overflow:hidden">"#, 55),
    ] {
        let body = document(
            &format!("{before}{open}"),
            "</div></div>",
            "",
            "margin:10px 0 0;padding-top:40px",
            "A",
        );
        assert_eq!(glyph_tops(&body), [(1, 'A', top), (2, 'Z', 5)], "{body}");
    }
    // Text before the paragraph in the block it begins comes between.
    assert_eq!(
        glyph_tops(&format!(
            r#"{before}<div style="margin-top:10px">Q<p style="margin:10px 0 0;width:20px">A</p></div></div><div>Z</div>"#
        )),
        [(1, 'A', 45), (1, 'Q', 5), (1, 'Z', 75)],
    );
}

#[test]
fn the_margin_after_a_generated_box_stays_where_no_break_cuts_it() {
    // Clearance below a float is no margin to cut.
    assert_eq!(
        glyph_tops(
            r#"<style>body::before{content:"";display:block;height:95px}</style><div style="float:left;width:10px;height:150px"></div><p style="clear:left;margin:10px 0 0;width:20px">A</p><div>Z</div>"#
        ),
        [(2, 'A', 50), (3, 'Z', 5)],
    );
    // The border of a box between the generated box and the paragraph.
    assert_eq!(
        glyph_tops(
            r#"<style>body::before{content:"";display:block;height:95px}</style><div style="border-top:20px solid"><p style="margin:10px 0 0;width:20px">A</p></div><div>Z</div>"#
        ),
        [(1, 'A', 35), (1, 'Z', 65)],
    );
    // An empty box between the generated box and the paragraph that ends
    // below it, whose negative margin pulls the paragraph's margin above that
    // end: the page breaks before the paragraph, which keeps its margin.
    for (before, tops) in [
        (
            r#"<style>body::before{content:"";display:block;height:95px}</style><div style="height:10px;margin-bottom:-10px"></div>"#,
            [(1, 'A', 60), (2, 'Z', 5)],
        ),
        (
            r#"<style>body::before{content:"";display:block;height:95px}</style><div style="height:10px;margin-bottom:-20px"></div>"#,
            [(1, 'A', 50), (2, 'Z', 5)],
        ),
        // Where the page end falls in the margins after that box, they are
        // cut as after any box.
        (
            r#"<style>body::before{content:"";display:block;height:90px}</style><div style="height:5px;margin-bottom:-5px"></div>"#,
            [(1, 'A', 45), (1, 'Z', 75)],
        ),
    ] {
        assert_eq!(
            glyph_tops(&format!(
                r#"{before}<p style="margin:20px 0 0;padding-top:40px;width:20px">A</p><div>Z</div>"#
            )),
            tops,
            "{before}"
        );
    }
    // A generated box of no height that a forced break puts at the top of a
    // page starts it: the margin after it is kept.
    for before in [
        r#"<style>.r::before{content:" ";display:table}</style>"#,
        r#"<style>.r::before{content:"";display:block}</style>"#,
    ] {
        assert_eq!(
            glyph_tops(&format!(
                r#"<div style="height:30px"></div>{before}<div class="r" style="break-before:page"><p style="margin:20px 0 0;width:20px">A</p></div><div>Z</div>"#
            )),
            [(1, 'A', 25), (1, 'Z', 55)],
            "{before}"
        );
    }
    // At a page edge that no forced break makes, a generated table of no
    // height starts the page and keeps the margin after it, while an empty
    // generated block lets the margin collapse through it to the edge.
    for (before, tops) in [
        (
            r#"<style>.r::before{content:" ";display:table}</style>"#,
            [(1, 'A', 25), (1, 'Z', 55)],
        ),
        (
            r#"<style>.r::before{content:"";display:block}</style>"#,
            [(1, 'A', 5), (1, 'Z', 35)],
        ),
    ] {
        assert_eq!(
            glyph_tops(&format!(
                r#"<div style="height:100px"></div>{before}<div class="r"><p style="margin:20px 0 0;width:20px">A</p></div><div>Z</div>"#
            )),
            tops,
            "{before}"
        );
    }
}

#[test]
fn a_box_that_avoids_a_break_after_generated_content_moves_whole() {
    // Generated content before the first box gives the page a break before
    // it, as a box there does.
    for before in [
        r#"<style>body::before{content:" ";display:table}</style>"#,
        "<div></div>",
        "<span></span>",
    ] {
        assert_eq!(
            glyph_tops(&format!(
                r#"{before}<div style="break-inside:avoid;margin-top:50px;width:20px">A<br>B</div><div>Z</div>"#
            )),
            [(1, 'A', 5), (1, 'B', 35), (1, 'Z', 65)],
            "{before}"
        );
    }
    assert_eq!(
        glyph_tops(
            r#"<style>body::before{content:"x";display:block}</style><div style="break-inside:avoid;width:20px">A<br>B<br>C</div><div>Z</div>"#
        ),
        [
            (0, 'x', 5),
            (1, 'A', 5),
            (1, 'B', 35),
            (1, 'C', 65),
            (2, 'Z', 5)
        ],
    );
    // With nothing before it, the box breaks inside.
    assert_eq!(
        glyph_tops(
            r#"<div style="break-inside:avoid;margin-top:50px;width:20px">A<br>B</div><div>Z</div>"#
        ),
        [(0, 'A', 55), (1, 'B', 5), (1, 'Z', 35)],
    );
}
