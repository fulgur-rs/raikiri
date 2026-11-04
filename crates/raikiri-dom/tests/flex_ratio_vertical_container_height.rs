//! A flex or grid container on the axes of a vertical writing mode, with an
//! `aspect-ratio` and an `auto` height, takes its height from its width through
//! the ratio, and its content raises that height to its min-content size along
//! the lines, up to the `max-height` it states, unless it scrolls or states a
//! `min-height`: a `2 / 1` container `100px` wide with content `100px` high is
//! `100px` high, not `50px`. The same holds for such a container that is an
//! item of a flex row, where the flex algorithm leaves the height to the item;
//! as an item of a flex column it keeps the width its ratio gives from the
//! height it states.
//!
//! The expected border boxes were recorded from Chromium for the same markup in
//! an 800px by 600px viewport, with the body margin of 8px.

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

/// Run every case whose name satisfies `select`, and make sure some do.
fn check_group(select: fn(&str) -> bool) {
    let cases: Vec<&Case> = CASES.iter().filter(|case| select(case.0)).collect();
    assert!(!cases.is_empty(), "no case selected");
    for case in cases {
        check_case(case);
    }
}

fn plain_containers(name: &str) -> bool {
    name.starts_with("a_")
}

#[test]
fn a_container_on_vertical_axes_takes_its_height_from_the_ratio_and_its_content() {
    check_group(plain_containers);
}

fn block_level(name: &str) -> bool {
    name.starts_with("b_") && name.ends_with("_block")
}

#[test]
fn a_block_level_container_takes_its_height_from_the_ratio_and_its_content() {
    check_group(block_level);
}

fn inline_level(name: &str) -> bool {
    name.starts_with("b_") && name.ends_with("_inline")
}

#[test]
fn an_inline_level_container_takes_its_height_from_the_ratio_and_its_content() {
    check_group(inline_level);
}

fn flex_items(name: &str) -> bool {
    name.starts_with("b_") && name.contains("_ctx_flex")
}

#[test]
fn a_container_that_is_a_flex_item_takes_its_height_from_the_ratio_and_its_content() {
    check_group(flex_items);
}

fn other_contexts(name: &str) -> bool {
    name.starts_with("b_")
        && !name.contains("_ctx_flex")
        && !name.ends_with("_block")
        && !name.ends_with("_inline")
}

#[test]
fn a_container_in_other_contexts_takes_its_height_from_the_ratio_and_its_content() {
    check_group(other_contexts);
}

fn limits_and_scrolling(name: &str) -> bool {
    name.starts_with("e_")
}

#[test]
fn a_container_that_scrolls_or_states_limits_keeps_the_height_the_ratio_gives() {
    check_group(limits_and_scrolling);
}

fn items_of_flex_and_grid_parents(name: &str) -> bool {
    name.starts_with("f_")
}

#[test]
fn a_container_that_is_an_item_of_a_flex_or_grid_parent_takes_its_height_from_the_ratio() {
    check_group(items_of_flex_and_grid_parents);
}

/// A case that no group selects would never be run.
#[test]
fn every_case_is_in_a_group() {
    let groups: &[fn(&str) -> bool] = &[
        plain_containers,
        block_level,
        inline_level,
        flex_items,
        other_contexts,
        limits_and_scrolling,
        items_of_flex_and_grid_parents,
    ];
    for case in CASES {
        assert!(
            groups.iter().any(|group| group(case.0)),
            "{}: no group runs it",
            case.0
        );
    }
}

