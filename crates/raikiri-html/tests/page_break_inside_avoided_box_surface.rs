//! A box that avoids a break inside it but holds a forced break: the page
//! breaks there anyway, so the box moves to the next page only when the
//! content before that break crosses the page end (CSS Fragmentation 3
//! §3.2, §4.1). A forced break before its first box or after its last one is
//! a break before or after the box itself, and leaves the box to move as any
//! box that avoids a break inside it. Where each glyph falls is what Chromium
//! prints for the same documents on pages 200px wide and 100px tall, with
//! Ahem glyphs of 10px on lines of 20px.

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
        "<!doctype html><style>@page{{size:200px 100px;margin:0}} body{{margin:0;font:10px/20px Ahem}} table{{border-spacing:0}} td{{padding:0;vertical-align:top}}</style><body>{body}</body>"
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
            // Every Ahem glyph is 10px wide.
            for (offset, glyph) in run.text.chars().enumerate() {
                if !glyph.is_whitespace() {
                    glyphs.push((
                        index,
                        glyph,
                        (run.origin.0 + 10.0 * offset as f32).round() as i32,
                        (run.origin.1 - run.ascent).round() as i32,
                    ));
                }
            }
        }
    }
    glyphs.sort_unstable();
    glyphs
}

/// The box above the one that avoids a break inside it: it leaves 50px of
/// the first page.
const ABOVE: &str = r#"<div style="height:50px">W</div>"#;

#[test]
fn a_box_whose_content_before_the_forced_break_fits_stays() {
    for content in [
        // The break on a child, after the line it holds, in a box taller
        // than what is left of the page.
        r#"<div style="height:70px;break-inside:avoid">Y<div style="height:20px;break-before:page">Q</div></div>"#,
        // A break after a child, and a break on a nested box.
        r#"<div style="break-inside:avoid"><div style="height:40px;break-after:page">Y</div><div style="height:20px">Q</div></div>"#,
        r#"<div style="break-inside:avoid"><div style="break-inside:avoid"><div style="height:40px">Y</div><div style="break-before:page">Q</div></div></div>"#,
        // A box in a table cell.
        r#"<table style="border-spacing:0"><tr><td style="padding:0"><div style="height:50px">W</div><div style="height:70px;break-inside:avoid">Y<div style="height:20px;break-before:page">Q</div></div></td></tr></table>"#,
    ] {
        let body = if content.starts_with("<table") {
            format!("{content}<div>X</div>")
        } else {
            format!("{ABOVE}{content}<div>X</div>")
        };
        assert_eq!(
            glyphs(&body),
            [
                (0, 'W', 0, 5),
                (0, 'Y', 0, 55),
                (1, 'Q', 0, 5),
                (1, 'X', 0, 25)
            ],
            "{content}"
        );
    }
    // A box deep inside, with content after it.
    assert_eq!(
        glyphs(&format!(
            r#"{ABOVE}<div style="break-inside:avoid"><div>Y</div><div><div style="break-before:page">Q</div></div><div>Z</div></div><div>X</div>"#
        )),
        [
            (0, 'W', 0, 5),
            (0, 'Y', 0, 55),
            (1, 'Q', 0, 5),
            (1, 'X', 0, 45),
            (1, 'Z', 0, 25)
        ]
    );
}

#[test]
fn a_box_inside_that_names_another_page_breaks_it_as_well() {
    assert_eq!(
        glyphs(&format!(
            r#"{ABOVE}<div style="break-inside:avoid"><div style="height:40px">Y</div><div style="page:named">Q</div></div><div>X</div>"#
        )),
        [
            (0, 'W', 0, 5),
            (0, 'Y', 0, 55),
            (1, 'Q', 0, 5),
            (2, 'X', 0, 5)
        ]
    );
}

#[test]
fn a_box_whose_content_before_the_forced_break_crosses_the_page_end_moves() {
    // A child, and lines.
    assert_eq!(
        glyphs(&format!(
            r#"{ABOVE}<div style="break-inside:avoid"><div style="height:60px">Y</div><div style="break-before:page">Q</div></div><div>X</div>"#
        )),
        [
            (0, 'W', 0, 5),
            (1, 'Y', 0, 5),
            (2, 'Q', 0, 5),
            (2, 'X', 0, 25)
        ]
    );
    assert_eq!(
        glyphs(&format!(
            r#"{ABOVE}<div style="break-inside:avoid">Y<br>Y<br>Y<div style="break-before:page">Q</div></div><div>X</div>"#
        )),
        [
            (0, 'W', 0, 5),
            (1, 'Y', 0, 5),
            (1, 'Y', 0, 25),
            (1, 'Y', 0, 45),
            (2, 'Q', 0, 5),
            (2, 'X', 0, 25)
        ]
    );
}

#[test]
fn a_forced_break_after_the_last_box_breaks_after_the_box() {
    assert_eq!(
        glyphs(&format!(
            r#"{ABOVE}<div style="height:70px;break-inside:avoid"><div style="height:20px">Y</div><div style="break-after:page">Q</div></div><div>X</div>"#
        )),
        [
            (0, 'W', 0, 5),
            (1, 'Q', 0, 25),
            (1, 'Y', 0, 5),
            (2, 'X', 0, 5)
        ]
    );
}
