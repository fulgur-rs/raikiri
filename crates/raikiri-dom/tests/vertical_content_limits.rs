//! Intrinsic sizing keywords in `min-height`, `max-height`, `min-width` and
//! `max-width` of a box in a vertical writing mode (CSS Sizing 3 §5.1). The
//! physical height of such a box is its inline size, so a keyword height
//! limit stands for its min-content or max-content inline size, and a keyword
//! width limit for its block size at the inline size it has.
//!
//! The expected border boxes were recorded from Chromium for the same markup
//! in an 800px by 600px viewport, with the body margin of 8px and Ahem lines
//! 10px tall: `xxxx yy` is 40px at its min-content inline size and 70px at its
//! max-content one.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// A border box `(x, y, width, height)`.
type Rect = (f32, f32, f32, f32);

/// The border box of `#b` in `body` on the first page.
fn border_box(body: &str) -> Rect {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
    );
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(result) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("expected a complete layout");
    };
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    page.fragments()
        .find(|fragment| dom.attr(fragment.node(), "id") == Some("b"))
        .map(|fragment| {
            let rect = fragment.rect();
            (rect.x, rect.y, rect.width, rect.height)
        })
        .expect("a fragment for #b")
}

fn assert_cases(cases: &[(&str, &str, &str, Rect)]) {
    for &(container, style, content, expected) in cases {
        let body = format!(
            "<div style='{container}'><div id=b style='writing-mode:vertical-lr;{style}'>{content}</div></div>"
        );
        let actual = border_box(&body);
        let close = [
            (actual.0, expected.0),
            (actual.1, expected.1),
            (actual.2, expected.2),
            (actual.3, expected.3),
        ]
        .iter()
        .all(|(a, e)| (a - e).abs() < 0.01);
        assert!(close, "{body}: {actual:?}, expected {expected:?}");
    }
}

const TEXT: &str = "xxxx yy";

/// Three items 15px tall and 10px, 15px and 10px wide.
const WRAPPED: &str = "<div style='width:10px;height:15px'></div><div style='width:15px;height:15px'></div><div style='width:10px;height:15px'></div>";

/// One word 500px long.
const LONG: &str = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";

#[test]
fn a_keyword_height_limit_is_an_inline_size_of_the_content() {
    #[rustfmt::skip]
    assert_cases(&[
        ("", "width:100px;height:5px;min-height:min-content", TEXT, (8.0, 8.0, 100.0, 40.0)),
        ("", "width:100px;height:5px;min-height:max-content", TEXT, (8.0, 8.0, 100.0, 70.0)),
        ("", "width:100px;height:5px;min-height:fit-content", TEXT, (8.0, 8.0, 100.0, 70.0)),
        ("", "width:100px;height:300px;max-height:min-content", TEXT, (8.0, 8.0, 100.0, 40.0)),
        ("", "width:100px;height:300px;max-height:max-content", TEXT, (8.0, 8.0, 100.0, 70.0)),
        ("", "width:100px;height:300px;max-height:fit-content", TEXT, (8.0, 8.0, 100.0, 70.0)),
        ("", "width:100px;min-height:max-content", TEXT, (8.0, 8.0, 100.0, 70.0)),
        ("", "width:100px;max-height:min-content", TEXT, (8.0, 8.0, 100.0, 40.0)),
        ("", "writing-mode:vertical-rl;width:100px;height:5px;min-height:min-content", TEXT, (8.0, 8.0, 100.0, 40.0)),
        ("height:200px", "width:100px;height:5px;min-height:stretch", TEXT, (8.0, 8.0, 100.0, 200.0)),
        // The margins of the box take from the room `fit-content` fits in.
        ("height:100px;display:flow-root", "width:100px;height:300px;max-height:fit-content;margin:30px 0", "xxxx yy zz", (8.0, 38.0, 100.0, 40.0)),
        ("position:relative;height:100px", "position:absolute;width:100px;height:300px;max-height:fit-content", "xxxx yy zz", (8.0, 8.0, 100.0, 100.0)),
        ("", "width:100px;height:5px;min-height:min-content", "<div>xxxx yy</div><div>zzzzzz</div>", (8.0, 8.0, 100.0, 60.0)),
    ]);
}

