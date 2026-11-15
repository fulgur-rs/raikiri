//! A grid item with a preferred aspect ratio whose alignment stretches it
//! along its block axis, or whose size is contained, takes the sizes Chromium
//! gives it (CSS Grid 1 §6.2, CSS Sizing 4 §5.1).
//!
//! The expected border boxes `[x, y, width, height]` are the ones Chromium
//! computes for the same markup in an 800px by 600px viewport with no body
//! margin, `font: 10px/10px Ahem` and `div { box-sizing: border-box }`.

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
         body {{ margin: 0; font: 10px/10px Ahem }} \
         div {{ box-sizing: border-box }}</style>{body}"
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

/// An item with `align-self: stretch` and a `normal` `justify-self`
/// contributes the height its ratio gives for the width of its column, then
/// takes the height of its row and the width the ratio gives for it: empty,
/// with text, padding or a border, as a flex or grid container, beside a
/// taller sibling, under `max-width` or `max-height`, or scrolling.
#[test]
fn an_item_stretched_in_the_block_axis_only_takes_its_width_from_the_row_height() {
    assert_cases(&[
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;box-sizing:content-box;border:5px solid'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;border:5px solid'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch'>XX XX XX</div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;box-sizing:content-box;border:5px solid'>XX XX XX</div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;border:5px solid'>XX XX XX</div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;align-items:stretch"><div id=i style='aspect-ratio:2/1'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch'></div><div id=s style='height:30px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
                ("s", [0.0, 50.0, 100.0, 30.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:50px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch'></div><div id=s style='height:80px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 160.0, 80.0]),
                ("s", [50.0, 0.0, 50.0, 80.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;padding:5px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;padding:5px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;padding:5px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;padding:5px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;padding:5px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 160.0, 80.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;padding:5px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 300.0, 150.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;box-sizing:content-box;padding:5px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;box-sizing:content-box;padding:5px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 230.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;box-sizing:content-box;padding:5px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 230.0, 120.0]),
                ("s", [230.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;box-sizing:content-box;padding:5px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 230.0, 120.0]),
                ("s", [230.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;box-sizing:content-box;padding:5px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 150.0, 80.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;box-sizing:content-box;padding:5px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 290.0, 150.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;display:flex;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;display:flex;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;display:flex;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;display:flex;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;display:flex;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 160.0, 80.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;display:flex;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 300.0, 150.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;display:grid;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;display:grid;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;display:grid;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;display:grid;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;display:grid;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 160.0, 80.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;display:grid;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 300.0, 150.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;min-height:70px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 70.0]),
                ("i", [0.0, 0.0, 140.0, 70.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;min-height:70px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;min-height:70px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 160.0, 80.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;min-height:70px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 300.0, 150.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;max-width:60px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 30.0]),
                ("i", [0.0, 0.0, 60.0, 30.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;max-width:60px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 60.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;max-width:60px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 60.0, 120.0]),
                ("s", [80.0, 0.0, 20.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;max-width:60px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 60.0, 120.0]),
                ("s", [60.0, 0.0, 40.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;max-width:60px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 60.0, 80.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;max-width:60px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 60.0, 150.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;max-height:40px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 40.0]),
                ("i", [0.0, 0.0, 80.0, 40.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;max-height:40px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 80.0, 40.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;max-height:40px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 80.0, 40.0]),
                ("s", [90.0, 0.0, 10.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;max-height:40px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 80.0, 40.0]),
                ("s", [80.0, 0.0, 20.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;max-height:40px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 80.0, 40.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;max-height:40px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 80.0, 40.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;margin-top:auto;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;margin-top:auto;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 70.0, 100.0, 50.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;margin-top:auto;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 95.0, 50.0, 25.0]),
                ("s", [50.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;margin-top:auto;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 95.0, 50.0, 25.0]),
                ("s", [50.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;margin-top:auto;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 30.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;margin-top:auto;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 100.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;margin-left:auto;'></div></div>"#,
            &[("c", [0.0, 0.0, 100.0, 0.0]), ("i", [100.0, 0.0, 0.0, 0.0])],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;margin-left:auto;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;margin-left:auto;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;margin-left:auto;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;margin-left:auto;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 160.0, 80.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;margin-left:auto;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 300.0, 150.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;'>XX</div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;'>XX</div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;'>XX</div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;'>XX</div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;'>XX</div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 160.0, 80.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;'>XX</div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 300.0, 150.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;overflow:hidden;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;overflow:hidden;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;overflow:hidden;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [100.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;overflow:hidden;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [50.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;overflow:hidden;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 160.0, 80.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;overflow:hidden;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 300.0, 150.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;contain:size;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;contain:size;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;contain:size;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;contain:size;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;contain:size;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 160.0, 80.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;contain:size;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 300.0, 150.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;height:fit-content;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;height:fit-content;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;height:fit-content;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 50.0, 25.0]),
                ("s", [50.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;height:fit-content;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 50.0, 25.0]),
                ("s", [50.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;height:fit-content;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;height:fit-content;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
    ]);
}

/// An item stretched in both axes, or stretched in one and sized in the
/// other, fills its grid area whatever its ratio gives, once an `auto` row
/// is sized: its ratio still gives the height it contributes to that row,
/// and the width it contributes to an `auto` column beside a fixed row,
/// explicit or implicit.
#[test]
fn an_item_stretched_in_both_axes_fills_its_area_past_its_ratio() {
    assert_cases(&[
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 100.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px;height:200px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 200.0]),
                ("i", [0.0, 0.0, 100.0, 200.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:130px 50px;height:200px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 200.0]),
                ("i", [0.0, 0.0, 130.0, 200.0]),
                ("s", [130.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch'></div><div id=s style='height:20px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
                ("s", [100.0, 0.0, 50.0, 20.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch'>XX XX XX</div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 100.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;width:60px'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 60.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;justify-self:stretch;height:30px'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 100.0, 30.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;padding:5px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;padding:5px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 100.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;padding:5px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;padding:5px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;padding:5px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 160.0, 80.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;padding:5px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 300.0, 150.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;box-sizing:content-box;padding:5px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;box-sizing:content-box;padding:5px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 100.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;box-sizing:content-box;padding:5px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 230.0, 120.0]),
                ("s", [230.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;box-sizing:content-box;padding:5px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 230.0, 120.0]),
                ("s", [230.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;box-sizing:content-box;padding:5px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 150.0, 80.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;box-sizing:content-box;padding:5px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 290.0, 150.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;display:flex;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;display:flex;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 100.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;display:flex;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;display:flex;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;display:flex;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 160.0, 80.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;display:flex;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 300.0, 150.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;display:grid;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;display:grid;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 100.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;display:grid;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;display:grid;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;display:grid;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 160.0, 80.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;display:grid;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 300.0, 150.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;min-height:70px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 70.0]),
                ("i", [0.0, 0.0, 140.0, 70.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;min-height:70px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 100.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;min-height:70px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 160.0, 80.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;min-height:70px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 300.0, 150.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;max-width:60px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 30.0]),
                ("i", [0.0, 0.0, 60.0, 30.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;max-width:60px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 60.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;max-width:60px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 60.0, 120.0]),
                ("s", [80.0, 0.0, 20.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;max-width:60px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 60.0, 120.0]),
                ("s", [60.0, 0.0, 40.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;max-width:60px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 60.0, 80.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;max-width:60px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 60.0, 150.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;max-height:40px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 40.0]),
                ("i", [0.0, 0.0, 100.0, 40.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;max-height:40px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 100.0, 40.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;max-height:40px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 90.0, 40.0]),
                ("s", [90.0, 0.0, 10.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;max-height:40px;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 80.0, 40.0]),
                ("s", [80.0, 0.0, 20.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;max-height:40px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 100.0, 40.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;max-height:40px;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 100.0, 40.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;margin-top:auto;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;margin-top:auto;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 70.0, 100.0, 50.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;margin-top:auto;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 95.0, 50.0, 25.0]),
                ("s", [50.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;margin-top:auto;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 95.0, 50.0, 25.0]),
                ("s", [50.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;margin-top:auto;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 30.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;margin-top:auto;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 100.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;margin-left:auto;'></div></div>"#,
            &[("c", [0.0, 0.0, 100.0, 0.0]), ("i", [100.0, 0.0, 0.0, 0.0])],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;margin-left:auto;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;margin-left:auto;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;margin-left:auto;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;margin-left:auto;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 160.0, 80.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;margin-left:auto;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 300.0, 150.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;'>XX</div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;'>XX</div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 100.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;'>XX</div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;'>XX</div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;'>XX</div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 160.0, 80.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;'>XX</div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 300.0, 150.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;overflow:hidden;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;overflow:hidden;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 100.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;overflow:hidden;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 100.0, 120.0]),
                ("s", [100.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;overflow:hidden;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 50.0, 120.0]),
                ("s", [50.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;overflow:hidden;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 100.0, 80.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;overflow:hidden;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 100.0, 150.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;contain:size;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;contain:size;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 100.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;contain:size;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;contain:size;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [240.0, 0.0, 0.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;contain:size;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 160.0, 80.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;contain:size;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 300.0, 150.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;height:fit-content;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;height:fit-content;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto auto"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;height:fit-content;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 50.0, 25.0]),
                ("s", [50.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:1fr 1fr"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;height:fit-content;'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 50.0, 25.0]),
                ("s", [50.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:80px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;height:fit-content;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-rows:auto;height:150px"><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch;height:fit-content;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px;align-items:stretch"><div id=i style='aspect-ratio:2/1'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px;align-items:stretch;justify-items:stretch"><div id=i style='aspect-ratio:2/1'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 100.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:100px 50px"><img id=i style='aspect-ratio:2/1;align-self:stretch;width:auto'><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 240.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style='display:inline-grid;grid-template-columns:100px;grid-template-rows:60px'><div id=i style='grid-column:2;aspect-ratio:2/1;align-self:stretch;justify-self:stretch'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 220.0, 60.0]),
                ("i", [100.0, 0.0, 120.0, 60.0]),
            ],
        ),
        (
            r#"<div id=c style='display:inline-grid;grid-template-columns:100px;grid-template-rows:60px;grid-auto-flow:column'><div id=s></div><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 220.0, 60.0]),
                ("i", [100.0, 0.0, 120.0, 60.0]),
                ("s", [0.0, 0.0, 100.0, 60.0]),
            ],
        ),
        (
            r#"<div id=c style='display:inline-grid;grid-template-columns:100px;grid-template-rows:60px;grid-auto-columns:30px'><div id=i style='grid-column:2;aspect-ratio:2/1;align-self:stretch;justify-self:stretch'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 130.0, 60.0]),
                ("i", [100.0, 0.0, 30.0, 60.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px;grid-template-columns:repeat(2,50px);grid-template-rows:60px'><div id=i style='aspect-ratio:2/1;align-self:stretch;justify-self:stretch'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 60.0]),
                ("i", [0.0, 0.0, 50.0, 60.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px;grid-template-columns:100px 50px'><div id=i style='width:50%;aspect-ratio:2/1;align-self:stretch'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 50.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
    ]);
}

/// An empty size-contained item whose ratio relates its content box keeps
/// its padding and border out of the ratio, as one with content does, and
/// one whose ratio relates its border box keeps the ratio there.
#[test]
fn an_empty_size_contained_item_keeps_its_edges_out_of_a_content_box_ratio() {
    assert_cases(&[
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;contain:size'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='align-self:stretch;justify-self:stretch;aspect-ratio:2/1;box-sizing:content-box;border:5px solid;contain:size'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='justify-self:stretch;aspect-ratio:2/1;box-sizing:content-box;border:5px solid;contain:size'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;box-sizing:content-box;padding:5px;contain:size'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;border:5px solid;contain:size'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:auto 2/1;border:5px solid;contain:size'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;contain:size'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;contain:size'>X</div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;contain:inline-size'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;contain:size;justify-self:start'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 10.0]),
                ("i", [0.0, 0.0, 10.0, 10.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;contain:size;contain-intrinsic-size:40px 10px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;contain:size;display:flex'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;contain:size;overflow:hidden'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px"><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;contain:size;width:50px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 35.0]),
                ("i", [0.0, 0.0, 60.0, 35.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:auto"><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;contain:size'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style="display:grid;width:100px;grid-template-columns:60px"><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;contain:size'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 35.0]),
                ("i", [0.0, 0.0, 60.0, 35.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;border:5px solid;contain:size'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
    ]);
}
