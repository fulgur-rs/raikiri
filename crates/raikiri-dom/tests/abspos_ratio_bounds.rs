//! The size of an absolutely positioned box with an aspect ratio.
//!
//! The box states some of its sizes and the rest follow from them through the
//! preferred aspect ratio (CSS Sizing 4 section 4). A size the box states is not
//! one a minimum or maximum of the other axis moves, a maximum below the padding
//! and border is the padding and border for what it carries across the ratio,
//! and the height that the ratio gives is no less than the content needs at the
//! width (section 5.1, the automatic minimum size). Every border box here has
//! 25px of border unless it says otherwise, and the expected boxes are the ones
//! Chromium lays out for the same markup, in an 800px by 600px viewport without a
//! body margin and with Ahem at 10px, where a word of three letters is 30px wide.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The border box `(x, y, width, height)` of every element with an `id` when
/// `body` is laid out on a page without margins.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
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

/// The expected border box `(x, y, width, height)` of each element, by `id`.
type Expected<'a> = &'a [(&'a str, [f32; 4])];

/// Check that every `(id, expected box)` of every case is laid out as given.
fn check(cases: Vec<(&str, String, Expected<'_>)>) {
    for (what, body, expected) in cases {
        let actual = boxes(&body);
        for &(id, want) in expected {
            let got = actual
                .iter()
                .find(|(name, _)| name == id)
                .unwrap_or_else(|| panic!("{what}: no fragment for #{id} in {body}"))
                .1;
            assert!(
                got.iter()
                    .zip(want)
                    .all(|(got, want)| (got - want).abs() < 0.01),
                "{what}: #{id} is {got:?}, expected {want:?} for {body}"
            );
        }
    }
}

#[test]
fn an_out_of_flow_ratio_box_that_states_its_width_is_as_tall_as_its_content() {
    // A box that states its width and leaves its height `auto` is no shorter than its content at that width: the ratio gives the height only when the content fits it.
    check(vec![
        (
            r#"position:absolute;display:flex;width:80px;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:flex;border:solid 25px green;width:80px;aspect-ratio:2/1"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 70.0])],
        ),
        (
            r#"position:absolute;display:flex;width:40px;aspect-ratio:1/2;flex-direction:column"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:flex;border:solid 25px green;width:40px;aspect-ratio:1/2;flex-direction:column"><div style="width:20px;height:30px"></div><div style="width:20px;height:40px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 50.0, 120.0])],
        ),
        (
            r#"position:absolute;display:flex;height:40px;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:flex;border:solid 25px green;height:40px;aspect-ratio:2/1"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 120.0, 50.0])],
        ),
        (
            r#"position:absolute;display:flex;width:200px;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:flex;border:solid 25px green;width:200px;aspect-ratio:2/1"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 200.0, 100.0])],
        ),
        (
            r#"position:absolute;display:flex;width:80px;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:flex;border:solid 25px green;width:80px;aspect-ratio:2/1"><div style="width:30px;height:60px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 110.0])],
        ),
        (
            r#"position:absolute;display:flex;width:80px;aspect-ratio:2/1;overflow:hidden"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:flex;border:solid 25px green;width:80px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 50.0])],
        ),
        (
            r#"position:absolute;display:flex;width:80px;aspect-ratio:2/1;display:grid"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:flex;border:solid 25px green;width:80px;aspect-ratio:2/1;display:grid"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 90.0])],
        ),
        (
            r#"position:absolute;display:flex;width:80px;aspect-ratio:2/1;display:block"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:flex;border:solid 25px green;width:80px;aspect-ratio:2/1;display:block"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 90.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:80px;aspect-ratio:2/1">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 80.0])],
        ),
        (
            r#"position:absolute;display:block;width:200px;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:200px;aspect-ratio:2/1">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 200.0, 100.0])],
        ),
        (
            r#"position:absolute;display:block;width:50%;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:50%;aspect-ratio:2/1">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 150.0, 75.0])],
        ),
        (
            r#"position:absolute;display:block;width:max-content;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:max-content;aspect-ratio:2/1">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 160.0, 80.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;aspect-ratio:2/1;overflow:hidden"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:80px;aspect-ratio:2/1;overflow:hidden">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 50.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;aspect-ratio:2/1;box-sizing:content-box"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:80px;aspect-ratio:2/1;box-sizing:content-box">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 130.0, 90.0])],
        ),
        (
            r#"position:absolute;display:block;width:30px;aspect-ratio:2/1;box-sizing:content-box"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:30px;aspect-ratio:2/1;box-sizing:content-box">aaa bbb ccc ddd eee fff</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 110.0])],
        ),
        (
            r#"position:absolute;display:block;height:30px;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;height:30px;aspect-ratio:2/1">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 100.0, 50.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;height:40px;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:80px;height:40px;aspect-ratio:2/1">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 50.0])],
        ),
        (
            r#"position:absolute;display:flex;width:80px;aspect-ratio:2/1;border-width:5px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:flex;border:solid 25px green;width:80px;aspect-ratio:2/1;border-width:5px"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 40.0])],
        ),
        (
            r#"position:fixed;display:block;width:80px;aspect-ratio:2/1"#,
            r#"<div style="width:300px;height:300px"><div id=o style="position:fixed;display:block;border:solid 25px green;width:80px;aspect-ratio:2/1">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 80.0])],
        ),
        (
            r#"position:absolute;display:block;width:60px;aspect-ratio:2/1;grid-area:2/2"#,
            r#"<div style="position:relative;display:grid;grid-template-columns:200px 100px;grid-template-rows:200px 100px;width:300px;height:300px"><div></div><div></div><div></div><div></div><div id=o style="position:absolute;display:block;border:solid 25px green;width:60px;aspect-ratio:2/1;grid-area:2/2">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [200.0, 200.0, 60.0, 80.0])],
        ),
        (
            r#"position:absolute;display:block;width:60%;aspect-ratio:2/1;grid-area:1/1"#,
            r#"<div style="position:relative;display:grid;grid-template-columns:200px 100px;grid-template-rows:200px 100px;width:300px;height:300px"><div></div><div></div><div></div><div></div><div id=o style="position:absolute;display:block;border:solid 25px green;width:60%;aspect-ratio:2/1;grid-area:1/1">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 180.0, 90.0])],
        ),
        (
            r#"position:absolute;display:flex;width:80px;aspect-ratio:2/1;box-sizing:content-box"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:flex;border:solid 25px green;width:80px;aspect-ratio:2/1;box-sizing:content-box"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 130.0, 90.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:80px;aspect-ratio:2/1"><div style="width:30px;aspect-ratio:1/1">q</div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 80.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;aspect-ratio:2/1;margin:10px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:80px;aspect-ratio:2/1;margin:10px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [10.0, 10.0, 80.0, 80.0])],
        ),
        (
            r#"position:absolute;display:block;width:100px;aspect-ratio:2/1;padding:10px;border-width:5px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:100px;aspect-ratio:2/1;padding:10px;border-width:5px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 100.0, 50.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;aspect-ratio:2/1;left:20px;top:30px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:80px;aspect-ratio:2/1;left:20px;top:30px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [20.0, 30.0, 80.0, 80.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;aspect-ratio:2/1;left:20px;bottom:30px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:80px;aspect-ratio:2/1;left:20px;bottom:30px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [20.0, 190.0, 80.0, 80.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;aspect-ratio:2/1;top:20px;bottom:30px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:80px;aspect-ratio:2/1;top:20px;bottom:30px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 20.0, 80.0, 80.0])],
        ),
    ]);
}

#[test]
fn a_bound_carried_across_the_ratio_does_not_move_a_size_the_box_states() {
    // A minimum or maximum along one axis does not move the size the box states along the other.
    check(vec![
        (
            r#"position:absolute;display:flex;width:80px;aspect-ratio:2/1;min-height:100px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:flex;border:solid 25px green;width:80px;aspect-ratio:2/1;min-height:100px"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 100.0])],
        ),
        (
            r#"position:absolute;display:flex;width:80px;aspect-ratio:2/1;max-height:60px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:flex;border:solid 25px green;width:80px;aspect-ratio:2/1;max-height:60px"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 60.0])],
        ),
        (
            r#"position:absolute;display:flex;width:80px;aspect-ratio:2/1;min-width:0"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:flex;border:solid 25px green;width:80px;aspect-ratio:2/1;min-width:0"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 70.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;aspect-ratio:2/1;max-height:30px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:80px;aspect-ratio:2/1;max-height:30px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 50.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;aspect-ratio:2/1;max-height:70px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:80px;aspect-ratio:2/1;max-height:70px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 70.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;aspect-ratio:2/1;min-height:90px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:80px;aspect-ratio:2/1;min-height:90px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 90.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;aspect-ratio:2/1;min-width:0"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:80px;aspect-ratio:2/1;min-width:0">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 80.0])],
        ),
        (
            r#"position:absolute;display:flex;width:80px;aspect-ratio:2/1;max-height:30px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:flex;border:solid 25px green;width:80px;aspect-ratio:2/1;max-height:30px"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 50.0])],
        ),
        (
            r#"position:absolute;display:flex;width:80px;aspect-ratio:2/1;max-height:30px;border-width:5px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:flex;border:solid 25px green;width:80px;aspect-ratio:2/1;max-height:30px;border-width:5px"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 30.0])],
        ),
        (
            r#"position:absolute;display:flex;width:80px;aspect-ratio:2/1;border-width:5px;min-height:40px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:flex;border:solid 25px green;width:80px;aspect-ratio:2/1;border-width:5px;min-height:40px"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 40.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;aspect-ratio:2/1;min-height:auto"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:80px;aspect-ratio:2/1;min-height:auto">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 80.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;aspect-ratio:2/1;max-height:200px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:80px;aspect-ratio:2/1;max-height:200px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 80.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;aspect-ratio:2/1;min-width:120px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:80px;aspect-ratio:2/1;min-width:120px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 120.0, 70.0])],
        ),
        (
            r#"position:absolute;display:block;width:200px;aspect-ratio:2/1;max-width:80px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:200px;aspect-ratio:2/1;max-width:80px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 80.0])],
        ),
        (
            r#"position:absolute;display:block;height:60px;aspect-ratio:2/1;max-width:80px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;height:60px;aspect-ratio:2/1;max-width:80px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 60.0])],
        ),
        (
            r#"position:absolute;display:block;height:60px;aspect-ratio:2/1;min-width:200px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;height:60px;aspect-ratio:2/1;min-width:200px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 200.0, 60.0])],
        ),
        (
            r#"position:absolute;display:block;height:60px;aspect-ratio:2/1;max-width:30px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;height:60px;aspect-ratio:2/1;max-width:30px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 50.0, 60.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;height:80px;aspect-ratio:2/1;max-height:30px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:80px;height:80px;aspect-ratio:2/1;max-height:30px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 50.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;height:60px;aspect-ratio:2/1;min-height:100px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 25px green;width:80px;height:60px;aspect-ratio:2/1;min-height:100px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 100.0])],
        ),
    ]);
}

#[test]
fn a_maximum_below_the_padding_and_border_is_raised_before_it_is_carried() {
    // A border-box maximum smaller than the padding and border is raised to them before the ratio carries it to the other axis.
    check(vec![
        (
            r#"position:absolute;display:flex;aspect-ratio:2/1;max-height:40px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:flex;border:solid 25px green;aspect-ratio:2/1;max-height:40px"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 100.0, 50.0])],
        ),
        (
            r#"display:flex;width:80px;aspect-ratio:2/1"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:flex;border:solid 25px green;width:80px;aspect-ratio:2/1"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 70.0])],
        ),
        (
            r#"display:flex;width:40px;aspect-ratio:1/2;flex-direction:column"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:flex;border:solid 25px green;width:40px;aspect-ratio:1/2;flex-direction:column"><div style="width:20px;height:30px"></div><div style="width:20px;height:40px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 50.0, 120.0])],
        ),
        (
            r#"display:flex;aspect-ratio:2/1;max-height:40px"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:flex;border:solid 25px green;aspect-ratio:2/1;max-height:40px"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 100.0, 50.0])],
        ),
        (
            r#"display:flex;aspect-ratio:2/1;max-height:40px"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:flex;border:solid 25px green;aspect-ratio:2/1;max-height:40px"></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 100.0, 50.0])],
        ),
        (
            r#"display:flex;height:40px;aspect-ratio:2/1"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:flex;border:solid 25px green;height:40px;aspect-ratio:2/1"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 120.0, 50.0])],
        ),
        (
            r#"display:flex;width:200px;aspect-ratio:2/1"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:flex;border:solid 25px green;width:200px;aspect-ratio:2/1"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 200.0, 100.0])],
        ),
        (
            r#"display:flex;width:80px;aspect-ratio:2/1"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:flex;border:solid 25px green;width:80px;aspect-ratio:2/1"><div style="width:30px;height:60px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 110.0])],
        ),
        (
            r#"display:flex;width:80px;aspect-ratio:2/1;min-height:100px"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:flex;border:solid 25px green;width:80px;aspect-ratio:2/1;min-height:100px"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 100.0])],
        ),
        (
            r#"display:flex;width:80px;aspect-ratio:2/1;max-height:60px"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:flex;border:solid 25px green;width:80px;aspect-ratio:2/1;max-height:60px"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 60.0])],
        ),
        (
            r#"display:flex;width:80px;aspect-ratio:2/1;overflow:hidden"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:flex;border:solid 25px green;width:80px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 50.0])],
        ),
        (
            r#"display:flex;width:80px;aspect-ratio:2/1;min-width:0"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:flex;border:solid 25px green;width:80px;aspect-ratio:2/1;min-width:0"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 70.0])],
        ),
        (
            r#"display:flex;width:80px;aspect-ratio:2/1;display:grid"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:flex;border:solid 25px green;width:80px;aspect-ratio:2/1;display:grid"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 90.0])],
        ),
        (
            r#"display:flex;width:80px;aspect-ratio:2/1;display:block"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:flex;border:solid 25px green;width:80px;aspect-ratio:2/1;display:block"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 80.0, 90.0])],
        ),
        (
            r#"display:block;aspect-ratio:2/1;max-height:40px"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:block;border:solid 25px green;aspect-ratio:2/1;max-height:40px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 100.0, 50.0])],
        ),
        (
            r#"display:block;aspect-ratio:2/1;max-height:30px;border-width:5px"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:block;border:solid 25px green;aspect-ratio:2/1;max-height:30px;border-width:5px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 60.0, 30.0])],
        ),
        (
            r#"display:grid;aspect-ratio:2/1;max-height:40px"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:grid;border:solid 25px green;aspect-ratio:2/1;max-height:40px"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 100.0, 50.0])],
        ),
        (
            r#"display:flex;aspect-ratio:2/1;max-height:40px;width:60px"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:flex;border:solid 25px green;aspect-ratio:2/1;max-height:40px;width:60px"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 60.0, 50.0])],
        ),
        (
            r#"display:flex;aspect-ratio:2/1;max-width:40px"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:flex;border:solid 25px green;aspect-ratio:2/1;max-width:40px"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 50.0, 70.0])],
        ),
        (
            r#"display:flex;aspect-ratio:2/1;min-height:40px"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:flex;border:solid 25px green;aspect-ratio:2/1;min-height:40px"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 300.0, 150.0])],
        ),
        (
            r#"display:flex;aspect-ratio:1/2;min-width:120px"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:flex;border:solid 25px green;aspect-ratio:1/2;min-width:120px"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 300.0, 600.0])],
        ),
        (
            r#"position:absolute;display:flex;aspect-ratio:2/1;max-height:40px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:flex;border:solid 25px green;aspect-ratio:2/1;max-height:40px"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 100.0, 50.0])],
        ),
        (
            r#"position:absolute;display:flex;aspect-ratio:2/1;max-width:40px;height:200px"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:flex;border:solid 25px green;aspect-ratio:2/1;max-width:40px;height:200px"><div style="width:30px;height:20px"></div><div style="width:40px;height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 50.0, 200.0])],
        ),
        (
            r#"display:block;max-height:40px"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:block;border:solid 25px green;max-height:40px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 300.0, 50.0])],
        ),
        (
            r#"display:block;max-height:40px"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:block;border:solid 25px green;max-height:40px"></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 300.0, 50.0])],
        ),
        (
            r#"display:block;max-height:20px;padding:10px"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:block;border:solid 25px green;max-height:20px;padding:10px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 300.0, 70.0])],
        ),
        (
            r#"display:block;aspect-ratio:2/1;max-height:40px"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:block;border:solid 25px green;aspect-ratio:2/1;max-height:40px"></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 100.0, 50.0])],
        ),
        (
            r#"display:block;aspect-ratio:2/1;max-height:40px;width:100px"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:block;border:solid 25px green;aspect-ratio:2/1;max-height:40px;width:100px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 100.0, 50.0])],
        ),
        (
            r#"display:block;aspect-ratio:2/1;max-height:60px"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:block;border:solid 25px green;aspect-ratio:2/1;max-height:60px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 120.0, 60.0])],
        ),
        (
            r#"display:block;aspect-ratio:2/1;max-width:40px"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:block;border:solid 25px green;aspect-ratio:2/1;max-width:40px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 50.0, 80.0])],
        ),
        (
            r#"display:block;aspect-ratio:2/1;min-height:30px"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:block;border:solid 25px green;aspect-ratio:2/1;min-height:30px">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 300.0, 150.0])],
        ),
        (
            r#"display:block;aspect-ratio:2/1;max-height:40px;box-sizing:content-box"#,
            r#"<div style="width:300px;height:300px"><div id=o style="display:block;border:solid 25px green;aspect-ratio:2/1;max-height:40px;box-sizing:content-box">aaa bbb ccc</div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 130.0, 90.0])],
        ),
    ]);
}

#[test]
fn a_minimum_width_raises_the_height_the_ratio_gives_even_below_the_content_floor() {
    // A box that states its width is raised to its `min-width`, and the ratio gives the height of that width, whatever its content and `min-height` are. These boxes have no border unless they say so.
    check(vec![
        (
            r#"position:absolute;display:block;width:80px;min-width:200px;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;width:80px;min-width:200px;aspect-ratio:2/1"></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 200.0, 100.0])],
        ),
        (
            r#"position:absolute;display:block;border:solid 10px green;width:80px;min-width:200px;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 10px green;width:80px;min-width:200px;aspect-ratio:2/1"></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 200.0, 100.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;min-width:200px;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;width:80px;min-width:200px;aspect-ratio:2/1"><div style="height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 200.0, 100.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;min-width:200px;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;width:80px;min-width:200px;aspect-ratio:2/1"><div style="height:150px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 200.0, 150.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;min-width:200px;max-height:60px;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;width:80px;min-width:200px;max-height:60px;aspect-ratio:2/1"><div style="height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 200.0, 60.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;min-width:200px;max-height:60px;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;width:80px;min-width:200px;max-height:60px;aspect-ratio:2/1"><div style="height:150px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 200.0, 60.0])],
        ),
        (
            r#"position:absolute;display:block;border:solid 10px green;width:80px;min-width:200px;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;border:solid 10px green;width:80px;min-width:200px;aspect-ratio:2/1"><div style="height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 200.0, 100.0])],
        ),
        (
            r#"position:absolute;display:block;box-sizing:content-box;padding:10px;width:80px;min-width:200px;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;box-sizing:content-box;padding:10px;width:80px;min-width:200px;aspect-ratio:2/1"><div style="height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 220.0, 120.0])],
        ),
        (
            r#"position:absolute;display:block;box-sizing:content-box;padding:10px;width:80px;min-width:200px;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;box-sizing:content-box;padding:10px;width:80px;min-width:200px;aspect-ratio:2/1"><div style="height:150px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 220.0, 170.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;min-width:50%;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;width:80px;min-width:50%;aspect-ratio:2/1"><div style="height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 150.0, 75.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;min-width:200px;max-width:150px;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;width:80px;min-width:200px;max-width:150px;aspect-ratio:2/1"><div style="height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 200.0, 100.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;min-width:200px;aspect-ratio:2/1;overflow:auto"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;width:80px;min-width:200px;aspect-ratio:2/1;overflow:auto"><div style="height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 200.0, 100.0])],
        ),
        (
            r#"position:absolute;display:block;width:80px;min-width:200px;min-height:30px;aspect-ratio:2/1"#,
            r#"<div style="position:relative;width:300px;height:300px"><div id=o style="position:absolute;display:block;width:80px;min-width:200px;min-height:30px;aspect-ratio:2/1"><div style="height:20px"></div></div></div>"#.to_string(),
            &[("o", [0.0, 0.0, 200.0, 100.0])],
        ),
    ]);
}
