//! An intrinsic `min-height` or `max-height` keyword of a box with an
//! `aspect-ratio` stands for the height the ratio gives its width, even when
//! the box states its height too, which leaves the ratio nothing to do for its
//! own size (CSS Sizing 3 §5.1, CSS Sizing 4 §5.1): a `2 / 1` box 100px wide
//! with `height: 20px; min-height: min-content` is 50px tall.
//!
//! The expected border boxes were recorded from Chromium for the same markup in
//! an 800px by 600px viewport, with the body margin of 8px.

use raikiri_html::{LayoutOptions, LayoutStatus, layout, parse_html};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// A border box `(x, y, width, height)`.
type Rect = (f32, f32, f32, f32);

/// The border box of `#b` in `body` on the first page.
fn border_box(body: &str) -> Rect {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px monospace }} div {{ box-sizing: border-box }}</style>{body}"
    );
    let doc = parse_html(html.as_bytes(), &Default::default()).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new(),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
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

/// Each case: the container, the style of `#b`, its content and its box.
#[rustfmt::skip]
const CASES: &[(&str, &str, &str, Rect)] = &[
    ("width:300px", "width:100px;height:20px;min-height:min-content", "", (8.0, 8.0, 100.0, 50.0)),
    ("width:300px", "width:100px;height:100px;max-height:min-content", "", (8.0, 8.0, 100.0, 50.0)),
    ("width:300px", "width:100px;height:100px;max-height:fit-content", "", (8.0, 8.0, 100.0, 50.0)),
    ("width:300px", "width:50%;height:20px;min-height:min-content", "", (8.0, 8.0, 150.0, 75.0)),
    ("width:300px", "width:100px;height:20px;min-height:min-content;padding:10px;box-sizing:content-box", "", (8.0, 8.0, 120.0, 70.0)),
    ("width:300px", "display:inline-block;width:100px;height:20px;min-height:min-content", "", (8.0, 8.0, 100.0, 50.0)),
    ("position:relative;width:300px", "position:absolute;width:100px;height:20px;min-height:min-content", "", (8.0, 8.0, 100.0, 50.0)),
    ("display:grid;width:min-content", "width:100px;height:20px;min-height:min-content", "", (8.0, 8.0, 100.0, 50.0)),
    ("display:grid;width:min-content", "height:1%;width:100px;min-height:min-content", "", (8.0, 8.0, 100.0, 50.0)),
    ("display:flex;width:300px;align-items:start", "width:100px;height:20px;min-height:min-content", "", (8.0, 8.0, 100.0, 50.0)),
    ("width:300px", "width:100px;height:20px;min-height:min-content", "x<br>x<br>x<br>x<br>x<br>x<br>x", (8.0, 8.0, 100.0, 50.0)),
    ("width:300px", "width:100px;min-height:min-content", "x<br>x<br>x<br>x<br>x<br>x<br>x", (8.0, 8.0, 100.0, 50.0)),
    ("width:300px", "width:100px;max-height:max-content", "x<br>x<br>x<br>x<br>x<br>x<br>x", (8.0, 8.0, 100.0, 50.0)),
    ("width:300px", "width:100px;height:20px;min-height:max-content", "x<br>x<br>x<br>x<br>x<br>x<br>x", (8.0, 8.0, 100.0, 50.0)),
    ("display:grid;width:300px;align-items:start", "width:100px;min-height:min-content", "x<br>x<br>x<br>x<br>x<br>x<br>x", (8.0, 8.0, 100.0, 50.0)),
    ("width:300px", "width:100px", "x<br>x<br>x<br>x<br>x<br>x<br>x", (8.0, 8.0, 100.0, 70.0)),
    ("width:300px", "width:100px;height:20px;min-height:auto", "", (8.0, 8.0, 100.0, 20.0)),
];

#[test]
fn an_intrinsic_height_limit_is_the_height_the_ratio_gives() {
    for &(container, style, content, expected) in CASES {
        let body = format!(
            "<div style='{container}'><div id=b style='aspect-ratio:2/1;{style}'>{content}</div></div>"
        );
        assert_eq!(border_box(&body), expected, "{body}");
    }
}
