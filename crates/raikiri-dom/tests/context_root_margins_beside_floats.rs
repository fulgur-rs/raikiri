//! Where a box that establishes its own formatting context (`flow-root`, a
//! table, a scroller) goes beside floats, and how wide it is, when it has
//! side margins. Beside floats the margins are measured from the content
//! edges of the containing block rather than from the floats: a positive
//! margin only narrows the room where it reaches past a float, and a negative
//! margin never widens it. An `auto` width whose negative margins are wider
//! than the room does not fit there and moves below the floats, where the
//! margins apply as usual.
//!
//! The expected boxes are the ones Chromium computes for the same markup, with
//! an 8px body margin, `border-box` sizing for `div` and `10px Ahem` text: each
//! is the element's left, top, width and height.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The left, top, width and height of the elements of `body` that have an
/// `id`, on the first page.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
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

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.04;

/// A label, the markup of a case and the `(id, left, top, width, height)` it
/// expects.
type Case<'a> = (&'a str, &'a str, &'a [(&'a str, f32, f32, f32, f32)]);

/// Lays out each case and compares the elements it names with the expected
/// boxes.
fn check_boxes(cases: &[Case<'_>]) {
    for &(label, body, expected) in cases {
        let actual = boxes(body);
        for &(id, left, top, width, height) in expected {
            let (_, rect) = actual
                .iter()
                .find(|(name, _)| name == id)
                .unwrap_or_else(|| panic!("{label}: no fragment for #{id}"));
            let want = [left, top, width, height];
            assert!(
                rect.iter()
                    .zip(want)
                    .all(|(got, want)| (got - want).abs() <= TOLERANCE),
                "{label}: #{id} is {rect:?}, expected {want:?} (left, top, width, height)"
            );
        }
    }
}

#[test]
fn negative_margins_beside_a_float_as_filed() {
    check_boxes(&[
        (
            "the margin pulls the box out of the room, so it moves below the float",
            "<div id=o style=\"display:inline-block\"><div style=\"float:left\">XXXXXXXXXX</div><div id=b style=\"display:flow-root;margin-left:-20px\">BBB</div></div>",
            &[("b", -12.0, 18.0, 130.0, 10.0)],
        ),
        (
            "a negative trailing margin does not widen the room",
            "<div id=o style=\"display:inline-block\"><div style=\"float:left\">XXXXXXXXXX</div><div id=b style=\"display:flow-root;margin-right:-5px\">BBB</div></div>",
            &[("b", 108.0, 8.0, 25.0, 10.0)],
        ),
        (
            "a negative leading margin beside a right float does not widen the room",
            "<div id=o style=\"display:inline-block\"><div style=\"float:right\">XXXXXXXXXX</div><div id=b style=\"display:flow-root;margin-left:-5px\">BBB</div></div>",
            &[("b", 8.0, 8.0, 25.0, 10.0)],
        ),
    ]);
}

#[test]
fn margins_beside_a_left_float() {
    check_boxes(&[
        (
            "a negative margin wider than the room moves an auto width below",
            "<div style=\"width:110px;\"><div id=f style=\"float:left;width:100px;height:20px\"></div><div id=b style=\"display:flow-root;margin-left:-20px\">BBB</div></div>",
            &[("b", -12.0, 28.0, 130.0, 10.0)],
        ),
        (
            "a room as wide as the negative margin keeps the box beside the float",
            "<div style=\"width:120px;\"><div id=f style=\"float:left;width:100px;height:20px\"></div><div id=b style=\"display:flow-root;margin-left:-20px\">BBB</div></div>",
            &[("b", 108.0, 8.0, 20.0, 10.0)],
        ),
        (
            "a negative trailing margin leaves the auto width at the room",
            "<div style=\"width:200px;\"><div id=f style=\"float:left;width:100px;height:20px\"></div><div id=b style=\"display:flow-root;margin-right:-20px\">BBB</div></div>",
            &[("b", 108.0, 8.0, 100.0, 10.0)],
        ),
        (
            "a fixed width wider than the room moves below",
            "<div style=\"width:200px;\"><div id=f style=\"float:left;width:100px;height:20px\"></div><div id=b style=\"display:flow-root;width:105px;margin-right:-20px\">BBB</div></div>",
            &[("b", 8.0, 28.0, 105.0, 10.0)],
        ),
        (
            "a positive margin inside the float is taken by the float",
            "<div style=\"width:200px;\"><div id=f style=\"float:left;width:100px;height:20px\"></div><div id=b style=\"display:flow-root;margin-left:20px\">BBB</div></div>",
            &[("b", 108.0, 8.0, 100.0, 10.0)],
        ),
        (
            "a positive margin past the float shrinks the room to nothing",
            "<div style=\"width:120px;\"><div id=f style=\"float:left;width:100px;height:20px\"></div><div id=b style=\"display:flow-root;margin-left:150px\">BBB</div></div>",
            &[("b", 158.0, 8.0, 0.0, 10.0)],
        ),
        (
            "a box as wide as the space between the floats fits past its margin",
            "<div style=\"width:200px;\"><div id=f style=\"float:left;width:100px;height:20px\"></div><div id=b style=\"display:flow-root;width:100px;margin-left:150px\">BBB</div></div>",
            &[("b", 158.0, 8.0, 100.0, 10.0)],
        ),
        (
            "negative margins on both sides",
            "<div style=\"width:200px;\"><div id=f style=\"float:left;width:100px;height:20px\"></div><div id=b style=\"display:flow-root;margin:0 -20px\">BBB</div></div>",
            &[("b", 108.0, 8.0, 100.0, 10.0)],
        ),
        (
            "the box's own direction does not change where it goes",
            "<div style=\"width:200px;\"><div id=f style=\"float:left;width:100px;height:20px\"></div><div id=b style=\"display:flow-root;direction:rtl;width:100px;margin-left:150px\">BBB</div></div>",
            &[("b", 158.0, 8.0, 100.0, 10.0)],
        ),
    ]);
}

#[test]
fn margins_beside_a_right_float() {
    check_boxes(&[
        (
            "a negative leading margin",
            "<div style=\"width:200px;\"><div id=f style=\"float:right;width:100px;height:20px\"></div><div id=b style=\"display:flow-root;margin-left:-20px\">BBB</div></div>",
            &[("b", 8.0, 8.0, 100.0, 10.0)],
        ),
        (
            "a fixed width as wide as the room",
            "<div style=\"width:200px;\"><div id=f style=\"float:right;width:100px;height:20px\"></div><div id=b style=\"display:flow-root;width:100px;margin-left:-20px\">BBB</div></div>",
            &[("b", 8.0, 8.0, 100.0, 10.0)],
        ),
        (
            "a negative trailing margin wider than the room",
            "<div style=\"width:110px;\"><div id=f style=\"float:right;width:100px;height:20px\"></div><div id=b style=\"display:flow-root;margin-right:-20px\">BBB</div></div>",
            &[("b", 8.0, 28.0, 130.0, 10.0)],
        ),
    ]);
}

#[test]
fn margins_beside_floats_in_a_right_to_left_container() {
    check_boxes(&[
        (
            "a negative leading margin beside a left float",
            "<div style=\"width:200px;direction:rtl\"><div id=f style=\"float:left;width:100px;height:20px\"></div><div id=b style=\"display:flow-root;margin-right:-20px\">BBB</div></div>",
            &[("b", 108.0, 8.0, 100.0, 10.0)],
        ),
        (
            "a positive trailing margin past a left float shrinks the room",
            "<div style=\"width:200px;direction:rtl\"><div id=f style=\"float:left;width:100px;height:20px\"></div><div id=b style=\"display:flow-root;margin-left:150px\">BBB</div></div>",
            &[("b", 158.0, 8.0, 50.0, 10.0)],
        ),
        (
            "a fixed width keeps clear of a left float past its trailing margin",
            "<div style=\"width:200px;direction:rtl\"><div id=f style=\"float:left;width:100px;height:20px\"></div><div id=b style=\"display:flow-root;width:100px;margin-left:150px\">BBB</div></div>",
            &[("b", 108.0, 8.0, 100.0, 10.0)],
        ),
        (
            "a leading margin that pushes the box over a left float moves it below",
            "<div style=\"width:200px;direction:rtl\"><div id=f style=\"float:left;width:100px;height:20px\"></div><div id=b style=\"display:flow-root;width:100px;margin-right:150px\">BBB</div></div>",
            &[("b", -42.0, 28.0, 100.0, 10.0)],
        ),
        (
            "a negative trailing margin beside a right float",
            "<div style=\"width:200px;direction:rtl\"><div id=f style=\"float:right;width:100px;height:20px\"></div><div id=b style=\"display:flow-root;margin-left:-20px\">BBB</div></div>",
            &[("b", 8.0, 8.0, 100.0, 10.0)],
        ),
        (
            "a negative leading margin wider than the room",
            "<div style=\"width:110px;direction:rtl\"><div id=f style=\"float:right;width:100px;height:20px\"></div><div id=b style=\"display:flow-root;margin-right:-20px\">BBB</div></div>",
            &[("b", 8.0, 28.0, 130.0, 10.0)],
        ),
        (
            "a room as wide as the negative margin",
            "<div style=\"width:120px;direction:rtl\"><div id=f style=\"float:right;width:100px;height:20px\"></div><div id=b style=\"display:flow-root;margin-right:-20px\">BBB</div></div>",
            &[("b", 8.0, 8.0, 20.0, 10.0)],
        ),
        (
            "a fixed width wider than the room",
            "<div style=\"width:200px;direction:rtl\"><div id=f style=\"float:right;width:100px;height:20px\"></div><div id=b style=\"display:flow-root;width:105px;margin:0 -20px\">BBB</div></div>",
            &[("b", 123.0, 28.0, 105.0, 10.0)],
        ),
    ]);
}

#[test]
fn margins_beside_floats_in_other_containers() {
    check_boxes(&[
        (
            "floats on both sides",
            "<div style=\"width:200px\"><div style=\"float:left;width:100px;height:20px\"></div><div style=\"float:right;width:50px;height:20px\"></div><div id=b style=\"display:flow-root;margin-left:-5px\">BBB</div></div>",
            &[("b", 108.0, 8.0, 50.0, 10.0)],
        ),
        (
            "floats on both sides and a fixed width wider than the room",
            "<div style=\"width:200px\"><div style=\"float:left;width:100px;height:20px\"></div><div style=\"float:right;width:50px;height:20px\"></div><div id=b style=\"display:flow-root;margin-right:-30px;width:60px\">BBB</div></div>",
            &[("b", 8.0, 28.0, 60.0, 10.0)],
        ),
        (
            "a padded container",
            "<div style=\"width:200px;padding:0 10px\"><div style=\"float:left;width:100px;height:20px\"></div><div id=b style=\"display:flow-root;margin-left:-20px\">BBB</div></div>",
            &[("b", 118.0, 8.0, 80.0, 10.0)],
        ),
        (
            "a float that stays outside the containing block lets the margin apply",
            "<div style=\"width:200px\"><div style=\"float:left;width:50px;height:20px\"></div><div style=\"margin-left:100px\"><div id=b style=\"display:flow-root;margin-left:-80px\">BBB</div></div></div>",
            &[("b", 28.0, 8.0, 180.0, 10.0)],
        ),
        (
            "the same, right to left",
            "<div style=\"width:200px;direction:rtl;\"><div style=\"float:left;width:50px;height:20px\"></div><div style=\"margin-left:100px\"><div id=b style=\"display:flow-root;margin-left:-80px\">BBB</div></div></div>",
            &[("b", 28.0, 8.0, 180.0, 10.0)],
        ),
        (
            "a table",
            "<div style=\"width:200px\"><div style=\"float:left;width:100px;height:20px\"></div><table id=b style=\"margin-left:-20px;border-spacing:0\"><tr><td style=\"padding:0\">BBB</td></tr></table></div>",
            &[("b", 108.0, 8.0, 30.0, 10.0)],
        ),
        (
            "a box inside an inline element",
            "<div style=\"width:110px\"><div style=\"float:left;width:100px;height:20px\"></div><span>X<div id=b style=\"display:flow-root;margin-right:-20px\">BBB</div>X</span></div>",
            &[("b", 8.0, 28.0, 130.0, 10.0)],
        ),
    ]);
}
