//! `width: stretch` and `height: stretch` fill the room of the containing block
//! on the physical axes, whatever the writing modes of the box and of its
//! parent are. In a vertical parent the width is the block axis: a margin of the
//! child takes room off it only where the parent holds the margin in, because
//! it is a formatting context of its own or has a border or padding on that
//! side, and the height is the inline axis, which every margin takes room off.
//! While the content of the parent decides its width, a vertical child reads
//! `stretch` as `auto`, and a horizontal child, an orthogonal flow, fills the
//! page area that stands in for the initial containing block.
//!
//! The expected border boxes were recorded from Chromium for the same markup in
//! an 800px by 600px viewport, with the body margin of 8px and
//! `box-sizing: border-box` on every `div`.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A border box `(x, y, width, height)`.
type Rect = (f32, f32, f32, f32);

/// A case: its name, its markup, and the border boxes by id.
type Case = (&'static str, &'static str, &'static [(&'static str, Rect)]);

/// The border boxes of the elements of `body` with an `id` on the first page.
fn border_boxes(body: &str) -> Vec<(String, Rect)> {
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
            Some((id.to_owned(), (rect.x, rect.y, rect.width, rect.height)))
        })
        .collect()
}

/// Layout units are 1/64px; allow a little more for the 32-bit sums.
const TOLERANCE: f32 = 0.02;

fn check(name: &str, body: &str, expected: &[(&str, Rect)]) {
    let actual = border_boxes(body);
    for &(id, (x, y, width, height)) in expected {
        let found = actual
            .iter()
            .find(|(box_id, _)| box_id == id)
            .unwrap_or_else(|| panic!("{name}: no fragment for #{id} in {body}"))
            .1;
        let close = |a: f32, b: f32| (a - b).abs() <= TOLERANCE;
        assert!(
            close(found.0, x)
                && close(found.1, y)
                && close(found.2, width)
                && close(found.3, height),
            "{name}: {body}: #{id} is {found:?}, expected {:?}",
            (x, y, width, height)
        );
    }
}

fn check_case(&(name, body, expected): &Case) {
    check(name, body, expected);
}

#[test]
fn a_stretch_child_of_a_vertical_parent_fills_the_physical_axes_less_the_margins_held() {
    // The parent is a context of its own, a plain block with and without
    // border and padding, and a scroll container, in both vertical modes.
    VERTICAL_PARENT.iter().for_each(check_case);
}

#[test]
fn a_stretch_child_of_an_orthogonal_parent_flex_or_grid_container_fills_its_room() {
    ORTHOGONAL_AND_FLEX.iter().for_each(check_case);
}

#[test]
fn a_vertical_inline_block_or_float_fills_the_room_with_its_stretch_width() {
    ATOMIC_AND_FLOAT.iter().for_each(check_case);
}

#[test]
fn stretch_reads_as_auto_while_the_content_of_a_vertical_parent_decides_its_width() {
    // Limits, edges, auto margins, nested boxes, floats, sideways modes and
    // right-to-left parents.
    EDGE_CASES.iter().for_each(check_case);
}

