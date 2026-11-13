//! A content box with an `auto` width and borders on its left or right side
//! only is sized as its content box says when an aspect ratio or a
//! `min-width` or `max-width` takes part: the ratio gives the width of the
//! content box, and the bounds hold the content box, with the borders added
//! outside. Where each box falls is what Chromium lays out for the same
//! documents.
use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// A document body and the id and border box of each box with an id in it.
type Row = (&'static str, &'static [(&'static str, [i32; 4])]);

/// The id and border box of every box with an id in `body`, laid out on one
/// page 1280px wide.
fn boxes(body: &str) -> Vec<(String, [i32; 4])> {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:1280px 1000px;margin:0}} body{{margin:0}}</style><body>{body}</body>"
    );
    let document = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("layout must complete")
    };
    let page = result.page(0).unwrap();
    page.fragments()
        .filter_map(|fragment| {
            let id = page.dom().attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((
                id.to_owned(),
                [rect.x, rect.y, rect.width, rect.height].map(|value| value.round() as i32),
            ))
        })
        .collect()
}

fn assert_rows(rows: &[Row]) {
    for &(body, expected) in rows {
        let expected = expected
            .iter()
            .map(|&(id, rect)| (id.to_owned(), rect))
            .collect::<Vec<_>>();
        assert_eq!(boxes(body), expected, "{body}");
    }
}

