//! A forced page break before the first box of the flow, after content that
//! places no box of the flow: generated content of the body or of an element
//! with no box of its own, an empty box, or the phantom line of an empty
//! inline element. That content comes before the box, so the break opens the
//! page after the one the content ends on, as it does after a box, and a page
//! type the box names opens a page after it too. Nothing but margins or edges
//! before the box gives no break. Where each glyph falls is what Chromium
//! prints for the same documents on pages 100px square, with Ahem glyphs of
//! 20px on lines of 30px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The page, glyph and glyph top of every word of text in `body`, laid out
/// on the pages and with the font of the Chromium documents.
fn glyph_tops(body: &str) -> Vec<(u32, char, i32)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:100px 100px;margin:0}} @page n{{size:100px 100px;margin:0}} body{{margin:0;font:20px/30px Ahem}} .n{{page:n}}</style><body>{body}</body>"
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

const BREAK: &str = r#"<p style="break-before:page;margin:0;width:20px">A</p><div>Z</div>"#;

#[test]
fn a_forced_break_after_generated_content_opens_the_next_page() {
    for (name, before) in [
        (
            "body",
            "<style>body::before{content:\"x\";display:block}</style>",
        ),
        ("inline body", "<style>body::before{content:\"x\"}</style>"),
        (
            "empty span",
            r#"<style>.s::before{content:"x";display:block}</style><span class="s"></span>"#,
        ),
        (
            "empty span after",
            r#"<style>.s::after{content:"x";display:block}</style><span class="s"></span>"#,
        ),
        (
            "inline in empty span",
            r#"<style>.s::before{content:"x"}</style><span class="s"></span>"#,
        ),
        (
            "contents sibling",
            r#"<style>.c::before{content:"x";display:block}</style><div class="c" style="display:contents"></div>"#,
        ),
    ] {
        assert_eq!(
            glyph_tops(&format!("{before}{BREAK}")),
            [(0, 'x', 5), (1, 'A', 5), (1, 'Z', 35)],
            "{name}"
        );
    }
    // The ::before of a contents ancestor, and a box inside a wrapper.
    assert_eq!(
        glyph_tops(
            r#"<style>.c::before{content:"x";display:block}</style><div class="c" style="display:contents"><p style="break-before:page;margin:0;width:20px">A</p></div><div>Z</div>"#
        ),
        [(0, 'x', 5), (1, 'A', 5), (1, 'Z', 35)],
    );
    assert_eq!(
        glyph_tops(
            r#"<style>body::before{content:"x";display:block}</style><div><p style="break-before:page;margin:0;width:20px">A</p></div><div>Z</div>"#
        ),
        [(0, 'x', 5), (1, 'A', 5), (1, 'Z', 35)],
    );
    // A forced break keeps the margin of the box on the new page.
    assert_eq!(
        glyph_tops(
            r#"<style>body::before{content:"x";display:block}</style><p style="break-before:page;margin:10px 0 0;width:20px">A</p><div>Z</div>"#
        ),
        [(0, 'x', 5), (1, 'A', 15), (1, 'Z', 45)],
    );
}

#[test]
fn the_break_opens_the_page_after_the_one_the_content_ends_on() {
    for (height, page) in [(10, 1), (70, 1), (100, 1), (120, 2), (140, 2), (200, 2)] {
        assert_eq!(
            glyph_tops(&format!(
                "<style>body::before{{content:\"\";display:block;height:{height}px}}</style>{BREAK}"
            )),
            [(page, 'A', 5), (page, 'Z', 35)],
            "{height}px"
        );
    }
    assert_eq!(
        glyph_tops(&format!(
            r#"<style>.s::before{{content:"";display:block;height:140px}}</style><span class="s"></span>{BREAK}"#
        )),
        [(2, 'A', 5), (2, 'Z', 35)],
    );
    assert_eq!(
        glyph_tops(
            r#"<style>body::before{content:"";display:block;height:140px}</style><p style="break-before:page;margin:10px 0 0;width:20px">A</p><div>Z</div>"#
        ),
        [(2, 'A', 15), (2, 'Z', 45)],
    );
}

#[test]
fn content_of_no_height_on_the_first_page_comes_before_the_box() {
    for (name, before) in [
        ("empty div", "<div></div>"),
        ("phantom line", "<span></span>"),
        (
            "empty generated box",
            r#"<style>.s::before{content:"";display:block}</style><span class="s"></span>"#,
        ),
    ] {
        assert_eq!(
            glyph_tops(&format!("{before}{BREAK}")),
            [(1, 'A', 5), (1, 'Z', 35)],
            "{name}"
        );
    }
    // On a later page, such content stays at the end of the page before, where
    // the break already is.
    assert_eq!(
        glyph_tops(&format!(
            r#"<div style="height:100px"></div><div></div>{BREAK}"#
        )),
        [(1, 'A', 5), (1, 'Z', 35)],
    );
}

#[test]
fn nothing_but_margins_or_edges_before_the_box_gives_no_break() {
    assert_eq!(glyph_tops(BREAK), [(0, 'A', 5), (0, 'Z', 35)]);
    assert_eq!(
        glyph_tops(&format!("<style>body{{padding-top:10px}}</style>{BREAK}")),
        [(0, 'A', 15), (0, 'Z', 45)],
    );
}

#[test]
fn the_page_follows_where_the_content_ends_not_its_margin() {
    // The margin below the generated box ends on the next page, the box
    // itself on the first: the break opens the second page.
    assert_eq!(
        glyph_tops(&format!(
            r#"<style>body::before{{content:"";display:block;height:90px;margin-bottom:30px}}</style>{BREAK}"#
        )),
        [(1, 'A', 5), (1, 'Z', 35)],
    );
}

#[test]
fn a_box_out_of_the_flow_gives_no_break() {
    let generated = r#"<style>body::before{content:"x";display:block}</style>"#;
    assert_eq!(
        glyph_tops(&format!(
            r#"{generated}<div style="position:absolute;top:0;left:50px;break-before:page">A</div><div>Z</div>"#
        )),
        [(0, 'A', 5), (0, 'Z', 35), (0, 'x', 5)],
    );
}

#[test]
fn a_page_type_after_generated_content_opens_a_page() {
    let named = r#"<p class="n" style="margin:0;width:20px">A</p><div>Z</div>"#;
    for (name, before) in [
        (
            "body",
            "<style>body::before{content:\"x\";display:block}</style>",
        ),
        ("inline body", "<style>body::before{content:\"x\"}</style>"),
        (
            "empty span",
            r#"<style>.s::before{content:"x";display:block}</style><span class="s"></span>"#,
        ),
        (
            "contents sibling",
            r#"<style>.c::before{content:"x";display:block}</style><div class="c" style="display:contents"></div>"#,
        ),
    ] {
        assert_eq!(
            glyph_tops(&format!("{before}{named}")),
            [(0, 'x', 5), (1, 'A', 5), (2, 'Z', 5)],
            "{name}"
        );
    }
    for before in ["<div></div>", "<span></span>"] {
        assert_eq!(
            glyph_tops(&format!("{before}{named}")),
            [(1, 'A', 5), (2, 'Z', 5)],
            "{before}"
        );
    }
    assert_eq!(glyph_tops(named), [(0, 'A', 5), (1, 'Z', 5)]);
}
