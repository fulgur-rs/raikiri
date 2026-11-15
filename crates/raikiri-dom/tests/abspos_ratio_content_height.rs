//! An absolutely positioned box with a preferred aspect ratio and `auto`
//! sizes:
//!
//! - Its `auto` height is the height its ratio gives the width its content
//!   settles, also when the box holds lines, raised by the height of its
//!   content.
//! - A `max-width` carried across the ratio does not clamp the height of the
//!   content (CSS Sizing 4 §5.1), with or without insets.
//! - A ratio that relates the content boxes carries a `max-height` across
//!   between the content boxes.
//!
//! The expected border boxes `[x, y, width, height]` of `#t` are the ones
//! Chromium computes for the same markup in an 800px by 600px viewport with
//! no body margin.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The border box `[x, y, width, height]` of `#t` on the first page of an
/// 800x600 document that holds `body`.
fn target_box(body: &str) -> Option<[f32; 4]> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }}</style>{body}"
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
    page.fragments().find_map(|fragment| {
        (dom.attr(fragment.node(), "id") == Some("t")).then(|| {
            let rect = fragment.rect();
            [rect.x, rect.y, rect.width, rect.height]
        })
    })
}

fn assert_rows(rows: &[(&str, [f32; 4])]) {
    let failures: Vec<_> = rows
        .iter()
        .filter_map(|(body, want)| {
            let got = target_box(body);
            let matches = got.is_some_and(|got| {
                got.iter()
                    .zip(want)
                    .all(|(got, want)| (got - want).abs() <= 0.5)
            });
            (!matches).then(|| format!("{body}\n  expected {want:?}, got {got:?}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A `max-width` carried across the ratio does not clamp the height of the
/// content, with and without insets and in both box-sizing models.
#[test]
fn a_carried_maximum_height_does_not_clamp_the_content() {
    assert_rows(&[
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px'>X</div></div>",
            [0.0, 0.0, 10.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px'>XXXXX</div></div>",
            [0.0, 0.0, 10.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px'>X</div></div>",
            [0.0, 0.0, 10.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px'>XXXXX</div></div>",
            [0.0, 0.0, 10.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:2/1;max-width:10px'>X</div></div>",
            [5.0, 5.0, 10.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:2/1;max-width:10px'>XX XX</div></div>",
            [5.0, 5.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:2/1;max-width:10px'>XXXXX</div></div>",
            [5.0, 5.0, 10.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:2/1;max-width:10px'>X</div></div>",
            [0.0, 0.0, 10.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:2/1;max-width:10px'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:2/1;max-width:10px'>XXXXX</div></div>",
            [0.0, 0.0, 10.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:2/1;max-width:10px'>X</div></div>",
            [0.0, 0.0, 10.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:2/1;max-width:10px'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:2/1;max-width:10px'>XXXXX</div></div>",
            [0.0, 0.0, 10.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:2/1;max-width:10px'>X</div></div>",
            [5.0, 5.0, 10.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:2/1;max-width:10px'>XX XX</div></div>",
            [5.0, 5.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:2/1;max-width:10px'>XXXXX</div></div>",
            [5.0, 5.0, 10.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:20px'>X</div></div>",
            [0.0, 0.0, 10.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:20px'>XX XX</div></div>",
            [0.0, 0.0, 20.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:20px'>XXXXX</div></div>",
            [0.0, 0.0, 20.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:20px'>X</div></div>",
            [0.0, 0.0, 20.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:20px'>XX XX</div></div>",
            [0.0, 0.0, 20.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:20px'>XXXXX</div></div>",
            [0.0, 0.0, 20.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:2/1;max-width:20px'>X</div></div>",
            [0.0, 0.0, 20.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:2/1;max-width:20px'>XXXXX</div></div>",
            [0.0, 0.0, 20.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:2/1;max-width:20px'>X</div></div>",
            [5.0, 5.0, 10.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:2/1;max-width:20px'>XX XX</div></div>",
            [5.0, 5.0, 20.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:2/1;max-width:20px'>XXXXX</div></div>",
            [5.0, 5.0, 20.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:2/1;max-width:20px'>X</div></div>",
            [0.0, 0.0, 10.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:2/1;max-width:20px'>XX XX</div></div>",
            [0.0, 0.0, 20.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:2/1;max-width:20px'>XXXXX</div></div>",
            [0.0, 0.0, 20.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:2/1;max-width:20px'>X</div></div>",
            [0.0, 0.0, 20.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:2/1;max-width:20px'>XX XX</div></div>",
            [0.0, 0.0, 20.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:2/1;max-width:20px'>XXXXX</div></div>",
            [0.0, 0.0, 20.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;box-sizing:border-box;aspect-ratio:2/1;max-width:20px'>X</div></div>",
            [0.0, 0.0, 20.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;box-sizing:border-box;aspect-ratio:2/1;max-width:20px'>XXXXX</div></div>",
            [0.0, 0.0, 20.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:2/1;max-width:20px'>X</div></div>",
            [5.0, 5.0, 10.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:2/1;max-width:20px'>XX XX</div></div>",
            [5.0, 5.0, 20.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:2/1;max-width:20px'>XXXXX</div></div>",
            [5.0, 5.0, 20.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-height:8px;padding:2px'>X</div></div>",
            [0.0, 0.0, 14.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-height:8px;padding:2px'>XX XX</div></div>",
            [0.0, 0.0, 20.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-height:8px;padding:2px'>XXXXX</div></div>",
            [0.0, 0.0, 20.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-height:8px;padding:2px'>X</div></div>",
            [0.0, 0.0, 20.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-height:8px;padding:2px'>XX XX</div></div>",
            [0.0, 0.0, 20.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-height:8px;padding:2px'>XXXXX</div></div>",
            [0.0, 0.0, 20.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:2/1;max-height:8px;padding:2px'>X</div></div>",
            [0.0, 0.0, 20.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:2/1;max-height:8px;padding:2px'>XX XX</div></div>",
            [0.0, 0.0, 24.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:2/1;max-height:8px;padding:2px'>XXXXX</div></div>",
            [0.0, 0.0, 54.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:2/1;max-height:8px;padding:2px'>X</div></div>",
            [5.0, 5.0, 14.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:2/1;max-height:8px;padding:2px'>XX XX</div></div>",
            [5.0, 5.0, 20.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:2/1;max-height:8px;padding:2px'>XXXXX</div></div>",
            [5.0, 5.0, 20.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:2/1;max-height:8px;padding:2px'>X</div></div>",
            [0.0, 0.0, 14.0, 8.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:2/1;max-height:8px;padding:2px'>XX XX</div></div>",
            [0.0, 0.0, 16.0, 8.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:2/1;max-height:8px;padding:2px'>XXXXX</div></div>",
            [0.0, 0.0, 16.0, 8.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:2/1;max-height:8px;padding:2px'>X</div></div>",
            [0.0, 0.0, 16.0, 8.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:2/1;max-height:8px;padding:2px'>XX XX</div></div>",
            [0.0, 0.0, 16.0, 8.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:2/1;max-height:8px;padding:2px'>XXXXX</div></div>",
            [0.0, 0.0, 16.0, 8.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;box-sizing:border-box;aspect-ratio:2/1;max-height:8px;padding:2px'>X</div></div>",
            [0.0, 0.0, 16.0, 8.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;box-sizing:border-box;aspect-ratio:2/1;max-height:8px;padding:2px'>XX XX</div></div>",
            [0.0, 0.0, 24.0, 8.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;box-sizing:border-box;aspect-ratio:2/1;max-height:8px;padding:2px'>XXXXX</div></div>",
            [0.0, 0.0, 54.0, 8.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:2/1;max-height:8px;padding:2px'>X</div></div>",
            [5.0, 5.0, 14.0, 8.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:2/1;max-height:8px;padding:2px'>XX XX</div></div>",
            [5.0, 5.0, 16.0, 8.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:2/1;max-height:8px;padding:2px'>XXXXX</div></div>",
            [5.0, 5.0, 16.0, 8.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/1;max-width:10px;border:3px solid'>X</div></div>",
            [0.0, 0.0, 16.0, 16.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/1;max-width:10px;border:3px solid'>XX XX</div></div>",
            [0.0, 0.0, 16.0, 26.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/1;max-width:10px;border:3px solid'>XXXXX</div></div>",
            [0.0, 0.0, 16.0, 16.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:1/1;max-width:10px;border:3px solid'>X</div></div>",
            [0.0, 0.0, 16.0, 16.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:1/1;max-width:10px;border:3px solid'>XX XX</div></div>",
            [0.0, 0.0, 16.0, 26.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:1/1;max-width:10px;border:3px solid'>XXXXX</div></div>",
            [0.0, 0.0, 16.0, 16.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:1/1;max-width:10px;border:3px solid'>X</div></div>",
            [0.0, 0.0, 16.0, 16.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:1/1;max-width:10px;border:3px solid'>XXXXX</div></div>",
            [0.0, 0.0, 16.0, 16.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:1/1;max-width:10px;border:3px solid'>X</div></div>",
            [5.0, 5.0, 16.0, 16.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:1/1;max-width:10px;border:3px solid'>XX XX</div></div>",
            [5.0, 5.0, 16.0, 26.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:1/1;max-width:10px;border:3px solid'>XXXXX</div></div>",
            [5.0, 5.0, 16.0, 16.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:1/1;max-width:10px;border:3px solid'>X</div></div>",
            [0.0, 0.0, 10.0, 16.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:1/1;max-width:10px;border:3px solid'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 26.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:1/1;max-width:10px;border:3px solid'>XXXXX</div></div>",
            [0.0, 0.0, 10.0, 16.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:1/1;max-width:10px;border:3px solid'>X</div></div>",
            [0.0, 0.0, 10.0, 16.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:1/1;max-width:10px;border:3px solid'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 26.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:1/1;max-width:10px;border:3px solid'>XXXXX</div></div>",
            [0.0, 0.0, 10.0, 16.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:1/1;max-width:10px;border:3px solid'>X</div></div>",
            [5.0, 5.0, 10.0, 16.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:1/1;max-width:10px;border:3px solid'>XX XX</div></div>",
            [5.0, 5.0, 10.0, 26.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:1/1;max-width:10px;border:3px solid'>XXXXX</div></div>",
            [5.0, 5.0, 10.0, 16.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;min-height:0'>X</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;min-height:0'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;min-height:0'>XXXXX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;min-height:0'>X</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;min-height:0'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;min-height:0'>XXXXX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:2/1;max-width:10px;min-height:0'>X</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:2/1;max-width:10px;min-height:0'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:2/1;max-width:10px;min-height:0'>XXXXX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:2/1;max-width:10px;min-height:0'>X</div></div>",
            [5.0, 5.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:2/1;max-width:10px;min-height:0'>XX XX</div></div>",
            [5.0, 5.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:2/1;max-width:10px;min-height:0'>XXXXX</div></div>",
            [5.0, 5.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;min-height:0'>X</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;min-height:0'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;min-height:0'>XXXXX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;min-height:0'>X</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;min-height:0'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;min-height:0'>XXXXX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;min-height:0'>X</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;min-height:0'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;min-height:0'>XXXXX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;min-height:0'>X</div></div>",
            [5.0, 5.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;min-height:0'>XX XX</div></div>",
            [5.0, 5.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;min-height:0'>XXXXX</div></div>",
            [5.0, 5.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;overflow:hidden'>X</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;overflow:hidden'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;overflow:hidden'>XXXXX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;overflow:hidden'>X</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;overflow:hidden'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;overflow:hidden'>XXXXX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:2/1;max-width:10px;overflow:hidden'>X</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:2/1;max-width:10px;overflow:hidden'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:2/1;max-width:10px;overflow:hidden'>XXXXX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:2/1;max-width:10px;overflow:hidden'>X</div></div>",
            [5.0, 5.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:2/1;max-width:10px;overflow:hidden'>XX XX</div></div>",
            [5.0, 5.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:2/1;max-width:10px;overflow:hidden'>XXXXX</div></div>",
            [5.0, 5.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;overflow:hidden'>X</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;overflow:hidden'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;overflow:hidden'>XXXXX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;overflow:hidden'>X</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;overflow:hidden'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;overflow:hidden'>XXXXX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;overflow:hidden'>X</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;overflow:hidden'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;overflow:hidden'>XXXXX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;overflow:hidden'>X</div></div>",
            [5.0, 5.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;overflow:hidden'>XX XX</div></div>",
            [5.0, 5.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:2/1;max-width:10px;overflow:hidden'>XXXXX</div></div>",
            [5.0, 5.0, 10.0, 5.0],
        ),
    ]);
}

/// The height the ratio gives the width of lines, and a `max-height` carried
/// across between the content boxes.
#[test]
fn the_ratio_gives_the_height_of_a_box_with_lines() {
    assert_rows(&[
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2;max-height:30px;padding:2px'>X</div></div>",
            [0.0, 0.0, 14.0, 24.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2;max-height:30px;padding:2px'>XX XX</div></div>",
            [0.0, 0.0, 19.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2;max-height:30px;padding:2px'>XXXXX</div></div>",
            [0.0, 0.0, 19.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:1/2;max-height:30px;padding:2px'>X</div></div>",
            [0.0, 0.0, 19.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:1/2;max-height:30px;padding:2px'>XX XX</div></div>",
            [0.0, 0.0, 19.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:1/2;max-height:30px;padding:2px'>XXXXX</div></div>",
            [0.0, 0.0, 19.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:1/2;max-height:30px;padding:2px'>X</div></div>",
            [0.0, 0.0, 19.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:1/2;max-height:30px;padding:2px'>XX XX</div></div>",
            [0.0, 0.0, 24.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:1/2;max-height:30px;padding:2px'>XXXXX</div></div>",
            [0.0, 0.0, 54.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:1/2;max-height:30px;padding:2px'>X</div></div>",
            [5.0, 5.0, 14.0, 24.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:1/2;max-height:30px;padding:2px'>XX XX</div></div>",
            [5.0, 5.0, 19.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:1/2;max-height:30px;padding:2px'>XXXXX</div></div>",
            [5.0, 5.0, 19.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px'>X</div></div>",
            [0.0, 0.0, 14.0, 28.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px'>XX XX</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px'>XXXXX</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px'>X</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px'>XX XX</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px'>XXXXX</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px'>X</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px'>XX XX</div></div>",
            [0.0, 0.0, 24.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px'>XXXXX</div></div>",
            [0.0, 0.0, 54.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px'>X</div></div>",
            [5.0, 5.0, 14.0, 28.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px'>XX XX</div></div>",
            [5.0, 5.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px'>XXXXX</div></div>",
            [5.0, 5.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>X</div></div>",
            [0.0, 0.0, 20.0, 24.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>XX XX</div></div>",
            [0.0, 0.0, 25.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>XXXXX</div></div>",
            [0.0, 0.0, 25.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>X</div></div>",
            [0.0, 0.0, 25.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>XX XX</div></div>",
            [0.0, 0.0, 25.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>XXXXX</div></div>",
            [0.0, 0.0, 25.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>X</div></div>",
            [0.0, 0.0, 25.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>XX XX</div></div>",
            [0.0, 0.0, 30.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>XXXXX</div></div>",
            [0.0, 0.0, 60.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>X</div></div>",
            [5.0, 5.0, 20.0, 24.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>XX XX</div></div>",
            [5.0, 5.0, 25.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>XXXXX</div></div>",
            [5.0, 5.0, 25.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>X</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>XX XX</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>XXXXX</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>X</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>XX XX</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>XXXXX</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>X</div></div>",
            [0.0, 0.0, 20.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>XX XX</div></div>",
            [0.0, 0.0, 30.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>XXXXX</div></div>",
            [0.0, 0.0, 60.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>X</div></div>",
            [5.0, 5.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>XX XX</div></div>",
            [5.0, 5.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:1/2;max-height:30px;padding:2px 5px'>XXXXX</div></div>",
            [5.0, 5.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2;max-height:30px'>X</div></div>",
            [0.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2;max-height:30px'>XX XX</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2;max-height:30px'>XXXXX</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:1/2;max-height:30px'>X</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:1/2;max-height:30px'>XX XX</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:1/2;max-height:30px'>XXXXX</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:1/2;max-height:30px'>X</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:1/2;max-height:30px'>XX XX</div></div>",
            [0.0, 0.0, 20.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:1/2;max-height:30px'>XXXXX</div></div>",
            [0.0, 0.0, 50.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:1/2;max-height:30px'>X</div></div>",
            [5.0, 5.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:1/2;max-height:30px'>XX XX</div></div>",
            [5.0, 5.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;aspect-ratio:1/2;max-height:30px'>XXXXX</div></div>",
            [5.0, 5.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:1/2;max-height:30px'>X</div></div>",
            [0.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:1/2;max-height:30px'>XX XX</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;box-sizing:border-box;aspect-ratio:1/2;max-height:30px'>XXXXX</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:1/2;max-height:30px'>X</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:1/2;max-height:30px'>XX XX</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;box-sizing:border-box;aspect-ratio:1/2;max-height:30px'>XXXXX</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;box-sizing:border-box;aspect-ratio:1/2;max-height:30px'>X</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;box-sizing:border-box;aspect-ratio:1/2;max-height:30px'>XX XX</div></div>",
            [0.0, 0.0, 20.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;box-sizing:border-box;aspect-ratio:1/2;max-height:30px'>XXXXX</div></div>",
            [0.0, 0.0, 50.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:1/2;max-height:30px'>X</div></div>",
            [5.0, 5.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:1/2;max-height:30px'>XX XX</div></div>",
            [5.0, 5.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:5px;top:5px;box-sizing:border-box;aspect-ratio:1/2;max-height:30px'>XXXXX</div></div>",
            [5.0, 5.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2'>X</div></div>",
            [0.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2'>XX XX</div></div>",
            [0.0, 0.0, 50.0, 100.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2;max-height:300px'>X</div></div>",
            [0.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2;max-height:300px'>XX XX</div></div>",
            [0.0, 0.0, 50.0, 100.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2;max-height:30px'>X</div></div>",
            [0.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2;max-height:30px'>XX XX</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1'>X</div></div>",
            [0.0, 0.0, 10.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1'>XX XX</div></div>",
            [0.0, 0.0, 50.0, 25.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:100px'>X</div></div>",
            [0.0, 0.0, 10.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:100px'>XX XX</div></div>",
            [0.0, 0.0, 50.0, 25.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2;max-width:100px'>X</div></div>",
            [0.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2;max-width:100px'>XX XX</div></div>",
            [0.0, 0.0, 50.0, 100.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-height:300px'>X</div></div>",
            [0.0, 0.0, 10.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-height:300px'>XX XX</div></div>",
            [0.0, 0.0, 50.0, 25.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2;max-height:30px;max-width:100px'>X</div></div>",
            [0.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2;max-height:30px;max-width:100px'>XX XX</div></div>",
            [0.0, 0.0, 15.0, 30.0],
        ),
    ]);
}

/// Block, grid, flex and fixed boxes, percentages, minimums, margins,
/// direction and scroll containers next to the cases above.
#[test]
fn neighbouring_boxes_follow_the_same_rules() {
    assert_rows(&[
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 6.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 4.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 10.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 10.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px'><div style='width:6px;height:12px'></div></div></div>",
            [145.0, 0.0, 10.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px'>XX XX</div></div>",
            [145.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [145.0, 0.0, 10.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:5%'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 6.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:5%'>XX XX</div></div>",
            [0.0, 0.0, 15.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:5%'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 4.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;min-width:4px'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 6.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;min-width:4px'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;min-width:4px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 4.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;min-width:4px'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 10.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;min-width:4px'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;min-width:4px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 10.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px;min-width:4px'><div style='width:6px;height:12px'></div></div></div>",
            [145.0, 0.0, 10.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px;min-width:4px'>XX XX</div></div>",
            [145.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px;min-width:4px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [145.0, 0.0, 10.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;max-height:15px'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 6.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;max-height:15px'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;max-height:15px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 4.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;max-height:15px'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 10.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;max-height:15px'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;max-height:15px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 10.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:2/1;max-width:10px;max-height:15px'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:2/1;max-width:10px;max-height:15px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 10.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;inset:0;aspect-ratio:2/1;max-width:10px;max-height:15px'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;inset:0;aspect-ratio:2/1;max-width:10px;max-height:15px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 10.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px;max-height:15px'><div style='width:6px;height:12px'></div></div></div>",
            [145.0, 0.0, 10.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px;max-height:15px'>XX XX</div></div>",
            [145.0, 0.0, 10.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px;max-height:15px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [145.0, 0.0, 10.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 6.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2'>XX XX</div></div>",
            [0.0, 0.0, 50.0, 100.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 4.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:1/2'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 300.0, 600.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:1/2'>XX XX</div></div>",
            [0.0, 0.0, 300.0, 600.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:1/2'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 300.0, 600.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:1/2'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 100.0, 200.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:1/2'>XX XX</div></div>",
            [0.0, 0.0, 100.0, 200.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:1/2'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 100.0, 200.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;inset:0;aspect-ratio:1/2'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 300.0, 600.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;inset:0;aspect-ratio:1/2'>XX XX</div></div>",
            [0.0, 0.0, 300.0, 600.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;inset:0;aspect-ratio:1/2'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 300.0, 600.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:1/2'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 300.0, 600.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:1/2'>XX XX</div></div>",
            [0.0, 0.0, 300.0, 600.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:1/2'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 300.0, 600.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2;max-height:30px;padding:2px'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 10.0, 16.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2;max-height:30px;padding:2px'>XX XX</div></div>",
            [0.0, 0.0, 19.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:1/2;max-height:30px;padding:2px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 8.0, 19.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:1/2;max-height:30px;padding:2px'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 19.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:1/2;max-height:30px;padding:2px'>XX XX</div></div>",
            [0.0, 0.0, 19.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:1/2;max-height:30px;padding:2px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 19.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:1/2;max-height:30px;padding:2px'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 19.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:1/2;max-height:30px;padding:2px'>XX XX</div></div>",
            [0.0, 0.0, 24.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:1/2;max-height:30px;padding:2px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 19.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;inset:0;aspect-ratio:1/2;max-height:30px;padding:2px'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 19.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;inset:0;aspect-ratio:1/2;max-height:30px;padding:2px'>XX XX</div></div>",
            [0.0, 0.0, 19.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;inset:0;aspect-ratio:1/2;max-height:30px;padding:2px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 19.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:1/2;max-height:30px;padding:2px'><div style='width:6px;height:12px'></div></div></div>",
            [140.5, 0.0, 19.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:1/2;max-height:30px;padding:2px'>XX XX</div></div>",
            [140.5, 0.0, 19.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:1/2;max-height:30px;padding:2px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [140.5, 0.0, 19.0, 34.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;display:flex;aspect-ratio:2/1;max-width:10px'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 6.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;display:flex;aspect-ratio:2/1;max-width:10px'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;display:flex;aspect-ratio:2/1;max-width:10px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 4.0, 13.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;display:grid;aspect-ratio:2/1;max-width:10px'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 6.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;display:grid;aspect-ratio:2/1;max-width:10px'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;display:grid;aspect-ratio:2/1;max-width:10px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 4.0, 13.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;display:grid;aspect-ratio:2/1;max-width:10px'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 10.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;display:grid;aspect-ratio:2/1;max-width:10px'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;display:grid;aspect-ratio:2/1;max-width:10px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 10.0, 13.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;display:grid;aspect-ratio:2/1;max-width:10px'><div style='width:6px;height:12px'></div></div></div>",
            [145.0, 0.0, 10.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;display:grid;aspect-ratio:2/1;max-width:10px'>XX XX</div></div>",
            [145.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;display:grid;aspect-ratio:2/1;max-width:10px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [145.0, 0.0, 10.0, 13.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;position:fixed'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 6.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;position:fixed'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;position:fixed'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 4.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;position:fixed'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 10.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;position:fixed'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;position:fixed'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 10.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px;position:fixed'><div style='width:6px;height:12px'></div></div></div>",
            [395.0, 0.0, 10.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px;position:fixed'>XX XX</div></div>",
            [395.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px;position:fixed'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [395.0, 0.0, 10.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;direction:rtl'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 6.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;direction:rtl'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;direction:rtl'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 4.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;direction:rtl'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 10.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;direction:rtl'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;direction:rtl'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 10.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px;direction:rtl'><div style='width:6px;height:12px'></div></div></div>",
            [145.0, 0.0, 10.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px;direction:rtl'>XX XX</div></div>",
            [145.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px;direction:rtl'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [145.0, 0.0, 10.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;overflow:hidden'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 6.0, 3.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;overflow:hidden'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;overflow:hidden'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 4.0, 2.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;overflow:hidden'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;overflow:hidden'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;overflow:hidden'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:2/1;max-width:10px;overflow:hidden'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:2/1;max-width:10px;overflow:hidden'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:2/1;max-width:10px;overflow:hidden'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;inset:0;aspect-ratio:2/1;max-width:10px;overflow:hidden'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;inset:0;aspect-ratio:2/1;max-width:10px;overflow:hidden'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;inset:0;aspect-ratio:2/1;max-width:10px;overflow:hidden'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px;overflow:hidden'><div style='width:6px;height:12px'></div></div></div>",
            [145.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px;overflow:hidden'>XX XX</div></div>",
            [145.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px;overflow:hidden'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [145.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;min-height:0'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 6.0, 3.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;min-height:0'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;min-height:0'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 4.0, 2.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;min-height:0'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;min-height:0'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;min-height:0'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:2/1;max-width:10px;min-height:0'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:2/1;max-width:10px;min-height:0'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;top:0;bottom:0;aspect-ratio:2/1;max-width:10px;min-height:0'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;inset:0;aspect-ratio:2/1;max-width:10px;min-height:0'><div style='width:6px;height:12px'></div></div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;inset:0;aspect-ratio:2/1;max-width:10px;min-height:0'>XX XX</div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;inset:0;aspect-ratio:2/1;max-width:10px;min-height:0'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px;min-height:0'><div style='width:6px;height:12px'></div></div></div>",
            [145.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px;min-height:0'>XX XX</div></div>",
            [145.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px;min-height:0'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [145.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;margin:0 3px'><div style='width:6px;height:12px'></div></div></div>",
            [3.0, 0.0, 6.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;margin:0 3px'>XX XX</div></div>",
            [3.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;aspect-ratio:2/1;max-width:10px;margin:0 3px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [3.0, 0.0, 4.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;margin:0 3px'><div style='width:6px;height:12px'></div></div></div>",
            [3.0, 0.0, 10.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;margin:0 3px'>XX XX</div></div>",
            [3.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:10px;margin:0 3px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [3.0, 0.0, 10.0, 15.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px;margin:0 3px'><div style='width:6px;height:12px'></div></div></div>",
            [3.0, 0.0, 10.0, 12.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px;margin:0 3px'>XX XX</div></div>",
            [3.0, 0.0, 10.0, 20.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;margin:0 auto;aspect-ratio:2/1;max-width:10px;margin:0 3px'><span style='display:inline-block;width:4px;height:13px'></span></div></div>",
            [3.0, 0.0, 10.0, 15.0],
        ),
    ]);
}

/// A box that `justify-self` places within the room its left and right
/// insets give it takes the width of its content, held by its `max-width`,
/// and the height its ratio gives that width.
#[test]
fn a_box_placed_within_its_insets_takes_the_ratio_of_its_content_width() {
    assert_rows(&[
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:20px;justify-self:start'><div style='width:10px;height:2px'></div></div></div>",
            [0.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:20px;justify-self:start'>X</div></div>",
            [0.0, 0.0, 10.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:20px;justify-self:end'><div style='width:10px;height:2px'></div></div></div>",
            [290.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:20px;justify-self:end'>X</div></div>",
            [290.0, 0.0, 10.0, 10.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:20px;justify-self:center'><div style='width:10px;height:2px'></div></div></div>",
            [145.0, 0.0, 10.0, 5.0],
        ),
        (
            "<div style='position:relative;width:300px;height:200px'><div id=t style='position:absolute;left:0;right:0;aspect-ratio:2/1;max-width:20px;justify-self:center'>X</div></div>",
            [145.0, 0.0, 10.0, 10.0],
        ),
    ]);
}