#[test]
fn a_ratio_box_with_side_borders_takes_its_width_from_its_content_box() {
    // Boxes with a ratio of 1 or 2 and a `height`, a `min-height` or a
    // `width`, with a left or right border or both, with padding beside the
    // border, under a `max-width`, or with a top or bottom border only; in
    // flow with a stretched width, out of flow, floated, inline-block, and as
    // flex and grid items.
    assert_rows(&[
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='position:absolute;aspect-ratio:1;height:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 120, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='position:absolute;aspect-ratio:1;min-height:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 120, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='position:absolute;aspect-ratio:1;height:100px;border-right:20px solid'></div></div>",
            &[("c", [0, 0, 120, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='position:absolute;aspect-ratio:1;height:100px;border-left:10px solid;border-right:5px solid'></div></div>",
            &[("c", [0, 0, 115, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='position:absolute;aspect-ratio:1;max-width:50px;min-height:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 70, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='position:absolute;aspect-ratio:1;width:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 120, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='position:absolute;aspect-ratio:1;height:100px;border-left:20px solid;padding-right:5px'></div></div>",
            &[("c", [0, 0, 125, 100])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='position:absolute;aspect-ratio:2;height:50px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 120, 50])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='position:absolute;aspect-ratio:1;width:100px;border-top:20px solid'></div></div>",
            &[("c", [0, 0, 100, 120])],
        ),
        (
            "<div style='width:500px;height:300px;position:relative'><div id=c style='position:absolute;aspect-ratio:1;min-width:100px;border-bottom:20px solid'></div></div>",
            &[("c", [0, 0, 100, 120])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='float:left;aspect-ratio:1;height:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 120, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='float:left;aspect-ratio:1;min-height:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 120, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='float:left;aspect-ratio:1;height:100px;border-right:20px solid'></div></div>",
            &[("c", [0, 0, 120, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='float:left;aspect-ratio:1;height:100px;border-left:10px solid;border-right:5px solid'></div></div>",
            &[("c", [0, 0, 115, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='float:left;aspect-ratio:1;max-width:50px;min-height:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 70, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='float:left;aspect-ratio:1;width:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 120, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='float:left;aspect-ratio:1;height:100px;border-left:20px solid;padding-right:5px'></div></div>",
            &[("c", [0, 0, 125, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='float:left;aspect-ratio:2;height:50px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 120, 50])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='float:left;aspect-ratio:1;width:100px;border-top:20px solid'></div></div>",
            &[("c", [0, 0, 100, 120])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='float:left;aspect-ratio:1;min-width:100px;border-bottom:20px solid'></div></div>",
            &[("c", [0, 0, 100, 120])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;height:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 120, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;min-height:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 120, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;height:100px;border-right:20px solid'></div></div>",
            &[("c", [0, 0, 120, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;height:100px;border-left:10px solid;border-right:5px solid'></div></div>",
            &[("c", [0, 0, 115, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;max-width:50px;min-height:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 70, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;width:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 120, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;height:100px;border-left:20px solid;padding-right:5px'></div></div>",
            &[("c", [0, 0, 125, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:2;height:50px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 120, 50])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;width:100px;border-top:20px solid'></div></div>",
            &[("c", [0, 0, 100, 120])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;min-width:100px;border-bottom:20px solid'></div></div>",
            &[("c", [0, 0, 100, 120])],
        ),
        (
            "<div style='width:500px;height:300px;display:flex;align-items:flex-start'><div id=c style='aspect-ratio:1;height:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 120, 100])],
        ),
        (
            "<div style='width:500px;height:300px;display:flex;align-items:flex-start'><div id=c style='aspect-ratio:1;min-height:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 120, 100])],
        ),
        (
            "<div style='width:500px;height:300px;display:flex;align-items:flex-start'><div id=c style='aspect-ratio:1;height:100px;border-right:20px solid'></div></div>",
            &[("c", [0, 0, 120, 100])],
        ),
        (
            "<div style='width:500px;height:300px;display:flex;align-items:flex-start'><div id=c style='aspect-ratio:1;height:100px;border-left:10px solid;border-right:5px solid'></div></div>",
            &[("c", [0, 0, 115, 100])],
        ),
        (
            "<div style='width:500px;height:300px;display:flex;align-items:flex-start'><div id=c style='aspect-ratio:1;max-width:50px;min-height:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 70, 100])],
        ),
        (
            "<div style='width:500px;height:300px;display:flex;align-items:flex-start'><div id=c style='aspect-ratio:1;width:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 120, 100])],
        ),
        (
            "<div style='width:500px;height:300px;display:flex;align-items:flex-start'><div id=c style='aspect-ratio:1;height:100px;border-left:20px solid;padding-right:5px'></div></div>",
            &[("c", [0, 0, 125, 100])],
        ),
        (
            "<div style='width:500px;height:300px;display:flex;align-items:flex-start'><div id=c style='aspect-ratio:2;height:50px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 120, 50])],
        ),
        (
            "<div style='width:500px;height:300px;display:flex;align-items:flex-start'><div id=c style='aspect-ratio:1;width:100px;border-top:20px solid'></div></div>",
            &[("c", [0, 0, 100, 120])],
        ),
        (
            "<div style='width:500px;height:300px;display:flex;align-items:flex-start'><div id=c style='aspect-ratio:1;min-width:100px;border-bottom:20px solid'></div></div>",
            &[("c", [0, 0, 100, 120])],
        ),
        (
            "<div style='width:500px;height:300px;display:grid;justify-items:start;align-items:start'><div id=c style='aspect-ratio:1;height:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 120, 100])],
        ),
        (
            "<div style='width:500px;height:300px;display:grid;justify-items:start;align-items:start'><div id=c style='aspect-ratio:1;min-height:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 120, 100])],
        ),
        (
            "<div style='width:500px;height:300px;display:grid;justify-items:start;align-items:start'><div id=c style='aspect-ratio:1;height:100px;border-right:20px solid'></div></div>",
            &[("c", [0, 0, 120, 100])],
        ),
        (
            "<div style='width:500px;height:300px;display:grid;justify-items:start;align-items:start'><div id=c style='aspect-ratio:1;height:100px;border-left:10px solid;border-right:5px solid'></div></div>",
            &[("c", [0, 0, 115, 100])],
        ),
        (
            "<div style='width:500px;height:300px;display:grid;justify-items:start;align-items:start'><div id=c style='aspect-ratio:1;max-width:50px;min-height:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 70, 100])],
        ),
        (
            "<div style='width:500px;height:300px;display:grid;justify-items:start;align-items:start'><div id=c style='aspect-ratio:1;width:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 120, 100])],
        ),
        (
            "<div style='width:500px;height:300px;display:grid;justify-items:start;align-items:start'><div id=c style='aspect-ratio:1;height:100px;border-left:20px solid;padding-right:5px'></div></div>",
            &[("c", [0, 0, 125, 100])],
        ),
        (
            "<div style='width:500px;height:300px;display:grid;justify-items:start;align-items:start'><div id=c style='aspect-ratio:2;height:50px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 120, 50])],
        ),
        (
            "<div style='width:500px;height:300px;display:grid;justify-items:start;align-items:start'><div id=c style='aspect-ratio:1;width:100px;border-top:20px solid'></div></div>",
            &[("c", [0, 0, 100, 120])],
        ),
        (
            "<div style='width:500px;height:300px;display:grid;justify-items:start;align-items:start'><div id=c style='aspect-ratio:1;min-width:100px;border-bottom:20px solid'></div></div>",
            &[("c", [0, 0, 100, 120])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='aspect-ratio:1;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 500, 480])],
        ),
    ]);
}

#[test]
fn the_width_bounds_of_a_box_with_side_borders_hold_its_content_box() {
    // Boxes with a left or right border and a `min-width`, a `max-width` or
    // a length `flex-basis`:
    // floated, in flow, out of flow and as flex items, and multicol
    // containers with those bounds or a ratio.
    assert_rows(&[
        (
            "<div style='width:500px;height:300px'><div id=c style='float:left;min-width:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 120, 0])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='max-width:50px;border-left:20px solid;height:10px'></div></div>",
            &[("c", [0, 0, 70, 10])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='position:absolute;min-width:100px;border-right:20px solid;height:10px'></div></div>",
            &[("c", [0, 0, 120, 10])],
        ),
        (
            "<div style='width:500px;height:300px'><div style='display:flex;align-items:flex-start'><div id=c style='min-width:100px;border-left:20px solid;height:10px'></div></div></div>",
            &[("c", [0, 0, 120, 10])],
        ),
        (
            "<div style='width:500px;height:300px'><div style='display:flex;align-items:flex-start'><div id=c style='max-width:50px;border-left:20px solid;height:10px;flex-grow:1'></div></div></div>",
            &[("c", [0, 0, 70, 10])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='columns:2;column-gap:0;max-width:100px;border-left:20px solid;height:10px'></div></div>",
            &[("c", [0, 0, 120, 10])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='columns:2;column-gap:0;float:left;min-width:100px;border-right:20px solid;height:10px'></div></div>",
            &[("c", [0, 0, 120, 10])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='columns:2;column-gap:0;aspect-ratio:1;height:50px;border-left:20px solid;float:left'></div></div>",
            &[("c", [0, 0, 70, 50])],
        ),
        (
            "<div style='width:500px;height:300px'><div style='display:flex;align-items:flex-start'><div id=c style='flex-basis:100px;border-left:20px solid;height:10px;flex-shrink:0'></div></div></div>",
            &[("c", [0, 0, 120, 10])],
        ),
    ]);
}

#[test]
fn a_ratio_box_with_side_borders_keeps_a_stated_height_and_width_bounds() {
    // Boxes with a ratio whose `height` holds against a `min-width`, or whose
    // `min-width` raises the height through the ratio up to a `max-height`,
    // also where the ratio relates the content box of a border box, and
    // replaced canvases with a side border, sized from their content box.
    assert_rows(&[
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;height:100px;min-width:300px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 320, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;height:100px;min-width:300px'></div></div>",
            &[("c", [0, 0, 300, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;min-width:150px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 170, 150])],
        ),
        (
            "<div style='width:500px;height:300px'><canvas id=c style='display:block;border-left:20px solid'></canvas></div>",
            &[("c", [0, 0, 320, 150])],
        ),
        (
            "<div style='width:500px;height:300px'><canvas id=c style='float:left;border-left:20px solid;height:75px'></canvas></div>",
            &[("c", [0, 0, 170, 75])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;min-width:300px;max-height:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 320, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;aspect-ratio:1;min-width:300px;max-height:100px'></div></div>",
            &[("c", [0, 0, 300, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='float:left;aspect-ratio:1;height:100px;min-width:300px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 320, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='position:absolute;aspect-ratio:1;height:100px;min-width:300px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 320, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div style='display:flex;align-items:flex-start'><div id=c style='aspect-ratio:1;height:100px;min-width:300px;border-left:20px solid'></div></div></div>",
            &[("c", [0, 0, 320, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;box-sizing:border-box;aspect-ratio:auto 1;min-width:300px;max-height:100px;border-left:20px solid'></div></div>",
            &[("c", [0, 0, 300, 100])],
        ),
        (
            "<div style='width:500px;height:300px'><div id=c style='display:inline-block;vertical-align:top;box-sizing:border-box;aspect-ratio:auto 1;min-width:300px;max-height:100px;border-left:20px solid;padding-top:10px'></div></div>",
            &[("c", [0, 0, 300, 100])],
        ),
    ]);
}
