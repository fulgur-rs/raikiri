//! A flex item that is a multicol container or a vertical paragraph takes its
//! size through its `aspect-ratio` as a horizontal paragraph does. An item that
//! states its main size and leaves the other `auto` takes the other size from the
//! main size it ends up with after flexing: a `20px` wide square that grows to
//! `100px` is `100px` high. Content raises that size above the ratio unless the
//! item scrolls or states a `min-*` size, and a vertical paragraph is raised by
//! its longest line that cannot break, not by the room its lines take when
//! nothing limits them.
//!
//! A paragraph item of a column that states no height takes its width from the
//! height the flex algorithm settled on, when it grows, shrinks or has a flex
//! basis.
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

fn case(name: &str) -> &'static Case {
    CASES
        .iter()
        .find(|case| case.0 == name)
        .unwrap_or_else(|| panic!("no case {name}"))
}

#[test]
fn a_multicol_item_takes_its_height_from_the_flexed_width() {
    for name in [
        "flex_mc_grow_1",
        "flex_mc_grow_L",
        "flex_mc_nogrow_1",
        "flex_mc_nogrow_L",
        "flex_mc_shrink_1",
        "flex_mc_shrink_L",
        "flex_mc_grow_scroll_1",
        "flex_mc_grow_scroll_L",
        "flex_mc_grow_minh_1",
        "flex_mc_grow_minh_L",
        "flex_mc_grow_tall_1",
        "flex_mc_grow_tall_L",
        "flex_mc_grow_basis_1",
        "flex_mc_grow_basis_L",
        "flex_mc_grow_pad_1",
        "flex_mc_grow_pad_L",
        "flex_mc_grow_gap_1",
        "flex_mc_grow_gap_L",
    ] {
        check_case(case(name));
    }
}

#[test]
fn a_multicol_item_of_a_column_takes_its_width_from_the_flexed_height() {
    for name in [
        "flex_mcc_grow_1",
        "flex_mcc_nogrow_1",
        "flex_mcc_grow_w_1",
        "flex_mc_grow_h_1",
        "flex_mc_grow_h_L",
    ] {
        check_case(case(name));
    }
}

#[test]
fn a_vertical_item_takes_its_height_from_the_flexed_width() {
    for name in [
        "flex_v_grow_1",
        "flex_v_grow_L",
        "flex_v_nogrow_1",
        "flex_v_nogrow_L",
        "flex_v_grow_lr_1",
        "flex_v_grow_lr_L",
        "flex_v_grow_scroll_1",
        "flex_v_grow_scroll_L",
        "flex_v_grow_minh_1",
        "flex_v_grow_minh_L",
        "flex_v_grow_h_1",
        "flex_v_grow_h_L",
        "flex_v_grow_ar2_1",
        "flex_v_grow_ar2_L",
    ] {
        check_case(case(name));
    }
}

#[test]
fn a_vertical_item_of_a_column_takes_its_width_from_the_flexed_height() {
    for name in ["flex_vc_grow_1", "flex_vc_grow_w_1"] {
        check_case(case(name));
    }
}

#[test]
fn a_vertical_item_is_raised_by_its_longest_line_that_cannot_break() {
    // The unbreakable text is 500px long and the ratio gives 100px; the text
    // that breaks at its spaces is 10px long at the narrowest, and 190px in
    // one line.
    for name in ["flex_v_grow_L", "ctl_vs_ratio_mw0_L", "ctl_vs_ratio_mw0_S"] {
        check_case(case(name));
    }
}

#[test]
fn items_with_a_stated_width_in_a_narrow_row_are_sized_as_chromium_sizes_them() {
    for case in CASES.iter().filter(|case| case.0.starts_with("ctl_")) {
        check_case(case);
    }
}

#[test]
fn a_multicol_or_vertical_box_outside_a_flex_container_follows_its_ratio() {
    for case in CASES.iter().filter(|case| case.0.starts_with("outside_")) {
        check_case(case);
    }
}

#[test]
fn a_multicol_item_is_sized_the_same_in_every_kind_of_flex_container() {
    for case in CASES.iter().filter(|case| case.0.starts_with("wrap_")) {
        check_case(case);
    }
}

#[test]
fn a_paragraph_item_of_a_column_that_states_no_height_takes_its_width_from_the_flexed_height() {
    // The height comes from the lines, a flex basis, a shrink or a grow, and
    // the width follows the height the flex algorithm settled on.
    for case in CASES.iter().filter(|case| case.0.starts_with("col_")) {
        check_case(case);
    }
}

#[test]
fn the_lines_of_a_multicol_box_raise_no_height_through_its_ratio() {
    // The columns fill the height the ratio gives and spill sideways into
    // more columns, in a flex container or outside one.
    for case in CASES
        .iter()
        .filter(|case| case.0.starts_with("mcblock_") || case.0.starts_with("mcflex_"))
    {
        check_case(case);
    }
}

#[test]
fn a_column_paragraph_item_with_min_or_max_sizes_takes_its_width_from_the_flexed_height() {
    for case in CASES.iter().filter(|case| case.0.starts_with("colpara_")) {
        check_case(case);
    }
}

#[test]
fn a_box_that_is_no_flex_item_ignores_the_direction_of_its_flex_parent_look_alike() {
    // `flex-direction` computes on a block, grid or inline-block container too,
    // where no flex algorithm settles a main size.
    for case in CASES.iter().filter(|case| case.0.starts_with("nonflex_")) {
        check_case(case);
    }
}

#[test]
fn every_case_matches_chromium() {
    CASES.iter().for_each(check_case);
}

