//! A floated `inline-flex` or `inline-grid` box is blockified to a flex or
//! grid container (CSS Display 3 §2.7: blockification changes the outer
//! display type and keeps the inner one), so its children are flex or grid
//! items, as they are in a floated `flex` or `grid` box, and not boxes on a
//! line. An absolutely positioned or in-flow one is laid out the same way.
//!
//! The expected border boxes `[x, y, width, height]` of the elements with an
//! `id` are the ones Chromium computes for the same markup in an 800px by
//! 600px viewport with no body margin.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The border boxes `[x, y, width, height]` of the elements with an `id` on
/// the first page of an 800x600 document that holds `body`.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
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
    let mut boxes: Vec<(String, [f32; 4])> = Vec::new();
    for fragment in page.fragments() {
        if let Some(id) = dom.attr(fragment.node(), "id")
            && !boxes.iter().any(|(seen, _)| seen == id)
        {
            let rect = fragment.rect();
            boxes.push((id.to_owned(), [rect.x, rect.y, rect.width, rect.height]));
        }
    }
    boxes
}

/// Markup and the border boxes Chromium gives the elements with an `id` in it.
type Row<'a> = (&'a str, &'a [(&'a str, [f32; 4])]);

fn assert_rows(rows: &[Row<'_>]) {
    let failures: Vec<_> = rows
        .iter()
        .filter_map(|(body, want)| {
            let got = boxes(body);
            let matches = want.iter().all(|(id, want)| {
                got.iter().any(|(got_id, got)| {
                    got_id == id
                        && got
                            .iter()
                            .zip(want)
                            .all(|(got, want)| (got - want).abs() <= 0.5)
                })
            });
            (!matches).then(|| format!("{body}\n  expected {want:?}, got {got:?}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A floated `inline-flex` or `inline-grid` box lays out its inline-block
/// child as a flex or grid item, as an absolutely positioned or in-flow one
/// does and as a floated `flex` or `grid` box does.
#[test]
fn a_floated_inline_flex_or_grid_box_is_a_flex_or_grid_container() {
    assert_rows(&[
        (
            "<div><div id=t style='float:left;display:inline-flex;border:2px solid'><span id=s style='display:inline-block;width:3px;height:1px'></span></div></div>",
            &[("t", [0.0, 0.0, 7.0, 5.0]), ("s", [2.0, 2.0, 3.0, 1.0])],
        ),
        (
            "<div style='position:relative'><div id=t style='position:absolute;display:inline-flex;border:2px solid'><span id=s style='display:inline-block;width:3px;height:1px'></span></div></div>",
            &[("t", [0.0, 0.0, 7.0, 5.0]), ("s", [2.0, 2.0, 3.0, 1.0])],
        ),
        (
            "<div><div id=t style='display:inline-flex;border:2px solid'><span id=s style='display:inline-block;width:3px;height:1px'></span></div></div>",
            &[("t", [0.0, 5.0, 7.0, 5.0]), ("s", [2.0, 7.0, 3.0, 1.0])],
        ),
        (
            "<div><div id=t style='float:left;display:inline-grid;border:2px solid'><span id=s style='display:inline-block;width:3px;height:1px'></span></div></div>",
            &[("t", [0.0, 0.0, 7.0, 5.0]), ("s", [2.0, 2.0, 3.0, 1.0])],
        ),
        (
            "<div style='position:relative'><div id=t style='position:absolute;display:inline-grid;border:2px solid'><span id=s style='display:inline-block;width:3px;height:1px'></span></div></div>",
            &[("t", [0.0, 0.0, 7.0, 5.0]), ("s", [2.0, 2.0, 3.0, 1.0])],
        ),
        (
            "<div><div id=t style='display:inline-grid;border:2px solid'><span id=s style='display:inline-block;width:3px;height:1px'></span></div></div>",
            &[("t", [0.0, 5.0, 7.0, 5.0]), ("s", [2.0, 7.0, 3.0, 1.0])],
        ),
        (
            "<div><div id=t style='float:left;display:flex;border:2px solid'><span id=s style='display:inline-block;width:3px;height:1px'></span></div></div>",
            &[("t", [0.0, 0.0, 7.0, 5.0]), ("s", [2.0, 2.0, 3.0, 1.0])],
        ),
        (
            "<div style='position:relative'><div id=t style='position:absolute;display:flex;border:2px solid'><span id=s style='display:inline-block;width:3px;height:1px'></span></div></div>",
            &[("t", [0.0, 0.0, 7.0, 5.0]), ("s", [2.0, 2.0, 3.0, 1.0])],
        ),
        (
            "<div><div id=t style='display:flex;border:2px solid'><span id=s style='display:inline-block;width:3px;height:1px'></span></div></div>",
            &[("t", [0.0, 0.0, 800.0, 5.0]), ("s", [2.0, 2.0, 3.0, 1.0])],
        ),
        (
            "<div><div id=t style='float:left;display:grid;border:2px solid'><span id=s style='display:inline-block;width:3px;height:1px'></span></div></div>",
            &[("t", [0.0, 0.0, 7.0, 5.0]), ("s", [2.0, 2.0, 3.0, 1.0])],
        ),
        (
            "<div style='position:relative'><div id=t style='position:absolute;display:grid;border:2px solid'><span id=s style='display:inline-block;width:3px;height:1px'></span></div></div>",
            &[("t", [0.0, 0.0, 7.0, 5.0]), ("s", [2.0, 2.0, 3.0, 1.0])],
        ),
        (
            "<div><div id=t style='display:grid;border:2px solid'><span id=s style='display:inline-block;width:3px;height:1px'></span></div></div>",
            &[("t", [0.0, 0.0, 800.0, 5.0]), ("s", [2.0, 2.0, 3.0, 1.0])],
        ),
    ]);
}

/// Floated to either side, the blockified container keeps its flex direction,
/// wrapping, alignment and grid tracks for inline-block, text and block
/// children, and the text beside it wraps around it.
#[test]
fn a_floated_inline_flex_or_grid_box_keeps_its_layout_beside_text() {
    assert_rows(&[
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-flex;border:2px solid'><span id=a style='display:inline-block;width:3px;height:1px'></span><span id=b style='display:inline-block;width:4px;height:6px'></span></div>Z</div>",
            &[
                ("t", [0.0, 0.0, 11.0, 10.0]),
                ("a", [2.0, 2.0, 3.0, 1.0]),
                ("b", [5.0, 2.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-flex;border:2px solid'>X<span id=a>YY</span></div>Z</div>",
            &[
                ("t", [0.0, 0.0, 34.0, 14.0]),
                ("a", [12.0, 2.0, 20.0, 10.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-flex;border:2px solid'><div id=a style='width:5px;height:3px'></div><div id=b style='width:4px;height:6px'></div></div>Z</div>",
            &[
                ("t", [0.0, 0.0, 13.0, 10.0]),
                ("a", [2.0, 2.0, 5.0, 3.0]),
                ("b", [7.0, 2.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:right;display:inline-flex;border:2px solid'><span id=a style='display:inline-block;width:3px;height:1px'></span><span id=b style='display:inline-block;width:4px;height:6px'></span></div>Z</div>",
            &[
                ("t", [289.0, 0.0, 11.0, 10.0]),
                ("a", [291.0, 2.0, 3.0, 1.0]),
                ("b", [294.0, 2.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:right;display:inline-flex;border:2px solid'>X<span id=a>YY</span></div>Z</div>",
            &[
                ("t", [266.0, 0.0, 34.0, 14.0]),
                ("a", [278.0, 2.0, 20.0, 10.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:right;display:inline-flex;border:2px solid'><div id=a style='width:5px;height:3px'></div><div id=b style='width:4px;height:6px'></div></div>Z</div>",
            &[
                ("t", [287.0, 0.0, 13.0, 10.0]),
                ("a", [289.0, 2.0, 5.0, 3.0]),
                ("b", [294.0, 2.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-flex;flex-direction:column;border:2px solid'><span id=a style='display:inline-block;width:3px;height:1px'></span><span id=b style='display:inline-block;width:4px;height:6px'></span></div>Z</div>",
            &[
                ("t", [0.0, 0.0, 8.0, 11.0]),
                ("a", [2.0, 2.0, 3.0, 1.0]),
                ("b", [2.0, 3.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-flex;flex-direction:column;border:2px solid'>X<span id=a>YY</span></div>Z</div>",
            &[
                ("t", [0.0, 0.0, 24.0, 24.0]),
                ("a", [2.0, 12.0, 20.0, 10.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-flex;flex-direction:column;border:2px solid'><div id=a style='width:5px;height:3px'></div><div id=b style='width:4px;height:6px'></div></div>Z</div>",
            &[
                ("t", [0.0, 0.0, 9.0, 13.0]),
                ("a", [2.0, 2.0, 5.0, 3.0]),
                ("b", [2.0, 5.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:right;display:inline-flex;flex-direction:column;border:2px solid'><span id=a style='display:inline-block;width:3px;height:1px'></span><span id=b style='display:inline-block;width:4px;height:6px'></span></div>Z</div>",
            &[
                ("t", [292.0, 0.0, 8.0, 11.0]),
                ("a", [294.0, 2.0, 3.0, 1.0]),
                ("b", [294.0, 3.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:right;display:inline-flex;flex-direction:column;border:2px solid'>X<span id=a>YY</span></div>Z</div>",
            &[
                ("t", [276.0, 0.0, 24.0, 24.0]),
                ("a", [278.0, 12.0, 20.0, 10.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:right;display:inline-flex;flex-direction:column;border:2px solid'><div id=a style='width:5px;height:3px'></div><div id=b style='width:4px;height:6px'></div></div>Z</div>",
            &[
                ("t", [291.0, 0.0, 9.0, 13.0]),
                ("a", [293.0, 2.0, 5.0, 3.0]),
                ("b", [293.0, 5.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-flex;flex-wrap:wrap;width:6px;border:2px solid'><span id=a style='display:inline-block;width:3px;height:1px'></span><span id=b style='display:inline-block;width:4px;height:6px'></span></div>Z</div>",
            &[
                ("t", [0.0, 0.0, 10.0, 11.0]),
                ("a", [2.0, 2.0, 3.0, 1.0]),
                ("b", [2.0, 3.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-flex;flex-wrap:wrap;width:6px;border:2px solid'>X<span id=a>YY</span></div>Z</div>",
            &[
                ("t", [0.0, 0.0, 10.0, 24.0]),
                ("a", [2.0, 12.0, 20.0, 10.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-flex;flex-wrap:wrap;width:6px;border:2px solid'><div id=a style='width:5px;height:3px'></div><div id=b style='width:4px;height:6px'></div></div>Z</div>",
            &[
                ("t", [0.0, 0.0, 10.0, 13.0]),
                ("a", [2.0, 2.0, 5.0, 3.0]),
                ("b", [2.0, 5.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:right;display:inline-flex;flex-wrap:wrap;width:6px;border:2px solid'><span id=a style='display:inline-block;width:3px;height:1px'></span><span id=b style='display:inline-block;width:4px;height:6px'></span></div>Z</div>",
            &[
                ("t", [290.0, 0.0, 10.0, 11.0]),
                ("a", [292.0, 2.0, 3.0, 1.0]),
                ("b", [292.0, 3.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:right;display:inline-flex;flex-wrap:wrap;width:6px;border:2px solid'>X<span id=a>YY</span></div>Z</div>",
            &[
                ("t", [290.0, 0.0, 10.0, 24.0]),
                ("a", [292.0, 12.0, 20.0, 10.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:right;display:inline-flex;flex-wrap:wrap;width:6px;border:2px solid'><div id=a style='width:5px;height:3px'></div><div id=b style='width:4px;height:6px'></div></div>Z</div>",
            &[
                ("t", [290.0, 0.0, 10.0, 13.0]),
                ("a", [292.0, 2.0, 5.0, 3.0]),
                ("b", [292.0, 5.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-flex;align-items:center;border:2px solid'><span id=a style='display:inline-block;width:3px;height:1px'></span><span id=b style='display:inline-block;width:4px;height:6px'></span></div>Z</div>",
            &[
                ("t", [0.0, 0.0, 11.0, 10.0]),
                ("a", [2.0, 4.5, 3.0, 1.0]),
                ("b", [5.0, 2.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-flex;align-items:center;border:2px solid'>X<span id=a>YY</span></div>Z</div>",
            &[
                ("t", [0.0, 0.0, 34.0, 14.0]),
                ("a", [12.0, 2.0, 20.0, 10.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-flex;align-items:center;border:2px solid'><div id=a style='width:5px;height:3px'></div><div id=b style='width:4px;height:6px'></div></div>Z</div>",
            &[
                ("t", [0.0, 0.0, 13.0, 10.0]),
                ("a", [2.0, 3.5, 5.0, 3.0]),
                ("b", [7.0, 2.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:right;display:inline-flex;align-items:center;border:2px solid'><span id=a style='display:inline-block;width:3px;height:1px'></span><span id=b style='display:inline-block;width:4px;height:6px'></span></div>Z</div>",
            &[
                ("t", [289.0, 0.0, 11.0, 10.0]),
                ("a", [291.0, 4.5, 3.0, 1.0]),
                ("b", [294.0, 2.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:right;display:inline-flex;align-items:center;border:2px solid'>X<span id=a>YY</span></div>Z</div>",
            &[
                ("t", [266.0, 0.0, 34.0, 14.0]),
                ("a", [278.0, 2.0, 20.0, 10.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:right;display:inline-flex;align-items:center;border:2px solid'><div id=a style='width:5px;height:3px'></div><div id=b style='width:4px;height:6px'></div></div>Z</div>",
            &[
                ("t", [287.0, 0.0, 13.0, 10.0]),
                ("a", [289.0, 3.5, 5.0, 3.0]),
                ("b", [294.0, 2.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-grid;border:2px solid'><span id=a style='display:inline-block;width:3px;height:1px'></span><span id=b style='display:inline-block;width:4px;height:6px'></span></div>Z</div>",
            &[
                ("t", [0.0, 0.0, 8.0, 11.0]),
                ("a", [2.0, 2.0, 3.0, 1.0]),
                ("b", [2.0, 3.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-grid;border:2px solid'>X<span id=a>YY</span></div>Z</div>",
            &[
                ("t", [0.0, 0.0, 24.0, 24.0]),
                ("a", [2.0, 12.0, 20.0, 10.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-grid;border:2px solid'><div id=a style='width:5px;height:3px'></div><div id=b style='width:4px;height:6px'></div></div>Z</div>",
            &[
                ("t", [0.0, 0.0, 9.0, 13.0]),
                ("a", [2.0, 2.0, 5.0, 3.0]),
                ("b", [2.0, 5.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:right;display:inline-grid;border:2px solid'><span id=a style='display:inline-block;width:3px;height:1px'></span><span id=b style='display:inline-block;width:4px;height:6px'></span></div>Z</div>",
            &[
                ("t", [292.0, 0.0, 8.0, 11.0]),
                ("a", [294.0, 2.0, 3.0, 1.0]),
                ("b", [294.0, 3.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:right;display:inline-grid;border:2px solid'>X<span id=a>YY</span></div>Z</div>",
            &[
                ("t", [276.0, 0.0, 24.0, 24.0]),
                ("a", [278.0, 12.0, 20.0, 10.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:right;display:inline-grid;border:2px solid'><div id=a style='width:5px;height:3px'></div><div id=b style='width:4px;height:6px'></div></div>Z</div>",
            &[
                ("t", [291.0, 0.0, 9.0, 13.0]),
                ("a", [293.0, 2.0, 5.0, 3.0]),
                ("b", [293.0, 5.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-grid;grid-template-columns:4px 4px;border:2px solid'><span id=a style='display:inline-block;width:3px;height:1px'></span><span id=b style='display:inline-block;width:4px;height:6px'></span></div>Z</div>",
            &[
                ("t", [0.0, 0.0, 12.0, 10.0]),
                ("a", [2.0, 2.0, 3.0, 1.0]),
                ("b", [6.0, 2.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-grid;grid-template-columns:4px 4px;border:2px solid'>X<span id=a>YY</span></div>Z</div>",
            &[("t", [0.0, 0.0, 12.0, 14.0]), ("a", [6.0, 2.0, 4.0, 10.0])],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-grid;grid-template-columns:4px 4px;border:2px solid'><div id=a style='width:5px;height:3px'></div><div id=b style='width:4px;height:6px'></div></div>Z</div>",
            &[
                ("t", [0.0, 0.0, 12.0, 10.0]),
                ("a", [2.0, 2.0, 5.0, 3.0]),
                ("b", [6.0, 2.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:right;display:inline-grid;grid-template-columns:4px 4px;border:2px solid'><span id=a style='display:inline-block;width:3px;height:1px'></span><span id=b style='display:inline-block;width:4px;height:6px'></span></div>Z</div>",
            &[
                ("t", [288.0, 0.0, 12.0, 10.0]),
                ("a", [290.0, 2.0, 3.0, 1.0]),
                ("b", [294.0, 2.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:right;display:inline-grid;grid-template-columns:4px 4px;border:2px solid'>X<span id=a>YY</span></div>Z</div>",
            &[
                ("t", [288.0, 0.0, 12.0, 14.0]),
                ("a", [294.0, 2.0, 4.0, 10.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:right;display:inline-grid;grid-template-columns:4px 4px;border:2px solid'><div id=a style='width:5px;height:3px'></div><div id=b style='width:4px;height:6px'></div></div>Z</div>",
            &[
                ("t", [288.0, 0.0, 12.0, 10.0]),
                ("a", [290.0, 2.0, 5.0, 3.0]),
                ("b", [294.0, 2.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-grid;grid-auto-flow:column;gap:1px;border:2px solid'><span id=a style='display:inline-block;width:3px;height:1px'></span><span id=b style='display:inline-block;width:4px;height:6px'></span></div>Z</div>",
            &[
                ("t", [0.0, 0.0, 12.0, 10.0]),
                ("a", [2.0, 2.0, 3.0, 1.0]),
                ("b", [6.0, 2.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-grid;grid-auto-flow:column;gap:1px;border:2px solid'>X<span id=a>YY</span></div>Z</div>",
            &[
                ("t", [0.0, 0.0, 35.0, 14.0]),
                ("a", [13.0, 2.0, 20.0, 10.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:left;display:inline-grid;grid-auto-flow:column;gap:1px;border:2px solid'><div id=a style='width:5px;height:3px'></div><div id=b style='width:4px;height:6px'></div></div>Z</div>",
            &[
                ("t", [0.0, 0.0, 14.0, 10.0]),
                ("a", [2.0, 2.0, 5.0, 3.0]),
                ("b", [8.0, 2.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:right;display:inline-grid;grid-auto-flow:column;gap:1px;border:2px solid'><span id=a style='display:inline-block;width:3px;height:1px'></span><span id=b style='display:inline-block;width:4px;height:6px'></span></div>Z</div>",
            &[
                ("t", [288.0, 0.0, 12.0, 10.0]),
                ("a", [290.0, 2.0, 3.0, 1.0]),
                ("b", [294.0, 2.0, 4.0, 6.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:right;display:inline-grid;grid-auto-flow:column;gap:1px;border:2px solid'>X<span id=a>YY</span></div>Z</div>",
            &[
                ("t", [265.0, 0.0, 35.0, 14.0]),
                ("a", [278.0, 2.0, 20.0, 10.0]),
            ],
        ),
        (
            "<div style='width:300px'><div id=t style='float:right;display:inline-grid;grid-auto-flow:column;gap:1px;border:2px solid'><div id=a style='width:5px;height:3px'></div><div id=b style='width:4px;height:6px'></div></div>Z</div>",
            &[
                ("t", [286.0, 0.0, 14.0, 10.0]),
                ("a", [288.0, 2.0, 5.0, 3.0]),
                ("b", [294.0, 2.0, 4.0, 6.0]),
            ],
        ),
    ]);
}
