//! The percentage width of a child of a vertical box refers to the box's width,
//! which is its block size, only when the content of the box does not decide
//! that width. The width an inline-block shrinks to, or a keyword width such as
//! `min-content` or `max-content` gives, comes from the content, so a
//! percentage width inside it reads as `auto`: a `50%` wide image in a vertical
//! inline-block is as wide as the image, not half of that. A width the box
//! states, or its parent fixes, is a basis, as is the height of a horizontal
//! box in the same position.
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

#[test]
fn a_percentage_width_in_a_shrink_to_fit_vertical_box_reads_as_auto() {
    SHRINK_TO_FIT.iter().for_each(check_case);
}

#[test]
fn a_percentage_width_in_a_vertical_box_with_a_keyword_width_reads_as_auto() {
    KEYWORD_WIDTH.iter().for_each(check_case);
}

#[test]
fn a_percentage_width_in_a_vertical_box_of_a_fixed_width_refers_to_it() {
    // The width is stated, comes from a percentage of a stated width, or is
    // settled by the table, flex or grid layout around the box.
    FIXED_WIDTH.iter().for_each(check_case);
}

#[test]
fn a_percentage_width_of_a_block_in_a_vertical_box_follows_the_same_rule() {
    PLAIN_BLOCKS.iter().for_each(check_case);
}

