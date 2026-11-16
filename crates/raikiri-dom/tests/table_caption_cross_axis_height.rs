//! A table with a caption in a flex row or a grid is as tall as its whole
//! wrapper box there: the `height` it states, and its `min-height` and
//! `max-height`, are those of its grid box, and the captions come on top of
//! it (CSS Tables 3 §3.1). The line of a flex row and an auto track of a grid
//! hold the stated height and the captions, a line or track of a definite
//! height aligns that box in it, and a stretched table keeps its grid box
//! within its `max-height`. A flex column bounds the whole wrapper box by a
//! `min-height`, as Chromium does.
//!
//! The expected border boxes `[x, y, width, height]` are the ones Chromium
//! computes for the same markup in an 800px by 600px viewport with no body
//! margin, `font: 10px/10px Ahem` and `div { box-sizing: border-box }`. The
//! box of the table element itself is left out: Chromium reports the wrapper
//! box for it, raikiri the grid box. A spacer keeps a table that overflows
//! its line upwards on the page.

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

/// The line of a flex row whose height follows its items is as tall as the
/// stated height of a table item and its captions, whatever the item aligns to
/// and whether the table has rows, and a row of a definite height aligns that
/// whole box in its line.
#[test]
fn a_flex_row_is_as_tall_as_the_whole_wrapper_box_of_a_table_that_states_its_height() {
    assert_cases(&[
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 60.0]),
                ("cap", [0.0, 20.0, 0.0, 10.0]),
                ("d", [0.0, 20.0, 20.0, 60.0]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex'><table id=t style='height:50px;border-spacing:0'></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 50.0]),
                ("d", [0.0, 20.0, 20.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex'><table id=t style=';border-spacing:0'><caption id=cap style='height:10px'></caption></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 10.0]),
                ("cap", [0.0, 20.0, 0.0, 10.0]),
                ("d", [0.0, 20.0, 20.0, 10.0]),
                ("z", [0.0, 30.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex'><table id=t style=';border-spacing:0'></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 20.0, 20.0, 10.0]),
                ("z", [0.0, 30.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex'><table id=t style='min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 60.0]),
                ("cap", [0.0, 20.0, 0.0, 10.0]),
                ("d", [0.0, 20.0, 20.0, 60.0]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex'><table id=t style='min-height:50px;border-spacing:0'></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 50.0]),
                ("d", [0.0, 20.0, 20.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;align-items:start'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 60.0]),
                ("cap", [0.0, 20.0, 0.0, 10.0]),
                ("d", [0.0, 20.0, 20.0, 10.0]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;align-items:start'><table id=t style='height:50px;border-spacing:0'></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 50.0]),
                ("d", [0.0, 20.0, 20.0, 10.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;align-items:start'><table id=t style=';border-spacing:0'><caption id=cap style='height:10px'></caption></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 10.0]),
                ("cap", [0.0, 20.0, 0.0, 10.0]),
                ("d", [0.0, 20.0, 20.0, 10.0]),
                ("z", [0.0, 30.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;align-items:start'><table id=t style=';border-spacing:0'></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 20.0, 20.0, 10.0]),
                ("z", [0.0, 30.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;align-items:start'><table id=t style='min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 60.0]),
                ("cap", [0.0, 20.0, 0.0, 10.0]),
                ("d", [0.0, 20.0, 20.0, 10.0]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;align-items:start'><table id=t style='min-height:50px;border-spacing:0'></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 50.0]),
                ("d", [0.0, 20.0, 20.0, 10.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style=';height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 60.0]),
                ("r", [0.0, 10.0, 10.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style=';height:50px;border-spacing:0'><caption id=cap >aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 20.0, 20.0]),
                ("d", [20.0, 0.0, 20.0, 70.0]),
                ("r", [0.0, 20.0, 20.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style=';height:50px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 50.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 60.0]),
                ("r", [0.0, 0.0, 10.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style=';height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 20.0, 10.0]),
                ("d", [20.0, 0.0, 20.0, 60.0]),
                ("r", [5.0, 15.0, 10.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style=';height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 66.0]),
                ("cap", [0.0, 0.0, 16.0, 10.0]),
                ("d", [16.0, 0.0, 20.0, 66.0]),
                ("r", [3.0, 13.0, 10.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 60.0]),
                ("r", [0.0, 10.0, 10.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap >aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 20.0, 20.0]),
                ("d", [20.0, 0.0, 20.0, 70.0]),
                ("r", [0.0, 20.0, 20.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 50.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 60.0]),
                ("r", [0.0, 0.0, 10.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style='align-self:end;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 20.0, 10.0]),
                ("d", [20.0, 0.0, 20.0, 60.0]),
                ("r", [5.0, 15.0, 10.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style='align-self:end;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 66.0]),
                ("cap", [0.0, 0.0, 16.0, 10.0]),
                ("d", [16.0, 0.0, 20.0, 66.0]),
                ("r", [3.0, 13.0, 10.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style='align-self:start;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 60.0]),
                ("r", [0.0, 10.0, 10.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style='align-self:start;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 20.0, 10.0]),
                ("d", [20.0, 0.0, 20.0, 60.0]),
                ("r", [5.0, 15.0, 10.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style='align-self:start;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 66.0]),
                ("cap", [0.0, 0.0, 16.0, 10.0]),
                ("d", [16.0, 0.0, 20.0, 66.0]),
                ("r", [3.0, 13.0, 10.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style='align-self:center;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 60.0]),
                ("r", [0.0, 10.0, 10.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style='align-self:center;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 20.0, 10.0]),
                ("d", [20.0, 0.0, 20.0, 60.0]),
                ("r", [5.0, 15.0, 10.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style='align-self:center;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 66.0]),
                ("cap", [0.0, 0.0, 16.0, 10.0]),
                ("d", [16.0, 0.0, 20.0, 66.0]),
                ("r", [3.0, 13.0, 10.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;height:80px'><table id=t style=';height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 80.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 80.0]),
                ("r", [0.0, 10.0, 10.0, 50.0]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;height:80px'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 80.0]),
                ("cap", [0.0, 20.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 80.0]),
                ("r", [0.0, 30.0, 10.0, 50.0]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;height:80px'><table id=t style='align-self:center;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 80.0]),
                ("cap", [0.0, 10.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 80.0]),
                ("r", [0.0, 20.0, 10.0, 50.0]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:start'><table id=t style=';height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 10.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:start'><table id=t style=';height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 20.0, 10.0]),
                ("d", [20.0, 0.0, 20.0, 10.0]),
                ("r", [5.0, 15.0, 10.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:start'><table id=t style=';height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 66.0]),
                ("cap", [0.0, 0.0, 16.0, 10.0]),
                ("d", [16.0, 0.0, 20.0, 10.0]),
                ("r", [3.0, 13.0, 10.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:start'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 10.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:start'><table id=t style='align-self:end;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 20.0, 10.0]),
                ("d", [20.0, 0.0, 20.0, 10.0]),
                ("r", [5.0, 15.0, 10.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:start'><table id=t style='align-self:end;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 66.0]),
                ("cap", [0.0, 0.0, 16.0, 10.0]),
                ("d", [16.0, 0.0, 20.0, 10.0]),
                ("r", [3.0, 13.0, 10.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:start'><table id=t style='align-self:start;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 10.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:start'><table id=t style='align-self:start;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 20.0, 10.0]),
                ("d", [20.0, 0.0, 20.0, 10.0]),
                ("r", [5.0, 15.0, 10.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:start'><table id=t style='align-self:start;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 66.0]),
                ("cap", [0.0, 0.0, 16.0, 10.0]),
                ("d", [16.0, 0.0, 20.0, 10.0]),
                ("r", [3.0, 13.0, 10.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:start'><table id=t style='align-self:center;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 10.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:start'><table id=t style='align-self:center;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 20.0, 10.0]),
                ("d", [20.0, 0.0, 20.0, 10.0]),
                ("r", [5.0, 15.0, 10.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:start'><table id=t style='align-self:center;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 66.0]),
                ("cap", [0.0, 0.0, 16.0, 10.0]),
                ("d", [16.0, 0.0, 20.0, 10.0]),
                ("r", [3.0, 13.0, 10.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:center'><table id=t style=';height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 25.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 10.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:center'><table id=t style=';height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 20.0, 10.0]),
                ("d", [20.0, 25.0, 20.0, 10.0]),
                ("r", [5.0, 15.0, 10.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:center'><table id=t style=';height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 66.0]),
                ("cap", [0.0, 0.0, 16.0, 10.0]),
                ("d", [16.0, 28.0, 20.0, 10.0]),
                ("r", [3.0, 13.0, 10.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:center'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 25.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 10.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:center'><table id=t style='align-self:end;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 20.0, 10.0]),
                ("d", [20.0, 25.0, 20.0, 10.0]),
                ("r", [5.0, 15.0, 10.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:center'><table id=t style='align-self:end;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 66.0]),
                ("cap", [0.0, 0.0, 16.0, 10.0]),
                ("d", [16.0, 28.0, 20.0, 10.0]),
                ("r", [3.0, 13.0, 10.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:center'><table id=t style='align-self:start;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 25.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 10.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:center'><table id=t style='align-self:start;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 20.0, 10.0]),
                ("d", [20.0, 25.0, 20.0, 10.0]),
                ("r", [5.0, 15.0, 10.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:center'><table id=t style='align-self:start;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 66.0]),
                ("cap", [0.0, 0.0, 16.0, 10.0]),
                ("d", [16.0, 28.0, 20.0, 10.0]),
                ("r", [3.0, 13.0, 10.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:center'><table id=t style='align-self:center;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 25.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 10.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:center'><table id=t style='align-self:center;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 20.0, 10.0]),
                ("d", [20.0, 25.0, 20.0, 10.0]),
                ("r", [5.0, 15.0, 10.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:center'><table id=t style='align-self:center;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 66.0]),
                ("cap", [0.0, 0.0, 16.0, 10.0]),
                ("d", [16.0, 28.0, 20.0, 10.0]),
                ("r", [3.0, 13.0, 10.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-wrap:wrap;width:300px'><table id=t style=';height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 300.0, 60.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 60.0]),
                ("r", [0.0, 10.0, 10.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-wrap:wrap;width:300px'><table id=t style=';height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 300.0, 60.0]),
                ("cap", [0.0, 0.0, 20.0, 10.0]),
                ("d", [20.0, 0.0, 20.0, 60.0]),
                ("r", [5.0, 15.0, 10.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-wrap:wrap;width:300px'><table id=t style=';height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 300.0, 66.0]),
                ("cap", [0.0, 0.0, 16.0, 10.0]),
                ("d", [16.0, 0.0, 20.0, 66.0]),
                ("r", [3.0, 13.0, 10.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-wrap:wrap;width:300px'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 300.0, 60.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 60.0]),
                ("r", [0.0, 10.0, 10.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-wrap:wrap;width:300px'><table id=t style='align-self:end;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 300.0, 60.0]),
                ("cap", [0.0, 0.0, 20.0, 10.0]),
                ("d", [20.0, 0.0, 20.0, 60.0]),
                ("r", [5.0, 15.0, 10.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-wrap:wrap;width:300px'><table id=t style='align-self:end;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 300.0, 66.0]),
                ("cap", [0.0, 0.0, 16.0, 10.0]),
                ("d", [16.0, 0.0, 20.0, 66.0]),
                ("r", [3.0, 13.0, 10.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-wrap:wrap;width:300px'><table id=t style='align-self:start;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 300.0, 60.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 60.0]),
                ("r", [0.0, 10.0, 10.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-wrap:wrap;width:300px'><table id=t style='align-self:start;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 300.0, 60.0]),
                ("cap", [0.0, 0.0, 20.0, 10.0]),
                ("d", [20.0, 0.0, 20.0, 60.0]),
                ("r", [5.0, 15.0, 10.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-wrap:wrap;width:300px'><table id=t style='align-self:start;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 300.0, 66.0]),
                ("cap", [0.0, 0.0, 16.0, 10.0]),
                ("d", [16.0, 0.0, 20.0, 66.0]),
                ("r", [3.0, 13.0, 10.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-wrap:wrap;width:300px'><table id=t style='align-self:center;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 300.0, 60.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 60.0]),
                ("r", [0.0, 10.0, 10.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-wrap:wrap;width:300px'><table id=t style='align-self:center;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 300.0, 60.0]),
                ("cap", [0.0, 0.0, 20.0, 10.0]),
                ("d", [20.0, 0.0, 20.0, 60.0]),
                ("r", [5.0, 15.0, 10.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;flex-wrap:wrap;width:300px'><table id=t style='align-self:center;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 300.0, 66.0]),
                ("cap", [0.0, 0.0, 16.0, 10.0]),
                ("d", [16.0, 0.0, 20.0, 66.0]),
                ("r", [3.0, 13.0, 10.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:100px'><table id=t style=';height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 20.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 100.0]),
                ("r", [0.0, 30.0, 10.0, 50.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:100px'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 60.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 100.0]),
                ("r", [0.0, 70.0, 10.0, 50.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;direction:rtl'><table id=t style=';height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 60.0]),
                ("cap", [790.0, 20.0, 10.0, 10.0]),
                ("d", [770.0, 20.0, 20.0, 60.0]),
                ("r", [790.0, 30.0, 10.0, 50.0]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;direction:rtl'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 60.0]),
                ("cap", [790.0, 20.0, 10.0, 10.0]),
                ("d", [770.0, 20.0, 20.0, 60.0]),
                ("r", [790.0, 30.0, 10.0, 50.0]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:inline-flex'><table id=t style=';height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 30.0, 60.0]),
                ("cap", [0.0, 20.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 60.0]),
                ("r", [0.0, 30.0, 10.0, 50.0]),
                ("z", [0.0, 82.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:inline-flex'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 30.0, 60.0]),
                ("cap", [0.0, 20.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 60.0]),
                ("r", [0.0, 30.0, 10.0, 50.0]),
                ("z", [0.0, 82.0, 800.0, 10.0]),
            ],
        ),
    ]);
}

/// A table item taller than a flex line or a grid track of a definite height
/// keeps its stated height and its captions, and its alignment lets it overflow
/// the line.
#[test]
fn a_table_that_overflows_a_shorter_line_keeps_its_captions() {
    assert_cases(&[
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:55px'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 55.0]),
                ("cap", [0.0, 15.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 55.0]),
                ("r", [0.0, 25.0, 10.0, 50.0]),
                ("z", [0.0, 75.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:55px'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap >aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 55.0]),
                ("cap", [0.0, 5.0, 20.0, 20.0]),
                ("d", [20.0, 20.0, 20.0, 55.0]),
                ("r", [0.0, 25.0, 20.0, 50.0]),
                ("z", [0.0, 75.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:55px'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 55.0]),
                ("cap", [0.0, 65.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 55.0]),
                ("r", [0.0, 15.0, 10.0, 50.0]),
                ("z", [0.0, 75.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:55px'><table id=t style='align-self:end;min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 55.0]),
                ("cap", [0.0, 15.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 55.0]),
                ("r", [0.0, 25.0, 10.0, 50.0]),
                ("z", [0.0, 75.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:55px'><table id=t style='align-self:end;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 55.0]),
                ("cap", [0.0, 15.0, 20.0, 10.0]),
                ("d", [20.0, 20.0, 20.0, 55.0]),
                ("r", [5.0, 30.0, 10.0, 40.0]),
                ("z", [0.0, 75.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:55px'><table id=t style='align-self:end;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 55.0]),
                ("cap", [0.0, 9.0, 16.0, 10.0]),
                ("d", [16.0, 20.0, 20.0, 55.0]),
                ("r", [3.0, 22.0, 10.0, 50.0]),
                ("z", [0.0, 75.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:55px'><table id=t style='align-self:center;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 55.0]),
                ("cap", [0.0, 17.5, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 55.0]),
                ("r", [0.0, 27.5, 10.0, 50.0]),
                ("z", [0.0, 75.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:55px'><table id=t style='align-self:center;min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 55.0]),
                ("cap", [0.0, 17.5, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 55.0]),
                ("r", [0.0, 27.5, 10.0, 50.0]),
                ("z", [0.0, 75.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:55px'><table id=t style='align-self:center;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 55.0]),
                ("cap", [0.0, 17.5, 20.0, 10.0]),
                ("d", [20.0, 20.0, 20.0, 55.0]),
                ("r", [5.0, 32.5, 10.0, 40.0]),
                ("z", [0.0, 75.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:55px'><table id=t style='align-self:center;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 55.0]),
                ("cap", [0.0, 14.5, 16.0, 10.0]),
                ("d", [16.0, 20.0, 20.0, 55.0]),
                ("r", [3.0, 27.5, 10.0, 50.0]),
                ("z", [0.0, 75.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:55px'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 65.0]),
                ("cap", [0.0, 15.0, 800.0, 10.0]),
                ("d", [0.0, 75.0, 20.0, 10.0]),
                ("r", [0.0, 25.0, 800.0, 50.0]),
                ("z", [0.0, 85.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:55px'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap >aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 65.0]),
                ("cap", [0.0, 15.0, 800.0, 10.0]),
                ("d", [0.0, 75.0, 20.0, 10.0]),
                ("r", [0.0, 25.0, 800.0, 50.0]),
                ("z", [0.0, 85.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:55px'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 65.0]),
                ("cap", [0.0, 65.0, 800.0, 10.0]),
                ("d", [0.0, 75.0, 20.0, 10.0]),
                ("r", [0.0, 15.0, 800.0, 50.0]),
                ("z", [0.0, 85.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:55px'><table id=t style='align-self:end;min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 65.0]),
                ("cap", [0.0, 15.0, 800.0, 10.0]),
                ("d", [0.0, 75.0, 20.0, 10.0]),
                ("r", [0.0, 25.0, 800.0, 50.0]),
                ("z", [0.0, 85.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:55px'><table id=t style='align-self:end;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 65.0]),
                ("cap", [0.0, 15.0, 800.0, 10.0]),
                ("d", [0.0, 75.0, 20.0, 10.0]),
                ("r", [5.0, 30.0, 790.0, 40.0]),
                ("z", [0.0, 85.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:55px'><table id=t style='align-self:end;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 65.0]),
                ("cap", [0.0, 9.0, 800.0, 10.0]),
                ("d", [0.0, 75.0, 20.0, 10.0]),
                ("r", [3.0, 22.0, 794.0, 50.0]),
                ("z", [0.0, 85.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:55px'><table id=t style='align-self:center;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 65.0]),
                ("cap", [0.0, 17.5, 800.0, 10.0]),
                ("d", [0.0, 75.0, 20.0, 10.0]),
                ("r", [0.0, 27.5, 800.0, 50.0]),
                ("z", [0.0, 85.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:55px'><table id=t style='align-self:center;min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 65.0]),
                ("cap", [0.0, 17.5, 800.0, 10.0]),
                ("d", [0.0, 75.0, 20.0, 10.0]),
                ("r", [0.0, 27.5, 800.0, 50.0]),
                ("z", [0.0, 85.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:55px'><table id=t style='align-self:center;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 65.0]),
                ("cap", [0.0, 17.5, 800.0, 10.0]),
                ("d", [0.0, 75.0, 20.0, 10.0]),
                ("r", [5.0, 32.5, 790.0, 40.0]),
                ("z", [0.0, 85.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:55px'><table id=t style='align-self:center;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 65.0]),
                ("cap", [0.0, 14.5, 800.0, 10.0]),
                ("d", [0.0, 75.0, 20.0, 10.0]),
                ("r", [3.0, 27.5, 794.0, 50.0]),
                ("z", [0.0, 85.0, 800.0, 10.0]),
            ],
        ),
    ]);
}

/// An auto track of a grid holds the whole wrapper box of a table item that
/// states its height, whether the table has rows, and a fixed track aligns that
/// box in it.
#[test]
fn a_grid_track_holds_the_stated_height_of_a_table_item_and_its_captions() {
    assert_cases(&[
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 70.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 80.0, 20.0, 10.0]),
                ("z", [0.0, 90.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid'><table id=t style='height:50px;border-spacing:0'></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 60.0]),
                ("d", [0.0, 70.0, 20.0, 10.0]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid'><table id=t style=';border-spacing:0'><caption id=cap style='height:10px'></caption></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 20.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 30.0, 20.0, 10.0]),
                ("z", [0.0, 40.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid'><table id=t style=';border-spacing:0'></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 20.0, 20.0, 10.0]),
                ("z", [0.0, 30.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid'><table id=t style='min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 70.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 80.0, 20.0, 10.0]),
                ("z", [0.0, 90.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid'><table id=t style='min-height:50px;border-spacing:0'></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 60.0]),
                ("d", [0.0, 70.0, 20.0, 10.0]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style='height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style='height:50px;border-spacing:0'></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style=';border-spacing:0'><caption id=cap style='height:10px'></caption></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style=';border-spacing:0'></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style='min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style='min-height:50px;border-spacing:0'></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style=';height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style=';height:50px;border-spacing:0'><caption id=cap >aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style=';height:50px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 50.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [0.0, 0.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style=';height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 40.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style=';height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 76.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 66.0, 20.0, 10.0]),
                ("r", [3.0, 13.0, 794.0, 50.0]),
                ("z", [0.0, 76.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap >aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 50.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [0.0, 0.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style='align-self:end;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 40.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style='align-self:end;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 76.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 66.0, 20.0, 10.0]),
                ("r", [3.0, 13.0, 794.0, 50.0]),
                ("z", [0.0, 76.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style='align-self:start;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style='align-self:start;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 40.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style='align-self:start;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 76.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 66.0, 20.0, 10.0]),
                ("r", [3.0, 13.0, 794.0, 50.0]),
                ("z", [0.0, 76.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style='align-self:stretch;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style='align-self:stretch;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 40.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style='align-self:stretch;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 76.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 66.0, 20.0, 10.0]),
                ("r", [3.0, 13.0, 794.0, 50.0]),
                ("z", [0.0, 76.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style='align-self:center;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style='align-self:center;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 40.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style='align-self:center;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 76.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 66.0, 20.0, 10.0]),
                ("r", [3.0, 13.0, 794.0, 50.0]),
                ("z", [0.0, 76.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-columns:auto auto'><table id=t style=';height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 395.0, 10.0]),
                ("d", [395.0, 0.0, 20.0, 60.0]),
                ("r", [0.0, 10.0, 395.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-columns:auto auto'><table id=t style=';height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 400.0, 10.0]),
                ("d", [400.0, 0.0, 20.0, 60.0]),
                ("r", [5.0, 15.0, 390.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-columns:auto auto'><table id=t style=';height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 66.0]),
                ("cap", [0.0, 0.0, 398.0, 10.0]),
                ("d", [398.0, 0.0, 20.0, 66.0]),
                ("r", [3.0, 13.0, 392.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-columns:auto auto'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 395.0, 10.0]),
                ("d", [395.0, 0.0, 20.0, 60.0]),
                ("r", [0.0, 10.0, 395.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-columns:auto auto'><table id=t style='align-self:end;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 400.0, 10.0]),
                ("d", [400.0, 0.0, 20.0, 60.0]),
                ("r", [5.0, 15.0, 390.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-columns:auto auto'><table id=t style='align-self:end;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 66.0]),
                ("cap", [0.0, 0.0, 398.0, 10.0]),
                ("d", [398.0, 0.0, 20.0, 66.0]),
                ("r", [3.0, 13.0, 392.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-columns:auto auto'><table id=t style='align-self:start;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 395.0, 10.0]),
                ("d", [395.0, 0.0, 20.0, 60.0]),
                ("r", [0.0, 10.0, 395.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-columns:auto auto'><table id=t style='align-self:start;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 400.0, 10.0]),
                ("d", [400.0, 0.0, 20.0, 60.0]),
                ("r", [5.0, 15.0, 390.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-columns:auto auto'><table id=t style='align-self:start;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 66.0]),
                ("cap", [0.0, 0.0, 398.0, 10.0]),
                ("d", [398.0, 0.0, 20.0, 66.0]),
                ("r", [3.0, 13.0, 392.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-columns:auto auto'><table id=t style='align-self:stretch;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 395.0, 10.0]),
                ("d", [395.0, 0.0, 20.0, 60.0]),
                ("r", [0.0, 10.0, 395.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-columns:auto auto'><table id=t style='align-self:stretch;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 400.0, 10.0]),
                ("d", [400.0, 0.0, 20.0, 60.0]),
                ("r", [5.0, 15.0, 390.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-columns:auto auto'><table id=t style='align-self:stretch;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 66.0]),
                ("cap", [0.0, 0.0, 398.0, 10.0]),
                ("d", [398.0, 0.0, 20.0, 66.0]),
                ("r", [3.0, 13.0, 392.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-columns:auto auto'><table id=t style='align-self:center;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 395.0, 10.0]),
                ("d", [395.0, 0.0, 20.0, 60.0]),
                ("r", [0.0, 10.0, 395.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-columns:auto auto'><table id=t style='align-self:center;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 400.0, 10.0]),
                ("d", [400.0, 0.0, 20.0, 60.0]),
                ("r", [5.0, 15.0, 390.0, 40.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-columns:auto auto'><table id=t style='align-self:center;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 66.0]),
                ("cap", [0.0, 0.0, 398.0, 10.0]),
                ("d", [398.0, 0.0, 20.0, 66.0]),
                ("r", [3.0, 13.0, 392.0, 50.0]),
                ("z", [0.0, 66.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-rows:80px'><table id=t style=';height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 90.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 80.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 90.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-rows:80px'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 90.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 80.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 800.0, 50.0]),
                ("z", [0.0, 90.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-rows:80px'><table id=t style='align-self:center;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 90.0]),
                ("cap", [0.0, 10.0, 800.0, 10.0]),
                ("d", [0.0, 80.0, 20.0, 10.0]),
                ("r", [0.0, 20.0, 800.0, 50.0]),
                ("z", [0.0, 90.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;align-items:start'><table id=t style=';height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;align-items:start'><table id=t style=';height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 40.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;align-items:start'><table id=t style=';height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 76.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 66.0, 20.0, 10.0]),
                ("r", [3.0, 13.0, 794.0, 50.0]),
                ("z", [0.0, 76.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;align-items:start'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;align-items:start'><table id=t style='align-self:end;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 40.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;align-items:start'><table id=t style='align-self:end;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 76.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 66.0, 20.0, 10.0]),
                ("r", [3.0, 13.0, 794.0, 50.0]),
                ("z", [0.0, 76.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;align-items:start'><table id=t style='align-self:start;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;align-items:start'><table id=t style='align-self:start;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 40.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;align-items:start'><table id=t style='align-self:start;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 76.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 66.0, 20.0, 10.0]),
                ("r", [3.0, 13.0, 794.0, 50.0]),
                ("z", [0.0, 76.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;align-items:start'><table id=t style='align-self:stretch;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;align-items:start'><table id=t style='align-self:stretch;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 40.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;align-items:start'><table id=t style='align-self:stretch;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 76.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 66.0, 20.0, 10.0]),
                ("r", [3.0, 13.0, 794.0, 50.0]),
                ("z", [0.0, 76.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;align-items:start'><table id=t style='align-self:center;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;align-items:start'><table id=t style='align-self:center;height:50px;padding:3px;border:2px solid;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [5.0, 15.0, 790.0, 40.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;align-items:start'><table id=t style='align-self:center;height:50px;box-sizing:content-box;padding:3px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 76.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 66.0, 20.0, 10.0]),
                ("r", [3.0, 13.0, 794.0, 50.0]),
                ("z", [0.0, 76.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;height:100px'><table id=t style=';height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 95.0, 20.0, 25.0]),
                ("r", [0.0, 30.0, 800.0, 50.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;height:100px'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 35.0, 800.0, 10.0]),
                ("d", [0.0, 95.0, 20.0, 25.0]),
                ("r", [0.0, 45.0, 800.0, 50.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style=';height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 800.0, 50.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("cap", [0.0, 60.0, 800.0, 10.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("r", [0.0, 70.0, 800.0, 50.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:inline-grid'><table id=t style=';height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 20.0, 70.0]),
                ("cap", [0.0, 20.0, 20.0, 10.0]),
                ("d", [0.0, 80.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 20.0, 50.0]),
                ("z", [0.0, 90.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:inline-grid'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 20.0, 70.0]),
                ("cap", [0.0, 20.0, 20.0, 10.0]),
                ("d", [0.0, 80.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 20.0, 50.0]),
                ("z", [0.0, 90.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;justify-items:start;grid-template-columns:100px'><table id=t style=';height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 70.0]),
                ("cap", [0.0, 20.0, 10.0, 10.0]),
                ("d", [0.0, 80.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 10.0, 50.0]),
                ("z", [0.0, 90.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;justify-items:start;grid-template-columns:100px'><table id=t style='align-self:end;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 70.0]),
                ("cap", [0.0, 20.0, 10.0, 10.0]),
                ("d", [0.0, 80.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 10.0, 50.0]),
                ("z", [0.0, 90.0, 800.0, 10.0]),
            ],
        ),
    ]);
}

/// A `min-height` or `max-height` bounds the grid box of a table item in a flex
/// row or a grid, with the captions on top, and a stretched table keeps its
/// grid box, the rows that are not collapsed, within its `max-height`; a flex
/// column bounds the whole wrapper box by a `min-height`, as Chromium does.
#[test]
fn the_limits_of_a_table_item_bound_its_grid_box_or_its_wrapper_as_in_chromium() {
    assert_cases(&[
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:100px'><table id=t style='max-height:30px;border-spacing:0'><tr id=r><td style='padding:0'>x</td></tr><tr id=r2 style='visibility:collapse'><td style='padding:0'>y</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("d", [10.0, 20.0, 20.0, 100.0]),
                ("r", [0.0, 20.0, 10.0, 15.0]),
                ("r2", [0.0, 35.0, 10.0, 0.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:100px'><table id=t style='max-height:30px;border-spacing:0'><caption id=cap style=\"height:10px\"></caption><tr id=r><td style='padding:0'>x</td></tr><tr id=r2 style='visibility:collapse'><td style='padding:0'>y</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 20.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 100.0]),
                ("r", [0.0, 30.0, 10.0, 15.0]),
                ("r2", [0.0, 45.0, 10.0, 0.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;height:100px'><table id=t style='max-height:30px;border-spacing:0'><tr id=r><td style='padding:0'>x</td></tr><tr id=r2 style='visibility:collapse'><td style='padding:0'>y</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("d", [0.0, 70.0, 20.0, 50.0]),
                ("r", [0.0, 20.0, 800.0, 15.0]),
                ("r2", [0.0, 35.0, 800.0, 0.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;height:100px'><table id=t style='max-height:30px;border-spacing:0'><caption id=cap style=\"height:10px\"></caption><tr id=r><td style='padding:0'>x</td></tr><tr id=r2 style='visibility:collapse'><td style='padding:0'>y</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 75.0, 20.0, 45.0]),
                ("r", [0.0, 30.0, 800.0, 15.0]),
                ("r2", [0.0, 45.0, 800.0, 0.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style='max-height:30px;border-spacing:0'><tr id=r><td style='padding:0'>x</td></tr><tr id=r2 style='visibility:collapse'><td style='padding:0'>y</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("r", [0.0, 20.0, 800.0, 15.0]),
                ("r2", [0.0, 35.0, 800.0, 0.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style='max-height:30px;border-spacing:0'><caption id=cap style=\"height:10px\"></caption><tr id=r><td style='padding:0'>x</td></tr><tr id=r2 style='visibility:collapse'><td style='padding:0'>y</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 800.0, 15.0]),
                ("r2", [0.0, 45.0, 800.0, 0.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style=';min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 60.0]),
                ("r", [0.0, 10.0, 10.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style='align-self:end;min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 60.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 60.0]),
                ("r", [0.0, 10.0, 10.0, 50.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;height:55px'><table id=t style=';min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 55.0]),
                ("r", [0.0, 10.0, 10.0, 50.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style=';min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style='align-self:end;min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 70.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 60.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 70.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-rows:55px'><table id=t style=';min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 65.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 55.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 65.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:100px'><table id=t style=';min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 20.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 100.0]),
                ("r", [0.0, 30.0, 10.0, 90.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:100px'><table id=t style=';min-height:50px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 110.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 100.0]),
                ("r", [0.0, 20.0, 10.0, 90.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:100px'><table id=t style=';max-height:30px;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 20.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 100.0]),
                ("r", [0.0, 30.0, 10.0, 30.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:100px'><table id=t style=';max-height:30px;height:50px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 50.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 100.0]),
                ("r", [0.0, 20.0, 10.0, 30.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:100px'><table id=t style=';max-height:30px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 20.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 100.0]),
                ("r", [0.0, 30.0, 10.0, 30.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:100px'><table id=t style=';max-height:30px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 50.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 100.0]),
                ("r", [0.0, 20.0, 10.0, 30.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:100px'><table id=t style=';min-height:50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 20.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 100.0]),
                ("r", [0.0, 30.0, 10.0, 90.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:100px'><table id=t style=';min-height:50%;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 110.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 100.0]),
                ("r", [0.0, 20.0, 10.0, 90.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:100px'><table id=t style='align-self:end;min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 60.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 100.0]),
                ("r", [0.0, 70.0, 10.0, 50.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:100px'><table id=t style='align-self:end;max-height:30px;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 80.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 100.0]),
                ("r", [0.0, 90.0, 10.0, 30.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:100px'><table id=t style='align-self:end;max-height:30px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 100.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 100.0]),
                ("r", [0.0, 110.0, 10.0, 10.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:100px'><table id=t style='align-self:end;min-height:50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 60.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 100.0]),
                ("r", [0.0, 70.0, 10.0, 50.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;height:100px'><table id=t style=';min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 95.0, 20.0, 25.0]),
                ("r", [0.0, 30.0, 800.0, 65.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;height:100px'><table id=t style=';min-height:50px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 85.0, 800.0, 10.0]),
                ("d", [0.0, 95.0, 20.0, 25.0]),
                ("r", [0.0, 20.0, 800.0, 65.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;height:100px'><table id=t style=';max-height:30px;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 85.0, 20.0, 35.0]),
                ("r", [0.0, 30.0, 800.0, 30.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;height:100px'><table id=t style=';max-height:30px;height:50px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 50.0, 800.0, 10.0]),
                ("d", [0.0, 85.0, 20.0, 35.0]),
                ("r", [0.0, 20.0, 800.0, 30.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;height:100px'><table id=t style=';max-height:30px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 75.0, 20.0, 45.0]),
                ("r", [0.0, 30.0, 800.0, 30.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;height:100px'><table id=t style=';max-height:30px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 50.0, 800.0, 10.0]),
                ("d", [0.0, 75.0, 20.0, 45.0]),
                ("r", [0.0, 20.0, 800.0, 30.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;height:100px'><table id=t style=';min-height:50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 75.0, 20.0, 45.0]),
                ("r", [0.0, 30.0, 800.0, 45.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;height:100px'><table id=t style=';min-height:50%;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 65.0, 800.0, 10.0]),
                ("d", [0.0, 75.0, 20.0, 45.0]),
                ("r", [0.0, 20.0, 800.0, 45.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;height:100px'><table id=t style='align-self:end;min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 35.0, 800.0, 10.0]),
                ("d", [0.0, 95.0, 20.0, 25.0]),
                ("r", [0.0, 45.0, 800.0, 50.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;height:100px'><table id=t style='align-self:end;max-height:30px;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 45.0, 800.0, 10.0]),
                ("d", [0.0, 85.0, 20.0, 35.0]),
                ("r", [0.0, 55.0, 800.0, 30.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;height:100px'><table id=t style='align-self:end;max-height:30px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 55.0, 800.0, 10.0]),
                ("d", [0.0, 75.0, 20.0, 45.0]),
                ("r", [0.0, 65.0, 800.0, 10.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;height:100px'><table id=t style='align-self:end;min-height:50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 37.5, 800.0, 10.0]),
                ("d", [0.0, 75.0, 20.0, 45.0]),
                ("r", [0.0, 47.5, 800.0, 27.5]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style=';min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 800.0, 90.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style=';min-height:50px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("cap", [0.0, 110.0, 800.0, 10.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("r", [0.0, 20.0, 800.0, 90.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style=';max-height:30px;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 800.0, 30.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style=';max-height:30px;height:50px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("cap", [0.0, 50.0, 800.0, 10.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("r", [0.0, 20.0, 800.0, 30.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style=';max-height:30px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 800.0, 30.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style=';max-height:30px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("cap", [0.0, 50.0, 800.0, 10.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("r", [0.0, 20.0, 800.0, 30.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style=';min-height:50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 800.0, 90.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style=';min-height:50%;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("cap", [0.0, 110.0, 800.0, 10.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("r", [0.0, 20.0, 800.0, 90.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style='align-self:end;min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("cap", [0.0, 60.0, 800.0, 10.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("r", [0.0, 70.0, 800.0, 50.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style='align-self:end;max-height:30px;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("cap", [0.0, 80.0, 800.0, 10.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("r", [0.0, 90.0, 800.0, 30.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style='align-self:end;max-height:30px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("cap", [0.0, 100.0, 800.0, 10.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("r", [0.0, 110.0, 800.0, 10.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style='align-self:end;min-height:50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("cap", [0.0, 60.0, 800.0, 10.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("r", [0.0, 70.0, 800.0, 50.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;align-items:start;width:200px'><table id=t style=';min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 60.0]),
                ("cap", [0.0, 20.0, 10.0, 10.0]),
                ("d", [0.0, 70.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 10.0, 40.0]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;align-items:start;width:200px'><table id=t style=';min-height:50px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 60.0]),
                ("cap", [0.0, 60.0, 10.0, 10.0]),
                ("d", [0.0, 70.0, 20.0, 10.0]),
                ("r", [0.0, 20.0, 10.0, 40.0]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;align-items:start;width:200px'><table id=t style=';max-height:30px;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 40.0]),
                ("cap", [0.0, 20.0, 10.0, 10.0]),
                ("d", [0.0, 50.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 10.0, 20.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;align-items:start;width:200px'><table id=t style=';max-height:30px;height:50px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 40.0]),
                ("cap", [0.0, 40.0, 10.0, 10.0]),
                ("d", [0.0, 50.0, 20.0, 10.0]),
                ("r", [0.0, 20.0, 10.0, 20.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;align-items:start;width:200px'><table id=t style=';max-height:30px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 30.0]),
                ("cap", [0.0, 20.0, 10.0, 10.0]),
                ("d", [0.0, 40.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 10.0, 10.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;align-items:start;width:200px'><table id=t style=';max-height:30px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 30.0]),
                ("cap", [0.0, 30.0, 10.0, 10.0]),
                ("d", [0.0, 40.0, 20.0, 10.0]),
                ("r", [0.0, 20.0, 10.0, 10.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;align-items:start;width:200px'><table id=t style=';min-height:50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 30.0]),
                ("cap", [0.0, 20.0, 10.0, 10.0]),
                ("d", [0.0, 40.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 10.0, 10.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;align-items:start;width:200px'><table id=t style=';min-height:50%;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 30.0]),
                ("cap", [0.0, 30.0, 10.0, 10.0]),
                ("d", [0.0, 40.0, 20.0, 10.0]),
                ("r", [0.0, 20.0, 10.0, 10.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;align-items:start;width:200px'><table id=t style='align-self:end;min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 60.0]),
                ("cap", [190.0, 20.0, 10.0, 10.0]),
                ("d", [0.0, 70.0, 20.0, 10.0]),
                ("r", [190.0, 30.0, 10.0, 40.0]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;align-items:start;width:200px'><table id=t style='align-self:end;max-height:30px;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 40.0]),
                ("cap", [190.0, 20.0, 10.0, 10.0]),
                ("d", [0.0, 50.0, 20.0, 10.0]),
                ("r", [190.0, 30.0, 10.0, 20.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;align-items:start;width:200px'><table id=t style='align-self:end;max-height:30px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 30.0]),
                ("cap", [190.0, 20.0, 10.0, 10.0]),
                ("d", [0.0, 40.0, 20.0, 10.0]),
                ("r", [190.0, 30.0, 10.0, 10.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;align-items:start;width:200px'><table id=t style='align-self:end;min-height:50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 30.0]),
                ("cap", [190.0, 20.0, 10.0, 10.0]),
                ("d", [0.0, 40.0, 20.0, 10.0]),
                ("r", [190.0, 30.0, 10.0, 10.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px'><table id=t style=';min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 60.0]),
                ("cap", [0.0, 20.0, 200.0, 10.0]),
                ("d", [0.0, 70.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 200.0, 40.0]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px'><table id=t style=';min-height:50px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 60.0]),
                ("cap", [0.0, 60.0, 200.0, 10.0]),
                ("d", [0.0, 70.0, 20.0, 10.0]),
                ("r", [0.0, 20.0, 200.0, 40.0]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px'><table id=t style=';max-height:30px;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 40.0]),
                ("cap", [0.0, 20.0, 200.0, 10.0]),
                ("d", [0.0, 50.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 200.0, 20.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px'><table id=t style=';max-height:30px;height:50px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 40.0]),
                ("cap", [0.0, 40.0, 200.0, 10.0]),
                ("d", [0.0, 50.0, 20.0, 10.0]),
                ("r", [0.0, 20.0, 200.0, 20.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px'><table id=t style=';max-height:30px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 30.0]),
                ("cap", [0.0, 20.0, 200.0, 10.0]),
                ("d", [0.0, 40.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 200.0, 10.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px'><table id=t style=';max-height:30px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 30.0]),
                ("cap", [0.0, 30.0, 200.0, 10.0]),
                ("d", [0.0, 40.0, 20.0, 10.0]),
                ("r", [0.0, 20.0, 200.0, 10.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px'><table id=t style=';min-height:50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 30.0]),
                ("cap", [0.0, 20.0, 200.0, 10.0]),
                ("d", [0.0, 40.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 200.0, 10.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px'><table id=t style=';min-height:50%;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 30.0]),
                ("cap", [0.0, 30.0, 200.0, 10.0]),
                ("d", [0.0, 40.0, 20.0, 10.0]),
                ("r", [0.0, 20.0, 200.0, 10.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px'><table id=t style='align-self:end;min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 60.0]),
                ("cap", [190.0, 20.0, 10.0, 10.0]),
                ("d", [0.0, 70.0, 20.0, 10.0]),
                ("r", [190.0, 30.0, 10.0, 40.0]),
                ("z", [0.0, 80.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px'><table id=t style='align-self:end;max-height:30px;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 40.0]),
                ("cap", [190.0, 20.0, 10.0, 10.0]),
                ("d", [0.0, 50.0, 20.0, 10.0]),
                ("r", [190.0, 30.0, 10.0, 20.0]),
                ("z", [0.0, 60.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px'><table id=t style='align-self:end;max-height:30px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 30.0]),
                ("cap", [190.0, 20.0, 10.0, 10.0]),
                ("d", [0.0, 40.0, 20.0, 10.0]),
                ("r", [190.0, 30.0, 10.0, 10.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px'><table id=t style='align-self:end;min-height:50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 30.0]),
                ("cap", [190.0, 20.0, 10.0, 10.0]),
                ("d", [0.0, 40.0, 20.0, 10.0]),
                ("r", [190.0, 30.0, 10.0, 10.0]),
                ("z", [0.0, 50.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px;height:100px'><table id=t style=';min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 100.0]),
                ("cap", [0.0, 20.0, 200.0, 10.0]),
                ("d", [0.0, 70.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 200.0, 40.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px;height:100px'><table id=t style=';min-height:50px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 100.0]),
                ("cap", [0.0, 60.0, 200.0, 10.0]),
                ("d", [0.0, 70.0, 20.0, 10.0]),
                ("r", [0.0, 20.0, 200.0, 40.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px;height:100px'><table id=t style=';max-height:30px;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 100.0]),
                ("cap", [0.0, 20.0, 200.0, 10.0]),
                ("d", [0.0, 50.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 200.0, 20.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px;height:100px'><table id=t style=';max-height:30px;height:50px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 100.0]),
                ("cap", [0.0, 40.0, 200.0, 10.0]),
                ("d", [0.0, 50.0, 20.0, 10.0]),
                ("r", [0.0, 20.0, 200.0, 20.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px;height:100px'><table id=t style=';max-height:30px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 100.0]),
                ("cap", [0.0, 20.0, 200.0, 10.0]),
                ("d", [0.0, 40.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 200.0, 10.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px;height:100px'><table id=t style=';max-height:30px;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 100.0]),
                ("cap", [0.0, 30.0, 200.0, 10.0]),
                ("d", [0.0, 40.0, 20.0, 10.0]),
                ("r", [0.0, 20.0, 200.0, 10.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px;height:100px'><table id=t style=';min-height:50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 100.0]),
                ("cap", [0.0, 20.0, 200.0, 10.0]),
                ("d", [0.0, 70.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 200.0, 40.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px;height:100px'><table id=t style=';min-height:50%;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 100.0]),
                ("cap", [0.0, 60.0, 200.0, 10.0]),
                ("d", [0.0, 70.0, 20.0, 10.0]),
                ("r", [0.0, 20.0, 200.0, 40.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px;height:100px'><table id=t style='align-self:end;min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 100.0]),
                ("cap", [190.0, 20.0, 10.0, 10.0]),
                ("d", [0.0, 70.0, 20.0, 10.0]),
                ("r", [190.0, 30.0, 10.0, 40.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px;height:100px'><table id=t style='align-self:end;max-height:30px;height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 100.0]),
                ("cap", [190.0, 20.0, 10.0, 10.0]),
                ("d", [0.0, 50.0, 20.0, 10.0]),
                ("r", [190.0, 30.0, 10.0, 20.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px;height:100px'><table id=t style='align-self:end;max-height:30px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 100.0]),
                ("cap", [190.0, 20.0, 10.0, 10.0]),
                ("d", [0.0, 40.0, 20.0, 10.0]),
                ("r", [190.0, 30.0, 10.0, 10.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px;height:100px'><table id=t style='align-self:end;min-height:50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 100.0]),
                ("cap", [190.0, 20.0, 10.0, 10.0]),
                ("d", [0.0, 70.0, 20.0, 10.0]),
                ("r", [190.0, 30.0, 10.0, 40.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;height:55px'><table id=t style='min-height:50px;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 50.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 50.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
    ]);
}

/// A percentage, `calc()`, `stretch` or `fit-content` height of a table item
/// sizes its grid box, and the captions come on top of it.
#[test]
fn percentages_and_keywords_of_the_height_resolve_for_the_grid_box() {
    assert_cases(&[
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:100px'><table id=t style=';height:50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 20.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 100.0]),
                ("r", [0.0, 30.0, 10.0, 50.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:100px'><table id=t style=';height:calc(20% + 20px);border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 20.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 100.0]),
                ("r", [0.0, 30.0, 10.0, 40.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:100px'><table id=t style=';height:stretch;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 20.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 100.0]),
                ("r", [0.0, 30.0, 10.0, 90.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;height:100px'><table id=t style=';height:fit-content;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 20.0, 10.0, 10.0]),
                ("d", [10.0, 20.0, 20.0, 100.0]),
                ("r", [0.0, 30.0, 10.0, 10.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;height:100px'><table id=t style=';height:50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 75.0, 20.0, 45.0]),
                ("r", [0.0, 30.0, 800.0, 27.5]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;height:100px'><table id=t style=';height:calc(20% + 20px);border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 75.0, 20.0, 45.0]),
                ("r", [0.0, 30.0, 800.0, 31.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;height:100px'><table id=t style=';height:stretch;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 75.0, 20.0, 45.0]),
                ("r", [0.0, 30.0, 800.0, 45.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;height:100px'><table id=t style=';height:fit-content;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 100.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 75.0, 20.0, 45.0]),
                ("r", [0.0, 30.0, 800.0, 10.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style=';height:50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 800.0, 50.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style=';height:calc(20% + 20px);border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 800.0, 40.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style=';height:stretch;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 800.0, 90.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:grid;grid-template-rows:100px'><table id=t style=';height:fit-content;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 800.0, 110.0]),
                ("cap", [0.0, 20.0, 800.0, 10.0]),
                ("d", [0.0, 120.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 800.0, 10.0]),
                ("z", [0.0, 130.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px;height:100px'><table id=t style=';height:50%;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 100.0]),
                ("cap", [0.0, 20.0, 200.0, 10.0]),
                ("d", [0.0, 80.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 200.0, 50.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px;height:100px'><table id=t style=';height:calc(20% + 20px);border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 100.0]),
                ("cap", [0.0, 20.0, 200.0, 10.0]),
                ("d", [0.0, 70.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 200.0, 40.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px;height:100px'><table id=t style=';height:stretch;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 100.0]),
                ("cap", [0.0, 20.0, 200.0, 10.0]),
                ("d", [0.0, 110.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 200.0, 80.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=pad style='height:20px'></div><div id=c style='display:flex;flex-direction:column;width:200px;height:100px'><table id=t style=';height:fit-content;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 20.0, 200.0, 100.0]),
                ("cap", [0.0, 20.0, 200.0, 10.0]),
                ("d", [0.0, 40.0, 20.0, 10.0]),
                ("r", [0.0, 30.0, 200.0, 10.0]),
                ("z", [0.0, 120.0, 800.0, 10.0]),
            ],
        ),
    ]);
}

/// A table item with an `auto` height is measured as before: the line or track
/// holds its rows and captions.
#[test]
fn tables_with_an_auto_height_are_left_as_they_were() {
    assert_cases(&[
        (
            "<div id=c style='display:flex'><table id=t style=';;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 20.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 20.0]),
                ("r", [0.0, 10.0, 10.0, 10.0]),
                ("z", [0.0, 20.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style=';;border-spacing:0'><caption id=cap >aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 30.0]),
                ("cap", [0.0, 0.0, 20.0, 20.0]),
                ("d", [20.0, 0.0, 20.0, 30.0]),
                ("r", [0.0, 20.0, 20.0, 10.0]),
                ("z", [0.0, 30.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style=';;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 20.0]),
                ("cap", [0.0, 10.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 20.0]),
                ("r", [0.0, 0.0, 10.0, 10.0]),
                ("z", [0.0, 20.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style='align-self:end;;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 20.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 20.0]),
                ("r", [0.0, 10.0, 10.0, 10.0]),
                ("z", [0.0, 20.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style='align-self:end;;border-spacing:0'><caption id=cap >aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 30.0]),
                ("cap", [0.0, 0.0, 20.0, 20.0]),
                ("d", [20.0, 0.0, 20.0, 30.0]),
                ("r", [0.0, 20.0, 20.0, 10.0]),
                ("z", [0.0, 30.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex'><table id=t style='align-self:end;;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 20.0]),
                ("cap", [0.0, 10.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 20.0]),
                ("r", [0.0, 0.0, 10.0, 10.0]),
                ("z", [0.0, 20.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;height:55px'><table id=t style=';;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 55.0]),
                ("r", [0.0, 10.0, 10.0, 45.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;height:55px'><table id=t style=';;border-spacing:0'><caption id=cap >aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 0.0, 20.0, 20.0]),
                ("d", [20.0, 0.0, 20.0, 55.0]),
                ("r", [0.0, 20.0, 20.0, 35.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;height:55px'><table id=t style=';;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 45.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 55.0]),
                ("r", [0.0, 0.0, 10.0, 45.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;height:55px'><table id=t style='align-self:end;;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 35.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 55.0]),
                ("r", [0.0, 45.0, 10.0, 10.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;height:55px'><table id=t style='align-self:end;;border-spacing:0'><caption id=cap >aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 25.0, 20.0, 20.0]),
                ("d", [20.0, 0.0, 20.0, 55.0]),
                ("r", [0.0, 45.0, 20.0, 10.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;height:55px'><table id=t style='align-self:end;;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 55.0]),
                ("cap", [0.0, 45.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 55.0]),
                ("r", [0.0, 35.0, 10.0, 10.0]),
                ("z", [0.0, 55.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:start'><table id=t style=';;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 20.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 10.0, 10.0]),
                ("z", [0.0, 20.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:start'><table id=t style=';;border-spacing:0'><caption id=cap >aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 30.0]),
                ("cap", [0.0, 0.0, 20.0, 20.0]),
                ("d", [20.0, 0.0, 20.0, 10.0]),
                ("r", [0.0, 20.0, 20.0, 10.0]),
                ("z", [0.0, 30.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:start'><table id=t style=';;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 20.0]),
                ("cap", [0.0, 10.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 10.0]),
                ("r", [0.0, 0.0, 10.0, 10.0]),
                ("z", [0.0, 20.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:start'><table id=t style='align-self:end;;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 20.0]),
                ("cap", [0.0, 0.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 10.0, 10.0]),
                ("z", [0.0, 20.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:start'><table id=t style='align-self:end;;border-spacing:0'><caption id=cap >aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 30.0]),
                ("cap", [0.0, 0.0, 20.0, 20.0]),
                ("d", [20.0, 0.0, 20.0, 10.0]),
                ("r", [0.0, 20.0, 20.0, 10.0]),
                ("z", [0.0, 30.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:flex;align-items:start'><table id=t style='align-self:end;;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 20.0]),
                ("cap", [0.0, 10.0, 10.0, 10.0]),
                ("d", [10.0, 0.0, 20.0, 10.0]),
                ("r", [0.0, 0.0, 10.0, 10.0]),
                ("z", [0.0, 20.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style=';;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 30.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 20.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 10.0]),
                ("z", [0.0, 30.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style=';;border-spacing:0'><caption id=cap >aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 30.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 20.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 10.0]),
                ("z", [0.0, 30.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style=';;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 30.0]),
                ("cap", [0.0, 10.0, 800.0, 10.0]),
                ("d", [0.0, 20.0, 20.0, 10.0]),
                ("r", [0.0, 0.0, 800.0, 10.0]),
                ("z", [0.0, 30.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style='align-self:end;;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 30.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 20.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 10.0]),
                ("z", [0.0, 30.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style='align-self:end;;border-spacing:0'><caption id=cap >aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 30.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 20.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 10.0]),
                ("z", [0.0, 30.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid'><table id=t style='align-self:end;;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 30.0]),
                ("cap", [0.0, 10.0, 800.0, 10.0]),
                ("d", [0.0, 20.0, 20.0, 10.0]),
                ("r", [0.0, 0.0, 800.0, 10.0]),
                ("z", [0.0, 30.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-rows:55px'><table id=t style=';;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 65.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 55.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 45.0]),
                ("z", [0.0, 65.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-rows:55px'><table id=t style=';;border-spacing:0'><caption id=cap >aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 65.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 55.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 45.0]),
                ("z", [0.0, 65.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-rows:55px'><table id=t style=';;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 65.0]),
                ("cap", [0.0, 45.0, 800.0, 10.0]),
                ("d", [0.0, 55.0, 20.0, 10.0]),
                ("r", [0.0, 0.0, 800.0, 45.0]),
                ("z", [0.0, 65.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-rows:55px'><table id=t style='align-self:end;;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 65.0]),
                ("cap", [0.0, 35.0, 800.0, 10.0]),
                ("d", [0.0, 55.0, 20.0, 10.0]),
                ("r", [0.0, 45.0, 800.0, 10.0]),
                ("z", [0.0, 65.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-rows:55px'><table id=t style='align-self:end;;border-spacing:0'><caption id=cap >aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 65.0]),
                ("cap", [0.0, 35.0, 800.0, 10.0]),
                ("d", [0.0, 55.0, 20.0, 10.0]),
                ("r", [0.0, 45.0, 800.0, 10.0]),
                ("z", [0.0, 65.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;grid-template-rows:55px'><table id=t style='align-self:end;;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 65.0]),
                ("cap", [0.0, 45.0, 800.0, 10.0]),
                ("d", [0.0, 55.0, 20.0, 10.0]),
                ("r", [0.0, 35.0, 800.0, 10.0]),
                ("z", [0.0, 65.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;align-items:start'><table id=t style=';;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 30.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 20.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 10.0]),
                ("z", [0.0, 30.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;align-items:start'><table id=t style=';;border-spacing:0'><caption id=cap >aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 30.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 20.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 10.0]),
                ("z", [0.0, 30.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;align-items:start'><table id=t style=';;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 30.0]),
                ("cap", [0.0, 10.0, 800.0, 10.0]),
                ("d", [0.0, 20.0, 20.0, 10.0]),
                ("r", [0.0, 0.0, 800.0, 10.0]),
                ("z", [0.0, 30.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;align-items:start'><table id=t style='align-self:end;;border-spacing:0'><caption id=cap style='height:10px'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 30.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 20.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 10.0]),
                ("z", [0.0, 30.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;align-items:start'><table id=t style='align-self:end;;border-spacing:0'><caption id=cap >aa bb</caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 30.0]),
                ("cap", [0.0, 0.0, 800.0, 10.0]),
                ("d", [0.0, 20.0, 20.0, 10.0]),
                ("r", [0.0, 10.0, 800.0, 10.0]),
                ("z", [0.0, 30.0, 800.0, 10.0]),
            ],
        ),
        (
            "<div id=c style='display:grid;align-items:start'><table id=t style='align-self:end;;border-spacing:0'><caption id=cap style='height:10px;caption-side:bottom'></caption><tr id=r><td style='padding:0'>x</td></tr></table><div id=d style='width:20px'>d</div></div><div id=z>z</div>",
            &[
                ("c", [0.0, 0.0, 800.0, 30.0]),
                ("cap", [0.0, 10.0, 800.0, 10.0]),
                ("d", [0.0, 20.0, 20.0, 10.0]),
                ("r", [0.0, 0.0, 800.0, 10.0]),
                ("z", [0.0, 30.0, 800.0, 10.0]),
            ],
        ),
    ]);
}
