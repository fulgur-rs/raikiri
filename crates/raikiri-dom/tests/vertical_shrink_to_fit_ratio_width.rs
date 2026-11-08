//! A box in a vertical writing mode whose `auto` width its parent does not fix
//! (an inline-block, a float, an absolutely positioned box, an inline flex or
//! grid container) settles its height, the inline size, first and takes its
//! width, the block size, from that height through its `aspect-ratio`. The
//! height is the one it states or the one its content takes, held to the
//! bounds the width carries over through the ratio (`max-width: 30px` makes a
//! `2/1` box 15px tall). The content raises the width unless the box scrolls
//! or states a `min-width`; with a stated height the width is definite, so
//! the percentage widths of a horizontal child refer to it.
//!
//! The expected border boxes were recorded from Chromium for the same markup in
//! an 800px by 600px viewport with no body margin. Some cases matched before
//! and guard the parts that already worked: block-level boxes, horizontal
//! boxes, and boxes with a stated width.

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
fn a_vertical_box_takes_its_width_from_the_height_its_content_takes() {
    CONTENT.iter().for_each(check_case);
}

#[test]
fn a_vertical_box_takes_its_width_from_the_height_it_states() {
    STATED.iter().for_each(check_case);
}

#[test]
fn nested_vertical_ratio_boxes_settle_once_per_level() {
    // Twelve inline-blocks nested in each other each measure their content
    // without the ratio and lay it out again at the width the ratio gives.
    // The size settled for the same request is kept, so the depth does not
    // multiply the work; Chromium makes every level 20px square.
    let mut body = String::from("bb");
    for level in 0..12 {
        body = format!(
            r#"<div id=n{level} style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:1/1">{body}</div>"#
        );
    }
    let expected: Vec<(String, Rect)> = (0..12)
        .map(|level| (format!("n{level}"), (0.0, 0.0, 20.0, 20.0)))
        .collect();
    let expected: Vec<(&str, Rect)> = expected
        .iter()
        .map(|(id, rect)| (id.as_str(), *rect))
        .collect();
    let started = std::time::Instant::now();
    check("nest12", &body, &expected);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "nested ratio boxes took {:?}",
        started.elapsed()
    );
}

