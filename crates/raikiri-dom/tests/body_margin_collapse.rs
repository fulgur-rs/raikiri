//! The block-start margin of `<body>` collapses with the top margins of its
//! first in-flow children when nothing separates them (CSS 2.1, 8.3.1), the
//! `<html>` margin adds to it because margins of the root element's box do
//! not collapse, and the UA `body { margin: 8px }` (HTML, 15.3.3) applies
//! whatever the body holds.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// `0.67em` of the UA `h1` (`font-size: 2em` of the 16px default).
const H1_MARGIN: f32 = 21.44;

/// Border box `(page, x, y, height)` of every fragment of each element of
/// `ids` (or of the first text run, for the id `"#text"`), in source order,
/// after laying out the standards-mode document `html` with Ahem as the only
/// font.
fn fragments(html: &str, id: &str) -> Vec<(u32, f32, f32, f32)> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let mut out = Vec::new();
    for index in 0..result.page_count() {
        let page = result.page(index).expect("page");
        let dom = page.dom();
        for fragment in page.fragments() {
            let node = fragment.node();
            let matches = if id == "#text" {
                dom.local_name(node).is_none()
            } else {
                dom.attr(node, "id") == Some(id)
            };
            if matches {
                let rect = fragment.rect();
                out.push((index, rect.x, rect.y, rect.height));
            }
        }
    }
    out
}

/// Top of the first fragment of `#id` (or of the first text run).
fn top(html: &str, id: &str) -> f32 {
    fragments(html, id)
        .first()
        .unwrap_or_else(|| panic!("no fragment for {id} in {html}"))
        .2
}

fn assert_close(actual: f32, expected: f32, html: &str) {
    assert!(
        (actual - expected).abs() < 0.01,
        "{html}: expected {expected}, got {actual}"
    );
}

#[test]
fn authored_body_margin_collapses_with_the_first_heading() {
    let html = "<!DOCTYPE html><style>body{margin:20px}</style><h1 id=h>A</h1>";
    assert_close(top(html, "h"), H1_MARGIN, html);
    let html = "<!DOCTYPE html><style>body{margin:30px}</style>\n<h1 id=h>A</h1>";
    assert_close(top(html, "h"), 30.0, html);
}

#[test]
fn ua_body_margin_applies_above_a_block_without_margins() {
    let html = "<!DOCTYPE html><div id=d>a</div>";
    assert_close(top(html, "d"), 8.0, html);
    let html = "<!DOCTYPE html><body>\n  <div id=d>a</div>\n</body>";
    assert_close(top(html, "d"), 8.0, html);
}

#[test]
fn a_larger_child_margin_wins_over_the_ua_body_margin() {
    let html = "<!DOCTYPE html><p id=p>a</p>";
    assert_close(top(html, "p"), 16.0, html);
    // Margins escaping through a nested first child collapse too.
    let html = "<!DOCTYPE html><div id=o><p id=p>a</p></div>";
    assert_close(top(html, "o"), 16.0, html);
    assert_close(top(html, "p"), 16.0, html);
}

#[test]
fn top_padding_or_border_keeps_the_margins_apart() {
    let html = "<!DOCTYPE html><style>body{margin:20px;padding-top:1px}</style><h1 id=h>A</h1>";
    assert_close(top(html, "h"), 20.0 + 1.0 + H1_MARGIN, html);
    let html =
        "<!DOCTYPE html><style>body{margin:20px;border-top:2px solid}</style><h1 id=h>A</h1>";
    assert_close(top(html, "h"), 20.0 + 2.0 + H1_MARGIN, html);
}

#[test]
fn a_body_that_establishes_a_formatting_context_keeps_its_margins_apart() {
    let html = "<!DOCTYPE html><style>body{display:flow-root}</style><p id=p>a</p>";
    assert_close(top(html, "p"), 8.0 + 16.0, html);
    let html = "<!DOCTYPE html><style>body{display:flex}</style><p id=p>a</p>";
    assert_close(top(html, "p"), 8.0 + 16.0, html);
    // Overflow on the body propagates to the viewport when the root's is
    // visible, and the body's used value becomes visible.
    let html = "<!DOCTYPE html><style>body{overflow:hidden}</style><p id=p>a</p>";
    assert_close(top(html, "p"), 16.0, html);
    let html = "<!DOCTYPE html><style>html,body{overflow:hidden}</style><p id=p>a</p>";
    assert_close(top(html, "p"), 8.0 + 16.0, html);
}