#[test]
fn the_limit_holds_the_edges_of_the_box() {
    #[rustfmt::skip]
    assert_cases(&[
        ("", "width:100px;height:5px;min-height:min-content;padding:3px;border:2px solid;box-sizing:content-box", TEXT, (8.0, 8.0, 110.0, 50.0)),
        ("", "width:100px;height:5px;min-height:min-content;padding:3px;border:2px solid", TEXT, (8.0, 8.0, 100.0, 50.0)),
    ]);
}

#[test]
fn every_kind_of_box_takes_the_limit() {
    #[rustfmt::skip]
    assert_cases(&[
        ("", "display:inline-block;width:100px;height:5px;min-height:min-content", TEXT, (8.0, 8.0, 100.0, 40.0)),
        ("", "float:left;width:100px;height:5px;min-height:min-content", TEXT, (8.0, 8.0, 100.0, 40.0)),
        ("position:relative", "position:absolute;width:100px;height:5px;min-height:min-content", TEXT, (8.0, 8.0, 100.0, 40.0)),
        ("position:relative", "position:absolute;width:100px;height:5px;min-height:max-content", TEXT, (8.0, 8.0, 100.0, 70.0)),
        ("position:relative", "position:absolute;width:100px;height:300px;max-height:min-content", TEXT, (8.0, 8.0, 100.0, 40.0)),
        ("position:relative", "position:absolute;height:30px;width:300px;max-width:min-content", "xxxx yy zz", (8.0, 8.0, 30.0, 30.0)),
        ("display:grid", "width:100px;height:5px;min-height:min-content", TEXT, (8.0, 8.0, 100.0, 40.0)),
        ("display:grid", "width:100px;height:300px;max-height:min-content", TEXT, (8.0, 8.0, 100.0, 40.0)),
        ("display:flex;align-items:start", "width:100px;height:5px;min-height:min-content", TEXT, (8.0, 8.0, 100.0, 40.0)),
        ("display:flex;flex-direction:column;align-items:start", "width:100px;height:5px;min-height:min-content;flex:none", TEXT, (8.0, 8.0, 100.0, 40.0)),
        // A vertical box in a vertical flow resolves the keywords of its own
        // when it is out of that flow.
        ("writing-mode:vertical-lr;height:300px;position:relative", "position:absolute;height:5px;min-height:max-content", "xxxx yy zz", (8.0, 8.0, 10.0, 100.0)),
        ("writing-mode:vertical-lr;height:300px", "float:left;height:5px;min-height:max-content", "xxxx yy zz", (8.0, 8.0, 10.0, 100.0)),
    ]);
}

#[test]
fn a_keyword_width_limit_is_the_block_size_of_the_content() {
    #[rustfmt::skip]
    assert_cases(&[
        ("", "width:5px;height:100px;min-width:min-content", "xxxx yy zz", (8.0, 8.0, 10.0, 100.0)),
        ("", "width:300px;height:30px;max-width:min-content", "xxxx yy zz", (8.0, 8.0, 30.0, 30.0)),
        ("display:flex;align-items:start", "width:300px;height:30px;max-width:min-content", "xxxx yy zz", (8.0, 8.0, 30.0, 30.0)),
        ("display:flex;align-items:start", "width:5px;height:100px;min-width:max-content", "xxxx yy zz", (8.0, 8.0, 10.0, 100.0)),
        // Both axes: the block size is measured at the inline size the
        // height limits give the box.
        ("", "width:5px;height:5px;min-height:max-content;min-width:min-content", TEXT, (8.0, 8.0, 10.0, 70.0)),
        ("position:relative", "position:absolute;width:5px;height:5px;min-height:max-content;min-width:min-content", TEXT, (8.0, 8.0, 10.0, 70.0)),
        // `stretch` fills the room the containing block offers, less the
        // margins.
        ("width:300px", "width:5px;height:100px;min-width:stretch;margin-left:20px", "xx", (28.0, 8.0, 280.0, 100.0)),
        ("width:300px", "width:500px;height:100px;max-width:stretch;margin-left:20px", "xx", (28.0, 8.0, 280.0, 100.0)),
        // The block size is measured at the inline size a percentage height
        // gives the box.
        ("position:relative;height:300px", "position:absolute;height:50%;min-width:min-content", "xxxx yy zz qqqqq rr", (8.0, 8.0, 20.0, 150.0)),
    ]);
}

