//! An inline box whose writing mode differs from its parent's is laid out as
//! an `inline-block`: it keeps its own writing mode's axes and takes its size
//! from its content, instead of flowing in the line of its parent.
//!
//! The expected border boxes are the ones Chromium computes for the same
//! markup in an 800px by 600px viewport, with the body margin of 8px.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A border box `(x, y, width, height)`.
type Rect = (f32, f32, f32, f32);

/// The border boxes of the elements of `body` with an `id` on the first page.
fn border_boxes(body: &str) -> Vec<(String, Rect)> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
    );
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
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    page.fragments()
        .filter_map(|fragment| {
            let id = dom.attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((id.to_owned(), (rect.x, rect.y, rect.width, rect.height)))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.02;

fn check(body: &str, expected: &[(&str, Rect)]) {
    let actual = border_boxes(body);
    for &(id, (x, y, width, height)) in expected {
        let found = actual
            .iter()
            .find(|(name, _)| name == id)
            .unwrap_or_else(|| panic!("no fragment for #{id} in {body}"))
            .1;
        let close = |a: f32, b: f32| (a - b).abs() <= TOLERANCE;
        assert!(
            close(found.0, x)
                && close(found.1, y)
                && close(found.2, width)
                && close(found.3, height),
            "{body}: #{id} is {found:?}, expected {:?}",
            (x, y, width, height)
        );
    }
}

#[test]
fn a_vertical_inline_in_a_horizontal_line_is_laid_out_as_an_inline_block() {
    check(
        r#"<div id="p" style="width:200px">x<span id="s" style="writing-mode:vertical-rl">ab</span>y</div>"#,
        &[
            ("p", (8.0, 8.0, 200.0, 22.0)),
            ("s", (18.0, 8.0, 10.0, 20.0)),
        ],
    );
}

#[test]
fn a_horizontal_inline_in_a_vertical_line_is_laid_out_as_an_inline_block() {
    check(
        r#"<div id="p" style="writing-mode:vertical-lr;height:200px">x<span id="s" style="writing-mode:horizontal-tb">ab</span>y</div>"#,
        &[
            ("p", (8.0, 8.0, 20.0, 200.0)),
            ("s", (8.0, 18.0, 20.0, 10.0)),
        ],
    );
}

#[test]
fn an_inline_in_the_other_vertical_mode_is_laid_out_as_an_inline_block() {
    check(
        r#"<div id="p" style="writing-mode:vertical-rl;height:200px">x<span id="s" style="writing-mode:vertical-lr">ab</span>y</div>"#,
        &[
            ("p", (8.0, 8.0, 10.0, 200.0)),
            ("s", (8.0, 18.0, 10.0, 20.0)),
        ],
    );
}

#[test]
fn a_floated_inline_in_another_writing_mode_is_a_block() {
    check(
        r#"<div id="p" style="width:200px"><span id="s" style="writing-mode:vertical-rl;float:left">ab</span>y</div>"#,
        &[
            ("p", (8.0, 8.0, 200.0, 10.0)),
            ("s", (8.0, 8.0, 10.0, 20.0)),
        ],
    );
}