#[rustfmt::skip]
const VERTICAL_PARENT: &[Case] = &[
    ("b_p1_rl_h_width_0", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 10.0))]),
    ("b_p1_rl_h_width_m", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="margin:10px 20px;writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 10.0))]),
    ("b_p1_rl_h_height_0", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (238.0, 8.0, 70.0, 200.0))]),
    ("b_p1_rl_h_height_m", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="margin:10px 20px;writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (218.0, 18.0, 70.0, 180.0))]),
    ("b_p1_rl_v_width_0", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("b_p1_rl_v_width_m", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="margin:10px 20px;writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 180.0))]),
    ("b_p1_rl_v_height_0", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="writing-mode:vertical-rl;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (298.0, 8.0, 10.0, 200.0))]),
    ("b_p1_rl_v_height_m", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="margin:10px 20px;writing-mode:vertical-rl;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (278.0, 18.0, 10.0, 180.0))]),
    ("b_p2_rl_h_width_0", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px"><div id=c style="writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (58.0, 8.0, 250.0, 150.0)), ("c", (58.0, 8.0, 250.0, 10.0))]),
    ("b_p2_rl_h_width_m", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px"><div id=c style="margin:10px 20px;writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (38.0, 8.0, 250.0, 150.0)), ("c", (38.0, 18.0, 250.0, 10.0))]),
    ("b_p2_rl_h_height_0", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px"><div id=c style="writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (58.0, 8.0, 250.0, 150.0)), ("c", (238.0, 8.0, 70.0, 150.0))]),
    ("b_p2_rl_h_height_m", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px"><div id=c style="margin:10px 20px;writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (38.0, 8.0, 250.0, 150.0)), ("c", (218.0, 18.0, 70.0, 130.0))]),
    ("b_p2_rl_v_width_0", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px"><div id=c style="writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (58.0, 8.0, 250.0, 150.0)), ("c", (58.0, 8.0, 250.0, 150.0))]),
    ("b_p2_rl_v_width_m", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px"><div id=c style="margin:10px 20px;writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (38.0, 8.0, 250.0, 150.0)), ("c", (38.0, 18.0, 250.0, 130.0))]),
    ("b_p2_rl_v_height_0", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px"><div id=c style="writing-mode:vertical-rl;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (58.0, 8.0, 250.0, 150.0)), ("c", (298.0, 8.0, 10.0, 150.0))]),
    ("b_p2_rl_v_height_m", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px"><div id=c style="margin:10px 20px;writing-mode:vertical-rl;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (38.0, 8.0, 250.0, 150.0)), ("c", (278.0, 18.0, 10.0, 130.0))]),
    ("b_p3_rl_h_width_0", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px;padding:5px"><div id=c style="writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (58.0, 8.0, 250.0, 150.0)), ("c", (63.0, 13.0, 240.0, 10.0))]),
    ("b_p3_rl_h_width_m", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px;padding:5px"><div id=c style="margin:10px 20px;writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (58.0, 8.0, 250.0, 150.0)), ("c", (83.0, 23.0, 200.0, 10.0))]),
    ("b_p3_rl_h_height_0", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px;padding:5px"><div id=c style="writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (58.0, 8.0, 250.0, 150.0)), ("c", (233.0, 13.0, 70.0, 140.0))]),
    ("b_p3_rl_h_height_m", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px;padding:5px"><div id=c style="margin:10px 20px;writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (58.0, 8.0, 250.0, 150.0)), ("c", (213.0, 23.0, 70.0, 120.0))]),
    ("b_p3_rl_v_width_0", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px;padding:5px"><div id=c style="writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (58.0, 8.0, 250.0, 150.0)), ("c", (63.0, 13.0, 240.0, 140.0))]),
    ("b_p3_rl_v_width_m", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px;padding:5px"><div id=c style="margin:10px 20px;writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (58.0, 8.0, 250.0, 150.0)), ("c", (83.0, 23.0, 200.0, 120.0))]),
    ("b_p3_rl_v_height_0", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px;padding:5px"><div id=c style="writing-mode:vertical-rl;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (58.0, 8.0, 250.0, 150.0)), ("c", (293.0, 13.0, 10.0, 140.0))]),
    ("b_p3_rl_v_height_m", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px;padding:5px"><div id=c style="margin:10px 20px;writing-mode:vertical-rl;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (58.0, 8.0, 250.0, 150.0)), ("c", (273.0, 23.0, 10.0, 120.0))]),
    ("b_p4_rl_h_width_0", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px;overflow:hidden"><div id=c style="writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (58.0, 8.0, 250.0, 150.0)), ("c", (58.0, 8.0, 250.0, 10.0))]),
    ("b_p4_rl_h_width_m", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px;overflow:hidden"><div id=c style="margin:10px 20px;writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (58.0, 8.0, 250.0, 150.0)), ("c", (78.0, 18.0, 210.0, 10.0))]),
    ("b_p4_rl_h_height_0", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px;overflow:hidden"><div id=c style="writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (58.0, 8.0, 250.0, 150.0)), ("c", (238.0, 8.0, 70.0, 150.0))]),
    ("b_p4_rl_h_height_m", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px;overflow:hidden"><div id=c style="margin:10px 20px;writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (58.0, 8.0, 250.0, 150.0)), ("c", (218.0, 18.0, 70.0, 130.0))]),
    ("b_p4_rl_v_width_0", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px;overflow:hidden"><div id=c style="writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (58.0, 8.0, 250.0, 150.0)), ("c", (58.0, 8.0, 250.0, 150.0))]),
    ("b_p4_rl_v_width_m", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px;overflow:hidden"><div id=c style="margin:10px 20px;writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (58.0, 8.0, 250.0, 150.0)), ("c", (78.0, 18.0, 210.0, 130.0))]),
    ("b_p4_rl_v_height_0", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px;overflow:hidden"><div id=c style="writing-mode:vertical-rl;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (58.0, 8.0, 250.0, 150.0)), ("c", (298.0, 8.0, 10.0, 150.0))]),
    ("b_p4_rl_v_height_m", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=q style="width:250px;height:150px;overflow:hidden"><div id=c style="margin:10px 20px;writing-mode:vertical-rl;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (58.0, 8.0, 250.0, 150.0)), ("c", (278.0, 18.0, 10.0, 130.0))]),
    ("b_p1_lr_h_width_0", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 10.0))]),
    ("b_p1_lr_h_width_m", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="margin:10px 20px;writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 10.0))]),
    ("b_p1_lr_h_height_0", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 70.0, 200.0))]),
    ("b_p1_lr_h_height_m", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="margin:10px 20px;writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 70.0, 180.0))]),
    ("b_p1_lr_v_width_0", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("b_p1_lr_v_width_m", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="margin:10px 20px;writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 180.0))]),
    ("b_p1_lr_v_height_0", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="writing-mode:vertical-lr;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 10.0, 200.0))]),
    ("b_p1_lr_v_height_m", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="margin:10px 20px;writing-mode:vertical-lr;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 10.0, 180.0))]),
    ("b_p2_lr_h_width_0", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px"><div id=c style="writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (8.0, 8.0, 250.0, 150.0)), ("c", (8.0, 8.0, 250.0, 10.0))]),
    ("b_p2_lr_h_width_m", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px"><div id=c style="margin:10px 20px;writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (28.0, 8.0, 250.0, 150.0)), ("c", (28.0, 18.0, 250.0, 10.0))]),
    ("b_p2_lr_h_height_0", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px"><div id=c style="writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (8.0, 8.0, 250.0, 150.0)), ("c", (8.0, 8.0, 70.0, 150.0))]),
    ("b_p2_lr_h_height_m", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px"><div id=c style="margin:10px 20px;writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (28.0, 8.0, 250.0, 150.0)), ("c", (28.0, 18.0, 70.0, 130.0))]),
    ("b_p2_lr_v_width_0", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px"><div id=c style="writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (8.0, 8.0, 250.0, 150.0)), ("c", (8.0, 8.0, 250.0, 150.0))]),
    ("b_p2_lr_v_width_m", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px"><div id=c style="margin:10px 20px;writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (28.0, 8.0, 250.0, 150.0)), ("c", (28.0, 18.0, 250.0, 130.0))]),
    ("b_p2_lr_v_height_0", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px"><div id=c style="writing-mode:vertical-lr;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (8.0, 8.0, 250.0, 150.0)), ("c", (8.0, 8.0, 10.0, 150.0))]),
    ("b_p2_lr_v_height_m", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px"><div id=c style="margin:10px 20px;writing-mode:vertical-lr;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (28.0, 8.0, 250.0, 150.0)), ("c", (28.0, 18.0, 10.0, 130.0))]),
    ("b_p3_lr_h_width_0", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px;padding:5px"><div id=c style="writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (8.0, 8.0, 250.0, 150.0)), ("c", (13.0, 13.0, 240.0, 10.0))]),
    ("b_p3_lr_h_width_m", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px;padding:5px"><div id=c style="margin:10px 20px;writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (8.0, 8.0, 250.0, 150.0)), ("c", (33.0, 23.0, 200.0, 10.0))]),
    ("b_p3_lr_h_height_0", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px;padding:5px"><div id=c style="writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (8.0, 8.0, 250.0, 150.0)), ("c", (13.0, 13.0, 70.0, 140.0))]),
    ("b_p3_lr_h_height_m", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px;padding:5px"><div id=c style="margin:10px 20px;writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (8.0, 8.0, 250.0, 150.0)), ("c", (33.0, 23.0, 70.0, 120.0))]),
    ("b_p3_lr_v_width_0", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px;padding:5px"><div id=c style="writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (8.0, 8.0, 250.0, 150.0)), ("c", (13.0, 13.0, 240.0, 140.0))]),
    ("b_p3_lr_v_width_m", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px;padding:5px"><div id=c style="margin:10px 20px;writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (8.0, 8.0, 250.0, 150.0)), ("c", (33.0, 23.0, 200.0, 120.0))]),
    ("b_p3_lr_v_height_0", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px;padding:5px"><div id=c style="writing-mode:vertical-lr;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (8.0, 8.0, 250.0, 150.0)), ("c", (13.0, 13.0, 10.0, 140.0))]),
    ("b_p3_lr_v_height_m", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px;padding:5px"><div id=c style="margin:10px 20px;writing-mode:vertical-lr;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (8.0, 8.0, 250.0, 150.0)), ("c", (33.0, 23.0, 10.0, 120.0))]),
    ("b_p4_lr_h_width_0", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px;overflow:hidden"><div id=c style="writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (8.0, 8.0, 250.0, 150.0)), ("c", (8.0, 8.0, 250.0, 10.0))]),
    ("b_p4_lr_h_width_m", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px;overflow:hidden"><div id=c style="margin:10px 20px;writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (8.0, 8.0, 250.0, 150.0)), ("c", (28.0, 18.0, 210.0, 10.0))]),
    ("b_p4_lr_h_height_0", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px;overflow:hidden"><div id=c style="writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (8.0, 8.0, 250.0, 150.0)), ("c", (8.0, 8.0, 70.0, 150.0))]),
    ("b_p4_lr_h_height_m", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px;overflow:hidden"><div id=c style="margin:10px 20px;writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (8.0, 8.0, 250.0, 150.0)), ("c", (28.0, 18.0, 70.0, 130.0))]),
    ("b_p4_lr_v_width_0", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px;overflow:hidden"><div id=c style="writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (8.0, 8.0, 250.0, 150.0)), ("c", (8.0, 8.0, 250.0, 150.0))]),
    ("b_p4_lr_v_width_m", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px;overflow:hidden"><div id=c style="margin:10px 20px;writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (8.0, 8.0, 250.0, 150.0)), ("c", (28.0, 18.0, 210.0, 130.0))]),
    ("b_p4_lr_v_height_0", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px;overflow:hidden"><div id=c style="writing-mode:vertical-lr;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (8.0, 8.0, 250.0, 150.0)), ("c", (8.0, 8.0, 10.0, 150.0))]),
    ("b_p4_lr_v_height_m", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=q style="width:250px;height:150px;overflow:hidden"><div id=c style="margin:10px 20px;writing-mode:vertical-lr;height:stretch;background:red">aaa bbb</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("q", (8.0, 8.0, 250.0, 150.0)), ("c", (28.0, 18.0, 10.0, 130.0))]),
];

