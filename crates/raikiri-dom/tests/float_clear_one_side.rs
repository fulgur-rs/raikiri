//! A box that clears the floats on one side goes below the bottom of those
//! floats and no further (CSS 2 §9.5.2): it may sit beside an earlier float on
//! the other side, even one that is itself below a cleared float. The expected
//! boxes are the ones Chromium lays out for the same markup on an 800x600
//! page.

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

/// The border box `(x, y, width, height)` of every element with an `id` on the
/// first page of an 800x600 document that holds `body`.
fn boxes(body: &str) -> HashMap<String, [f32; 4]> {
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
    let mut found = HashMap::new();
    for fragment in page.fragments() {
        if let Some(id) = dom.attr(fragment.node(), "id") {
            let rect = fragment.rect();
            found.insert(id.to_string(), [rect.x, rect.y, rect.width, rect.height]);
        }
    }
    found
}

/// One case: the markup of the body and the expected border box of each
/// element by its `id`.
type Case<'a> = (&'a str, &'a [(&'a str, [f32; 4])]);

fn assert_cases(cases: &[Case<'_>]) {
    let mut failures = Vec::new();
    for (body, expected) in cases {
        let found = boxes(body);
        for (id, want) in *expected {
            let got = found.get(*id).copied();
            let matches = got.is_some_and(|got| {
                got.iter()
                    .zip(want)
                    .all(|(got, want)| (got - want).abs() <= 0.02)
            });
            if !matches {
                failures.push(format!("{body}: {id} expected {want:?}, got {got:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_float_that_clears_one_side_sits_beside_a_float_on_the_other_side() {
    assert_cases(&[
        (
            "<div style='width:60px'><div id=x style='float:left;width:20px'>X</div><div id=y style='float:right;width:20px;clear:left'>Y</div><div id=w style='float:left;width:20px;clear:left'>W</div></div>",
            &[
                ("w", [0.0, 10.0, 20.0, 10.0]),
                ("x", [0.0, 0.0, 20.0, 10.0]),
                ("y", [40.0, 10.0, 20.0, 10.0]),
            ],
        ),
        (
            "<div style='width:60px'><div id=x style='float:left;width:20px;height:30px'></div><div id=y style='float:right;width:20px;clear:left;height:30px'></div><div id=w style='float:left;width:20px;clear:left;height:30px'></div></div>",
            &[
                ("w", [0.0, 30.0, 20.0, 30.0]),
                ("x", [0.0, 0.0, 20.0, 30.0]),
                ("y", [40.0, 30.0, 20.0, 30.0]),
            ],
        ),
        (
            "<div style='width:60px'><div id=x style='float:right;width:20px;height:30px'></div><div id=y style='float:left;width:20px;clear:right;height:30px'></div><div id=w style='float:right;width:20px;clear:right;height:30px'></div></div>",
            &[
                ("w", [40.0, 30.0, 20.0, 30.0]),
                ("x", [40.0, 0.0, 20.0, 30.0]),
                ("y", [0.0, 30.0, 20.0, 30.0]),
            ],
        ),
        (
            "<div style='width:60px'><div id=x style='float:left;width:20px;height:30px'></div><div id=y style='float:right;width:20px;height:50px'></div><div id=v style='float:left;width:20px;clear:left;height:10px'></div><div id=w style='float:left;width:20px;clear:left;height:10px'></div></div>",
            &[
                ("v", [0.0, 30.0, 20.0, 10.0]),
                ("w", [0.0, 40.0, 20.0, 10.0]),
                ("x", [0.0, 0.0, 20.0, 30.0]),
                ("y", [40.0, 0.0, 20.0, 50.0]),
            ],
        ),
        (
            "<div style='width:60px'><div id=x style='float:left;width:20px;height:30px'></div><div id=y style='float:right;width:20px;clear:both;height:30px'></div><div id=w style='float:left;width:20px;clear:left;height:30px'></div></div>",
            &[
                ("w", [0.0, 30.0, 20.0, 30.0]),
                ("x", [0.0, 0.0, 20.0, 30.0]),
                ("y", [40.0, 30.0, 20.0, 30.0]),
            ],
        ),
        (
            "<div style='width:60px'>A<div id=x style='float:left;width:20px;height:30px'></div><div id=y style='float:right;width:20px;clear:left;height:30px'></div><div id=w style='float:left;width:20px;clear:left;height:30px'></div>B</div>",
            &[
                ("w", [0.0, 30.0, 20.0, 30.0]),
                ("x", [0.0, 0.0, 20.0, 30.0]),
                ("y", [40.0, 30.0, 20.0, 30.0]),
            ],
        ),
        (
            "<div style='float:left'><div id=x style='float:left;width:20px;height:30px'></div><div id=y style='float:right;width:20px;clear:left;height:30px'></div><div id=w style='float:left;width:20px;clear:left;height:30px'></div></div>",
            &[
                ("w", [0.0, 30.0, 20.0, 30.0]),
                ("x", [0.0, 0.0, 20.0, 30.0]),
                ("y", [20.0, 30.0, 20.0, 30.0]),
            ],
        ),
        (
            "<div style='width:60px'><div id=x style='float:right;width:20px;height:30px'></div><div id=w style='float:left;width:20px;clear:left;height:30px'></div></div>",
            &[
                ("w", [0.0, 0.0, 20.0, 30.0]),
                ("x", [40.0, 0.0, 20.0, 30.0]),
            ],
        ),
        (
            "<div style='width:60px'><div id=x style='float:left;width:20px;height:0'></div><div id=y style='float:right;width:20px;height:30px'></div><div id=w style='float:left;width:20px;clear:left;height:30px'></div></div>",
            &[
                ("w", [0.0, 0.0, 20.0, 30.0]),
                ("x", [0.0, 0.0, 20.0, 0.0]),
                ("y", [40.0, 0.0, 20.0, 30.0]),
            ],
        ),
        (
            "<div style='width:60px'><div id=x style='float:left;width:20px;height:30px'></div><div id=y style='float:right;width:20px;clear:left;height:30px'></div><div id=w style='float:left;width:30px;clear:left;height:30px'></div></div>",
            &[
                ("w", [0.0, 30.0, 30.0, 30.0]),
                ("x", [0.0, 0.0, 20.0, 30.0]),
                ("y", [40.0, 30.0, 20.0, 30.0]),
            ],
        ),
        (
            "<div style='width:60px'><div id=x style='float:left;width:20px;height:30px'></div><div id=y style='float:right;width:20px;clear:left;height:30px'></div><div id=z style='float:right;width:20px;height:10px'></div><div id=w style='float:left;width:20px;clear:left;height:30px'></div></div>",
            &[
                ("w", [0.0, 30.0, 20.0, 30.0]),
                ("x", [0.0, 0.0, 20.0, 30.0]),
                ("y", [40.0, 30.0, 20.0, 30.0]),
                ("z", [20.0, 30.0, 20.0, 10.0]),
            ],
        ),
    ]);
}

#[test]
fn a_block_that_clears_one_side_sits_beside_a_float_on_the_other_side() {
    assert_cases(&[
        (
            "<div style='width:60px'><div id=x style='float:left;width:20px;height:30px'></div><div id=y style='float:right;width:20px;clear:left;height:30px'></div><div id=w style='overflow:hidden;clear:left;height:10px'></div></div>",
            &[
                ("w", [0.0, 30.0, 40.0, 10.0]),
                ("x", [0.0, 0.0, 20.0, 30.0]),
                ("y", [40.0, 30.0, 20.0, 30.0]),
            ],
        ),
        (
            "<div style='width:60px'><div id=x style='float:left;width:20px;height:30px'></div><div id=y style='float:right;width:20px;clear:left;height:30px'></div><div id=w style='display:flow-root;clear:left'>Z</div></div>",
            &[
                ("w", [0.0, 30.0, 40.0, 10.0]),
                ("x", [0.0, 0.0, 20.0, 30.0]),
                ("y", [40.0, 30.0, 20.0, 30.0]),
            ],
        ),
        (
            "<div style='width:60px'><div id=x style='float:left;width:20px;height:30px'></div><div id=y style='float:right;width:20px;clear:left;height:30px'></div><div id=w style='clear:left'>Z</div></div>",
            &[
                ("w", [0.0, 30.0, 60.0, 10.0]),
                ("x", [0.0, 0.0, 20.0, 30.0]),
                ("y", [40.0, 30.0, 20.0, 30.0]),
            ],
        ),
        (
            "<div style='width:60px'><div id=x style='float:left;width:20px;height:30px'></div><div id=y style='float:right;width:20px;clear:left;height:30px'></div><div id=w style='clear:left'><span id=s>Z</span></div></div>",
            &[
                ("s", [0.0, 30.0, 10.0, 10.0]),
                ("w", [0.0, 30.0, 60.0, 10.0]),
                ("x", [0.0, 0.0, 20.0, 30.0]),
                ("y", [40.0, 30.0, 20.0, 30.0]),
            ],
        ),
        (
            "<div style='width:60px'><div id=x style='float:left;width:20px;height:30px'></div><div id=y style='float:right;width:20px;clear:left;height:30px'></div><div id=w style='overflow:hidden;clear:left;height:10px;direction:rtl'></div></div>",
            &[
                ("w", [0.0, 30.0, 40.0, 10.0]),
                ("x", [0.0, 0.0, 20.0, 30.0]),
                ("y", [40.0, 30.0, 20.0, 30.0]),
            ],
        ),
        (
            "<div style='width:60px'><div id=x style='float:left;width:20px;height:30px'></div><div id=y style='float:right;width:20px;height:20px'></div><div id=w style='overflow:hidden;clear:right;height:10px'></div></div>",
            &[
                ("w", [20.0, 20.0, 40.0, 10.0]),
                ("x", [0.0, 0.0, 20.0, 30.0]),
                ("y", [40.0, 0.0, 20.0, 20.0]),
            ],
        ),
        (
            "<div style='width:60px'><div id=x style='float:left;width:20px;height:30px'></div><div id=y style='float:right;width:20px;clear:left;height:30px'></div><p id=p style='margin:0;clear:left'>A A A A</p></div>",
            &[
                ("p", [0.0, 30.0, 60.0, 20.0]),
                ("x", [0.0, 0.0, 20.0, 30.0]),
                ("y", [40.0, 30.0, 20.0, 30.0]),
            ],
        ),
    ]);
}