#[test]
fn a_box_with_a_ratio_takes_the_inline_size_the_ratio_gives() {
    // A `2 / 1` box 100px wide is 50px tall, whatever its stated height.
    #[rustfmt::skip]
    assert_cases(&[
        ("", "aspect-ratio:2/1;width:100px;height:20px;min-height:min-content", "", (8.0, 8.0, 100.0, 50.0)),
        ("display:grid;grid-template-columns:auto;width:min-content", "aspect-ratio:2/1;width:100px;height:1%;min-height:min-content", "", (8.0, 8.0, 100.0, 50.0)),
        ("display:grid;grid-template-columns:auto;grid-template-rows:200px;width:min-content", "aspect-ratio:2/1;width:100px;height:1%;min-height:min-content", "", (8.0, 8.0, 100.0, 50.0)),
        ("display:grid;grid-template-rows:200px;width:300px", "aspect-ratio:2/1;width:100px;height:20px;min-height:min-content", "", (8.0, 8.0, 100.0, 50.0)),
        // The automatic minimum a flex item carries across its ratio stays
        // within its stated size.
        ("display:flex", "aspect-ratio:1/1;width:50px;height:200px", "xxxx yy zz qq", (8.0, 8.0, 50.0, 200.0)),
        // The automatic minimum of a column item keeps the inline size of
        // its content, which an authored keyword gives up for the ratio.
        ("display:flex;flex-direction:column;height:100px", "aspect-ratio:1/1;width:20px", LONG, (8.0, 8.0, 20.0, 500.0)),
        ("display:flex;flex-direction:column;height:100px", "aspect-ratio:1/1;width:20px;min-height:min-content", LONG, (8.0, 8.0, 20.0, 20.0)),
    ]);
}

#[test]
fn a_flex_or_grid_container_takes_the_limit() {
    #[rustfmt::skip]
    assert_cases(&[
        ("", "display:grid;width:100px;height:5px;min-height:min-content", TEXT, (8.0, 8.0, 100.0, 40.0)),
        ("", "display:grid;width:100px;height:5px;min-height:max-content", TEXT, (8.0, 8.0, 100.0, 70.0)),
        ("", "display:grid;width:100px;height:300px;max-height:min-content", TEXT, (8.0, 8.0, 100.0, 40.0)),
        ("", "writing-mode:vertical-rl;display:grid;width:100px;height:5px;min-height:min-content", TEXT, (8.0, 8.0, 100.0, 40.0)),
        ("", "display:grid;width:100px;height:5px;min-height:min-content;padding:3px;border:2px solid", TEXT, (8.0, 8.0, 100.0, 50.0)),
        ("height:100px;display:flow-root", "display:grid;width:100px;height:300px;max-height:fit-content", "xxxx yy zz", (8.0, 8.0, 100.0, 100.0)),
        // The items of a row flex container follow one another along its
        // inline axis, those of a column one across it.
        ("", "display:flex;width:100px;height:5px;min-height:min-content", "<div>xxxx</div><div>yy</div>", (8.0, 8.0, 100.0, 60.0)),
        ("", "display:flex;width:100px;height:5px;min-height:max-content", "<div>xxxx</div><div>yy</div>", (8.0, 8.0, 100.0, 60.0)),
        ("", "display:flex;flex-direction:column;width:100px;height:5px;min-height:min-content", "<div>xxxx yy</div><div>zzz</div>", (8.0, 8.0, 100.0, 40.0)),
        ("", "display:flex;flex-direction:column;width:100px;height:5px;min-height:max-content", "<div>xxxx yy</div><div>zzz</div>", (8.0, 8.0, 100.0, 70.0)),
        ("", "float:left;display:flex;flex-direction:column;width:100px;height:5px;min-height:max-content", "<div>xxxx yy</div><div>zzz</div>", (8.0, 8.0, 100.0, 70.0)),
        ("position:relative", "position:absolute;display:grid;width:100px;height:5px;min-height:max-content", TEXT, (8.0, 8.0, 100.0, 70.0)),
        ("display:grid", "display:grid;width:100px;height:5px;min-height:max-content", TEXT, (8.0, 8.0, 100.0, 70.0)),
        ("display:flex;align-items:start", "display:grid;width:100px;height:5px;min-height:max-content", TEXT, (8.0, 8.0, 100.0, 70.0)),
        // A wrapping column is as tall as its lines side by side are wide at
        // its stated width.
        ("", "display:flex;flex-direction:column;flex-wrap:wrap;width:20px;height:5px;min-height:max-content", WRAPPED, (8.0, 8.0, 20.0, 45.0)),
        // Its min-content height, the tallest line, is not measured: the
        // box keeps its stated height.
        ("", "display:flex;flex-direction:column;flex-wrap:wrap;width:20px;height:30px;min-height:min-content", WRAPPED, (8.0, 8.0, 20.0, 30.0)),
        // A keyword width limit of a column container is the block size of
        // its content; a grid or a row container keeps its stated width.
        ("", "display:grid;width:300px;height:30px;max-width:min-content", "xxxx yy zz", (8.0, 8.0, 300.0, 30.0)),
        ("", "display:flex;width:5px;height:100px;min-width:max-content", "<div>xxxx yy zz</div>", (8.0, 8.0, 5.0, 100.0)),
        ("", "display:flex;width:300px;height:30px;max-width:min-content", "<div>xxxx yy zz</div>", (8.0, 8.0, 300.0, 30.0)),
        ("", "display:flex;flex-direction:column;width:5px;height:100px;min-width:min-content", "<div>xxxx yy zz</div>", (8.0, 8.0, 10.0, 100.0)),
        ("", "display:flex;flex-direction:column;width:300px;height:30px;max-width:min-content", "<div>xxxx yy zz</div>", (8.0, 8.0, 30.0, 30.0)),
    ]);
}