/// Every case: name, markup and the border boxes Chromium lays out for it.
#[rustfmt::skip]
const CASES: &[Case] = &[
    ("a_flex_h_widen", r#"<div id=i style="display:inline-flex;height:100px;aspect-ratio:2/1"><div style="width:300px;height:100%"></div></div>"#, &[("i", (8.0, 8.0, 300.0, 100.0))]),
    ("a_flex_h_widen_scroll", r#"<div id=i style="display:inline-flex;height:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:300px;height:100%"></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("a_flex_h_widen_blockw", r#"<div style="width:500px"><div id=i style="display:flex;width:100px;aspect-ratio:1/2"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("a_flex_vertical-rl_w100_ar2", r#"<div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("a_flex_vertical-rl_w100_ar2_scroll", r#"<div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("a_flex_vertical-rl_w100_ar2_minh0", r#"<div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:0"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("a_flex_vertical-rl_h100_ar2", r#"<div id=i style="display:flex;writing-mode:vertical-rl;height:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("a_flex_vertical-rl_w100_ar2_tall", r#"<div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("a_flex_vertical-rl_w100_ar2_short", r#"<div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:20px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("a_flex_vertical-rl_w100_ar2_maxh", r#"<div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:80px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 80.0))]),
    ("a_flex_vertical-rl_w100_ar2_minh", r#"<div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:150px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 150.0))]),
    ("a_flex_vertical-rl_inline_w100_ar2", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("a_flex_vertical-rl_in_block", r#"<div style="width:300px"><div id=i style="display:flex;writing-mode:vertical-rl;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("a_flex_vertical-rl_col_w100_ar2", r#"<div id=i style="display:flex;flex-direction:column;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("a_flex_vertical-lr_w100_ar2", r#"<div id=i style="display:flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("a_flex_vertical-lr_w100_ar2_scroll", r#"<div id=i style="display:flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("a_flex_vertical-lr_w100_ar2_minh0", r#"<div id=i style="display:flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;min-height:0"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("a_flex_vertical-lr_h100_ar2", r#"<div id=i style="display:flex;writing-mode:vertical-lr;height:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("a_flex_vertical-lr_w100_ar2_tall", r#"<div id=i style="display:flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("a_flex_vertical-lr_w100_ar2_short", r#"<div id=i style="display:flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:20px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("a_flex_vertical-lr_w100_ar2_maxh", r#"<div id=i style="display:flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;max-height:80px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 80.0))]),
    ("a_flex_vertical-lr_w100_ar2_minh", r#"<div id=i style="display:flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;min-height:150px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 150.0))]),
    ("a_flex_vertical-lr_inline_w100_ar2", r#"<div id=i style="display:inline-flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("a_flex_vertical-lr_in_block", r#"<div style="width:300px"><div id=i style="display:flex;writing-mode:vertical-lr;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("a_flex_vertical-lr_col_w100_ar2", r#"<div id=i style="display:flex;flex-direction:column;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("a_grid_h_widen", r#"<div id=i style="display:inline-grid;height:100px;aspect-ratio:2/1"><div style="width:300px;height:100%"></div></div>"#, &[("i", (8.0, 8.0, 300.0, 100.0))]),
    ("a_grid_h_widen_scroll", r#"<div id=i style="display:inline-grid;height:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:300px;height:100%"></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("a_grid_h_widen_blockw", r#"<div style="width:500px"><div id=i style="display:grid;width:100px;aspect-ratio:1/2"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 200.0))]),
    ("a_grid_vertical-rl_w100_ar2", r#"<div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("a_grid_vertical-rl_w100_ar2_scroll", r#"<div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("a_grid_vertical-rl_w100_ar2_minh0", r#"<div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:0"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("a_grid_vertical-rl_h100_ar2", r#"<div id=i style="display:grid;writing-mode:vertical-rl;height:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("a_grid_vertical-rl_w100_ar2_tall", r#"<div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("a_grid_vertical-rl_w100_ar2_short", r#"<div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:20px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("a_grid_vertical-rl_w100_ar2_maxh", r#"<div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:80px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 80.0))]),
    ("a_grid_vertical-rl_w100_ar2_minh", r#"<div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:150px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 150.0))]),
    ("a_grid_vertical-rl_inline_w100_ar2", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("a_grid_vertical-rl_col_w100_ar2", r#"<div id=i style="display:grid;flex-direction:column;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("a_grid_vertical-lr_w100_ar2", r#"<div id=i style="display:grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("a_grid_vertical-lr_w100_ar2_scroll", r#"<div id=i style="display:grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("a_grid_vertical-lr_w100_ar2_minh0", r#"<div id=i style="display:grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;min-height:0"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("a_grid_vertical-lr_h100_ar2", r#"<div id=i style="display:grid;writing-mode:vertical-lr;height:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("a_grid_vertical-lr_w100_ar2_tall", r#"<div id=i style="display:grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("a_grid_vertical-lr_w100_ar2_short", r#"<div id=i style="display:grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:20px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("a_grid_vertical-lr_w100_ar2_maxh", r#"<div id=i style="display:grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;max-height:80px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 80.0))]),
    ("a_grid_vertical-lr_w100_ar2_minh", r#"<div id=i style="display:grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;min-height:150px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 150.0))]),
    ("a_grid_vertical-lr_inline_w100_ar2", r#"<div id=i style="display:inline-grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("a_grid_vertical-lr_col_w100_ar2", r#"<div id=i style="display:grid;flex-direction:column;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_flex_vertical-rl_i100_w100_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_flex_vertical-rl_i100_w100_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_flex_vertical-rl_i100_w100_pad_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 120.0))]),
    ("b_flex_vertical-rl_i100_w100_pad_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 120.0))]),
    ("b_flex_vertical-rl_i100_w100_cb_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px;box-sizing:content-box"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 120.0, 120.0))]),
    ("b_flex_vertical-rl_i100_w100_cb_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px;box-sizing:content-box"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 120.0, 120.0))]),
    ("b_flex_vertical-rl_i100_w100_border_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;border:5px solid"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 110.0))]),
    ("b_flex_vertical-rl_i100_w100_border_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;border:5px solid"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 110.0))]),
    ("b_flex_vertical-rl_i100_w100_maxh_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_flex_vertical-rl_i100_w100_maxh_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_flex_vertical-rl_i100_w100_wrap_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-wrap:wrap"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_flex_vertical-rl_i100_w100_wrap_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-wrap:wrap"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_flex_vertical-rl_i100_w100_gap_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;gap:10px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_flex_vertical-rl_i100_w100_gap_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;gap:10px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_flex_vertical-rl_i100_ctx_flexrow", r#"<div style="display:flex"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_flex_vertical-rl_i100_ctx_flexrow_start", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_flex_vertical-rl_i100_ctx_flexcol", r#"<div style="display:flex;flex-direction:column;width:300px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_flex_vertical-rl_i100_ctx_float", r#"<div><div id=i style="float:left;display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_flex_vertical-rl_i100_ctx_flexgrow_row", r#"<div style="display:flex;width:300px"><div id=i style="display:flex;writing-mode:vertical-rl;width:20px;aspect-ratio:1;flex-grow:1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 300.0))]),
    ("b_flex_vertical-rl_i100_ctx_fixedh", r#"<div style="height:80px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_flex_vertical-rl_tall_w100_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_flex_vertical-rl_tall_w100_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_flex_vertical-rl_tall_w100_r1_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:1/1"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_flex_vertical-rl_tall_w100_r1_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:1/1"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_flex_vertical-rl_tall_w100_r12_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:1/2"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_flex_vertical-rl_tall_w100_r12_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:1/2"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_flex_vertical-rl_tall_w100_pad_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 320.0))]),
    ("b_flex_vertical-rl_tall_w100_pad_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 320.0))]),
    ("b_flex_vertical-rl_tall_w100_cb_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px;box-sizing:content-box"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 120.0, 320.0))]),
    ("b_flex_vertical-rl_tall_w100_cb_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px;box-sizing:content-box"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 120.0, 320.0))]),
    ("b_flex_vertical-rl_tall_w100_border_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;border:5px solid"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 310.0))]),
    ("b_flex_vertical-rl_tall_w100_border_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;border:5px solid"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 310.0))]),
    ("b_flex_vertical-rl_tall_w100_maxh_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_flex_vertical-rl_tall_w100_maxh_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_flex_vertical-rl_tall_w100_wrap_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-wrap:wrap"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_flex_vertical-rl_tall_w100_wrap_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-wrap:wrap"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_flex_vertical-rl_tall_w100_gap_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;gap:10px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_flex_vertical-rl_tall_w100_gap_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;gap:10px"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_flex_vertical-rl_tall_wpct_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:50%;aspect-ratio:2/1"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 300.0))]),
    ("b_flex_vertical-rl_tall_wpct_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:50%;aspect-ratio:2/1"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 392.0, 300.0))]),
    ("b_flex_vertical-rl_tall_ctx_flexrow", r#"<div style="display:flex"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_flex_vertical-rl_tall_ctx_flexrow_start", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_flex_vertical-rl_tall_ctx_flexcol", r#"<div style="display:flex;flex-direction:column;width:300px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_flex_vertical-rl_tall_ctx_float", r#"<div><div id=i style="float:left;display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_flex_vertical-rl_tall_ctx_flexgrow_row", r#"<div style="display:flex;width:300px"><div id=i style="display:flex;writing-mode:vertical-rl;width:20px;aspect-ratio:1;flex-grow:1"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 300.0))]),
    ("b_flex_vertical-rl_tall_ctx_fixedh", r#"<div style="height:80px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_flex_vertical-rl_two_w100_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("b_flex_vertical-rl_two_w100_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("b_flex_vertical-rl_two_w100_r1_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:1/1"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("b_flex_vertical-rl_two_w100_r1_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:1/1"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("b_flex_vertical-rl_two_w100_pad_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 150.0))]),
    ("b_flex_vertical-rl_two_w100_pad_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 150.0))]),
    ("b_flex_vertical-rl_two_w100_cb_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px;box-sizing:content-box"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 120.0, 150.0))]),
    ("b_flex_vertical-rl_two_w100_cb_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px;box-sizing:content-box"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div>"#, &[("i", (8.0, 8.0, 120.0, 150.0))]),
    ("b_flex_vertical-rl_two_w100_border_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;border:5px solid"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 140.0))]),
    ("b_flex_vertical-rl_two_w100_border_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;border:5px solid"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 140.0))]),
    ("b_flex_vertical-rl_two_w100_maxh_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_flex_vertical-rl_two_w100_maxh_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_flex_vertical-rl_two_w100_gap_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;gap:10px"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 140.0))]),
    ("b_flex_vertical-rl_two_w100_gap_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;gap:10px"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 140.0))]),
    ("b_flex_vertical-rl_two_wpct_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:50%;aspect-ratio:2/1"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 130.0))]),
    ("b_flex_vertical-rl_two_ctx_flexrow", r#"<div style="display:flex"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("b_flex_vertical-rl_two_ctx_flexrow_start", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("b_flex_vertical-rl_two_ctx_flexcol", r#"<div style="display:flex;flex-direction:column;width:300px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("b_flex_vertical-rl_two_ctx_float", r#"<div><div id=i style="float:left;display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("b_flex_vertical-rl_two_ctx_flexgrow_row", r#"<div style="display:flex;width:300px"><div id=i style="display:flex;writing-mode:vertical-rl;width:20px;aspect-ratio:1;flex-grow:1"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 300.0))]),
    ("b_flex_vertical-rl_two_ctx_fixedh", r#"<div style="height:80px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("b_flex_vertical-lr_i100_w100_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_flex_vertical-lr_i100_w100_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_flex_vertical-lr_i100_w100_pad_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;padding:10px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 120.0))]),
    ("b_flex_vertical-lr_i100_w100_pad_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;padding:10px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 120.0))]),
    ("b_flex_vertical-lr_i100_w100_cb_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;padding:10px;box-sizing:content-box"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 120.0, 120.0))]),
    ("b_flex_vertical-lr_i100_w100_cb_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;padding:10px;box-sizing:content-box"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 120.0, 120.0))]),
    ("b_flex_vertical-lr_i100_w100_border_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;border:5px solid"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 110.0))]),
    ("b_flex_vertical-lr_i100_w100_border_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;border:5px solid"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 110.0))]),
    ("b_flex_vertical-lr_i100_w100_maxh_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_flex_vertical-lr_i100_w100_maxh_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_flex_vertical-lr_i100_w100_wrap_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;flex-wrap:wrap"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_flex_vertical-lr_i100_w100_wrap_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;flex-wrap:wrap"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_flex_vertical-lr_i100_w100_gap_block", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;gap:10px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_flex_vertical-lr_i100_w100_gap_inline", r#"<div id=i style="display:inline-flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;gap:10px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_flex_vertical-lr_i100_ctx_flexrow", r#"<div style="display:flex"><div id=i style="display:flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_flex_vertical-lr_i100_ctx_flexrow_start", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_flex_vertical-lr_i100_ctx_flexcol", r#"<div style="display:flex;flex-direction:column;width:300px"><div id=i style="display:flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_flex_vertical-lr_i100_ctx_float", r#"<div><div id=i style="float:left;display:flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_flex_vertical-lr_i100_ctx_flexgrow_row", r#"<div style="display:flex;width:300px"><div id=i style="display:flex;writing-mode:vertical-lr;width:20px;aspect-ratio:1;flex-grow:1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 300.0))]),
    ("b_flex_vertical-lr_i100_ctx_fixedh", r#"<div style="height:80px"><div id=i style="display:flex;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_grid_vertical-rl_i100_w100_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_grid_vertical-rl_i100_w100_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_grid_vertical-rl_i100_w100_pad_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 120.0))]),
    ("b_grid_vertical-rl_i100_w100_pad_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 120.0))]),
    ("b_grid_vertical-rl_i100_w100_cb_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px;box-sizing:content-box"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 120.0, 120.0))]),
    ("b_grid_vertical-rl_i100_w100_cb_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px;box-sizing:content-box"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 120.0, 120.0))]),
    ("b_grid_vertical-rl_i100_w100_border_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;border:5px solid"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 110.0))]),
    ("b_grid_vertical-rl_i100_w100_border_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;border:5px solid"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 110.0))]),
    ("b_grid_vertical-rl_i100_w100_maxh_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_grid_vertical-rl_i100_w100_maxh_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_grid_vertical-rl_i100_w100_wrap_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;grid-auto-flow:column"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_grid_vertical-rl_i100_w100_wrap_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;grid-auto-flow:column"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_grid_vertical-rl_i100_w100_gap_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;gap:10px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_grid_vertical-rl_i100_w100_gap_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;gap:10px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_grid_vertical-rl_i100_ctx_flexrow", r#"<div style="display:flex"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_grid_vertical-rl_i100_ctx_flexcol", r#"<div style="display:flex;flex-direction:column;width:300px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_grid_vertical-rl_i100_ctx_float", r#"<div><div id=i style="float:left;display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_grid_vertical-rl_i100_ctx_flexgrow_row", r#"<div style="display:flex;width:300px"><div id=i style="display:grid;writing-mode:vertical-rl;width:20px;aspect-ratio:1;flex-grow:1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 300.0))]),
    ("b_grid_vertical-rl_i100_ctx_fixedh", r#"<div style="height:80px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_grid_vertical-rl_tall_w100_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_grid_vertical-rl_tall_w100_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_grid_vertical-rl_tall_w100_r1_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:1/1"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_grid_vertical-rl_tall_w100_r1_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:1/1"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_grid_vertical-rl_tall_w100_r12_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:1/2"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_grid_vertical-rl_tall_w100_r12_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:1/2"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_grid_vertical-rl_tall_w100_pad_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 320.0))]),
    ("b_grid_vertical-rl_tall_w100_pad_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 320.0))]),
    ("b_grid_vertical-rl_tall_w100_cb_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px;box-sizing:content-box"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 120.0, 320.0))]),
    ("b_grid_vertical-rl_tall_w100_cb_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px;box-sizing:content-box"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 120.0, 320.0))]),
    ("b_grid_vertical-rl_tall_w100_border_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;border:5px solid"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 310.0))]),
    ("b_grid_vertical-rl_tall_w100_border_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;border:5px solid"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 310.0))]),
    ("b_grid_vertical-rl_tall_w100_maxh_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_grid_vertical-rl_tall_w100_maxh_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_grid_vertical-rl_tall_w100_wrap_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;grid-auto-flow:column"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_grid_vertical-rl_tall_w100_wrap_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;grid-auto-flow:column"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_grid_vertical-rl_tall_w100_gap_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;gap:10px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_grid_vertical-rl_tall_w100_gap_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;gap:10px"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_grid_vertical-rl_tall_wpct_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:50%;aspect-ratio:2/1"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 300.0))]),
    ("b_grid_vertical-rl_tall_wpct_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:50%;aspect-ratio:2/1"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 392.0, 300.0))]),
    ("b_grid_vertical-rl_tall_ctx_flexrow", r#"<div style="display:flex"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_grid_vertical-rl_tall_ctx_flexcol", r#"<div style="display:flex;flex-direction:column;width:300px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_grid_vertical-rl_tall_ctx_float", r#"<div><div id=i style="float:left;display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_grid_vertical-rl_tall_ctx_fixedh", r#"<div style="height:80px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("b_grid_vertical-rl_two_w100_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_grid_vertical-rl_two_w100_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_grid_vertical-rl_two_w100_pad_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 90.0))]),
    ("b_grid_vertical-rl_two_w100_pad_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 90.0))]),
    ("b_grid_vertical-rl_two_w100_cb_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px;box-sizing:content-box"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 120.0, 90.0))]),
    ("b_grid_vertical-rl_two_w100_cb_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;padding:10px;box-sizing:content-box"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div>"#, &[("i", (8.0, 8.0, 120.0, 90.0))]),
    ("b_grid_vertical-rl_two_w100_border_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;border:5px solid"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 80.0))]),
    ("b_grid_vertical-rl_two_w100_border_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;border:5px solid"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 80.0))]),
    ("b_grid_vertical-rl_two_w100_maxh_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_grid_vertical-rl_two_w100_maxh_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_grid_vertical-rl_two_w100_wrap_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;grid-auto-flow:column"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("b_grid_vertical-rl_two_w100_wrap_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;grid-auto-flow:column"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("b_grid_vertical-rl_two_w100_gap_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;gap:10px"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_grid_vertical-rl_two_w100_gap_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;gap:10px"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_grid_vertical-rl_two_ctx_flexrow", r#"<div style="display:flex"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_grid_vertical-rl_two_ctx_flexcol", r#"<div style="display:flex;flex-direction:column;width:300px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_grid_vertical-rl_two_ctx_float", r#"<div><div id=i style="float:left;display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_grid_vertical-rl_two_ctx_flexgrow_row", r#"<div style="display:flex;width:300px"><div id=i style="display:grid;writing-mode:vertical-rl;width:20px;aspect-ratio:1;flex-grow:1"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 300.0))]),
    ("b_grid_vertical-rl_two_ctx_fixedh", r#"<div style="height:80px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_grid_vertical-lr_i100_w100_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_grid_vertical-lr_i100_w100_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_grid_vertical-lr_i100_w100_pad_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;padding:10px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 120.0))]),
    ("b_grid_vertical-lr_i100_w100_pad_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;padding:10px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 120.0))]),
    ("b_grid_vertical-lr_i100_w100_cb_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;padding:10px;box-sizing:content-box"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 120.0, 120.0))]),
    ("b_grid_vertical-lr_i100_w100_cb_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;padding:10px;box-sizing:content-box"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 120.0, 120.0))]),
    ("b_grid_vertical-lr_i100_w100_border_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;border:5px solid"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 110.0))]),
    ("b_grid_vertical-lr_i100_w100_border_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;border:5px solid"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 110.0))]),
    ("b_grid_vertical-lr_i100_w100_maxh_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_grid_vertical-lr_i100_w100_maxh_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("b_grid_vertical-lr_i100_w100_wrap_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;grid-auto-flow:column"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_grid_vertical-lr_i100_w100_wrap_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;grid-auto-flow:column"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_grid_vertical-lr_i100_w100_gap_block", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;gap:10px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_grid_vertical-lr_i100_w100_gap_inline", r#"<div id=i style="display:inline-grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1;gap:10px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_grid_vertical-lr_i100_ctx_flexrow", r#"<div style="display:flex"><div id=i style="display:grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_grid_vertical-lr_i100_ctx_flexcol", r#"<div style="display:flex;flex-direction:column;width:300px"><div id=i style="display:grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_grid_vertical-lr_i100_ctx_float", r#"<div><div id=i style="float:left;display:grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("b_grid_vertical-lr_i100_ctx_flexgrow_row", r#"<div style="display:flex;width:300px"><div id=i style="display:grid;writing-mode:vertical-lr;width:20px;aspect-ratio:1;flex-grow:1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 300.0, 300.0))]),
    ("b_grid_vertical-lr_i100_ctx_fixedh", r#"<div style="height:80px"><div id=i style="display:grid;writing-mode:vertical-lr;width:100px;aspect-ratio:2/1"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("e_flex_vertical-rl_block_i100_maxh30", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:30px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 30.0))]),
    ("e_flex_vertical-rl_block_i100_minh150", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:150px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 150.0))]),
    ("e_flex_vertical-rl_block_i100_minh30", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_flex_vertical-rl_block_i100_ovh", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_flex_vertical-rl_block_i100_ova", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:auto"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_flex_vertical-rl_block_i100_ovc", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:clip"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("e_flex_vertical-rl_block_i100_maxh30_minh60", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:30px;min-height:60px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 60.0))]),
    ("e_flex_vertical-rl_block_tall_maxh30", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:30px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 30.0))]),
    ("e_flex_vertical-rl_block_tall_minh150", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:150px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 150.0))]),
    ("e_flex_vertical-rl_block_tall_minh30", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_flex_vertical-rl_block_tall_ovh", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_flex_vertical-rl_block_tall_ova", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:auto"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_flex_vertical-rl_block_tall_ovc", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:clip"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("e_flex_vertical-rl_block_tall_maxh30_minh60", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:30px;min-height:60px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 60.0))]),
    ("e_flex_vertical-rl_block_two_wrap_ovh", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-wrap:wrap;overflow:hidden"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_flex_vertical-rl_block_two_col", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-direction:column"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("e_flex_vertical-rl_block_three_wrap_ovh", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-wrap:wrap;overflow:hidden"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div><div style="width:30px;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_flex_vertical-rl_block_three_col", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-direction:column"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div><div style="width:30px;height:20px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("e_flex_vertical-rl_block_i100_wrap", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-wrap:wrap"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("e_flex_vertical-rl_block_i100_wrap_maxh", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-wrap:wrap;max-height:70px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("e_flex_vertical-rl_block_i100_wrap_ovh", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-wrap:wrap;overflow:hidden"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_flex_vertical-rl_block_i100_col", r#"<div style="width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-direction:column"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("e_flex_vertical-rl_inline_i100_maxh30", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:30px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 30.0))]),
    ("e_flex_vertical-rl_inline_i100_minh150", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:150px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 150.0))]),
    ("e_flex_vertical-rl_inline_i100_minh30", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_flex_vertical-rl_inline_i100_ovh", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_flex_vertical-rl_inline_i100_ova", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:auto"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_flex_vertical-rl_inline_i100_ovc", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:clip"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("e_flex_vertical-rl_inline_i100_maxh30_minh60", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:30px;min-height:60px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 60.0))]),
    ("e_flex_vertical-rl_inline_tall_maxh30", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:30px"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 30.0))]),
    ("e_flex_vertical-rl_inline_tall_minh150", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:150px"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 150.0))]),
    ("e_flex_vertical-rl_inline_tall_minh30", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_flex_vertical-rl_inline_tall_ovh", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_flex_vertical-rl_inline_tall_ova", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:auto"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_flex_vertical-rl_inline_tall_ovc", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:clip"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("e_flex_vertical-rl_inline_tall_maxh30_minh60", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:30px;min-height:60px"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 60.0))]),
    ("e_flex_vertical-rl_inline_two_wrap_ovh", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-wrap:wrap;overflow:hidden"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_flex_vertical-rl_inline_two_col", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-direction:column"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("e_flex_vertical-rl_inline_three_wrap_ovh", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-wrap:wrap;overflow:hidden"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div><div style="width:30px;height:20px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_flex_vertical-rl_inline_three_col", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-direction:column"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div><div style="width:30px;height:20px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("e_flex_vertical-rl_inline_i100_wrap", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-wrap:wrap"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("e_flex_vertical-rl_inline_i100_wrap_maxh", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-wrap:wrap;max-height:70px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("e_flex_vertical-rl_inline_i100_wrap_ovh", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-wrap:wrap;overflow:hidden"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_flex_vertical-rl_inline_i100_col", r#"<div id=i style="display:inline-flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-direction:column"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("e_grid_vertical-rl_block_i100_maxh30", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:30px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 30.0))]),
    ("e_grid_vertical-rl_block_i100_minh150", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:150px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 150.0))]),
    ("e_grid_vertical-rl_block_i100_minh30", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_grid_vertical-rl_block_i100_ovh", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_grid_vertical-rl_block_i100_ova", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:auto"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_grid_vertical-rl_block_i100_ovc", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:clip"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("e_grid_vertical-rl_block_i100_maxh30_minh60", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:30px;min-height:60px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 60.0))]),
    ("e_grid_vertical-rl_block_tall_maxh30", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:30px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 30.0))]),
    ("e_grid_vertical-rl_block_tall_minh150", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:150px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 150.0))]),
    ("e_grid_vertical-rl_block_tall_minh30", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_grid_vertical-rl_block_tall_ovh", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_grid_vertical-rl_block_tall_ova", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:auto"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_grid_vertical-rl_block_tall_ovc", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:clip"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("e_grid_vertical-rl_block_tall_maxh30_minh60", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:30px;min-height:60px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 60.0))]),
    ("e_grid_vertical-rl_block_two_cols", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;grid-template-columns:30px 30px"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 60.0))]),
    ("e_grid_vertical-rl_block_two_rows", r#"<div style="width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;grid-auto-flow:column;grid-template-rows:60px"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("e_grid_vertical-rl_inline_i100_maxh30", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:30px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 30.0))]),
    ("e_grid_vertical-rl_inline_i100_minh150", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:150px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 150.0))]),
    ("e_grid_vertical-rl_inline_i100_minh30", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_grid_vertical-rl_inline_i100_ovh", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_grid_vertical-rl_inline_i100_ova", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:auto"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_grid_vertical-rl_inline_i100_ovc", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:clip"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("e_grid_vertical-rl_inline_i100_maxh30_minh60", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:30px;min-height:60px"><div style="width:30px;height:100px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 60.0))]),
    ("e_grid_vertical-rl_inline_tall_maxh30", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:30px"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 30.0))]),
    ("e_grid_vertical-rl_inline_tall_minh150", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:150px"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 150.0))]),
    ("e_grid_vertical-rl_inline_tall_minh30", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_grid_vertical-rl_inline_tall_ovh", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_grid_vertical-rl_inline_tall_ova", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:auto"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("e_grid_vertical-rl_inline_tall_ovc", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:clip"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("e_grid_vertical-rl_inline_tall_maxh30_minh60", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:30px;min-height:60px"><div style="width:30px;height:300px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 60.0))]),
    ("e_grid_vertical-rl_inline_two_cols", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;grid-template-columns:30px 30px"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 60.0))]),
    ("e_grid_vertical-rl_inline_two_rows", r#"<div id=i style="display:inline-grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;grid-auto-flow:column;grid-template-rows:60px"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("f_flex_flexrow_i100_plain", r#"<div style="display:flex"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("f_flex_flexrow_i100_ovh", r#"<div style="display:flex"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_flexrow_i100_minh30", r#"<div style="display:flex"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_flexrow_i100_maxh70", r#"<div style="display:flex"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("f_flex_flexrow_i100_two", r#"<div style="display:flex"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("f_flex_flexrow_i100_col", r#"<div style="display:flex"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-direction:column"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("f_flex_flexrow_i100_h100", r#"<div style="display:flex"><div id=i style="display:flex;writing-mode:vertical-rl;height:100px;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("f_flex_flexrow_tall_plain", r#"<div style="display:flex"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("f_flex_flexrow_tall_ovh", r#"<div style="display:flex"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_flexrow_tall_minh30", r#"<div style="display:flex"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_flexrow_tall_maxh70", r#"<div style="display:flex"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("f_flex_flexrow_tall_two", r#"<div style="display:flex"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("f_flex_flexrow_tall_col", r#"<div style="display:flex"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-direction:column"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("f_flex_flexrow_start_i100_plain", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("f_flex_flexrow_start_i100_ovh", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_flexrow_start_i100_minh30", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_flexrow_start_i100_maxh70", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("f_flex_flexrow_start_i100_two", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("f_flex_flexrow_start_i100_col", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-direction:column"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("f_flex_flexrow_start_i100_h100", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;height:100px;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("f_flex_flexrow_start_tall_plain", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("f_flex_flexrow_start_tall_ovh", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_flexrow_start_tall_minh30", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_flexrow_start_tall_maxh70", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("f_flex_flexrow_start_tall_two", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("f_flex_flexrow_start_tall_col", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-direction:column"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("f_flex_flexcol_i100_plain", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("f_flex_flexcol_i100_ovh", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_flexcol_i100_minh30", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_flexcol_i100_maxh70", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("f_flex_flexcol_i100_two", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("f_flex_flexcol_i100_col", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-direction:column"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("f_flex_flexcol_i100_h100", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;height:100px;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 100.0))]),
    ("f_flex_flexcol_i100_wauto", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("f_flex_flexcol_tall_plain", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("f_flex_flexcol_tall_ovh", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_flexcol_tall_minh30", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_flexcol_tall_maxh70", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("f_flex_flexcol_tall_two", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("f_flex_flexcol_tall_col", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-direction:column"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("f_flex_flexcol_tall_h100", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;height:100px;aspect-ratio:2/1;"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 100.0))]),
    ("f_flex_flexcol_tall_wauto", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:flex;writing-mode:vertical-rl;aspect-ratio:2/1;"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 300.0))]),
    ("f_flex_flexcol_start_i100_plain", r#"<div style="display:flex;flex-direction:column;width:400px;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("f_flex_flexcol_start_i100_ovh", r#"<div style="display:flex;flex-direction:column;width:400px;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_flexcol_start_i100_minh30", r#"<div style="display:flex;flex-direction:column;width:400px;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_flexcol_start_i100_maxh70", r#"<div style="display:flex;flex-direction:column;width:400px;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("f_flex_flexcol_start_i100_two", r#"<div style="display:flex;flex-direction:column;width:400px;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("f_flex_flexcol_start_i100_col", r#"<div style="display:flex;flex-direction:column;width:400px;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-direction:column"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("f_flex_flexcol_start_i100_h100", r#"<div style="display:flex;flex-direction:column;width:400px;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;height:100px;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("f_flex_flexcol_start_tall_plain", r#"<div style="display:flex;flex-direction:column;width:400px;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("f_flex_flexcol_start_tall_ovh", r#"<div style="display:flex;flex-direction:column;width:400px;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_flexcol_start_tall_minh30", r#"<div style="display:flex;flex-direction:column;width:400px;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_flexcol_start_tall_maxh70", r#"<div style="display:flex;flex-direction:column;width:400px;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("f_flex_flexcol_start_tall_two", r#"<div style="display:flex;flex-direction:column;width:400px;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("f_flex_flexcol_start_tall_col", r#"<div style="display:flex;flex-direction:column;width:400px;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-direction:column"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("f_flex_flexcol_start_tall_h100", r#"<div style="display:flex;flex-direction:column;width:400px;align-items:flex-start"><div id=i style="display:flex;writing-mode:vertical-rl;height:100px;aspect-ratio:2/1;"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("f_flex_gridctx_i100_ovh", r#"<div style="display:grid;width:400px;justify-items:start;align-items:start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_gridctx_i100_minh150", r#"<div style="display:grid;width:400px;justify-items:start;align-items:start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:150px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 150.0))]),
    ("f_flex_gridctx_i100_minh30", r#"<div style="display:grid;width:400px;justify-items:start;align-items:start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_gridctx_i100_h100", r#"<div style="display:grid;width:400px;justify-items:start;align-items:start"><div id=i style="display:flex;writing-mode:vertical-rl;height:100px;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("f_flex_gridctx_i100_wauto", r#"<div style="display:grid;width:400px;justify-items:start;align-items:start"><div id=i style="display:flex;writing-mode:vertical-rl;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("f_flex_gridctx_tall_ovh", r#"<div style="display:grid;width:400px;justify-items:start;align-items:start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_gridctx_tall_minh150", r#"<div style="display:grid;width:400px;justify-items:start;align-items:start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:150px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 150.0))]),
    ("f_flex_gridctx_tall_minh30", r#"<div style="display:grid;width:400px;justify-items:start;align-items:start"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_gridctx_tall_h100", r#"<div style="display:grid;width:400px;justify-items:start;align-items:start"><div id=i style="display:flex;writing-mode:vertical-rl;height:100px;aspect-ratio:2/1;"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("f_flex_gridctx_tall_wauto", r#"<div style="display:grid;width:400px;justify-items:start;align-items:start"><div id=i style="display:flex;writing-mode:vertical-rl;aspect-ratio:2/1;"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 600.0, 300.0))]),
    ("f_flex_flexwrap_i100_plain", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("f_flex_flexwrap_i100_ovh", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_flexwrap_i100_minh30", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_flexwrap_i100_maxh70", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("f_flex_flexwrap_i100_two", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("f_flex_flexwrap_i100_col", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-direction:column"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("f_flex_flexwrap_i100_h100", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:flex;writing-mode:vertical-rl;height:100px;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("f_flex_flexwrap_tall_plain", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("f_flex_flexwrap_tall_ovh", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_flexwrap_tall_minh30", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_flex_flexwrap_tall_maxh70", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("f_flex_flexwrap_tall_two", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:60px"></div><div style="width:30px;height:70px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 130.0))]),
    ("f_flex_flexwrap_tall_col", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:flex;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;flex-direction:column"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("f_grid_flexrow_i100_plain", r#"<div style="display:flex"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("f_grid_flexrow_i100_ovh", r#"<div style="display:flex"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_flexrow_i100_minh30", r#"<div style="display:flex"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_flexrow_i100_maxh70", r#"<div style="display:flex"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("f_grid_flexrow_i100_h100", r#"<div style="display:flex"><div id=i style="display:grid;writing-mode:vertical-rl;height:100px;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("f_grid_flexrow_tall_plain", r#"<div style="display:flex"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("f_grid_flexrow_tall_ovh", r#"<div style="display:flex"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_flexrow_tall_minh30", r#"<div style="display:flex"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_flexrow_tall_maxh70", r#"<div style="display:flex"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("f_grid_flexrow_tall_h100", r#"<div style="display:flex"><div id=i style="display:grid;writing-mode:vertical-rl;height:100px;aspect-ratio:2/1;"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("f_grid_flexrow_start_i100_plain", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("f_grid_flexrow_start_i100_ovh", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_flexrow_start_i100_minh30", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_flexrow_start_i100_maxh70", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("f_grid_flexrow_start_i100_h100", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:grid;writing-mode:vertical-rl;height:100px;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("f_grid_flexrow_start_tall_plain", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("f_grid_flexrow_start_tall_ovh", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_flexrow_start_tall_minh30", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_flexrow_start_tall_maxh70", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("f_grid_flexrow_start_tall_h100", r#"<div style="display:flex;align-items:flex-start"><div id=i style="display:grid;writing-mode:vertical-rl;height:100px;aspect-ratio:2/1;"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("f_grid_flexcol_i100_plain", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("f_grid_flexcol_i100_ovh", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_flexcol_i100_minh30", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_flexcol_i100_maxh70", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("f_grid_flexcol_i100_h100", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;height:100px;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 100.0))]),
    ("f_grid_flexcol_i100_wauto", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 200.0))]),
    ("f_grid_flexcol_tall_plain", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("f_grid_flexcol_tall_ovh", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_flexcol_tall_minh30", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_flexcol_tall_maxh70", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("f_grid_flexcol_tall_h100", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;height:100px;aspect-ratio:2/1;"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 100.0))]),
    ("f_grid_flexcol_tall_wauto", r#"<div style="display:flex;flex-direction:column;width:400px"><div id=i style="display:grid;writing-mode:vertical-rl;aspect-ratio:2/1;"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 400.0, 300.0))]),
    ("f_grid_flexcol_start_i100_plain", r#"<div style="display:flex;flex-direction:column;width:400px;align-items:flex-start"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("f_grid_flexcol_start_i100_ovh", r#"<div style="display:flex;flex-direction:column;width:400px;align-items:flex-start"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_flexcol_start_i100_minh30", r#"<div style="display:flex;flex-direction:column;width:400px;align-items:flex-start"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_flexcol_start_i100_maxh70", r#"<div style="display:flex;flex-direction:column;width:400px;align-items:flex-start"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("f_grid_flexcol_start_tall_plain", r#"<div style="display:flex;flex-direction:column;width:400px;align-items:flex-start"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("f_grid_flexcol_start_tall_ovh", r#"<div style="display:flex;flex-direction:column;width:400px;align-items:flex-start"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_flexcol_start_tall_minh30", r#"<div style="display:flex;flex-direction:column;width:400px;align-items:flex-start"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_flexcol_start_tall_maxh70", r#"<div style="display:flex;flex-direction:column;width:400px;align-items:flex-start"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("f_grid_gridctx_i100_ovh", r#"<div style="display:grid;width:400px;justify-items:start;align-items:start"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_gridctx_i100_minh150", r#"<div style="display:grid;width:400px;justify-items:start;align-items:start"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:150px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 150.0))]),
    ("f_grid_gridctx_i100_minh30", r#"<div style="display:grid;width:400px;justify-items:start;align-items:start"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_gridctx_i100_h100", r#"<div style="display:grid;width:400px;justify-items:start;align-items:start"><div id=i style="display:grid;writing-mode:vertical-rl;height:100px;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("f_grid_gridctx_tall_ovh", r#"<div style="display:grid;width:400px;justify-items:start;align-items:start"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_gridctx_tall_minh150", r#"<div style="display:grid;width:400px;justify-items:start;align-items:start"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:150px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 150.0))]),
    ("f_grid_gridctx_tall_minh30", r#"<div style="display:grid;width:400px;justify-items:start;align-items:start"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_gridctx_tall_h100", r#"<div style="display:grid;width:400px;justify-items:start;align-items:start"><div id=i style="display:grid;writing-mode:vertical-rl;height:100px;aspect-ratio:2/1;"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("f_grid_flexwrap_i100_plain", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("f_grid_flexwrap_i100_ovh", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_flexwrap_i100_minh30", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_flexwrap_i100_maxh70", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("f_grid_flexwrap_i100_h100", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:grid;writing-mode:vertical-rl;height:100px;aspect-ratio:2/1;"><div style="width:30px;height:100px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("f_grid_flexwrap_tall_plain", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 300.0))]),
    ("f_grid_flexwrap_tall_ovh", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;overflow:hidden"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_flexwrap_tall_minh30", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;min-height:30px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("f_grid_flexwrap_tall_maxh70", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:grid;writing-mode:vertical-rl;width:100px;aspect-ratio:2/1;max-height:70px"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 100.0, 70.0))]),
    ("f_grid_flexwrap_tall_h100", r#"<div style="display:flex;flex-wrap:wrap"><div id=i style="display:grid;writing-mode:vertical-rl;height:100px;aspect-ratio:2/1;"><div style="width:30px;height:300px"></div></div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
];
