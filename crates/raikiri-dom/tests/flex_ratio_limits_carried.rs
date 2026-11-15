//! The `min-*` and `max-*` sizes of a flex item with an `aspect-ratio`
//! reach the other axis through the ratio only where that axis is `auto`
//! (CSS Sizing 4 §5.1), and a stretched item in a container whose cross size
//! is indefinite takes its flex base size from its content, not from the
//! maximum cross size it may be stretched to (CSS Flexbox 1 §9.2, §9.8).
//!
//! The expected border boxes `[x, y, width, height]` are the ones Chromium
//! computes for the same markup in an 800px by 600px viewport with no body
//! margin.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};
use std::collections::HashMap;

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The border box `[x, y, width, height]` of every element with an `id` on
/// the first page of an 800x600 document that holds `body`.
fn border_boxes(body: &str) -> HashMap<String, [f32; 4]> {
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
    page.fragments()
        .filter_map(|fragment| {
            let id = dom.attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((id.to_owned(), [rect.x, rect.y, rect.width, rect.height]))
        })
        .collect()
}

/// The border boxes expected of the elements with these ids.
type Expected<'a> = &'a [(&'a str, [f32; 4])];

/// A container style, an item style, and the border boxes of the container,
/// the item and its sibling.
type Row = (&'static str, &'static str, [f32; 4], [f32; 4], [f32; 4]);

fn assert_cases(cases: &[(&str, Expected<'_>)]) {
    let mut failures = Vec::new();
    for (body, expected) in cases {
        let found = border_boxes(body);
        for (id, want) in *expected {
            let got = found.get(*id).copied();
            let matches = got.is_some_and(|got| {
                got.iter()
                    .zip(want.iter())
                    .all(|(got, want)| (got - want).abs() < 0.02)
            });
            if !matches {
                failures.push(format!("{body}\n  #{id}: expected {want:?}, got {got:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A limit carried across the ratio does not move a size the item states, and
/// a minimum carried to an `auto` main size raises the flex base size.
#[test]
fn limits_reach_the_other_axis_only_where_it_is_auto() {
    assert_cases(&[
        (
            r#"<div style="display:flex;width:100px"><div id=t style="width:20px;aspect-ratio:1/1;flex-grow:1;min-height:150px"></div></div>"#,
            &[("t", [0.0, 0.0, 100.0, 150.0])],
        ),
        (
            r#"<div style="display:flex;width:300px"><div id=t style="flex-basis:content;height:100px;aspect-ratio:2/1;width:20px;min-width:0;max-width:50px"></div></div>"#,
            &[("t", [0.0, 0.0, 50.0, 100.0])],
        ),
        (
            r#"<div style="display:flex"><div id=t style="min-height:50px;aspect-ratio:2/1;min-width:0"></div></div>"#,
            &[("t", [0.0, 0.0, 100.0, 50.0])],
        ),
        (
            r#"<div style="display:flex"><div id=t style="max-height:50px;aspect-ratio:2/1"><div style="width:200px"></div></div></div>"#,
            &[("t", [0.0, 0.0, 100.0, 50.0])],
        ),
    ]);
}

/// A stretched item with a ratio in a container whose cross size is
/// indefinite is not measured at its maximum cross size for its flex base
/// size, whichever way the line runs, and keeps the ratio where the cross size
/// is definite or the item is not stretched. Each row is the container's
/// style, the item's style, and the border boxes of the container, the item
/// and a 20px by 5px sibling after it.
#[test]
fn a_stretched_ratio_item_takes_its_flex_base_size_from_its_content() {
    let rows: &[Row] = &[
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;max-height:10px",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 0.0, 5.0],
            [0.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;max-height:10px;min-width:0",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 0.0, 5.0],
            [0.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;max-width:10px",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 0.0, 5.0],
            [0.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:1/2;max-height:30px;padding:2px",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 4.0, 5.0],
            [4.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;max-height:10px;flex:1",
            [0.0, 0.0, 300.0, 10.0],
            [0.0, 0.0, 280.0, 10.0],
            [280.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;max-height:10px;margin-top:auto",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 5.0, 0.0, 0.0],
            [0.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;height:50%;max-height:10px",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;height:stretch;max-height:10px",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 0.0, 5.0],
            [0.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;height:min-content;max-height:10px",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:2/1;max-height:10px",
            [0.0, 3.0, 20.0, 5.0],
            [0.0, 3.0, 0.0, 5.0],
            [0.0, 3.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:2/1;max-height:10px;min-width:0",
            [0.0, 3.0, 20.0, 5.0],
            [0.0, 3.0, 0.0, 5.0],
            [0.0, 3.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:2/1;max-width:10px",
            [0.0, 3.0, 20.0, 5.0],
            [0.0, 3.0, 0.0, 5.0],
            [0.0, 3.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:1/2;max-height:30px;padding:2px",
            [0.0, 3.0, 24.0, 5.0],
            [0.0, 3.0, 4.0, 5.0],
            [4.0, 3.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:2/1;max-height:10px;flex:1",
            [0.0, 3.0, 20.0, 5.0],
            [0.0, 3.0, 0.0, 5.0],
            [0.0, 3.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:2/1;max-height:10px;margin-top:auto",
            [0.0, 3.0, 20.0, 5.0],
            [0.0, 8.0, 0.0, 0.0],
            [0.0, 3.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:2/1;height:50%;max-height:10px",
            [0.0, 8.0, 20.0, 5.0],
            [0.0, 8.0, 0.0, 0.0],
            [0.0, 8.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:2/1;height:stretch;max-height:10px",
            [0.0, 3.0, 20.0, 5.0],
            [0.0, 3.0, 0.0, 5.0],
            [0.0, 3.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:2/1;height:min-content;max-height:10px",
            [0.0, 8.0, 20.0, 5.0],
            [0.0, 8.0, 0.0, 0.0],
            [0.0, 8.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:2/1;max-height:10px",
            [0.0, 0.0, 300.0, 5.0],
            [300.0, 0.0, 0.0, 5.0],
            [280.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:2/1;max-height:10px;min-width:0",
            [0.0, 0.0, 300.0, 5.0],
            [300.0, 0.0, 0.0, 5.0],
            [280.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:2/1;max-width:10px",
            [0.0, 0.0, 300.0, 5.0],
            [300.0, 0.0, 0.0, 5.0],
            [280.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:1/2;max-height:30px;padding:2px",
            [0.0, 0.0, 300.0, 5.0],
            [296.0, 0.0, 4.0, 5.0],
            [276.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:2/1;max-height:10px;flex:1",
            [0.0, 0.0, 300.0, 10.0],
            [20.0, 0.0, 280.0, 10.0],
            [0.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:2/1;max-height:10px;margin-top:auto",
            [0.0, 0.0, 300.0, 5.0],
            [300.0, 5.0, 0.0, 0.0],
            [280.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:2/1;height:50%;max-height:10px",
            [0.0, 0.0, 300.0, 5.0],
            [300.0, 0.0, 0.0, 0.0],
            [280.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:2/1;height:stretch;max-height:10px",
            [0.0, 0.0, 300.0, 5.0],
            [300.0, 0.0, 0.0, 5.0],
            [280.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:2/1;height:min-content;max-height:10px",
            [0.0, 0.0, 300.0, 5.0],
            [300.0, 0.0, 0.0, 0.0],
            [280.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;height:50px",
            "aspect-ratio:2/1;max-height:10px",
            [0.0, 0.0, 300.0, 50.0],
            [0.0, 0.0, 20.0, 10.0],
            [20.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;height:50px",
            "aspect-ratio:2/1;max-height:10px;min-width:0",
            [0.0, 0.0, 300.0, 50.0],
            [0.0, 0.0, 20.0, 10.0],
            [20.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;height:50px",
            "aspect-ratio:2/1;max-width:10px",
            [0.0, 0.0, 300.0, 50.0],
            [0.0, 0.0, 10.0, 50.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;height:50px",
            "aspect-ratio:1/2;max-height:30px;padding:2px",
            [0.0, 0.0, 300.0, 50.0],
            [0.0, 0.0, 19.0, 34.0],
            [19.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;height:50px",
            "aspect-ratio:2/1;max-height:10px;flex:1",
            [0.0, 0.0, 300.0, 50.0],
            [0.0, 0.0, 280.0, 10.0],
            [280.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;height:50px",
            "aspect-ratio:2/1;max-height:10px;margin-top:auto",
            [0.0, 0.0, 300.0, 50.0],
            [0.0, 50.0, 0.0, 0.0],
            [0.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;height:50px",
            "aspect-ratio:2/1;height:50%;max-height:10px",
            [0.0, 0.0, 300.0, 50.0],
            [0.0, 0.0, 20.0, 10.0],
            [20.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;height:50px",
            "aspect-ratio:2/1;height:stretch;max-height:10px",
            [0.0, 0.0, 300.0, 50.0],
            [0.0, 0.0, 20.0, 10.0],
            [20.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;height:50px",
            "aspect-ratio:2/1;height:min-content;max-height:10px",
            [0.0, 0.0, 300.0, 50.0],
            [0.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:2/1;max-height:10px",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 0.0, 5.0],
            [0.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:2/1;max-height:10px;min-width:0",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 0.0, 5.0],
            [0.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:2/1;max-width:10px",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 0.0, 5.0],
            [0.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:1/2;max-height:30px;padding:2px",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 4.0, 5.0],
            [4.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:2/1;max-height:10px;flex:1",
            [0.0, 0.0, 300.0, 10.0],
            [0.0, 0.0, 280.0, 10.0],
            [280.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:2/1;max-height:10px;margin-top:auto",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 5.0, 0.0, 0.0],
            [0.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:2/1;height:50%;max-height:10px",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:2/1;height:stretch;max-height:10px",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 0.0, 5.0],
            [0.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:2/1;height:min-content;max-height:10px",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex;flex-direction:column",
            "aspect-ratio:2/1;max-height:10px",
            [0.0, 0.0, 20.0, 15.0],
            [0.0, 0.0, 20.0, 10.0],
            [0.0, 10.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex;flex-direction:column",
            "aspect-ratio:2/1;max-height:10px;min-width:0",
            [0.0, 0.0, 20.0, 15.0],
            [0.0, 0.0, 20.0, 10.0],
            [0.0, 10.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex;flex-direction:column",
            "aspect-ratio:2/1;max-width:10px",
            [0.0, 3.0, 20.0, 10.0],
            [0.0, 3.0, 10.0, 5.0],
            [0.0, 8.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex;flex-direction:column",
            "aspect-ratio:1/2;max-height:30px;padding:2px",
            [0.0, 0.0, 20.0, 39.0],
            [0.0, 0.0, 20.0, 34.0],
            [0.0, 34.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex;flex-direction:column",
            "aspect-ratio:2/1;max-height:10px;flex:1",
            [0.0, 0.0, 20.0, 15.0],
            [0.0, 0.0, 20.0, 10.0],
            [0.0, 10.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex;flex-direction:column",
            "aspect-ratio:2/1;max-height:10px;margin-top:auto",
            [0.0, 0.0, 20.0, 15.0],
            [0.0, 0.0, 20.0, 10.0],
            [0.0, 10.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex;flex-direction:column",
            "aspect-ratio:2/1;height:50%;max-height:10px",
            [0.0, 0.0, 20.0, 15.0],
            [0.0, 0.0, 20.0, 10.0],
            [0.0, 10.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex;flex-direction:column",
            "aspect-ratio:2/1;height:stretch;max-height:10px",
            [0.0, 0.0, 20.0, 15.0],
            [0.0, 0.0, 20.0, 10.0],
            [0.0, 10.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex;flex-direction:column",
            "aspect-ratio:2/1;height:min-content;max-height:10px",
            [0.0, 0.0, 20.0, 15.0],
            [0.0, 0.0, 20.0, 10.0],
            [0.0, 10.0, 20.0, 5.0],
        ),
        (
            "display:flex;flex-direction:column;width:300px",
            "aspect-ratio:2/1;max-height:10px",
            [0.0, 0.0, 300.0, 15.0],
            [0.0, 0.0, 300.0, 10.0],
            [0.0, 10.0, 20.0, 5.0],
        ),
        (
            "display:flex;flex-direction:column;width:300px",
            "aspect-ratio:2/1;max-height:10px;min-width:0",
            [0.0, 0.0, 300.0, 15.0],
            [0.0, 0.0, 300.0, 10.0],
            [0.0, 10.0, 20.0, 5.0],
        ),
        (
            "display:flex;flex-direction:column;width:300px",
            "aspect-ratio:2/1;max-width:10px",
            [0.0, 0.0, 300.0, 10.0],
            [0.0, 0.0, 10.0, 5.0],
            [0.0, 5.0, 20.0, 5.0],
        ),
        (
            "display:flex;flex-direction:column;width:300px",
            "aspect-ratio:1/2;max-height:30px;padding:2px",
            [0.0, 0.0, 300.0, 39.0],
            [0.0, 0.0, 300.0, 34.0],
            [0.0, 34.0, 20.0, 5.0],
        ),
        (
            "display:flex;flex-direction:column;width:300px",
            "aspect-ratio:2/1;max-height:10px;flex:1",
            [0.0, 0.0, 300.0, 15.0],
            [0.0, 0.0, 300.0, 10.0],
            [0.0, 10.0, 20.0, 5.0],
        ),
        (
            "display:flex;flex-direction:column;width:300px",
            "aspect-ratio:2/1;max-height:10px;margin-top:auto",
            [0.0, 0.0, 300.0, 15.0],
            [0.0, 0.0, 300.0, 10.0],
            [0.0, 10.0, 20.0, 5.0],
        ),
        (
            "display:flex;flex-direction:column;width:300px",
            "aspect-ratio:2/1;height:50%;max-height:10px",
            [0.0, 0.0, 300.0, 15.0],
            [0.0, 0.0, 300.0, 10.0],
            [0.0, 10.0, 20.0, 5.0],
        ),
        (
            "display:flex;flex-direction:column;width:300px",
            "aspect-ratio:2/1;height:stretch;max-height:10px",
            [0.0, 0.0, 300.0, 15.0],
            [0.0, 0.0, 300.0, 10.0],
            [0.0, 10.0, 20.0, 5.0],
        ),
        (
            "display:flex;flex-direction:column;width:300px",
            "aspect-ratio:2/1;height:min-content;max-height:10px",
            [0.0, 0.0, 300.0, 15.0],
            [0.0, 0.0, 300.0, 10.0],
            [0.0, 10.0, 20.0, 5.0],
        ),
    ];
    let bodies: Vec<String> = rows
        .iter()
        .map(|(container, item, ..)| {
            format!(
                "<div id=c style='{container}'><div id=t style='{item}'></div>\
                 <div id=s style='width:20px;height:5px'></div></div>"
            )
        })
        .collect();
    let expected: Vec<[(&str, [f32; 4]); 3]> = rows
        .iter()
        .map(|&(_, _, c, t, s)| [("c", c), ("t", t), ("s", s)])
        .collect();
    let cases: Vec<(&str, Expected<'_>)> = bodies
        .iter()
        .zip(&expected)
        .map(|(body, expected)| (body.as_str(), &expected[..]))
        .collect();
    assert_cases(&cases);
}
