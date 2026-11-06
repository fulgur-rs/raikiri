//! A grid item whose `height` is a percentage resolves it against its grid
//! area once the tracks are settled, also when the row is sized by the item
//! itself: a row that holds a 10px tall box is 10px, and a `height: 50%` item
//! in it is 5px, not the 10px of its content. This holds for an item with an
//! `aspect-ratio` and a `max-height` or `max-width` that lowers the row
//! (`width: 40px; height: 50%; max-height: 15px` is 40px by 7.5px in a row of
//! 15px), as it does for one without.
//!
//! The expected border boxes were recorded from Chromium for the same markup in
//! an 800px by 600px viewport, with the body margin of 8px. The grids are 100px
//! wide unless a case says otherwise.

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
fn border_boxes(body: &str, body_style: &str) -> Vec<(String, Rect)> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ {body_style}; font: 10px/10px Ahem }} div {{ box-sizing: border-box }}</style>{body}"
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
    check_with_body_style(name, body, expected, "margin:8px");
}

fn check_with_body_style(name: &str, body: &str, expected: &[(&str, Rect)], body_style: &str) {
    let actual = border_boxes(body, body_style);
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

/// Run every case whose name satisfies `select`, and make sure some do.
fn check_group(select: fn(&str) -> bool) {
    let cases: Vec<&Case> = CASES.iter().filter(|case| select(case.0)).collect();
    assert!(!cases.is_empty(), "no case selected");
    for &&(name, body, expected) in &cases {
        check(name, body, expected);
    }
}

fn plain_items(name: &str) -> bool {
    name.starts_with("a_")
}

#[test]
fn a_percentage_height_resolves_against_the_row_the_item_sized() {
    check_group(plain_items);
}

fn ratio_items(name: &str) -> bool {
    name.starts_with("b_")
}

#[test]
fn a_ratio_item_with_a_percentage_height_resolves_it_against_the_row_its_bounds_gave() {
    check_group(ratio_items);
}

fn unchanged(name: &str) -> bool {
    name.starts_with("d_")
}

#[test]
fn items_that_were_already_sized_as_in_chromium_keep_their_boxes() {
    check_group(unchanged);
}

/// A case that no group selects would never be run.
#[test]
fn every_case_is_in_a_group() {
    let groups: &[fn(&str) -> bool] = &[plain_items, ratio_items, unchanged];
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
    ("a_p1_fixedrow10", r#"<div id=c style="width:100px;display:grid;grid-template-rows:10px;"><div id=i style="width:40px;height:50%;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 10.0)), ("i", (8.0, 8.0, 40.0, 5.0))]),
    ("a_p2_autorow", r#"<div id=c style="width:100px;display:grid;"><div id=i style="width:40px;height:50%;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 10.0)), ("i", (8.0, 8.0, 40.0, 5.0))]),
    ("a_p3_autorow_k30", r#"<div id=c style="width:100px;display:grid;"><div id=i style="width:40px;height:50%;"><div style="height:30px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 30.0)), ("i", (8.0, 8.0, 40.0, 15.0))]),
    ("a_p4_mincontent_row", r#"<div id=c style="width:100px;display:grid;grid-template-rows:min-content;"><div id=i style="width:40px;height:50%;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 10.0)), ("i", (8.0, 8.0, 40.0, 5.0))]),
    ("a_p5_maxcontent_row", r#"<div id=c style="width:100px;display:grid;grid-template-rows:max-content;"><div id=i style="width:40px;height:50%;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 10.0)), ("i", (8.0, 8.0, 40.0, 5.0))]),
    ("a_p6_autorow_start", r#"<div id=c style="width:100px;display:grid;align-items:start;"><div id=i style="width:40px;height:50%;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 10.0)), ("i", (8.0, 8.0, 40.0, 5.0))]),
    ("a_p7_autorow_sibling", r#"<div id=c style="width:100px;display:grid;grid-template-columns:50px 50px;"><div id=s style="height:10px"></div><div id=i style="width:40px;height:50%;"></div></div>"#, &[("c", (8.0, 8.0, 100.0, 10.0)), ("s", (8.0, 8.0, 50.0, 10.0)), ("i", (58.0, 8.0, 40.0, 5.0))]),
    ("a_p8_fixedcontainer_autorow", r#"<div id=c style="width:100px;height:100px;display:grid;grid-template-rows:auto;"><div id=i style="width:40px;height:50%;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 100.0)), ("i", (8.0, 8.0, 40.0, 50.0))]),
    ("a_p9_autorow_overflow", r#"<div id=c style="width:100px;display:grid;"><div id=i style="width:40px;height:50%;overflow:hidden;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 10.0)), ("i", (8.0, 8.0, 40.0, 5.0))]),
    ("a_p10_autorow_minh0", r#"<div id=c style="width:100px;display:grid;"><div id=i style="width:40px;height:50%;min-height:0;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 10.0)), ("i", (8.0, 8.0, 40.0, 5.0))]),
    ("a_p11_autorow_p75", r#"<div id=c style="width:100px;display:grid;"><div id=i style="width:40px;height:75%;"><div style="height:30px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 30.0)), ("i", (8.0, 8.0, 40.0, 22.5))]),
    ("a_p12_autorow_p150", r#"<div id=c style="width:100px;display:grid;"><div id=i style="width:40px;height:150%;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 10.0)), ("i", (8.0, 8.0, 40.0, 15.0))]),
    ("b_a_plain_maxh15", r#"<div id=c style="width:100px;display:grid;"><div id=i style="width:40px;height:50%;max-height:15px;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 10.0)), ("i", (8.0, 8.0, 40.0, 5.0))]),
    ("b_a_plain", r#"<div id=c style="width:100px;display:grid;"><div id=i style="width:40px;height:50%;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 10.0)), ("i", (8.0, 8.0, 40.0, 5.0))]),
    ("b_b_ratio_maxh15", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;max-height:15px;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 15.0)), ("i", (8.0, 8.0, 40.0, 7.5))]),
    ("b_c_ratio_maxh15_ovh", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;max-height:15px;overflow:hidden;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 15.0)), ("i", (8.0, 8.0, 40.0, 7.5))]),
    ("b_c_ratio_maxh15_minh0", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;max-height:15px;min-height:0;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 15.0)), ("i", (8.0, 8.0, 40.0, 7.5))]),
    ("b_d_ratio_maxh15_wauto", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;height:50%;max-height:15px;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 15.0)), ("i", (8.0, 8.0, 15.0, 7.5))]),
    ("b_e_ratio_maxw30_hauto_wp", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:50%;height:50%;max-width:30px;overflow:hidden;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 15.0)), ("i", (8.0, 8.0, 30.0, 7.5))]),
    ("b_f_ratio_nomax_ovh", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;overflow:hidden;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 20.0)), ("i", (8.0, 8.0, 40.0, 10.0))]),
    ("b_g_ratio_minh15_ovh", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;min-height:15px;overflow:hidden;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 20.0)), ("i", (8.0, 8.0, 40.0, 15.0))]),
    ("b_h_ratio_maxh15_stretchrow", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;max-height:15px;align-self:stretch;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 15.0)), ("i", (8.0, 8.0, 40.0, 7.5))]),
    ("b_i_ratio_maxh15_fixedrow", r#"<div id=c style="width:100px;display:grid;grid-template-rows:30px;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;max-height:15px;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 30.0)), ("i", (8.0, 8.0, 40.0, 15.0))]),
    ("b_m_ga_w40_h50p_maxh15_e_s", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;max-height:15px;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 15.0)), ("i", (8.0, 8.0, 40.0, 7.5))]),
    ("b_m_ga_w40_h50p_maxh15_e_st", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;max-height:15px;justify-self:start;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 15.0)), ("i", (8.0, 8.0, 40.0, 7.5))]),
    ("b_m_ga_w40_h50p_maxh15_k_s", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;max-height:15px;"><div style="width:100px;height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 15.0)), ("i", (8.0, 8.0, 40.0, 7.5))]),
    ("b_m_ga_w40_h50p_maxh15_k_st", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;max-height:15px;justify-self:start;"><div style="width:100px;height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 15.0)), ("i", (8.0, 8.0, 40.0, 7.5))]),
    ("b_m_ga_w40_h50p_maxh5_e_s", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;max-height:5px;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 5.0)), ("i", (8.0, 8.0, 40.0, 2.5))]),
    ("b_m_ga_w40_h50p_maxh5_e_st", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;max-height:5px;justify-self:start;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 5.0)), ("i", (8.0, 8.0, 40.0, 2.5))]),
    ("b_m_ga_w40_h50p_maxh5_k_s", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;max-height:5px;"><div style="width:100px;height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 5.0)), ("i", (8.0, 8.0, 40.0, 2.5))]),
    ("b_m_ga_w40_h50p_maxh5_k_st", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;max-height:5px;justify-self:start;"><div style="width:100px;height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 5.0)), ("i", (8.0, 8.0, 40.0, 2.5))]),
    ("b_m_ga_w50p_h50p_maxw30_e_s", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:50%;height:50%;max-width:30px;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 15.0)), ("i", (8.0, 8.0, 30.0, 7.5))]),
    ("b_m_ga_w50p_h50p_maxw30_e_st", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:50%;height:50%;max-width:30px;justify-self:start;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 15.0)), ("i", (8.0, 8.0, 30.0, 7.5))]),
    ("b_m_ga_w50p_h50p_maxw30_k_s", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:50%;height:50%;max-width:30px;"><div style="width:100px;height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 15.0)), ("i", (8.0, 8.0, 30.0, 7.5))]),
    ("b_m_ga_w50p_h50p_maxw30_k_st", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:50%;height:50%;max-width:30px;justify-self:start;"><div style="width:100px;height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 15.0)), ("i", (8.0, 8.0, 30.0, 7.5))]),
    ("d_m_ga_w40_h50p_e_s", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 20.0)), ("i", (8.0, 8.0, 40.0, 10.0))]),
    ("d_m_ga_w40_h50p_k_s", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;"><div style="width:100px;height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 20.0)), ("i", (8.0, 8.0, 40.0, 10.0))]),
    ("d_m_ga_w40_h50p_minh15_e_s", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;min-height:15px;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 20.0)), ("i", (8.0, 8.0, 40.0, 15.0))]),
    ("d_m_ga_w40_h50p_minh15_k_s", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;min-height:15px;"><div style="width:100px;height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 20.0)), ("i", (8.0, 8.0, 40.0, 15.0))]),
    ("d_m_ga_w40_h50p_minh30_e_s", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;min-height:30px;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 30.0)), ("i", (8.0, 8.0, 40.0, 30.0))]),
    ("d_m_ga_w40_h50p_minh30_k_s", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;min-height:30px;"><div style="width:100px;height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 30.0)), ("i", (8.0, 8.0, 40.0, 30.0))]),
    ("d_m_ga_w50p_h50p_e_s", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:50%;height:50%;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 25.0)), ("i", (8.0, 8.0, 50.0, 12.5))]),
    ("d_m_ga_w50p_h50p_k_s", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:50%;height:50%;"><div style="width:100px;height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 25.0)), ("i", (8.0, 8.0, 50.0, 12.5))]),
    ("d_m_ga_h50p_minh5_e_s", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;height:50%;min-height:5px;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 50.0)), ("i", (8.0, 8.0, 50.0, 25.0))]),
    ("d_m_ga_h50p_maxh5_e_s", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;height:50%;max-height:5px;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 5.0)), ("i", (8.0, 8.0, 5.0, 2.5))]),
    ("d_m_ga_h50p_maxh40_e_s", r#"<div id=c style="width:100px;display:grid;"><div id=i style="aspect-ratio:2/1;height:50%;max-height:40px;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 40.0)), ("i", (8.0, 8.0, 40.0, 20.0))]),
    ("d_m_gf_w40_h50p_maxh15_e_s", r#"<div id=c style="width:100px;height:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;max-height:15px;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 100.0)), ("i", (8.0, 8.0, 40.0, 15.0))]),
    ("d_m_gf_w40_h50p_maxh15_k_s", r#"<div id=c style="width:100px;height:100px;display:grid;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;max-height:15px;"><div style="width:100px;height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 100.0)), ("i", (8.0, 8.0, 40.0, 15.0))]),
    ("d_m_gf_h50p_maxh5_e_s", r#"<div id=c style="width:100px;height:100px;display:grid;"><div id=i style="aspect-ratio:2/1;height:50%;max-height:5px;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 100.0)), ("i", (8.0, 8.0, 10.0, 5.0))]),
    ("d_m_gr_h50p_e_s", r#"<div id=c style="width:100px;display:grid;grid-template-columns:100px;grid-template-rows:100px;"><div id=i style="aspect-ratio:2/1;height:50%;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 100.0)), ("i", (8.0, 8.0, 100.0, 50.0))]),
    ("d_m_gr_h50p_k_s", r#"<div id=c style="width:100px;display:grid;grid-template-columns:100px;grid-template-rows:100px;"><div id=i style="aspect-ratio:2/1;height:50%;"><div style="width:100px;height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 100.0)), ("i", (8.0, 8.0, 100.0, 50.0))]),
    ("d_m_gr_w40_h50p_maxh15_e_s", r#"<div id=c style="width:100px;display:grid;grid-template-columns:100px;grid-template-rows:100px;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;max-height:15px;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 100.0)), ("i", (8.0, 8.0, 40.0, 15.0))]),
    ("d_m_gr_w40_h50p_maxh15_k_s", r#"<div id=c style="width:100px;display:grid;grid-template-columns:100px;grid-template-rows:100px;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;max-height:15px;"><div style="width:100px;height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 100.0)), ("i", (8.0, 8.0, 40.0, 15.0))]),
    ("d_m_gc_h50p_e_st", r#"<div id=c style="display:grid;grid-template-columns:max-content;grid-template-rows:100px;"><div id=i style="aspect-ratio:2/1;height:50%;justify-self:start;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 784.0, 100.0)), ("i", (8.0, 8.0, 100.0, 50.0))]),
    ("d_m_gc_h50p_k_st", r#"<div id=c style="display:grid;grid-template-columns:max-content;grid-template-rows:100px;"><div id=i style="aspect-ratio:2/1;height:50%;justify-self:start;"><div style="width:100px;height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 784.0, 100.0)), ("i", (8.0, 8.0, 100.0, 50.0))]),
    ("d_m_gc_w40_h50p_maxh15_e_st", r#"<div id=c style="display:grid;grid-template-columns:max-content;grid-template-rows:100px;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;max-height:15px;justify-self:start;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 784.0, 100.0)), ("i", (8.0, 8.0, 40.0, 15.0))]),
    ("d_m_gc_w40_h50p_maxh15_k_st", r#"<div id=c style="display:grid;grid-template-columns:max-content;grid-template-rows:100px;"><div id=i style="aspect-ratio:2/1;width:40px;height:50%;max-height:15px;justify-self:start;"><div style="width:100px;height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 784.0, 100.0)), ("i", (8.0, 8.0, 40.0, 15.0))]),
    ("d_m_gc_h50p_maxh5_e_st", r#"<div id=c style="display:grid;grid-template-columns:max-content;grid-template-rows:100px;"><div id=i style="aspect-ratio:2/1;height:50%;max-height:5px;justify-self:start;"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 784.0, 100.0)), ("i", (8.0, 8.0, 10.0, 5.0))]),
    ("a_p13_calc", r#"<div id=c style="width:100px;display:grid;"><div id=i style="width:40px;height:calc(50% + 2px);"><div style="height:10px"></div></div></div>"#, &[("c", (8.0, 8.0, 100.0, 10.0)), ("i", (8.0, 8.0, 40.0, 7.0))]),
];

#[test]
fn a_cyclic_percentage_height_keeps_the_ratio_while_its_auto_row_is_sized() {
    for (child, width) in [("height:10px", 15.0), ("width:100px;height:10px", 30.0)] {
        check(
            child,
            &format!(
                "<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;height:50%;max-width:30px'><div style='{child}'></div></div></div>"
            ),
            &[
                ("c", (8.0, 8.0, 100.0, 15.0)),
                ("i", (8.0, 8.0, width, 7.5)),
            ],
        );
    }
}

#[test]
fn a_percentage_sized_ratio_item_contributes_to_its_max_content_column() {
    for alignment in ["", "justify-self:start"] {
        for (limit, width) in [("", 50.0), ("max-width:30px", 15.0)] {
            check(
                alignment,
                &format!(
                    "<div style='display:grid;grid-template-columns:max-content;grid-template-rows:100px'><div id=i style='aspect-ratio:2/1;width:50%;height:50%;{limit};{alignment}'><div style='height:10px'></div></div></div>"
                ),
                &[("i", (8.0, 8.0, width, 50.0))],
            );
        }
    }
}

#[test]
fn ratio_width_content_floors_keep_the_real_padding_basis() {
    check(
        "percentage padding and content floor",
        "<div id=c style='display:grid;width:100px'><div id=i style='aspect-ratio:2/1;height:50%;max-width:30px;padding:5%'><div style='width:100px;height:10px'></div></div></div>",
        &[
            ("c", (8.0, 8.0, 100.0, 15.0)),
            ("i", (8.0, 8.0, 30.0, 10.0)),
        ],
    );
}

#[test]
fn a_cyclic_width_contributes_content_even_with_a_small_max_height() {
    check(
        "content contributes before the percentage width resolves",
        "<div style='display:grid;grid-template-columns:max-content;grid-template-rows:100px'><div id=i style='aspect-ratio:2/1;width:50%;height:50%;justify-self:stretch;max-height:15px'><div style='width:100px;height:10px'></div></div></div>",
        &[("i", (8.0, 8.0, 50.0, 15.0))],
    );
}

#[test]
fn cyclic_ratio_contributions_use_a_min_content_floor() {
    for (text, width) in [("xx xx xx", 15.0), ("xxxx xxxx xxxx", 20.0)] {
        check(
            text,
            &format!(
                "<div style='display:grid;grid-template-columns:max-content;grid-template-rows:100px'><div id=i style='aspect-ratio:2/1;width:50%;height:50%;max-height:15px'>{text}</div></div>"
            ),
            &[("i", (8.0, 8.0, width, 15.0))],
        );
    }
}

#[test]
fn a_resolved_percentage_width_keeps_its_own_bounds() {
    for (bounds, height) in [("min-height:150px", 150.0), ("max-height:15px", 15.0)] {
        check(
            bounds,
            &format!(
                "<div style='display:grid;width:300px;grid-template:100px/200px'><div id=i style='aspect-ratio:2/1;width:50%;height:50%;{bounds}'><div style='height:10px'></div></div></div>"
            ),
            &[("i", (8.0, 8.0, 100.0, height))],
        );
    }
}

#[test]
fn a_nested_grid_uses_its_ratio_for_a_cyclic_percentage_height() {
    check(
        "a grid item that is itself a grid",
        "<div id=c style='display:grid;width:100px'><div id=i style='display:grid;aspect-ratio:2/1;height:50%;max-width:30px'><div style='width:100px;height:10px'></div></div></div>",
        &[("c", (8.0, 8.0, 100.0, 15.0)), ("i", (8.0, 8.0, 30.0, 7.5))],
    );
}

#[test]
fn percentage_ratio_items_match_the_chromium_matrix() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("data/grid-ratio-percent.json"))
            .expect("Chromium fixture");
    let cases = fixture["cases"].as_array().expect("cases");
    assert_eq!(cases.len(), 89);
    for case in cases {
        let container = &case["chromium"]["container"];
        let item = &case["chromium"]["item"];
        let px = |value: &serde_json::Value| value.as_f64().expect("coordinate") as f32;
        check_with_body_style(
            case["name"].as_str().expect("case name"),
            case["body"].as_str().expect("case body"),
            &[
                ("c", (0.0, 0.0, px(&container[0]), px(&container[1]))),
                (
                    "i",
                    (px(&item[0]), px(&item[1]), px(&item[2]), px(&item[3])),
                ),
            ],
            "margin:0;width:800px",
        );
    }
}
