//! A flex or grid container, an absolutely positioned box or an inline-block
//! whose preferred aspect ratio relates its border boxes, with `auto` sizes,
//! is no shorter than its padding and border, so its ratio gives it at least
//! the width of that height: an empty `display: inline-flex; aspect-ratio:
//! 2 / 1` box with 2px of border is 8px by 4px. A `max-width` caps the floor;
//! a smaller `min-width` and narrower room do not lower it.
//!
//! The expected border boxes `[x, y, width, height]` of the elements with an
//! `id` are the ones Chromium computes for the same markup in an 800px by
//! 600px viewport with no body margin.

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

/// Markup and the expected border boxes of its elements by `id`.
type Row = (&'static str, &'static [(&'static str, [f32; 4])]);

fn assert_rows(rows: &[Row]) {
    let mut failures = Vec::new();
    for (body, expected) in rows {
        let boxes = border_boxes(body);
        for (id, want) in *expected {
            let got = boxes.get(*id).copied();
            let matches = got.is_some_and(|got| {
                got.iter()
                    .zip(want)
                    .all(|(got, want)| (got - want).abs() <= 0.5)
            });
            if !matches {
                failures.push(format!("{body}\n  #{id}: expected {want:?}, got {got:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// An empty inline-level or floated flex or grid container, with or without a
/// `min-width` or `max-width` of its own.
#[test]
fn an_empty_container_is_as_wide_as_its_ratio_gives_its_edges() {
    assert_rows(&[
        (
            "<div style='width:300px'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 4.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;max-width:5px'></div></div>",
            &[("t", [0.0, 4.0, 5.0, 4.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;padding:3px'></div></div>",
            &[("t", [0.0, 2.0, 12.0, 6.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div></div>",
            &[("t", [0.0, 0.0, 16.0, 8.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 4.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;max-width:5px'></div></div>",
            &[("t", [0.0, 4.0, 5.0, 4.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;padding:3px'></div></div>",
            &[("t", [0.0, 2.0, 12.0, 6.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div></div>",
            &[("t", [0.0, 0.0, 16.0, 8.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 300.0, 150.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;padding:3px'></div></div>",
            &[("t", [0.0, 0.0, 300.0, 150.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:flex;aspect-ratio:2/1;box-sizing:border-box;padding:3px'></div></div>",
            &[("t", [0.0, 0.0, 12.0, 6.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div></div>",
            &[("t", [0.0, 0.0, 300.0, 150.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div></div>",
            &[("t", [0.0, 0.0, 16.0, 8.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 300.0, 150.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;padding:3px'></div></div>",
            &[("t", [0.0, 0.0, 300.0, 150.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:grid;aspect-ratio:2/1;box-sizing:border-box;padding:3px'></div></div>",
            &[("t", [0.0, 0.0, 12.0, 6.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div></div>",
            &[("t", [0.0, 0.0, 300.0, 150.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div></div>",
            &[("t", [0.0, 0.0, 16.0, 8.0])],
        ),
        (
            "<div><div id=t style='float:left;display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:2px'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:flex;aspect-ratio:2/1;padding:1%'></div></div>",
            &[("t", [0.0, 0.0, 6.0, 6.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;max-width:1%'></div></div>",
            &[("t", [0.0, 0.0, 4.0, 4.0])],
        ),
        (
            "<div><div id=t style='float:left;display:flex;aspect-ratio:auto 2/1;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 4.0, 4.0])],
        ),
        (
            "<div><div id=t style='float:left;display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:2px'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:grid;aspect-ratio:2/1;padding:1%'></div></div>",
            &[("t", [0.0, 0.0, 6.0, 6.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;max-width:1%'></div></div>",
            &[("t", [0.0, 0.0, 4.0, 4.0])],
        ),
        (
            "<div><div id=t style='float:left;display:grid;aspect-ratio:auto 2/1;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 4.0, 4.0])],
        ),
        (
            "<div><div id=t style='float:left;display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:2px'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-flex;aspect-ratio:2/1;padding:1%'></div></div>",
            &[("t", [0.0, 0.0, 6.0, 6.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;max-width:1%'></div></div>",
            &[("t", [0.0, 0.0, 4.0, 4.0])],
        ),
        (
            "<div><div id=t style='float:left;display:inline-flex;aspect-ratio:auto 2/1;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 4.0, 4.0])],
        ),
        (
            "<div><div id=t style='float:left;display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:2px'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-grid;aspect-ratio:2/1;padding:1%'></div></div>",
            &[("t", [0.0, 0.0, 6.0, 6.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;max-width:1%'></div></div>",
            &[("t", [0.0, 0.0, 4.0, 4.0])],
        ),
        (
            "<div><div id=t style='float:left;display:inline-grid;aspect-ratio:auto 2/1;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 4.0, 4.0])],
        ),
        (
            "<div><div id=t style='float:left;display:block;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:2px'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div><div id=t style='float:left;display:flow-root;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:2px'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
    ]);
}

/// A flex or grid container laid out as a flex item, a grid item or the content
/// of a table cell, including rooms narrower than the floor.
#[test]
fn a_container_inside_a_flex_or_grid_container_or_a_table_keeps_the_floor() {
    assert_rows(&[
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 5.0]),
                ("s", [8.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 8.0, 5.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;align-items:start'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 5.0]),
                ("s", [8.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 8.0, 4.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;flex-wrap:wrap'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 5.0]),
                ("s", [8.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 8.0, 5.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;width:300px;justify-items:start'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("c", [0.0, 0.0, 300.0, 4.0]), ("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:inline-flex;aspect-ratio:1/2;box-sizing:border-box;border:2px solid'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 8.0]),
                ("s", [4.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 4.0, 8.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;padding:3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 6.0]),
                ("s", [12.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 12.0, 6.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;align-items:start'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;padding:3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 6.0]),
                ("s", [12.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 12.0, 6.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;flex-wrap:wrap'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;padding:3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 6.0]),
                ("s", [12.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 12.0, 6.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;width:300px;justify-items:start'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;padding:3px'></div></div>",
            &[("c", [0.0, 0.0, 300.0, 6.0]), ("t", [0.0, 0.0, 12.0, 6.0])],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:inline-flex;aspect-ratio:1/2;box-sizing:border-box;padding:3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 12.0]),
                ("s", [6.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 6.0, 12.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 8.0]),
                ("s", [16.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 16.0, 8.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;align-items:start'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 8.0]),
                ("s", [16.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 16.0, 8.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;flex-wrap:wrap'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 8.0]),
                ("s", [16.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 16.0, 8.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;width:300px;justify-items:start'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div></div>",
            &[("c", [0.0, 0.0, 300.0, 8.0]), ("t", [0.0, 0.0, 16.0, 8.0])],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:inline-flex;aspect-ratio:1/2;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 24.0]),
                ("s", [12.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 12.0, 24.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 5.0]),
                ("s", [8.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 8.0, 5.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;align-items:start'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 5.0]),
                ("s", [8.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 8.0, 4.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;flex-wrap:wrap'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 5.0]),
                ("s", [8.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 8.0, 5.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;width:300px;justify-items:start'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("c", [0.0, 0.0, 300.0, 4.0]), ("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:inline-grid;aspect-ratio:1/2;box-sizing:border-box;border:2px solid'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 8.0]),
                ("s", [4.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 4.0, 8.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;padding:3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 6.0]),
                ("s", [12.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 12.0, 6.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;align-items:start'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;padding:3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 6.0]),
                ("s", [12.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 12.0, 6.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;flex-wrap:wrap'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;padding:3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 6.0]),
                ("s", [12.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 12.0, 6.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;width:300px;justify-items:start'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;padding:3px'></div></div>",
            &[("c", [0.0, 0.0, 300.0, 6.0]), ("t", [0.0, 0.0, 12.0, 6.0])],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:inline-grid;aspect-ratio:1/2;box-sizing:border-box;padding:3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 12.0]),
                ("s", [6.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 6.0, 12.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 8.0]),
                ("s", [16.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 16.0, 8.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;align-items:start'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 8.0]),
                ("s", [16.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 16.0, 8.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;flex-wrap:wrap'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 8.0]),
                ("s", [16.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 16.0, 8.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;width:300px;justify-items:start'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div></div>",
            &[("c", [0.0, 0.0, 300.0, 8.0]), ("t", [0.0, 0.0, 16.0, 8.0])],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:inline-grid;aspect-ratio:1/2;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 24.0]),
                ("s", [12.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 12.0, 24.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 5.0]),
                ("s", [8.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 8.0, 5.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;align-items:start'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 5.0]),
                ("s", [8.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 8.0, 4.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;flex-wrap:wrap'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 5.0]),
                ("s", [8.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 8.0, 5.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:flex;aspect-ratio:1/2;box-sizing:border-box;border:2px solid'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 8.0]),
                ("s", [4.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 4.0, 8.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;padding:3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 6.0]),
                ("s", [12.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 12.0, 6.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;align-items:start'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;padding:3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 6.0]),
                ("s", [12.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 12.0, 6.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;flex-wrap:wrap'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;padding:3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 6.0]),
                ("s", [12.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 12.0, 6.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:flex;aspect-ratio:1/2;box-sizing:border-box;padding:3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 12.0]),
                ("s", [6.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 6.0, 12.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 8.0]),
                ("s", [16.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 16.0, 8.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;align-items:start'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 8.0]),
                ("s", [16.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 16.0, 8.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;flex-wrap:wrap'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 8.0]),
                ("s", [16.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 16.0, 8.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:flex;aspect-ratio:1/2;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 24.0]),
                ("s", [12.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 12.0, 24.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 5.0]),
                ("s", [8.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 8.0, 5.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;align-items:start'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 5.0]),
                ("s", [8.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 8.0, 4.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;flex-wrap:wrap'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 5.0]),
                ("s", [8.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 8.0, 5.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:grid;aspect-ratio:1/2;box-sizing:border-box;border:2px solid'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 8.0]),
                ("s", [4.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 4.0, 8.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;padding:3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 6.0]),
                ("s", [12.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 12.0, 6.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;align-items:start'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;padding:3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 6.0]),
                ("s", [12.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 12.0, 6.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;flex-wrap:wrap'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;padding:3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 6.0]),
                ("s", [12.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 12.0, 6.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:grid;aspect-ratio:1/2;box-sizing:border-box;padding:3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 12.0]),
                ("s", [6.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 6.0, 12.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 8.0]),
                ("s", [16.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 16.0, 8.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;align-items:start'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 8.0]),
                ("s", [16.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 16.0, 8.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;flex-wrap:wrap'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 8.0]),
                ("s", [16.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 16.0, 8.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;width:300px;'><div id=t style='display:grid;aspect-ratio:1/2;box-sizing:border-box;border:2px solid;padding:1px 4px 3px'></div><div id=s style='width:20px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 300.0, 24.0]),
                ("s", [12.0, 0.0, 20.0, 5.0]),
                ("t", [0.0, 0.0, 12.0, 24.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;width:5px'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("c", [0.0, 0.0, 5.0, 4.0]), ("t", [0.0, 0.0, 5.0, 4.0])],
        ),
        (
            "<div id=c style='display:grid;width:5px'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("c", [0.0, 0.0, 5.0, 4.0]), ("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div id=c style='display:flex;width:5px'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("c", [0.0, 0.0, 5.0, 4.0]), ("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div id=c style='display:flex;flex-wrap:wrap;width:10px'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div><div id=s style='width:5px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 10.0, 9.0]),
                ("s", [0.0, 4.0, 5.0, 5.0]),
                ("t", [0.0, 0.0, 8.0, 4.0]),
            ],
        ),
        (
            "<div style='display:inline-flex'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 4.0, 8.0, 4.0])],
        ),
        (
            "<table><tr><td id=d style='padding:0'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></td></tr></table>",
            &[("d", [2.0, 2.0, 8.0, 4.0]), ("t", [2.0, 2.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:5px'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div><div id=n style='height:3px;width:3px'></div>",
            &[("n", [0.0, 4.0, 3.0, 3.0]), ("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;width:5px'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("c", [0.0, 0.0, 5.0, 4.0]), ("t", [0.0, 0.0, 5.0, 4.0])],
        ),
        (
            "<div id=c style='display:grid;width:5px'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("c", [0.0, 0.0, 5.0, 4.0]), ("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div id=c style='display:flex;width:5px'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("c", [0.0, 0.0, 5.0, 4.0]), ("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div id=c style='display:flex;flex-wrap:wrap;width:10px'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div><div id=s style='width:5px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 10.0, 9.0]),
                ("s", [0.0, 4.0, 5.0, 5.0]),
                ("t", [0.0, 0.0, 8.0, 4.0]),
            ],
        ),
        (
            "<div style='display:inline-flex'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 4.0, 8.0, 4.0])],
        ),
        (
            "<table><tr><td id=d style='padding:0'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></td></tr></table>",
            &[("d", [2.0, 2.0, 8.0, 4.0]), ("t", [2.0, 2.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:5px'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div><div id=n style='height:3px;width:3px'></div>",
            &[("n", [0.0, 10.0, 3.0, 3.0]), ("t", [0.0, 4.0, 8.0, 4.0])],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;width:5px'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("c", [0.0, 0.0, 5.0, 4.0]), ("t", [0.0, 0.0, 5.0, 4.0])],
        ),
        (
            "<div id=c style='display:grid;width:5px'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("c", [0.0, 0.0, 5.0, 4.0]), ("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div id=c style='display:flex;width:5px'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("c", [0.0, 0.0, 5.0, 4.0]), ("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div id=c style='display:flex;flex-wrap:wrap;width:10px'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div><div id=s style='width:5px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 10.0, 9.0]),
                ("s", [0.0, 4.0, 5.0, 5.0]),
                ("t", [0.0, 0.0, 8.0, 4.0]),
            ],
        ),
        (
            "<div style='display:inline-flex'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 4.0, 8.0, 4.0])],
        ),
        (
            "<table><tr><td id=d style='padding:0'><div id=t style='display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></td></tr></table>",
            &[("d", [2.0, 2.0, 8.0, 10.0]), ("t", [2.0, 6.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:5px'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div><div id=n style='height:3px;width:3px'></div>",
            &[("n", [0.0, 10.0, 3.0, 3.0]), ("t", [0.0, 4.0, 8.0, 4.0])],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;width:5px'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("c", [0.0, 0.0, 5.0, 4.0]), ("t", [0.0, 0.0, 5.0, 4.0])],
        ),
        (
            "<div id=c style='display:grid;width:5px'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("c", [0.0, 0.0, 5.0, 4.0]), ("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div id=c style='display:flex;width:5px'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("c", [0.0, 0.0, 5.0, 4.0]), ("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div id=c style='display:flex;flex-wrap:wrap;width:10px'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div><div id=s style='width:5px;height:5px'></div></div>",
            &[
                ("c", [0.0, 0.0, 10.0, 9.0]),
                ("s", [0.0, 4.0, 5.0, 5.0]),
                ("t", [0.0, 0.0, 8.0, 4.0]),
            ],
        ),
        (
            "<div style='display:inline-flex'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 4.0, 8.0, 4.0])],
        ),
        (
            "<table><tr><td id=d style='padding:0'><div id=t style='display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></td></tr></table>",
            &[("d", [2.0, 2.0, 8.0, 10.0]), ("t", [2.0, 6.0, 8.0, 4.0])],
        ),
        (
            "<div id=c style='display:flex;width:5px'><div id=t style='display:block;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:0'></div></div>",
            &[("c", [0.0, 0.0, 5.0, 4.0]), ("t", [0.0, 0.0, 5.0, 4.0])],
        ),
        (
            "<div id=c style='display:grid;width:5px;justify-items:stretch'><div id=t style='display:block;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;justify-self:stretch'></div></div>",
            &[("c", [0.0, 0.0, 5.0, 4.0]), ("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:5px'><div id=t style='display:flow-root;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:5px'><div id=t style='display:flow-root;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'>X</div></div>",
            &[("t", [0.0, 0.0, 8.0, 14.0])],
        ),
        (
            "<div id=c style='display:flex;width:5px'><div id=t style='display:flow-root;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:0'></div></div>",
            &[("c", [0.0, 0.0, 5.0, 4.0]), ("t", [0.0, 0.0, 5.0, 4.0])],
        ),
        (
            "<div id=c style='display:grid;width:5px;justify-items:stretch'><div id=t style='display:flow-root;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;justify-self:stretch'></div></div>",
            &[("c", [0.0, 0.0, 5.0, 4.0]), ("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:5px'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:5px'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'>X</div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div id=c style='display:flex;width:5px'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:0'></div></div>",
            &[("c", [0.0, 0.0, 5.0, 4.0]), ("t", [0.0, 0.0, 5.0, 4.0])],
        ),
        (
            "<div id=c style='display:grid;width:5px;justify-items:stretch'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;justify-self:stretch'></div></div>",
            &[("c", [0.0, 0.0, 5.0, 4.0]), ("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:5px'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:5px'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'>X</div></div>",
            &[("t", [0.0, 0.0, 8.0, 14.0])],
        ),
        (
            "<div id=c style='display:flex;width:5px'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:0'></div></div>",
            &[("c", [0.0, 0.0, 5.0, 4.0]), ("t", [0.0, 0.0, 5.0, 4.0])],
        ),
        (
            "<div id=c style='display:grid;width:5px;justify-items:stretch'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;justify-self:stretch'></div></div>",
            &[("c", [0.0, 0.0, 5.0, 4.0]), ("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:5px'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:0'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:5px'><div id=t style='display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:2px'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:5px'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:0'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:5px'><div id=t style='display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:2px'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:5px'><div id=t style='display:flow-root;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:0'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='width:5px'><div id=t style='display:flow-root;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:2px'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
    ]);
}

/// An absolutely positioned flex, grid or block container, shrink-to-fit or
/// sized by its insets.
#[test]
fn an_absolutely_positioned_container_keeps_the_floor() {
    assert_rows(&[
        (
            "<div style='position:relative;width:300px;height:100px'><div id=t style='position:absolute;display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='position:relative;width:300px;height:100px'><div id=t style='position:absolute;left:0;right:0;display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 300.0, 150.0])],
        ),
        (
            "<div style='position:relative;width:300px;height:100px'><div id=t style='position:absolute;display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='position:relative;width:300px;height:100px'><div id=t style='position:absolute;left:0;right:0;display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 300.0, 150.0])],
        ),
        (
            "<div style='position:relative;width:300px;height:100px'><div id=t style='position:absolute;display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='position:relative;width:300px;height:100px'><div id=t style='position:absolute;left:0;right:0;display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 300.0, 150.0])],
        ),
        (
            "<div style='position:relative;width:300px;height:100px'><div id=t style='position:absolute;display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='position:relative;width:300px;height:100px'><div id=t style='position:absolute;left:0;right:0;display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 300.0, 150.0])],
        ),
        (
            "<div style='position:relative;width:300px;height:100px'><div id=t style='position:absolute;display:block;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='position:relative;width:300px;height:100px'><div id=t style='position:absolute;display:block;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:2px'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='position:relative;width:300px;height:100px'><div id=t style='position:absolute;display:flow-root;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='position:relative;width:300px;height:100px'><div id=t style='position:absolute;display:flow-root;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:2px'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='position:relative;width:300px;height:100px'><div id=t style='position:absolute;display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:2px'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='position:relative;width:300px;height:100px'><div id=t style='position:absolute;display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:2px'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='position:relative;width:300px;height:100px'><div id=t style='position:absolute;left:0;width:auto;right:295px;display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:0'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='position:relative;width:300px;height:100px'><div id=t style='position:absolute;left:0;right:295px;display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='position:relative;width:300px;height:100px'><div id=t style='position:absolute;left:0;width:auto;right:295px;display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:0'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='position:relative;width:300px;height:100px'><div id=t style='position:absolute;left:0;right:295px;display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='position:relative;width:300px;height:100px'><div id=t style='position:absolute;left:0;width:auto;right:295px;display:block;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:0'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='position:relative;width:300px;height:100px'><div id=t style='position:absolute;left:0;right:295px;display:block;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='position:relative;width:300px;height:100px'><div id=t style='position:absolute;left:0;width:auto;right:295px;display:flow-root;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:0'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
        (
            "<div style='position:relative;width:300px;height:100px'><div id=t style='position:absolute;left:0;right:295px;display:flow-root;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'></div></div>",
            &[("t", [0.0, 0.0, 8.0, 4.0])],
        ),
    ]);
}

/// A container or an inline-block whose content is narrower than the floor.
#[test]
fn a_box_with_narrow_content_is_as_wide_as_its_ratio_gives_its_edges() {
    assert_rows(&[
        (
            "<div><div id=t style='float:left;display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'><span id=k style='display:inline-block;width:3px;height:1px'></span></div></div>",
            &[("k", [2.0, 2.0, 3.0, 1.0]), ("t", [0.0, 0.0, 8.0, 5.0])],
        ),
        (
            "<div><div id=t style='float:left;display:flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'><div id=k style='width:3px;height:1px'></div></div></div>",
            &[("k", [2.0, 2.0, 3.0, 1.0]), ("t", [0.0, 0.0, 8.0, 5.0])],
        ),
        (
            "<div><div id=t style='float:left;display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'><span id=k style='display:inline-block;width:3px;height:1px'></span></div></div>",
            &[("k", [2.0, 2.0, 3.0, 1.0]), ("t", [0.0, 0.0, 8.0, 5.0])],
        ),
        (
            "<div><div id=t style='float:left;display:grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'><div id=k style='width:3px;height:1px'></div></div></div>",
            &[("k", [2.0, 2.0, 3.0, 1.0]), ("t", [0.0, 0.0, 8.0, 5.0])],
        ),
        (
            "<div><div id=t style='float:left;display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'><div id=k style='width:3px;height:1px'></div></div></div>",
            &[("k", [2.0, 2.0, 3.0, 1.0]), ("t", [0.0, 0.0, 8.0, 5.0])],
        ),
        (
            "<div><div id=t style='float:left;display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'><div id=k style='width:3px;height:1px'></div></div></div>",
            &[("k", [2.0, 2.0, 3.0, 1.0]), ("t", [0.0, 0.0, 8.0, 5.0])],
        ),
        (
            "<div><div id=t style='display:inline-flex;border:2px solid'><span id=k style='display:inline-block;width:3px;height:1px'></span></div></div>",
            &[("k", [2.0, 7.0, 3.0, 1.0]), ("t", [0.0, 5.0, 7.0, 5.0])],
        ),
        (
            "<div><div id=t style='display:inline-flex;display:inline-flex;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'><span id=k style='display:inline-block;width:3px;height:1px'></span></div></div>",
            &[("k", [2.0, 7.0, 3.0, 1.0]), ("t", [0.0, 5.0, 8.0, 5.0])],
        ),
        (
            "<div><div id=t style='display:inline-grid;border:2px solid'><span id=k style='display:inline-block;width:3px;height:1px'></span></div></div>",
            &[("k", [2.0, 7.0, 3.0, 1.0]), ("t", [0.0, 5.0, 7.0, 5.0])],
        ),
        (
            "<div><div id=t style='display:inline-grid;display:inline-grid;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'><span id=k style='display:inline-block;width:3px;height:1px'></span></div></div>",
            &[("k", [2.0, 7.0, 3.0, 1.0]), ("t", [0.0, 5.0, 8.0, 5.0])],
        ),
        (
            "<div><div id=t style='display:inline-block;border:2px solid'><span id=k style='display:inline-block;width:3px;height:1px'></span></div></div>",
            &[("k", [2.0, 9.0, 3.0, 1.0]), ("t", [0.0, 0.0, 7.0, 14.0])],
        ),
        (
            "<div><div id=t style='display:inline-block;display:inline-block;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'><span id=k style='display:inline-block;width:3px;height:1px'></span></div></div>",
            &[("k", [2.0, 9.0, 3.0, 1.0]), ("t", [0.0, 0.0, 8.0, 14.0])],
        ),
        (
            "<div><div id=t style='display:inline-flow-root;border:2px solid'><span id=k style='display:inline-block;width:3px;height:1px'></span></div></div>",
            &[("k", [2.0, 9.0, 3.0, 1.0]), ("t", [0.0, 0.0, 800.0, 14.0])],
        ),
        (
            "<div><div id=t style='display:inline-flow-root;display:inline-flow-root;aspect-ratio:2/1;box-sizing:border-box;border:2px solid'><span id=k style='display:inline-block;width:3px;height:1px'></span></div></div>",
            &[("k", [2.0, 9.0, 3.0, 1.0]), ("t", [0.0, 0.0, 800.0, 400.0])],
        ),
    ]);
}