/// Each `(body, expected)` pair: the border box of `#b` in `body`.
fn assert_bodies(cases: &[(&str, Rect)]) {
    for &(body, expected) in cases {
        assert_eq!(border_box(body), expected, "{body}");
    }
}

#[test]
fn an_inline_flex_or_grid_container_takes_the_limit() {
    // Its line offers it no room while the keyword is measured: it takes the
    // min-content or max-content inline size the keyword asks for.
    #[rustfmt::skip]
    assert_bodies(&[
        ("<div style=''><div id=b style='writing-mode:vertical-lr;display:inline-grid;width:100px;height:5px;min-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 100.0, 40.0)),
        ("<div style=''><div id=b style='writing-mode:vertical-lr;display:inline-grid;width:100px;height:5px;min-height:max-content'>xxxx yy</div></div>", (8.0, 8.0, 100.0, 70.0)),
        ("<div style=''><div id=b style='writing-mode:vertical-lr;display:inline-grid;width:100px;height:5px;min-height:fit-content'>xxxx yy</div></div>", (8.0, 8.0, 100.0, 70.0)),
        ("<div style=''><div id=b style='writing-mode:vertical-lr;display:inline-grid;width:100px;height:300px;max-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 100.0, 40.0)),
        ("<div style=''><div id=b style='writing-mode:vertical-lr;writing-mode:vertical-rl;display:inline-grid;width:100px;height:5px;min-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 100.0, 40.0)),
        ("<div style=''><div id=b style='writing-mode:vertical-lr;display:inline-flex;width:100px;height:5px;min-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 100.0, 40.0)),
        ("<div style=''><div id=b style='writing-mode:vertical-lr;display:inline-flex;width:100px;height:5px;min-height:max-content'>xxxx yy</div></div>", (8.0, 8.0, 100.0, 70.0)),
        ("<div style=''><div id=b style='writing-mode:vertical-lr;display:inline-flex;width:100px;height:5px;min-height:fit-content'>xxxx yy</div></div>", (8.0, 8.0, 100.0, 70.0)),
        ("<div style=''><div id=b style='writing-mode:vertical-lr;display:inline-flex;width:100px;height:300px;max-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 100.0, 40.0)),
        ("<div style=''><div id=b style='writing-mode:vertical-lr;display:inline-flex;flex-direction:column;width:100px;height:5px;min-height:min-content'><div>xxxx yy</div><div>zzz</div></div></div>", (8.0, 8.0, 100.0, 40.0)),
        ("<div style=''><div id=b style='writing-mode:vertical-lr;writing-mode:vertical-rl;display:inline-flex;width:100px;height:5px;min-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 100.0, 40.0)),
    ]);
}

#[test]
fn a_vertical_box_in_a_vertical_flow_takes_the_limit() {
    // The container lays out the lines of the box at the inline size the
    // keyword gives, not at the stated height.
    #[rustfmt::skip]
    assert_bodies(&[
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:block;height:5px;min-height:max-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:block;height:5px;min-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 40.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:block;height:300px;max-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 40.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:block;height:5px;min-height:fit-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:block;height:300px;max-height:max-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:block;height:300px;max-height:fit-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:block;max-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 40.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:block;height:5px;min-height:max-content;padding:3px;border:2px solid'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 80.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:block;box-sizing:content-box;height:5px;min-height:max-content;padding:3px;border:2px solid'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 80.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:block;width:5px;min-width:min-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 300.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:block;width:300px;max-width:max-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 300.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:block;height:5px;min-height:max-content'><div>xxxx yy</div><div>zz</div></div></div>", (8.0, 8.0, 20.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:block;height:5px;min-height:max-content;margin-left:4px'>xxxx yy</div></div>", (12.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:flow-root;height:5px;min-height:max-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:flow-root;height:5px;min-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 40.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:flow-root;height:300px;max-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 40.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:flow-root;height:5px;min-height:fit-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:flow-root;height:300px;max-height:max-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:flow-root;height:300px;max-height:fit-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:flow-root;max-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 40.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:flow-root;height:5px;min-height:max-content;padding:3px;border:2px solid'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 80.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:flow-root;box-sizing:content-box;height:5px;min-height:max-content;padding:3px;border:2px solid'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 80.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:flow-root;width:5px;min-width:min-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 300.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:flow-root;width:300px;max-width:max-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 300.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:flow-root;height:5px;min-height:max-content'><div>xxxx yy</div><div>zz</div></div></div>", (8.0, 8.0, 20.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:flow-root;height:5px;min-height:max-content;margin-left:4px'>xxxx yy</div></div>", (12.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:inline-block;height:5px;min-height:max-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:inline-block;height:5px;min-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 40.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:inline-block;height:300px;max-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 40.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:inline-block;height:5px;min-height:fit-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:inline-block;height:300px;max-height:max-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:inline-block;height:300px;max-height:fit-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:inline-block;max-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 40.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:inline-block;height:5px;min-height:max-content;padding:3px;border:2px solid'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 80.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:inline-block;box-sizing:content-box;height:5px;min-height:max-content;padding:3px;border:2px solid'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 80.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:inline-block;width:5px;min-width:min-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:inline-block;width:300px;max-width:max-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:inline-block;height:5px;min-height:max-content'><div>xxxx yy</div><div>zz</div></div></div>", (8.0, 8.0, 20.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:inline-block;height:5px;min-height:max-content;margin-left:4px'>xxxx yy</div></div>", (12.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:flex;height:5px;min-height:max-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:flex;height:5px;min-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 40.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:flex;height:300px;max-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 40.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:flex;height:5px;min-height:fit-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:flex;flex-direction:column;height:5px;min-height:max-content'><div>xxxx yy</div><div>zzz</div></div></div>", (8.0, 8.0, 20.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:grid;height:5px;min-height:max-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:grid;height:5px;min-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 40.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:grid;height:300px;max-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 40.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:grid;height:5px;min-height:fit-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:inline-flex;height:5px;min-height:max-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:inline-flex;height:5px;min-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 40.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:inline-flex;height:300px;max-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 40.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:inline-flex;height:5px;min-height:fit-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:inline-flex;flex-direction:column;height:5px;min-height:max-content'><div>xxxx yy</div><div>zzz</div></div></div>", (8.0, 8.0, 20.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:inline-grid;height:5px;min-height:max-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:inline-grid;height:5px;min-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 40.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:inline-grid;height:300px;max-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 40.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='display:inline-grid;height:5px;min-height:fit-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='height:2%;min-height:max-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='height:5px;margin:auto 0;min-height:max-content'>xxxx yy</div></div>", (8.0, 123.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='height:5px;min-height:max-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='height:2%;min-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 40.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='height:5px;margin:auto 0;min-height:min-content'>xxxx yy</div></div>", (8.0, 138.0, 20.0, 40.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='height:5px;min-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 40.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='height:5px;min-height:max-content;overflow:hidden'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div style='float:left;width:20px;height:30px'></div><div id=b style='height:5px;min-height:max-content'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div style='float:left;width:20px;height:30px'></div><div id=b style='display:flow-root;height:5px;min-height:max-content'>xxxx yy</div></div>", (8.0, 38.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'>aa<div id=b style='height:5px;min-height:max-content'>xxxx yy</div>bb</div>", (18.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='height:5px;min-height:max-content;max-height:50px'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='height:300px;max-height:min-content;min-height:60px'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 60.0)),
        ("<div style='writing-mode:vertical-rl;height:300px'><div id=b style='display:block;height:5px;min-height:max-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-rl;height:300px'><div id=b style='display:inline-block;height:5px;min-height:max-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-rl;height:300px'><div id=b style='display:grid;height:5px;min-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 40.0)),
        ("<div style='writing-mode:vertical-rl;height:300px'><div id=b style='display:inline-flex;height:5px;min-height:max-content'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-rl;height:300px'><div id=b style='display:inline-grid;height:5px;min-height:min-content'>xxxx yy</div></div>", (8.0, 8.0, 20.0, 40.0)),
        ("<div style='writing-mode:vertical-rl;height:300px'><div style='float:left;width:20px;height:30px'></div><div id=b style='display:flow-root;height:5px;min-height:max-content'>xxxx yy</div></div>", (18.0, 38.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-rl;height:300px'>aa<div id=b style='height:5px;min-height:max-content'>xxxx yy</div>bb</div>", (18.0, 8.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-rl;height:300px'><div id=b style='height:5px;min-height:max-content;max-height:50px'>xxxx yy</div></div>", (8.0, 8.0, 10.0, 70.0)),
    ]);
}

#[test]
fn a_vertical_flow_resolves_the_limit_in_the_room_it_leaves() {
    // A length minimum wins over a keyword maximum and the other way around,
    // which places the box by the size it keeps; `fit-content` is offered
    // the room less the margins, or the room beside a float.
    #[rustfmt::skip]
    assert_bodies(&[
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='height:300px;max-height:min-content;min-height:60px;margin:auto 0'>xxxx yy</div></div>", (8.0, 128.0, 20.0, 60.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='height:300px;max-height:min-content;min-height:60px;justify-self:end'>xxxx yy</div></div>", (8.0, 248.0, 20.0, 60.0)),
        ("<div style='writing-mode:vertical-lr;height:300px'><div id=b style='height:5px;min-height:max-content;max-height:50px;margin:auto 0'>xxxx yy</div></div>", (8.0, 123.0, 10.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:100px'><div id=b style='height:300px;max-height:fit-content;margin-top:20px'>xxxx yy zzzzz</div></div>", (8.0, 28.0, 20.0, 80.0)),
        ("<div style='writing-mode:vertical-lr;height:100px'><div id=b style='display:flow-root;height:300px;max-height:fit-content;margin-top:20px'>xxxx yy zzzzz</div></div>", (8.0, 28.0, 20.0, 80.0)),
        ("<div style='writing-mode:vertical-lr;height:100px'><div id=b style='height:5px;min-height:fit-content;margin-top:20px;margin-bottom:10px'>xxxx yy zzzzz</div></div>", (8.0, 28.0, 20.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:100px'><div style='float:left;width:20px;height:30px'></div><div id=b style='display:flow-root;height:300px;max-height:fit-content'>xxxx yy zzzzz</div></div>", (8.0, 38.0, 20.0, 70.0)),
        ("<div style='writing-mode:vertical-lr;height:100px'><div style='float:left;width:20px;height:30px'></div><div id=b style='display:flow-root;height:5px;min-height:fit-content'>xxxx yy zzzzz</div></div>", (8.0, 38.0, 20.0, 70.0)),
    ]);
}