#[rustfmt::skip]
const SHRINK_TO_FIT: &[Case] = &[
    ("vib_rl_width50p", r#"<div id=t style="display:inline-block;writing-mode:vertical-rl"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vib_lr_width50p", r#"<div id=t style="display:inline-block;writing-mode:vertical-lr"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vib_rl_width100p", r#"<div id=t style="display:inline-block;writing-mode:vertical-rl"><svg id=s width=100 height=50 style="width:100%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vib_lr_width100p", r#"<div id=t style="display:inline-block;writing-mode:vertical-lr"><svg id=s width=100 height=50 style="width:100%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vib_rl_min-width50p", r#"<div id=t style="display:inline-block;writing-mode:vertical-rl"><svg id=s width=100 height=50 style="min-width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vib_lr_min-width50p", r#"<div id=t style="display:inline-block;writing-mode:vertical-lr"><svg id=s width=100 height=50 style="min-width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vib_rl_max-width50p", r#"<div id=t style="display:inline-block;writing-mode:vertical-rl"><svg id=s width=100 height=50 style="max-width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vib_lr_max-width50p", r#"<div id=t style="display:inline-block;writing-mode:vertical-lr"><svg id=s width=100 height=50 style="max-width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vib_rl_height50p", r#"<div id=t style="display:inline-block;writing-mode:vertical-rl"><svg id=s width=100 height=50 style="height:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 25.0))]),
    ("vib_lr_height50p", r#"<div id=t style="display:inline-block;writing-mode:vertical-lr"><svg id=s width=100 height=50 style="height:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 25.0))]),
    ("vib_h_rl", r#"<div id=t style="display:inline-block;writing-mode:vertical-rl;height:120px"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 120.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vib_h_lr", r#"<div id=t style="display:inline-block;writing-mode:vertical-lr;height:120px"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 120.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("txt_rl", r#"<div id=t style="display:inline-block;writing-mode:vertical-rl">aa <svg id=s width=100 height=50 style="width:50%"></svg> bb</div>"#, &[("t", (8.0, 8.0, 100.0, 110.0)), ("s", (8.0, 38.0, 100.0, 50.0))]),
    ("txt_lr", r#"<div id=t style="display:inline-block;writing-mode:vertical-lr">aa <svg id=s width=100 height=50 style="width:50%"></svg> bb</div>"#, &[("t", (8.0, 8.0, 100.0, 110.0)), ("s", (8.0, 38.0, 100.0, 50.0))]),
    ("nest_rl", r#"<div id=t style="display:inline-block;writing-mode:vertical-rl"><div id=u style="display:inline-block;writing-mode:vertical-rl"><svg id=s width=100 height=50 style="width:50%"></svg></div></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("u", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("nest_lr", r#"<div id=t style="display:inline-block;writing-mode:vertical-lr"><div id=u style="display:inline-block;writing-mode:vertical-lr"><svg id=s width=100 height=50 style="width:50%"></svg></div></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("u", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
];

#[rustfmt::skip]
const KEYWORD_WIDTH: &[Case] = &[
    ("vmin_rl_w50", r#"<div id=t style="display:inline-block;writing-mode:vertical-rl;width:min-content"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vmin_lr_w50", r#"<div id=t style="display:inline-block;writing-mode:vertical-lr;width:min-content"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vkw_min_rl", r#"<div id=t style="display:inline-block;writing-mode:vertical-rl;width:min-content"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vkw_min_lr", r#"<div id=t style="display:inline-block;writing-mode:vertical-lr;width:min-content"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vkw_max_rl", r#"<div id=t style="display:inline-block;writing-mode:vertical-rl;width:max-content"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vkw_max_lr", r#"<div id=t style="display:inline-block;writing-mode:vertical-lr;width:max-content"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vkw_fit_rl", r#"<div id=t style="display:inline-block;writing-mode:vertical-rl;width:fit-content"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vkw_fit_lr", r#"<div id=t style="display:inline-block;writing-mode:vertical-lr;width:fit-content"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vkwb_min_rl", r#"<div id=t style="writing-mode:vertical-rl;width:min-content"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vkwb_min_lr", r#"<div id=t style="writing-mode:vertical-lr;width:min-content"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vkwb_max_rl", r#"<div id=t style="writing-mode:vertical-rl;width:max-content"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vkwb_max_lr", r#"<div id=t style="writing-mode:vertical-lr;width:max-content"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vkwb_fit_rl", r#"<div id=t style="writing-mode:vertical-rl;width:fit-content"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vkwb_fit_lr", r#"<div id=t style="writing-mode:vertical-lr;width:fit-content"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vkwb_str_rl", r#"<div id=t style="writing-mode:vertical-rl;width:stretch"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 784.0, 50.0)), ("s", (400.0, 8.0, 392.0, 50.0))]),
    ("vkwb_str_lr", r#"<div id=t style="writing-mode:vertical-lr;width:stretch"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 784.0, 50.0)), ("s", (8.0, 8.0, 392.0, 50.0))]),
    ("kwtext_grid_min-content", r#"<div id=g style="display:grid;writing-mode:vertical-rl;width:min-content;height:100px;grid-template-columns:100px;grid-template-rows:auto"><div id=c style="writing-mode:horizontal-tb">aaaa bbbb</div></div>"#, &[("g", (8.0, 8.0, 90.0, 100.0)), ("c", (8.0, 8.0, 90.0, 100.0))]),
    ("kwtext_grid_max-content", r#"<div id=g style="display:grid;writing-mode:vertical-rl;width:max-content;height:100px;grid-template-columns:100px;grid-template-rows:auto"><div id=c style="writing-mode:horizontal-tb">aaaa bbbb</div></div>"#, &[("g", (8.0, 8.0, 90.0, 100.0)), ("c", (8.0, 8.0, 90.0, 100.0))]),
    ("kwtext_grid_fit-content", r#"<div id=g style="display:grid;writing-mode:vertical-rl;width:fit-content;height:100px;grid-template-columns:100px;grid-template-rows:auto"><div id=c style="writing-mode:horizontal-tb">aaaa bbbb</div></div>"#, &[("g", (8.0, 8.0, 90.0, 100.0)), ("c", (8.0, 8.0, 90.0, 100.0))]),
    ("kwtext_blk_min-content", r#"<div id=g style="writing-mode:vertical-rl;width:min-content;height:100px"><div id=c style="writing-mode:horizontal-tb">aaaa bbbb</div></div>"#, &[("g", (8.0, 8.0, 90.0, 100.0)), ("c", (8.0, 8.0, 90.0, 10.0))]),
    ("kwtext_blk_max-content", r#"<div id=g style="writing-mode:vertical-rl;width:max-content;height:100px"><div id=c style="writing-mode:horizontal-tb">aaaa bbbb</div></div>"#, &[("g", (8.0, 8.0, 90.0, 100.0)), ("c", (8.0, 8.0, 90.0, 10.0))]),
    ("kwtext_blk_fit-content", r#"<div id=g style="writing-mode:vertical-rl;width:fit-content;height:100px"><div id=c style="writing-mode:horizontal-tb">aaaa bbbb</div></div>"#, &[("g", (8.0, 8.0, 90.0, 100.0)), ("c", (8.0, 8.0, 90.0, 10.0))]),
    ("kwtext_vtxt_min-content", r#"<div id=g style="writing-mode:vertical-rl;width:min-content;height:30px">aaaa bbbb cc</div>"#, &[("g", (8.0, 8.0, 30.0, 30.0))]),
];

#[rustfmt::skip]
const FIXED_WIDTH: &[Case] = &[
    ("vib_rl_stated_w", r#"<div id=t style="display:inline-block;writing-mode:vertical-rl;width:200px"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 200.0, 50.0)), ("s", (108.0, 8.0, 100.0, 50.0))]),
    ("vib_lr_stated_w", r#"<div id=t style="display:inline-block;writing-mode:vertical-lr;width:200px"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 200.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vblock_rl_w50", r#"<div id=t style="writing-mode:vertical-rl;width:200px;height:200px"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 200.0, 200.0)), ("s", (108.0, 8.0, 100.0, 50.0))]),
    ("vblock_lr_w50", r#"<div id=t style="writing-mode:vertical-lr;width:200px;height:200px"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 200.0, 200.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vfl_rl_w50", r#"<div id=t style="float:left;writing-mode:vertical-rl"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vfl_lr_w50", r#"<div id=t style="float:left;writing-mode:vertical-lr"><svg id=s width=100 height=50 style="width:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vcell_rl_w50", r#"<div id=t style="display:table"><div id=c style="display:table-cell;writing-mode:vertical-rl"><svg id=s width=100 height=50 style="width:50%"></svg></div></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("c", (8.0, 8.0, 100.0, 50.0)), ("s", (58.0, 8.0, 50.0, 50.0))]),
    ("vcell_lr_w50", r#"<div id=t style="display:table"><div id=c style="display:table-cell;writing-mode:vertical-lr"><svg id=s width=100 height=50 style="width:50%"></svg></div></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("c", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 50.0, 50.0))]),
    ("cellw_rl", r#"<div id=u style="display:table"><div id=c style="display:table-cell;writing-mode:vertical-rl;width:200px"><svg id=s width=100 height=50 style="width:50%"></svg></div></div>"#, &[("u", (8.0, 8.0, 200.0, 50.0)), ("c", (8.0, 8.0, 200.0, 50.0)), ("s", (108.0, 8.0, 100.0, 50.0))]),
    ("cellw_lr", r#"<div id=u style="display:table"><div id=c style="display:table-cell;writing-mode:vertical-lr;width:200px"><svg id=s width=100 height=50 style="width:50%"></svg></div></div>"#, &[("u", (8.0, 8.0, 200.0, 50.0)), ("c", (8.0, 8.0, 200.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("pct_rl", r#"<div style="width:400px"><div id=t style="display:inline-block;writing-mode:vertical-rl;width:50%"><svg id=s width=100 height=50 style="width:50%"></svg></div></div>"#, &[("t", (8.0, 8.0, 200.0, 50.0)), ("s", (108.0, 8.0, 100.0, 50.0))]),
    ("pct_lr", r#"<div style="width:400px"><div id=t style="display:inline-block;writing-mode:vertical-lr;width:50%"><svg id=s width=100 height=50 style="width:50%"></svg></div></div>"#, &[("t", (8.0, 8.0, 200.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("nest2_rl", r#"<div id=t style="display:inline-block;writing-mode:vertical-rl;width:300px"><div id=u style="display:inline-block;writing-mode:vertical-rl"><svg id=s width=100 height=50 style="width:50%"></svg></div></div>"#, &[("t", (8.0, 8.0, 300.0, 50.0)), ("u", (208.0, 8.0, 100.0, 50.0)), ("s", (208.0, 8.0, 100.0, 50.0))]),
    ("nest2_lr", r#"<div id=t style="display:inline-block;writing-mode:vertical-lr;width:300px"><div id=u style="display:inline-block;writing-mode:vertical-lr"><svg id=s width=100 height=50 style="width:50%"></svg></div></div>"#, &[("t", (8.0, 8.0, 300.0, 50.0)), ("u", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vflex_rl", r#"<div id=t style="display:inline-flex;writing-mode:vertical-rl"><div id=c><svg id=s width=100 height=50 style="width:50%"></svg></div></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("c", (8.0, 8.0, 100.0, 50.0)), ("s", (58.0, 8.0, 50.0, 50.0))]),
    ("vflex_lr", r#"<div id=t style="display:inline-flex;writing-mode:vertical-lr"><div id=c><svg id=s width=100 height=50 style="width:50%"></svg></div></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("c", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 50.0, 50.0))]),
    ("vgrid_rl", r#"<div id=t style="display:inline-grid;writing-mode:vertical-rl"><div id=c><svg id=s width=100 height=50 style="width:50%"></svg></div></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("c", (8.0, 8.0, 100.0, 50.0)), ("s", (58.0, 8.0, 50.0, 50.0))]),
    ("vgrid_lr", r#"<div id=t style="display:inline-grid;writing-mode:vertical-lr"><div id=c><svg id=s width=100 height=50 style="width:50%"></svg></div></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("c", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 50.0, 50.0))]),
    ("hv_rl", r#"<div id=t style="display:inline-block"><div id=c style="writing-mode:vertical-rl"><svg id=s width=100 height=50 style="width:50%"></svg></div></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("c", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("hv_lr", r#"<div id=t style="display:inline-block"><div id=c style="writing-mode:vertical-lr"><svg id=s width=100 height=50 style="width:50%"></svg></div></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("c", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 50.0))]),
    ("vh_rl", r#"<div id=t style="display:inline-block;writing-mode:vertical-rl;height:200px"><svg id=s width=100 height=50 style="height:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 200.0)), ("s", (8.0, 8.0, 100.0, 100.0))]),
    ("vh_lr", r#"<div id=t style="display:inline-block;writing-mode:vertical-lr;height:200px"><svg id=s width=100 height=50 style="height:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 200.0)), ("s", (8.0, 8.0, 100.0, 100.0))]),
    ("vh_auto_rl", r#"<div id=t style="display:inline-block;writing-mode:vertical-rl"><svg id=s width=100 height=50 style="height:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 25.0))]),
    ("vh_auto_lr", r#"<div id=t style="display:inline-block;writing-mode:vertical-lr"><svg id=s width=100 height=50 style="height:50%"></svg></div>"#, &[("t", (8.0, 8.0, 100.0, 50.0)), ("s", (8.0, 8.0, 100.0, 25.0))]),
];

#[rustfmt::skip]
const PLAIN_BLOCKS: &[Case] = &[
    ("nr_vib_rl", r#"<div id=t style="display:inline-block;writing-mode:vertical-rl"><div id=s style="width:50%;background:red"><div style="width:10px;height:10px"></div></div><div style="width:80px;height:20px"></div></div>"#, &[("t", (8.0, 8.0, 90.0, 20.0)), ("s", (88.0, 8.0, 10.0, 20.0))]),
    ("nr_vib_lr", r#"<div id=t style="display:inline-block;writing-mode:vertical-lr"><div id=s style="width:50%;background:red"><div style="width:10px;height:10px"></div></div><div style="width:80px;height:20px"></div></div>"#, &[("t", (8.0, 8.0, 90.0, 20.0)), ("s", (8.0, 8.0, 10.0, 20.0))]),
    ("nr_vib_stated_rl", r#"<div id=t style="display:inline-block;writing-mode:vertical-rl;width:200px"><div id=s style="width:50%;background:red"><div style="width:10px;height:10px"></div></div></div>"#, &[("t", (8.0, 8.0, 200.0, 10.0)), ("s", (108.0, 8.0, 100.0, 10.0))]),
    ("nr_vib_stated_lr", r#"<div id=t style="display:inline-block;writing-mode:vertical-lr;width:200px"><div id=s style="width:50%;background:red"><div style="width:10px;height:10px"></div></div></div>"#, &[("t", (8.0, 8.0, 200.0, 10.0)), ("s", (8.0, 8.0, 100.0, 10.0))]),
    ("nr_float_rl", r#"<div id=t style="float:left;writing-mode:vertical-rl"><div id=s style="width:50%;background:red"><div style="width:10px;height:10px"></div></div><div style="width:80px;height:20px"></div></div>"#, &[("t", (8.0, 8.0, 90.0, 20.0)), ("s", (88.0, 8.0, 10.0, 20.0))]),
    ("nr_float_lr", r#"<div id=t style="float:left;writing-mode:vertical-lr"><div id=s style="width:50%;background:red"><div style="width:10px;height:10px"></div></div><div style="width:80px;height:20px"></div></div>"#, &[("t", (8.0, 8.0, 90.0, 20.0)), ("s", (8.0, 8.0, 10.0, 20.0))]),
    ("nr_block_rl", r#"<div id=t style="writing-mode:vertical-rl"><div id=s style="width:50%;background:red"><div style="width:10px;height:10px"></div></div></div>"#, &[("t", (8.0, 8.0, 10.0, 10.0)), ("s", (8.0, 8.0, 10.0, 10.0))]),
    ("nr_block_lr", r#"<div id=t style="writing-mode:vertical-lr"><div id=s style="width:50%;background:red"><div style="width:10px;height:10px"></div></div></div>"#, &[("t", (8.0, 8.0, 10.0, 10.0)), ("s", (8.0, 8.0, 10.0, 10.0))]),
    ("nr_flexitem_rl", r#"<div style="display:flex"><div id=t style="writing-mode:vertical-rl"><div id=s style="width:50%;background:red"><div style="width:10px;height:10px"></div></div><div style="width:80px;height:20px"></div></div></div>"#, &[("t", (8.0, 8.0, 90.0, 20.0)), ("s", (53.0, 8.0, 45.0, 20.0))]),
    ("nr_flexitem_lr", r#"<div style="display:flex"><div id=t style="writing-mode:vertical-lr"><div id=s style="width:50%;background:red"><div style="width:10px;height:10px"></div></div><div style="width:80px;height:20px"></div></div></div>"#, &[("t", (8.0, 8.0, 90.0, 20.0)), ("s", (8.0, 8.0, 45.0, 20.0))]),
];