#[rustfmt::skip]
const ORTHOGONAL_AND_FLEX: &[Case] = &[
    ("a_hp_vc_rl_width", r#"<div id=p style="width:300px;height:200px"><div id=c style="writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 70.0))]),
    ("a_vp_hc_rl_width", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 10.0))]),
    ("a_vp_vc_rl_width", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("a_vp_vc_m_rl_width", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="margin:10px 20px;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 180.0))]),
    ("a_hp_vc_m_rl_width", r#"<div id=p style="width:300px;height:200px"><div id=c style="margin:10px 20px;writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 10.0, 300.0, 200.0)), ("c", (28.0, 10.0, 260.0, 70.0))]),
    ("a_flexi_rl_width", r#"<div id=p style="display:flex;width:300px;height:200px"><div id=c style="writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("a_flexc_rl_width", r#"<div id=p style="display:flex;flex-direction:column;width:300px;height:200px"><div id=c style="writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 70.0))]),
    ("a_gridi_rl_width", r#"<div id=p style="display:grid;width:300px;height:200px"><div id=c style="writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("a_abs_rl_width", r#"<div id=p style="position:relative;width:300px;height:200px"><div id=c style="position:absolute;writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 70.0))]),
    ("a_hp_vc_rl_height", r#"<div id=p style="width:300px;height:200px"><div id=c style="writing-mode:vertical-rl;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 10.0, 200.0))]),
    ("a_vp_hc_rl_height", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (238.0, 8.0, 70.0, 200.0))]),
    ("a_vp_vc_rl_height", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (298.0, 8.0, 10.0, 200.0))]),
    ("a_vp_vc_m_rl_height", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="margin:10px 20px;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (278.0, 18.0, 10.0, 180.0))]),
    ("a_hp_vc_m_rl_height", r#"<div id=p style="width:300px;height:200px"><div id=c style="margin:10px 20px;writing-mode:vertical-rl;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 10.0, 300.0, 200.0)), ("c", (28.0, 10.0, 10.0, 200.0))]),
    ("a_ib_rl_height", r#"<div id=p style="width:300px;height:200px"><div id=c style="display:inline-block;writing-mode:vertical-rl;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 10.0, 200.0))]),
    ("a_flexi_rl_height", r#"<div id=p style="display:flex;width:300px;height:200px"><div id=c style="writing-mode:vertical-rl;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 10.0, 200.0))]),
    ("a_flexc_rl_height", r#"<div id=p style="display:flex;flex-direction:column;width:300px;height:200px"><div id=c style="writing-mode:vertical-rl;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("a_gridi_rl_height", r#"<div id=p style="display:grid;width:300px;height:200px"><div id=c style="writing-mode:vertical-rl;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("a_abs_rl_height", r#"<div id=p style="position:relative;width:300px;height:200px"><div id=c style="position:absolute;writing-mode:vertical-rl;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 10.0, 200.0))]),
    ("a_hp_vc_lr_width", r#"<div id=p style="width:300px;height:200px"><div id=c style="writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 70.0))]),
    ("a_vp_hc_lr_width", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 10.0))]),
    ("a_vp_vc_lr_width", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("a_vp_vc_m_lr_width", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="margin:10px 20px;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 180.0))]),
    ("a_hp_vc_m_lr_width", r#"<div id=p style="width:300px;height:200px"><div id=c style="margin:10px 20px;writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 10.0, 300.0, 200.0)), ("c", (28.0, 10.0, 260.0, 70.0))]),
    ("a_flexi_lr_width", r#"<div id=p style="display:flex;width:300px;height:200px"><div id=c style="writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("a_flexc_lr_width", r#"<div id=p style="display:flex;flex-direction:column;width:300px;height:200px"><div id=c style="writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 70.0))]),
    ("a_gridi_lr_width", r#"<div id=p style="display:grid;width:300px;height:200px"><div id=c style="writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("a_abs_lr_width", r#"<div id=p style="position:relative;width:300px;height:200px"><div id=c style="position:absolute;writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 70.0))]),
    ("a_hp_vc_lr_height", r#"<div id=p style="width:300px;height:200px"><div id=c style="writing-mode:vertical-lr;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 10.0, 200.0))]),
    ("a_vp_hc_lr_height", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 70.0, 200.0))]),
    ("a_vp_vc_lr_height", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 10.0, 200.0))]),
    ("a_vp_vc_m_lr_height", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="margin:10px 20px;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 10.0, 180.0))]),
    ("a_hp_vc_m_lr_height", r#"<div id=p style="width:300px;height:200px"><div id=c style="margin:10px 20px;writing-mode:vertical-lr;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 10.0, 300.0, 200.0)), ("c", (28.0, 10.0, 10.0, 200.0))]),
    ("a_ib_lr_height", r#"<div id=p style="width:300px;height:200px"><div id=c style="display:inline-block;writing-mode:vertical-lr;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 10.0, 200.0))]),
    ("a_flexi_lr_height", r#"<div id=p style="display:flex;width:300px;height:200px"><div id=c style="writing-mode:vertical-lr;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 10.0, 200.0))]),
    ("a_flexc_lr_height", r#"<div id=p style="display:flex;flex-direction:column;width:300px;height:200px"><div id=c style="writing-mode:vertical-lr;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("a_gridi_lr_height", r#"<div id=p style="display:grid;width:300px;height:200px"><div id=c style="writing-mode:vertical-lr;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("a_abs_lr_height", r#"<div id=p style="position:relative;width:300px;height:200px"><div id=c style="position:absolute;writing-mode:vertical-lr;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 10.0, 200.0))]),
    ("f_flex_rl_h_width_0", r#"<div id=p style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 10.0))]),
    ("f_flex_rl_h_width_m", r#"<div id=p style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;margin:10px 20px;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 10.0))]),
    ("f_flex_rl_h_height_0", r#"<div id=p style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("f_flex_rl_h_height_m", r#"<div id=p style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;margin:10px 20px;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 180.0))]),
    ("f_flex_rl_v_width_0", r#"<div id=p style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 70.0))]),
    ("f_flex_rl_v_width_m", r#"<div id=p style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="margin:10px 20px;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 70.0))]),
    ("f_flex_rl_v_height_0", r#"<div id=p style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("f_flex_rl_v_height_m", r#"<div id=p style="display:flex;writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="margin:10px 20px;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 180.0))]),
    ("f_flex_lr_h_width_0", r#"<div id=p style="display:flex;writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 10.0))]),
    ("f_flex_lr_h_width_m", r#"<div id=p style="display:flex;writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;margin:10px 20px;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 10.0))]),
    ("f_flex_lr_h_height_0", r#"<div id=p style="display:flex;writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("f_flex_lr_h_height_m", r#"<div id=p style="display:flex;writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;margin:10px 20px;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 180.0))]),
    ("f_flex_lr_v_width_0", r#"<div id=p style="display:flex;writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 70.0))]),
    ("f_flex_lr_v_width_m", r#"<div id=p style="display:flex;writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="margin:10px 20px;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 70.0))]),
    ("f_flex_lr_v_height_0", r#"<div id=p style="display:flex;writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("f_flex_lr_v_height_m", r#"<div id=p style="display:flex;writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="margin:10px 20px;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 180.0))]),
    ("f_grid_rl_h_width_0", r#"<div id=p style="display:grid;writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("f_grid_rl_h_width_m", r#"<div id=p style="display:grid;writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;margin:10px 20px;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 180.0))]),
    ("f_grid_rl_h_height_0", r#"<div id=p style="display:grid;writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("f_grid_rl_h_height_m", r#"<div id=p style="display:grid;writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;margin:10px 20px;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 180.0))]),
    ("f_grid_rl_v_width_0", r#"<div id=p style="display:grid;writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("f_grid_rl_v_width_m", r#"<div id=p style="display:grid;writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="margin:10px 20px;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 180.0))]),
    ("f_grid_rl_v_height_0", r#"<div id=p style="display:grid;writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("f_grid_rl_v_height_m", r#"<div id=p style="display:grid;writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="margin:10px 20px;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 180.0))]),
    ("f_grid_lr_h_width_0", r#"<div id=p style="display:grid;writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("f_grid_lr_h_width_m", r#"<div id=p style="display:grid;writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;margin:10px 20px;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 180.0))]),
    ("f_grid_lr_h_height_0", r#"<div id=p style="display:grid;writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("f_grid_lr_h_height_m", r#"<div id=p style="display:grid;writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;margin:10px 20px;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 180.0))]),
    ("f_grid_lr_v_width_0", r#"<div id=p style="display:grid;writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("f_grid_lr_v_width_m", r#"<div id=p style="display:grid;writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="margin:10px 20px;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 180.0))]),
    ("f_grid_lr_v_height_0", r#"<div id=p style="display:grid;writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("f_grid_lr_v_height_m", r#"<div id=p style="display:grid;writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="margin:10px 20px;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 180.0))]),
];

#[rustfmt::skip]
const ATOMIC_AND_FLOAT: &[Case] = &[
    ("c_ib_h_width_0", r#"<div id=p style="width:300px;height:200px"><div id=c style="display:inline-block;writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 10.0))]),
    ("c_ib_h_width_m", r#"<div id=p style="width:300px;height:200px"><div id=c style="display:inline-block;margin:10px 20px;writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 10.0))]),
    ("c_ib_h_height_0", r#"<div id=p style="width:300px;height:200px"><div id=c style="display:inline-block;writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 70.0, 200.0))]),
    ("c_ib_h_height_m", r#"<div id=p style="width:300px;height:200px"><div id=c style="display:inline-block;margin:10px 20px;writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 70.0, 180.0))]),
    ("c_ib_h_pad", r#"<div id=p style="width:300px;height:200px"><div id=c style="display:inline-block;padding:5px;border:3px solid;writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 26.0))]),
    ("c_ib_h_max", r#"<div id=p style="width:300px;height:200px"><div id=c style="display:inline-block;max-width:120px;writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 120.0, 10.0))]),
    ("c_ib_rl_width_0", r#"<div id=p style="width:300px;height:200px"><div id=c style="display:inline-block;writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 70.0))]),
    ("c_ib_rl_width_m", r#"<div id=p style="width:300px;height:200px"><div id=c style="display:inline-block;margin:10px 20px;writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 70.0))]),
    ("c_ib_rl_height_0", r#"<div id=p style="width:300px;height:200px"><div id=c style="display:inline-block;writing-mode:vertical-rl;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 10.0, 200.0))]),
    ("c_ib_rl_height_m", r#"<div id=p style="width:300px;height:200px"><div id=c style="display:inline-block;margin:10px 20px;writing-mode:vertical-rl;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 10.0, 180.0))]),
    ("c_ib_rl_pad", r#"<div id=p style="width:300px;height:200px"><div id=c style="display:inline-block;padding:5px;border:3px solid;writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 86.0))]),
    ("c_ib_rl_max", r#"<div id=p style="width:300px;height:200px"><div id=c style="display:inline-block;max-width:120px;writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 120.0, 70.0))]),
    ("c_ib_lr_width_0", r#"<div id=p style="width:300px;height:200px"><div id=c style="display:inline-block;writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 70.0))]),
    ("c_ib_lr_width_m", r#"<div id=p style="width:300px;height:200px"><div id=c style="display:inline-block;margin:10px 20px;writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 70.0))]),
    ("c_ib_lr_height_0", r#"<div id=p style="width:300px;height:200px"><div id=c style="display:inline-block;writing-mode:vertical-lr;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 10.0, 200.0))]),
    ("c_ib_lr_height_m", r#"<div id=p style="width:300px;height:200px"><div id=c style="display:inline-block;margin:10px 20px;writing-mode:vertical-lr;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 10.0, 180.0))]),
    ("c_ib_lr_pad", r#"<div id=p style="width:300px;height:200px"><div id=c style="display:inline-block;padding:5px;border:3px solid;writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 86.0))]),
    ("c_ib_lr_max", r#"<div id=p style="width:300px;height:200px"><div id=c style="display:inline-block;max-width:120px;writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 120.0, 70.0))]),
    ("c_fl_h_width_0", r#"<div id=p style="width:300px;height:200px"><div id=c style="float:left;writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 10.0))]),
    ("c_fl_h_width_m", r#"<div id=p style="width:300px;height:200px"><div id=c style="float:left;margin:10px 20px;writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 10.0))]),
    ("c_fl_h_height_0", r#"<div id=p style="width:300px;height:200px"><div id=c style="float:left;writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 70.0, 200.0))]),
    ("c_fl_h_height_m", r#"<div id=p style="width:300px;height:200px"><div id=c style="float:left;margin:10px 20px;writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 70.0, 180.0))]),
    ("c_fl_h_pad", r#"<div id=p style="width:300px;height:200px"><div id=c style="float:left;padding:5px;border:3px solid;writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 26.0))]),
    ("c_fl_h_max", r#"<div id=p style="width:300px;height:200px"><div id=c style="float:left;max-width:120px;writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 120.0, 10.0))]),
    ("c_fl_rl_width_0", r#"<div id=p style="width:300px;height:200px"><div id=c style="float:left;writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 70.0))]),
    ("c_fl_rl_width_m", r#"<div id=p style="width:300px;height:200px"><div id=c style="float:left;margin:10px 20px;writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 70.0))]),
    ("c_fl_rl_height_0", r#"<div id=p style="width:300px;height:200px"><div id=c style="float:left;writing-mode:vertical-rl;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 10.0, 200.0))]),
    ("c_fl_rl_height_m", r#"<div id=p style="width:300px;height:200px"><div id=c style="float:left;margin:10px 20px;writing-mode:vertical-rl;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 10.0, 180.0))]),
    ("c_fl_rl_pad", r#"<div id=p style="width:300px;height:200px"><div id=c style="float:left;padding:5px;border:3px solid;writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 86.0))]),
    ("c_fl_rl_max", r#"<div id=p style="width:300px;height:200px"><div id=c style="float:left;max-width:120px;writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 120.0, 70.0))]),
    ("c_fl_lr_width_0", r#"<div id=p style="width:300px;height:200px"><div id=c style="float:left;writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 70.0))]),
    ("c_fl_lr_width_m", r#"<div id=p style="width:300px;height:200px"><div id=c style="float:left;margin:10px 20px;writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 260.0, 70.0))]),
    ("c_fl_lr_height_0", r#"<div id=p style="width:300px;height:200px"><div id=c style="float:left;writing-mode:vertical-lr;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 10.0, 200.0))]),
    ("c_fl_lr_height_m", r#"<div id=p style="width:300px;height:200px"><div id=c style="float:left;margin:10px 20px;writing-mode:vertical-lr;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (28.0, 18.0, 10.0, 180.0))]),
    ("c_fl_lr_pad", r#"<div id=p style="width:300px;height:200px"><div id=c style="float:left;padding:5px;border:3px solid;writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 86.0))]),
    ("c_fl_lr_max", r#"<div id=p style="width:300px;height:200px"><div id=c style="float:left;max-width:120px;writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 120.0, 70.0))]),
    ("a_ib_rl_width", r#"<div id=p style="width:300px;height:200px"><div id=c style="display:inline-block;writing-mode:vertical-rl;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 70.0))]),
    ("a_ib_lr_width", r#"<div id=p style="width:300px;height:200px"><div id=c style="display:inline-block;writing-mode:vertical-lr;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 70.0))]),
];

#[rustfmt::skip]
const EDGE_CASES: &[Case] = &[
    ("d_auto_ib_hc_w_rl", r#"<div id=p style="display:inline-block;writing-mode:vertical-rl"><div id=c style="writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 800.0, 10.0)), ("c", (8.0, 8.0, 800.0, 10.0))]),
    ("d_auto_ib_vc_w_rl", r#"<div id=p style="display:inline-block;writing-mode:vertical-rl"><div id=c style="width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 10.0, 70.0)), ("c", (8.0, 8.0, 10.0, 70.0))]),
    ("d_auto_ib_hc_h_rl", r#"<div id=p style="display:inline-block;writing-mode:vertical-rl"><div id=c style="writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 70.0, 10.0)), ("c", (8.0, 8.0, 70.0, 10.0))]),
    ("d_autoh_hc_h_rl", r#"<div id=p style="writing-mode:vertical-rl;width:300px"><div id=c style="writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 10.0)), ("c", (238.0, 8.0, 70.0, 10.0))]),
    ("d_lim_hc_w_rl", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;width:stretch;max-width:120px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (188.0, 8.0, 120.0, 10.0))]),
    ("d_lim_hc_h_rl", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;height:stretch;max-height:80px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (238.0, 8.0, 70.0, 80.0))]),
    ("d_lim_vc_w_rl", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="width:stretch;max-width:120px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (188.0, 8.0, 120.0, 200.0))]),
    ("d_lim_vc_minw_rl", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="width:stretch;min-width:350px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (-42.0, 8.0, 350.0, 200.0))]),
    ("d_pad_hc_w_rl", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;width:stretch;padding:5px;border:3px solid;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 26.0))]),
    ("d_pad_hc_h_rl", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;height:stretch;padding:5px;border:3px solid;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (222.0, 8.0, 86.0, 200.0))]),
    ("d_pad_vc_w_rl", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="width:stretch;padding:5px;border:3px solid;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("d_bb_hc_w_rl", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="box-sizing:border-box;writing-mode:horizontal-tb;width:stretch;padding:5px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 20.0))]),
    ("d_am_hc_w_rl", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;width:stretch;margin:0 auto;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 10.0))]),
    ("d_am_hc_h_rl", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;height:stretch;margin:auto 0;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (238.0, 8.0, 70.0, 200.0))]),
    ("d_both_hc_rl", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;width:stretch;height:stretch;margin:5px 15px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (23.0, 13.0, 270.0, 190.0))]),
    ("d_both_vc_rl", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="width:stretch;height:stretch;margin:5px 15px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (23.0, 13.0, 270.0, 190.0))]),
    ("d_nest_rl", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="width:stretch;height:stretch;margin:5px 15px;padding:4px;background:red"><div id=d style="width:stretch;height:stretch;margin:2px 3px;writing-mode:horizontal-tb">aaa</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (23.0, 13.0, 270.0, 190.0)), ("d", (30.0, 19.0, 256.0, 178.0))]),
    ("d_pct_rl", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="width:stretch;background:red"><svg id=s width=100 height=50 style="width:50%"></svg></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0)), ("s", (158.0, 8.0, 150.0, 50.0))]),
    ("d_hp_m_rl", r#"<div id=p style="width:300px;height:200px;border:2px solid"><div id=c style="margin:10px 20px;writing-mode:vertical-rl;width:stretch;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (30.0, 20.0, 256.0, 176.0))]),
    ("d_rtl_hc_rl", r#"<div id=p style="writing-mode:vertical-rl;direction:rtl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;width:stretch;height:stretch;margin:5px 15px 25px 35px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (43.0, 13.0, 250.0, 170.0))]),
    ("d_sib_rl", r#"<div id=p style="writing-mode:vertical-rl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;width:stretch;height:stretch;background:red">aaa</div><div id=d style="writing-mode:horizontal-tb;width:stretch;height:stretch;background:blue">bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0)), ("d", (-292.0, 8.0, 300.0, 200.0))]),
    ("d_auto_ib_hc_w_lr", r#"<div id=p style="display:inline-block;writing-mode:vertical-lr"><div id=c style="writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 800.0, 10.0)), ("c", (8.0, 8.0, 800.0, 10.0))]),
    ("d_auto_ib_vc_w_lr", r#"<div id=p style="display:inline-block;writing-mode:vertical-lr"><div id=c style="width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 10.0, 70.0)), ("c", (8.0, 8.0, 10.0, 70.0))]),
    ("d_auto_ib_hc_h_lr", r#"<div id=p style="display:inline-block;writing-mode:vertical-lr"><div id=c style="writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 70.0, 10.0)), ("c", (8.0, 8.0, 70.0, 10.0))]),
    ("d_autoh_hc_h_lr", r#"<div id=p style="writing-mode:vertical-lr;width:300px"><div id=c style="writing-mode:horizontal-tb;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 10.0)), ("c", (8.0, 8.0, 70.0, 10.0))]),
    ("d_lim_hc_w_lr", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;width:stretch;max-width:120px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 120.0, 10.0))]),
    ("d_lim_hc_h_lr", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;height:stretch;max-height:80px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 70.0, 80.0))]),
    ("d_lim_vc_w_lr", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="width:stretch;max-width:120px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 120.0, 200.0))]),
    ("d_lim_vc_minw_lr", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="width:stretch;min-width:350px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 350.0, 200.0))]),
    ("d_pad_hc_w_lr", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;width:stretch;padding:5px;border:3px solid;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 26.0))]),
    ("d_pad_hc_h_lr", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;height:stretch;padding:5px;border:3px solid;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 86.0, 200.0))]),
    ("d_pad_vc_w_lr", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="width:stretch;padding:5px;border:3px solid;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0))]),
    ("d_bb_hc_w_lr", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="box-sizing:border-box;writing-mode:horizontal-tb;width:stretch;padding:5px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 20.0))]),
    ("d_am_hc_w_lr", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;width:stretch;margin:0 auto;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 10.0))]),
    ("d_am_hc_h_lr", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;height:stretch;margin:auto 0;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 70.0, 200.0))]),
    ("d_both_hc_lr", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;width:stretch;height:stretch;margin:5px 15px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (23.0, 13.0, 270.0, 190.0))]),
    ("d_both_vc_lr", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="width:stretch;height:stretch;margin:5px 15px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (23.0, 13.0, 270.0, 190.0))]),
    ("d_nest_lr", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="width:stretch;height:stretch;margin:5px 15px;padding:4px;background:red"><div id=d style="width:stretch;height:stretch;margin:2px 3px;writing-mode:horizontal-tb">aaa</div></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (23.0, 13.0, 270.0, 190.0)), ("d", (30.0, 19.0, 256.0, 178.0))]),
    ("d_pct_lr", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="width:stretch;background:red"><svg id=s width=100 height=50 style="width:50%"></svg></div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0)), ("s", (8.0, 8.0, 150.0, 50.0))]),
    ("d_hp_m_lr", r#"<div id=p style="width:300px;height:200px;border:2px solid"><div id=c style="margin:10px 20px;writing-mode:vertical-lr;width:stretch;height:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (30.0, 20.0, 256.0, 176.0))]),
    ("d_rtl_hc_lr", r#"<div id=p style="writing-mode:vertical-lr;direction:rtl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;width:stretch;height:stretch;margin:5px 15px 25px 35px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (43.0, 13.0, 250.0, 170.0))]),
    ("d_sib_lr", r#"<div id=p style="writing-mode:vertical-lr;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;width:stretch;height:stretch;background:red">aaa</div><div id=d style="writing-mode:horizontal-tb;width:stretch;height:stretch;background:blue">bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (8.0, 8.0, 300.0, 200.0)), ("d", (308.0, 8.0, 300.0, 200.0))]),
    ("d_sw_hc_rls", r#"<div id=p style="writing-mode:sideways-rl;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;width:stretch;height:stretch;margin:5px 15px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (23.0, 13.0, 270.0, 190.0))]),
    ("d_sw_vc_rls", r#"<div id=p style="writing-mode:sideways-rl;width:300px;height:200px"><div id=c style="width:stretch;height:stretch;margin:5px 15px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (23.0, 13.0, 270.0, 190.0))]),
    ("d_sw_hc_lrs", r#"<div id=p style="writing-mode:sideways-lr;width:300px;height:200px"><div id=c style="writing-mode:horizontal-tb;width:stretch;height:stretch;margin:5px 15px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (23.0, 13.0, 270.0, 190.0))]),
    ("d_sw_vc_lrs", r#"<div id=p style="writing-mode:sideways-lr;width:300px;height:200px"><div id=c style="width:stretch;height:stretch;margin:5px 15px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 200.0)), ("c", (23.0, 13.0, 270.0, 190.0))]),
    ("e_i_m", r#"<div id=p style="display:inline-block;writing-mode:vertical-rl"><div id=c style="writing-mode:horizontal-tb;width:stretch;margin:5px 30px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 800.0, 20.0)), ("c", (38.0, 13.0, 740.0, 10.0))]),
    ("e_i_pad", r#"<div id=p style="display:inline-block;writing-mode:vertical-rl;padding:7px"><div id=c style="writing-mode:horizontal-tb;width:stretch;margin:5px 30px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 814.0, 34.0)), ("c", (45.0, 20.0, 740.0, 10.0))]),
    ("e_i_fl", r#"<div id=p style="float:left;writing-mode:vertical-rl"><div id=c style="writing-mode:horizontal-tb;width:stretch;margin:5px 30px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 800.0, 20.0)), ("c", (38.0, 13.0, 740.0, 10.0))]),
    ("e_i_max", r#"<div id=p style="display:inline-block;writing-mode:vertical-rl"><div id=c style="writing-mode:horizontal-tb;width:stretch;max-width:300px;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 300.0, 10.0)), ("c", (8.0, 8.0, 300.0, 10.0))]),
    ("e_i_mc", r#"<div id=p style="display:inline-block;writing-mode:vertical-rl;width:min-content"><div id=c style="writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 800.0, 10.0)), ("c", (8.0, 8.0, 800.0, 10.0))]),
    ("e_i_two", r#"<div id=p style="display:inline-block;writing-mode:vertical-rl"><div id=c style="writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div><div id=d style="writing-mode:horizontal-tb;background:blue">aaa bbb ccc</div></div>"#, &[("p", (8.0, 8.0, 910.0, 10.0)), ("c", (118.0, 8.0, 800.0, 10.0)), ("d", (8.0, 8.0, 110.0, 10.0))]),
    ("e_i_abs", r#"<div id=p style="position:absolute;writing-mode:vertical-rl"><div id=c style="writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 800.0, 10.0)), ("c", (8.0, 8.0, 800.0, 10.0))]),
    ("e_i_blk", r#"<div id=p style="writing-mode:vertical-rl"><div id=c style="writing-mode:horizontal-tb;width:stretch;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 800.0, 10.0)), ("c", (8.0, 8.0, 800.0, 10.0))]),
    ("e_i_fit", r#"<div id=p style="display:inline-block;writing-mode:vertical-rl"><div id=c style="writing-mode:horizontal-tb;width:fit-content;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 70.0, 10.0)), ("c", (8.0, 8.0, 70.0, 10.0))]),
    ("e_i_auto", r#"<div id=p style="display:inline-block;writing-mode:vertical-rl"><div id=c style="writing-mode:horizontal-tb;background:red">aaa bbb</div></div>"#, &[("p", (8.0, 8.0, 70.0, 10.0)), ("c", (8.0, 8.0, 70.0, 10.0))]),
];
