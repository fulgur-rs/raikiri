//! A table with a caption in a column flex container is flexed from a flex
//! base size that holds its captions: the `flex-basis` it states, or the
//! `height` that stands in for an `auto` one, is the height of its grid box,
//! and the captions come on top of it (CSS Tables 3 §3.1, CSS Flexbox 1 §9.2).
//! A container no taller than that shrinks the item, the grid box taking what
//! the captions leave, and a container whose height follows its items grows
//! by the captions.
//!
//! The expected border boxes `[x, y, width, height]` are the ones Chromium
//! computes for the same markup in an 800px by 600px viewport with no body
//! margin, `font: 10px/10px Ahem` and `div { box-sizing: border-box }`. The
//! box of the table element itself is left out: Chromium reports the wrapper
//! box for it, raikiri the grid box.

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
    let mut boxes = HashMap::new();
    for fragment in page.fragments() {
        if let Some(id) = dom.attr(fragment.node(), "id") {
            let rect = fragment.rect();
            boxes
                .entry(id.to_owned())
                .or_insert([rect.x, rect.y, rect.width, rect.height]);
        }
    }
    boxes
}

/// The border boxes expected of the elements with these ids.
type Expected<'a> = &'a [(&'a str, [f32; 4])];

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.04;

fn assert_cases(cases: &[(&str, Expected<'_>)]) {
    let mut failures = Vec::new();
    for (body, expected) in cases {
        let boxes = border_boxes(body);
        for (id, want) in expected.iter() {
            match boxes.get(*id) {
                Some(got)
                    if got
                        .iter()
                        .zip(want)
                        .all(|(g, w)| (g - w).abs() <= TOLERANCE) => {}
                got => failures.push(format!("{body}\n  #{id}: want {want:?}, got {got:?}")),
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A column of a definite height that is no taller than the stated height and
/// the captions shrinks the table item to it, and one that is taller grows it
/// or leaves it at the stated height and the captions.
#[test]
fn a_container_no_taller_than_the_basis_and_the_captions_shrinks_the_table() {
    assert_cases(&[
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 40.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style=''>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 40.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='padding:3px;border:2px solid;flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 30.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='padding:3px;border:2px solid;flex:0 1 50px;border-spacing:0'><caption id=cap style=''>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 30.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 40.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style=''>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 40.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='padding:3px;border:2px solid;flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 30.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='padding:3px;border:2px solid;flex:1 1 50px;border-spacing:0'><caption id=cap style=''>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 30.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 40.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style=''>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 40.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='padding:3px;border:2px solid;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 30.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='padding:3px;border:2px solid;height:50px;border-spacing:0'><caption id=cap style=''>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 30.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='flex:0 1 50px;min-height:0;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 40.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='flex:0 1 50px;min-height:0;border-spacing:0'><caption id=cap style=''>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 40.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='padding:3px;border:2px solid;flex:0 1 50px;min-height:0;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 30.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='padding:3px;border:2px solid;flex:0 1 50px;min-height:0;border-spacing:0'><caption id=cap style=''>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 30.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='flex:0 1 auto;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 40.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='flex:0 1 auto;height:50px;border-spacing:0'><caption id=cap style=''>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 40.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='padding:3px;border:2px solid;flex:0 1 auto;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 30.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='padding:3px;border:2px solid;flex:0 1 auto;height:50px;border-spacing:0'><caption id=cap style=''>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 30.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 45.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style=''>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 45.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='padding:3px;border:2px solid;flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 35.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='padding:3px;border:2px solid;flex:0 1 50px;border-spacing:0'><caption id=cap style=''>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 35.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='flex:1 0 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='flex:1 0 50px;border-spacing:0'><caption id=cap style=''>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='padding:3px;border:2px solid;flex:1 0 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 40.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='padding:3px;border:2px solid;flex:1 0 50px;border-spacing:0'><caption id=cap style=''>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 40.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 45.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style=''>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 45.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='padding:3px;border:2px solid;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 35.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='padding:3px;border:2px solid;height:50px;border-spacing:0'><caption id=cap style=''>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 35.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='flex:0 1 50px;min-height:0;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 45.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='flex:0 1 50px;min-height:0;border-spacing:0'><caption id=cap style=''>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 45.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='padding:3px;border:2px solid;flex:0 1 50px;min-height:0;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 35.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='padding:3px;border:2px solid;flex:0 1 50px;min-height:0;border-spacing:0'><caption id=cap style=''>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 35.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='flex:0 1 auto;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 45.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='flex:0 1 auto;height:50px;border-spacing:0'><caption id=cap style=''>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 45.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='padding:3px;border:2px solid;flex:0 1 auto;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 35.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='padding:3px;border:2px solid;flex:0 1 auto;height:50px;border-spacing:0'><caption id=cap style=''>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 35.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 40.0, 800.0, 10.0]),
                ("r", [0.0, 0.0, 800.0, 40.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 40.0, 800.0, 10.0]),
                ("r", [0.0, 0.0, 800.0, 40.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px;margin:2px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [2.0, 2.0, 796.0, 10.0]),
                ("r", [0.0, 14.0, 800.0, 36.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px;margin:2px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [2.0, 2.0, 796.0, 10.0]),
                ("r", [0.0, 14.0, 800.0, 36.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 45.0, 800.0, 10.0]),
                ("r", [0.0, 0.0, 800.0, 45.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 45.0, 800.0, 10.0]),
                ("r", [0.0, 0.0, 800.0, 45.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px;margin:2px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [2.0, 2.0, 796.0, 10.0]),
                ("r", [0.0, 14.0, 800.0, 41.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px;margin:2px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [2.0, 2.0, 796.0, 10.0]),
                ("r", [0.0, 14.0, 800.0, 41.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:40px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 40.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 30.0]),
                ("z", [0.0, 40.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:40px'><table id=t style='padding:3px;border:2px solid;flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 40.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 20.0]),
                ("z", [0.0, 40.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:40px'><table id=t style='flex:1 0 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 40.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 40.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:40px'><table id=t style='padding:3px;border:2px solid;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 40.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 20.0]),
                ("z", [0.0, 40.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:40px'><table id=t style='flex:0 1 50px;min-height:0;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 40.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 30.0]),
                ("z", [0.0, 40.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:40px'><table id=t style='padding:3px;border:2px solid;flex:0 1 auto;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 40.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 20.0]),
                ("z", [0.0, 40.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 35.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px'><table id=t style='padding:3px;border:2px solid;flex:0 0 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 40.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 35.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px'><table id=t style='padding:3px;border:2px solid;height:50px;flex-shrink:0;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 40.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px'><table id=t style='flex:0 1 auto;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 35.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='padding:3px;border:2px solid;flex:0 0 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 40.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:50px'><table id=t style='height:50px;flex-shrink:0;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 50.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='padding:3px;border:2px solid;flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 35.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='height:50px;flex-shrink:0;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:60px'><table id=t style='padding:3px;border:2px solid;flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:60px'><table id=t style='flex:0 0 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:60px'><table id=t style='padding:3px;border:2px solid;flex:1 0 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:60px'><table id=t style='height:50px;flex-shrink:0;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:60px'><table id=t style='padding:3px;border:2px solid;flex:0 1 50px;min-height:0;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:65px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 65.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 65.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:65px'><table id=t style='padding:3px;border:2px solid;flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 65.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 45.0]),
                ("z", [0.0, 65.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:65px'><table id=t style='flex:1 0 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 65.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 55.0]),
                ("z", [0.0, 65.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:65px'><table id=t style='padding:3px;border:2px solid;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 65.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 40.0]),
                ("z", [0.0, 65.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:65px'><table id=t style='flex:0 1 50px;min-height:0;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 65.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 65.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:65px'><table id=t style='padding:3px;border:2px solid;flex:0 1 auto;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 65.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 40.0]),
                ("z", [0.0, 65.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 60.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='padding:3px;border:2px solid;flex:0 0 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 40.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='padding:3px;border:2px solid;height:50px;flex-shrink:0;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 40.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='flex:0 1 auto;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
    ]);
}

/// Beside other items, the table shrinks and grows in proportion to a flex
/// base size that holds its captions; a column whose height follows its items
/// is as tall as the stated height and the captions.
#[test]
fn the_captions_take_part_in_flexing_beside_other_items_and_in_the_container_height() {
    assert_cases(&[
        (
            "<div id=c style='display:flex;flex-direction:column;height:60px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:0 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 40.0, 800.0, 20.0]),
                ("r", [0.0, 10.0, 800.0, 30.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:60px'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:0 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 40.0, 800.0, 20.0]),
                ("r", [0.0, 10.0, 800.0, 30.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:60px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:0 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 40.0, 800.0, 20.0]),
                ("r", [0.0, 10.0, 800.0, 30.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:60px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:1 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 40.0, 800.0, 20.0]),
                ("r", [0.0, 10.0, 800.0, 30.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:60px'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:1 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 40.0, 800.0, 20.0]),
                ("r", [0.0, 10.0, 800.0, 30.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:60px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:1 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 40.0, 800.0, 20.0]),
                ("r", [0.0, 10.0, 800.0, 30.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:60px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 40.0, 800.0, 20.0]),
                ("r", [0.0, 10.0, 800.0, 30.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:60px'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 40.0, 800.0, 20.0]),
                ("r", [0.0, 10.0, 800.0, 30.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:60px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 40.0, 800.0, 20.0]),
                ("r", [0.0, 10.0, 800.0, 30.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:0 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 46.671875, 800.0, 23.328125]),
                ("r", [0.0, 10.0, 800.0, 36.671875]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:0 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 46.671875, 800.0, 23.328125]),
                ("r", [0.0, 10.0, 800.0, 36.671875]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:0 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 46.671875, 800.0, 23.328125]),
                ("r", [0.0, 10.0, 800.0, 36.671875]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:1 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 46.671875, 800.0, 23.328125]),
                ("r", [0.0, 10.0, 800.0, 36.671875]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:1 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 46.671875, 800.0, 23.328125]),
                ("r", [0.0, 10.0, 800.0, 36.671875]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:1 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 46.671875, 800.0, 23.328125]),
                ("r", [0.0, 10.0, 800.0, 36.671875]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 46.671875, 800.0, 23.328125]),
                ("r", [0.0, 10.0, 800.0, 36.671875]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 46.671875, 800.0, 23.328125]),
                ("r", [0.0, 10.0, 800.0, 36.671875]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 46.671875, 800.0, 23.328125]),
                ("r", [0.0, 10.0, 800.0, 36.671875]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:80px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:0 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 80.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 53.328125, 800.0, 26.671875]),
                ("r", [0.0, 10.0, 800.0, 43.328125]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:80px'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:0 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 80.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 53.328125, 800.0, 26.671875]),
                ("r", [0.0, 10.0, 800.0, 43.328125]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:80px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:0 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 80.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 53.328125, 800.0, 26.671875]),
                ("r", [0.0, 10.0, 800.0, 43.328125]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:80px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:1 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 80.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 53.328125, 800.0, 26.671875]),
                ("r", [0.0, 10.0, 800.0, 43.328125]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:80px'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:1 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 80.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 53.328125, 800.0, 26.671875]),
                ("r", [0.0, 10.0, 800.0, 43.328125]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:80px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:1 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 80.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 53.328125, 800.0, 26.671875]),
                ("r", [0.0, 10.0, 800.0, 43.328125]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:80px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 80.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 53.328125, 800.0, 26.671875]),
                ("r", [0.0, 10.0, 800.0, 43.328125]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:80px'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 80.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 53.328125, 800.0, 26.671875]),
                ("r", [0.0, 10.0, 800.0, 43.328125]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:80px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 80.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 53.328125, 800.0, 26.671875]),
                ("r", [0.0, 10.0, 800.0, 43.328125]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:100px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:0 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 100.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 800.0, 30.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 100.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:100px'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:0 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 100.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 70.0, 800.0, 30.0]),
                ("r", [0.0, 10.0, 800.0, 60.0]),
                ("z", [0.0, 100.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:100px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:0 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 100.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 800.0, 30.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 100.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:100px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:1 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 100.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 800.0, 40.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 100.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:100px'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:1 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 100.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 65.0, 800.0, 35.0]),
                ("r", [0.0, 10.0, 800.0, 55.0]),
                ("z", [0.0, 100.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:100px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:1 1 30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 100.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 800.0, 40.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 100.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:100px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 100.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 800.0, 30.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 100.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:100px'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 100.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 70.0, 800.0, 30.0]),
                ("r", [0.0, 10.0, 800.0, 60.0]),
                ("z", [0.0, 100.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:100px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:30px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 100.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 800.0, 30.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 100.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;max-height:55px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 45.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;min-height:0'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;max-height:55px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 45.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;min-height:0'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
    ]);
}

/// Captions that wrap at the table's width, captions on both sides, the
/// padding and border of the table in either box-sizing, its margins, minimum
/// and maximum heights and percentage bases, and wrapping or reversed columns.
#[test]
fn captions_of_any_size_and_side_count_once_whatever_the_table_states() {
    assert_cases(&[
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px;align-items:start'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 40.0, 40.0]),
                ("r", [0.0, 40.0, 40.0, 10.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px;flex-direction:column-reverse'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:10px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 6.421875, 800.0, 10.0]),
                ("d", [0.0, 0.0, 800.0, 6.421875]),
                ("r", [0.0, 16.421875, 800.0, 28.578125]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px'><table id=t style='flex:0 1 80%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 35.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px;align-items:start'><table id=t style='height:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 40.0, 40.0]),
                ("r", [0.0, 40.0, 40.0, 10.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px;flex-direction:column-reverse'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:10px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 6.421875, 800.0, 10.0]),
                ("d", [0.0, 0.0, 800.0, 6.421875]),
                ("r", [0.0, 16.421875, 800.0, 28.578125]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px'><table id=t style='height:80%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 35.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px;align-items:start'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 40.0, 40.0]),
                ("r", [0.0, 40.0, 40.0, 10.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px;flex-direction:column-reverse'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:10px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 6.421875, 800.0, 10.0]),
                ("d", [0.0, 0.0, 800.0, 6.421875]),
                ("r", [0.0, 16.421875, 800.0, 28.578125]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='flex:0 1 50px;width:60px;border-spacing:0'><caption id=cap>aaa bbb ccc</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 60.0, 30.0]),
                ("r", [0.0, 30.0, 60.0, 25.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;align-items:start'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 40.0, 40.0]),
                ("r", [0.0, 40.0, 40.0, 15.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><caption id=cap2 style='height:7px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("cap2", [0.0, 48.0, 800.0, 7.0]),
                ("r", [0.0, 10.0, 800.0, 38.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='flex:0 1 50px;box-sizing:border-box;padding:4px;border:3px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [7.0, 17.0, 786.0, 31.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;flex-direction:column-reverse'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:10px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 7.859375, 800.0, 10.0]),
                ("d", [0.0, 0.0, 800.0, 7.859375]),
                ("r", [0.0, 17.859375, 800.0, 37.140625]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;flex-wrap:wrap'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 45.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr><tr id=r2><td style='padding:0'>y</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 22.5]),
                ("r2", [0.0, 32.5, 800.0, 22.5]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;display:inline-flex'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 20.0, 55.0]),
                ("cap", [0.0, 0.0, 20.0, 20.0]),
                ("r", [0.0, 20.0, 20.0, 35.0]),
                ("z", [0.0, 57.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='height:50px;width:60px;border-spacing:0'><caption id=cap>aaa bbb ccc</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 60.0, 30.0]),
                ("r", [0.0, 30.0, 60.0, 25.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;align-items:start'><table id=t style='height:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 40.0, 40.0]),
                ("r", [0.0, 40.0, 40.0, 15.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><caption id=cap2 style='height:7px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("cap2", [0.0, 48.0, 800.0, 7.0]),
                ("r", [0.0, 10.0, 800.0, 38.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='height:50px;box-sizing:border-box;padding:4px;border:3px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [7.0, 17.0, 786.0, 31.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;flex-direction:column-reverse'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:10px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 7.859375, 800.0, 10.0]),
                ("d", [0.0, 0.0, 800.0, 7.859375]),
                ("r", [0.0, 17.859375, 800.0, 37.140625]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;flex-wrap:wrap'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 45.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr><tr id=r2><td style='padding:0'>y</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 22.5]),
                ("r2", [0.0, 32.5, 800.0, 22.5]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;display:inline-flex'><table id=t style='height:50px;border-spacing:0'><caption id=cap>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 20.0, 55.0]),
                ("cap", [0.0, 0.0, 20.0, 20.0]),
                ("r", [0.0, 20.0, 20.0, 35.0]),
                ("z", [0.0, 57.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;flex-direction:column-reverse'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:10px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 7.859375, 800.0, 10.0]),
                ("d", [0.0, 0.0, 800.0, 7.859375]),
                ("r", [0.0, 17.859375, 800.0, 37.140625]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='flex:0 1 50px;width:60px;border-spacing:0'><caption id=cap>aaa bbb ccc</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 60.0, 30.0]),
                ("r", [0.0, 30.0, 60.0, 40.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px;align-items:start'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 40.0, 40.0]),
                ("r", [0.0, 40.0, 40.0, 30.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='flex:0 1 50px;box-sizing:content-box;padding:4px;border:3px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [7.0, 17.0, 786.0, 46.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='flex:0 1 50px;min-height:58px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px;flex-direction:column-reverse'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:10px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 10.0, 800.0, 10.0]),
                ("d", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 20.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='height:50px;width:60px;border-spacing:0'><caption id=cap>aaa bbb ccc</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 60.0, 30.0]),
                ("r", [0.0, 30.0, 60.0, 40.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px;align-items:start'><table id=t style='height:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 40.0, 40.0]),
                ("r", [0.0, 40.0, 40.0, 30.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='height:50px;box-sizing:content-box;padding:4px;border:3px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [7.0, 17.0, 786.0, 46.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='height:50px;min-height:58px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px;flex-direction:column-reverse'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:10px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 10.0, 800.0, 10.0]),
                ("d", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 20.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='flex:0 1 50px;width:60px;border-spacing:0'><caption id=cap>aaa bbb ccc</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 80.0]),
                ("cap", [0.0, 0.0, 60.0, 30.0]),
                ("r", [0.0, 30.0, 60.0, 50.0]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;;align-items:start'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 90.0]),
                ("cap", [0.0, 0.0, 40.0, 40.0]),
                ("r", [0.0, 40.0, 40.0, 50.0]),
                ("z", [0.0, 90.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><caption id=cap2 style='height:7px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 67.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("cap2", [0.0, 60.0, 800.0, 7.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 67.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='flex:0 1 50px;box-sizing:content-box;padding:4px;border:3px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 74.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [7.0, 17.0, 786.0, 50.0]),
                ("z", [0.0, 74.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='flex:0 1 50px;box-sizing:border-box;padding:4px;border:3px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [7.0, 17.0, 786.0, 36.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='flex:0 1 50px;min-height:58px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;;flex-direction:column-reverse'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:10px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 10.0, 800.0, 10.0]),
                ("d", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 20.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;;flex-wrap:wrap'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='flex:0 1 50px;margin:3px 20px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 66.0]),
                ("cap", [20.0, 3.0, 760.0, 10.0]),
                ("r", [20.0, 13.0, 760.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr><tr id=r2><td style='padding:0'>y</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 25.0]),
                ("r2", [0.0, 35.0, 800.0, 25.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='height:50px;width:60px;border-spacing:0'><caption id=cap>aaa bbb ccc</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 80.0]),
                ("cap", [0.0, 0.0, 60.0, 30.0]),
                ("r", [0.0, 30.0, 60.0, 50.0]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;;align-items:start'><table id=t style='height:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 90.0]),
                ("cap", [0.0, 0.0, 40.0, 40.0]),
                ("r", [0.0, 40.0, 40.0, 50.0]),
                ("z", [0.0, 90.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><caption id=cap2 style='height:7px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 67.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("cap2", [0.0, 60.0, 800.0, 7.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 67.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='height:50px;box-sizing:content-box;padding:4px;border:3px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 74.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [7.0, 17.0, 786.0, 50.0]),
                ("z", [0.0, 74.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='height:50px;box-sizing:border-box;padding:4px;border:3px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [7.0, 17.0, 786.0, 36.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='height:50px;min-height:58px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;;flex-direction:column-reverse'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:10px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 10.0, 800.0, 10.0]),
                ("d", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 20.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;;flex-wrap:wrap'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='height:50px;margin:3px 20px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 66.0]),
                ("cap", [20.0, 3.0, 760.0, 10.0]),
                ("r", [20.0, 13.0, 760.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr><tr id=r2><td style='padding:0'>y</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 25.0]),
                ("r2", [0.0, 35.0, 800.0, 25.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='flex:1 1 50px;width:60px;border-spacing:0'><caption id=cap>aaa bbb ccc</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 80.0]),
                ("cap", [0.0, 0.0, 60.0, 30.0]),
                ("r", [0.0, 30.0, 60.0, 50.0]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;;align-items:start'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 90.0]),
                ("cap", [0.0, 0.0, 40.0, 40.0]),
                ("r", [0.0, 40.0, 40.0, 50.0]),
                ("z", [0.0, 90.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><caption id=cap2 style='height:7px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 67.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("cap2", [0.0, 60.0, 800.0, 7.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 67.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='flex:1 1 50px;box-sizing:content-box;padding:4px;border:3px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 74.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [7.0, 17.0, 786.0, 50.0]),
                ("z", [0.0, 74.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='flex:1 1 50px;box-sizing:border-box;padding:4px;border:3px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [7.0, 17.0, 786.0, 36.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='flex:1 1 50px;min-height:58px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;;flex-direction:column-reverse'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='height:10px'></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 10.0, 800.0, 10.0]),
                ("d", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 20.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;;flex-wrap:wrap'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='flex:1 1 50px;margin:3px 20px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 66.0]),
                ("cap", [20.0, 3.0, 760.0, 10.0]),
                ("r", [20.0, 13.0, 760.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr><tr id=r2><td style='padding:0'>y</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 25.0]),
                ("r2", [0.0, 35.0, 800.0, 25.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px'><table id=t style='flex:0 1 50px;width:60px;border-spacing:0'><caption id=cap>aaa bbb ccc</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 60.0, 30.0]),
                ("r", [0.0, 30.0, 60.0, 15.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px'><table id=t style='flex:0 1 50px;max-height:45px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 35.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px;display:inline-flex'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 20.0, 45.0]),
                ("cap", [0.0, 0.0, 20.0, 20.0]),
                ("r", [0.0, 20.0, 20.0, 25.0]),
                ("z", [0.0, 47.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px'><table id=t style='height:50px;min-height:58px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 48.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr><tr id=r2><td style='padding:0'>y</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 17.5]),
                ("r2", [0.0, 27.5, 800.0, 17.5]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px'><table id=t style='flex:1 1 50px;box-sizing:border-box;padding:4px;border:3px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [7.0, 17.0, 786.0, 21.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px'><table id=t style='flex:1 1 50px;margin:3px 20px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [20.0, 3.0, 760.0, 10.0]),
                ("r", [20.0, 13.0, 760.0, 29.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='flex:0 1 50px;min-height:58px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 48.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='height:50px;box-sizing:content-box;padding:4px;border:3px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [7.0, 17.0, 786.0, 31.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;flex-direction:row'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("r", [0.0, 10.0, 10.0, 50.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='flex:1 1 50px;box-sizing:border-box;padding:4px;border:3px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [7.0, 17.0, 786.0, 31.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='flex:1 1 50px;margin:3px 20px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [20.0, 3.0, 760.0, 10.0]),
                ("r", [20.0, 13.0, 760.0, 39.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='flex:0 1 50px;box-sizing:border-box;padding:4px;border:3px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [7.0, 17.0, 786.0, 36.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px;flex-direction:row'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 50.0, 10.0]),
                ("r", [0.0, 10.0, 50.0, 60.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='height:50px;max-height:45px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 35.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr><tr id=r2><td style='padding:0'>y</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 25.0]),
                ("r2", [0.0, 35.0, 800.0, 25.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px'><table id=t style='flex:1 1 50px;box-sizing:content-box;padding:4px;border:3px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [7.0, 17.0, 786.0, 46.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px;flex-wrap:wrap'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 60.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:70px;display:inline-flex'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap>aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 20.0, 70.0]),
                ("cap", [0.0, 0.0, 20.0, 20.0]),
                ("r", [0.0, 20.0, 20.0, 50.0]),
                ("z", [0.0, 72.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='height:50px;max-height:45px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 35.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;;flex-direction:row'><table id=t style='flex:1 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 20.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 10.0]),
                ("z", [0.0, 20.0, 800.0, 10.0]),
            ],
        ),
    ]);
}

/// The captions are measured at the width the table takes: the room its
/// margins leave within its `min-width` and `max-width` when the column
/// stretches it, and no less than its columns need; in a column that wraps
/// its items, the width it takes for itself before the lines stretch it.
/// A percentage basis resolves against a definite column height only.
#[test]
fn the_captions_are_measured_at_the_width_the_table_takes() {
    assert_cases(&[
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;width:100px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'><div style='width:300px;height:5px'></div></td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("cap", [0.0, 0.0, 300.0, 10.0]),
                ("r", [0.0, 10.0, 300.0, 45.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;width:200px'><table id=t style='flex:0 1 50px;max-width:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 200.0, 55.0]),
                ("cap", [0.0, 0.0, 50.0, 40.0]),
                ("r", [0.0, 40.0, 50.0, 15.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;flex-wrap:wrap;width:800px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 45.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;flex-wrap:wrap;align-content:start;width:800px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 40.0, 40.0]),
                ("r", [0.0, 40.0, 40.0, 15.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;align-items:start;width:100px'><table id=t style='flex:0 1 50px;margin:0 30px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd eee fff</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("cap", [30.0, 0.0, 30.0, 60.0]),
                ("r", [30.0, 60.0, 30.0, 10.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;width:100px'><table id=t style='flex:0 1 50px;margin:0 30px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd eee fff</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("cap", [30.0, 0.0, 40.0, 60.0]),
                ("r", [30.0, 60.0, 40.0, 10.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='flex:0 1 50px;min-height:58px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 48.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='flex:0 1 50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 27.5]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><div id=n style='display:flex;flex-direction:column;flex:0 1 auto'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("n", [0.0, 0.0, 800.0, 60.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;width:100px'><table id=t style='height:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'><div style='width:300px;height:5px'></div></td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("cap", [0.0, 0.0, 300.0, 10.0]),
                ("r", [0.0, 10.0, 300.0, 45.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;width:200px'><table id=t style='height:50px;max-width:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 200.0, 55.0]),
                ("cap", [0.0, 0.0, 50.0, 40.0]),
                ("r", [0.0, 40.0, 50.0, 15.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;flex-wrap:wrap;width:800px'><table id=t style='height:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 45.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;flex-wrap:wrap;align-content:start;width:800px'><table id=t style='height:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 40.0, 40.0]),
                ("r", [0.0, 40.0, 40.0, 15.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;align-items:start;width:100px'><table id=t style='height:50px;margin:0 30px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd eee fff</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("cap", [30.0, 0.0, 30.0, 60.0]),
                ("r", [30.0, 60.0, 30.0, 10.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;width:100px'><table id=t style='height:50px;margin:0 30px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd eee fff</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 55.0]),
                ("cap", [30.0, 0.0, 40.0, 60.0]),
                ("r", [30.0, 60.0, 40.0, 10.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='height:50px;min-height:58px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 48.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='height:50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 27.5]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><div id=n style='display:flex;flex-direction:column;flex:0 1 auto'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("n", [0.0, 0.0, 800.0, 60.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:200px;width:100px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'><div style='width:300px;height:5px'></div></td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 200.0]),
                ("cap", [0.0, 0.0, 300.0, 10.0]),
                ("r", [0.0, 10.0, 300.0, 50.0]),
                ("z", [0.0, 200.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:200px;width:200px'><table id=t style='flex:0 1 50px;max-width:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 200.0, 200.0]),
                ("cap", [0.0, 0.0, 50.0, 40.0]),
                ("r", [0.0, 40.0, 50.0, 50.0]),
                ("z", [0.0, 200.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:200px;flex-wrap:wrap;width:800px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 200.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 80.0]),
                ("z", [0.0, 200.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:200px;flex-wrap:wrap;align-content:start;width:800px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 200.0]),
                ("cap", [0.0, 0.0, 40.0, 40.0]),
                ("r", [0.0, 40.0, 40.0, 50.0]),
                ("z", [0.0, 200.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:200px;align-items:start;width:100px'><table id=t style='flex:0 1 50px;margin:0 30px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd eee fff</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 200.0]),
                ("cap", [30.0, 0.0, 30.0, 60.0]),
                ("r", [30.0, 60.0, 30.0, 50.0]),
                ("z", [0.0, 200.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:200px;width:100px'><table id=t style='flex:0 1 50px;margin:0 30px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd eee fff</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 200.0]),
                ("cap", [30.0, 0.0, 40.0, 60.0]),
                ("r", [30.0, 60.0, 40.0, 50.0]),
                ("z", [0.0, 200.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:200px'><table id=t style='flex:0 1 50px;min-height:58px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 200.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 200.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:200px'><table id=t style='flex:0 1 50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 200.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 100.0]),
                ("z", [0.0, 200.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:200px'><div id=n style='display:flex;flex-direction:column;flex:0 1 auto'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 200.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("n", [0.0, 0.0, 800.0, 60.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 200.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:200px;width:100px'><table id=t style='height:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'><div style='width:300px;height:5px'></div></td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 200.0]),
                ("cap", [0.0, 0.0, 300.0, 10.0]),
                ("r", [0.0, 10.0, 300.0, 50.0]),
                ("z", [0.0, 200.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:200px;width:200px'><table id=t style='height:50px;max-width:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 200.0, 200.0]),
                ("cap", [0.0, 0.0, 50.0, 40.0]),
                ("r", [0.0, 40.0, 50.0, 50.0]),
                ("z", [0.0, 200.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:200px;flex-wrap:wrap;width:800px'><table id=t style='height:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 200.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 80.0]),
                ("z", [0.0, 200.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:200px;flex-wrap:wrap;align-content:start;width:800px'><table id=t style='height:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 200.0]),
                ("cap", [0.0, 0.0, 40.0, 40.0]),
                ("r", [0.0, 40.0, 40.0, 50.0]),
                ("z", [0.0, 200.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:200px;align-items:start;width:100px'><table id=t style='height:50px;margin:0 30px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd eee fff</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 200.0]),
                ("cap", [30.0, 0.0, 30.0, 60.0]),
                ("r", [30.0, 60.0, 30.0, 50.0]),
                ("z", [0.0, 200.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:200px;width:100px'><table id=t style='height:50px;margin:0 30px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd eee fff</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 200.0]),
                ("cap", [30.0, 0.0, 40.0, 60.0]),
                ("r", [30.0, 60.0, 40.0, 50.0]),
                ("z", [0.0, 200.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:200px'><table id=t style='height:50px;min-height:58px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 200.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 200.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:200px'><table id=t style='height:50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 200.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 100.0]),
                ("z", [0.0, 200.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:200px'><div id=n style='display:flex;flex-direction:column;flex:0 1 auto'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 200.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("n", [0.0, 0.0, 800.0, 60.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 200.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px;width:100px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'><div style='width:300px;height:5px'></div></td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 45.0]),
                ("cap", [0.0, 0.0, 300.0, 10.0]),
                ("r", [0.0, 10.0, 300.0, 35.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px;width:200px'><table id=t style='flex:0 1 50px;max-width:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 200.0, 45.0]),
                ("cap", [0.0, 0.0, 50.0, 40.0]),
                ("r", [0.0, 40.0, 50.0, 10.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px;flex-wrap:wrap;align-content:start;width:800px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 40.0, 40.0]),
                ("r", [0.0, 40.0, 40.0, 10.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px;align-items:start;width:100px'><table id=t style='flex:0 1 50px;margin:0 30px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd eee fff</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 45.0]),
                ("cap", [30.0, 0.0, 30.0, 60.0]),
                ("r", [30.0, 60.0, 30.0, 10.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px;width:100px'><table id=t style='flex:0 1 50px;margin:0 30px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd eee fff</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 45.0]),
                ("cap", [30.0, 0.0, 40.0, 60.0]),
                ("r", [30.0, 60.0, 40.0, 10.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px'><table id=t style='flex:0 1 50px;min-height:58px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 48.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px'><table id=t style='flex:0 1 50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 22.5]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px'><div id=n style='display:flex;flex-direction:column;flex:0 1 auto'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("n", [0.0, 0.0, 800.0, 60.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px;width:100px'><table id=t style='height:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'><div style='width:300px;height:5px'></div></td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 45.0]),
                ("cap", [0.0, 0.0, 300.0, 10.0]),
                ("r", [0.0, 10.0, 300.0, 35.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px;width:200px'><table id=t style='height:50px;max-width:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 200.0, 45.0]),
                ("cap", [0.0, 0.0, 50.0, 40.0]),
                ("r", [0.0, 40.0, 50.0, 10.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px;flex-wrap:wrap;align-content:start;width:800px'><table id=t style='height:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 40.0, 40.0]),
                ("r", [0.0, 40.0, 40.0, 10.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px;align-items:start;width:100px'><table id=t style='height:50px;margin:0 30px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd eee fff</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 45.0]),
                ("cap", [30.0, 0.0, 30.0, 60.0]),
                ("r", [30.0, 60.0, 30.0, 10.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px;width:100px'><table id=t style='height:50px;margin:0 30px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd eee fff</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 45.0]),
                ("cap", [30.0, 0.0, 40.0, 60.0]),
                ("r", [30.0, 60.0, 40.0, 10.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px'><table id=t style='height:50px;min-height:58px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 48.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px'><table id=t style='height:50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 22.5]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:45px'><div id=n style='display:flex;flex-direction:column;flex:0 1 auto'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 45.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("n", [0.0, 0.0, 800.0, 60.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 45.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;;width:100px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'><div style='width:300px;height:5px'></div></td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 60.0]),
                ("cap", [0.0, 0.0, 300.0, 10.0]),
                ("r", [0.0, 10.0, 300.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;;width:200px'><table id=t style='flex:0 1 50px;max-width:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 200.0, 90.0]),
                ("cap", [0.0, 0.0, 50.0, 40.0]),
                ("r", [0.0, 40.0, 50.0, 50.0]),
                ("z", [0.0, 90.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;;flex-wrap:wrap;width:800px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 90.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 80.0]),
                ("z", [0.0, 90.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;;flex-wrap:wrap;align-content:start;width:800px'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 90.0]),
                ("cap", [0.0, 0.0, 40.0, 40.0]),
                ("r", [0.0, 40.0, 40.0, 50.0]),
                ("z", [0.0, 90.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;;align-items:start;width:100px'><table id=t style='flex:0 1 50px;margin:0 30px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd eee fff</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 110.0]),
                ("cap", [30.0, 0.0, 30.0, 60.0]),
                ("r", [30.0, 60.0, 30.0, 50.0]),
                ("z", [0.0, 110.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;;width:100px'><table id=t style='flex:0 1 50px;margin:0 30px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd eee fff</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 110.0]),
                ("cap", [30.0, 0.0, 40.0, 60.0]),
                ("r", [30.0, 60.0, 40.0, 50.0]),
                ("z", [0.0, 110.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='flex:0 1 50px;min-height:58px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='flex:0 1 50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 20.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 10.0]),
                ("z", [0.0, 20.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><div id=n style='display:flex;flex-direction:column;flex:0 1 auto'><table id=t style='flex:0 1 50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("n", [0.0, 0.0, 800.0, 60.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;;width:100px'><table id=t style='height:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'><div style='width:300px;height:5px'></div></td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 60.0]),
                ("cap", [0.0, 0.0, 300.0, 10.0]),
                ("r", [0.0, 10.0, 300.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;;width:200px'><table id=t style='height:50px;max-width:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 200.0, 90.0]),
                ("cap", [0.0, 0.0, 50.0, 40.0]),
                ("r", [0.0, 40.0, 50.0, 50.0]),
                ("z", [0.0, 90.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;;flex-wrap:wrap;width:800px'><table id=t style='height:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 90.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 80.0]),
                ("z", [0.0, 90.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;;flex-wrap:wrap;align-content:start;width:800px'><table id=t style='height:50px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd</caption><tr id=r><td style='padding:0'>xxxx</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 90.0]),
                ("cap", [0.0, 0.0, 40.0, 40.0]),
                ("r", [0.0, 40.0, 40.0, 50.0]),
                ("z", [0.0, 90.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;;align-items:start;width:100px'><table id=t style='height:50px;margin:0 30px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd eee fff</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 110.0]),
                ("cap", [30.0, 0.0, 30.0, 60.0]),
                ("r", [30.0, 60.0, 30.0, 50.0]),
                ("z", [0.0, 110.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;;width:100px'><table id=t style='height:50px;margin:0 30px;border-spacing:0'><caption id=cap>aaa bbb ccc ddd eee fff</caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 100.0, 110.0]),
                ("cap", [30.0, 0.0, 40.0, 60.0]),
                ("r", [30.0, 60.0, 40.0, 50.0]),
                ("z", [0.0, 110.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='height:50px;min-height:58px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><table id=t style='height:50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 20.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 10.0]),
                ("z", [0.0, 20.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;'><div id=n style='display:flex;flex-direction:column;flex:0 1 auto'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("n", [0.0, 0.0, 800.0, 60.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
    ]);
}

/// An absolutely positioned table is no flex item, and a `content` flex
/// basis leaves the table to the size of its content: neither has a stated
/// basis for the captions to be added to.
#[test]
fn tables_without_a_stated_basis_are_left_as_they_were() {
    assert_cases(&[
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px;position:relative'><table id=t style='position:absolute;top:0;left:0;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='flex:0 1 50px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [0.0, 0.0, 800.0, 50.0]),
                ("r", [0.0, 10.0, 10.0, 50.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column;height:55px'><table id=t style='flex-basis:content;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 10.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-direction:column'><table id=t style='flex-basis:content;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 20.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 10.0]),
                ("z", [0.0, 20.0, 800.0, 10.0]),
            ],
        ),
    ]);
}