#[rustfmt::skip]
const CASES: &[Case] = &[
    ("flex_mc_grow_1", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:20px;aspect-ratio:1;flex-grow:1">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("flex_mc_grow_L", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:20px;aspect-ratio:1;flex-grow:1">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("flex_mc_nogrow_1", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:50px;aspect-ratio:1">x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 50.0))]),
    ("flex_mc_nogrow_L", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:50px;aspect-ratio:1">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 50.0, 50.0))]),
    ("flex_mc_shrink_1", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:200px;aspect-ratio:1">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("flex_mc_shrink_L", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:200px;aspect-ratio:1">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 200.0, 200.0))]),
    ("flex_mc_grow_h_1", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;height:20px;aspect-ratio:1;flex-grow:1">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 20.0))]),
    ("flex_mc_grow_h_L", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;height:20px;aspect-ratio:1;flex-grow:1">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 1010.0, 20.0))]),
    ("flex_mc_grow_scroll_1", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:20px;aspect-ratio:1;flex-grow:1;overflow:hidden">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("flex_mc_grow_scroll_L", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:20px;aspect-ratio:1;flex-grow:1;overflow:hidden">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("flex_mc_grow_minh_1", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:20px;aspect-ratio:1;flex-grow:1;min-height:0">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("flex_mc_grow_minh_L", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:20px;aspect-ratio:1;flex-grow:1;min-height:0">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("flex_mc_grow_tall_1", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:20px;aspect-ratio:2/1;flex-grow:1">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("flex_mc_grow_tall_L", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:20px;aspect-ratio:2/1;flex-grow:1">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("flex_mc_grow_basis_1", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:20px;flex-basis:40px;aspect-ratio:1;flex-grow:1">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("flex_mc_grow_basis_L", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:20px;flex-basis:40px;aspect-ratio:1;flex-grow:1">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("flex_mc_grow_pad_1", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:20px;padding:5px;aspect-ratio:1;flex-grow:1">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("flex_mc_grow_pad_L", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:20px;padding:5px;aspect-ratio:1;flex-grow:1">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("flex_mc_grow_gap_1", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;column-gap:10px;width:20px;aspect-ratio:1;flex-grow:1">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("flex_mc_grow_gap_L", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;column-gap:10px;width:20px;aspect-ratio:1;flex-grow:1">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("flex_mcc_grow_1", r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=i style="columns:2;height:20px;aspect-ratio:1;flex-grow:1">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("flex_mcc_nogrow_1", r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=i style="columns:2;height:50px;aspect-ratio:1">x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 50.0))]),
    ("flex_mcc_grow_w_1", r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=i style="columns:2;width:20px;aspect-ratio:1;flex-grow:1">x</div></div>"#, &[("i", (8.0, 8.0, 20.0, 100.0))]),
    ("flex_v_grow_1", r#"<div style="display:flex;width:100px"><div id=i style="width:20px;aspect-ratio:1;flex-grow:1;writing-mode:vertical-rl">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("flex_v_grow_L", r#"<div style="display:flex;width:100px"><div id=i style="width:20px;aspect-ratio:1;flex-grow:1;writing-mode:vertical-rl">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 500.0))]),
    ("flex_v_nogrow_1", r#"<div style="display:flex;width:100px"><div id=i style="width:50px;aspect-ratio:1;writing-mode:vertical-rl">x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 50.0))]),
    ("flex_v_nogrow_L", r#"<div style="display:flex;width:100px"><div id=i style="width:50px;aspect-ratio:1;writing-mode:vertical-rl">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 50.0, 500.0))]),
    ("flex_v_grow_lr_1", r#"<div style="display:flex;width:100px"><div id=i style="width:20px;aspect-ratio:1;flex-grow:1;writing-mode:vertical-lr">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("flex_v_grow_lr_L", r#"<div style="display:flex;width:100px"><div id=i style="width:20px;aspect-ratio:1;flex-grow:1;writing-mode:vertical-lr">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 500.0))]),
    ("flex_v_grow_scroll_1", r#"<div style="display:flex;width:100px"><div id=i style="width:20px;aspect-ratio:1;flex-grow:1;writing-mode:vertical-rl;overflow:hidden">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("flex_v_grow_scroll_L", r#"<div style="display:flex;width:100px"><div id=i style="width:20px;aspect-ratio:1;flex-grow:1;writing-mode:vertical-rl;overflow:hidden">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("flex_v_grow_minh_1", r#"<div style="display:flex;width:100px"><div id=i style="width:20px;aspect-ratio:1;flex-grow:1;writing-mode:vertical-rl;min-height:0">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("flex_v_grow_minh_L", r#"<div style="display:flex;width:100px"><div id=i style="width:20px;aspect-ratio:1;flex-grow:1;writing-mode:vertical-rl;min-height:0">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("flex_v_grow_h_1", r#"<div style="display:flex;width:100px"><div id=i style="height:20px;aspect-ratio:1;flex-grow:1;writing-mode:vertical-rl">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 20.0))]),
    ("flex_v_grow_h_L", r#"<div style="display:flex;width:100px"><div id=i style="height:20px;aspect-ratio:1;flex-grow:1;writing-mode:vertical-rl">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 20.0))]),
    ("flex_v_grow_ar2_1", r#"<div style="display:flex;width:100px"><div id=i style="width:20px;aspect-ratio:2/1;flex-grow:1;writing-mode:vertical-rl">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("flex_v_grow_ar2_L", r#"<div style="display:flex;width:100px"><div id=i style="width:20px;aspect-ratio:2/1;flex-grow:1;writing-mode:vertical-rl">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 500.0))]),
    ("flex_vc_grow_1", r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=i style="height:20px;aspect-ratio:1;flex-grow:1;writing-mode:vertical-rl">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("flex_vc_grow_w_1", r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=i style="width:20px;aspect-ratio:1;flex-grow:1;writing-mode:vertical-rl">x</div></div>"#, &[("i", (8.0, 8.0, 20.0, 100.0))]),
    ("ctl_vs_noratio_1", r#"<div style="display:flex;width:100px"><div id=i style="width:200px;writing-mode:vertical-rl">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 10.0))]),
    ("ctl_vs_ratio_mw0_1", r#"<div style="display:flex;width:100px"><div id=i style="width:200px;aspect-ratio:1;min-width:0;writing-mode:vertical-rl">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("ctl_vs_ratio_h_1", r#"<div style="display:flex;width:100px"><div id=i style="width:200px;height:200px;aspect-ratio:1;writing-mode:vertical-rl">x</div></div>"#, &[("i", (8.0, 8.0, 200.0, 200.0))]),
    ("ctl_hs_ratio_1", r#"<div style="display:flex;width:100px"><div id=i style="width:200px;aspect-ratio:1">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("ctl_hs_noratio_1", r#"<div style="display:flex;width:100px"><div id=i style="width:200px">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 10.0))]),
    ("ctl_ms_ratio_1", r#"<div style="display:flex;width:100px"><div id=i style="width:200px;aspect-ratio:1;columns:2">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("ctl_ms_noratio_1", r#"<div style="display:flex;width:100px"><div id=i style="width:200px;columns:2">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 10.0))]),
    ("ctl_vs_noratio_L", r#"<div style="display:flex;width:100px"><div id=i style="width:200px;writing-mode:vertical-rl">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 500.0))]),
    ("ctl_vs_ratio_mw0_L", r#"<div style="display:flex;width:100px"><div id=i style="width:200px;aspect-ratio:1;min-width:0;writing-mode:vertical-rl">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 500.0))]),
    ("ctl_vs_ratio_h_L", r#"<div style="display:flex;width:100px"><div id=i style="width:200px;height:200px;aspect-ratio:1;writing-mode:vertical-rl">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 200.0, 200.0))]),
    ("ctl_hs_ratio_L", r#"<div style="display:flex;width:100px"><div id=i style="width:200px;aspect-ratio:1">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 200.0, 200.0))]),
    ("ctl_hs_noratio_L", r#"<div style="display:flex;width:100px"><div id=i style="width:200px">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 200.0, 10.0))]),
    ("ctl_ms_ratio_L", r#"<div style="display:flex;width:100px"><div id=i style="width:200px;aspect-ratio:1;columns:2">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 200.0, 200.0))]),
    ("ctl_ms_noratio_L", r#"<div style="display:flex;width:100px"><div id=i style="width:200px;columns:2">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 200.0, 10.0))]),
    ("ctl_vs_noratio_S", r#"<div style="display:flex;width:100px"><div id=i style="width:200px;writing-mode:vertical-rl">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 190.0))]),
    ("ctl_vs_ratio_mw0_S", r#"<div style="display:flex;width:100px"><div id=i style="width:200px;aspect-ratio:1;min-width:0;writing-mode:vertical-rl">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("ctl_vs_ratio_h_S", r#"<div style="display:flex;width:100px"><div id=i style="width:200px;height:200px;aspect-ratio:1;writing-mode:vertical-rl">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 200.0, 200.0))]),
    ("ctl_hs_ratio_S", r#"<div style="display:flex;width:100px"><div id=i style="width:200px;aspect-ratio:1">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("ctl_hs_noratio_S", r#"<div style="display:flex;width:100px"><div id=i style="width:200px">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 20.0))]),
    ("ctl_ms_ratio_S", r#"<div style="display:flex;width:100px"><div id=i style="width:200px;aspect-ratio:1;columns:2">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("ctl_ms_noratio_S", r#"<div style="display:flex;width:100px"><div id=i style="width:200px;columns:2">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 30.0))]),
    ("outside_blk_mc_h_L", r#"<div style="width:300px"><div id=i style="columns:2;height:50px;aspect-ratio:1">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 1010.0, 50.0))]),
    ("outside_blk_mc_wx_1", r#"<div style="width:300px"><div id=i style="columns:2;width:50px;aspect-ratio:1;max-height:30px">x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_blk_mc_wx_L", r#"<div style="width:300px"><div id=i style="columns:2;width:50px;aspect-ratio:1;max-height:30px">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_blk_mc_wx_S", r#"<div style="width:300px"><div id=i style="columns:2;width:50px;aspect-ratio:1;max-height:30px">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_blk_mc_hr2_L", r#"<div style="width:300px"><div id=i style="columns:2;aspect-ratio:2/1;height:30px">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 1010.0, 30.0))]),
    ("outside_blk_vr_w_L", r#"<div style="width:300px"><div id=i style="writing-mode:vertical-rl;width:50px;aspect-ratio:1">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 50.0, 500.0))]),
    ("outside_blk_vr_wx_1", r#"<div style="width:300px"><div id=i style="writing-mode:vertical-rl;width:50px;aspect-ratio:1;max-height:30px">x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_blk_vr_wx_L", r#"<div style="width:300px"><div id=i style="writing-mode:vertical-rl;width:50px;aspect-ratio:1;max-height:30px">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_blk_vr_wx_S", r#"<div style="width:300px"><div id=i style="writing-mode:vertical-rl;width:50px;aspect-ratio:1;max-height:30px">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_blk_vr_r2_L", r#"<div style="width:300px"><div id=i style="writing-mode:vertical-rl;aspect-ratio:2/1;width:100px">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 500.0))]),
    ("outside_blk_vl_w_L", r#"<div style="width:300px"><div id=i style="writing-mode:vertical-lr;width:50px;aspect-ratio:1">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 50.0, 500.0))]),
    ("outside_blk_vl_wx_1", r#"<div style="width:300px"><div id=i style="writing-mode:vertical-lr;width:50px;aspect-ratio:1;max-height:30px">x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_blk_vl_wx_L", r#"<div style="width:300px"><div id=i style="writing-mode:vertical-lr;width:50px;aspect-ratio:1;max-height:30px">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_blk_vl_wx_S", r#"<div style="width:300px"><div id=i style="writing-mode:vertical-lr;width:50px;aspect-ratio:1;max-height:30px">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_blk_vl_r2_L", r#"<div style="width:300px"><div id=i style="writing-mode:vertical-lr;aspect-ratio:2/1;width:100px">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 500.0))]),
    ("outside_ib_mc_h_L", r#"<div style="width:300px"><div id=i style="display:inline-block;columns:2;height:50px;aspect-ratio:1">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 1010.0, 50.0))]),
    ("outside_ib_mc_wx_1", r#"<div style="width:300px"><div id=i style="display:inline-block;columns:2;width:50px;aspect-ratio:1;max-height:30px">x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_ib_mc_wx_L", r#"<div style="width:300px"><div id=i style="display:inline-block;columns:2;width:50px;aspect-ratio:1;max-height:30px">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_ib_mc_wx_S", r#"<div style="width:300px"><div id=i style="display:inline-block;columns:2;width:50px;aspect-ratio:1;max-height:30px">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_ib_mc_hr2_L", r#"<div style="width:300px"><div id=i style="display:inline-block;columns:2;aspect-ratio:2/1;height:30px">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 1010.0, 30.0))]),
    ("outside_ib_vr_w_L", r#"<div style="width:300px"><div id=i style="display:inline-block;writing-mode:vertical-rl;width:50px;aspect-ratio:1">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 50.0, 500.0))]),
    ("outside_ib_vr_wx_1", r#"<div style="width:300px"><div id=i style="display:inline-block;writing-mode:vertical-rl;width:50px;aspect-ratio:1;max-height:30px">x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_ib_vr_wx_L", r#"<div style="width:300px"><div id=i style="display:inline-block;writing-mode:vertical-rl;width:50px;aspect-ratio:1;max-height:30px">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_ib_vr_wx_S", r#"<div style="width:300px"><div id=i style="display:inline-block;writing-mode:vertical-rl;width:50px;aspect-ratio:1;max-height:30px">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_ib_vr_r2_L", r#"<div style="width:300px"><div id=i style="display:inline-block;writing-mode:vertical-rl;aspect-ratio:2/1;width:100px">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 500.0))]),
    ("outside_ib_vl_w_L", r#"<div style="width:300px"><div id=i style="display:inline-block;writing-mode:vertical-lr;width:50px;aspect-ratio:1">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 50.0, 500.0))]),
    ("outside_ib_vl_wx_1", r#"<div style="width:300px"><div id=i style="display:inline-block;writing-mode:vertical-lr;width:50px;aspect-ratio:1;max-height:30px">x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_ib_vl_wx_L", r#"<div style="width:300px"><div id=i style="display:inline-block;writing-mode:vertical-lr;width:50px;aspect-ratio:1;max-height:30px">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_ib_vl_wx_S", r#"<div style="width:300px"><div id=i style="display:inline-block;writing-mode:vertical-lr;width:50px;aspect-ratio:1;max-height:30px">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_ib_vl_r2_L", r#"<div style="width:300px"><div id=i style="display:inline-block;writing-mode:vertical-lr;aspect-ratio:2/1;width:100px">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 500.0))]),
    ("outside_fl_mc_h_L", r#"<div style="width:300px"><div id=i style="float:left;columns:2;height:50px;aspect-ratio:1">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 1010.0, 50.0))]),
    ("outside_fl_mc_wx_1", r#"<div style="width:300px"><div id=i style="float:left;columns:2;width:50px;aspect-ratio:1;max-height:30px">x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_fl_mc_wx_L", r#"<div style="width:300px"><div id=i style="float:left;columns:2;width:50px;aspect-ratio:1;max-height:30px">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_fl_mc_wx_S", r#"<div style="width:300px"><div id=i style="float:left;columns:2;width:50px;aspect-ratio:1;max-height:30px">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_fl_mc_hr2_L", r#"<div style="width:300px"><div id=i style="float:left;columns:2;aspect-ratio:2/1;height:30px">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 1010.0, 30.0))]),
    ("outside_fl_vr_w_L", r#"<div style="width:300px"><div id=i style="float:left;writing-mode:vertical-rl;width:50px;aspect-ratio:1">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 50.0, 500.0))]),
    ("outside_fl_vr_h_1", r#"<div style="width:300px"><div id=i style="float:left;writing-mode:vertical-rl;height:50px;aspect-ratio:1">x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 50.0))]),
    ("outside_fl_vr_h_L", r#"<div style="width:300px"><div id=i style="float:left;writing-mode:vertical-rl;height:50px;aspect-ratio:1">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 50.0, 50.0))]),
    ("outside_fl_vr_h_S", r#"<div style="width:300px"><div id=i style="float:left;writing-mode:vertical-rl;height:50px;aspect-ratio:1">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 50.0))]),
    ("outside_fl_vr_wx_1", r#"<div style="width:300px"><div id=i style="float:left;writing-mode:vertical-rl;width:50px;aspect-ratio:1;max-height:30px">x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_fl_vr_wx_L", r#"<div style="width:300px"><div id=i style="float:left;writing-mode:vertical-rl;width:50px;aspect-ratio:1;max-height:30px">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_fl_vr_wx_S", r#"<div style="width:300px"><div id=i style="float:left;writing-mode:vertical-rl;width:50px;aspect-ratio:1;max-height:30px">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_fl_vr_r2_L", r#"<div style="width:300px"><div id=i style="float:left;writing-mode:vertical-rl;aspect-ratio:2/1;width:100px">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 500.0))]),
    ("outside_fl_vr_hr2_1", r#"<div style="width:300px"><div id=i style="float:left;writing-mode:vertical-rl;aspect-ratio:2/1;height:30px">x</div></div>"#, &[("i", (8.0, 8.0, 60.0, 30.0))]),
    ("outside_fl_vr_hr2_L", r#"<div style="width:300px"><div id=i style="float:left;writing-mode:vertical-rl;aspect-ratio:2/1;height:30px">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 60.0, 30.0))]),
    ("outside_fl_vr_hr2_S", r#"<div style="width:300px"><div id=i style="float:left;writing-mode:vertical-rl;aspect-ratio:2/1;height:30px">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 60.0, 30.0))]),
    ("outside_fl_vl_w_L", r#"<div style="width:300px"><div id=i style="float:left;writing-mode:vertical-lr;width:50px;aspect-ratio:1">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 50.0, 500.0))]),
    ("outside_fl_vl_h_1", r#"<div style="width:300px"><div id=i style="float:left;writing-mode:vertical-lr;height:50px;aspect-ratio:1">x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 50.0))]),
    ("outside_fl_vl_h_L", r#"<div style="width:300px"><div id=i style="float:left;writing-mode:vertical-lr;height:50px;aspect-ratio:1">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 50.0, 50.0))]),
    ("outside_fl_vl_h_S", r#"<div style="width:300px"><div id=i style="float:left;writing-mode:vertical-lr;height:50px;aspect-ratio:1">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 50.0))]),
    ("outside_fl_vl_wx_1", r#"<div style="width:300px"><div id=i style="float:left;writing-mode:vertical-lr;width:50px;aspect-ratio:1;max-height:30px">x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_fl_vl_wx_L", r#"<div style="width:300px"><div id=i style="float:left;writing-mode:vertical-lr;width:50px;aspect-ratio:1;max-height:30px">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_fl_vl_wx_S", r#"<div style="width:300px"><div id=i style="float:left;writing-mode:vertical-lr;width:50px;aspect-ratio:1;max-height:30px">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 30.0))]),
    ("outside_fl_vl_r2_L", r#"<div style="width:300px"><div id=i style="float:left;writing-mode:vertical-lr;aspect-ratio:2/1;width:100px">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 500.0))]),
    ("outside_fl_vl_hr2_1", r#"<div style="width:300px"><div id=i style="float:left;writing-mode:vertical-lr;aspect-ratio:2/1;height:30px">x</div></div>"#, &[("i", (8.0, 8.0, 60.0, 30.0))]),
    ("outside_fl_vl_hr2_L", r#"<div style="width:300px"><div id=i style="float:left;writing-mode:vertical-lr;aspect-ratio:2/1;height:30px">xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 60.0, 30.0))]),
    ("outside_fl_vl_hr2_S", r#"<div style="width:300px"><div id=i style="float:left;writing-mode:vertical-lr;aspect-ratio:2/1;height:30px">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 60.0, 30.0))]),
    ("wrap_wrow_mc_S", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap"><div id=i style="columns:2;aspect-ratio:1/1">x x x x x x x x x x</div></div>"#, &[("o", (8.0, 8.0, 440.0, 100.0)), ("i", (33.0, 33.0, 390.0, 390.0))]),
    ("wrap_wrow_mc_L", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap"><div id=i style="columns:2;aspect-ratio:1/1">xxxxxxxxxxxxxxx</div></div>"#, &[("o", (8.0, 8.0, 360.0, 100.0)), ("i", (33.0, 33.0, 310.0, 310.0))]),
    ("wrap_wrow_fs_mc_1", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap;align-items:flex-start"><div id=i style="columns:2;aspect-ratio:1/1">x</div></div>"#, &[("o", (8.0, 8.0, 80.0, 100.0)), ("i", (33.0, 33.0, 30.0, 30.0))]),
    ("wrap_wrow_fs_mc_S", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap;align-items:flex-start"><div id=i style="columns:2;aspect-ratio:1/1">x x x x x x x x x x</div></div>"#, &[("o", (8.0, 8.0, 440.0, 100.0)), ("i", (33.0, 33.0, 390.0, 390.0))]),
    ("wrap_wrow_fs_mc_L", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;flex-wrap:wrap;align-items:flex-start"><div id=i style="columns:2;aspect-ratio:1/1">xxxxxxxxxxxxxxx</div></div>"#, &[("o", (8.0, 8.0, 360.0, 100.0)), ("i", (33.0, 33.0, 310.0, 310.0))]),
    ("wrap_wrow_auto_mc_1", r#"<div id=o style="display:inline-flex;border:solid 25px green;flex-wrap:wrap"><div id=i style="columns:2;aspect-ratio:1/1">x</div></div>"#, &[("o", (8.0, 8.0, 80.0, 80.0)), ("i", (33.0, 33.0, 30.0, 30.0))]),
    ("wrap_wrow_auto_mc_S", r#"<div id=o style="display:inline-flex;border:solid 25px green;flex-wrap:wrap"><div id=i style="columns:2;aspect-ratio:1/1">x x x x x x x x x x</div></div>"#, &[("o", (8.0, 8.0, 440.0, 440.0)), ("i", (33.0, 33.0, 390.0, 390.0))]),
    ("wrap_wrow_auto_mc_L", r#"<div id=o style="display:inline-flex;border:solid 25px green;flex-wrap:wrap"><div id=i style="columns:2;aspect-ratio:1/1">xxxxxxxxxxxxxxx</div></div>"#, &[("o", (8.0, 8.0, 360.0, 360.0)), ("i", (33.0, 33.0, 310.0, 310.0))]),
    ("wrap_nrow_auto_mc_1", r#"<div id=o style="display:inline-flex;border:solid 25px green"><div id=i style="columns:2;aspect-ratio:1/1">x</div></div>"#, &[("o", (8.0, 8.0, 80.0, 80.0)), ("i", (33.0, 33.0, 30.0, 30.0))]),
    ("wrap_nrow_auto_mc_S", r#"<div id=o style="display:inline-flex;border:solid 25px green"><div id=i style="columns:2;aspect-ratio:1/1">x x x x x x x x x x</div></div>"#, &[("o", (8.0, 8.0, 440.0, 440.0)), ("i", (33.0, 33.0, 390.0, 390.0))]),
    ("wrap_nrow_auto_mc_L", r#"<div id=o style="display:inline-flex;border:solid 25px green"><div id=i style="columns:2;aspect-ratio:1/1">xxxxxxxxxxxxxxx</div></div>"#, &[("o", (8.0, 8.0, 360.0, 360.0)), ("i", (33.0, 33.0, 310.0, 310.0))]),
    ("wrap_nrow_fs_mc_1", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;align-items:flex-start"><div id=i style="columns:2;aspect-ratio:1/1">x</div></div>"#, &[("o", (8.0, 8.0, 80.0, 100.0)), ("i", (33.0, 33.0, 30.0, 30.0))]),
    ("wrap_nrow_fs_mc_S", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;align-items:flex-start"><div id=i style="columns:2;aspect-ratio:1/1">x x x x x x x x x x</div></div>"#, &[("o", (8.0, 8.0, 440.0, 100.0)), ("i", (33.0, 33.0, 390.0, 390.0))]),
    ("wrap_nrow_fs_mc_L", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green;align-items:flex-start"><div id=i style="columns:2;aspect-ratio:1/1">xxxxxxxxxxxxxxx</div></div>"#, &[("o", (8.0, 8.0, 360.0, 100.0)), ("i", (33.0, 33.0, 310.0, 310.0))]),
    ("wrap_nrow_st_mc_L", r#"<div id=o style="display:inline-flex;height:100px;border:solid 25px green"><div id=i style="columns:2;aspect-ratio:1/1">xxxxxxxxxxxxxxx</div></div>"#, &[("o", (8.0, 8.0, 360.0, 100.0)), ("i", (33.0, 33.0, 310.0, 50.0))]),
    ("wrap_wcol_mc_1", r#"<div id=o style="display:inline-flex;flex-direction:column;flex-wrap:wrap;height:100px;border:solid 25px green"><div id=i style="columns:2;aspect-ratio:1/1">x</div></div>"#, &[("o", (8.0, 8.0, 80.0, 100.0)), ("i", (33.0, 33.0, 30.0, 30.0))]),
    ("wrap_wcol_mc_S", r#"<div id=o style="display:inline-flex;flex-direction:column;flex-wrap:wrap;height:100px;border:solid 25px green"><div id=i style="columns:2;aspect-ratio:1/1">x x x x x x x x x x</div></div>"#, &[("o", (8.0, 8.0, 440.0, 100.0)), ("i", (33.0, 33.0, 390.0, 390.0))]),
    ("wrap_wcol_mc_L", r#"<div id=o style="display:inline-flex;flex-direction:column;flex-wrap:wrap;height:100px;border:solid 25px green"><div id=i style="columns:2;aspect-ratio:1/1">xxxxxxxxxxxxxxx</div></div>"#, &[("o", (8.0, 8.0, 360.0, 100.0)), ("i", (33.0, 33.0, 310.0, 310.0))]),
    ("col_col_g_fs_1", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_col_g_basis_fs_1", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex:1 1 20px;align-self:flex-start">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_col_g_min0_1", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;min-height:0;min-width:0;align-self:flex-start">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_col_g_min0_L", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;min-height:0;min-width:0;align-self:flex-start">xxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_col_g_min0_S", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;min-height:0;min-width:0;align-self:flex-start">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_col_sh_fs_1", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-shrink:1;align-self:flex-start;flex-basis:200px">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_col_g_ov_1", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start;overflow:hidden">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_col_g_ov_L", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start;overflow:hidden">xxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_col_g_ov_S", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start;overflow:hidden">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_colw_g_fs_1", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;flex-wrap:wrap"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_colw_g_basis_fs_1", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;flex-wrap:wrap"><div id=i style="aspect-ratio:1;flex:1 1 20px;align-self:flex-start">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_colw_g_min0_1", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;flex-wrap:wrap"><div id=i style="aspect-ratio:1;flex-grow:1;min-height:0;min-width:0;align-self:flex-start">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_colw_g_min0_L", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;flex-wrap:wrap"><div id=i style="aspect-ratio:1;flex-grow:1;min-height:0;min-width:0;align-self:flex-start">xxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_colw_g_min0_S", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;flex-wrap:wrap"><div id=i style="aspect-ratio:1;flex-grow:1;min-height:0;min-width:0;align-self:flex-start">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_colw_sh_fs_1", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;flex-wrap:wrap"><div id=i style="aspect-ratio:1;flex-shrink:1;align-self:flex-start;flex-basis:200px">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_colw_g_ov_1", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;flex-wrap:wrap"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start;overflow:hidden">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_colw_g_ov_L", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;flex-wrap:wrap"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start;overflow:hidden">xxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_colw_g_ov_S", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px;flex-wrap:wrap"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start;overflow:hidden">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("col_colauto_g_basis_fs_1", r#"<div style="display:flex;flex-direction:column;width:300px"><div id=i style="aspect-ratio:1;flex:1 1 20px;align-self:flex-start">x</div></div>"#, &[("i", (8.0, 8.0, 20.0, 20.0))]),
    ("col_colauto_sh_fs_1", r#"<div style="display:flex;flex-direction:column;width:300px"><div id=i style="aspect-ratio:1;flex-shrink:1;align-self:flex-start;flex-basis:200px">x</div></div>"#, &[("i", (8.0, 8.0, 200.0, 200.0))]),
    ("col_colauto_sh_fs_L", r#"<div style="display:flex;flex-direction:column;width:300px"><div id=i style="aspect-ratio:1;flex-shrink:1;align-self:flex-start;flex-basis:200px">xxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 200.0, 200.0))]),
    ("col_colauto_sh_fs_S", r#"<div style="display:flex;flex-direction:column;width:300px"><div id=i style="aspect-ratio:1;flex-shrink:1;align-self:flex-start;flex-basis:200px">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 200.0, 200.0))]),
    ("colpara_g_minh_1", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start;min-height:30px">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("colpara_g_minh_S", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start;min-height:30px">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("colpara_g_minh150_S", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start;min-height:150px">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 150.0, 150.0))]),
    ("colpara_g_maxw_1", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start;max-width:60px">x</div></div>"#, &[("i", (8.0, 8.0, 60.0, 100.0))]),
    ("colpara_g_pad_1", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start;padding:5px">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("colpara_g_pad_cb_1", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start;padding:5px;box-sizing:content-box">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("colpara_g_2_1", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:2/1;flex-grow:1;align-self:flex-start">x</div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("colpara_g_2_L", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:2/1;flex-grow:1;align-self:flex-start">xxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("colpara_g_2_S", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:2/1;flex-grow:1;align-self:flex-start">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 200.0, 100.0))]),
    ("colpara_g_mar_1", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start;margin:5px">x</div></div>"#, &[("i", (13.0, 13.0, 90.0, 90.0))]),
    ("colpara_sh_min0_1", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-shrink:1;flex-basis:200px;align-self:flex-start;min-height:0">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("colpara_sh_min0_S", r#"<div style="display:flex;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-shrink:1;flex-basis:200px;align-self:flex-start;min-height:0">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("nonflex_blk_1", r#"<div style="display:block;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start;height:20px">x</div></div>"#, &[("i", (8.0, 8.0, 20.0, 20.0))]),
    ("nonflex_blk_nh_1", r#"<div style="display:block;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start">x</div></div>"#, &[("i", (8.0, 8.0, 300.0, 300.0))]),
    ("nonflex_blk_L", r#"<div style="display:block;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start;height:20px">xxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 150.0, 20.0))]),
    ("nonflex_blk_nh_L", r#"<div style="display:block;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start">xxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 300.0, 300.0))]),
    ("nonflex_blk_S", r#"<div style="display:block;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start;height:20px">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 20.0, 20.0))]),
    ("nonflex_blk_nh_S", r#"<div style="display:block;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 300.0, 300.0))]),
    ("nonflex_grid_1", r#"<div style="display:grid;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start;height:20px">x</div></div>"#, &[("i", (8.0, 8.0, 20.0, 20.0))]),
    ("nonflex_grid_nh_1", r#"<div style="display:grid;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start">x</div></div>"#, &[("i", (8.0, 8.0, 300.0, 300.0))]),
    ("nonflex_grid_L", r#"<div style="display:grid;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start;height:20px">xxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 150.0, 20.0))]),
    ("nonflex_grid_nh_L", r#"<div style="display:grid;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start">xxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 300.0, 300.0))]),
    ("nonflex_grid_S", r#"<div style="display:grid;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start;height:20px">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 20.0, 20.0))]),
    ("nonflex_grid_nh_S", r#"<div style="display:grid;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 300.0, 300.0))]),
    ("nonflex_ib_1", r#"<div style="display:inline-block;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start;height:20px">x</div></div>"#, &[("i", (8.0, 8.0, 20.0, 20.0))]),
    ("nonflex_ib_nh_1", r#"<div style="display:inline-block;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start">x</div></div>"#, &[("i", (8.0, 8.0, 300.0, 300.0))]),
    ("nonflex_ib_L", r#"<div style="display:inline-block;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start;height:20px">xxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 150.0, 20.0))]),
    ("nonflex_ib_nh_L", r#"<div style="display:inline-block;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start">xxxxxxxxxxxxxxx</div></div>"#, &[("i", (8.0, 8.0, 300.0, 300.0))]),
    ("nonflex_ib_S", r#"<div style="display:inline-block;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start;height:20px">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 20.0, 20.0))]),
    ("nonflex_ib_nh_S", r#"<div style="display:inline-block;flex-direction:column;width:300px;height:100px"><div id=i style="aspect-ratio:1;flex-grow:1;align-self:flex-start">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 300.0, 300.0))]),
    ("mcblock_fillauto_1", r#"<div style="width:300px"><div id=i style="columns:2;column-fill:auto;width:50px;aspect-ratio:1">x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 50.0))]),
    ("mcblock_fillauto_S", r#"<div style="width:300px"><div id=i style="columns:2;column-fill:auto;width:50px;aspect-ratio:1">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 50.0, 50.0))]),
    ("mcblock_fillauto_M", r#"<div style="width:300px"><div id=i style="columns:2;column-fill:auto;width:50px;aspect-ratio:1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 50.0, 50.0))]),
    ("mcblock_cnt3_1", r#"<div style="width:300px"><div id=i style="columns:3;width:90px;aspect-ratio:2/1">x</div></div>"#, &[("i", (8.0, 8.0, 90.0, 45.0))]),
    ("mcblock_cnt3_S", r#"<div style="width:300px"><div id=i style="columns:3;width:90px;aspect-ratio:2/1">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 90.0, 45.0))]),
    ("mcblock_cnt3_M", r#"<div style="width:300px"><div id=i style="columns:3;width:90px;aspect-ratio:2/1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 90.0, 45.0))]),
    ("mcblock_both_1", r#"<div style="width:300px"><div id=i style="columns:2;width:100px;height:40px;aspect-ratio:1">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 40.0))]),
    ("mcblock_both_S", r#"<div style="width:300px"><div id=i style="columns:2;width:100px;height:40px;aspect-ratio:1">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 40.0))]),
    ("mcblock_both_M", r#"<div style="width:300px"><div id=i style="columns:2;width:100px;height:40px;aspect-ratio:1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 40.0))]),
    ("mcblock_h60_1", r#"<div style="width:300px"><div id=i style="columns:2;height:60px;aspect-ratio:1">x</div></div>"#, &[("i", (8.0, 8.0, 60.0, 60.0))]),
    ("mcblock_h60_S", r#"<div style="width:300px"><div id=i style="columns:2;height:60px;aspect-ratio:1">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 60.0, 60.0))]),
    ("mcblock_h60_M", r#"<div style="width:300px"><div id=i style="columns:2;height:60px;aspect-ratio:1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 60.0, 60.0))]),
    ("mcblock_gap_1", r#"<div style="width:300px"><div id=i style="column-count:2;column-gap:10px;width:80px;aspect-ratio:1">x</div></div>"#, &[("i", (8.0, 8.0, 80.0, 80.0))]),
    ("mcblock_gap_S", r#"<div style="width:300px"><div id=i style="column-count:2;column-gap:10px;width:80px;aspect-ratio:1">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 80.0, 80.0))]),
    ("mcblock_gap_M", r#"<div style="width:300px"><div id=i style="column-count:2;column-gap:10px;width:80px;aspect-ratio:1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 80.0, 80.0))]),
    ("mcblock_rule_1", r#"<div style="width:300px"><div id=i style="column-count:2;column-rule:2px solid red;width:80px;aspect-ratio:1">x</div></div>"#, &[("i", (8.0, 8.0, 80.0, 80.0))]),
    ("mcblock_rule_S", r#"<div style="width:300px"><div id=i style="column-count:2;column-rule:2px solid red;width:80px;aspect-ratio:1">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 80.0, 80.0))]),
    ("mcblock_rule_M", r#"<div style="width:300px"><div id=i style="column-count:2;column-rule:2px solid red;width:80px;aspect-ratio:1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 80.0, 80.0))]),
    ("mcblock_span_1", r#"<div style="width:300px"><div id=i style="columns:2;width:80px;aspect-ratio:1">x</div></div>"#, &[("i", (8.0, 8.0, 80.0, 80.0))]),
    ("mcblock_span_S", r#"<div style="width:300px"><div id=i style="columns:2;width:80px;aspect-ratio:1">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 80.0, 80.0))]),
    ("mcblock_span_M", r#"<div style="width:300px"><div id=i style="columns:2;width:80px;aspect-ratio:1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 80.0, 80.0))]),
    ("mcblock_minw_1", r#"<div style="width:300px"><div id=i style="columns:2;width:50px;min-width:80px;aspect-ratio:1">x</div></div>"#, &[("i", (8.0, 8.0, 80.0, 80.0))]),
    ("mcblock_minw_S", r#"<div style="width:300px"><div id=i style="columns:2;width:50px;min-width:80px;aspect-ratio:1">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 80.0, 80.0))]),
    ("mcblock_minw_M", r#"<div style="width:300px"><div id=i style="columns:2;width:50px;min-width:80px;aspect-ratio:1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 80.0, 80.0))]),
    ("mcblock_maxw_1", r#"<div style="width:300px"><div id=i style="columns:2;width:150px;max-width:80px;aspect-ratio:1">x</div></div>"#, &[("i", (8.0, 8.0, 80.0, 80.0))]),
    ("mcblock_maxw_S", r#"<div style="width:300px"><div id=i style="columns:2;width:150px;max-width:80px;aspect-ratio:1">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 80.0, 80.0))]),
    ("mcblock_maxw_M", r#"<div style="width:300px"><div id=i style="columns:2;width:150px;max-width:80px;aspect-ratio:1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 80.0, 80.0))]),
    ("mcblock_pad_1", r#"<div style="width:300px"><div id=i style="columns:2;width:80px;padding:10px;aspect-ratio:1">x</div></div>"#, &[("i", (8.0, 8.0, 80.0, 80.0))]),
    ("mcblock_pad_S", r#"<div style="width:300px"><div id=i style="columns:2;width:80px;padding:10px;aspect-ratio:1">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 80.0, 80.0))]),
    ("mcblock_pad_M", r#"<div style="width:300px"><div id=i style="columns:2;width:80px;padding:10px;aspect-ratio:1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 80.0, 80.0))]),
    ("mcblock_pad_cb_1", r#"<div style="width:300px"><div id=i style="columns:2;width:80px;padding:10px;box-sizing:content-box;aspect-ratio:1">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcblock_pad_cb_S", r#"<div style="width:300px"><div id=i style="columns:2;width:80px;padding:10px;box-sizing:content-box;aspect-ratio:1">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcblock_pad_cb_M", r#"<div style="width:300px"><div id=i style="columns:2;width:80px;padding:10px;box-sizing:content-box;aspect-ratio:1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcblock_auto_1", r#"<div style="width:300px"><div id=i style="columns:2;aspect-ratio:2/1">x</div></div>"#, &[("i", (8.0, 8.0, 300.0, 150.0))]),
    ("mcblock_auto_S", r#"<div style="width:300px"><div id=i style="columns:2;aspect-ratio:2/1">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 300.0, 150.0))]),
    ("mcblock_auto_M", r#"<div style="width:300px"><div id=i style="columns:2;aspect-ratio:2/1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 300.0, 150.0))]),
    ("mcblock_hauto_1", r#"<div style="width:300px"><div id=i style="columns:2;height:50px;aspect-ratio:2/1">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("mcblock_hauto_S", r#"<div style="width:300px"><div id=i style="columns:2;height:50px;aspect-ratio:2/1">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("mcblock_hauto_M", r#"<div style="width:300px"><div id=i style="columns:2;height:50px;aspect-ratio:2/1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 50.0))]),
    ("mcblock_colw_1", r#"<div style="width:300px"><div id=i style="column-width:30px;width:100px;aspect-ratio:1">x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcblock_colw_S", r#"<div style="width:300px"><div id=i style="column-width:30px;width:100px;aspect-ratio:1">x x x x x x x x x x</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcblock_colw_M", r#"<div style="width:300px"><div id=i style="column-width:30px;width:100px;aspect-ratio:1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_grow_M", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:20px;aspect-ratio:1;flex-grow:1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx </div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_grow_M_al", r#"<div style="display:flex;width:100px;align-items:flex-start"><div id=i style="columns:2;width:20px;aspect-ratio:1;flex-grow:1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx </div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_grow_S", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:20px;aspect-ratio:1;flex-grow:1">xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_grow_S_al", r#"<div style="display:flex;width:100px;align-items:flex-start"><div id=i style="columns:2;width:20px;aspect-ratio:1;flex-grow:1">xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_grow3_M", r#"<div style="display:flex;width:100px"><div id=i style="columns:3;width:20px;aspect-ratio:1;flex-grow:1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx </div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_grow3_M_al", r#"<div style="display:flex;width:100px;align-items:flex-start"><div id=i style="columns:3;width:20px;aspect-ratio:1;flex-grow:1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx </div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_grow3_S", r#"<div style="display:flex;width:100px"><div id=i style="columns:3;width:20px;aspect-ratio:1;flex-grow:1">xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_grow3_S_al", r#"<div style="display:flex;width:100px;align-items:flex-start"><div id=i style="columns:3;width:20px;aspect-ratio:1;flex-grow:1">xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_shrink_M", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:200px;aspect-ratio:1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx </div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_shrink_M_al", r#"<div style="display:flex;width:100px;align-items:flex-start"><div id=i style="columns:2;width:200px;aspect-ratio:1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx </div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_shrink_S", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:200px;aspect-ratio:1">xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_shrink_S_al", r#"<div style="display:flex;width:100px;align-items:flex-start"><div id=i style="columns:2;width:200px;aspect-ratio:1">xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_grow_minh0_M", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:20px;aspect-ratio:1;flex-grow:1;min-height:0">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx </div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_grow_minh0_M_al", r#"<div style="display:flex;width:100px;align-items:flex-start"><div id=i style="columns:2;width:20px;aspect-ratio:1;flex-grow:1;min-height:0">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx </div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_grow_minh0_S", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:20px;aspect-ratio:1;flex-grow:1;min-height:0">xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_grow_minh0_S_al", r#"<div style="display:flex;width:100px;align-items:flex-start"><div id=i style="columns:2;width:20px;aspect-ratio:1;flex-grow:1;min-height:0">xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_grow_ov_M", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:20px;aspect-ratio:1;flex-grow:1;overflow:hidden">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx </div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_grow_ov_M_al", r#"<div style="display:flex;width:100px;align-items:flex-start"><div id=i style="columns:2;width:20px;aspect-ratio:1;flex-grow:1;overflow:hidden">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx </div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_grow_ov_S", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;width:20px;aspect-ratio:1;flex-grow:1;overflow:hidden">xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_grow_ov_S_al", r#"<div style="display:flex;width:100px;align-items:flex-start"><div id=i style="columns:2;width:20px;aspect-ratio:1;flex-grow:1;overflow:hidden">xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_grow_fill_M", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;column-fill:auto;width:20px;aspect-ratio:1;flex-grow:1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx </div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_grow_fill_M_al", r#"<div style="display:flex;width:100px;align-items:flex-start"><div id=i style="columns:2;column-fill:auto;width:20px;aspect-ratio:1;flex-grow:1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx </div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_grow_fill_S", r#"<div style="display:flex;width:100px"><div id=i style="columns:2;column-fill:auto;width:20px;aspect-ratio:1;flex-grow:1">xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mc_grow_fill_S_al", r#"<div style="display:flex;width:100px;align-items:flex-start"><div id=i style="columns:2;column-fill:auto;width:20px;aspect-ratio:1;flex-grow:1">xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mcc_grow_M", r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=i style="columns:2;height:20px;aspect-ratio:1;flex-grow:1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx </div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mcc_grow_S", r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=i style="columns:2;height:20px;aspect-ratio:1;flex-grow:1">xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 100.0, 100.0))]),
    ("mcflex_mcc_nostate_M", r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=i style="columns:2;aspect-ratio:1;flex-grow:1">xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx xx </div></div>"#, &[("i", (8.0, 8.0, 300.0, 300.0))]),
    ("mcflex_mcc_nostate_S", r#"<div style="display:flex;flex-direction:column;height:100px;width:300px;align-items:flex-start"><div id=i style="columns:2;aspect-ratio:1;flex-grow:1">xx xx xx xx xx</div></div>"#, &[("i", (8.0, 8.0, 290.0, 290.0))]),
];
