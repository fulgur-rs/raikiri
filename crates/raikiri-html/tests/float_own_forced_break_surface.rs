//! A float's own `break-before` or `break-after` does not break the page.
//! The float stays beside the content after it on the page it falls on, and
//! a forced break of the boxes around it still applies. Where each glyph
//! falls is what Chromium prints for the same documents on pages 100px
//! square, with Ahem glyphs of 20px on lines of 30px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// The page, glyph, left and top of every glyph of text in `body`, and the
/// number of pages.
fn glyphs(body: &str) -> (Vec<(u32, char, i32, i32)>, u32) {
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
    let mut glyphs = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).unwrap();
        for run in page.text_runs() {
            let top = (run.origin.1 - run.ascent).round() as i32;
            for (offset, glyph) in run.text.chars().enumerate() {
                if glyph != ' ' {
                    let left = (run.origin.0 + 20.0 * offset as f32).round() as i32;
                    glyphs.push((index, glyph, left, top));
                }
            }
        }
    }
    glyphs.sort_unstable();
    (glyphs, result.page_count())
}

const FLOAT: &str = "float:left;width:40px";

#[test]
fn a_float_own_forced_break_leaves_it_on_its_page() {
    let beside = vec![(0, 'F', 0, 35), (0, 'T', 0, 5), (0, 'Z', 40, 35)];
    for (body, expected) in [
        (
            format!("<div>T</div><div style='{FLOAT};break-before:page'>F</div>Z"),
            (beside.clone(), 1),
        ),
        (
            format!("<div>T</div><div style='{FLOAT};break-before:page'>F</div><div>Z</div>"),
            (beside.clone(), 1),
        ),
        (
            format!("<div>T</div><div style='{FLOAT};page-break-before:always'>F</div>Z"),
            (beside.clone(), 1),
        ),
        (
            format!("<div>T</div><div style='{FLOAT};break-before:left'>F</div>Z"),
            (beside.clone(), 1),
        ),
        (
            format!("<div>T</div><div><div style='{FLOAT};break-before:page'>F</div>Z</div>"),
            (beside.clone(), 1),
        ),
        (
            format!("<div>T</div><div style='{FLOAT};break-after:page'>F</div>Z"),
            (beside, 1),
        ),
        // At the top of the first page it opens no empty page.
        (
            format!("<div style='{FLOAT};break-before:page'>F</div>Z"),
            (vec![(0, 'F', 0, 5), (0, 'Z', 40, 5)], 1),
        ),
        (
            "<div>T</div><div style='float:right;width:40px;break-before:page'>F</div>Z".to_owned(),
            (vec![(0, 'F', 60, 35), (0, 'T', 0, 5), (0, 'Z', 0, 35)], 1),
        ),
        // A float that ends in a break of its own and a float that starts
        // with one line up beside each other.
        (
            format!(
                "<div>T</div><div style='{FLOAT};break-before:page;break-after:page'>F</div>\
                 <div style='{FLOAT};break-before:page'>G</div>Z"
            ),
            (
                vec![
                    (0, 'F', 0, 35),
                    (0, 'G', 40, 35),
                    (0, 'T', 0, 5),
                    (0, 'Z', 80, 35),
                ],
                1,
            ),
        ),
    ] {
        assert_eq!(glyphs(&body), expected, "{body}");
    }
}

#[test]
fn forced_breaks_around_a_float_still_apply() {
    for (body, expected) in [
        // The box after the float breaks the page; the float stays.
        (
            format!(
                "<div>T</div><div style='{FLOAT};break-before:page'>F</div>\
                 <div style='break-before:page'>Z</div>"
            ),
            (vec![(0, 'F', 0, 35), (0, 'T', 0, 5), (1, 'Z', 0, 5)], 2),
        ),
        // The float after a box that breaks the page starts the next page.
        (
            format!(
                "<div>T</div><div style='break-before:page'>U</div>\
                 <div style='{FLOAT};break-before:page'>F</div>Z"
            ),
            (
                vec![
                    (0, 'T', 0, 5),
                    (1, 'F', 0, 35),
                    (1, 'U', 0, 5),
                    (1, 'Z', 40, 35),
                ],
                2,
            ),
        ),
        // A change of named page after the float breaks the page there, and
        // the float stays on the page before it.
        (
            format!(
                "<div style='page:foo'>T</div><div style='{FLOAT};break-before:page'>F</div>\
                 <div style='page:bar'>Z</div>"
            ),
            (vec![(0, 'F', 0, 35), (0, 'T', 0, 5), (1, 'Z', 0, 5)], 2),
        ),
        // A flex item floats nothing, so its break applies.
        (
            format!(
                "<div style='display:flex;flex-direction:column'><div>T</div>\
                 <div style='{FLOAT};break-before:page'>F</div><div>Z</div></div>"
            ),
            (vec![(0, 'T', 0, 5), (1, 'F', 0, 5), (1, 'Z', 0, 35)], 2),
        ),
    ] {
        assert_eq!(glyphs(&body), expected, "{body}");
    }
}
