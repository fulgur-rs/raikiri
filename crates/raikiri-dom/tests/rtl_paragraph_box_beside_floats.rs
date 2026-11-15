//! In a right-to-left paragraph, a block that establishes a formatting
//! context and sits among the paragraph's lines is placed beside the floats
//! from the right: the room it takes starts at the float on the right, and a
//! box narrower than the room keeps the right end of it, as an
//! over-constrained right-to-left block does (CSS 2.2 §10.3.3). The expected
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
fn a_box_beside_one_float_in_a_right_to_left_paragraph_keeps_the_right_end_of_the_room() {
    assert_cases(&[
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=b style='display:block;width:40px;height:10px'></div>T</div>",
            &[
                ("b", [60.0, 10.0, 40.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=b style='display:flow-root;width:40px;height:10px'></div>T</div>",
            &[
                ("b", [30.0, 10.0, 40.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=b style='display:flow-root;height:10px'></div>T</div>",
            &[
                ("b", [0.0, 10.0, 70.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=b style='overflow:hidden;width:40px;height:10px;margin-left:5px'></div>T</div>",
            &[
                ("b", [30.0, 10.0, 40.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=b style='overflow:hidden;width:40px;height:10px;margin-right:5px'></div>T</div>",
            &[
                ("b", [30.0, 10.0, 40.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=b style='overflow:hidden;width:80px;height:10px'></div>T</div>",
            &[
                ("b", [20.0, 30.0, 80.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=b style='display:flow-root;width:40px;height:10px;margin:0 auto'></div>T</div>",
            &[
                ("b", [15.0, 10.0, 40.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=b style='display:flow-root;width:40px;height:10px;margin-left:auto'></div>T</div>",
            &[
                ("b", [30.0, 10.0, 40.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=b style='display:block;width:40px;height:10px;margin-right:auto'></div>T</div>",
            &[
                ("b", [0.0, 10.0, 40.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=b style='display:table;width:40px;height:10px'></div>T</div>",
            &[
                ("b", [30.0, 10.0, 40.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='display:block;width:40px;height:10px'></div>T</div>",
            &[
                ("b", [60.0, 10.0, 40.0, 10.0]),
                ("f", [0.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='display:flow-root;width:40px;height:10px'></div>T</div>",
            &[
                ("b", [60.0, 10.0, 40.0, 10.0]),
                ("f", [0.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='display:flow-root;height:10px'></div>T</div>",
            &[
                ("b", [30.0, 10.0, 70.0, 10.0]),
                ("f", [0.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='overflow:hidden;width:40px;height:10px;margin-left:5px'></div>T</div>",
            &[
                ("b", [60.0, 10.0, 40.0, 10.0]),
                ("f", [0.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='overflow:hidden;width:40px;height:10px;margin-right:5px'></div>T</div>",
            &[
                ("b", [55.0, 10.0, 40.0, 10.0]),
                ("f", [0.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='overflow:hidden;width:80px;height:10px'></div>T</div>",
            &[
                ("b", [20.0, 30.0, 80.0, 10.0]),
                ("f", [0.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='display:flow-root;width:40px;height:10px;margin:0 auto'></div>T</div>",
            &[
                ("b", [45.0, 10.0, 40.0, 10.0]),
                ("f", [0.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='display:flow-root;width:40px;height:10px;margin-left:auto'></div>T</div>",
            &[
                ("b", [60.0, 10.0, 40.0, 10.0]),
                ("f", [0.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='display:block;width:40px;height:10px;margin-right:auto'></div>T</div>",
            &[
                ("b", [0.0, 10.0, 40.0, 10.0]),
                ("f", [0.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='display:table;width:40px;height:10px'></div>T</div>",
            &[
                ("b", [60.0, 10.0, 40.0, 10.0]),
                ("f", [0.0, 0.0, 30.0, 30.0]),
            ],
        ),
    ]);
}

#[test]
fn a_box_between_two_floats_in_a_right_to_left_paragraph_keeps_the_right_end_of_the_room() {
    assert_cases(&[
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=g style='float:left;width:30px;height:10px'></div><div id=b style='overflow:hidden;width:30px;height:10px'></div>T</div>",
            &[
                ("b", [40.0, 10.0, 30.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("g", [0.0, 0.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=g style='float:left;width:30px;height:10px'></div><div id=b style='overflow:hidden;width:50px;height:10px'></div>T</div>",
            &[
                ("b", [20.0, 10.0, 50.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("g", [0.0, 0.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'><div id=f style='float:right;width:30px;height:30px'></div><div id=g style='float:left;width:30px;height:10px'></div><div id=b style='overflow:hidden;width:50px;height:10px'></div></div>",
            &[
                ("b", [20.0, 10.0, 50.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("g", [0.0, 0.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=g style='float:left;width:30px;height:10px'></div><div id=b style='overflow:hidden;width:30px;height:10px'></div>T</div>",
            &[
                ("b", [0.0, 10.0, 30.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("g", [0.0, 0.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=g style='float:left;width:30px;height:10px'></div><div id=b style='overflow:hidden;width:50px;height:10px'></div>T</div>",
            &[
                ("b", [0.0, 10.0, 50.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("g", [0.0, 0.0, 30.0, 10.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:ltr'><div id=f style='float:right;width:30px;height:30px'></div><div id=g style='float:left;width:30px;height:10px'></div><div id=b style='overflow:hidden;width:50px;height:10px'></div></div>",
            &[
                ("b", [0.0, 10.0, 50.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
                ("g", [0.0, 0.0, 30.0, 10.0]),
            ],
        ),
    ]);
}

#[test]
fn margins_alignment_and_limits_place_a_box_beside_floats_in_a_right_to_left_paragraph() {
    assert_cases(&[
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=b style='display:flow-root;width:40px;height:10px;justify-self:center'></div>T</div>",
            &[
                ("b", [15.0, 10.0, 40.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=b style='display:flow-root;width:40px;height:10px;justify-self:start'></div>T</div>",
            &[
                ("b", [30.0, 10.0, 40.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=b style='display:flow-root;width:40px;height:10px;justify-self:end'></div>T</div>",
            &[
                ("b", [0.0, 10.0, 40.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=b style='display:flow-root;width:40px;height:10px;margin-left:-10px'></div>T</div>",
            &[
                ("b", [30.0, 10.0, 40.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=b style='display:flow-root;width:40px;height:10px;margin-right:-10px'></div>T</div>",
            &[
                ("b", [30.0, 10.0, 40.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=b style='display:flow-root;width:40px;height:10px;margin-right:10px;margin-left:auto'></div>T</div>",
            &[
                ("b", [30.0, 10.0, 40.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=b style='display:flow-root;min-width:40px;max-width:50px;height:10px'></div>T</div>",
            &[
                ("b", [20.0, 10.0, 50.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=b style='display:flow-root;width:40px;height:40px'></div>T</div>",
            &[
                ("b", [30.0, 10.0, 40.0, 40.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=b style='display:flow-root;width:60px;height:40px'></div>T</div>",
            &[
                ("b", [10.0, 10.0, 60.0, 40.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=b style='display:flow-root;width:40px;height:10px;direction:ltr'></div>T</div>",
            &[
                ("b", [30.0, 10.0, 40.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:right;width:30px;height:30px'></div><div id=b style='display:flow-root;height:10px;padding:0 5px'>AB</div>T</div>",
            &[
                ("b", [0.0, 10.0, 70.0, 10.0]),
                ("f", [70.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='display:flow-root;width:40px;height:10px;justify-self:center'></div>T</div>",
            &[
                ("b", [45.0, 10.0, 40.0, 10.0]),
                ("f", [0.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='display:flow-root;width:40px;height:10px;justify-self:start'></div>T</div>",
            &[
                ("b", [60.0, 10.0, 40.0, 10.0]),
                ("f", [0.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='display:flow-root;width:40px;height:10px;justify-self:end'></div>T</div>",
            &[
                ("b", [30.0, 10.0, 40.0, 10.0]),
                ("f", [0.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='display:flow-root;width:40px;height:10px;margin-left:-10px'></div>T</div>",
            &[
                ("b", [60.0, 10.0, 40.0, 10.0]),
                ("f", [0.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='display:flow-root;width:40px;height:10px;margin-right:-10px'></div>T</div>",
            &[
                ("b", [60.0, 10.0, 40.0, 10.0]),
                ("f", [0.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='display:flow-root;width:40px;height:10px;margin-right:10px;margin-left:auto'></div>T</div>",
            &[
                ("b", [50.0, 10.0, 40.0, 10.0]),
                ("f", [0.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='display:flow-root;min-width:40px;max-width:50px;height:10px'></div>T</div>",
            &[
                ("b", [50.0, 10.0, 50.0, 10.0]),
                ("f", [0.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='display:flow-root;width:40px;height:40px'></div>T</div>",
            &[
                ("b", [60.0, 10.0, 40.0, 40.0]),
                ("f", [0.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='display:flow-root;width:60px;height:40px'></div>T</div>",
            &[
                ("b", [40.0, 10.0, 60.0, 40.0]),
                ("f", [0.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='display:flow-root;width:40px;height:10px;direction:ltr'></div>T</div>",
            &[
                ("b", [60.0, 10.0, 40.0, 10.0]),
                ("f", [0.0, 0.0, 30.0, 30.0]),
            ],
        ),
        (
            "<div style='width:100px;direction:rtl'>T<div id=f style='float:left;width:30px;height:30px'></div><div id=b style='display:flow-root;height:10px;padding:0 5px'>AB</div>T</div>",
            &[
                ("b", [30.0, 10.0, 70.0, 10.0]),
                ("f", [0.0, 0.0, 30.0, 30.0]),
            ],
        ),
    ]);
}
