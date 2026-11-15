//! A flex item with an `aspect-ratio` and `auto` sizes in a row flex
//! container, and a box with an `aspect-ratio` that relates its border
//! boxes, sized by its content:
//!
//! - The automatic minimum height of the item is the height of its content,
//!   which the `max-width` carried across the ratio does not clamp (CSS Sizing
//!   4 §5.1).
//! - A ratio that relates the content boxes carries a `max-height` across
//!   between the content boxes, not with the padding and border of the height.
//! - A box that relates its border boxes is no shorter than its padding and
//!   border, and the ratio gives an `auto` width from that height.
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

/// A container style, an item style, the text of the item, and the border
/// boxes of the container, the item and its 20px by 5px sibling.
type Row = (
    &'static str,
    &'static str,
    &'static str,
    [f32; 4],
    [f32; 4],
    [f32; 4],
);

fn assert_rows(rows: &[Row]) {
    let mut failures = Vec::new();
    for (container, item, text, c, t, s) in rows {
        let body = format!(
            "<div id=c style='{container}'><div id=t style='{item}'>{text}</div>\
             <div id=s style='width:20px;height:5px'></div></div>"
        );
        let found = border_boxes(&body);
        for (id, want) in [("c", c), ("t", t), ("s", s)] {
            let got = found.get(id).copied();
            let matches = got.is_some_and(|got| {
                got.iter()
                    .zip(want.iter())
                    .all(|(got, want)| (got - want).abs() < 0.02)
            });
            if !matches {
                failures.push(format!(
                    "{body}
  #{id}: expected {want:?}, got {got:?}"
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{}",
        failures.join(
            "
"
        )
    );
}

#[test]
fn the_content_height_of_an_item_is_not_clamped_by_a_carried_maximum() {
    assert_rows(&[
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;max-width:10px",
            "X",
            [0.0, 0.0, 300.0, 10.0],
            [0.0, 0.0, 10.0, 10.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;max-width:10px",
            "XX XX",
            [0.0, 0.0, 300.0, 20.0],
            [0.0, 0.0, 10.0, 20.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;max-width:20px",
            "XX XX XX",
            [0.0, 0.0, 300.0, 30.0],
            [0.0, 0.0, 20.0, 30.0],
            [20.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:1/1;max-width:10px",
            "XX XX",
            [0.0, 0.0, 300.0, 20.0],
            [0.0, 0.0, 10.0, 20.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:4/1;max-width:40px",
            "XXXX XXXX",
            [0.0, 0.0, 300.0, 20.0],
            [0.0, 0.0, 40.0, 20.0],
            [40.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;max-width:10px;padding:2px",
            "X",
            [0.0, 0.0, 300.0, 14.0],
            [0.0, 0.0, 14.0, 14.0],
            [14.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:2/1;max-width:10px",
            "X",
            [0.0, 0.0, 300.0, 10.0],
            [0.0, 0.0, 10.0, 10.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:2/1;max-width:10px",
            "XX XX",
            [0.0, 0.0, 300.0, 20.0],
            [0.0, 0.0, 10.0, 20.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:2/1;max-width:20px",
            "XX XX XX",
            [0.0, 0.0, 300.0, 30.0],
            [0.0, 0.0, 20.0, 30.0],
            [20.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:1/1;max-width:10px",
            "XX XX",
            [0.0, 0.0, 300.0, 20.0],
            [0.0, 0.0, 10.0, 20.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:4/1;max-width:40px",
            "XXXX XXXX",
            [0.0, 0.0, 300.0, 20.0],
            [0.0, 0.0, 40.0, 20.0],
            [40.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:2/1;max-width:10px;padding:2px",
            "X",
            [0.0, 0.0, 300.0, 14.0],
            [0.0, 0.0, 14.0, 14.0],
            [14.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:baseline",
            "aspect-ratio:2/1;max-width:10px",
            "X",
            [0.0, 0.0, 300.0, 10.0],
            [0.0, 0.0, 10.0, 10.0],
            [10.0, 3.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:baseline",
            "aspect-ratio:2/1;max-width:10px",
            "XX XX",
            [0.0, 0.0, 300.0, 20.0],
            [0.0, 0.0, 10.0, 20.0],
            [10.0, 3.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:baseline",
            "aspect-ratio:2/1;max-width:20px",
            "XX XX XX",
            [0.0, 0.0, 300.0, 30.0],
            [0.0, 0.0, 20.0, 30.0],
            [20.0, 3.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:baseline",
            "aspect-ratio:1/1;max-width:10px",
            "XX XX",
            [0.0, 0.0, 300.0, 20.0],
            [0.0, 0.0, 10.0, 20.0],
            [10.0, 3.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:baseline",
            "aspect-ratio:4/1;max-width:40px",
            "XXXX XXXX",
            [0.0, 0.0, 300.0, 20.0],
            [0.0, 0.0, 40.0, 20.0],
            [40.0, 3.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:baseline",
            "aspect-ratio:2/1;max-width:10px;padding:2px",
            "X",
            [0.0, 0.0, 300.0, 14.0],
            [0.0, 0.0, 14.0, 14.0],
            [14.0, 5.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:center",
            "aspect-ratio:2/1;max-width:10px",
            "X",
            [0.0, 0.0, 300.0, 10.0],
            [0.0, 0.0, 10.0, 10.0],
            [10.0, 2.5, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:center",
            "aspect-ratio:2/1;max-width:10px",
            "XX XX",
            [0.0, 0.0, 300.0, 20.0],
            [0.0, 0.0, 10.0, 20.0],
            [10.0, 7.5, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:center",
            "aspect-ratio:2/1;max-width:20px",
            "XX XX XX",
            [0.0, 0.0, 300.0, 30.0],
            [0.0, 0.0, 20.0, 30.0],
            [20.0, 12.5, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:center",
            "aspect-ratio:1/1;max-width:10px",
            "XX XX",
            [0.0, 0.0, 300.0, 20.0],
            [0.0, 0.0, 10.0, 20.0],
            [10.0, 7.5, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:center",
            "aspect-ratio:4/1;max-width:40px",
            "XXXX XXXX",
            [0.0, 0.0, 300.0, 20.0],
            [0.0, 0.0, 40.0, 20.0],
            [40.0, 7.5, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:center",
            "aspect-ratio:2/1;max-width:10px;padding:2px",
            "X",
            [0.0, 0.0, 300.0, 14.0],
            [0.0, 0.0, 14.0, 14.0],
            [14.0, 4.5, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:2/1;max-width:10px",
            "X",
            [0.0, 0.0, 300.0, 10.0],
            [0.0, 0.0, 10.0, 10.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:2/1;max-width:10px",
            "XX XX",
            [0.0, 0.0, 300.0, 20.0],
            [0.0, 0.0, 10.0, 20.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:2/1;max-width:20px",
            "XX XX XX",
            [0.0, 0.0, 300.0, 30.0],
            [0.0, 0.0, 20.0, 30.0],
            [20.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:1/1;max-width:10px",
            "XX XX",
            [0.0, 0.0, 300.0, 20.0],
            [0.0, 0.0, 10.0, 20.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:4/1;max-width:40px",
            "XXXX XXXX",
            [0.0, 0.0, 300.0, 20.0],
            [0.0, 0.0, 40.0, 20.0],
            [40.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:2/1;max-width:10px;padding:2px",
            "X",
            [0.0, 0.0, 300.0, 14.0],
            [0.0, 0.0, 14.0, 14.0],
            [14.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:2/1;max-width:10px",
            "X",
            [0.0, 0.0, 300.0, 10.0],
            [290.0, 0.0, 10.0, 10.0],
            [270.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:2/1;max-width:10px",
            "XX XX",
            [0.0, 0.0, 300.0, 20.0],
            [290.0, 0.0, 10.0, 20.0],
            [270.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:2/1;max-width:20px",
            "XX XX XX",
            [0.0, 0.0, 300.0, 30.0],
            [280.0, 0.0, 20.0, 30.0],
            [260.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:1/1;max-width:10px",
            "XX XX",
            [0.0, 0.0, 300.0, 20.0],
            [290.0, 0.0, 10.0, 20.0],
            [270.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:4/1;max-width:40px",
            "XXXX XXXX",
            [0.0, 0.0, 300.0, 20.0],
            [260.0, 0.0, 40.0, 20.0],
            [240.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:2/1;max-width:10px;padding:2px",
            "X",
            [0.0, 0.0, 300.0, 14.0],
            [286.0, 0.0, 14.0, 14.0],
            [266.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:2/1;max-width:10px",
            "X",
            [0.0, 0.0, 30.0, 10.0],
            [0.0, 0.0, 10.0, 10.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:2/1;max-width:10px",
            "XX XX",
            [0.0, 0.0, 30.0, 20.0],
            [0.0, 0.0, 10.0, 20.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:2/1;max-width:20px",
            "XX XX XX",
            [0.0, 0.0, 40.0, 30.0],
            [0.0, 0.0, 20.0, 30.0],
            [20.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:1/1;max-width:10px",
            "XX XX",
            [0.0, 0.0, 30.0, 20.0],
            [0.0, 0.0, 10.0, 20.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:4/1;max-width:40px",
            "XXXX XXXX",
            [0.0, 0.0, 60.0, 20.0],
            [0.0, 0.0, 40.0, 20.0],
            [40.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:2/1;max-width:10px;padding:2px",
            "X",
            [0.0, 0.0, 34.0, 14.0],
            [0.0, 0.0, 14.0, 14.0],
            [14.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;max-width:10px;flex:1",
            "X",
            [0.0, 0.0, 300.0, 10.0],
            [0.0, 0.0, 10.0, 10.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;max-width:10px;flex-basis:40px",
            "X",
            [0.0, 0.0, 300.0, 10.0],
            [0.0, 0.0, 10.0, 10.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;max-width:10px;display:grid",
            "X",
            [0.0, 0.0, 300.0, 10.0],
            [0.0, 0.0, 10.0, 10.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:2/1;max-width:10px;flex:1",
            "X",
            [0.0, 0.0, 300.0, 10.0],
            [0.0, 0.0, 10.0, 10.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:2/1;max-width:10px;flex-basis:40px",
            "X",
            [0.0, 0.0, 300.0, 10.0],
            [0.0, 0.0, 10.0, 10.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:2/1;max-width:10px;display:grid",
            "X",
            [0.0, 0.0, 300.0, 10.0],
            [0.0, 0.0, 10.0, 10.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:12px",
            "aspect-ratio:2/1;max-width:50%",
            "XX XX XX XX",
            [0.0, 0.0, 12.0, 40.0],
            [0.0, 0.0, 6.0, 40.0],
            [6.0, 0.0, 6.0, 5.0],
        ),
        (
            "display:flex;width:12px",
            "aspect-ratio:2/1;max-width:10px;flex:1",
            "X",
            [0.0, 0.0, 12.0, 10.0],
            [0.0, 0.0, 10.0, 10.0],
            [10.0, 0.0, 2.0, 5.0],
        ),
        (
            "display:flex;width:12px",
            "aspect-ratio:2/1;max-width:10px;flex-basis:40px",
            "X",
            [0.0, 0.0, 12.0, 10.0],
            [0.0, 0.0, 10.0, 10.0],
            [10.0, 0.0, 2.0, 5.0],
        ),
        (
            "display:flex;width:12px",
            "aspect-ratio:2/1;max-width:10px;display:grid",
            "X",
            [0.0, 0.0, 12.0, 10.0],
            [0.0, 0.0, 10.0, 10.0],
            [10.0, 0.0, 2.0, 5.0],
        ),
    ]);
}

#[test]
fn a_content_box_ratio_carries_a_maximum_height_between_content_boxes() {
    assert_rows(&[
        (
            "display:flex;width:300px",
            "aspect-ratio:1/2;max-height:30px;padding:2px",
            "XX XX",
            [0.0, 0.0, 300.0, 34.0],
            [0.0, 0.0, 19.0, 34.0],
            [19.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:1/2;max-height:30px;border:2px solid",
            "XX XX",
            [0.0, 0.0, 300.0, 34.0],
            [0.0, 0.0, 19.0, 34.0],
            [19.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:1/2;max-height:30px;padding:2px 5px",
            "X",
            [0.0, 0.0, 300.0, 24.0],
            [0.0, 0.0, 20.0, 24.0],
            [20.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:1/2;max-height:30px;padding:2px",
            "XX XX",
            [0.0, 0.0, 300.0, 34.0],
            [0.0, 0.0, 19.0, 34.0],
            [19.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:1/2;max-height:30px;border:2px solid",
            "XX XX",
            [0.0, 0.0, 300.0, 34.0],
            [0.0, 0.0, 19.0, 34.0],
            [19.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:1/2;max-height:30px;padding:2px 5px",
            "X",
            [0.0, 0.0, 300.0, 24.0],
            [0.0, 0.0, 20.0, 24.0],
            [20.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:baseline",
            "aspect-ratio:1/2;max-height:30px;padding:2px",
            "XX XX",
            [0.0, 0.0, 300.0, 34.0],
            [0.0, 0.0, 19.0, 34.0],
            [19.0, 5.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:baseline",
            "aspect-ratio:1/2;max-height:30px;border:2px solid",
            "XX XX",
            [0.0, 0.0, 300.0, 34.0],
            [0.0, 0.0, 19.0, 34.0],
            [19.0, 5.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:baseline",
            "aspect-ratio:1/2;max-height:30px;padding:2px 5px",
            "X",
            [0.0, 0.0, 300.0, 24.0],
            [0.0, 0.0, 20.0, 24.0],
            [20.0, 5.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:center",
            "aspect-ratio:1/2;max-height:30px;padding:2px",
            "XX XX",
            [0.0, 0.0, 300.0, 34.0],
            [0.0, 0.0, 19.0, 34.0],
            [19.0, 14.5, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:center",
            "aspect-ratio:1/2;max-height:30px;border:2px solid",
            "XX XX",
            [0.0, 0.0, 300.0, 34.0],
            [0.0, 0.0, 19.0, 34.0],
            [19.0, 14.5, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:center",
            "aspect-ratio:1/2;max-height:30px;padding:2px 5px",
            "X",
            [0.0, 0.0, 300.0, 24.0],
            [0.0, 0.0, 20.0, 24.0],
            [20.0, 9.5, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:1/2;max-height:30px;padding:2px",
            "XX XX",
            [0.0, 0.0, 300.0, 34.0],
            [0.0, 0.0, 19.0, 34.0],
            [19.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:1/2;max-height:30px;border:2px solid",
            "XX XX",
            [0.0, 0.0, 300.0, 34.0],
            [0.0, 0.0, 19.0, 34.0],
            [19.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:1/2;max-height:30px;padding:2px 5px",
            "X",
            [0.0, 0.0, 300.0, 24.0],
            [0.0, 0.0, 20.0, 24.0],
            [20.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:1/2;max-height:30px;padding:2px",
            "XX XX",
            [0.0, 0.0, 300.0, 34.0],
            [281.0, 0.0, 19.0, 34.0],
            [261.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:1/2;max-height:30px;border:2px solid",
            "XX XX",
            [0.0, 0.0, 300.0, 34.0],
            [281.0, 0.0, 19.0, 34.0],
            [261.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:1/2;max-height:30px;padding:2px 5px",
            "X",
            [0.0, 0.0, 300.0, 24.0],
            [280.0, 0.0, 20.0, 24.0],
            [260.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:1/2;max-height:30px;padding:2px",
            "XX XX",
            [0.0, 0.0, 39.0, 34.0],
            [0.0, 0.0, 19.0, 34.0],
            [19.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:1/2;max-height:30px;border:2px solid",
            "XX XX",
            [0.0, 0.0, 39.0, 34.0],
            [0.0, 0.0, 19.0, 34.0],
            [19.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:1/2;max-height:30px;padding:2px 5px",
            "X",
            [0.0, 0.0, 40.0, 24.0],
            [0.0, 0.0, 20.0, 24.0],
            [20.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:1/2;max-height:30px;padding:10%",
            "X",
            [0.0, 0.0, 300.0, 80.0],
            [0.0, 0.0, 70.0, 80.0],
            [70.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:1/2;max-height:30px;padding:2px;display:flex",
            "XX XX",
            [0.0, 0.0, 300.0, 34.0],
            [0.0, 0.0, 19.0, 34.0],
            [19.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:1/2;max-height:30px;padding:10%",
            "X",
            [0.0, 0.0, 300.0, 80.0],
            [0.0, 0.0, 70.0, 80.0],
            [70.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:1/2;max-height:30px;padding:2px;display:flex",
            "XX XX",
            [0.0, 0.0, 300.0, 34.0],
            [0.0, 0.0, 19.0, 34.0],
            [19.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:12px",
            "aspect-ratio:1/2;max-height:30px;padding:2px;display:flex",
            "XX XX",
            [0.0, 0.0, 12.0, 34.0],
            [0.0, 0.0, 19.0, 34.0],
            [19.0, 0.0, 0.0, 5.0],
        ),
        (
            "display:flex;width:300px;height:100px;align-items:start",
            "aspect-ratio:1/2;padding:2px;max-height:30%",
            "XXXX",
            [0.0, 0.0, 300.0, 100.0],
            [0.0, 0.0, 19.0, 34.0],
            [19.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;height:100px;align-items:start",
            "aspect-ratio:1/2;padding:2px;max-height:calc(10px + 20%)",
            "XXXX",
            [0.0, 0.0, 300.0, 100.0],
            [0.0, 0.0, 19.0, 34.0],
            [19.0, 0.0, 20.0, 5.0],
        ),
    ]);
}

#[test]
fn a_border_box_ratio_gives_the_width_of_its_padding_and_border() {
    assert_rows(&[
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;max-width:10px;box-sizing:border-box;padding:2px",
            "X",
            [0.0, 0.0, 300.0, 14.0],
            [0.0, 0.0, 10.0, 14.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;max-height:20px;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 8.0, 5.0],
            [8.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 8.0, 5.0],
            [8.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;max-height:20px;box-sizing:border-box;padding:3px",
            "",
            [0.0, 0.0, 300.0, 6.0],
            [0.0, 0.0, 12.0, 6.0],
            [12.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;max-height:2px;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 8.0, 4.0],
            [8.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:2/1;max-width:10px;box-sizing:border-box;padding:2px",
            "X",
            [0.0, 0.0, 300.0, 14.0],
            [0.0, 0.0, 10.0, 14.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:2/1;max-height:20px;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 8.0, 4.0],
            [8.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 8.0, 4.0],
            [8.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:2/1;max-height:20px;box-sizing:border-box;padding:3px",
            "",
            [0.0, 0.0, 300.0, 6.0],
            [0.0, 0.0, 12.0, 6.0],
            [12.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:2/1;max-height:2px;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 8.0, 4.0],
            [8.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:baseline",
            "aspect-ratio:2/1;max-width:10px;box-sizing:border-box;padding:2px",
            "X",
            [0.0, 0.0, 300.0, 14.0],
            [0.0, 0.0, 10.0, 14.0],
            [10.0, 5.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:baseline",
            "aspect-ratio:2/1;max-height:20px;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 1.0, 8.0, 4.0],
            [8.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:baseline",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 1.0, 8.0, 4.0],
            [8.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:baseline",
            "aspect-ratio:2/1;max-height:20px;box-sizing:border-box;padding:3px",
            "",
            [0.0, 0.0, 300.0, 6.0],
            [0.0, 0.0, 12.0, 6.0],
            [12.0, 1.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:baseline",
            "aspect-ratio:2/1;max-height:2px;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 1.0, 8.0, 4.0],
            [8.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:center",
            "aspect-ratio:2/1;max-width:10px;box-sizing:border-box;padding:2px",
            "X",
            [0.0, 0.0, 300.0, 14.0],
            [0.0, 0.0, 10.0, 14.0],
            [10.0, 4.5, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:center",
            "aspect-ratio:2/1;max-height:20px;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.5, 8.0, 4.0],
            [8.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:center",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.5, 8.0, 4.0],
            [8.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:center",
            "aspect-ratio:2/1;max-height:20px;box-sizing:border-box;padding:3px",
            "",
            [0.0, 0.0, 300.0, 6.0],
            [0.0, 0.0, 12.0, 6.0],
            [12.0, 0.5, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:center",
            "aspect-ratio:2/1;max-height:2px;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.5, 8.0, 4.0],
            [8.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:2/1;max-width:10px;box-sizing:border-box;padding:2px",
            "X",
            [0.0, 0.0, 300.0, 14.0],
            [0.0, 0.0, 10.0, 14.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:2/1;max-height:20px;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 8.0, 5.0],
            [8.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 8.0, 5.0],
            [8.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:2/1;max-height:20px;box-sizing:border-box;padding:3px",
            "",
            [0.0, 0.0, 300.0, 6.0],
            [0.0, 0.0, 12.0, 6.0],
            [12.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-wrap:wrap",
            "aspect-ratio:2/1;max-height:2px;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 8.0, 4.0],
            [8.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:2/1;max-width:10px;box-sizing:border-box;padding:2px",
            "X",
            [0.0, 0.0, 300.0, 14.0],
            [290.0, 0.0, 10.0, 14.0],
            [270.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:2/1;max-height:20px;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [292.0, 0.0, 8.0, 5.0],
            [272.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [292.0, 0.0, 8.0, 5.0],
            [272.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:2/1;max-height:20px;box-sizing:border-box;padding:3px",
            "",
            [0.0, 0.0, 300.0, 6.0],
            [288.0, 0.0, 12.0, 6.0],
            [268.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;flex-direction:row-reverse",
            "aspect-ratio:2/1;max-height:2px;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [292.0, 0.0, 8.0, 4.0],
            [272.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:2/1;max-width:10px;box-sizing:border-box;padding:2px",
            "X",
            [0.0, 0.0, 30.0, 14.0],
            [0.0, 0.0, 10.0, 14.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:2/1;max-height:20px;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 3.0, 28.0, 5.0],
            [0.0, 3.0, 8.0, 5.0],
            [8.0, 3.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 3.0, 28.0, 5.0],
            [0.0, 3.0, 8.0, 5.0],
            [8.0, 3.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:2/1;max-height:20px;box-sizing:border-box;padding:3px",
            "",
            [0.0, 2.0, 32.0, 6.0],
            [0.0, 2.0, 12.0, 6.0],
            [12.0, 2.0, 20.0, 5.0],
        ),
        (
            "display:inline-flex",
            "aspect-ratio:2/1;max-height:2px;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 4.0, 28.0, 5.0],
            [0.0, 4.0, 8.0, 4.0],
            [8.0, 4.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:auto 1/2;max-height:30px;padding:2px;box-sizing:border-box",
            "XX XX",
            [0.0, 0.0, 300.0, 30.0],
            [0.0, 0.0, 17.0, 30.0],
            [17.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:2px",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 8.0, 5.0],
            [8.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid;max-width:5px",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 5.0, 5.0],
            [5.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;box-sizing:border-box;padding:10%",
            "",
            [0.0, 0.0, 300.0, 60.0],
            [0.0, 0.0, 120.0, 60.0],
            [120.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid;overflow:hidden",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 8.0, 5.0],
            [8.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid;flex-basis:0",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 8.0, 5.0],
            [8.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:auto 1/2;max-height:30px;padding:2px;box-sizing:border-box",
            "XX XX",
            [0.0, 0.0, 300.0, 30.0],
            [0.0, 0.0, 17.0, 30.0],
            [17.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:2px",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 8.0, 4.0],
            [8.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid;max-width:5px",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 5.0, 4.0],
            [5.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:2/1;box-sizing:border-box;padding:10%",
            "",
            [0.0, 0.0, 300.0, 60.0],
            [0.0, 0.0, 120.0, 60.0],
            [120.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid;overflow:hidden",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 8.0, 4.0],
            [8.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid;flex-basis:0",
            "",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 8.0, 4.0],
            [8.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:12px",
            "aspect-ratio:auto 1/2;max-height:30px;padding:2px;box-sizing:border-box",
            "XX XX",
            [0.0, 0.0, 12.0, 30.0],
            [0.0, 0.0, 17.0, 30.0],
            [17.0, 0.0, 0.0, 5.0],
        ),
        (
            "display:flex;width:12px",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:2px",
            "",
            [0.0, 0.0, 12.0, 5.0],
            [0.0, 0.0, 5.32812, 5.0],
            [5.32812, 0.0, 6.67188, 5.0],
        ),
        (
            "display:flex;width:12px",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid;flex:1",
            "",
            [0.0, 0.0, 12.0, 5.0],
            [0.0, 0.0, 8.0, 5.0],
            [8.0, 0.0, 4.0, 5.0],
        ),
        (
            "display:flex;width:12px",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid;max-width:5px",
            "",
            [0.0, 0.0, 12.0, 5.0],
            [0.0, 0.0, 5.0, 5.0],
            [5.0, 0.0, 7.0, 5.0],
        ),
        (
            "display:flex;width:12px",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid;overflow:hidden",
            "",
            [0.0, 0.0, 12.0, 5.0],
            [0.0, 0.0, 5.32812, 5.0],
            [5.32812, 0.0, 6.67188, 5.0],
        ),
        (
            "display:flex;width:12px",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid;flex-basis:0",
            "",
            [0.0, 0.0, 12.0, 5.0],
            [0.0, 0.0, 8.0, 5.0],
            [8.0, 0.0, 4.0, 5.0],
        ),
        (
            "display:grid;width:300px;justify-items:start",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid;overflow:hidden",
            "",
            [0.0, 0.0, 300.0, 9.0],
            [0.0, 0.0, 8.0, 4.0],
            [0.0, 4.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;max-height:2px;box-sizing:border-box;padding:1%",
            "",
            [0.0, 0.0, 300.0, 6.0],
            [0.0, 0.0, 12.0, 6.0],
            [12.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:2/1;max-height:2px;box-sizing:border-box;padding:1%",
            "",
            [0.0, 0.0, 300.0, 6.0],
            [0.0, 0.0, 12.0, 6.0],
            [12.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;height:100px;align-items:start",
            "aspect-ratio:2/1;max-height:2px;box-sizing:border-box;padding:1%",
            "",
            [0.0, 0.0, 300.0, 100.0],
            [0.0, 0.0, 12.0, 6.0],
            [12.0, 0.0, 20.0, 5.0],
        ),
    ]);
}

/// A minimum that opts out of the content height, a scroll container, an item
/// that grows past the width its `max-height` carries over, and an item that
/// shrinks to the width of its content keep the sizes they had.
#[test]
fn the_limits_leave_other_items_alone() {
    assert_rows(&[
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;max-width:10px;min-height:0",
            "X",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 10.0, 5.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:2/1;max-width:10px;overflow:hidden",
            "X",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 10.0, 5.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:2/1;max-width:10px;min-height:0",
            "X",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 10.0, 5.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:2/1;max-width:10px;overflow:hidden",
            "X",
            [0.0, 0.0, 300.0, 5.0],
            [0.0, 0.0, 10.0, 5.0],
            [10.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:1/2;max-height:30px;padding:2px;flex:1",
            "X",
            [0.0, 0.0, 300.0, 34.0],
            [0.0, 0.0, 280.0, 34.0],
            [280.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px;align-items:start",
            "aspect-ratio:1/2;max-height:30px;padding:2px;flex:1",
            "X",
            [0.0, 0.0, 300.0, 34.0],
            [0.0, 0.0, 280.0, 34.0],
            [280.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:12px",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid",
            "X",
            [0.0, 0.0, 12.0, 14.0],
            [0.0, 0.0, 14.0, 14.0],
            [14.0, 0.0, 0.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:1/2;padding:2px;max-height:30%",
            "XXXX",
            [0.0, 0.0, 300.0, 84.0],
            [0.0, 0.0, 44.0, 84.0],
            [44.0, 0.0, 20.0, 5.0],
        ),
        (
            "display:flex;width:300px",
            "aspect-ratio:1/2;padding:2px;max-height:calc(10px + 20%)",
            "XXXX",
            [0.0, 0.0, 300.0, 84.0],
            [0.0, 0.0, 44.0, 84.0],
            [44.0, 0.0, 20.0, 5.0],
        ),
    ]);
}

/// A box that relates its border boxes and is sized by its content is as wide
/// as its ratio gives its padding and border, within the width it states, and
/// a box that fills the room it is offered keeps that width.
#[test]
fn a_border_box_ratio_sized_by_its_content_is_as_wide_as_its_edges_give() {
    let rows: &[(&str, &str, &str, [f32; 4])] = &[
        (
            "300px",
            "display:inline-block;aspect-ratio:2/1;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 4.0, 8.0, 4.0],
        ),
        (
            "300px",
            "display:inline-block;aspect-ratio:2/1;box-sizing:border-box;padding:3px",
            "",
            [0.0, 2.0, 12.0, 6.0],
        ),
        (
            "300px",
            "float:left;aspect-ratio:2/1;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 8.0, 4.0],
        ),
        (
            "300px",
            "float:left;aspect-ratio:2/1;box-sizing:border-box;padding:3px",
            "",
            [0.0, 0.0, 12.0, 6.0],
        ),
        (
            "300px",
            "position:absolute;aspect-ratio:2/1;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 8.0, 4.0],
        ),
        (
            "300px",
            "position:absolute;aspect-ratio:2/1;box-sizing:border-box;padding:3px",
            "",
            [0.0, 0.0, 12.0, 6.0],
        ),
        (
            "300px",
            "display:table-cell;aspect-ratio:2/1;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 8.0, 4.0],
        ),
        (
            "300px",
            "display:table-cell;aspect-ratio:2/1;box-sizing:border-box;padding:3px",
            "",
            [0.0, 0.0, 12.0, 6.0],
        ),
        (
            "4px",
            "display:inline-block;width:stretch;aspect-ratio:2/1;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 4.0, 4.0, 4.0],
        ),
        (
            "300px",
            "display:inline-block;height:50%;aspect-ratio:2/1;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 4.0, 8.0, 4.0],
        ),
        (
            "300px",
            "display:inline-block;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;max-width:5px",
            "",
            [0.0, 4.0, 5.0, 4.0],
        ),
        (
            "300px",
            "display:inline-block;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:20px",
            "",
            [0.0, 0.0, 20.0, 10.0],
        ),
        (
            "300px",
            "display:inline-block;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;max-height:2px",
            "",
            [0.0, 4.0, 8.0, 4.0],
        ),
        (
            "300px",
            "display:inline-block;aspect-ratio:2/1;box-sizing:border-box;border:2px solid",
            "X",
            [0.0, 0.0, 14.0, 14.0],
        ),
        (
            "300px",
            "display:inline-block;aspect-ratio:2/1;box-sizing:border-box;padding:2px 30px",
            "",
            [0.0, 0.0, 60.0, 30.0],
        ),
        (
            "4px",
            "float:left;width:stretch;aspect-ratio:2/1;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 4.0, 4.0],
        ),
        (
            "300px",
            "float:left;height:50%;aspect-ratio:2/1;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 8.0, 4.0],
        ),
        (
            "300px",
            "float:left;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;max-width:5px",
            "",
            [0.0, 0.0, 5.0, 4.0],
        ),
        (
            "300px",
            "float:left;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;min-width:20px",
            "",
            [0.0, 0.0, 20.0, 10.0],
        ),
        (
            "300px",
            "float:left;aspect-ratio:2/1;box-sizing:border-box;border:2px solid;max-height:2px",
            "",
            [0.0, 0.0, 8.0, 4.0],
        ),
        (
            "300px",
            "float:left;aspect-ratio:2/1;box-sizing:border-box;border:2px solid",
            "X",
            [0.0, 0.0, 14.0, 14.0],
        ),
        (
            "300px",
            "float:left;aspect-ratio:2/1;box-sizing:border-box;padding:2px 30px",
            "",
            [0.0, 0.0, 60.0, 30.0],
        ),
        (
            "300px",
            "aspect-ratio:2/1;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 300.0, 150.0],
        ),
        (
            "300px",
            "width:stretch;aspect-ratio:2/1;box-sizing:border-box;border:2px solid",
            "",
            [0.0, 0.0, 300.0, 150.0],
        ),
    ];
    let mut failures = Vec::new();
    for (width, style, text, want) in rows {
        let body =
            format!("<div style='width:{width}'><div id=t style='{style}'>{text}</div></div>");
        let got = border_boxes(&body).get("t").copied();
        let matches = got.is_some_and(|got| {
            got.iter()
                .zip(want.iter())
                .all(|(got, want)| (got - want).abs() < 0.02)
        });
        if !matches {
            failures.push(format!(
                "{body}
  expected {want:?}, got {got:?}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{}",
        failures.join(
            "
"
        )
    );
}
