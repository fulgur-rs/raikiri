//! A grid item whose aspect ratio relates its content box, under
//! `box-sizing: content-box` or `aspect-ratio: auto <ratio>`, keeps its
//! padding and border out of the ratio when its alignment stretches it or it
//! scrolls (CSS Sizing 4 §5.1), as it does when it is aligned to the start.
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

/// An item with a 25px border, a 10px padding or both, in a 100px column,
/// alone, with a sibling, between `1fr` columns, held by `min-width` or
/// `max-width`, or stretched in both axes.
#[test]
fn a_content_box_ratio_leaves_out_the_padding_and_border() {
    assert_cases(&[
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 75.0]),
                ("i", [0.0, 0.0, 100.0, 75.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid'><div style='height:10px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 75.0]),
                ("i", [0.0, 0.0, 100.0, 75.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid'><div style='width:100px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 100.0]),
                ("i", [0.0, 0.0, 150.0, 100.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid'><div style='height:10px;width:100px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 100.0]),
                ("i", [0.0, 0.0, 150.0, 100.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;max-width:30px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 65.0]),
                ("i", [0.0, 0.0, 80.0, 65.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;max-width:30px'><div style='height:10px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 65.0]),
                ("i", [0.0, 0.0, 80.0, 65.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;max-width:30px'><div style='width:100px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 65.0]),
                ("i", [0.0, 0.0, 80.0, 65.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;max-width:30px'><div style='height:10px;width:100px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 65.0]),
                ("i", [0.0, 0.0, 80.0, 65.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;min-width:30px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 75.0]),
                ("i", [0.0, 0.0, 100.0, 75.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;min-width:30px'><div style='height:10px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 75.0]),
                ("i", [0.0, 0.0, 100.0, 75.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;min-width:30px'><div style='width:100px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 75.0]),
                ("i", [0.0, 0.0, 100.0, 75.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;min-width:30px'><div style='height:10px;width:100px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 75.0]),
                ("i", [0.0, 0.0, 100.0, 75.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;padding:10px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 60.0]),
                ("i", [0.0, 0.0, 100.0, 60.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;padding:10px'><div style='height:10px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 60.0]),
                ("i", [0.0, 0.0, 100.0, 60.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;padding:10px'><div style='width:100px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 70.0]),
                ("i", [0.0, 0.0, 120.0, 70.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;padding:10px'><div style='height:10px;width:100px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 70.0]),
                ("i", [0.0, 0.0, 120.0, 70.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;border:25px solid'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;border:25px solid'><div style='height:10px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 60.0]),
                ("i", [0.0, 0.0, 120.0, 60.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;border:25px solid'><div style='width:100px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 75.0]),
                ("i", [0.0, 0.0, 150.0, 75.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;border:25px solid'><div style='height:10px;width:100px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 75.0]),
                ("i", [0.0, 0.0, 150.0, 75.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;justify-self:stretch;align-self:stretch'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 75.0]),
                ("i", [0.0, 0.0, 100.0, 75.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;justify-self:stretch;align-self:stretch'><div style='height:10px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 75.0]),
                ("i", [0.0, 0.0, 100.0, 75.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;justify-self:stretch;align-self:stretch'><div style='width:100px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 100.0]),
                ("i", [0.0, 0.0, 150.0, 100.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;justify-self:stretch;align-self:stretch'><div style='height:10px;width:100px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 100.0]),
                ("i", [0.0, 0.0, 150.0, 100.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;padding:3px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 58.0]),
                ("i", [0.0, 0.0, 100.0, 58.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;padding:3px'><div style='height:10px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 58.0]),
                ("i", [0.0, 0.0, 100.0, 58.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;padding:3px'><div style='width:100px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 66.0]),
                ("i", [0.0, 0.0, 116.0, 66.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;padding:3px'><div style='height:10px;width:100px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 66.0]),
                ("i", [0.0, 0.0, 116.0, 66.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid'><div style='height:10px'></div></div><div id=j style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 150.0]),
                ("i", [0.0, 0.0, 100.0, 75.0]),
                ("j", [0.0, 75.0, 100.0, 75.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px;grid-template-columns:1fr 1fr'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid'><div style='height:10px'></div></div><div id=j style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 60.0]),
                ("i", [0.0, 0.0, 70.0, 60.0]),
                ("j", [70.0, 0.0, 70.0, 60.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;max-width:30px'><div style='height:10px'></div></div><div id=j style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;max-width:30px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 130.0]),
                ("i", [0.0, 0.0, 80.0, 65.0]),
                ("j", [0.0, 65.0, 80.0, 65.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px;grid-template-columns:1fr 1fr'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;max-width:30px'><div style='height:10px'></div></div><div id=j style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;max-width:30px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 60.0]),
                ("i", [0.0, 0.0, 70.0, 60.0]),
                ("j", [70.0, 0.0, 70.0, 60.0]),
            ],
        ),
    ]);
}

/// Explicitly stretched items, items that scroll or are size-contained,
/// `aspect-ratio: auto <ratio>` on a border box, flex and grid containers,
/// and items that state a width, in auto, fixed and definite rows, with items
/// that already matched beside them.
#[test]
fn a_stretched_or_scrolling_item_keeps_its_edges_out_of_the_ratio() {
    assert_cases(&[
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;align-self:stretch'><div style='height:80px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 130.0]),
                ("i", [0.0, 0.0, 210.0, 130.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;align-self:stretch;justify-self:stretch'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 75.0]),
                ("i", [0.0, 0.0, 100.0, 75.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;align-self:stretch;justify-self:start'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 50.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;padding:5px;align-self:stretch'><div style='height:80px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 90.0]),
                ("i", [0.0, 0.0, 170.0, 90.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;padding:5px;align-self:stretch;justify-self:stretch'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;padding:5px;align-self:stretch;justify-self:start'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 10.0]),
                ("i", [0.0, 0.0, 10.0, 10.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:1/2;box-sizing:content-box;border:5px solid;max-height:40px;align-self:stretch'><div style='height:80px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 30.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:1/2;box-sizing:content-box;border:5px solid;max-height:40px;align-self:stretch;justify-self:stretch'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:1/2;box-sizing:content-box;border:5px solid;max-height:40px;align-self:stretch;justify-self:start'><div style='height:80px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 50.0]),
                ("i", [0.0, 0.0, 30.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;min-height:60px;align-self:stretch'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 70.0]),
                ("i", [0.0, 0.0, 130.0, 70.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;min-height:60px;align-self:stretch;justify-self:start'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 70.0]),
                ("i", [0.0, 0.0, 130.0, 70.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;overflow:hidden;justify-self:stretch'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;overflow:hidden;align-self:stretch;justify-self:stretch'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;overflow:hidden;align-self:stretch;justify-self:start'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 10.0]),
                ("i", [0.0, 0.0, 10.0, 10.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;overflow:hidden;align-self:stretch;margin-top:auto'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;width:60px;align-self:stretch'><div style='height:80px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 90.0]),
                ("i", [0.0, 0.0, 70.0, 90.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;width:60px;align-self:stretch;justify-self:stretch'><div style='height:80px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 90.0]),
                ("i", [0.0, 0.0, 70.0, 90.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;width:60px;align-self:stretch;justify-self:start'><div style='height:80px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 90.0]),
                ("i", [0.0, 0.0, 70.0, 90.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px;grid-template-columns:100px 50px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;min-height:60px;align-self:stretch;justify-self:stretch'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 100.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px;grid-template-columns:100px 50px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;overflow:hidden;align-self:stretch'></div><div id=s style='height:120px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 120.0]),
                ("i", [0.0, 0.0, 230.0, 120.0]),
                ("s", [100.0, 0.0, 50.0, 120.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px;height:200px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;justify-self:stretch'><div style='height:80px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 200.0]),
                ("i", [0.0, 0.0, 350.0, 200.0]),
            ],
        ),
        (
            r#"<div id=c style='display:inline-grid'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;min-height:60px;justify-self:stretch'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 130.0, 70.0]),
                ("i", [0.0, 0.0, 130.0, 70.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px;height:200px'><div id=i style='aspect-ratio:1/2;box-sizing:content-box;border:5px solid;max-height:40px;justify-self:stretch'>XX XX XX</div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 200.0]),
                ("i", [0.0, 0.0, 100.0, 50.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:auto 2/1;border:25px solid;align-self:stretch;justify-self:stretch'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 75.0]),
                ("i", [0.0, 0.0, 100.0, 75.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;display:flex;align-self:stretch;justify-self:stretch'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 75.0]),
                ("i", [0.0, 0.0, 100.0, 75.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;display:grid;align-self:stretch;justify-self:stretch'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 75.0]),
                ("i", [0.0, 0.0, 100.0, 75.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;contain:size;align-self:stretch;justify-self:stretch'><div style='height:10px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;contain:size;justify-self:stretch'><div style='height:10px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;contain:size;align-self:center'><div style='height:10px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;overflow:auto;align-self:stretch;justify-self:stretch'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 75.0]),
                ("i", [0.0, 0.0, 100.0, 75.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;overflow:auto;justify-self:stretch'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 75.0]),
                ("i", [0.0, 0.0, 100.0, 75.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;overflow:auto;align-self:center'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 75.0]),
                ("i", [0.0, 0.0, 100.0, 75.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;padding:5px;width:60px;align-self:stretch;justify-self:stretch'><div style='width:100px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 40.0]),
                ("i", [0.0, 0.0, 70.0, 40.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;padding:5px;width:60px;align-self:stretch;justify-self:stretch'><div style='height:10px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 40.0]),
                ("i", [0.0, 0.0, 70.0, 40.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;padding:5px;width:60px;justify-self:stretch'><div style='width:100px'></div></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 40.0]),
                ("i", [0.0, 0.0, 70.0, 40.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;width:max-content;align-self:stretch;justify-self:stretch'><div style='width:100px;height:10px'></div></div><div id=j style='height:5px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 105.0]),
                ("i", [0.0, 0.0, 150.0, 100.0]),
                ("j", [0.0, 100.0, 150.0, 5.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;width:max-content;align-self:stretch'><div style='width:100px;height:10px'></div></div><div id=j style='height:5px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 105.0]),
                ("i", [0.0, 0.0, 150.0, 100.0]),
                ("j", [0.0, 100.0, 150.0, 5.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px;height:160px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;width:40px;align-self:stretch;justify-self:stretch'></div><div id=j style='height:5px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 160.0]),
                ("i", [0.0, 0.0, 90.0, 112.5]),
                ("j", [0.0, 112.5, 100.0, 5.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px;height:160px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;width:40px;align-self:stretch'></div><div id=j style='height:5px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 160.0]),
                ("i", [0.0, 0.0, 90.0, 112.5]),
                ("j", [0.0, 112.5, 100.0, 5.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:300px;grid-template-columns:1fr 1fr'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:min-content;justify-self:stretch'><div style='width:100px;height:10px'></div></div><div id=j style='height:5px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 300.0, 100.0]),
                ("i", [0.0, 0.0, 150.0, 100.0]),
                ("j", [150.0, 0.0, 150.0, 5.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:300px;grid-template-columns:1fr 1fr'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;height:40px;'><div style='width:100px;height:10px'></div></div><div id=j style='height:5px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 300.0, 90.0]),
                ("i", [0.0, 0.0, 150.0, 90.0]),
                ("j", [150.0, 0.0, 150.0, 5.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:25px solid;width:50%;justify-self:stretch'></div><div id=j style='height:5px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 80.0]),
                ("i", [0.0, 0.0, 100.0, 75.0]),
                ("j", [0.0, 75.0, 100.0, 5.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;contain:size;overflow-y:auto;min-height:60px;align-self:stretch;justify-self:stretch'></div><div id=j style='height:5px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 75.0]),
                ("i", [0.0, 0.0, 100.0, 70.0]),
                ("j", [0.0, 70.0, 100.0, 5.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;contain:inline-size;'></div><div id=j style='height:5px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 60.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
                ("j", [0.0, 55.0, 100.0, 5.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;contain:inline-size;align-self:stretch;justify-self:stretch'></div><div id=j style='height:5px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 60.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
                ("j", [0.0, 55.0, 100.0, 5.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;contain:inline-size;align-self:end'></div><div id=j style='height:5px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 60.0]),
                ("i", [0.0, 0.0, 100.0, 55.0]),
                ("j", [0.0, 55.0, 100.0, 5.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;contain:size;min-height:70px;'></div><div id=j style='height:5px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 100.0, 85.0]),
                ("i", [0.0, 0.0, 150.0, 80.0]),
                ("j", [0.0, 80.0, 150.0, 5.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:200px;grid-template-columns:1fr 1fr'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;border:5px solid;contain:size;min-height:70px;align-self:end'><div style='height:80px'></div></div><div id=j style='height:5px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 200.0, 80.0]),
                ("i", [0.0, 0.0, 150.0, 80.0]),
                ("j", [150.0, 0.0, 50.0, 5.0]),
            ],
        ),
        (
            r#"<div id=c style='display:grid;width:200px;grid-template-columns:1fr 1fr'><div id=i style='aspect-ratio:2/1;box-sizing:content-box;padding:10px;contain:size;contain-intrinsic-size:100px 100px;align-self:end'><div style='height:80px'></div></div><div id=j style='height:5px'></div></div>"#,
            &[
                ("c", [0.0, 0.0, 200.0, 120.0]),
                ("i", [0.0, 0.0, 120.0, 120.0]),
                ("j", [120.0, 0.0, 80.0, 5.0]),
            ],
        ),
    ]);
}