#[rustfmt::skip]
const STATED: &[Case] = &[
    ("arh_fl_h100_ib50", r#"<div id=r style="float:left;writing-mode:vertical-rl;aspect-ratio:2/1;height:100px"><div id=c style="display:inline-block;writing-mode:horizontal-tb;width:50%">bb</div></div>"#, &[("r", (0.0, 0.0, 200.0, 100.0)), ("c", (100.0, 0.0, 100.0, 10.0))]),
    ("arh_ib_h100_ib50", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1;height:100px"><div id=c style="display:inline-block;writing-mode:horizontal-tb;width:50%">bb</div></div>"#, &[("r", (0.0, 0.0, 200.0, 100.0)), ("c", (100.0, 0.0, 100.0, 10.0))]),
    ("arh_fl_h100_ibauto", r#"<div id=r style="float:left;writing-mode:vertical-rl;aspect-ratio:2/1;height:100px"><div id=c style="display:inline-block;writing-mode:horizontal-tb">bb</div></div>"#, &[("r", (0.0, 0.0, 200.0, 100.0)), ("c", (180.0, 0.0, 20.0, 10.0))]),
    ("arh_fl_h100_text", r#"<div id=r style="float:left;writing-mode:vertical-rl;aspect-ratio:2/1;height:100px">bb</div>"#, &[("r", (0.0, 0.0, 200.0, 100.0))]),
    ("arh_fl_h100_blk50", r#"<div id=r style="float:left;writing-mode:vertical-rl;aspect-ratio:2/1;height:100px"><div id=c style="writing-mode:horizontal-tb;width:50%">bb</div></div>"#, &[("r", (0.0, 0.0, 200.0, 100.0)), ("c", (100.0, 0.0, 100.0, 10.0))]),
    ("arh_fl_h100_vib50", r#"<div id=r style="float:left;writing-mode:vertical-rl;aspect-ratio:2/1;height:100px"><div id=c style="display:inline-block;height:50%">bb</div></div>"#, &[("r", (0.0, 0.0, 200.0, 100.0)), ("c", (190.0, 0.0, 10.0, 50.0))]),
    ("arh_hfl_h100_ib50", r#"<div id=r style="float:left;aspect-ratio:2/1;height:100px"><div id=c style="display:inline-block;width:50%">bb</div></div>"#, &[("r", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 100.0, 10.0))]),
    ("arh_hfl_h100_vib50", r#"<div id=r style="float:left;aspect-ratio:2/1;height:100px"><div id=c style="display:inline-block;writing-mode:vertical-rl;width:50%">bb</div></div>"#, &[("r", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 100.0, 20.0))]),
    ("f_ib_h10_wide", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1;height:10px"><div id=c style="display:inline-block;writing-mode:horizontal-tb">bbbbbb</div></div>"#, &[("r", (0.0, 0.0, 60.0, 10.0)), ("c", (0.0, 0.0, 60.0, 10.0))]),
    ("f_ib_h10_vtext", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1;height:10px">b b b b b b</div>"#, &[("r", (0.0, 0.0, 60.0, 10.0))]),
    ("f_ib_h100_vtext", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1;height:100px">b b b b b b</div>"#, &[("r", (0.0, 0.0, 200.0, 100.0))]),
    ("f_ib_h20_vtext_many", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:1/1;height:20px">bb bb bb bb bb bb</div>"#, &[("r", (0.0, 0.0, 60.0, 20.0))]),
    ("f_ib_h100_ovh_wide", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1;height:10px;overflow:hidden"><div id=c style="display:inline-block;writing-mode:horizontal-tb">bbbbbb</div></div>"#, &[("r", (0.0, 0.0, 20.0, 10.0)), ("c", (-40.0, 0.0, 60.0, 10.0))]),
    ("f_ib_h100_minw0_wide", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1;height:10px;min-width:0"><div id=c style="display:inline-block;writing-mode:horizontal-tb">bbbbbb</div></div>"#, &[("r", (0.0, 0.0, 20.0, 10.0)), ("c", (-40.0, 0.0, 60.0, 10.0))]),
    ("f_ib_h100_ib50_max", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1;height:100px"><div id=c style="display:inline-block;writing-mode:horizontal-tb;width:50%;min-width:30%">bb</div></div>"#, &[("r", (0.0, 0.0, 200.0, 100.0)), ("c", (100.0, 0.0, 100.0, 10.0))]),
    ("f_ib_h100_pad", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1;height:100px;padding:5px"><div id=c style="display:inline-block;writing-mode:horizontal-tb;width:50%">bb</div></div>"#, &[("r", (0.0, 0.0, 200.0, 100.0)), ("c", (100.0, 5.0, 95.0, 10.0))]),
    ("f_ib_h100_cb", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1;height:100px;padding:5px;box-sizing:content-box"><div id=c style="display:inline-block;writing-mode:horizontal-tb;width:50%">bb</div></div>"#, &[("r", (0.0, 0.0, 210.0, 110.0)), ("c", (105.0, 5.0, 100.0, 10.0))]),
    ("g_fl_px300", r#"<div id=r style="float:left;writing-mode:vertical-rl;aspect-ratio:2/1;height:100px"><div id=c style="display:inline-block;writing-mode:horizontal-tb;width:300px">bb</div></div>"#, &[("r", (0.0, 0.0, 300.0, 100.0)), ("c", (0.0, 0.0, 300.0, 10.0))]),
    ("g_fl_blk_px300", r#"<div id=r style="float:left;writing-mode:vertical-rl;aspect-ratio:2/1;height:100px"><div id=c style="writing-mode:horizontal-tb;width:300px">bb</div></div>"#, &[("r", (0.0, 0.0, 300.0, 100.0)), ("c", (0.0, 0.0, 300.0, 10.0))]),
    ("g_abs_h100_ib50", r#"<div id=r style="position:absolute;writing-mode:vertical-rl;aspect-ratio:2/1;height:100px"><div id=c style="display:inline-block;writing-mode:horizontal-tb;width:50%">bb</div></div>"#, &[("r", (0.0, 0.0, 200.0, 100.0)), ("c", (100.0, 0.0, 100.0, 10.0))]),
    ("g_lr_ib_h100_ib50", r#"<div id=r style="display:inline-block;writing-mode:vertical-lr;aspect-ratio:2/1;height:100px"><div id=c style="display:inline-block;writing-mode:horizontal-tb;width:50%">bb</div></div>"#, &[("r", (0.0, 0.0, 200.0, 100.0)), ("c", (0.0, 0.0, 100.0, 10.0))]),
    ("g_ib_h_pct", r#"<div style="height:100px"><div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1;height:50%">bbbb</div></div>"#, &[("r", (0.0, 0.0, 100.0, 50.0))]),
    ("r_abs_child_left", r#"<div id=r style="display:inline-block;position:relative;writing-mode:vertical-rl;aspect-ratio:2/1;height:10px"><div id=c style="position:absolute;left:-200px;width:10px;height:10px"></div></div>"#, &[("r", (0.0, 0.0, 20.0, 10.0)), ("c", (-200.0, 0.0, 10.0, 10.0))]),
    ("r_rel_child", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1;height:10px"><div id=c style="position:relative;left:-100px;writing-mode:horizontal-tb;width:10px;height:10px"></div></div>"#, &[("r", (0.0, 0.0, 20.0, 10.0)), ("c", (-90.0, 0.0, 10.0, 10.0))]),
    ("r_flex_abs", r#"<div style="display:flex;position:relative"><div id=r style="position:absolute;writing-mode:vertical-rl;aspect-ratio:2/1;height:100px">bb</div></div>"#, &[("r", (0.0, 0.0, 200.0, 100.0))]),
    ("r_abs_pct_h", r#"<div style="writing-mode:vertical-rl;position:relative;height:200px;width:300px"><div id=r style="position:absolute;aspect-ratio:2/1;height:50%">bb</div></div>"#, &[("r", (100.0, 0.0, 200.0, 100.0))]),
];

#[rustfmt::skip]
const CONTENT: &[Case] = &[
    ("nh_rl_blk_ar", r#"<div style="writing-mode:vertical-rl;height:200px"><div id=r style="aspect-ratio:2/1">b</div></div>"#, &[("r", (0.0, 0.0, 400.0, 200.0))]),
    ("nh_rl_blk_ar_empty", r#"<div style="writing-mode:vertical-rl;height:200px"><div id=r style="aspect-ratio:2/1"></div></div>"#, &[("r", (0.0, 0.0, 400.0, 200.0))]),
    ("nh_rl_blk_ar_w40", r#"<div style="writing-mode:vertical-rl;height:200px"><div id=r style="aspect-ratio:2/1;width:40px"></div></div>"#, &[("r", (0.0, 0.0, 40.0, 20.0))]),
    ("nh_rl_ib_ar", r#"<div style="writing-mode:vertical-rl;height:200px"><div id=r style="display:inline-block;aspect-ratio:2/1">b</div></div>"#, &[("r", (0.0, 0.0, 20.0, 10.0))]),
    ("nh_rl_fl_ar", r#"<div style="writing-mode:vertical-rl;height:200px"><div id=r style="float:left;aspect-ratio:2/1">b</div></div>"#, &[("r", (0.0, 0.0, 20.0, 10.0))]),
    ("nh_rl_root_ar", r#"<div id=r style="writing-mode:vertical-rl;aspect-ratio:2/1">b</div>"#, &[("r", (0.0, 0.0, 20.0, 10.0))]),
    ("nh_rl_hchild_ar", r#"<div style="writing-mode:vertical-rl;height:200px"><div id=r style="writing-mode:horizontal-tb;aspect-ratio:2/1">b</div></div>"#, &[("r", (0.0, 0.0, 10.0, 10.0))]),
    ("nh_h_blk_ar", r#"<div style="width:200px"><div id=r style="aspect-ratio:2/1">b</div></div>"#, &[("r", (0.0, 0.0, 200.0, 100.0))]),
    ("n_ib_text", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1">bbbb</div>"#, &[("r", (0.0, 0.0, 80.0, 40.0))]),
    ("n_ib_text_wrap", r#"<div style="height:30px;writing-mode:vertical-rl"><div id=r style="display:inline-block;aspect-ratio:2/1">bb bb bb</div></div>"#, &[("r", (0.0, 0.0, 60.0, 30.0))]),
    ("n_ib_text_maxw", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1;max-width:30px">bbbb</div>"#, &[("r", (0.0, 0.0, 30.0, 15.0))]),
    ("n_ib_text_minh", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1;min-height:50px">bb</div>"#, &[("r", (0.0, 0.0, 100.0, 50.0))]),
    ("n_ib_text_pad", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1;padding:3px">bbbb</div>"#, &[("r", (0.0, 0.0, 92.0, 46.0))]),
    ("n_ib_text_cb", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1;padding:3px;box-sizing:content-box">bbbb</div>"#, &[("r", (0.0, 0.0, 86.0, 46.0))]),
    ("n_ib_empty", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1"></div>"#, &[("r", (0.0, 8.0, 0.0, 0.0))]),
    ("n_fl_two_lines", r#"<div id=r style="float:left;writing-mode:vertical-rl;aspect-ratio:1/1"><div>bbbb</div><div>bb</div></div>"#, &[("r", (0.0, 0.0, 40.0, 40.0))]),
    ("n_abs_text", r#"<div id=r style="position:absolute;writing-mode:vertical-rl;aspect-ratio:2/1">bbbb</div>"#, &[("r", (0.0, 0.0, 80.0, 40.0))]),
    ("n_ib_hchild", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:1/1"><div id=c style="writing-mode:horizontal-tb">bbbbbb</div></div>"#, &[("r", (0.0, 0.0, 60.0, 10.0)), ("c", (0.0, 0.0, 60.0, 10.0))]),
    ("n_hib_vtext", r#"<div id=r style="display:inline-block;aspect-ratio:1/2">bbbb</div>"#, &[("r", (0.0, 0.0, 40.0, 80.0))]),
    ("n_inlinegrid", r#"<div id=r style="display:inline-grid;writing-mode:vertical-rl;aspect-ratio:2/1">bbbb</div>"#, &[("r", (0.0, 0.0, 80.0, 40.0))]),
    ("n_inlineflex", r#"<div id=r style="display:inline-flex;writing-mode:vertical-rl;aspect-ratio:2/1">bbbb</div>"#, &[("r", (0.0, 0.0, 80.0, 40.0))]),
    ("g_lr_n_text", r#"<div id=r style="display:inline-block;writing-mode:vertical-lr;aspect-ratio:2/1">bbbb</div>"#, &[("r", (0.0, 0.0, 80.0, 40.0))]),
    ("g_ib_text_two_ib", r#"<div style="writing-mode:vertical-rl;height:100px"><span id=a style="display:inline-block;aspect-ratio:1/1">bb</span><span id=b style="display:inline-block;aspect-ratio:1/2">bbbb</span></div>"#, &[("a", (0.0, 0.0, 20.0, 20.0)), ("b", (0.0, 20.0, 20.0, 40.0))]),
    ("g_ib_text_border", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1;border:2px solid">bbbb</div>"#, &[("r", (0.0, 0.0, 88.0, 44.0))]),
    ("g_ib_text_minw", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1;min-width:100px">bb</div>"#, &[("r", (0.0, 0.0, 100.0, 50.0))]),
    ("g_ib_text_maxh", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1;max-height:20px">bbbb</div>"#, &[("r", (0.0, 0.0, 40.0, 20.0))]),
    ("g_ib_text_inside_h", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:1/1"><div id=c style="height:30px">b</div></div>"#, &[("r", (0.0, 0.0, 30.0, 30.0)), ("c", (20.0, 0.0, 10.0, 30.0))]),
    ("g_ib_ovh_text", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:1/4;overflow:hidden">bbbb</div>"#, &[("r", (0.0, 0.0, 10.0, 40.0))]),
    ("g_ib_auto_ratio", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:auto 2/1">bbbb</div>"#, &[("r", (0.0, 0.0, 80.0, 40.0))]),
    ("g_ib_nested", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1"><div id=c style="display:inline-block;aspect-ratio:1/1">bb</div></div>"#, &[("r", (0.0, 0.0, 40.0, 20.0)), ("c", (20.0, 0.0, 20.0, 20.0))]),
    ("g_tbl_cell", r#"<table style="writing-mode:vertical-rl"><tr><td><div id=r style="display:inline-block;aspect-ratio:2/1">bb</div></td></tr></table>"#, &[("r", (3.0, 3.0, 40.0, 20.0))]),
    ("r_imposed_maxw", r#"<div style="writing-mode:vertical-rl;height:200px"><div id=r style="aspect-ratio:2/1;max-width:30px"></div></div>"#, &[("r", (0.0, 0.0, 30.0, 15.0))]),
    ("r_float_after_float", r#"<div style="writing-mode:vertical-rl;height:100px"><div style="float:left;width:20px;height:50px"></div><div id=r style="float:left;aspect-ratio:2/1">bbbb</div></div>"#, &[("r", (0.0, 50.0, 80.0, 40.0))]),
    ("r_nested3", r#"<div id=r style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1"><div id=a style="display:inline-block;aspect-ratio:1/1"><div id=b style="display:inline-block;aspect-ratio:1/2">bb</div></div></div>"#, &[("r", (0.0, 0.0, 40.0, 20.0)), ("a", (20.0, 0.0, 20.0, 20.0)), ("b", (30.0, 0.0, 10.0, 20.0))]),
    ("r_vlr_float", r#"<div id=r style="float:left;writing-mode:vertical-lr;aspect-ratio:3/1">bb bb</div>"#, &[("r", (0.0, 0.0, 150.0, 50.0))]),
];