#[test]
fn negative_margins_collapse_with_the_body_margin() {
    // max(25, 30) + min(-10) = 20
    let html = "<!DOCTYPE html><style>body{margin:25px}</style>\
        <div id=o style='margin-top:30px'><div id=i style='margin-top:-10px'>a</div></div>";
    assert_close(top(html, "o"), 20.0, html);
    assert_close(top(html, "i"), 20.0, html);
    // max(10) + min(-4) = 6
    let html = "<!DOCTYPE html><style>body{margin-top:-4px}</style>\
        <div id=d style='margin-top:10px'>a</div>";
    assert_close(top(html, "d"), 6.0, html);
}

#[test]
fn the_html_margin_adds_to_the_collapsed_body_margin() {
    let html = "<!DOCTYPE html><style>html{margin-top:10px}</style><div id=d>a</div>";
    assert_close(top(html, "d"), 10.0 + 8.0, html);
    let html =
        "<!DOCTYPE html><style>html{margin-top:10px}body{margin:20px}</style><h1 id=h>A</h1>";
    assert_close(top(html, "h"), 10.0 + H1_MARGIN, html);
}

#[test]
fn percentage_body_margins_resolve_against_the_page_content_width() {
    let html = "<!DOCTYPE html><style>body{margin:10%}</style><div id=d>a</div>";
    let (_, x, y, _) = fragments(html, "d")[0];
    assert!(x > 0.0, "{html}: {x}");
    assert_close(y, x, html);
}

#[test]
fn a_body_with_only_text_or_nothing_gets_its_margin() {
    let html = "<!DOCTYPE html><body>text</body>";
    assert_close(top(html, "#text"), 8.0, html);
    let html = "<!DOCTYPE html><body style='background:red'>text</body>";
    assert_close(top(html, "#text"), 8.0, html);
    let html = "<!DOCTYPE html><style>body{margin-top:-5px}</style><body>text</body>";
    assert_close(top(html, "#text"), 0.0, html);
    let html = "<!DOCTYPE html><body></body>";
    assert!(fragments(html, "#text").is_empty());
}

#[test]
fn a_block_that_starts_a_mixed_body_collapses_with_the_body_margin() {
    let html = "<!DOCTYPE html><body><h1 id=h>A</h1>text</body>";
    assert_close(top(html, "h"), H1_MARGIN, html);
    let html = "<!DOCTYPE html><style>body{margin:30px}</style><body><h1 id=h>A</h1>text</body>";
    assert_close(top(html, "h"), 30.0, html);
    // Text before the block keeps them apart.
    let html = "<!DOCTYPE html><style>body{font:10px/10px Ahem}</style>\
        <body>text<div id=d style='margin-top:4px'>a</div></body>";
    assert_close(top(html, "#text"), 8.0, html);
    assert_close(top(html, "d"), 8.0 + 10.0 + 4.0, html);
}

#[test]
fn absolutely_positioned_children_follow_the_body_margin_only_at_their_static_position() {
    let html = "<!DOCTYPE html><div id=a style='position:absolute;top:0'>a</div>\
        <div id=s style='position:absolute'>b</div><p id=p>c</p>";
    assert_close(top(html, "a"), 0.0, html);
    assert_close(top(html, "s"), 16.0, html);
    assert_close(top(html, "p"), 16.0, html);
}

#[test]
fn later_pages_do_not_repeat_the_body_margin() {
    // CSS Fragmentation 3, 5.2: margins adjoining a break are truncated, and
    // the body's top margin belongs to the first page only.
    let html = "<!DOCTYPE html><style>body{margin:20px}</style>\
        <div id=a style='height:2000px'></div>";
    let pieces = fragments(html, "a");
    assert_eq!(pieces.len(), 2, "{pieces:?}");
    assert_eq!((pieces[0].0, pieces[0].2), (0, 20.0));
    assert_eq!((pieces[1].0, pieces[1].2), (1, 0.0));
    let html = "<!DOCTYPE html><style>body{margin:20px}</style>\
        <div id=a style='height:10px'></div><div id=b style='break-before:page'>b</div>";
    assert_eq!(fragments(html, "a")[0].2, 20.0);
    let b = fragments(html, "b");
    assert_eq!((b[0].0, b[0].2), (1, 0.0), "{b:?}");
}

#[test]
fn a_body_margin_taller_than_a_page_starts_the_content_on_a_later_page() {
    let html = "<!DOCTYPE html><style>body{margin-top:1200px}</style><div id=d>a</div>";
    let d = fragments(html, "d");
    assert_eq!((d[0].0, d[0].2), (1, 0.0), "{d:?}");
}
